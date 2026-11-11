#!/usr/bin/env python3
"""The shared gizmo journey in the REAL window (WKWebView), from a process the runner starts itself (F055 / BL015.21).

    python3 scripts/gizmo-perf/native.py --phase build [--out DIR]
    python3 scripts/gizmo-perf/native.py --phase run   [--out DIR] [--corpora small dense deep] [--functional-only]

PROVENANCE. `build` builds the HEAD release editor through the repository's cargo wrapper (default target, finite
timeout, clean tree only) and writes `build.json`: HEAD, tree, whether the tree was dirty (`--allow-dirty`), the editor's
sha256, the runtime page's sha256 and source tree id (a build that cannot read it is refused), the corpus manifest and
journey hashes. `run` refuses (`PROVENANCE_REFUSED`, exit 1) unless the editor it is about to start IS that binary
(sha256), the tree is HEAD's and clean, the build was clean too (a record that says dirty, or does not say it was clean,
is refused: stashing the changes does not turn that binary into HEAD's), `runtime/dist` is the recorded one and newer than
the runtime sources, and every file the corpus manifest hashes is what it says. A number from another head, another build
or another corpus cannot come out of this runner.

THE FLOW. For each corpus: (1) a throwaway process of the SAME binary authors the corpus project (`project.create`,
`scene.attach`, `project.save` into its own HOME) and exits; (2) a FRESH process (own HOME and socket, the repository's
runtime page) is started and the journey's first step opens that SAVED project with the real `project.open`: not
`project.create` posing as an opening. The same `docs/migration/fixtures/gizmo-perf/journey.json` the core and the GPUI
host run is then replayed through the window's socket with semantic requests only (`op`, `get`, `set`, `invoke`): no
coordinate clicks, no key events, the user's own editor is never addressed. A reopened project starts with a clean history
(core contract), so the golden trace's undo depths are compared with that one offset, recorded.

A NEW PICTURE. A picture counts only when the monitor reports (`monitor.viewport`) that: the phase is `ready`, nothing is
in flight, the directed scene is the requested one, the picture it shows (`directed.shown.request_id`) is the very picture
it read back (`last_frame.request`), it is for the requested edit frame, and its request id is STRICTLY newer than the
one observed right before the command. A reply on the socket, the page's `frame.done`, a cached value or the previous
scene's picture can therefore never satisfy a wait. The checks are on the picture's pixel hash (read back by the runtime):
a commit must change it, an undo must restore the exact earlier hash, a switch must show the new scene.

CANCEL. Before the release the candidate is proven different from the base (the move reply's own candidate value and
`would_change`) and must have been DRAWN (a picture newer than the press, within 2 s: a candidate that differs from the
base and never got a picture fails its check, it is not just recorded) with a hash that differs from the base picture's;
after the cancel the stored transform, the undo/redo depths and the picture hash are the base's, with a new picture.

WHAT IT MEASURES (all raw samples are kept, nothing is trimmed): `client_ms` per request (socket round trip: core + host +
IPC, NOT the host alone); the editor's own report of each picture (`render_ms`, `total_ms`, ... and the hash); the host's
session trace spans; the cold open as THREE separate numbers (`launch_to_socket_ms`, the `project.open` request, and
`monitor.show_scene` to its first new picture) kept apart from the hot gestures; the overlay's own acknowledgement.

WHAT IT DOES NOT PROVE (`limits`): the compositor/GPU scan-out is not observable (directed-scene pictures carry no
presented-frame stamp: `presented` is reported as not observable, never as a pass); the OS capture is a witness, not an
assertion; no physical pointer or keyboard (BL015.2/BL043/BL044 stay pending); no splat case exists; nothing is budgeted.

The performance lock is taken ONCE, without waiting, for the whole run; busy or a failed guard (load 1/5 min, thermal,
disk below the heavy-start reserve) is BLOCKED (exit 3); the guards are read again after the run and a dirty reading
makes it INVALID (exit 3), and a run that FAILED anyway keeps the status FAIL and carries that guard text in its reason.
Every wait and the whole corpus are finite. Exit 0 only when every functional check passed; 1 on a failure or a refusal;
3 BLOCKED/INVALID.

OWNERSHIP. `--out/.native.lock` is held by whichever phase runs over that directory, and `build` also holds the lock of the
CHECKOUT (`harness.checkout_lock_path`, one file under /tmp per checkout) before it reads or writes anything: no other
run may build from the checkout, or put a mutant in its tree, meanwhile. Neither lock waits: a taken one is ONE `REFUSED`
line (exit 1) and nothing is written, the refusal's own manifest included.

GIT. A checkout git cannot name (no head, no tree, a failing or slow git) is REFUSED before anything is built or started,
`--allow-dirty` included: it is not a dirty tree (`harness.git_identity` is strict for decisions; `harness.out` stays
tolerant for the telemetry of a report). A runtime whose sources git cannot list is not "fresh" either.

THE ENVIRONMENT. The editor (and the build) never inherits a `TESSEL_GIZMO_PERF_*` switch from the caller, and the build
no `CARGO_TARGET_DIR`/`TESSEL_CARGO_LANE` (`harness.child_env`); `os.environ` itself is left as it was.

THE ACKNOWLEDGEMENT. The overlay acknowledgement kept for a step must be THAT step's: the page keeps the last one it
matched until a newer arrives, so each overlay step polls (`ACK_SECONDS`) until the acknowledged tool, space, selection,
scene, revision and frame are the ones of the step's own request and answer. A stale or missing acknowledgement is a
failing check that shows both sides, never a recorded value. A step whose overlay is hidden has none to wait for and is
not checked (BL015.23, item 23). A `render.status` that did not answer ok is no trace: it fails its own checks.
"""
import argparse
import json
from pathlib import Path
import shutil
import socket
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))
import harness  # noqa: E402
import proof_surface  # noqa: E402

ROOT = harness.ROOT
FIXTURES = ROOT / 'docs/migration/fixtures/gizmo-perf'
TOLERANCE = 1e-9
CORPUS_TIMEOUT_S = 1200
OUT_LOCK = '.native.lock'  # in --out: held by whichever phase owns that directory
LIMITS = [
    "the compositor / GPU scan-out is not observable; directed-scene pictures carry no presented-frame stamp (the session trace's frames_total is 0): `presented` is NOT OBSERVABLE here, so core->host->runtime is measured, the last hop is not",
    "a socket reply and the page's frame.done are not proof of painting; the picture hash is the runtime read-back and a picture only counts when its request id is newer than the one before the command",
    "the OS capture is observational (one window witness per corpus), never an assertion",
    "no physical pointer, keyboard or popover focus is used: BL015.2, BL043, BL044 remain pending",
    "client_ms includes the socket round trip and the client process: it is not the host cost",
    "no splat case: the synthetic corpus has no splat asset (BL015.23)",
    "observations of one machine/backend; nothing is budgeted and no A/B against a baseline native binary exists yet",
]


class Timeout(Exception):
    """A finite wait ran out (a check, never a hang)."""


def near(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= TOLERANCE
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(near(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(near(a[k], b[k]) for k in a)
    return a == b


def fill(value, variables):
    if isinstance(value, str) and value.startswith('$'):
        return variables[value[1:]]
    if isinstance(value, list):
        return [fill(item, variables) for item in value]
    if isinstance(value, dict):
        return {key: fill(item, variables) for key, item in value.items()}
    return value


def pct(values, q):
    ordered = sorted(values)
    return ordered[max(0, min(len(ordered) - 1, int(-(-q * len(ordered) // 1)) - 1))] if ordered else None


def summary(values):
    return {'n': len(values), 'min': min(values), 'p50': pct(values, 0.5), 'p95': pct(values, 0.95),
            'p99': pct(values, 0.99), 'max': max(values)} if values else {'n': 0}


# --- what the monitor says about a picture (pure: testable without a window) --------------------------------------


def picture(state):
    """The facts that identify the picture the monitor shows."""
    directed = state.get('directed') or {}
    shown = directed.get('shown') or {}
    last = state.get('last_frame') or {}
    return {'scene': directed.get('scene'), 'request': shown.get('request_id'), 'edit_frame': shown.get('edit_frame'),
            'frame_request': last.get('request'), 'revision': last.get('revision'), 'hash': last.get('hash'),
            'presented': 'not_observable'}


def mark(state):
    """The id of the newest picture shown now: what a later picture must be strictly newer than."""
    request = picture(state)['request']
    return request if isinstance(request, int) and not isinstance(request, bool) else -1


def new_picture(state, scene, frame, after):
    """`(done, info)`: is the picture the monitor shows now the NEW picture of `scene` at `frame`?"""
    directed = state.get('directed') or {}
    info = picture(state)
    request = info['request']
    done = (state.get('phase') == 'ready' and directed.get('scene') == scene and directed.get('active') is None
            and directed.get('queued') is None and not directed.get('suspended')
            and not str(state.get('headline', '')).startswith('Falha')
            and isinstance(request, int) and not isinstance(request, bool)
            and info['frame_request'] == request and info['hash'] is not None
            and (frame is None or info['edit_frame'] == frame)
            and (after is None or request > after))
    return done, info


ACK_SECONDS = 5  # how long a step waits for the page to acknowledge ITS overlay (the wait was always 5 s)


def ack_expectation(request, overlay, tool=None):
    """What the page's acknowledgement (`monitor.viewport` -> `overlay.drawn`) must say to be the acknowledgement of THIS
    step's overlay. It is built from the step's own request (the scene, the tool and space it set) and from the core's
    own answer to it (`overlay`: tool, space, selected entity, stamp), never from an earlier acknowledgement: the host
    keeps the last acknowledgement it matched until a newer one arrives, so reading it at once returns the PREVIOUS step's
    (the first real measurement recorded the rotate/world overlay of revision 8 under `state.scale.local`)."""
    stamp = overlay.get('stamp') if isinstance(overlay.get('stamp'), dict) else {}
    tool_name = (tool or {}).get('tool') or overlay.get('tool')
    space = (tool or {}).get('space') or overlay.get('space')
    return {'visible': overlay.get('visible'), 'tool': tool_name, 'space': space, 'entity': overlay.get('entity'),
            'stamp.scene': request.get('scene'), 'stamp.tool': tool_name, 'stamp.space': space,
            'stamp.revision': stamp.get('revision'), 'stamp.frame': stamp.get('frame', request.get('frame'))}


def ack_mismatch(expected, ack):
    """What differs between the page's acknowledgement `ack` and `expected` (`ack_expectation`): `[]` means `ack` IS the
    acknowledgement of this step. No acknowledgement at all (`overlay.drawn` null, or not an object) is a mismatch of its
    own, never a pass: a step whose overlay the page never confirmed has nothing to show for it."""
    if not isinstance(ack, dict):
        return [f'no acknowledgement (overlay.drawn is {ack!r})']
    stamp = ack.get('stamp') if isinstance(ack.get('stamp'), dict) else {}
    seen = {'visible': ack.get('visible'), 'tool': ack.get('tool'), 'space': ack.get('space'),
            'entity': ack.get('entity'), 'stamp.scene': stamp.get('scene'), 'stamp.tool': stamp.get('tool'),
            'stamp.space': stamp.get('space'), 'stamp.revision': stamp.get('revision'), 'stamp.frame': stamp.get('frame')}
    problems = [f'{key}: expected {want!r}, got {seen[key]!r}' for key, want in expected.items()
                if want is not None and seen[key] != want]
    entity = expected.get('entity')
    selection = stamp.get('selection')
    if entity is not None and not (isinstance(selection, list) and entity in selection):
        problems.append(f'stamp.selection: expected it to hold {entity!r}, got {selection!r}')
    return problems


def overlay_ack(window, request, overlay, tool=None, seconds=None):
    """`(ack, problems, expected)`: polls the page's acknowledgement until it belongs to this step (`ack_mismatch` is
    empty) or `seconds` (`ACK_SECONDS`) pass. On expiry `ack` is the LAST one seen (or None) and `problems` says what
    differs from `expected`, so a stale acknowledgement is a failing check that shows both sides, not a recorded value."""
    seconds = ACK_SECONDS if seconds is None else seconds
    expected = ack_expectation(request, overlay, tool)
    deadline = time.monotonic() + seconds
    while True:
        ack = (window.viewport().get('overlay') or {}).get('drawn')
        problems = ack_mismatch(expected, ack)
        if not problems or time.monotonic() >= deadline:
            return ack, problems, expected
        time.sleep(0.02)


def candidate_differs(reply, base):
    """Is the candidate a drag reply reports different from the stored transform `base` (`[position, rotation,
    scale]`)? Uses the reply's own candidate value and `would_change`, never the picture."""
    if not isinstance(reply, dict) or reply.get('applied') is not True:
        return False
    if reply.get('would_change') is not True:
        return False
    for key, at in (('position', 0), ('rotation', 1), ('scale', 2)):
        if key in reply:
            return not near(reply[key], base[at])
    return bool(reply.get('changes'))


def history_offset(want_undo, got_undo):
    """A reopened project starts with a clean history: the golden's undo depths are compared with this offset."""
    return want_undo - got_undo


# --- provenance ------------------------------------------------------------------------------------------------


def runtime_source_tree():
    """The tree id of `HEAD:runtime`, or `None` when git cannot say. `git rev-parse` prints what it was given even
    when it fails (and `harness.out` turns a failure into a text), so stdout alone is not an answer: a non-zero exit, a
    timeout, a command that cannot start or an empty output is `None`, never a value two records could agree on."""
    done = harness.run_bounded(['git', 'rev-parse', 'HEAD:runtime'], timeout=30)
    tree = done.stdout.strip()
    if done.timed_out or done.error or done.returncode != 0 or not tree:
        return None
    return tree


def runtime_stale_reason(root=ROOT):
    """`None` when `runtime/dist` is newer than every tracked runtime source, else why not. A git that cannot list the
    sources is NOT "no source is newer": the empty list a failing git prints used to read as a runtime that is not stale."""
    dist = root / 'runtime/dist/runtime.js'
    if not dist.exists():
        return 'runtime/dist/runtime.js is missing: run `./scripts/runtime setup`'
    ok, listing = harness.git_strict(['ls-files', 'runtime/src', 'runtime/host', 'runtime/worker', 'runtime/package.json',
                                      'runtime/pnpm-lock.yaml'], root)
    if not ok:
        return f'git cannot list the runtime sources, so their age cannot be compared with runtime/dist: {listing}'
    times = [(root / p).stat().st_mtime for p in listing.split() if (root / p).exists()]
    if not times:
        return 'git lists no runtime source that exists in the tree: the age of runtime/dist cannot be compared with anything'
    if max(times) > dist.stat().st_mtime:
        return 'runtime/dist is older than a runtime source file: run `./scripts/runtime build`'
    return None


def verify_corpus(manifest=None, fixtures=FIXTURES, root=ROOT):
    """`None` when every file the corpus manifest hashes (and the journey) is what it says, else why not."""
    manifest = manifest or json.loads((fixtures / 'corpus.manifest.json').read_text())
    for name, entry in manifest['files'].items():
        path = fixtures / name
        if not path.exists() or harness.sha256(path) != entry['sha256']:
            return f'{name} is not the generated bundle the corpus manifest hashes'
    if harness.sha256(fixtures / 'journey.json') != manifest['journey']['sha256']:
        return 'journey.json changed after the corpus manifest was written'
    return None


def verify_provenance(record, binary, identity, allow_dirty=False, root=ROOT, fixtures=FIXTURES):
    """`None` when `binary` is the build `record` describes for the current HEAD, else why not."""
    # An identity git could not name matches nothing: refused here as well as in `main`, so no caller can reach the
    # comparison below with two empty answers that would read as "the same head".
    problem = harness.identity_problem(identity)
    if problem:
        return problem
    if record is None:
        return 'no build record: run `--phase build` for this HEAD first'
    if identity['dirty'] and not allow_dirty:
        return 'the working tree is dirty: the binary cannot belong to HEAD'
    # The tree can read clean again after a `--phase build --allow-dirty` (the changes stashed): the record keeps what the
    # BUILD saw, and one that says `dirty` or does not say it was clean is not the build of HEAD.
    if record.get('dirty', True) and not allow_dirty:
        return 'the binary was built from a dirty tree (or its build record does not say it was clean): it cannot belong to HEAD'
    if not harness.same_head(record, identity):
        return f"the binary was built for {str(record.get('head'))[:9]}, HEAD is {identity['head'][:9]}"
    if harness.sha256(binary) != record['binary_sha256']:
        return 'the editor binary is not the one the build phase recorded (sha256 differs)'
    dist = root / 'runtime/dist/runtime.js'
    if not dist.exists() or harness.sha256(dist) != record['runtime_sha256']:
        return 'runtime/dist/runtime.js is not the one recorded at build time'
    runtime_tree = runtime_source_tree()
    if runtime_tree is None:
        return 'git cannot tell the tree id of HEAD:runtime: the runtime sources cannot be compared with the recorded ones'
    if record.get('runtime_source_tree') != runtime_tree:
        return 'the runtime sources differ from the ones the build recorded'
    stale = runtime_stale_reason(root)
    if stale:
        return stale
    if record['corpus_manifest_sha256'] != harness.sha256(fixtures / 'corpus.manifest.json'):
        return 'the corpus manifest is not the one recorded at build time'
    return verify_corpus(fixtures=fixtures, root=root)


def build_phase(args, manifest, manifest_path, out):
    problem = harness.identity_problem(manifest['identity'])
    if problem:  # a checkout git cannot name: no binary is built for it, `--allow-dirty` or not
        manifest.update(status='REFUSED', reason=problem)
        return harness.die(1, manifest, manifest_path)
    if manifest['identity']['dirty'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: the binary would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    reason = runtime_stale_reason() or verify_corpus()
    runtime_tree = runtime_source_tree()
    if not reason and runtime_tree is None:
        reason = 'git cannot tell the tree id of HEAD:runtime: the build would record no runtime source tree'
    if reason:
        manifest.update(status='PREREQUISITE_MISSING', reason=reason)
        return harness.die(1, manifest, manifest_path)
    command = ['./scripts/cargo', 'build', '--release', '--locked', '-p', 'tessel-editor', '--message-format=json']
    # No target directory of the caller's and no `TESSEL_GIZMO_PERF_*` reach the build (`harness.child_env`).
    done = harness.run_bounded(command, env=harness.child_env(build=True), timeout=harness.BUILD_TIMEOUT_S)
    exe = None
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        target = message.get('target') or {}
        if message.get('executable') and target.get('name') == 'tessel-editor' and 'bin' in target.get('kind', []):
            exe = message['executable']
    manifest['build'] = {'command': command, **done.record(), 'executable': exe, 'stderr_tail': done.stderr[-1500:],
                         'env_dropped': harness.dropped_env(build=True)}
    if done.returncode != 0 or not exe:
        manifest.update(status='BUILD_FAILED', reason='the release build failed or timed out')
        return harness.die(1, manifest, manifest_path)
    exes = out / 'exes'
    exes.mkdir(exist_ok=True)
    copy = exes / 'tessel-editor'
    try:
        shutil.copy(exe, copy)
    except OSError as error:  # the path cargo printed can be gone (a managed target was pruned): recorded, not a traceback
        manifest.update(status='INFRA_EXECUTABLE_MISSING', missing=str(exe),
                        reason=f'the executable the build named cannot be copied out ({exe}): {error}')
        return harness.die(1, manifest, manifest_path)
    too_big = harness.artifact_reason(exes)
    if too_big:
        manifest.update(status='REFUSED', reason=too_big)
        return harness.die(1, manifest, manifest_path)
    record = {'head': manifest['identity']['head'], 'tree': manifest['identity']['tree'],
              'dirty': bool(manifest['identity']['dirty']),
              'binary': str(copy), 'binary_sha256': harness.sha256(copy),
              'runtime_sha256': harness.sha256(ROOT / 'runtime/dist/runtime.js'),
              'runtime_source_tree': runtime_tree,
              'corpus_manifest_sha256': harness.sha256(FIXTURES / 'corpus.manifest.json'),
              'journey_sha256': harness.sha256(FIXTURES / 'journey.json'), 'built_utc': harness.utc(),
              'toolchain': manifest['toolchain']}
    harness.write(out / 'build.json', record)
    manifest.update(status='BUILT', reason='no run was made', exit=0, record=record)
    harness.write(manifest_path, manifest)
    print(json.dumps({'status': 'BUILT', 'binary_sha256': record['binary_sha256']}))
    return 0


# --- the window ------------------------------------------------------------------------------------------------


class Window:
    """One editor process, driven through its socket the way `./scripts/ui` does, with a fast client."""

    trace_failures = None  # `render.status` queries that did not answer ok (a list once `trace` has failed once)
    last_trace_ok = True

    def __init__(self, binary, home):
        # The editor gets the caller's environment WITHOUT every `TESSEL_GIZMO_PERF_*`, plus exactly the switches this
        # runner needs; `proof_surface.Native` copies `os.environ` when it is built (and can take no environment of its
        # own), so the environment is set for that call only and `os.environ` is what it was afterwards.
        env = harness.child_env({'TESSEL_VIEWPORT_TRACE': '1', 'TESSEL_VIEWPORT_INSPECT': '1',
                                 'TESSEL_VIEWPORT_OVERLAY_ACK': '1', 'TESSEL_RUNTIME_DIR': str(ROOT / 'runtime')})
        self.launched = time.perf_counter()
        with harness.scoped_environ(env):
            self.native = proof_surface.Native(binary, home)
        self.socket = self.native.socket
        self.frames = {}
        self.deadline = None
        self.trace_failures = []

    def check_deadline(self):
        if self.deadline is not None and time.monotonic() > self.deadline:
            raise Timeout(f'the corpus did not finish within {CORPUS_TIMEOUT_S}s')

    def ready(self):
        self.native.prepare()
        return (time.perf_counter() - self.launched) * 1000

    def send(self, request, timeout=90):
        self.check_deadline()
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket)
            conn.sendall((json.dumps({'version': 1, **request}) + '\n').encode())
            data = b''
            while not data.endswith(b'\n'):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=90):
        answer = self.send({'command': 'op', 'request': request}, timeout)
        return answer['ok'], (answer.get('data') if answer['ok'] else answer.get('error'))

    def viewport(self, timeout=90):
        return self.send({'command': 'get', 'target': 'monitor.viewport'}, timeout)['data']['state']

    def trace(self):
        """Polls what the editor itself reports: the picture the runtime last drew and read back (`last_frame`: its
        own render/total/prepare/draw/flush milliseconds and the pixel hash, one entry per request, never trimmed)
        and the session trace (host spans plan/evaluate/ipc/serve with their percentiles; `frames_total` counts only
        the pictures the session stamps, which directed-scene pictures are not).

        A `render.status` that did not answer ok is NOT a report: `{}` comes back, `last_trace_ok` is False and the
        failure is kept in `trace_failures`, which the corpus turns into a failing check. The error text of a failed
        query used to be read as the report, with no session error in it, so a dead query looked like a clean session."""
        state = self.viewport()
        last = state.get('last_frame') or {}
        if last.get('request') is not None:
            self.frames[last['request']] = {key: last.get(key) for key in (
                'request', 'revision', 'hash', 'inspected', 'render_ms', 'total_ms', 'prepare_ms', 'draw_ms',
                'flush_ms', 'width', 'height')}
        ok, data = self.op({'op': 'render.status'})
        self.last_trace_ok = bool(ok)
        if not ok:
            if self.trace_failures is None:
                self.trace_failures = []
            self.trace_failures.append(f'render.status failed: {str(data)[:200]}')
            return {}
        return (data or {}).get('report', data) or {}

    def mark(self):
        return mark(self.viewport())

    def settle(self, scene, frame, after=None, seconds=30):
        """`(ms, state, info)`: seconds (client clock) until the monitor showed the NEW picture of `scene` at `frame`
        (strictly newer than request `after`) with nothing in flight; `ms` is None on timeout.

        `seconds` binds EVERY query, not only the start of one: each `get` waits at most the time that is left (a monitor
        that never answers used to hold a 2 s wait for 90 s), and a reply that arrives after the deadline is a late one:
        the picture it shows converged too late to count, so it is a timeout (`ms` None) with that state and info kept
        for the record."""
        started = time.perf_counter()
        deadline = started + seconds
        state, info = None, None
        while True:
            left = deadline - time.perf_counter()
            if left <= 0:
                return None, state, info
            self.check_deadline()
            try:
                state = self.viewport(timeout=max(left, 0.05))
            except (socket.timeout, TimeoutError):  # the monitor did not answer in the time that was left
                return None, state, info
            done, info = new_picture(state, scene, frame, after)
            if time.perf_counter() > deadline:
                return None, state, info
            if done:
                return (time.perf_counter() - started) * 1000, state, info
            time.sleep(0.01)

    def stop(self):
        return self.native.stop()


def transform(window, scene, node):
    out = []
    for prop in ('position', 'rotation', 'scale'):
        ok, data = window.op({'op': 'scene.get', 'scene': scene, 'property': f'{node}/{prop}'})
        assert ok, (prop, data)
        out.append(data['value'])
    return out


def depths(window):
    ok, data = window.op({'op': 'history.status'})
    assert ok, data
    pick = lambda key: data.get(key, (data.get('history') or {}).get(key))  # noqa: E731
    return pick('undo_depth'), pick('redo_depth')


def picture_hash(state):
    return (state.get('last_frame') or {}).get('hash')


def author_saved_project(binary, variables, corpus_name, scratch, cleanup_errors=None):
    """A throwaway process of the SAME binary writes the corpus project (create, attach, save) and exits. Returns
    `(path, record)`; the record is kept in the report (hash and size of the saved document).

    What stopping that process reported (an editor that ignored the terminate and had to be killed, a log that would not
    close) is appended to `cleanup_errors`, the corpus result's own list, so the check "the editor process ended without
    cleanup problems" fails for the authoring process too; it used to be thrown away. The temporary HOME is removed
    whatever happened, and a failing stop never replaces the authoring error that is already on its way up."""
    home = tempfile.mkdtemp(prefix='gp-author-', dir='/tmp')
    window = None
    try:
        window = Window(binary, home)
        window.ready()
        path = Path(scratch) / f'{corpus_name}.tessel.json'
        for request in ({'op': 'project.create', 'name': f'gizmo-perf-{corpus_name}'},
                        {'op': 'scene.attach', 'id': variables['scene'], 'path': variables['bundle']},
                        {'op': 'project.save', 'path': str(path)}):
            ok, data = window.op(request, 120)
            if not ok:
                raise RuntimeError(f"authoring `{request['op']}` failed: {data}")
        if not path.exists():
            raise RuntimeError('`project.save` answered ok but wrote no document')
        return path, {'path': str(path), 'sha256': harness.sha256(path), 'bytes': path.stat().st_size}
    finally:
        if window is not None:
            try:
                problems = list(window.stop() or [])
            except Exception as error:  # noqa: BLE001 - kept as a problem; it must not replace an authoring error
                problems = [f'stopping the process raised {type(error).__name__}: {error}']
            if cleanup_errors is not None:
                cleanup_errors.extend(f'authoring process: {problem}' for problem in problems)
        shutil.rmtree(home, ignore_errors=True)


def run_corpus(binary, journey, golden, name, out, notes):
    corpus = journey['corpora'][name]
    base = str(ROOT / journey['base'])
    variables = {'scene': journey['scene'], 'scene_b': journey['scene_b'], 'frame': journey['frame'],
                 'bundle': str(ROOT / corpus['bundle']), 'bundle_b': str(ROOT / corpus['bundle_b']), 'base': base,
                 'node': corpus['node'], 'node_b': corpus['node_b'], 'other': corpus['other']}
    scratch = tempfile.mkdtemp(prefix='gp-scratch-', dir='/tmp')
    home = tempfile.mkdtemp(prefix='gp-', dir='/tmp')
    window = None
    result = {'corpus': name, 'checks': [], 'samples': [], 'steps': {}, 'pictures': [], 'overlay_drawn': [],
              'captures': [], 'cleanup_errors': []}

    def check(label, condition, detail=''):
        result['checks'].append({'check': label, 'ok': bool(condition), 'detail': str(detail)[:300]})
        return bool(condition)

    try:
        project, authored = author_saved_project(binary, variables, name, scratch, result['cleanup_errors'])
        result['authored_project'] = authored
        window = Window(binary, home)
        window.deadline = time.monotonic() + CORPUS_TIMEOUT_S
        result['launch_to_socket_ms'] = window.ready()  # process start -> its socket answers: the cold launch
        result['pid'] = window.native.process.pid
        scene = journey['scene']
        node = corpus['node']
        shown = {'scene': None}
        committed, undone = [], []
        hashes = []  # picture hashes before each committed gesture, for the exact-undo pixel check
        offset = {'undo': 0}
        marks = {}

        def timed(step, request, timeout=120, phase=None):
            started = time.perf_counter()
            ok, data = window.op(request, timeout)
            result['samples'].append({'step': step, 'phase': phase, 'op': request['op'],
                                      'client_ms': (time.perf_counter() - started) * 1000,
                                      'ok': ok, 'code': None if ok else (data or {}).get('code')})
            return ok, data

        def commit_job(step, data):
            if not isinstance(data, dict) or data.get('state') != 'queued':
                return True
            deadline = time.time() + 60
            while time.time() < deadline:
                ok, status = window.op({'op': 'job.get', 'job': data['job']})
                if ok and status['state'] in ('succeeded', 'failed', 'cancelled'):
                    break
                time.sleep(0.03)
            else:
                return check(f'{step}: the job finished', False)
            if status['state'] != 'succeeded':
                return check(f'{step}: the job succeeded', False, status)
            ok, commit = window.op({'op': 'job.commit', 'job': data['job']})
            return check(f'{step}: the job was applied', ok or (commit or {}).get('code') == 'job_finished', commit)

        for raw in journey['steps']:
            step = fill(raw, variables)
            sid = step['id']
            phase = step.get('phase')
            if 'surfaces' in step and 'native' not in step['surfaces']:
                result['steps'][sid] = {'skipped': f"listed for {step['surfaces']}"}
                continue
            if sid == 'switch.attach':
                scene = journey['scene_b']
                node = corpus['node_b']
            record = result['steps'].setdefault(sid, {'phase': phase})
            if step.get('kind') == 'drag':
                drag(window, timed, check, step, scene, variables, node, committed, hashes, result, shown)
            elif sid == 'open.project':
                # The real opening of a SAVED project, in a process that has never drawn anything.
                ok, data = timed(sid, {'op': 'project.open', 'path': str(project)}, phase=phase)
                check(f'{sid}: project.open of the saved project answered', ok, data)
                check(f'{sid}: it was the opening of a document (action project.open)',
                      ok and isinstance(data, dict) and data.get('action') == 'project.open', data)
                record['kind'] = 'project.open of a saved project'
                result['opened_saved_project'] = True
            elif sid == 'open.attach':
                # The scene came with the saved project: there is no attach to run, only to verify.
                record['covered_by'] = 'project.open of the saved project'
                result['attached'] = True
                got_undo = depths(window)[0]
                check(f'{sid}: a reopened project starts with a clean history', got_undo == 0, got_undo)
                want = golden[name].get(sid)
                if want is not None:
                    offset['undo'] = history_offset(want['undo'], got_undo)
                record['history_offset'] = offset['undo']
            elif step.get('ui'):
                words = step['ui']
                if words[:3] == ['set', 'window', 'size']:
                    answer = window.send({'command': 'set', 'target': 'window', 'field': 'size', 'value': words[3]})
                else:
                    answer = window.send({'command': 'invoke', 'target': words[1]})
                check(f'{sid}: the window accepted `ui {" ".join(words)}`', answer.get('ok'), answer)
                if sid.startswith('present.resize'):
                    ms, state, info = window.settle(shown['scene'], journey['frame'])
                    record['settle_ms'] = ms
                    check(f'{sid}: the monitor converged after the resize', ms is not None, json.dumps(info))
                if sid == 'present.dialog.open':
                    time.sleep(0.3)
                    # The native page paints above every GPUI element, so an open dialog must hide it (AGENTS.md,
                    # `Workflow::has_dialog`). Recorded AND judged: a page that stayed on screen is a failed check
                    # (it was recorded and never read before).
                    deadline = time.monotonic() + 3
                    while True:
                        monitor = window.viewport()
                        hidden = monitor.get('on_screen') is False or bool(monitor.get('hidden'))
                        if hidden or time.monotonic() >= deadline:
                            break
                        time.sleep(0.05)
                    record['page_hidden_by_dialog'] = hidden
                    check(f'{sid}: the dialog hid the native page', hidden, json.dumps(
                        {key: monitor.get(key) for key in ('on_screen', 'hidden')}))
            elif step.get('wait') == 'presented':
                ms, state, info = window.settle(shown['scene'], journey['frame'], after=marks.get('seek'))
                record['client_ms_to_first_picture'] = ms
                record['picture'] = info
                check(f'{sid}: a NEW picture of frame {journey["frame"]} was drawn after the seek', ms is not None,
                      json.dumps(info))
                if ms is not None:
                    window.trace()
                    result['first_picture_hash'] = picture_hash(state)
                    overlay = state.get('overlay') or {}
                    check(f'{sid}: the page announced the overlay', overlay.get('supported') is True, overlay)
                    path = Path(out) / f'{name}.open.png'
                    shot, why = window.native.capture(path)
                    result['captures'].append({'at': sid, 'path': shot, 'why': why or '',
                                               'sha256': harness.sha256(shot) if shot else None,
                                               'observational_only': True})
            else:
                if step.get('tool'):
                    timed(sid, {'op': 'scene.gizmo.tool', **step['tool']}, phase=phase)
                request = step['op']
                before_hash = picture_hash(window.viewport()) if request['op'] in ('history.undo', 'history.redo') else None
                before_mark = window.mark() if request['op'] in ('monitor.show_scene', 'seek', 'history.undo',
                                                                  'history.redo') else None
                ok, data = timed(sid, request, phase=phase)
                check(f'{sid}: {request["op"]} answered', ok, data)
                if request['op'] == 'seek':
                    marks['seek'] = before_mark
                if request['op'] == 'scene.resources.resolve' and ok:
                    commit_job(sid, data)
                if request['op'] == 'monitor.show_scene' and ok:
                    target = request['scene']
                    ms, state, info = window.settle(target, None if sid == 'open.show' else journey['frame'],
                                                    after=before_mark, seconds=20)
                    record['client_ms_to_converged_picture'] = ms
                    record['picture'] = info
                    wanted = (step.get('expect') or {}).get('shown_scene', target)
                    check(f'{sid}: `{wanted}` drew a NEW picture (newer than request {before_mark})',
                          ms is not None and info['scene'] == wanted, json.dumps(info))
                    if ms is not None:
                        shown['scene'] = target
                if request['op'] == 'scene.gizmo.state' and ok:
                    overlay = data['overlay']
                    if overlay.get('visible'):
                        # The acknowledgement kept in the report must BE this step's: the host holds the last one it
                        # matched until a newer arrives, so the first one read is often the previous step's. Polled for
                        # `ACK_SECONDS` until its tool, space, selection, scene, revision and frame are the ones of this
                        # step's own request and answer; otherwise the check below fails and shows both sides.
                        wanted_tool = step.get('tool') if isinstance(step.get('tool'), dict) else None
                        drawn, mismatch, expected = overlay_ack(window, request, overlay, wanted_tool)
                        result['overlay_drawn'].append({'step': sid, 'drawn': drawn, 'expected': expected,
                                                        'mismatch': mismatch})
                        check(f'{sid}: the page acknowledged THIS step\'s overlay (tool, space, selection, scene, '
                              'revision, frame)', not mismatch, '; '.join(mismatch))
                if request['op'] in ('history.undo', 'history.redo') and ok:
                    ms, state, info = window.settle(shown['scene'], journey['frame'], after=before_mark)
                    record['client_ms_to_converged_picture'] = ms
                    record['picture'] = info
                    check(f'{sid}: the {request["op"]} drew a NEW picture', ms is not None, json.dumps(info))
                    if request['op'] == 'history.undo' and committed:
                        if before_hash is not None and hashes and ms is not None:
                            check(f'{sid}: the undo restored the exact picture of before the gesture',
                                  picture_hash(state) == hashes[-1], (picture_hash(state), hashes[-1]))
                            undone.append(hashes.pop())
                    elif request['op'] == 'history.redo' and undone:
                        hashes.append(undone.pop())
            if sid == 'open.attach' or sid == 'switch.attach':
                result['attached'] = True
            if result.get('attached') and not step.get('ui') and step.get('wait') is None and sid != 'open.project':
                got = {'transform': transform(window, scene, node), 'undo': depths(window)[0],
                       'redo': depths(window)[1]}
                record['state'] = got
                want = golden[name].get(sid)
                if want is not None:
                    check(f'{sid}: transform/undo/redo equal the core golden trace (undo offset {offset["undo"]})',
                          near(got['transform'], want['transform']) and got['undo'] + offset['undo'] == want['undo']
                          and got['redo'] == want['redo'], (got, want))
            window.trace()
        report = window.trace()
        result['frames_seen'] = len(window.frames)
        result['pictures'] = list(window.frames.values())
        trace = (report.get('session') or {}).get('trace') or {}
        result['trace'] = {'enabled': trace.get('enabled'), 'frames_total': trace.get('frames_total'),
                           'dropped': trace.get('dropped'), 'spans_ms': trace.get('spans')}
        result['overlay_report'] = (window.viewport().get('overlay') or {})
        result['session_error'] = ((report.get('session') or {}).get('error'))
        # A `render.status` that failed is no report: it fails here, once per corpus, with what the editor said, and the
        # session is never called error-free on the strength of a query that did not answer.
        result['trace_failures'] = list(window.trace_failures or [])
        check('every `render.status` query answered ok (a failed one is no session trace)',
              not result['trace_failures'], '; '.join(result['trace_failures'][:3]))
        check('the session reported no error', window.last_trace_ok and not result['session_error'],
              result['session_error'] if window.last_trace_ok else 'the last `render.status` query failed: no report')
        check('the overlay host reported no error', not result['overlay_report'].get('error'),
              result['overlay_report'].get('error'))
    except BaseException as error:  # noqa: BLE001 - any cause is a recorded failed check; the cleanup below still runs
        check('the journey ran to the end', False, f'{type(error).__name__}: {error}')
        if isinstance(error, (KeyboardInterrupt, SystemExit)):
            raise
    finally:
        if window is not None:
            result['cleanup_errors'] += window.stop()
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)
        check('the editor process ended without cleanup problems', not result['cleanup_errors'],
              result['cleanup_errors'])
    return result


def drag(window, timed, check, step, scene, variables, node, committed, hashes, result, shown):
    sid = step['id']
    phase = step.get('phase')
    frame = variables['frame']
    record = result['steps'][sid]
    timed(sid, {'op': 'scene.gizmo.tool', 'tool': step['tool'], 'space': step['space']}, phase=phase)
    before = transform(window, scene, node)
    undo_before, redo_before = depths(window)
    ms, state, info = window.settle(scene, frame)
    # The picture before the gesture is the base of every pixel comparison below. `settle` hands back the last state it
    # saw when it times out (`ms is None`): that picture never converged and is not a base, so nothing is pressed.
    if not check(f'{sid}: the picture before the gesture converged (it is the base of the pixel checks)',
                 ms is not None, json.dumps(info)[:260]):
        return
    start_hash = picture_hash(state)
    start_mark = mark(state)
    ok, listed = timed(sid, {'op': 'scene.gizmo.state', 'scene': scene, 'frame': frame}, phase=phase)
    if not check(f'{sid}: gizmo state', ok, listed):
        return
    handle = next((h for h in listed['overlay']['handles'] if h['id'] == step['handle']), None)
    if not check(f'{sid}: the overlay has the `{step["handle"]}` handle', handle is not None, listed['overlay']):
        return
    ring = []
    if handle.get('points'):
        points = handle['points']
        ring = [p for p in points[len(points) // 8:] if p]
        start = ring[0]
    else:
        start = handle['to']
    ok, down = timed(sid, {'op': 'scene.pointer.down', 'scene': scene, 'frame': frame, 'point': start}, phase=phase)
    if not (check(f'{sid}: the press landed on the handle', ok and down.get('dragging') is True, down)
            and check(f'{sid}: the hit handle is `{step["handle"]}`', down['hit']['handle'] == step['handle'], down.get('hit'))):
        return
    moves = step['moves']
    delta = step.get('delta') or [0, 0]
    applied = 0
    last_move = None
    for index in range(1, moves + 1):
        if ring:
            point = ring[min(index, len(ring) - 1)]
        else:
            point = {'u': start['u'] + delta[0] * index / moves, 'v': start['v'] + delta[1] * index / moves}
        ok, moved = timed(sid, {'op': 'scene.pointer.move', 'point': point}, phase=phase)
        check(f'{sid}: move {index} answered and stayed a candidate', ok and moved.get('document_changed') is False, moved)
        if ok and moved.get('applied') is True:
            applied += 1
            last_move = moved
    check(f'{sid}: a move was applied', applied > 0)
    candidate_real = candidate_differs(last_move, before)
    check(f'{sid}: the candidate differs from the base (the reply\'s own candidate value)', candidate_real, last_move)
    # Was the candidate DRAWN? A picture newer than the press, for this scene, shown while the gesture is open.
    ms_c, state_c, info_c = window.settle(scene, frame, after=start_mark, seconds=2)
    candidate_drawn = ms_c is not None
    record['candidate_picture_drawn'] = candidate_drawn
    if candidate_real:
        # A candidate that differs from the base has to be on the monitor while the gesture is open. Recorded only, a host
        # that never drew candidates would turn every "the cancel restored the base picture" below into the base compared
        # with itself, and the cancel branch would have nothing left to prove.
        check(f'{sid}: the candidate was drawn while the gesture was open (a picture newer than the press, within 2 s)',
              candidate_drawn, json.dumps(info_c)[:260])
    if candidate_drawn:
        check(f'{sid}: the drawn candidate picture differs from the base picture',
              picture_hash(state_c) != start_hash, (start_hash, picture_hash(state_c)))
        record['candidate_picture'] = info_c
    window.trace()
    check(f'{sid}: the candidate is not in the document before the release', near(transform(window, scene, node), before))
    gesture_mark = mark(window.viewport())
    if step['end'] == 'up':
        ok, up = timed(sid, {'op': 'scene.pointer.up'}, phase=phase)
        check(f'{sid}: the release committed', ok and up.get('document_changed') is True, up)
        ms, state, info = window.settle(scene, frame, after=gesture_mark if candidate_drawn else start_mark)
        record['client_ms_up_to_converged_picture'] = ms
        record['picture'] = info
        check(f'{sid}: the committed picture is a NEW picture', ms is not None, json.dumps(info))
        after_hash = picture_hash(state) if state else None
        check(f'{sid}: the committed picture differs from the one before the gesture',
              after_hash is not None and after_hash != start_hash, (start_hash, after_hash))
        check(f'{sid}: one undo entry per gesture', depths(window)[0] == undo_before + 1)
        committed.append((before, transform(window, scene, node)))
        hashes.append(start_hash)
    else:
        ok, cancel = timed(sid, {'op': 'scene.pointer.cancel', 'reason': 'api'}, phase=phase)
        check(f'{sid}: the cancel answered', ok, cancel)
        if candidate_drawn:
            ms, state, info = window.settle(scene, frame, after=gesture_mark)
            check(f'{sid}: the cancel drew a NEW picture (the candidate had been drawn)', ms is not None, json.dumps(info))
        else:
            ms, state, info = window.settle(scene, frame, after=None)
            check(f'{sid}: the monitor converged after the cancel', ms is not None, json.dumps(info))
        record['picture'] = info
        # A timeout (`ms is None`) leaves the last state seen, which can still hold the base picture's hash: only a
        # picture the monitor converged on after the cancel may be compared with the base.
        check(f'{sid}: after the cancel the picture is the base picture',
              ms is not None and state is not None and picture_hash(state) == start_hash,
              (ms, start_hash, picture_hash(state) if state else None))
        check(f'{sid}: the cancel left the base document (transform equals the one before the press)',
              near(transform(window, scene, node), before))
        check(f'{sid}: the cancel added no undo entry and no redo entry',
              depths(window) == (undo_before, redo_before), (depths(window), (undo_before, redo_before)))


def main(argv=None):
    harness.install_termination_handlers()
    parser = argparse.ArgumentParser()
    parser.add_argument('--phase', choices=['build', 'run'], default='run')
    parser.add_argument('--binary', help='the editor binary (default: the one `build.json` recorded)')
    parser.add_argument('--build-record', help='`build.json` of the build phase (default: --out/build.json)')
    parser.add_argument('--out', default='/tmp/gizmo-perf-native')
    parser.add_argument('--corpora', nargs='*', default=['small', 'dense', 'deep'])
    parser.add_argument('--allow-dirty', action='store_true',
                        help='run with a dirty tree (recorded; the binary cannot be claimed for HEAD)')
    parser.add_argument('--functional-only', action='store_true',
                        help='run the journey and its functional checks WITHOUT the performance lock and the load '
                             'guard; the timings of such a run are not a measurement and the status says so')
    args = parser.parse_args(argv)
    out = Path(args.out)
    # Ownership first, before anything in the checkout or in `--out` is read, written or removed (a refusal's own
    # manifest included), and kept until this returns: a build owns the CHECKOUT too (nothing else may build from it
    # or put a mutant in its tree meanwhile), every phase owns `--out/.native.lock`. Neither lock waits: a taken one is
    # ONE `REFUSED` line, exit 1, and nothing written.
    owns = 'manifest.json, manifest.build.json, build.json, report.json and exes/'
    with harness.guarded(out, OUT_LOCK, 'native.py', owns, builds=args.phase == 'build') as refusal:
        if refusal:
            return harness.die(1, refusal, None)
        return run_phase(args, out)


def run_phase(args, out):
    """The phase `args.phase` over `out`, which the caller owns (`out/.native.lock` is held). Returns the exit code."""
    manifest_path = out / ('manifest.build.json' if args.phase == 'build' else 'manifest.json')
    manifest = {'suite': 'gizmo-perf-native', 'phase': args.phase, 'utc_start': harness.utc(), 'cwd': str(ROOT),
                'limits': list(LIMITS), 'identity': harness.git_identity(), 'toolchain': harness.toolchain(),
                'status': 'ABORTED', 'command': sys.argv}
    if args.phase == 'build':
        return build_phase(args, manifest, manifest_path, out)
    problem = harness.identity_problem(manifest['identity'])
    if problem:  # a checkout git cannot name is refused before anything else, `--allow-dirty` included
        manifest.update(status='REFUSED', reason=problem)
        return harness.die(1, manifest, manifest_path)
    manifest['env_dropped'] = harness.dropped_env()  # what the editor and the builds below never inherited
    if not args.corpora:
        # `--corpora` with no value would run nothing and report "no check failed": zero checks are never a success.
        manifest.update(status='REFUSED', reason='no corpus was asked for (`--corpora` is empty): a run of zero checks '
                                                 'proves nothing')
        return harness.die(1, manifest, manifest_path)
    record_path = Path(args.build_record) if args.build_record else out / 'build.json'
    record = json.loads(record_path.read_text()) if record_path.exists() else None
    binary = args.binary or (record or {}).get('binary') or str(ROOT / 'target/release/tessel-editor')
    manifest['binary'] = binary
    if not Path(binary).exists() or not (ROOT / 'runtime/dist/runtime.js').exists():
        manifest.update(status='PREREQUISITE_MISSING',
                        reason='needs the release editor (`--phase build`) and `./scripts/runtime setup` (runtime/dist)')
        return harness.die(1, manifest, manifest_path)
    manifest['binary_sha256'] = harness.sha256(binary)
    manifest['runtime_sha256'] = harness.sha256(ROOT / 'runtime/dist/runtime.js')
    manifest['build_record'] = record
    # A run is dirty when the tree is, or when the binary came from a dirty build (or from a record that does not say).
    manifest['dirty_run'] = bool(manifest['identity']['dirty']) or bool(record is not None and record.get('dirty', True))
    refusal = verify_provenance(record, binary, manifest['identity'], args.allow_dirty)
    if refusal:
        manifest.update(status='PROVENANCE_REFUSED', reason=refusal)
        return harness.die(1, manifest, manifest_path)
    journey = json.loads((FIXTURES / 'journey.json').read_text())
    golden = json.loads((FIXTURES / 'journey.expected.json').read_text())
    manifest['corpus'] = json.loads((FIXTURES / 'corpus.manifest.json').read_text())
    manifest['measurement'] = not args.functional_only
    state, reason = harness.guards()
    manifest['machine_before'] = state
    manifest['limits'] += harness.claims(state)
    if state['free_gib'] < harness.MIN_FREE_GIB_FOR_HEAVY:
        reason = reason or f"only {state['free_gib']} GiB free (a heavy start needs {harness.MIN_FREE_GIB_FOR_HEAVY})"
    after_reason = None

    def collect():
        """Every corpus, in order. A termination (SIGTERM/Ctrl-C) leaves a `TERMINATED` manifest, never nothing."""
        try:
            return [run_corpus(binary, journey, golden, name, out, manifest) for name in args.corpora]
        except BaseException as error:
            manifest.update(status='TERMINATED', reason=f'{type(error).__name__}: {error}', utc_end=harness.utc())
            harness.write(manifest_path, manifest)
            raise

    if args.functional_only:
        if state['free_gib'] < harness.FLOOR_FREE_GIB:
            manifest.update(status='BLOCKED', reason=f"only {state['free_gib']} GiB free (floor {harness.FLOOR_FREE_GIB})")
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        results = collect()
        manifest['machine_after'] = harness.machine()
    else:
        if reason:
            manifest.update(status='BLOCKED', reason=reason)
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        with harness.Lock() as lock:
            if not lock.held:
                manifest.update(status='BLOCKED', reason='performance lock busy (not waited for)')
                return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
            results = collect()
            after_reason = harness.judge_after(manifest)
    report = {'journey_version': journey['version'], 'results': results}
    for result in results:
        pics = [p for p in result['pictures'] if p.get('total_ms') is not None]
        result['picture_summary'] = {
            'runtime_total_ms': summary([p['total_ms'] for p in pics]),
            'runtime_render_ms': summary([p['render_ms'] for p in pics if p.get('render_ms') is not None]),
            'pictures_seen': len(pics), 'all_read_back': all(p.get('inspected') for p in pics)}
        result['client_ms_by_op'] = {}
        result['client_ms_by_phase'] = {}
        for sample in result['samples']:
            result['client_ms_by_op'].setdefault(sample['op'], []).append(sample['client_ms'])
            result['client_ms_by_phase'].setdefault(sample.get('phase') or '?', []).append(sample['client_ms'])
        result['client_ms_summary'] = {op: summary(v) for op, v in result['client_ms_by_op'].items()}
        result['client_ms_phase_summary'] = {p: summary(v) for p, v in result['client_ms_by_phase'].items()}
    harness.write(out / 'report.json', report)
    failed = [(r['corpus'], c['check']) for r in results for c in r['checks'] if not c['ok']]
    manifest['report_sha256'] = harness.sha256(out / 'report.json')
    manifest['checks'] = {'total': sum(len(r['checks']) for r in results), 'failed': len(failed)}
    manifest['failed_checks'] = failed
    ok_status = 'PASS_FUNCTIONAL_OBSERVATION_ONLY' if manifest['measurement'] else 'FUNCTIONAL_ONLY_NOT_A_MEASUREMENT'
    if failed:
        status, code = 'FAIL', 1
    elif after_reason:
        status, code = 'INVALID_GUARD_AFTER_RUN', harness.BLOCKED_EXIT
        manifest['reason'] = after_reason
    elif manifest['dirty_run']:
        status, code = 'DIRTY_RUN_NOT_FOR_HEAD', 1
    else:
        status, code = ok_status, 0
    manifest['status'] = status
    manifest['reason'] = manifest.get('reason') if status == 'INVALID_GUARD_AFTER_RUN' else '; '.join(
        f'{c}: {k}' for c, k in failed[:6])
    if status == 'FAIL' and after_reason:
        # The status stays FAIL (a real failure is never masked by the guard), but the reason also carries the guard
        # text: it is the condition the failing checks were taken in.
        manifest['reason'] += f'; the machine guard ALSO failed after the run: {after_reason}'
    manifest['utc_end'] = harness.utc()
    manifest['exit'] = code
    harness.write(manifest_path, manifest)
    print(json.dumps({k: manifest[k] for k in ('status', 'reason', 'checks', 'exit')}))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
