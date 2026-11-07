"""Shared pieces of the BL140–BL144 surface proofs (no editor logic lives here).

* `Headless`: `tessel-headless`, JSON Lines over stdio. No window, no socket. A window's socket is
  NEVER headless evidence; the proofs report the two surfaces separately.
* `Native`: the DIRECT editor binary started with its own short `HOME` (own socket, settings and
  recents: the user's window and project are never touched), driven only by that binary's `ui` CLI
  (`ui op|get|set|invoke`). No pointer, no coordinates.
* `Recorder`: named checks; a failed one stops the run with its detail.
* `window_id` / `capture`: the OS capture of the editor's own window (`screencapture -l`).
"""
import json
import os
import shutil
import subprocess
import time

MIN_FREE_GIB = 12


class ProofFailure(AssertionError):
    pass


class Recorder:
    def __init__(self, surface):
        self.surface = surface
        self.checks = []

    def __call__(self, name, condition, detail=''):
        self.checks.append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:500]})
        print(('PASS ' if condition else 'FAIL ') + f'[{self.surface}] ' + name
              + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise ProofFailure(f'[{self.surface}] {name}')


def normalize(body):
    """`(ok, payload)` from either surface's envelope (`data`/`result`/`error`)."""
    if body.get('ok'):
        return True, body.get('data', body.get('result'))
    return False, body.get('error')


def free_gib(path='/'):
    return shutil.disk_usage(path).free / 2 ** 30


class Headless:
    name = 'headless'

    def __init__(self, binary, _home=None, extra=()):
        self.process = subprocess.Popen([binary, *extra], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True, bufsize=1)

    def raw(self, request):
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def op(self, request):
        return normalize(self.raw(request))

    def get(self, target):
        return False, {'code': 'desktop_required', 'message': 'no tree without a window'}

    def set(self, target, field, value):
        return normalize(self.raw({'op': 'set', 'target': target, 'field': field, 'value': value}))

    def invoke(self, target, action=None):
        request = {'op': 'invoke', 'target': target}
        if action:
            request['action'] = action
        return normalize(self.raw(request))

    def move(self, target, frame):
        return normalize(self.raw({'op': 'move', 'target': target, 'args': {'frame': frame}}))

    def prepare(self):
        pass

    def stop(self):
        self.process.stdin.close()
        self.process.wait(30)


class Native:
    name = 'native'

    def __init__(self, binary, home):
        self.binary = binary
        self.env = {**os.environ, 'HOME': home}
        self.socket = os.path.join(home, '.cache/tessel-editor/ui.sock')
        self.home = home
        self.log_path = os.path.join(home, 'editor.log')
        self.log = open(self.log_path, 'w')
        self.process = subprocess.Popen([binary], env=self.env, stdout=self.log, stderr=self.log,
                                        start_new_session=True)

    def ui(self, *args, timeout=90):
        done = subprocess.run([self.binary, 'ui', *args], env=self.env, capture_output=True, text=True,
                              timeout=timeout)
        try:
            return json.loads(done.stdout)
        except ValueError:
            raise RuntimeError(f'ui {args}: no JSON ({done.returncode}): {done.stdout[:200]} {done.stderr[:200]}')

    def op(self, request):
        return normalize(self.ui('op', json.dumps(request)))

    def get(self, target):
        return normalize(self.ui('get', target))

    def set(self, target, field, value):
        return normalize(self.ui('set', target, field, str(value)))

    def invoke(self, target, action=None):
        args = ['invoke', target] + ([action] if action else [])
        return normalize(self.ui(*args))

    def move(self, target, frame):
        return normalize(self.ui('move', target, '--frame', str(frame)))

    def prepare(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                with open(self.log_path) as handle:
                    raise RuntimeError(f'the editor exited ({self.process.returncode}): {handle.read()[-500:]}')
            if os.path.exists(self.socket):
                try:
                    if self.ui('status', timeout=10).get('ok'):
                        return
                except (RuntimeError, subprocess.TimeoutExpired):
                    pass
            time.sleep(0.25)
        raise RuntimeError('the window never answered on its socket')

    def stop(self):
        self.process.terminate()
        try:
            self.process.wait(15)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.log.close()

    def capture(self, path):
        """OS capture of this editor's window; `None` and a reason when the OS refuses."""
        self.invoke('window.activate')
        time.sleep(0.8)
        wid = window_id(self.process.pid)
        if not wid:
            return None, 'no on-screen window for this PID (screen locked, covered or capture denied)'
        run = subprocess.run(['screencapture', '-x', '-o', '-l', wid, str(path)], capture_output=True, text=True)
        if run.returncode != 0 or not os.path.exists(path):
            return None, f'screencapture failed ({run.returncode}): {run.stderr.strip()[:120]}'
        return str(path), ''


def window_id(pid):
    """The on-screen window of `pid`: the largest layer-0 one, so a WebKit helper is skipped."""
    script = (
        'import CoreGraphics\n'
        'let l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n'
        'var best = 0; var area = 0.0\n'
        'for w in l {\n'
        f'  if (w[kCGWindowOwnerPID as String] as? Int) == {pid} && (w[kCGWindowLayer as String] as? Int) == 0 {{\n'
        '    let b = w[kCGWindowBounds as String] as! [String: Double]\n'
        '    let a = (b["Width"] ?? 0) * (b["Height"] ?? 0)\n'
        '    if a > area { area = a; best = w[kCGWindowNumber as String] as! Int }\n'
        '  }\n'
        '}\n'
        'print(best)\n'
    )
    out = subprocess.run(['swift', '-e', script], capture_output=True, text=True).stdout.strip()
    return out if out and out != '0' else None
