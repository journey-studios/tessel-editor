import json, glob, sys, statistics as st
prefix = sys.argv[1]
def pct(v,q):
    v=sorted(v); return v[min(len(v)-1, round(q*(len(v)-1)))]
def load(variant):
    return {f: json.load(open(f)) for f in sorted(glob.glob(f"/tmp/bl131/results/{prefix}-{variant}-*.json"))}
def rows_of(d, kind):
    return [r for r in d["rows"] if r["pass"]>0 and ((186<=r["frame"]<=200)==(kind=="cross"))]
M=[("render_ms",None),("drain","sum"),("prepare_ms",None),("settle_total","sum"),("spark_generate_cpu","sum"),("rig_build","sum"),("lease_total","sum"),("rig_dispose","sum")]
for kind in ("cross","single"):
    print(f"== {kind} ==")
    res={}
    for variant in ("before","after"):
        runs=load(variant); res[variant]=runs
        n=sum(len(rows_of(d,kind)) for d in runs.values())
        print(f" {variant}: {len(runs)} runs, {n} frames")
        for name,sub in M:
            means=[];allv=[]
            for d in runs.values():
                v=[(r[name] if sub is None else r[sub].get(name,0)) for r in rows_of(d,kind)]
                means.append(round(st.mean(v),2)); allv+=v
            print(f"   {name:20s} mean per run {means}  pooled mean {st.mean(allv):.2f} p50 {pct(allv,.5):.1f} p95 {pct(allv,.95):.1f}")
        tot=lambda key: [sum(r.get("cache",{}).get(key,0) for r in rows_of(d,kind)) for d in runs.values()]
        print(f"   splat_decoded {tot('splat_decoded')}  splat_mesh_created {tot('splat_mesh_created')}  splat_mesh_reused {tot('splat_mesh_reused')}")
        print(f"   new Spark programs per run {[sum(r['cnt'].get('spark_program_new',0) for r in rows_of(d,kind)) for d in runs.values()]}; decodes {[sum(r['cnt'].get('decode',0) for r in rows_of(d,kind)) for d in runs.values()]}; mapping changes/frame {[round(sum(r['cnt'].get('spark_mapping_changed',0) for r in rows_of(d,kind))/max(1,len(rows_of(d,kind))),2) for d in runs.values()]}")
        print(f"   gpu util mean {[round(st.mean(d['gpu_util'])) for d in runs.values() if d['gpu_util']]}; load1 {[round(d['load1_start'],1) for d in runs.values()]}")
