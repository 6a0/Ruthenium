import sys, csv, glob, os, json, statistics as st
W=[(0.5,1.0),(0.5,2.0),(2.0,5.1)]
def win(f,lo,hi):
    t=0;n=0
    for r in csv.DictReader(open(f)):
        if lo*1000<=t<hi*1000: n+=1
        t+=float(r['ms'])
    return n
def fps(f): return [p for p in json.load(open(f))['phases'] if p['name']=='initial'][0]['avgFps']
ratios=[]; wr={w:[] for w in W}
for d in sys.argv[1:]:
    A=sorted(glob.glob(os.path.join(d,'a','frames','*-initial.csv'))); B=sorted(glob.glob(os.path.join(d,'b','frames','*-initial.csv')))
    JA=sorted(glob.glob(os.path.join(d,'a','*.json'))); JB=sorted(glob.glob(os.path.join(d,'b','*.json')))
    for a,b,ja,jb in zip(A,B,JA,JB):
        ratios.append(fps(jb)/fps(ja)-1)
        for w in W: wr[w].append(win(b,*w)/win(a,*w)-1)
print(f"pairs {len(ratios)}: phase b/a-1 median {st.median(ratios):+.2%} mean {st.mean(ratios):+.2%} sd {st.stdev(ratios):.2%} lower in {sum(r<0 for r in ratios)}")
for w in W: print(f"window {w}: median {st.median(wr[w]):+.2%} mean {st.mean(wr[w]):+.2%} lower in {sum(r<0 for r in wr[w])}/{len(wr[w])}")
