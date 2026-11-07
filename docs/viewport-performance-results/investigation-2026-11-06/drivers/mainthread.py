"""Busy/idle summary of the main thread in a macOS `sample` file: `mainthread.py sample.txt`.

Only the block of the thread whose header names `com.apple.main-thread` is read. A file without that header, or with a
header and no call-graph nodes below it, is an error (exit 2 and a message): it must never be summarised from another
thread's samples or from the whole file.
"""
import collections
import re
import sys

HEADER = re.compile(r"\s+\d+ Thread_")
NODE = re.compile(r"^([\s+!:|]*)(\d+) (.*)$")


class MainThreadError(ValueError):
    pass


def parse(lines):
    """[[depth, samples, name], ...] of the main thread's call graph (the header line first)."""
    start = None
    for i, l in enumerate(lines):
        if "com.apple.main-thread" in l and HEADER.match(l):
            start = i
            break
    if start is None:
        raise MainThreadError("no main-thread header (a `Thread_N ... com.apple.main-thread` line) in the file")
    nodes = []
    for l in lines[start:]:
        m = re.match(r"^(\s*)[+!:| ]*(\d+) (.*)$", l)
        if not m:
            if nodes and l.strip() == "":
                break
            continue
        if HEADER.match(l) and nodes:
            break
        pre = NODE.match(l)
        depth = len(pre.group(1)); nodes.append([depth, int(pre.group(2)), pre.group(3)])
    if len(nodes) < 2:
        raise MainThreadError("the main-thread header has no call-graph nodes below it")
    if nodes[0][1] <= 0:
        raise MainThreadError("the main-thread header reports no samples")
    return nodes


def summarize(nodes):
    selfc = collections.Counter()
    total = nodes[0][1]
    for idx, (d, c, name) in enumerate(nodes):
        child = 0
        j = idx + 1
        while j < len(nodes) and nodes[j][0] > d:
            if nodes[j][0] == nodes[idx + 1][0] if idx + 1 < len(nodes) and nodes[idx + 1][0] > d else False:
                child += nodes[j][1]
            j += 1
        s = c - child
        if s > 0:
            selfc[name.split("  (in")[0][:60] + " " + (re.search(r"\(in ([^)]*)\)", name).group(1) if re.search(r"\(in ([^)]*)\)", name) else "")] += s
    idle = sum(v for k, v in selfc.items() if "mach_msg" in k or "__psynch_cvwait" in k or "kevent" in k)
    return total, idle, selfc


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 1:
        print("usage: mainthread.py <sample-file>", file=sys.stderr)
        return 2
    try:
        with open(argv[0]) as handle:
            lines = handle.read().split("\n")
        nodes = parse(lines)
    except (OSError, MainThreadError) as exc:
        print(f"mainthread.py: {argv[0]}: {exc}", file=sys.stderr)
        return 2
    total, idle, selfc = summarize(nodes)
    print("total", total)
    print("idle", idle, round(100 * idle / total, 1), "% busy", round(100 * (total - idle) / total, 1))
    for k, v in selfc.most_common(14):
        print(v, k)
    return 0


if __name__ == "__main__":
    sys.exit(main())
