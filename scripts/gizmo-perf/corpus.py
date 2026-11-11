#!/usr/bin/env python3
"""Synthetic corpus of the gizmo performance suite (F055 / BL015.12 / BL015.18 / BL015.21).

    python3 scripts/gizmo-perf/corpus.py            # write the two generated bundles and the manifest
    python3 scripts/gizmo-perf/corpus.py --check    # fail when a committed file is not what the generator makes

Nothing private: the cases are the repository's own synthetic directed-scene corpus
(`scripts/directed-scene-proof/corpus`, drawn from scratch) plus two scenes GENERATED from it,
deterministically (no clock, no randomness), by adding nodes that reuse its three assets:

* `small`  the corpus as it is (5 nodes, animated: `stage` slides, `hero` sways, `echo` has a scale channel);
* `dense`  the same scene plus 12 rotated banks of 50 image planes (605 nodes, 617 with the groups): what a
           busy scene costs per plan, where the per-call work is dominated by the node count;
* `deep`   the same scene plus two parent chains: 24 rotated, uniformly scaled groups ending in `leaf-rot`
           (rotation and scale legal), 12 groups scaled unevenly ending in `leaf-aniso` (scale legal,
           rotation refused `non_uniform_parent_scale`): the hierarchy the world matrices go through.

There is no splat case: the corpus has no splat asset, and a splat is only as legal as its parents are
similarities (core tests `scene_gizmo_scale.rs` cover its refusals). Recorded as a gap, not simulated.

`corpus.manifest.json` carries the sha256 of every file (inputs, generated bundles, the journey, this
generator), so a result is only comparable with another one that names the same hashes.
"""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent.parent.parent
SOURCE = ROOT / 'scripts/directed-scene-proof/corpus/bundle.json'
OUT = ROOT / 'docs/migration/fixtures/gizmo-perf'
JOURNEY = OUT / 'journey.json'
MANIFEST = OUT / 'corpus.manifest.json'
VERSION = 1
BANKS = 12
PLANES_PER_BANK = 50
ROT_CHAIN = 24
ANISO_CHAIN = 12


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def quat(axis, degrees):
    length = math.sqrt(sum(a * a for a in axis))
    s, c = math.sin(math.radians(degrees) / 2), math.cos(math.radians(degrees) / 2)
    return {'x': round(axis[0] / length * s, 12), 'y': round(axis[1] / length * s, 12),
            'z': round(axis[2] / length * s, 12), 'w': round(c, 12)}


def vec(x, y, z):
    return {'x': x, 'y': y, 'z': z}


def group(node_id, parent, position, rotation, scale):
    return {'id': node_id, 'parentId': parent, 'type': 'group', 'localPosition': position,
            'localRotation': rotation, 'localScale': scale}


def plane(node_id, parent, position, asset, size, order):
    return {'id': node_id, 'parentId': parent, 'type': 'imagePlane', 'localPosition': position,
            'localRotation': quat([0, 0, 1], 0), 'localScale': vec(1, 1, 1), 'assetRef': dict(asset),
            'widthSceneUnits': size, 'heightSceneUnits': size, 'renderOrder': order}


def base():
    return json.loads(SOURCE.read_text())


def assets_of(bundle):
    by_id = {asset['assetId'] if 'assetId' in asset else asset['id']: asset for asset in bundle['assets']}
    refs = {}
    for node in bundle['sceneDefinitions'][0]['nodes']:
        if 'assetRef' in node:
            refs[node['assetRef']['assetId']] = node['assetRef']
    assert set(refs) == set(by_id) or set(refs) <= set(by_id), (refs.keys(), by_id.keys())
    return [refs[name] for name in sorted(refs)]


def dense():
    bundle = base()
    refs = assets_of(bundle)
    nodes = bundle['sceneDefinitions'][0]['nodes']
    order = 10
    for bank in range(BANKS):
        name = f'bank-{bank}'
        nodes.append(group(name, 'root', vec(-5.5 + bank * 1.0, -2.0 + (bank % 3) * 0.8, -1.0 - (bank % 4) * 0.4),
                           quat([0, 1, 0], 6.0 * bank), vec(1, 1, 1)))
        for index in range(PLANES_PER_BANK):
            nodes.append(plane(f'{name}-plane-{index}', name,
                               vec((index % 10) * 0.12, (index // 10) * 0.12, 0.0),
                               refs[(bank + index) % len(refs)], 0.1, order))
            order += 1
    return bundle


def deep():
    bundle = base()
    refs = assets_of(bundle)
    nodes = bundle['sceneDefinitions'][0]['nodes']
    parent = 'root'
    for level in range(ROT_CHAIN):
        name = f'chain-{level}'
        nodes.append(group(name, parent, vec(0.02, 0.01, 0.0), quat([0, 1, 1], 7.0), vec(0.99, 0.99, 0.99)))
        parent = name
    nodes.append(plane('leaf-rot', parent, vec(0.1, 0.0, 0.0), refs[0], 1.0, 20))
    parent = 'root'
    for level in range(ANISO_CHAIN):
        name = f'aniso-{level}'
        nodes.append(group(name, parent, vec(-0.02, 0.0, 0.0), quat([1, 0, 0], 5.0), vec(1.04, 0.97, 1.0)))
        parent = name
    nodes.append(plane('leaf-aniso', parent, vec(0.0, 0.1, 0.0), refs[1 % len(refs)], 1.0, 21))
    return bundle


def text(bundle):
    return json.dumps(bundle, indent=2, sort_keys=True) + '\n'


def generated():
    return {'dense.bundle.json': text(dense()), 'deep.bundle.json': text(deep())}


def manifest(files):
    nodes = {}
    for name, body in files.items():
        nodes[name] = len(json.loads(body)['sceneDefinitions'][0]['nodes'])
    nodes['small'] = len(base()['sceneDefinitions'][0]['nodes'])
    return {
        'corpusVersion': VERSION,
        'private_media': False,
        'generator': {'file': 'scripts/gizmo-perf/corpus.py', 'sha256': sha256(Path(__file__))},
        'journey': {'file': 'docs/migration/fixtures/gizmo-perf/journey.json',
                    'sha256': sha256(JOURNEY) if JOURNEY.exists() else None},
        'source': {'file': 'scripts/directed-scene-proof/corpus/bundle.json', 'sha256': sha256(SOURCE)},
        'files': {name: {'sha256': hashlib.sha256(body.encode()).hexdigest(), 'bytes': len(body.encode())}
                  for name, body in files.items()},
        'nodes': nodes,
        'gaps': ['no splat case: the corpus has no splat asset (core tests cover the refusals)'],
    }


def main(argv):
    files = generated()
    wanted = json.dumps(manifest(files), indent=2, sort_keys=True) + '\n'
    if '--check' in argv:
        bad = [name for name, body in files.items() if not (OUT / name).exists() or (OUT / name).read_text() != body]
        if not MANIFEST.exists() or MANIFEST.read_text() != wanted:
            bad.append(MANIFEST.name)
        if bad:
            print('stale:', ', '.join(bad))
            return 1
        print('corpus up to date')
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    for name, body in files.items():
        (OUT / name).write_text(body)
    MANIFEST.write_text(wanted)
    print(f'wrote {len(files)} bundles and {MANIFEST.relative_to(ROOT)}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
