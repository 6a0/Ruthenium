"""Runs a command while holding the machine-wide GPU lock, so only one Minecraft instance runs at a time.

Usage: python scripts/gpulock.py <command> [args...]
  e.g. python scripts/gpulock.py ./gradlew --console=plain benchRuthenium -Pbench.scene=mixed
       python scripts/gpulock.py sh scripts/run-ab.sh 3 mixed-dev/x "benchRuthenium" "-Pvm.x=true" ""

The lock is an OS file lock on %LOCALAPPDATA%/ruthenium-gpu.lock (shared by every worktree), released by the OS when this
process exits, however it exits. While holding it, the script also waits until no Minecraft JVM (Fabric's KnotClient or
vanilla's client main) is running, because an interrupted Gradle build can leave its game running in the daemon. The command
runs with RUTHENIUM_GPU_LOCK set; the bench tasks in build.gradle refuse to start without it (-PbenchNoLock overrides).
Holder details are in ruthenium-gpu.lock.info next to the lock.
"""
import msvcrt
import os
import re
import subprocess
import sys
import time

LOCK = os.path.join(os.environ.get("LOCALAPPDATA", os.path.expanduser("~")), "ruthenium-gpu.lock")
GAME = re.compile(r"KnotClient|net\.minecraft\.client\.main\.Main")


def running_games():
    ps = ("Get-CimInstance Win32_Process -Filter \"Name like 'java%'\" | "
          "ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }")
    out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True).stdout
    return [line.split(" ", 1)[0] for line in out.splitlines() if GAME.search(line)]


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    f = open(LOCK, "a+")
    waited = False
    while True:
        try:
            f.seek(0)
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            break
        except OSError:
            if not waited:
                try:
                    holder = open(LOCK + ".info").read().strip()
                except OSError:
                    holder = "unknown"
                print(f"gpulock: waiting for the GPU lock (held by: {holder})", file=sys.stderr, flush=True)
                waited = True
            time.sleep(5)
    with open(LOCK + ".info", "w") as info:
        info.write(f"pid {os.getpid()} since {time.strftime('%H:%M:%S')} in {os.getcwd()}: {' '.join(sys.argv[1:])}\n")
    games = running_games()
    if games:
        print(f"gpulock: waiting for running Minecraft JVM(s) {games} to exit", file=sys.stderr, flush=True)
        while running_games():
            time.sleep(5)
    env = dict(os.environ, RUTHENIUM_GPU_LOCK=str(os.getpid()))
    cmd = sys.argv[1:]
    # Shell scripts and ./gradlew need Git Bash's sh on Windows
    if cmd[0].endswith(".sh") or cmd[0] in ("./gradlew", "gradlew"):
        cmd = ["sh"] + cmd
    sys.exit(subprocess.run(cmd, env=env).returncode)


if __name__ == "__main__":
    main()
