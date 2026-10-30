import sys, csv, glob, os, statistics as st
lo,hi=float(sys.argv[1])*1000,float(sys.argv[2])*1000
tot={'a':[], 'b':[]}
for d in sys.argv[3:]:
  for side in 'ab':
    for f in sorted(glob.glob(os.path.join(d,side,'frames','*-initial.csv'))):
        rows=list(csv.DictReader(open(f))); t=0; prevg=None
        gc=[0,0.0]; tk=[0,0.0]; pl=[]
        for r in rows:
            m=float(r['ms']); g=int(r['gcs'])
            if lo<=t<hi:
                if prevg is not None and g>prevg: gc[0]+=1; gc[1]+=m
                elif int(r['ticks'])>0: tk[0]+=1; tk[1]+=m
                else: pl.append(m)
            prevg=g; t+=m
        tot[side].append((os.path.basename(d)[:14],len(pl)+gc[0]+tk[0],gc[0],gc[1],tk[0],tk[1],len(pl),sum(pl),st.mean(pl),st.median(pl)))
for side in 'ab':
    print(side)
    for x in tot[side]: print('  %-14s n=%4d gc %d/%5.1fms tick %2d/%5.1fms plain n=%4d sum=%6.1f mean=%.3f med=%.3f'%x)
    print('  median: gc ms %.1f, tick ms %.1f, plain mean %.3f, plain med %.3f, n %d'%(st.median(x[3] for x in tot[side]),st.median(x[5] for x in tot[side]),st.median(x[8] for x in tot[side]),st.median(x[9] for x in tot[side]), st.median(x[1] for x in tot[side])))
