# Metaspace by class loader and package group (investigation helper, uncommitted).
# Usage: python scripts/metaspace_breakdown.py <jcmd VM.metaspace show-loaders show-classes output> <jcmd VM.classes output> [jar ...]
# Per loader the numbers are measured (jcmd). Within a loader, the class space part per group is measured too (the Klass
# sizes of VM.classes); the non-class part is apportioned by class file size (jars given; lambdas and other hidden
# classes count as 1.2 KB), so it is an estimate.
import sys, re, zipfile, io, os, collections, glob

def parse_loaders(path):
    loaders = []  # (name, [classes], nonclass KB, class KB)
    cur = None
    for line in open(path, encoding='utf-8', errors='replace'):
        m = re.match(r'\s*\d+: CLD 0x[0-9a-f]+: (.*)', line)
        if m:
            cur = {'name': m.group(1).strip(), 'classes': [], 'non': 0.0, 'cls': 0.0}
            loaders.append(cur)
            continue
        if cur is None:
            continue
        m = re.match(r'\s+\d+:\s+(\S+)', line)
        if m and 'chunk' not in line:
            cur['classes'].append(m.group(1))
            continue
        m = re.match(r'\s+(Non-Class|Class):.*?([\d.]+) KB \(\s*[<>]?\d+%\) used', line)
        if m:
            cur['non' if m.group(1) == 'Non-Class' else 'cls'] = float(m.group(2))
        if line.startswith('Usage per space type'):
            break
    return loaders

def parse_klass_sizes(path):
    sizes = {}
    for line in open(path, encoding='utf-8', errors='replace'):
        m = re.match(r'0x[0-9a-f]+\s+(\d+)\s+\S+\s+\S*\s+(\S+)', line)
        if m:
            sizes[m.group(2)] = int(m.group(1)) * 8
    return sizes

def index_jar(data_or_path, index, depth=0):
    try:
        z = zipfile.ZipFile(data_or_path)
    except Exception:
        return
    for info in z.infolist():
        n = info.filename
        if n.endswith('.class') and not n.startswith('META-INF/versions/'):
            index.setdefault(n[:-6].replace('/', '.'), info.file_size)
        elif n.endswith('.jar') and depth < 2:
            index_jar(io.BytesIO(z.read(n)), index, depth + 1)

def group(name):
    n = name.split('/0x')[0].lstrip('[')
    if n.startswith('L'):
        n = n[1:].rstrip(';')
    rules = [
        ('net.ruthenium.gametest.', 'Ruthenium gametest (bench harness)'), ('net.ruthenium.', 'Ruthenium'),
        ('net.minecraft.util.datafix.', 'MC datafix (schemas, fixers)'),
        ('net.minecraft.client.', 'MC client'), ('net.minecraft.server.', 'MC server'),
        ('net.minecraft.world.level.levelgen.', 'MC worldgen'), ('net.minecraft.world.', 'MC world (other)'),
        ('net.minecraft.network.', 'MC network'), ('net.minecraft.', 'MC other'),
        ('com.mojang.blaze3d.', 'Mojang blaze3d/renderpearl'), ('com.mojang.renderpearl.', 'Mojang blaze3d/renderpearl'),
        ('com.mojang.datafixers.', 'DFU'), ('com.mojang.serialization.', 'Mojang serialization'), ('com.mojang.', 'Mojang other libs'),
        ('net.fabricmc.fabric.', 'Fabric API'), ('net.fabricmc.loader.', 'Fabric Loader'), ('net.fabricmc.', 'Fabric other'),
        ('org.spongepowered.', 'Mixin'), ('com.llamalad7.', 'MixinExtras'), ('org.objectweb.asm.', 'ASM'),
        ('org.lwjgl.', 'LWJGL'), ('it.unimi.dsi.fastutil.', 'fastutil'), ('com.google.', 'Guava/Gson'), ('io.netty.', 'Netty'),
        ('org.joml.', 'JOML'), ('org.apache.logging.', 'Log4j'), ('org.slf4j.', 'SLF4J'), ('com.ibm.icu.', 'ICU4J'),
        ('java.lang.invoke.', 'JDK invoke (LambdaForms, ...)'), ('java.', 'JDK'), ('javax.', 'JDK'), ('jdk.', 'JDK'), ('sun.', 'JDK'), ('com.sun.', 'JDK'),
    ]
    for prefix, g in rules:
        if n.startswith(prefix):
            return g
    return 'other: ' + '.'.join(n.split('.')[:2])

# A lambda proxy's non-class metaspace (~1.4 KB, measured with 3000 synthetic lambdas: 2.0-2.4 KB in all, 0.59 KB class
# space) expressed as class file bytes at the Knot loader's ratio of non-class metaspace per class file byte (~1.27)
LAMBDA_EQUIV = 1100

def main():
    loaders = parse_loaders(sys.argv[1])
    klass = parse_klass_sizes(sys.argv[2])
    index = {}
    for j in sys.argv[3:]:
        for p in glob.glob(j):
            index_jar(p, index)
    # Loader totals
    by_loader = collections.OrderedDict()
    for l in loaders:
        key = l['name'] if not l['name'].startswith('<hidden class>') else 'hidden classes, ' + l['name'].split('loaded by ')[-1]
        e = by_loader.setdefault(key, [0, 0.0, 0.0])
        e[0] += len(l['classes']); e[1] += l['non']; e[2] += l['cls']
    print('== Per loader (measured, MB used): classes, non-class, class space, total')
    for k, (c, non, cls) in sorted(by_loader.items(), key=lambda kv: -(kv[1][1] + kv[1][2])):
        print(f'{c:7d} {non/1024:8.2f} {cls/1024:7.2f} {(non+cls)/1024:8.2f}  {k}')
    # Per group within each big loader
    for l in loaders:
        if len(l['classes']) < 50:
            continue
        rows = collections.defaultdict(lambda: [0, 0, 0, 0, 0])  # classes, lambdas, class bytes, klass bytes, missing
        total_bytes = 0
        for c in l['classes']:
            g = group(c)
            r = rows[g]
            r[0] += 1
            hidden = '/0x' in c
            if c.startswith('['):
                b = 0  # array class: no class file, a small Klass in class space
            elif hidden:
                r[1] += 1
                b = LAMBDA_EQUIV
            else:
                b = index.get(c)
                if b is None:
                    r[4] += 1
                    b = 3000
            r[2] += b
            total_bytes += b
            r[3] += klass.get(c, 0)
        klass_total = sum(r[3] for r in rows.values())
        print(f"\n== {l['name']}: {len(l['classes'])} classes, non-class {l['non']/1024:.2f} MB, class space {l['cls']/1024:.2f} MB (Klass sizes sum {klass_total/1048576:.2f} MB)")
        print('  classes lambdas  classfile MB  est. non-class MB  class space MB  est. total MB  group  (missing class files)')
        for g, r in sorted(rows.items(), key=lambda kv: -kv[1][2]):
            non = l['non'] / 1024 * r[2] / total_bytes
            cls = l['cls'] / 1024 * (r[3] / klass_total if klass_total else r[0] / len(l['classes']))
            print(f'  {r[0]:7d} {r[1]:7d} {r[2]/1048576:12.2f} {non:18.2f} {cls:15.2f} {non+cls:14.2f}  {g}' + (f'  ({r[4]})' if r[4] else ''))

main()
