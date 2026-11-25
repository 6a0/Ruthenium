"""Run one fault at a time against the renderer in a disposable copy of the workspace."""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


# Each replacement must match exactly once. A moved rule is an error, not a kill.
FLUID = "terrain/fluid.rs"
SORT = "terrain/sort.rs"
FIXTURE = "terrain_check.rs"
CPU = [
    (FLUID, "height * 10.0", "height * 9.0"),
    (FLUID, "along_x > 0.0 || along_z > 0.0", "along_x > 0.0 && along_z > 0.0"),
    (FLUID, "here.own_height()", "1.0"),
    (FLUID, "!same(neighbour) && !occluded", "!same(neighbour) || !occluded"),
    (FLUID, "hh0.max(hh1)", "hh0.min(hh1)"),
    (FLUID, "let render_up = !same(&up);", "let render_up = true;"),
    (FLUID, "for ox in -1..=1", "for ox in 0..=0"),
    (FLUID, "const INSET: f32 = 0.001", "const INSET: f32 = 0.0"),
    (FLUID, "mth::atan2(flow[2], flow[0])", "mth::atan2(flow[0], flow[2])"),
    (FLUID, "a.1.max(b.1)", "a.1.min(b.1)"),
    (FLUID, "shades[UP] * shade", "shade"),
    (FLUID, "!overlay,", "true,"),
    (SORT, "keys[b as usize].total_cmp(&keys[a as usize])",
     "keys[a as usize].total_cmp(&keys[b as usize])"),
    (SORT, ".total_cmp(&keys[a as usize])", ".total_cmp(&keys[a as usize]).then(b.cmp(&a))"),
    (SORT, "dx * dx + (dy * dy + dz * dz)", "(dx * dx + dy * dy) + dz * dz"),
    (SORT, "(first[i] + third[i]) / 2.0", "first[i]"),
    (SORT, "mth::floor(camera[i]) >> 4", "(camera[i] as i32) >> 4"),
    (SORT, "pov.contains(&0)", "pov == [0; 3]"),
    (SORT, "pov != sorted_from ||", "pov != sorted_from &&"),
    (SORT, "centres.len() > NARROW_QUADS", "centres.len() >= NARROW_QUADS"),
    (SORT, "[0, 1, 2, 2, 3, 0]", "[0, 1, 2, 1, 3, 0]"),
    (FIXTURE, "Some(_) => 0.1", "Some(_) => 0.0"),
    (FIXTURE, "Some(_) => 0.1", "Some(_) => 0.5"),
    (FIXTURE, "scene.order.iter().map(|&i| &of_draw[i as usize]).collect()", "of_draw.iter().collect()"),
    (FIXTURE, "depth[i] = z;", "if !translucent { depth[i] = z; }"),
    (FIXTURE, "if z >= depth[i]", "if translucent || z >= depth[i]"),
    (FIXTURE, "let src = if k == 3 { 1.0 } else { a };", "let src = a;"),
    (FIXTURE, "under[k] * (1.0 - a)", "under[k] * a"),
]
GPU = [
    ("pipelines.rs", "&shaders::TERRAIN_RGSS_TRANSLUCENT_FRAG,", "&shaders::TERRAIN_RGSS_CUTOUT_FRAG,"),
    ("pipelines.rs", "depth: Depth::TestAndWrite,",
     "depth: if self.is_translucent_terrain() { Depth::Test } else { Depth::TestAndWrite },"),
    ("pipelines.rs", "if self.is_translucent_terrain() {\n                Blend::Alpha",
     "if self.is_translucent_terrain() {\n                Blend::Opaque"),
    (FIXTURE, "self.order\n            .iter()", "(0..self.order.len() as u32)"),
]


def run(checkout, target, args):
    result = subprocess.run(
        ["cargo", *args], cwd=checkout, env={**os.environ, "CARGO_TARGET_DIR": str(target)},
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
    )
    return result.returncode, result.stdout


def cpu_check(checkout, target):
    status, log = run(checkout, target, ["test", "--locked", "-p", "ruthenium-render", "--lib"])
    failures = re.findall(r"^test (\S+) \.\.\. FAILED$", log, re.M)
    if status == 0 and re.search(r"test result: ok\. [1-9]\d* passed", log):
        return []
    if status == 101 and failures and "test result: FAILED" in log:
        return failures
    raise RuntimeError(log)


def gpu_check(checkout, target):
    failures = []
    for draws in ("direct", "mdi"):
        status, log = run(checkout, target, [
            "run", "--locked", "-p", "ruthenium", "--", "--data-dir", str(checkout / "data"),
            "--exercise-terrain", "--terrain-draws", draws, "--quit-after", "5",
        ])
        report = re.search(r"terrain check: (\d+) frames of \d+ rectangles compared, (\d+) wrong", log)
        if not report or int(report[1]) < 20 or (int(report[2]) > 0) != (status != 0):
            raise RuntimeError(log)
        if draws == "mdi" and not all(
            f"{feature} on" in log for feature in ("multiDrawIndirect", "drawIndirectFirstInstance")
        ):
            raise RuntimeError("The selected GPU cannot exercise indirect terrain draws")
        print(f"{draws}: {report[0]}", flush=True)
        if int(report[2]):
            failures.append(f"{draws} ({report[2]} wrong)")
    if len(failures) == 1:
        raise RuntimeError(f"Only one draw path detected the mutation: {failures}")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gpu", action="store_true", help="run the windowed Vulkan mutations")
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    target = root / "target" / "render-mutations"
    mutations, check = (GPU, gpu_check) if options.gpu else (CPU, cpu_check)
    survivors = []
    with tempfile.TemporaryDirectory(prefix="ruthenium-render-") as tmp:
        checkout = Path(tmp)
        for name in ("Cargo.lock", "Cargo.toml"):
            shutil.copy2(root / name, checkout / name)
        for name in ("crates", "branding", "xtask"):
            shutil.copytree(root / name, checkout / name)
        if check(checkout, target):
            raise RuntimeError("Unmodified renderer failed the baseline")
        for file, before, after in mutations:
            path = checkout / "crates/ruthenium-render/src" / file
            source = path.read_text()
            if source.count(before) != 1:
                raise ValueError(f"Expected one match in {file} for {before!r}")
            path.write_text(source.replace(before, after))
            failures = check(checkout, target)
            path.write_text(source)
            rule = f"{file}: {before!r} -> {after!r}"
            if not failures:
                survivors.append(rule)
            print(f"{rule}: {', '.join(failures) or 'SURVIVED'}", flush=True)
    total = len(mutations)
    print(f"{total - len(survivors)} of {total} mutations detected")
    return bool(survivors)


if __name__ == "__main__":
    raise SystemExit(main())
