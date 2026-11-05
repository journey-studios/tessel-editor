import { readFileSync } from 'node:fs';
const rows=readFileSync(process.argv[2],'utf8').trim().split('\n').map(l=>JSON.parse(l));
const by={};
for(const r of rows){ if(r.fatal){console.log('FATAL',r.run.name,r.run.pass,r.fatal);continue;} (by[`${r.run.case}/${r.run.variant}`]??=[]).push(r);}
const med=a=>{a=a.filter(Number.isFinite).sort((x,y)=>x-y);return a[a.length>>1]};
const f=(v)=>v===undefined||v===null?'-':v.toFixed(1);
console.log('case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err');
for(const [c,l] of Object.entries(by)){
 console.log(c,'|',f(med(l.map(r=>r.cold_first.render))),'|',f(med(l.map(r=>r.render.mean))),'|',['p50','p95','p99'].map(k=>f(med(l.map(r=>r.wall[k])))).join('/'),'|',['p50','p95','p99'].map(k=>f(med(l.map(r=>r.complete?.[k])))).join('/'),'|',`${l[0].sources?.proxy_frames}/${l[0].sources?.original_frames}`,'|',l.find(r=>r.proxy_error)?.proxy_error?.message??'');
}
