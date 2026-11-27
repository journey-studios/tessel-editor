#!/usr/bin/env python3
"""BL081.2 functional proof. Usage: python3 scripts/split-fades-proof.py REPO OUT.

Requires built target/debug/tessel-headless and tessel-editor. Uses isolated
HOME/socket through proof_surface, WAV comparison, no coordinates or benchmarks.
"""
import sys,json,subprocess
from pathlib import Path
ROOT=Path(sys.argv[1]);OUT=Path(sys.argv[2]);OUT.mkdir(exist_ok=True,parents=True)
sys.path.insert(0,str(ROOT/'scripts'))
from proof_surface import Headless,Native,ProofFailure,run_surface
fixtures=OUT/'fixtures'
subprocess.run([str(ROOT/'scripts/fixtures'),str(fixtures)],check=True,capture_output=True,timeout=600)
def scenario(surface,check,scratch):
    n=0
    def ok(req):
        good,out=surface.op(req)
        if not good:raise ProofFailure(str((req,out)))
        return out
    def wav(a,b):
        nonlocal n
        n+=1;p=scratch/f'mix{n}.wav'
        ok({'op':'render.audio','from':a,'frames':b-a,'out':str(p)})
        return p.read_bytes()
    for fi,fo,cut,edited,protect,active in [(0,24,32,'fade_in',(40,48),(32,34)),(24,0,16,'fade_out',(0,8),(14,16))]:
        for line in (fixtures/'proof.jsonl').read_text().splitlines():
            if line.strip():ok(json.loads(line))
        for prop,value in [('fade_in',fi),('fade_out',fo)]:ok({'op':'clip.property','clip':'c-tone','property':prop,'value':value})
        uncut=wav(0,48)
        if surface.name=='native':
            surface.invoke('clip:c-tone','select')
            good,out=surface.set('timeline.playhead','frame',cut)
            if not good:raise ProofFailure(str(out))
            good,out=surface.invoke('timeline.split','invoke')
            if not good:raise ProofFailure(str(out))
        else:ok({'op':'clip.split','clip':'c-tone','frame':cut})
        check(edited+': cut preserves the uncut WAV',uncut==wav(0,48))
        clip='c-tone-split-1' if edited=='fade_in' else 'c-tone'
        before=wav(*protect);edge_before=wav(*active)
        ok({'op':'clip.property','clip':clip,'property':edited,'value':5})
        after=wav(*protect);edge_after=wav(*active)
        check(edited+': untouched inherited ramp keeps WAV bytes',before==after)
        check(edited+': edited ramp changes the correct edge',edge_before!=edge_after)
        ok({'op':'history.undo'})
        check(edited+': undo restores edited edge',wav(*active)==edge_before)
        ok({'op':'history.redo'})
        check(edited+': redo restores edit and untouched ramp',wav(*protect)==before and wav(*active)==edge_after)
        path=scratch/(edited+'.tessel.json')
        ok({'op':'project.save','path':str(path)})
        check(edited+': envelope saved as schema 5',json.loads(path.read_text())['schema_version']==5)
        ok({'op':'project.open','path':str(path)})
        check(edited+': reopen retains both ramps',wav(*protect)==before and wav(*active)==edge_after)
    return None
failed=False
for cls,binary in [(Headless,'tessel-headless'),(Native,'tessel-editor')]:
    r=run_surface(cls,str(ROOT/'target/debug'/binary),scenario,OUT,'independent-ramps')
    for c in r['checks']:print(r['name'],c,flush=True)
    print(r['name'],r['passed'],r['skipped'],r['failed'],r['error'],flush=True)
    failed=failed or r['failed']
sys.exit(1 if failed else 0)
