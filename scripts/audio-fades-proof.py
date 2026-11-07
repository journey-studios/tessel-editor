#!/usr/bin/env python3
"""BL142 — audio clip fades, proved on two separate surfaces with the same scenario.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/audio-fades-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out DIR] [--report FILE]

* native window: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` CLI
  (`op`, `get`, `set`, `invoke`), then captured with the OS (`screencapture -l`): no pointer, no coordinates.
* headless (no window): `tessel-headless`, JSON Lines over stdio.

The scenario reads the PROCESSED signal (`render.audio` writes the offline mix as a WAV; this script decodes it
with the standard `wave` module, independent of the Rust mixer): the proof project's 440 Hz tone is mixed with no
fade, then with `fade_in` / `fade_out` set through `set property:fade_*` on the selected clip. Samples before the
ramp, after it and the half-way gain are compared with the plain mix sample for sample; the same is done after a
trim; fades longer than the clip are shortened to meet; a rejected property (`zoom` on an audio clip) leaves the
mix byte-identical; one undo returns the plain mix. The native run also reads `inspector.fades` and
`property:fade_*` from the live tree and captures the window.

Not proven here: a heard signal (`render.audio` is an offline mix, not a device), a physical pointer drag
(`crates/desktop/src/workstation/audio_fades_tests.rs` drives the Kit slider), the exported AAC file
(`crates/media/tests/export.rs`).
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import wave

sys.path.insert(0, str(Path(__file__).resolve().parent))
from proof_surface import MIN_FREE_GIB, Headless, Native, ProofFailure, Recorder, free_gib, run_surface  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
PER_FRAME = 2000  # 48 kHz at 24 fps
CLIP_FRAMES = 48


def read_wav(path):
    """Left channel of a 16-bit stereo WAV, as floats in -1..1."""
    with wave.open(str(path)) as handle:
        assert handle.getsampwidth() == 2 and handle.getnchannels() == 2, handle.getparams()
        raw = handle.readframes(handle.getnframes())
    count = len(raw) // 4
    return [int.from_bytes(raw[i * 4:i * 4 + 2], 'little', signed=True) / 32768 for i in range(count)]


def rms(samples, start, stop):
    window = samples[start:stop]
    return (sum(x * x for x in window) / len(window)) ** 0.5


def scenario(surface, check, scratch, fixtures):
    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    counter = [0]

    def mix():
        counter[0] += 1
        target = scratch / f'mix-{counter[0]}.wav'
        answer = ok({'op': 'render.audio', 'from': 0, 'frames': CLIP_FRAMES, 'out': str(target)})
        return read_wav(target), answer

    def fade(property_, frames):
        good, payload = surface.set(f'property:{property_}', 'value', frames)
        if not good:
            raise ProofFailure(f'set {property_}={frames} -> {payload}')
        return payload

    for request in [json.loads(line) for line in (fixtures / 'proof.jsonl').read_text().splitlines() if line.strip()]:
        ok(request)
    surface.invoke('clip:c-tone', 'select')
    plain, answer = mix()
    check('the tone is mixed with no fade: 96000 samples, full level at the start',
          len(plain) == 96_000 and answer['plan']['segments'][0]['fade_in'] == 0
          and answer['plan']['segments'][0]['fade_out'] == 0)
    check('the plain mix is the tone (RMS 0.25 = 0.5 * cos(pi/4) / sqrt(2))',
          abs(rms(plain, 1000, 90_000) - 0.25) < 0.01, rms(plain, 1000, 90_000))

    # --- fade in ----------------------------------------------------------------------------------------
    fade('fade_in', 12)
    faded, answer = mix()
    ramp = 12 * PER_FRAME
    segment = answer['plan']['segments'][0]
    check('the plan carries the fade in output samples (12 frames = 24000 samples)',
          segment['fade_in'] == ramp and segment['fade_out'] == 0, segment)
    plain_peak = max(abs(value) for value in plain[1:256])
    faded_peak = max(abs(value) for value in faded[1:256])
    check('nonzero early samples are attenuated (not just the sine zero crossing)',
          plain_peak > 0.05 and faded_peak <= plain_peak * 256 / ramp + 2 / 32768,
          (plain_peak, faded_peak))
    check('half way up the ramp the gain is one half (windowed RMS, 1000 samples)',
          abs(rms(faded, ramp // 2 - 500, ramp // 2 + 500) / rms(plain, ramp // 2 - 500, ramp // 2 + 500) - 0.5) < 0.03,
          rms(faded, ramp // 2 - 500, ramp // 2 + 500))
    check('after the ramp the mix is the plain mix, sample for sample',
          all(abs(a - b) < 1 / 16384 for a, b in zip(faded[ramp:], plain[ramp:])))
    check('the ramp has the RMS of a linear ramp (1/sqrt(3) of the plain RMS)',
          abs(rms(faded, 0, ramp) / rms(plain, 0, ramp) - 3 ** -0.5) < 0.05, rms(faded, 0, ramp))

    # --- fade out ---------------------------------------------------------------------------------------
    fade('fade_out', 12)
    both, answer = mix()
    total = CLIP_FRAMES * PER_FRAME
    check('the last sample is almost silence and the one before the ramp is at full level',
          abs(both[total - 1]) < 0.001 and abs(both[total - ramp - 1] - plain[total - ramp - 1]) < 1 / 16384,
          (both[total - 1], both[total - ramp - 1]))
    check('half way down the ramp the gain is one half',
          abs(rms(both, total - ramp // 2 - 500, total - ramp // 2 + 500)
              / rms(plain, total - ramp // 2 - 500, total - ramp // 2 + 500) - 0.5) < 0.03)

    # --- trim: the fades follow the clip's edges ------------------------------------------------------------
    good, payload = surface.set('trim:c-tone:end', 'frame', 36)
    check('the end of the clip is trimmed to frame 36 (`set trim:`)', good, payload)
    trimmed, answer = mix()
    end = 36 * PER_FRAME
    check('after trimming the end to frame 36 the ramp down ends at the new last sample, then silence',
          abs(trimmed[end - 1]) < 0.001 and all(x == 0.0 for x in trimmed[end:])
          and rms(trimmed, end - ramp - 1000, end - ramp) > 0.2, (trimmed[end - 1], rms(trimmed, end - ramp - 1000, end - ramp)))
    check('the clip has 72000 samples in the plan', answer['plan']['segments'][0]['clip_frames'] == end,
          answer['plan']['segments'][0])
    ok({'op': 'history.undo'})
    check('one undo gives back the trimmed-off tail (the fade of the whole clip)',
          abs(mix()[0][total - 1]) < 0.001)

    # --- overlap: shortened to meet ------------------------------------------------------------------------------
    fade('fade_in', 48)
    fade('fade_out', 48)
    meeting, answer = mix()
    segment = answer['plan']['segments'][0]
    check('48 + 48 frames over a 48-frame clip become 24 + 24 (they meet, no dip)',
          segment['fade_in'] == total // 2 and segment['fade_out'] == total // 2, segment)
    check('the triangle peaks at the plain level in the middle',
          abs(meeting[total // 2] - plain[total // 2]) < 0.001, (meeting[total // 2], plain[total // 2]))

    # --- a rejection leaves everything as it was --------------------------------------------------------------------
    surface.set('property:fade_in', 'value', 12)
    surface.set('property:fade_out', 'value', 12)
    again, _ = mix()
    good, payload = surface.set('property:zoom', 'value', 50)
    # The window answers a disabled node first (`disabled_element`); the core says why (`not_applicable`).
    check('`set property:zoom` on an audio clip is refused',
          not good and payload.get('code') in ('not_applicable', 'disabled_element'), payload)
    good, payload = surface.op({'op': 'clip.property', 'clip': 'c-tone', 'property': 'zoom', 'value': 50})
    check('`clip.property zoom` on an audio clip is refused by the core as not_applicable',
          not good and payload.get('code') == 'not_applicable', payload)
    check('the refusal changed nothing (same mix, sample for sample)', mix()[0] == again)
    good, payload = surface.set('property:fade_in', 'value', 'soon')
    check('a fade that is not a number is refused', not good and payload.get('code') == 'invalid_value', payload)
    check('a fade of 500 frames is clamped to the slider extreme, 48',
          fade('fade_in', 500)['value'] == 48.0)

    # --- the video clip's own fade stays visual ---------------------------------------------------------------------
    fade('fade_in', 0)
    fade('fade_out', 0)
    check('with both fades back at 0 the mix is the plain mix again', mix()[0] == plain)
    surface.invoke('clip:c-vid', 'select')
    good, payload = surface.set('property:fade_in', 'value', 12)
    check('a video clip still takes `fade_in` (an opacity ramp)', good, payload)
    check('and the audio mix did not change', mix()[0] == plain)
    surface.invoke('clip:c-tone', 'select')

    if surface.name != 'native':
        return None

    # --- native window: tree and capture ------------------------------------------------------------------------------
    fade('fade_in', 12)
    fade('fade_out', 12)
    surface.set('inspector.tabs', 'value', 'audio')
    good, section = surface.get('inspector.fades')
    check('the Fades section is a visible accordion under the Audio tab for an audio clip',
          good and section['visible'] and section['kind'] == 'accordion' and section['state']['tab'] == 'audio',
          section)
    good, node = surface.get('property:fade_in')
    check('`property:fade_in` is one slider node under inspector.fades, value 12 frames',
          good and node['parent'] == 'inspector.fades' and node['state']['value'] == 12
          and node['state']['unit'] == 'fr' and node['visible'], node)
    surface.invoke('clip:c-vid', 'select')
    good, node = surface.get('property:fade_in')
    check('on a video clip the same node moves under inspector.timing (Video tab)',
          good and node['parent'] == 'inspector.timing', node)
    good, section = surface.get('inspector.fades')
    check('and the Fades section is not applicable to it', good and not section['visible'], section)
    surface.invoke('clip:c-tone', 'select')
    surface.set('inspector.tabs', 'value', 'audio')
    surface.invoke('window.activate')
    path, reason = surface.capture(scratch / 'bl142-audio-fades-window.png')
    if path:
        check(f'the native window was captured ({path})', True)
    else:
        check.skip('the native window was captured', reason)
    return path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out', default=str(ROOT / 'target/bl142-proof'))
    parser.add_argument('--report')
    parser.add_argument('--require-capture', action='store_true',
                        help='a missing OS window capture is a failure (default: an explicit SKIP)')
    args = parser.parse_args()
    if free_gib() < MIN_FREE_GIB:
        print(f'refusing to start: less than {MIN_FREE_GIB} GiB free', file=sys.stderr)
        return 2
    report = {'surfaces': {}, 'error': None}

    def finish(code):
        if args.report:
            try:
                Path(args.report).write_text(json.dumps(report, indent=2))
            except OSError as error:
                print(f'could not write the report: {error}', file=sys.stderr)
                return code or 1
        return code

    out = Path(args.out)
    fixtures = out / 'fixtures'
    try:
        out.mkdir(parents=True, exist_ok=True)
        shutil.rmtree(fixtures, ignore_errors=True)
        subprocess.run([str(ROOT / 'scripts/fixtures'), str(fixtures)], check=True, capture_output=True,
                       timeout=600)
    except (OSError, subprocess.SubprocessError) as error:
        report['error'] = f'fixtures: {type(error).__name__}: {error}'
        print(f"FAILED {report['error']}", file=sys.stderr)
        return finish(1)
    failed = False
    complete = True
    # One surface failing never stops the other, and the report is written either way.
    for cls, binary in ((Headless, args.headless), (Native, args.editor)):
        result = run_surface(cls, binary, lambda surface, check, scratch: scenario(surface, check, scratch,
                                                                                  fixtures), out, 'taf')
        report['surfaces'][result['name']] = result
        failed = failed or result['failed']
        complete = complete and result['complete']
    print(json.dumps({name: f"{s['passed']}/{len(s['checks'])} passed, {s['skipped']} skipped"
                      + (f", FAILED: {s['error']}" if s['failed'] else '')
                      for name, s in report['surfaces'].items()}))
    if not failed and not complete:
        print('INCOMPLETE: at least one check was skipped (see the SKIP lines); this is not a full pass',
              file=sys.stderr)
        if args.require_capture:
            failed = True
    report['complete'] = complete and not failed
    return finish(1 if failed else 0)


if __name__ == '__main__':
    sys.exit(main())
