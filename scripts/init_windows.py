import sys, csv, glob, os, statistics as st
# pair runs in run order: a[i] with b[i]; windows by cumulative time
W=[(0,0.5),(0.5,1.0),(1.0,1.5),(1.5,2.0),(2.0,3.0),(3.0,5.1)]
def load(f): return [float(r['ms']) for r in csv.DictReader(open(f))]
def win(ms):
    out=[];
    for lo,hi in W:
        t=0; n=0
        for m in ms:
            if lo*1000<=t<hi*1000: n+=1
            t+=m
        out.append(n)
    return out
for d in sys.argv[1:]:
    A=[win(load(f)) for f in sorted(glob.glob(os.path.join(d,'a','frames','*-initial.csv')))]
    B=[win(load(f)) for f in sorted(glob.glob(os.path.join(d,'b','frames','*-initial.csv')))]
    print('==',os.path.basename(d))
    for k,(lo,hi) in enumerate(W):
        diffs=[b[k]-a[k] for a,b in zip(A,B)]
        print(f'{lo:.1f}-{hi:.1f}s  a med {st.median([a[k] for a in A]):6.0f} b med {st.median([b[k] for b in B]):6.0f}  b-a per pair {diffs}  b lower in {sum(x<0 for x in diffs)}/{len(diffs)}')
