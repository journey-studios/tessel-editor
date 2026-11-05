#!/usr/bin/env python3
"""PNG pairs from different GL stacks: PSNR and mean absolute luma. cross-stack.py <png-dir> [out.json]"""
import glob, json, math, os, re, sys
from PIL import Image, ImageChops, ImageStat
d = sys.argv[1]; rows = []; files = {}
for p in glob.glob(f"{d}/*.png"):
    m = re.match(r"(wk|chrome-metal|chrome-swiftshader)-(h:.+)\.png$", os.path.basename(p))
    if m: files.setdefault(m.group(2), {})[m.group(1)] = p
def cmp(a, b):
    A, B = Image.open(a).convert("RGB"), Image.open(b).convert("RGB")
    if A.size != B.size: return None
    D = ImageChops.difference(A, B); hist = D.histogram(); n = A.size[0] * A.size[1] * 3
    mse = sum(i * i * hist[c * 256 + i] for c in range(3) for i in range(256)) / n
    return {"psnr_db": 99.0 if mse == 0 else round(10 * math.log10(255 ** 2 / mse), 2), "mean_abs": round(sum(i * hist[c * 256 + i] for c in range(3) for i in range(256)) / n, 3), "max_diff": max(i for c in range(3) for i in range(256) if hist[c * 256 + i])}
for key, f in sorted(files.items()):
    for a, b in (("wk", "chrome-metal"), ("wk", "chrome-swiftshader"), ("chrome-metal", "chrome-swiftshader")):
        if a in f and b in f:
            r = cmp(f[a], f[b])
            if r: rows.append(dict(frame=key, pair=f"{a}~{b}", **r))
    # same engine, baseline vs retained
    if ":baseline-" in key and key.replace(":baseline-", ":retained-") in files:
        for e, p in f.items():
            q = files[key.replace(":baseline-", ":retained-")].get(e)
            if q: r = cmp(p, q); rows.append(dict(frame=key, pair=f"{e}:baseline~retained", **r))
print(len(rows));
by = {}
for r in rows: by.setdefault(r["pair"].split(":")[-1] if "baseline" in r["pair"] else r["pair"], []).append(r)
for k, v in by.items():
    print(k, "n", len(v), "psnr min", min(x["psnr_db"] for x in v), "mean_abs max", max(x["mean_abs"] for x in v), "worst", min(v, key=lambda x: x["psnr_db"])["frame"])
if len(sys.argv) > 2: json.dump(rows, open(sys.argv[2], "w"), indent=0)
