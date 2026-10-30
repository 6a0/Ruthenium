# Diffs two class histograms of the footprint scene (results/footprint/*-histo-*.txt): the classes whose live bytes
# changed most from A to B. Usage: python scripts/histo_diff.py A.txt B.txt [rows]
import sys,re
def load(p):
    d={}
    for l in open(p,encoding='utf-8'):
        m=re.match(r'\s*\d+:\s+(\d+)\s+(\d+)\s+(\S+)',l)
        if m: d[m.group(3)]=(int(m.group(1)),int(m.group(2)))
    return d
a,b=load(sys.argv[1]),load(sys.argv[2])
rows=[]
for k in set(a)|set(b):
    ia,ba=a.get(k,(0,0)); ib,bb=b.get(k,(0,0))
    rows.append((bb-ba,ib-ia,k,bb))
rows.sort(key=lambda r:-abs(r[0]))
for d,di,k,bb in rows[:int(sys.argv[3]) if len(sys.argv)>3 else 20]:
    print(f"{d/1048576:+8.2f} MB {di:+9d} inst  {k}  (now {bb/1048576:.1f} MB)")
