#!/usr/bin/env python3
"""Check slider gesture commands in a private native window; no pointer coordinates.

Uses the direct binary with its own HOME/socket. Never edits the user's running project.
GPUI pointer dispatch has separate tests; this proves live widget state and window IPC.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('video_proof', ROOT / 'scripts/video-live-proof.py')
video = importlib.util.module_from_spec(spec)
spec.loader.exec_module(video)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default=str(ROOT / 'target/release/tessel-editor'))
    parser.add_argument('--out')
    args = parser.parse_args()
    report = {'checks': [], 'input': 'semantic IPC, not physical pointer'}
    with tempfile.TemporaryDirectory(prefix='tsg-', dir='/tmp') as home:
        lab = video.Editor(str(Path(args.binary).resolve()), home)
        def send(request):
            response = lab.send({'version': 1, **request})
            if not response.get('ok'):
                raise AssertionError(response)
            return response['data']
        def op(request):
            return send({'command': 'op', 'request': request})
        def depth():
            return op({'op': 'history.status'})['history']['undo_depth']
        def value():
            return send({'command': 'get', 'target': 'property:zoom'})['state']['value']
        def check(name, condition):
            report['checks'].append({'name': name, 'ok': bool(condition)})
            if not condition:
                raise AssertionError(name)
        try:
            lab.wait_ready(30)
            send({'command': 'invoke', 'target': 'clip:atlantic-coast', 'action': 'select'})
            before, start = value(), depth()
            op({'op': 'property.gesture.begin', 'property': 'zoom'})
            for index, next_value in enumerate([120, 130, 140]):
                op({'op': 'property.gesture.update', 'value': next_value})
                check(f'preview {index} reaches the live widget', value() == next_value)
                check(f'preview {index} adds no undo', depth() == start)
            op({'op': 'property.gesture.commit'})
            check('release records exactly one entry', depth() == start + 1)
            op({'op': 'history.undo'})
            check('undo restores the original widget value', value() == before)
            op({'op': 'history.redo'})
            check('redo restores the final widget value', value() == 140)
            settled = depth()
            op({'op': 'property.gesture.begin', 'property': 'zoom'})
            op({'op': 'property.gesture.update', 'value': 160})
            op({'op': 'property.gesture.cancel'})
            check('cancel restores the original value', value() == 140)
            check('cancel adds no undo', depth() == settled)
            op({'op': 'property.gesture.begin', 'property': 'zoom'})
            op({'op': 'property.gesture.update', 'value': 150})
            op({'op': 'property.gesture.update', 'value': 140})
            op({'op': 'property.gesture.commit'})
            check('returning to the original adds no entry', depth() == settled)
            report['ok'] = True
        except Exception as error:
            report['ok'] = False
            report['error'] = repr(error)
        finally:
            try:
                lab.stop()
            finally:
                lab.log.close()
                if args.out:
                    Path(args.out).write_text(json.dumps(report, indent=2) + '\n')
                print(json.dumps(report))
    return 0 if report.get('ok') else 1


if __name__ == '__main__':
    raise SystemExit(main())
