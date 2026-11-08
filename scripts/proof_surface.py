"""Shared pieces of the BL140–BL144 surface proofs (no editor logic lives here).

* `Headless`: `tessel-headless`, JSON Lines over stdio. No window, no socket. A window's socket is
  NEVER headless evidence; the proofs report the two surfaces separately.
* `Native`: the DIRECT editor binary started with its own short `HOME` (own socket, settings and
  recents: the user's window and project are never touched), driven only by that binary's `ui` CLI
  (`ui op|get|set|invoke`). No pointer, no coordinates.
* `Recorder`: named checks; a failed one stops the run with its detail; a `skip` is recorded as a
  SKIP, never as a pass, and is not counted as passed.
* `window_id` / `Native.capture`: the OS capture of the editor's own window (`screencapture -l`).
* `run_surface`: runs one scenario on one surface and NEVER raises: a surface that cannot start,
  answers garbage, hangs or crashes is a recorded failure, the cause is kept, cleanup problems are
  reported next to it (never replace it), and the caller can still try the other surface and write
  its report.

Every failure of the transport is a `SurfaceError` (a `RuntimeError`, so callers that already catch
`RuntimeError` keep working): end of file, non-JSON, a broken pipe or other `OSError`, a timeout,
a reply that is not an object. Replies are read with a deadline (a reader thread feeds a queue), so
a hung editor can no longer hang the proof.
"""
import json
import os
import queue
import shutil
import signal
import subprocess
import tempfile
import threading
import time

MIN_FREE_GIB = 12
REQUEST_TIMEOUT = 120      # seconds a headless reply or a `ui` call may take
CAPTURE_TIMEOUT = 60       # seconds for `swift` (cold start is slow) and `screencapture`


class ProofFailure(AssertionError):
    pass


class SurfaceError(RuntimeError):
    """The transport to a surface failed (not a failed check)."""


class Recorder:
    def __init__(self, surface):
        self.surface = surface
        self.checks = []

    def __call__(self, name, condition, detail=''):
        self.checks.append({'name': name, 'ok': bool(condition), 'status': 'pass' if condition else 'fail',
                            'detail': '' if condition else str(detail)[:500]})
        print(('PASS ' if condition else 'FAIL ') + f'[{self.surface}] ' + name
              + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise ProofFailure(f'[{self.surface}] {name}')

    def skip(self, name, reason):
        """A check that could not run. Not a pass: `ok` is False and `passed` never counts it."""
        self.checks.append({'name': name, 'ok': False, 'status': 'skip', 'detail': str(reason)[:500]})
        print(f'SKIP [{self.surface}] {name}  [{str(reason)[:300]}]', flush=True)

    @property
    def passed(self):
        return sum(1 for check in self.checks if check['status'] == 'pass')

    @property
    def skipped(self):
        return sum(1 for check in self.checks if check['status'] == 'skip')


def normalize(body):
    """`(ok, payload)` from either surface's envelope (`data`/`result`/`error`).

    A body that is not an object is a transport error. `payload` is never `None`: a success
    without data is `{}` and a failure without an error object is a typed `no_error_payload`, so
    callers can read `payload['code']`/`payload.get(...)` without a second guard.
    """
    if not isinstance(body, dict):
        raise SurfaceError(f'the reply is not a JSON object: {str(body)[:200]}')
    if body.get('ok'):
        payload = body.get('data', body.get('result'))
        return True, {} if payload is None else payload
    error = body.get('error')
    if not isinstance(error, dict):
        error = {'code': 'no_error_payload', 'message': f'failure without an error object: {str(body)[:200]}'}
    return False, error


def free_gib(path='/'):
    return shutil.disk_usage(path).free / 2 ** 30


def _stop_process(process, grace, group=False):
    """Ends `process` without raising; returns what went wrong (empty when nothing)."""
    problems = []
    if process is None:
        return problems
    try:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(grace)
            except subprocess.TimeoutExpired:
                problems.append(f'pid {process.pid} ignored terminate for {grace}s: killed')
                try:
                    if group:
                        os.killpg(process.pid, signal.SIGKILL)
                    else:
                        process.kill()
                except (OSError, ProcessLookupError) as error:
                    problems.append(f'kill failed: {error}')
                process.wait(10)
    except (OSError, subprocess.TimeoutExpired) as error:
        problems.append(f'cleanup of pid {process.pid} failed: {error}')
    return problems


class Headless:
    name = 'headless'
    needs_home = False

    def __init__(self, binary, _home=None, extra=(), timeout=REQUEST_TIMEOUT):
        self.timeout = timeout
        self.process = None
        self.cleanup_errors = []
        self._lines = queue.Queue()
        self._eof = False
        self.refused = 0          # replies with `ok: false`; `tessel-headless` then exits with status 1
        try:
            self.process = subprocess.Popen([binary, *extra], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=subprocess.DEVNULL, text=True, bufsize=1)
        except OSError as error:
            raise SurfaceError(f'cannot start `{binary}`: {error}') from error
        self._reader = threading.Thread(target=self._pump, daemon=True)
        self._reader.start()

    def _pump(self):
        try:
            for line in self.process.stdout:
                self._lines.put(line)
        except (OSError, ValueError):
            pass
        finally:
            self._lines.put(None)

    def _exit_detail(self):
        code = self.process.poll()
        return 'still running' if code is None else f'exit {code}'

    def raw(self, request):
        label = str(request.get('op', request)) if isinstance(request, dict) else str(request)
        if self._eof:
            raise SurfaceError(f'headless already ended ({self._exit_detail()}): `{label}` not sent')
        try:
            self.process.stdin.write(json.dumps(request) + '\n')
            self.process.stdin.flush()
        except (OSError, ValueError) as error:       # BrokenPipeError is an OSError
            raise SurfaceError(f'headless stdin is closed ({self._exit_detail()}) sending `{label}`: {error}') \
                from error
        try:
            line = self._lines.get(timeout=self.timeout)
        except queue.Empty:
            self._eof = True
            self.cleanup_errors += _stop_process(self.process, 0.1)
            raise SurfaceError(f'headless sent no reply to `{label}` within {self.timeout}s') from None
        if line is None:
            self._eof = True
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                pass
            raise SurfaceError(f'headless closed its output ({self._exit_detail()}) before replying to `{label}`')
        try:
            body = json.loads(line)
        except ValueError as error:
            raise SurfaceError(f'headless reply to `{label}` is not JSON: {line[:200]!r}') from error
        if not isinstance(body, dict):
            raise SurfaceError(f'headless reply to `{label}` is not an object: {line[:200]!r}')
        if not body.get('ok'):
            self.refused += 1
        return body

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
        """Ends the process; never raises (the cause of a failed run must not be replaced)."""
        problems = []
        try:
            self.process.stdin.close()
        except (OSError, ValueError) as error:
            problems.append(f'closing stdin: {error}')
        try:
            self.process.wait(30)
        except subprocess.TimeoutExpired:
            problems.append('headless did not exit within 30 s of end of input')
            problems += _stop_process(self.process, 5)
        except OSError as error:
            problems.append(f'waiting for headless: {error}')
        self.exit_code = self.process.poll()
        # `tessel-headless` answers every request and exits 1 when any of them failed: the scenarios
        # refuse edits on purpose, so that status is expected exactly when a refusal was seen.
        expected = 1 if self.refused else 0
        if self.exit_code not in (None, expected):
            problems.append(f"headless exited with status {self.exit_code} (expected {expected})")
        self._reader.join(2)
        for stream in (self.process.stdin, self.process.stdout):
            try:
                stream.close()
            except (OSError, ValueError):
                pass
        self.cleanup_errors += problems
        return problems


class Native:
    name = 'native'
    needs_home = True

    def __init__(self, binary, home):
        self.binary = binary
        self.env = {**os.environ, 'HOME': home, 'TESSEL_VIEWPORT_INSPECT': '1'}
        self.socket = os.path.join(home, '.cache/tessel-editor/ui.sock')
        self.home = home
        self.process = None
        self.log = None
        self.cleanup_errors = []
        self.log_path = os.path.join(home, 'editor.log')
        try:
            self.log = open(self.log_path, 'w')
            self.process = subprocess.Popen([binary], env=self.env, stdout=self.log, stderr=self.log,
                                            start_new_session=True)
        except OSError as error:
            if self.log is not None:
                self.log.close()
            raise SurfaceError(f'cannot start `{binary}`: {error}') from error

    def ui(self, *args, timeout=90):
        try:
            done = subprocess.run([self.binary, 'ui', *args], env=self.env, capture_output=True, text=True,
                                  timeout=timeout)
        except subprocess.TimeoutExpired as error:
            raise SurfaceError(f'ui {args}: no answer within {timeout}s') from error
        except OSError as error:
            raise SurfaceError(f'ui {args}: cannot run `{self.binary}`: {error}') from error
        try:
            body = json.loads(done.stdout)
        except ValueError as error:
            raise SurfaceError(f'ui {args}: no JSON ({done.returncode}): {done.stdout[:200]} '
                               f'{done.stderr[:200]}') from error
        if not isinstance(body, dict):
            raise SurfaceError(f'ui {args}: the reply is not a JSON object: {done.stdout[:200]}')
        return body

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
                try:
                    with open(self.log_path) as handle:
                        tail = handle.read()[-500:]
                except OSError as error:
                    tail = f'(log unreadable: {error})'
                raise SurfaceError(f'the editor exited ({self.process.returncode}): {tail}')
            if os.path.exists(self.socket):
                try:
                    if self.ui('status', timeout=10).get('ok'):
                        return
                except SurfaceError:
                    pass
            time.sleep(0.25)
        raise SurfaceError('the window never answered on its socket')

    def stop(self):
        """Ends the editor (and its session group when it ignores terminate); never raises."""
        problems = _stop_process(self.process, 15, group=True)
        try:
            if self.log is not None:
                self.log.close()
        except OSError as error:
            problems.append(f'closing the editor log: {error}')
        self.cleanup_errors += problems
        return problems

    def capture(self, path):
        """OS capture of this editor's window; `None` and a reason when the OS refuses."""
        try:
            self.invoke('window.activate')
        except SurfaceError as error:
            return None, f'could not activate the window: {error}'
        time.sleep(0.8)
        wid = window_id(self.process.pid)
        if not wid:
            return None, 'no on-screen window for this PID (screen locked, covered or capture denied)'
        try:
            run = subprocess.run(['screencapture', '-x', '-o', '-l', wid, str(path)], capture_output=True,
                                 text=True, timeout=CAPTURE_TIMEOUT)
        except (subprocess.TimeoutExpired, OSError) as error:
            return None, f'screencapture did not run: {error}'
        if run.returncode != 0 or not os.path.exists(path):
            return None, f'screencapture failed ({run.returncode}): {run.stderr.strip()[:120]}'
        return str(path), ''


def window_id(pid):
    """The on-screen window of `pid`: the largest layer-0 one, so a WebKit helper is skipped.

    `None` when there is none, when `swift` is missing or times out, or when its output is not a
    number (the caller reports a SKIP with a reason, never a pass).
    """
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
    try:
        out = subprocess.run(['swift', '-e', script], capture_output=True, text=True,
                             timeout=CAPTURE_TIMEOUT).stdout.strip()
    except (subprocess.TimeoutExpired, OSError):
        return None
    return out if out.isdigit() and out != '0' else None


def run_surface(cls, binary, scenario, out, home_prefix='tessel-proof', extra=()):
    """Runs `scenario(surface, check, scratch)` on one surface. Never raises.

    The surface is created, prepared and used inside the same `try`, so a binary that is missing,
    exits, hangs or answers garbage is a recorded failure. The first cause is kept in `error`; a
    problem while stopping the process goes to `cleanup_errors` and never replaces it. A SKIP
    (see `Recorder.skip`) is counted apart and makes `complete` false.
    """
    check = Recorder(cls.name)
    result = {'name': cls.name, 'checks': check.checks, 'passed': 0, 'skipped': 0, 'failed': False,
              'complete': False, 'error': None, 'screenshot': None, 'cleanup_errors': []}
    surface = None
    home = None
    try:
        home = tempfile.mkdtemp(prefix=home_prefix) if cls.needs_home else None
        surface = cls(binary, home, extra) if issubclass(cls, Headless) else cls(binary, home)
        surface.prepare()
        scratch = out / cls.name
        shutil.rmtree(scratch, ignore_errors=True)
        scratch.mkdir(parents=True)
        result['screenshot'] = scenario(surface, check, scratch)
    except ProofFailure as error:
        result['failed'] = True
        result['error'] = f'check failed: {error}'
    except Exception as error:      # noqa: BLE001 — a proof run must record any cause and go on
        result['failed'] = True
        result['error'] = f'{type(error).__name__}: {error}'
    finally:
        if surface is not None:
            try:
                result['cleanup_errors'] += surface.stop()
            except Exception as error:      # noqa: BLE001 — never mask the cause above
                result['cleanup_errors'].append(f'stop raised {type(error).__name__}: {error}')
        if home:
            shutil.rmtree(home, ignore_errors=True)
    if result['cleanup_errors'] and not result['failed']:
        result['failed'] = True
        result['error'] = '; '.join(result['cleanup_errors'])
    result['passed'] = check.passed
    result['skipped'] = check.skipped
    result['complete'] = not result['failed'] and check.skipped == 0
    if result['failed']:
        print(f"FAILED [{cls.name}]: {result['error']}", flush=True)
    for problem in result['cleanup_errors']:
        print(f'CLEANUP [{cls.name}]: {problem}', flush=True)
    return result
