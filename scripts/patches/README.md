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
| `s19-spread-builds.patch` | 19th, `s19-builds2`, `s19-budget` | `8fbc095`-era `RetainedBlockEntities` (`git apply --3way` on HEAD) |
| `s19-check-TextCacheCheck.java.txt` | 19th check of the name tag and text display caches (worktree `s1`, `s3/verify`) | `render/text/`; call it where the kept width, kept component and kept prepared text are returned (`-Druthenium.verifyTextCache=true`) |
| `s19-check-TextDisplayCullingCheckM.java.txt` | 19th check of the text display culling radius (`s3/verify`) | a mixin source for `mixin/render/entity/` (`-Druthenium.verifyTextDisplayCulling=true`) |
| `s20-downloaded-skins.patch` | 20th, `s20-dyn-*`, `s20-c3-*`: transparent face culling of downloaded skins (`DynamicTexture`, alpha copied after each upload) and face masks kept per texture alpha | `970dcc5` / HEAD (`git apply`) |
| `s21-sort-check.patch` | 21st exactness check of `reuseSortOrder` (`build/bench-logs/s21-check-*.log`): every sorted staged draw also sorted with vanilla's merge sort and compared, plus counters of the start orders and of bit-identical centroids | HEAD after the sort order commit (`git apply`; `-Pjvm.ruthenium.probe.sortCheck=true`) |
| `s22-pose-check.patch` | 22nd exactness check of `playerPoseDraws` (`build/bench-logs/s22-check-*.log`, `s22-parity-vk-lobby.log`): every posed member also written through the switch-off path (`ModelPartCubeM`'s direct writes) and through vanilla's `Cube.compile` arithmetic; the shader's position and normal arithmetic emulated from the uploaded rows; centroids and each draw's order compared with vanilla's merge sort; `-Pjvm.ruthenium.probe.poseFail=N` fails every N-th capture (fallback and materialization) | HEAD after the pose draw commit (`git apply`; `-Pjvm.ruthenium.probe.poseCheck=true`) |

The two old patches (5th, 7th) fail on HEAD because the code moved on; `git apply --3way` gets most of the way.
