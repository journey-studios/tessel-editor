"""SVG charts of the gaps between pictures and of the host's spans around a window."""
from ab.play import SLOW_DIAGNOSTIC_MS, SLOWER_DIAGNOSTIC_MS, gaps_of, pictures_of, play_pictures, play_segments


# ---------------------------------------------------------------------------------------------------- charts
PALETTE = {"sync": "#c2410c", "async": "#0369a1", "grid": "#d4d4d8", "text": "#27272a", "cut": "#16a34a", "pre": "#fef3c7"}


def svg_gaps(title, project, series, width=900, height=300):
    """Interval between accepted pictures (ms) against the frame shown after the gap; one polyline per mode."""
    left, right, top, bottom = 52, 16, 28, 36
    xs = [g[1] for gaps in series.values() for g in gaps]
    ys = [g[4] for gaps in series.values() for g in gaps]
    if not xs:
        return f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="40"><text x="8" y="24">{title}: no data</text></svg>'
    x0, x1 = 0, max(project["length"], max(xs))
    ymax = max(150.0, max(ys) * 1.05)
    px = lambda x: left + (x - x0) / (x1 - x0) * (width - left - right)
    py = lambda y: height - bottom - y / ymax * (height - top - bottom)
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" font-family="sans-serif" font-size="11">',
           f'<rect width="{width}" height="{height}" fill="white"/>',
           f'<text x="{left}" y="16" fill="{PALETTE["text"]}" font-size="13">{title}</text>']
    prefetch_frames = int(round(project["fps"] * 2))
    for cut in project["cuts"]:
        out.append(f'<rect x="{px(cut - prefetch_frames):.1f}" y="{top}" width="{px(cut) - px(cut - prefetch_frames):.1f}" height="{height - top - bottom}" fill="{PALETTE["pre"]}"/>')
        out.append(f'<line x1="{px(cut):.1f}" x2="{px(cut):.1f}" y1="{top}" y2="{height - bottom}" stroke="{PALETTE["cut"]}" stroke-width="1.5"/>')
    for level, label in ((1000 / project["fps"], "budget 41.7"), (SLOW_DIAGNOSTIC_MS, "diag 62.5"), (SLOWER_DIAGNOSTIC_MS, "diag 100")):
        out.append(f'<line x1="{left}" x2="{width - right}" y1="{py(level):.1f}" y2="{py(level):.1f}" stroke="{PALETTE["grid"]}" stroke-dasharray="4 3"/>')
        out.append(f'<text x="{width - right - 4}" y="{py(level) - 2:.1f}" text-anchor="end" fill="#71717a">{label}</text>')
    out.append(f'<line x1="{left}" x2="{left}" y1="{top}" y2="{height - bottom}" stroke="#71717a"/><line x1="{left}" x2="{width - right}" y1="{height - bottom}" y2="{height - bottom}" stroke="#71717a"/>')
    for mode, gaps in series.items():
        points = " ".join(f"{px(g[1]):.1f},{py(min(g[4], ymax)):.1f}" for g in gaps)
        out.append(f'<polyline points="{points}" fill="none" stroke="{PALETTE[mode]}" stroke-width="1.1" opacity="0.9"/>')
    legend = left + 8
    for mode in series:
        out.append(f'<rect x="{legend}" y="{top + 2}" width="10" height="10" fill="{PALETTE[mode]}"/><text x="{legend + 14}" y="{top + 11}" fill="{PALETTE["text"]}">{mode}</text>')
        legend += 70
    out.append(f'<text x="{(left + width - right) / 2}" y="{height - 8}" text-anchor="middle" fill="#52525b">frame shown after the gap (shaded: 2 s before a cut; green: cut)</text>')
    out.append(f'<text x="14" y="{(top + height - bottom) / 2}" transform="rotate(-90 14 {(top + height - bottom) / 2})" text-anchor="middle" fill="#52525b">interval (ms)</text>')
    out.append('</svg>')
    return "\n".join(out)


def svg_spans(title, events, frames, t0, t1, width=900):
    """Lanes of host spans in [t0, t1] (host clock): UI callbacks, reads, ticks (lateness), prefetch, sync and the gaps of pictures."""
    lanes = ["pictures (gap)", "UI callback", "asset read", "tick late", "prefetch", "sync"]
    left, top, lane_h = 118, 26, 22
    height = top + lane_h * len(lanes) + 34
    px = lambda t: left + (t - t0) / (t1 - t0) * (width - left - 12)
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" font-family="sans-serif" font-size="11">',
           f'<rect width="{width}" height="{height}" fill="white"/>',
           f'<text x="8" y="16" font-size="13" fill="{PALETTE["text"]}">{title} (host clock, ms)</text>']
    for index, lane in enumerate(lanes):
        y = top + index * lane_h
        out.append(f'<text x="6" y="{y + 14}" fill="#52525b">{lane}</text><line x1="{left}" x2="{width - 12}" y1="{y + lane_h}" y2="{y + lane_h}" stroke="{PALETTE["grid"]}"/>')

    def bar(lane, start, end, color, tip):
        if end < t0 or start > t1:
            return
        x = px(max(start, t0))
        w = max(1.0, px(min(end, t1)) - x)
        out.append(f'<rect x="{x:.1f}" y="{top + lane * lane_h + 4}" width="{w:.1f}" height="{lane_h - 8}" fill="{color}" opacity="0.85"><title>{tip}</title></rect>')

    shots = pictures_of(frames)
    for a, b in zip(shots, shots[1:]):
        gap = b["done_ms"] - a["done_ms"]
        bar(0, a["done_ms"], b["done_ms"], "#dc2626" if gap > SLOW_DIAGNOSTIC_MS else "#a1a1aa", f"{gap:.1f} ms")
    for e in events:
        f = e["fields"]
        if e["kind"] == "asset_cb":
            bar(1, f["recv_ms"], f["recv_ms"] + f["callback_ms"], PALETTE[f["mode"]], f"callback {f['callback_ms']} ms ({f['outcome']})")
        elif e["kind"] == "asset_io":
            bar(2, e["t_ms"] - f.get("read_ms", 0) - f.get("respond_ms", 0), e["t_ms"], PALETTE[f["mode"]], f"read {f.get('read_ms')} ms, {f.get('bytes')} bytes")
        elif e["kind"] == "tick" and f["late_ms"] > 5:
            bar(3, f["due_ms"], f["ran_ms"], "#7c3aed", f"late {f['late_ms']:.1f} ms")
        elif e["kind"] == "prefetch" and (f.get("total_ms") or 0) > 1:
            bar(4, e["t_ms"] - f["total_ms"], e["t_ms"], "#0d9488", f"prefetch {f['total_ms']:.1f} ms (plans_ahead {f.get('plans_ahead_ms')})")
        elif e["kind"] == "sync" and (f.get("total_ms") or 0) > 1:
            bar(5, e["t_ms"] - f["total_ms"], e["t_ms"], "#ca8a04", f"sync {f['total_ms']:.1f} ms")
    out.append(f'<text x="{left}" y="{height - 8}" fill="#52525b">{t0:.0f}</text><text x="{width - 12}" y="{height - 8}" text-anchor="end" fill="#52525b">{t1:.0f}</text>')
    out.append('</svg>')
    return "\n".join(out)


def write_charts(sessions, project, report, out):
    out.mkdir(parents=True, exist_ok=True)
    by_pair = {}
    for s in sessions:
        by_pair.setdefault(s["order_index"] // 2, {})[s["mode"]] = s
    names = []
    for pair, sides in sorted(by_pair.items()):
        for play_index, play in enumerate(("first", "repeat")):
            series = {}
            for mode, record in sides.items():
                segments = play_segments(record.get("events", []))
                if play_index < len(segments):
                    series[mode] = gaps_of(play_pictures(record["frames"], segments[play_index]))
            if series:
                name = f"gaps-pair{pair}-{play}.svg"
                (out / name).write_text(svg_gaps(f"pair {pair}, {play} play: interval between pictures", project, series))
                names.append(name)
    for s in sessions:
        segments = play_segments(s.get("events", []))
        if segments and project["cuts"] and segments[0]["clock_ms"] is not None:
            cut = project["cuts"][0]
            shots = [x for x in play_pictures(s["frames"], segments[0]) if x["shown_frame"] >= cut - int(project["fps"] * 2) - 2]
            if not shots:
                continue
            t0 = next((x["done_ms"] for x in shots), 0) - 300
            t1 = next((x["done_ms"] for x in shots if x["shown_frame"] >= cut + 6), shots[-1]["done_ms"]) + 300
            name = f"spans-session{s['order_index']}-{s['mode']}-first-cut{cut}.svg"
            (out / name).write_text(svg_spans(f"session {s['order_index']} ({s['mode']}), first play, window before cut {cut}", s["events"], s["frames"], t0, t1))
            names.append(name)
    return names
