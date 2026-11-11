#!/usr/bin/env python3
"""F061 (G4.1): one mutant at a time over the production code of the vignette handles; the file is restored (and its
hash compared) after each. KILLED needs a named failing test; a non-zero exit without one is an execution error."""
import hashlib, json, os, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # the checkout this script lives in, never another one
EVIDENCE = Path(os.environ.get('TESSEL_MUTATION_OUT', str(Path(__file__).resolve().parent)))
GEOMETRY = 'crates/core/src/directed_scene/fx/handles.rs'
VIEW = 'crates/core/src/scene_interaction/fx_handles.rs'
GLUE = 'crates/core/src/scene_project/fx_handles.rs'
STATE = 'crates/core/src/state/mod.rs'
STAMP = 'crates/core/src/scene_project/gizmo.rs'
HISTORY = 'crates/core/src/state/history.rs'
GIZMO_VIEW = 'crates/core/src/scene_interaction/gizmo.rs'
OVERLAY = 'runtime/src/overlay.js'

MUTANTS = [
    ('M01 roundness ignored by the stretch', GEOMETRY, 'let stretch = 1.0 + (aspect - 1.0) * roundness;', 'let stretch = 1.0 + (aspect - 1.0) * 1.0;'),
    ('M02 norm is not the length of the shape', GEOMETRY, '(stretch * stretch + 1.0).sqrt().max(1.0);', 'stretch.max(1.0);'),
    ('M03 horizontal semi-axis without the stretch', GEOMETRY, '(r * self.norm / (2.0 * self.stretch), r * self.norm / 2.0)', '(r * self.norm / 2.0, r * self.norm / 2.0)'),
    ('M04 softness handle at the softness, not radius + softness', GEOMETRY, 'self.edge_point(params.center, params.radius + params.softness)', 'self.edge_point(params.center, params.softness)'),
    ('M05 softness value forgets the radius', GEOMETRY, 'let raw = self.distance(params.center, at) - params.radius;', 'let raw = self.distance(params.center, at);'),
    ('M06 centre not put on its grid', GEOMETRY, 'x: quantize_control(at.0, min, max, CENTER_STEP, false),', 'x: at.0.clamp(min, max),'),
    ('M07 radius value not put on its grid', GEOMETRY, 'let raw = self.distance(params.center, at);\n                HandleValue::Number(quantize_control(raw, min, max, step, false))', 'let raw = self.distance(params.center, at);\n                HandleValue::Number(raw)'),
    ('M08 a tie goes to the inner handle', GEOMETRY, 'pub(crate) const PRIORITY: [Self; 3] = [Self::Softness, Self::Radius, Self::Center];', 'pub(crate) const PRIORITY: [Self; 3] = [Self::Center, Self::Radius, Self::Softness];'),
    ('M09 hit distance without the aspect', GEOMETRY, '((at.0 - point.0) * aspect).hypot(at.1 - point.1)', '(at.0 - point.0).hypot(at.1 - point.1)'),
    ('M10 a huge hit radius', GIZMO_VIEW, 'pub const HIT_RADIUS: f64 = 0.025;', 'pub const HIT_RADIUS: f64 = 0.5;'),
    ('M11 everything is on the picture', GEOMETRY, '(0.0..=1.0).contains(&at.0) && (0.0..=1.0).contains(&at.1)', 'true'),
    ('M12 keys never disable a handle', VIEW, '.is_some_and(|keys| !keys.is_empty());', '.is_some_and(|keys| keys.is_empty() && false);'),
    ('M13 the clip covers one frame too many', VIEW, 'frame < found.end_frame_exclusive()', 'frame <= found.end_frame_exclusive()'),
    ('M14 a handle off the picture is offered', VIEW, '} else if !on_picture(at) {', '} else if false && !on_picture(at) {'),
    ('M15 a press on a keyed handle is not refused by name', VIEW, '.filter(|(_, why)| *why == reason::ANIMATED_PARAM)', '.filter(|(_, why)| *why != reason::ANIMATED_PARAM)'),
    ('M16 selecting a clip keeps the entity selection', GLUE, 'if next.is_some() && !self.scene_interaction.selection.is_empty() {', 'if false && next.is_some() && !self.scene_interaction.selection.is_empty() {'),
    ('M17 moving the target keeps the drag', GLUE, 'let cancelled = if moved { self.cancel_live_drag() } else { None };', 'let cancelled: Option<String> = None;'),
    ('M18 selecting an entity keeps the drag of the clip', GLUE, 'let cancelled = self.cancel_live_drag();', 'let cancelled: Option<String> = None;'),
    ('M19 the drag writes another parameter', GLUE, 'clip: clip.clone(),\n            param: handle.param().to_string(),', 'clip: clip.clone(),\n            param: "intensity".to_string(),'),
    ('M20 a gone clip keeps the target', GLUE, '.is_some_and(|entry| fx_clip_is_vignette(&entry.scene, &target.clip));', '.is_some_and(|_| true);'),
    ('M21 the release is a scene edit in the history', STATE, '            "Edit effects"\n        } else {', '            "Edit scene"\n        } else {'),
    ('M22 the stamp does not name the clip', STAMP, 'stamp["fx_clip"] = json!(target.clip);', 'let _ = &target.clip;'),
    ('M23 a command leaves a target the project no longer holds', STATE, '        let outcome = self.apply_command(command);\n        self.prune_fx_target();\n        outcome', '        let outcome = self.apply_command(command);\n        outcome'),
    ('M24 undo and redo leave a target the project no longer holds', HISTORY, '        self.prune_fx_target();\n', ''),
    ('M25 the hit query does not predict the effect handles', GLUE, 'let Some(pick) = self.fx_pick(scene, frame, point) else {\n            return false;', 'let Some(pick) = self.fx_pick(scene, frame, point).filter(|_| false) else {\n            return false;'),
    ('M26 a press refused on a greyed handle leaves the open drag', GLUE, 'if let Some(open) = self.live_drag() {\n            answer["abandoned"]', 'if let Some(open) = self.live_drag().filter(|_| pick.refusal().is_none()) {\n            answer["abandoned"]'),
]

JS_MUTANTS = [
    ('J01 a tie goes to the last handle of the core list', OVERLAY, "if (best === null || distance < best.distance - 1e-12) best = { distance, id: handle.id };", "if (best === null || distance <= best.distance + 1e-12) best = { distance, id: handle.id };"),
    ('J02 hover distance without the aspect', OVERLAY, "const distance = Math.hypot(here.x - handle.at.u * overlay.aspect, here.y - handle.at.v);\n    if (distance > fx.hitRadius) continue;", "const distance = Math.hypot(here.x - handle.at.u, here.y - handle.at.v);\n    if (distance > fx.hitRadius) continue;"),
    ('J03 dimmed handles drawn wherever they are', OVERLAY, "if (entry.reason !== 'animated_param' || !entry.at || !insidePicture(entry.at)) continue;", "if (!entry.at) continue;"),
    ('J04 the softness ellipse is not dashed', OVERLAY, "context.setLineDash(ellipse.id === 'softness' ? [6, 4] : []);", "context.setLineDash([]);"),
    ('J05 a dimmed handle lights up under the pointer', OVERLAY, "for (const handle of fx.handles) {\n    const distance", "for (const handle of [...fx.handles, ...fx.disabled.filter((entry) => entry.at).map((entry) => ({ id: entry.id, at: entry.at, kind: 'edge' }))]) {\n    const distance"),
    ('J06 the vignette overlay is not the one hovered', OVERLAY, "if (overlay?.fx) return hoverFx(overlay, point);", "if (false) return hoverFx(overlay, point);"),
    ('J07 the page does not say the target is the fx', OVERLAY, "{ target: 'fx', fx:", "{ target: 'node', fx:"),
    ('J08 the ellipse is as wide as it is tall', OVERLAY, "const radii = { x: ellipse.rx * rect.width, y: ellipse.ry * rect.height };", "const radii = { x: ellipse.rx * rect.height, y: ellipse.ry * rect.height };"),
    ('J09 the active handle is lost in the parser', OVERLAY, "active: typeof value.active === 'string' ? value.active : null,", "active: null,"),
    ('J10 hidden handles are hit too', OVERLAY, "const { fx } = overlay;\n  if (!fx.visible) return null;", "const { fx } = overlay;"),
]
ALL_PATHS = sorted({m[1] for m in MUTANTS + JS_MUTANTS})


def digest(path):
    return hashlib.sha256((ROOT / path).read_bytes()).hexdigest()


def run(command, cwd):
    try:
        result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    except OSError as error:  # the runner is missing: an execution error, never a verdict
        return 127, f'{command[0]}: {error}'
    return result.returncode, result.stdout + result.stderr


def rebuild_dist():
    """`runtime/dist` is what the browser tests run: after a JS mutant it is built from the restored source.
    False (with the reason on stderr) when that build fails: `dist` would still hold the last mutant."""
    code, out = run(['sh', '-c', 'pnpm run build >/dev/null'], ROOT / 'runtime')
    if code != 0:
        print(f'FAILED: the rebuild of runtime/dist exited {code}; it still holds the last mutant. '
              f'Run `./scripts/runtime build` before trusting any runtime test.\n{out[-1500:]}', file=sys.stderr, flush=True)
    return code == 0


def main():
    only = set(sys.argv[1:])
    report = []
    hashes_before = {path: digest(path) for path in ALL_PATHS}
    dist_dirty = False  # a JS mutant was (or may have been) built into `runtime/dist`
    try:
        for name, path, old, new in MUTANTS + JS_MUTANTS:
            if only and name.split()[0] not in only:
                continue
            file = ROOT / path
            original = file.read_text()
            if original.count(old) != 1:
                report.append({'mutant': name, 'result': 'NOT-APPLICABLE', 'count': original.count(old)})
                print(name, 'NOT APPLICABLE', original.count(old), flush=True)
                continue
            started = time.time()
            try:
                file.write_text(original.replace(old, new))
                if path.endswith('.js'):
                    # The browser tests run the BUILT page (`runtime/dist`): the mutant has to be built into it.
                    dist_dirty = True
                    code, out = run(['sh', '-c', 'pnpm run build >/dev/null && node --test test/fx-handles-overlay.test.mjs'],
                                    ROOT / 'runtime')
                else:
                    code, out = run(['./scripts/cargo', 'test', '-p', 'tessel-editor-core', '--locked', '--no-fail-fast',
                                     '--lib', '--test', 'scene_fx_handles', '--test', 'scene_fx_target'], ROOT)
            finally:
                file.write_text(original)
            failed = sorted({line.split()[1] for line in out.splitlines() if line.startswith('test ') and line.endswith('FAILED')})
            failed += sorted({line.split(' - ', 1)[1] for line in out.splitlines() if line.startswith('not ok') and ' - ' in line})
            # KILLED needs a test that ran and failed by name. A non-zero exit without one is the
            # runner's own failure (no mbx, no node, a crash) and says nothing about the tests.
            if code == 0:
                result = 'SURVIVED'
            elif failed:
                result = 'KILLED'
            elif 'error[' in out or 'could not compile' in out:
                result = 'COMPILE-ERROR'
            else:
                result = 'EXECUTION-ERROR'
            report.append({'mutant': name, 'result': result, 'failed': failed[:6], 'seconds': round(time.time() - started)})
            print(name, result, failed[:3], flush=True)
    finally:
        # Also on an interruption: the source of the running mutant is already back (inner `finally`),
        # `dist` goes back to that source here.
        dist_rebuilt = rebuild_dist() if dist_dirty else None
    hashes_after = {path: digest(path) for path in ALL_PATHS}
    restored = hashes_before == hashes_after and dist_rebuilt is not False
    out = {'restored': restored, 'dist_rebuilt': dist_rebuilt, 'hashes': hashes_after, 'mutants': report}
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    name = 'mutation-report.json' if not only else 'mutation-report-' + '-'.join(sorted(only)) + '.json'
    (EVIDENCE / name).write_text(json.dumps(out, indent=2) + '\n')
    print('restored:', out['restored'])
    return 0 if restored and all(r['result'] in ('KILLED', 'SURVIVED') for r in report) else 2


raise SystemExit(main())
