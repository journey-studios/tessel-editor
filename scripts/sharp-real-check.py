#!/usr/bin/env python3
"""BL143.1/BL143.8 — a REAL SHARP PLY against the source image it was made from (read-only, local only).

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/sharp-real-check.py --ply PATH.ply --image PATH.png \
        [--headless target/debug/tessel-headless] [--render target/debug/tessel-render] [--report FILE]

Nothing is copied into the repository and nothing is written next to the inputs: the rendered frames go to a
private temporary directory that is removed before the script returns; only numbers are printed.

1. The Rust reader (`asset.add` in `tessel-headless`) reads the file's own `intrinsic`/`image_size`/`color_space`
   and the capture camera; they must describe the source image's size.
2. The real renderer (`tessel-render`: Node + Chrome + Three.js + Spark, SwiftShader by default) draws the splat
   at the size of the source image with the capture camera and axes, and the frame is compared with the image by
   FFmpeg (`ssim`, mean colour per channel and per quadrant). No numpy, no Pillow.
3. Negative control: the same file imported with `capture: ignore` (no axis turn, no capture camera) must NOT
   land on the picture. A metric that cannot tell the two apart would prove nothing.

Declared tolerances (BL143.8): SSIM >= 0.90 for the capture render, mean colour within 4/255 per channel and
per quadrant, and the control at least 0.5 below the capture render. They are loose on purpose: Spark draws
Gaussians, not the source pixels. Observed on `t1-teatro` (1672x941): SSIM 0.9625, mean colour within 1/255.
This checks the preview/export stack (headless Chrome); the native WKWebView is a different GL stack.
"""
import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SSIM_MIN = 0.90
MEAN_TOLERANCE = 4
CONTROL_MARGIN = 0.5


def run(cmd, stdin=None, timeout=600):
    return subprocess.run(cmd, input=stdin, capture_output=True, text=True, timeout=timeout)


def session(binary, requests):
    """JSON Lines in, one reply per request; a failed request is reported with its position."""
    done = run([binary], '\n'.join(json.dumps(r) for r in requests) + '\n')
    replies = [json.loads(line) for line in done.stdout.splitlines() if line.strip()]
    for index, reply in enumerate(replies):
        if not reply.get('ok'):
            raise SystemExit(f'request {index} ({requests[index].get("op")}) failed: {reply.get("error")}')
    if len(replies) != len(requests):
        raise SystemExit(f'{binary}: {len(replies)} replies for {len(requests)} requests (exit {done.returncode})')
    return replies


def ffmpeg(*args):
    return run(['ffmpeg', '-hide_banner', *args], timeout=300)


def ssim(frame, image):
    done = ffmpeg('-i', str(frame), '-i', str(image), '-lavfi',
                  '[0:v]format=rgb24[a];[1:v]format=rgb24[b];[a][b]ssim', '-f', 'null', '-')
    match = re.search(r'SSIM .*All:([0-9.]+)', done.stderr)
    if not match:
        raise SystemExit(f'ffmpeg ssim gave no score: {done.stderr[-300:]}')
    return float(match.group(1))


def means(path, grid):
    """Mean colour of the whole picture (grid 1) or of each cell of a grid x grid split, 0-255."""
    done = subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-i', str(path), '-vf',
                           f'format=rgb24,scale={grid}:{grid}:flags=area', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'],
                          capture_output=True, timeout=300)
    data = done.stdout
    if len(data) != grid * grid * 3:
        raise SystemExit(f'ffmpeg means: {len(data)} bytes for a {grid}x{grid} grid')
    return [list(data[i:i + 3]) for i in range(0, len(data), 3)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--ply', required=True)
    parser.add_argument('--image', required=True)
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--render', default=str(ROOT / 'target/debug/tessel-render'))
    parser.add_argument('--report')
    args = parser.parse_args()
    for path in (args.ply, args.image, args.headless, args.render):
        if not Path(path).is_file():
            raise SystemExit(f'missing: {path}')

    # 1. what the Rust reader says about the file
    head = [{'op': 'project.create', 'name': 'real', 'sequence': 'main', 'rate': 24, 'width': 64, 'height': 64,
             'length': 48},
            {'op': 'asset.add', 'kind': 'splat', 'id': 'shot', 'name': 'shot', 'path': args.ply}]
    capture = session(args.headless, head)[1]['result']['asset'].get('capture')
    if not capture:
        raise SystemExit('the Rust reader found no SHARP capture in the file')
    width, height = capture['image']
    print(f'capture: image {width}x{height} fx {capture["fx"]:.4f} fy {capture["fy"]:.4f} cx {capture["cx"]} '
          f'cy {capture["cy"]} vfov {capture["vfov"]:.4f} colour_space {capture["color_space"]} '
          f'declared {capture["color_space_declared"]} version {capture["version"]} '
          f'orientation {capture["orientation"]}')

    # 2 + 3. the real renderer at the source's size, with and without the capture
    scratch = Path(tempfile.mkdtemp(prefix='tessel-sharp-real-'))
    try:
        scores = {}
        for name, extra in (('capture', {}), ('ignore', {'capture': 'ignore'})):
            frame = scratch / f'{name}.png'
            requests = [
                {'op': 'project.create', 'name': 'real', 'sequence': 'main', 'rate': 24, 'width': width,
                 'height': height, 'length': 48},
                {'op': 'asset.add', 'kind': 'splat', 'id': 'shot', 'name': 'shot', 'path': args.ply, **extra},
                {'op': 'track.add', 'id': 't', 'name': 't', 'kind': 'video'},
                {'op': 'clip.add', 'id': 'c', 'track': 't', 'asset': 'shot', 'start': 0, 'duration': 48},
                {'op': 'render.frame', 'frame': 0, 'out': str(frame)},
            ]
            session(args.render, requests)
            scores[name] = {'ssim': ssim(frame, args.image), 'mean': means(frame, 1)[0], 'quadrants': means(frame, 2)}
            if name == 'capture':
                source_mean, source_quadrants = means(args.image, 1)[0], means(args.image, 2)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)

    drawn = scores['capture']
    mean_gap = max(abs(a - b) for a, b in zip(drawn['mean'], source_mean))
    quadrant_gap = max(abs(a - b) for cell, source in zip(drawn['quadrants'], source_quadrants)
                       for a, b in zip(cell, source))
    result = {
        'capture': capture, 'ssim_capture': drawn['ssim'], 'ssim_ignore_control': scores['ignore']['ssim'],
        'mean_render': drawn['mean'], 'mean_source': source_mean,
        'max_mean_gap': mean_gap, 'max_quadrant_gap': quadrant_gap,
        'tolerances': {'ssim_min': SSIM_MIN, 'mean': MEAN_TOLERANCE, 'control_margin': CONTROL_MARGIN},
    }
    print(json.dumps({k: v for k, v in result.items() if k != 'capture'}))
    failures = []
    if drawn['ssim'] < SSIM_MIN:
        failures.append(f'ssim {drawn["ssim"]:.4f} < {SSIM_MIN}')
    if mean_gap > MEAN_TOLERANCE or quadrant_gap > MEAN_TOLERANCE:
        failures.append(f'colour gap {mean_gap}/{quadrant_gap} > {MEAN_TOLERANCE}')
    if drawn['ssim'] - scores['ignore']['ssim'] < CONTROL_MARGIN:
        failures.append('the capture:ignore control is not distinguishable from the capture render')
    result['failures'] = failures
    if args.report:
        Path(args.report).write_text(json.dumps(result, indent=2))
    print('FAILED: ' + '; '.join(failures) if failures else 'PASS')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
