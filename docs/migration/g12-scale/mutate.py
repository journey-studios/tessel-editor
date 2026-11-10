#!/usr/bin/env python3
"""One mutant at a time over the new production code; the file is restored (and its hash compared) after each."""
import hashlib, json, subprocess, sys, time
from pathlib import Path

ROOT = Path('/Users/wallyssonnunes/.t3/worktrees/tessel-editor/t3-3902d1d1')
SCALING = 'crates/core/src/scene_interaction/gizmo/scaling.rs'
PROJECT = 'crates/core/src/scene_project/gizmo.rs'
OVERLAY = 'runtime/src/overlay.js'

MUTANTS = [
    ('M01 no maximum limit', SCALING, 'if new > MAX_SCALE && new > old {', 'if new > MAX_SCALE * 1e9 && new > old {'),
    ('M02 no minimum limit', SCALING, 'if new < MIN_SCALE && new < old {', 'if new < MIN_SCALE * 1e-9 && new < old {'),
    ('M03 zero factor allowed through', SCALING, 'if factor <= 0.0 {', 'if factor < 0.0 {'),
    ('M04 no-change band removed', SCALING, 'if (factor - 1.0).abs() < NO_CHANGE {', 'if (factor - 1.0).abs() < 1e-15 {'),
    ('M05 alignment never refused', SCALING, 'if skew.is_nan() || skew > ALIGNMENT_TOLERANCE {', 'if false && (skew.is_nan() || skew > ALIGNMENT_TOLERANCE) {'),
    ('M06 alignment tolerance loose', SCALING, 'const ALIGNMENT_TOLERANCE: f64 = 1e-9;', 'const ALIGNMENT_TOLERANCE: f64 = 1e-3;'),
    ('M07 residual shear guard off', SCALING, 'if (factor - 1.0).abs() * skew > SHEAR_TOLERANCE {', 'if (factor - 1.0).abs() * skew > 1e9 {'),
    ('M08 determinant bound off', SCALING, 'if !determinant.is_finite() || determinant.abs() < MIN_DETERMINANT {', 'if !determinant.is_finite() {'),
    ('M09 splat axes allowed', SCALING, '            if splat {\n                return Err(why::SPLAT_NON_UNIFORM);', '            if false && splat {\n                return Err(why::SPLAT_NON_UNIFORM);'),
    ('M10 animated scale allowed', SCALING, '        if animated {\n            return Err(why::ANIMATED_SCALE);', '        if false && animated {\n            return Err(why::ANIMATED_SCALE);'),
    ('M11 uniform scales x only', SCALING, '            Handle::Uniform => {}\n', '            Handle::Uniform => {\n                changed = [true, false, false];\n            }\n'),
    ('M12 local axes read as rows', SCALING, 'normalize([m[0][h], m[1][h], m[2][h]])', 'normalize([m[h][0], m[h][1], m[h][2]])'),
    ('M13 press at the origin allowed (axis)', SCALING, 'if parameter.is_nan() || parameter < MIN_PRESS_FRACTION * reference {', 'if parameter.is_nan() || parameter < 0.0 * reference {'),
    ('M14 press at the origin allowed (uniform)', SCALING, 'if distance.is_nan() || distance < MIN_UNIFORM_PRESS {', 'if false {'),
    ('M15 uniform box does not win', SCALING, '&& view.distance(point, uniform.to) <= CENTER_RADIUS', '&& view.distance(point, uniform.to) <= 0.0'),
    ('M16 component picked by the smallest diagonal', SCALING, '.max_by(|a, b| t(*a, *a).abs().total_cmp(&t(*b, *b).abs()))', '.min_by(|a, b| t(*a, *a).abs().total_cmp(&t(*b, *b).abs()))'),
    ('M17 shown-vs-authored guard off', SCALING, 'if shown != base {', 'if false && shown != base {'),
    ('M18 non-finite product not refused', SCALING, '    if !new.is_finite() {\n        return Err(why::ABOVE_MAXIMUM);\n    }\n', ''),
    ('M19 refused move drops the candidate', PROJECT, 'return Ok(json!({ "applied": false, "reason": reason, "factor": factor }));', 'if let Some(open) = self.scene_interaction.drag.as_mut() { open.last = None; }\n                return Ok(json!({ "applied": false, "reason": reason, "factor": factor }));'),
    ('M20 screen handle reaches the frame', PROJECT, '        if axis == Handle::Screen {\n            return Err(CommandError::invalid_value(\n                "`axis` must be `x`, `y`, `z` or `uniform`', '        if false && axis == Handle::Screen {\n            return Err(CommandError::invalid_value(\n                "`axis` must be `x`, `y`, `z` or `uniform`'),
    ('M21 non-finite factor reaches the frame', PROJECT, '        if !factor.is_finite() {\n            return Err(CommandError::invalid_value(\n                "`factor` must be a finite number",', '        if false && !factor.is_finite() {\n            return Err(CommandError::invalid_value(\n                "`factor` must be a finite number",'),
    ('M22 splat below not seen', PROJECT, '                if current.id == node.id {\n                    return true;\n                }\n                match current\n                    .parent', '                if false && current.id == node.id {\n                    return true;\n                }\n                match current\n                    .parent'),
    ('M23 animated scale looks at rotation', PROJECT, 'animates(document, plan, frame, &node.id, ChannelProperty::LocalScale)', 'animates(document, plan, frame, &node.id, ChannelProperty::LocalRotation)'),
    ('M24 scale writes to the wrong component (uniform)', SCALING, '                let result = *value * factor;', '                let result = *value * factor.sqrt();'),
    ('M25 limits judged on factor not result', SCALING, 'within_limits(*value, result)?;', 'within_limits(1.0, factor)?;'),
]

JS_MUTANTS = [
    ('J01 uniform box does not win in hover', OVERLAY, "if (uniform && Math.hypot(here.x - uniform.to.u * overlay.aspect, here.y - uniform.to.v) <= overlay.centerRadius) return uniform.id;", "if (false && uniform) return uniform.id;"),
    ('J02 scale axes not hoverable', OVERLAY, "['axis', 'ring', 'scale_axis'].includes(candidate.kind)", "['axis', 'ring'].includes(candidate.kind)"),
    ('J03 disabled handles dropped', OVERLAY, ".filter((entry) => isObject(entry) && typeof entry.id === 'string')", ".filter(() => false)"),
    ('J04 scale tool unknown to the parser', OVERLAY, "raw.tool === 'rotate' || raw.tool === 'scale' ? raw.tool : 'translate'", "raw.tool === 'rotate' ? raw.tool : 'translate'"),
]


def digest(path):
    return hashlib.sha256((ROOT / path).read_bytes()).hexdigest()


def run(command, cwd):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


def main():
    only = set(sys.argv[1:])
    report = []
    hashes_before = {path: digest(path) for path in (SCALING, PROJECT, OVERLAY)}
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
                code, out = run(['node', '--test', 'test/overlay.test.mjs'], ROOT / 'runtime')
            else:
                code, out = run(['./scripts/cargo', 'test', '-p', 'tessel-editor-core', '--locked', '--no-fail-fast',
                                 '--lib', '--test', 'scene_gizmo_scale', '--test', 'scene_gizmo_rotation', '--test', 'scene_gizmo'], ROOT)
        finally:
            file.write_text(original)
        compile_error = 'error[' in out or 'could not compile' in out and 'test failed' not in out
        failed = sorted({line.split()[1] for line in out.splitlines() if line.startswith('test ') and line.endswith('FAILED')})
        failed += sorted({line.split(' - ', 1)[1] for line in out.splitlines() if line.startswith('not ok')})
        result = 'COMPILE-ERROR' if compile_error and not failed else ('KILLED' if code != 0 else 'SURVIVED')
        report.append({'mutant': name, 'result': result, 'failed': failed[:6], 'seconds': round(time.time() - started)})
        print(name, result, failed[:3], flush=True)
    hashes_after = {path: digest(path) for path in (SCALING, PROJECT, OVERLAY)}
    out = {'restored': hashes_before == hashes_after, 'hashes': hashes_after, 'mutants': report}
    Path('/tmp/g12-evidence/mutation-report.json').write_text(json.dumps(out, indent=2) + '\n')
    print('restored:', out['restored'])


main()
