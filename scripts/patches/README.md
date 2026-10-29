# Parked experiments

Changes that were built, measured and binned (or diagnostics used once), kept so they can be re-tried. Each is
described in `CONTINUATION.md` section 7 under its session. Names were rewritten from `vulkanmod` to `ruthenium`
after the rename; results and logs stay in `build/run/bench/results/<dir>`.

| File | Session / results dir | Applies to |
|---|---|---|
| `s05-diagonal-groups.patch` | 5th, diagonal face groups | `b63badc` (rename `net/vulkanmod` back, or rebase) |
| `s07-terrain-occlusion.patch` | 7th, CPU occlusion culling of terrain | `dd183cb` (same) |
| `s15-nametags.patch` | 15th, `entities-dev/s15-nametags` | HEAD (`git apply`) |
| `s15-villager-ids.patch` | 15th, `entities-dev/s15-villager` | HEAD |
| `verify-s15-diag.patch` | 15th verification counters, `verify-s15` | HEAD |
| `s16-inside-EntityInsideBlocksM.java.txt` | 16th, `entities-dev/s16-inside` | a mixin source; rename to `.java` under `mixin/world/` and register it |
| `s16-sections-EntitySectionStorageM.java.txt` | 16th, `entities-dev/s16-sections` | same |
| `s17-cells.patch` | 17th, `s17-cells` | HEAD |
| `s17-f2b.patch` | 17th, `s17-f2b-*` | HEAD |
| `s17-sprites.patch` | 17th, `s17-sprites` | HEAD |
| `s19-check-TextCacheCheck.java.txt` | 19th check of the name tag and text display caches (worktree `s1`, `s3/verify`) | `render/text/`; call it where the kept width, kept component and kept prepared text are returned (`-Druthenium.verifyTextCache=true`) |
| `s19-check-TextDisplayCullingCheckM.java.txt` | 19th check of the text display culling radius (`s3/verify`) | a mixin source for `mixin/render/entity/` (`-Druthenium.verifyTextDisplayCulling=true`) |

The two old patches (5th, 7th) fail on HEAD because the code moved on; `git apply --3way` gets most of the way.
