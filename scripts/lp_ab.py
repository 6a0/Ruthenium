import re, sys, glob, os, statistics as st
from collections import defaultdict
# usage: lp.py dir phase [rows]   (a = off, b = on); prints per-section mean ms/frame (tree) and "other f" (no-tick frames)
d, phase = sys.argv[1], sys.argv[2]; rows=int(sys.argv[3]) if len(sys.argv)>3 else 30
def parse(f):
    L=open(f,encoding='utf-8',errors='replace').read().splitlines()
    tree={}; split={}; frames=None
    for i,l in enumerate(L):
        m=re.match(r'Light profile of %s: (\d+) frames, ([\d.]+) ms/frame'%phase, l)
        if not m: continue
        tree['TOTAL']=float(m.group(2)); j=i+2; stack=[]
        while j<len(L) and L[j].strip():
            mm=re.match(r'^( *)(\S.*?)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)$', L[j])
            if mm:
                dep=len(mm.group(1))//2; stack=stack[:dep]+[mm.group(2)]; tree['/'.join(stack)]=float(mm.group(3))
            j+=1
        while j<len(L) and not L[j].startswith('Split by frame kind'): j+=1
        j+=2; stack=[]
        while j<len(L) and L[j].strip():
            mm=re.match(r'^( *)(\S.*?)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(-?[\d.]+)$', L[j])
            if not mm: break
            dep=len(mm.group(1))//2; stack=stack[:dep]+[mm.group(2)]; split['/'.join(stack)]=float(mm.group(4))
            j+=1
        break
    return tree, split
data={s:[parse(f) for f in sorted(glob.glob(os.path.join(d,s,'*.log')))] for s in 'ab'}
for idx,name in ((0,'tree ms/frame (all frames)'),(1,'no-tick frames ms/frame')):
    print('==',name, 'runs', len(data['a']), len(data['b']))
    keys=set(data['a'][0][idx])&set(data['b'][0][idx])
    out=[]
    for k in keys:
        try:
            A=[r[idx][k] for r in data['a']]; B=[r[idx][k] for r in data['b']]
        except KeyError: continue
        out.append((st.mean(B)-st.mean(A),k,st.mean(A),st.mean(B),sum(b>a for a,b in zip(A,B))))
    out.sort(key=lambda x:-abs(x[0]))
    for dlt,k,a,b,h in out[:rows]: print(f'{dlt:+.4f}  off {a:.4f} on {b:.4f}  on higher {h}/{len(data["a"])}  {k}')
