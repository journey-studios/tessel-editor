#!/usr/bin/env python3
"""BL141 — Portuguese titles, proved on two separate surfaces with the same scenario.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/accented-titles-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out DIR] [--report FILE]

* native window: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` CLI
  (`op`, `get`, `set`, `invoke`), then captured with the OS (`screencapture -l`): no pointer, no coordinates.
* headless (no window): `tessel-headless`, JSON Lines over stdio (planning, validation, persistence only: the
  frames of the same title come from `crates/render/tests/accented_titles.rs`, real runtime, no window).

The scenario: a title with accents spelled decomposed (base letter + combining mark) is planned and saved composed
(NFC); composed and decomposed edits are the same document; a character the font does not have (`€`, a newline,
a dangling mark) is refused with the document, revision and undo depth untouched; a project file written with a
decomposed title reopens composed; `set clip_text:<clip>` edits the same command. The native run also reads the
`clip_text:` and `inspector.text` nodes and draws the title in the monitor (`render.frame` PNG, white on black): the
accent rows lie above and the cedilla below the same body, the picture of `ação`, `AÇÃO` and the decomposed
spelling is byte-identical, and the OS capture shows the window.

Not proven here: a physical pointer/typing into the input (the GPUI tests drive the input entity; this script
uses the semantic `set`), the exported video (`crates/render/tests/accented_titles.rs`).
"""
import argparse
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parent))
from proof_surface import MIN_FREE_GIB, Headless, Native, ProofFailure, Recorder, free_gib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
PHRASE = 'Coração: não é só açaí, ÀS ÔNIBUS Ü'
DECOMPOSED = ('Corac\u0327a\u0303o: na\u0303o e\u0301 so\u0301 ac\u0327ai\u0301, '
              'A\u0300S O\u0302NIBUS U\u0308')


def read_png(path):
    """(width, height, rows of RGB tuples) of an 8-bit RGB/RGBA PNG, without a library."""
    data = Path(path).read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n', 'not a PNG'
    pos, header, idat = 8, None, b''
    while pos < len(data):
        length, kind = struct.unpack('>I4s', data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if kind == b'IHDR':
            header = struct.unpack('>IIBBBBB', body)
        elif kind == b'IDAT':
            idat += body
    width, height, depth, color, _, _, interlace = header
    assert depth == 8 and color in (2, 6) and interlace == 0, header
    step = 3 if color == 2 else 4
    raw, stride, rows, previous = zlib.decompress(idat), width * step, [], bytearray(width * step)
    for y in range(height):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - step] if i >= step else 0
            b, c = previous[i], previous[i - step] if i >= step else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        previous = line
    return width, height, rows, step


class Picture:
    """White-on-black title: lit pixels are glyph pixels."""

    def __init__(self, path):
        self.width, self.height, rows, step = read_png(path)
        self.lit = {(x, y) for y, row in enumerate(rows) for x in range(self.width) if row[x * step] > 64}
        self.digest = zlib.crc32(b''.join(rows))
        assert self.lit, f'{path}: the title drew nothing'
        xs, ys = [p[0] for p in self.lit], [p[1] for p in self.lit]
        self.box = (min(xs), max(xs), min(ys), max(ys))


def scenario(surface, check, scratch, fixtures):
    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    def refused(request, code):
        good, payload = surface.op(request)
        if good or payload.get('code') != code:
            raise ProofFailure(f'{request} should be refused with {code}, got {good} {payload}')

    def set_text(content, clip='c-title'):
        return {'op': 'clip.text', 'clip': clip, 'text': {'content': content, 'size': 14, 'color': '#ffffff'}}

    def planned():
        layers = ok({'op': 'plan', 'frame': 0})['layers']
        return next(layer for layer in layers if layer['kind'] == 'text')['text']['content']

    saves = [0]

    def document():
        saves[0] += 1
        path = scratch / f'doc-{surface.name}-{saves[0]}.tessel.json'
        ok({'op': 'project.save', 'path': str(path)})
        return json.loads(path.read_text()), path

    def title_in(doc):
        clips = doc['sequences'][0]['clips']
        return next(clip for clip in clips if clip['id'] == 'c-title')['text']['content']

    def history():
        data = ok({'op': 'history.status'})
        return data['history']['undo_depth'], data['project']['revision']

    for request in [json.loads(line) for line in (fixtures / 'proof.jsonl').read_text().splitlines() if line.strip()]:
        ok(request)
    check('the proof project loads', len(ok({'op': 'status'})['timeline']['clips']) == 5)

    # --- decomposed in, composed everywhere --------------------------------------------------------------
    check('the spellings really differ', DECOMPOSED != PHRASE and len(DECOMPOSED) > len(PHRASE))
    ok(set_text(DECOMPOSED))
    check('the plan hands the runtime the composed (NFC) title', planned() == PHRASE, planned())
    doc, path = document()
    check('the saved project holds the composed title', title_in(doc) == PHRASE, title_in(doc))
    depth, revision = history()
    ok(set_text(PHRASE))
    check('the same title spelled composed is not an edit (no revision, no undo entry)',
          history() == (depth, revision), history())

    # --- the file written by another tool, decomposed, reopens composed ---------------------------------------
    raw = path.read_text()
    assert PHRASE in raw
    other = scratch / f'nfd-{surface.name}.tessel.json'
    other.write_text(raw.replace(PHRASE, DECOMPOSED))
    ok(set_text('ACAO'))
    ok({'op': 'project.open', 'path': str(other)})
    check('a project file with a decomposed title reopens composed', planned() == PHRASE, planned())

    # --- refused text changes nothing --------------------------------------------------------------------------
    before_doc, before_history = document()[0], history()
    for bad, code in [('preço €5', 'unsupported_text'), ('日本', 'unsupported_text'),
                      ('duas\nlinhas', 'unsupported_text'), ('a\u0307', 'unsupported_text'),
                      ('\u0301', 'unsupported_text'), ('', 'invalid_value')]:
        refused(set_text(bad), code)
    check('refused texts leave the document, revision and undo depth untouched',
          document()[0] == before_doc and history() == before_history)
    refused(set_text('AÇÃO', clip='c-img'), 'not_applicable')

    # --- target vocabulary (the same command as the inspector input) -------------------------------------------
    # The window serves a node only for the selected title, so it is selected first (headless: same call).
    surface.invoke('clip:c-title', 'select')
    good, payload = surface.set('clip_text:c-title', 'content', 'Fe\u0301rias de Ve\u0302nus')
    check('`set clip_text:<clip> content` accepts decomposed accents', good and planned() == 'Férias de Vênus',
          (good, payload))
    good, payload = surface.set('clip_text:c-title', 'content', 'caf€')
    check('`set clip_text` refuses an unsupported character', not good and payload.get('code') == 'unsupported_text',
          payload)
    good, payload = surface.set('clip_text:c-title', 'font', 'serif')
    check('`set clip_text` refuses an unknown field', not good and payload.get('code') == 'unknown_field', payload)
    ok({'op': 'history.undo'})
    check('one undo returns to the previous title', planned() == PHRASE, planned())

    if surface.name != 'native':
        return

    # --- native window: nodes, monitor frames, OS capture -----------------------------------------------------------
    surface.invoke('clip:c-title', 'select')
    good, section = surface.get('inspector.text')
    check('the Texto section is a visible accordion for the selected title',
          good and section['visible'] and section['kind'] == 'accordion', section)
    good, field = surface.get('clip_text:c-title')
    check('the title text is a node with three fields, stored composed',
          good and len(field['fields']) == 3 and field['state']['content'] == PHRASE
          and field['state']['normalization'] == 'NFC', field)
    surface.invoke('clip:c-img', 'select')
    good, field = surface.get('clip_text:c-title')
    check('another kind of clip has no title text node', not good)
    surface.invoke('clip:c-title', 'select')

    # Only the title is drawn, on black.
    for track in ('scene-track', 'media-track'):
        ok_set, payload = surface.set(f'track:{track}', 'hidden', 'true')
        assert ok_set, payload
    size = {'width': 320, 'height': 180}

    def frame(name, content):
        ok(set_text(content))
        target = scratch / f'{name}.png'
        ok({'op': 'render.frame', 'frame': 0, 'out': str(target), **size})
        return Picture(target)

    plain, accented = frame('plain', 'ACAO'), frame('accented', 'AÇÃO')
    above, below = plain.box[2] - accented.box[2], accented.box[3] - plain.box[3]
    check('the width of AÇÃO is the width of ACAO', plain.box[:2] == accented.box[:2], (plain.box, accented.box))
    check('the tilde is above and the cedilla below the same body, evenly',
          above >= 4 and below >= 4 and abs(above - below) <= 2, (above, below))
    check('the accents are lit pixels', len(accented.lit) > len(plain.lit) + 8, (len(accented.lit), len(plain.lit)))
    check('lowercase draws as capitals', frame('lower', 'ação').digest == accented.digest)
    check('a decomposed spelling draws the same picture', frame('nfd', 'Ac\u0327a\u0303o').digest == accented.digest)
    check('acute is not tilde', frame('acute', 'AÇÁO').digest != accented.digest)

    # A title is one line and is not wrapped: this one fits the 320 px canvas at 14 %.
    ok(set_text('Não é só açaí'))
    for section in ('transform', 'composite', 'timing'):
        surface.set(f'inspector.{section}', 'expanded', 'false')
    surface.invoke('window.activate')
    path, reason = surface.capture(scratch / 'bl141-accented-title-window.png')
    check(f'the native window was captured ({path})' if path else f'native capture unavailable: {reason}', True)
    return path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out', default=str(ROOT / 'target/bl141-proof'))
    parser.add_argument('--report')
    args = parser.parse_args()
    if free_gib() < MIN_FREE_GIB:
        print(f'refusing to start: less than {MIN_FREE_GIB} GiB free', file=sys.stderr)
        return 2
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    fixtures = out / 'fixtures'
    shutil.rmtree(fixtures, ignore_errors=True)
    subprocess.run([str(ROOT / 'scripts/fixtures'), str(fixtures)], check=True, capture_output=True)
    report = {'surfaces': {}}
    failed = False
    for cls in (Headless, Native):
        home = tempfile.mkdtemp(prefix='tat') if cls is Native else None
        surface = cls(args.headless if cls is Headless else args.editor, home)
        check = Recorder(surface.name)
        shot = None
        try:
            surface.prepare()
            scratch = out / surface.name
            shutil.rmtree(scratch, ignore_errors=True)
            scratch.mkdir(parents=True)
            shot = scenario(surface, check, scratch, fixtures)
        except (ProofFailure, RuntimeError, AssertionError, StopIteration) as error:
            failed = True
            print(f'FAILED [{surface.name}]: {error!r}', file=sys.stderr)
        finally:
            surface.stop()
            if home:
                shutil.rmtree(home, ignore_errors=True)
        report['surfaces'][surface.name] = {'checks': check.checks, 'passed': sum(c['ok'] for c in check.checks),
                                            'screenshot': shot}
    print(json.dumps({name: f"{s['passed']}/{len(s['checks'])}" for name, s in report['surfaces'].items()}))
    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
