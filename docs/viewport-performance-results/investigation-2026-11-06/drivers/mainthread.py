import sys,re,collections
lines=open(sys.argv[1]).read().split("\n")
# locate main-thread block
start=None
for i,l in enumerate(lines):
    if "com.apple.main-thread" in l and re.match(r"\s+\d+ Thread_",l):
        start=i;break
nodes=[]
for l in lines[start:]:
    m=re.match(r"^(\s*)[+!:| ]*(\d+) (.*)$",l)
    if not m:
        if nodes and l.strip()=="" : break
        continue
    if re.match(r"\s+\d+ Thread_",l) and nodes: break
    pre=re.match(r"^([\s+!:|]*)(\d+) (.*)$",l)
    depth=len(pre.group(1)); nodes.append([depth,int(pre.group(2)),pre.group(3)])
# self counts
selfc=collections.Counter(); total=nodes[0][1]
for idx,(d,c,name) in enumerate(nodes):
    child=0; j=idx+1
    while j<len(nodes) and nodes[j][0]>d:
        if nodes[j][0]==nodes[idx+1][0] if idx+1<len(nodes) and nodes[idx+1][0]>d else False: child+=nodes[j][1]
        j+=1
    s=c-child
    if s>0: selfc[name.split("  (in")[0][:60]+" "+(re.search(r"\(in ([^)]*)\)",name).group(1) if re.search(r"\(in ([^)]*)\)",name) else "")]+=s
print("total",total)
idle=sum(v for k,v in selfc.items() if "mach_msg" in k or "__psynch_cvwait" in k or "kevent" in k)
print("idle",idle, round(100*idle/total,1),"% busy",round(100*(total-idle)/total,1))
for k,v in selfc.most_common(14): print(v,k)
