#!/usr/bin/env python3
"""BL168.1 — colour correction (brightness, contrast, saturation), proved on two separate surfaces.

    python3 scripts/grading-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out DIR] [--report FILE]

The values are set with the inspector's own command (`set property:<name> value <n>` on the selected
clip, what the slider sends), never with a pointer.

* headless (no window): `tessel-headless`. What the core decides: the plan hands the runtime
  `layers[].grading` with the values set, only for the graded clip, and drops it when all are 0 again.
  (The frames come from `crates/render/tests/grading.rs` and `runtime/test/grading-browser.test.mjs`.)
* native window: the DIRECT editor binary with its own socket, driven only by its `ui` CLI. The monitor's
  runtime draws the graded picture (`render.frame` PNG): each quadrant of the test image matches the
  documented formula within 2 levels, the black around the image stays black, all three back at 0 is the
  ungraded picture byte for byte, and the OS capture shows the window with the graded image.

Not proven here: a physical pointer on the slider (the GPUI tests drive the slider entity), the exported video
(`crates/render/tests/grading.rs`).
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))
from proof_surface import (MIN_FREE_GIB, Headless, Native, ProofFailure, free_gib, run_surface,  # noqa: E402
                           script_command)

_titles = importlib.util.spec_from_file_location('accented_titles_proof', ROOT / 'scripts/accented-titles-proof.py')
read_png = importlib.util.module_from_spec(_titles)
_titles.loader.exec_module(read_png)
read_png = read_png.read_png

SIZE = {'width': 320, 'height': 180}
# The quadrant image (`img`, 16:9) at zoom 50 fills the middle half of the 320×180 frame: x 80..240, y 45..135.
QUADRANTS = [(120, 67), (200, 67), (120, 112), (200, 112)]
OUTSIDE = (20, 20)
LUMA = (0.2126, 0.7152, 0.0722)


def grade(rgb, brightness=0.0, contrast=0.0, saturation=0.0):
    """The formula of `plan::Grading` (crates/core/src/plan.rs), on 8-bit channels."""
    clamp = lambda value: min(1.0, max(0.0, value))  # noqa: E731
    color = [clamp(c / 255 + brightness / 200) for c in rgb]
    color = [clamp((c - 0.5) * (1 + contrast / 100) + 0.5) for c in color]
    luma = sum(c * w for c, w in zip(color, LUMA))
    return [round(clamp(luma + (c - luma) * (1 + saturation / 100)) * 255) for c in color]


def pixel(picture, point):
    width, _height, rows, step = picture
    x, y = point
    return list(rows[y][x * step:x * step + 3])


def scenario(surface, check, scratch, fixtures):
    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    def image_layer():
        layers = ok({'op': 'plan', 'frame': 0})['layers']
        return next(layer for layer in layers if layer.get('clip') == 'c-img')

    def grading(brightness, contrast, saturation):
        for name, value in (('brightness', brightness), ('contrast', contrast), ('saturation', saturation)):
            good, payload = surface.set(f'property:{name}', 'value', value)
            if not good:
                raise ProofFailure(f'set property:{name} value {value} -> {payload}')

    for request in [json.loads(line) for line in (fixtures / 'proof.jsonl').read_text().splitlines() if line.strip()]:
        ok(request)
    good, payload = surface.invoke('clip:c-img', 'select')
    if not good:
        raise ProofFailure(f'select c-img -> {payload}')

    check('an untouched clip has no grading in the plan', 'grading' not in image_layer(), image_layer())
    grading(60, 0, 0)
    layer = image_layer()
    check('the slider command puts the values in the plan',
          layer.get('grading') == {'brightness': 60.0, 'contrast': 0.0, 'saturation': 0.0}, layer)
    others = [layer for layer in ok({'op': 'plan', 'frame': 30})['layers'] if 'grading' in layer]
    check('another clip is not graded', others == [], others)
    grading(0, 0, 0)
    check('all three back at 0: no grading in the plan', 'grading' not in image_layer(), image_layer())

    if surface.name != 'native':
        return None

    # --- native window: the monitor's runtime draws it ------------------------------------------------------------
    for track in ('scene-track', 'title-track'):
        good, payload = surface.set(f'track:{track}', 'hidden', 'true')
        if not good:
            raise ProofFailure(f'hide {track} -> {payload}')

    def frame(name):
        target = scratch / f'{name}.png'
        ok({'op': 'render.frame', 'frame': 0, 'out': str(target), **SIZE})
        return read_png(target), target.read_bytes()

    plain, plain_bytes = frame('plain')
    colours = [pixel(plain, point) for point in QUADRANTS]
    check('the quadrants are four different colours', len({tuple(c) for c in colours}) == 4, colours)
    for values in ((60, 0, 0), (-50, 0, 0), (0, 60, 0), (0, -100, 0), (0, 0, -100), (20, 30, -40)):
        grading(*values)
        graded, _ = frame('graded-' + '_'.join(str(v) for v in values))
        got = [pixel(graded, point) for point in QUADRANTS]
        want = [grade(colour, *values) for colour in colours]
        close = all(abs(g - w) <= 2 for gs, ws in zip(got, want) for g, w in zip(gs, ws))
        check(f'brightness/contrast/saturation {values}: the monitor draws the formula (±2)', close,
              {'got': got, 'want': want})
        check(f'{values}: the black around the image is not graded', pixel(graded, OUTSIDE) == [0, 0, 0],
              pixel(graded, OUTSIDE))
    grading(0, 0, 0)
    _, reset_bytes = frame('reset')
    check('back at 0 the monitor draws the ungraded picture byte for byte', reset_bytes == plain_bytes)

    good, node = surface.get('property:brightness')
    check('the brightness slider is a node of the inspector', good, node)
    grading(60, 0, -60)
    surface.invoke('window.activate')
    path, reason = surface.capture(scratch / 'bl168-graded-window.png')
    if path:
        check(f'the native window was captured ({path})', True)
    else:
        check.skip('the native window was captured', reason)
    return path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out', default=str(ROOT / 'target/bl168-grading-proof'))
    parser.add_argument('--report')
    args = parser.parse_args()
    if free_gib() < MIN_FREE_GIB:
        print(f'refusing to start: less than {MIN_FREE_GIB} GiB free', file=sys.stderr)
        return 2
    out = Path(args.out).resolve()
    fixtures = out / 'fixtures'
    report = {'surfaces': {}, 'error': None}
    try:
        out.mkdir(parents=True, exist_ok=True)
        shutil.rmtree(fixtures, ignore_errors=True)
        subprocess.run(script_command(ROOT / 'scripts/fixtures', fixtures), check=True, capture_output=True,
                       timeout=600)
    except (OSError, subprocess.SubprocessError) as error:
        print(f'FAILED fixtures: {type(error).__name__}: {error}', file=sys.stderr)
        return 1
    failed = complete = False
    complete = True
    for cls, binary in ((Headless, args.headless), (Native, args.editor)):
        result = run_surface(cls, binary, lambda surface, check, scratch: scenario(surface, check, scratch, fixtures),
                             out, 'tgp')
        report['surfaces'][result['name']] = result
        failed = failed or result['failed']
        complete = complete and result['complete']
    print(json.dumps({name: f"{s['passed']}/{len(s['checks'])} passed, {s['skipped']} skipped"
                      + (f", FAILED: {s['error']}" if s['failed'] else '')
                      for name, s in report['surfaces'].items()}))
    report['complete'] = complete and not failed
    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
