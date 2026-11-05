#!/usr/bin/env python3
"""Attributes the main thread of `sample` output: for each marker, the largest sample count of a call-tree line holding it."""
import re, sys, json
MARK = {"total_main": None, "run_loop_wait(mach_msg)": r"mach_msg2_trap|mach_msg_trap", "gpui_window_draw": r"Window::draw\b|window::Window::draw", "gpui_layout": r"compute_layout|request_layout", "gpui_prepaint": r"prepaint", "gpui_paint": r"::paint\b", "metal_renderer_draw": r"MetalRenderer|metal_renderer|renderer::.*draw", "wk_script_eval": r"evaluateJavaScript|evaluate_script", "wk_ipc_message": r"didReceiveScriptMessage|didPostMessage|ScriptMessageHandler", "on_ipc": r"viewport_ipc", "dispatch_event": r"dispatch_event", "workstation_render": r"Workstation.*render|render_ruler|as gpui.*Render", "theme_tokens": r"Tokens::new", "text_layout": r"text_system|TextLayout|LineLayout|CTLine|shape_line"}
for path in sys.argv[1:]:
    text = open(path).read(); start = text.find("Call graph:"); end = text.find("Total number in stack"); body = text[start:end]
    threads = re.split(r"\n    (?=\d+ Thread_)", body); main = next(t for t in threads if "com.apple.main-thread" in t.split("\n")[0] or "DispatchQueue_1" in t.split("\n")[0])
    total = int(re.match(r"\s*(?:Call graph:\s*)?(\d+)", main).group(1)) if re.match(r"\s*(?:Call graph:\s*)?(\d+)", main) else None
    lines = main.split("\n"); res = {"samples": total}
    for name, rx in MARK.items():
        if not rx: continue
        # sum nonoverlapping: take lines whose ancestors do not match (shallowest occurrences)
        pat = re.compile(rx); tot = 0; depth_block = None
        for l in lines:
            m = re.match(r"([ +!:|]*)(\d+) (.*)", l)
            if not m: continue
            d = len(m.group(1))
            if depth_block is not None and d > depth_block: continue
            depth_block = None
            if pat.search(m.group(3)): tot += int(m.group(2)); depth_block = d
        res[name] = tot
    print(path.split("/")[-1], json.dumps(res))
