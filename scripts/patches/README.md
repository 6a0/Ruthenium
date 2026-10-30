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
| `s26-memo-probe.patch` | 26th, candidate 6 probe (`s26-c6-probe`, logs `2191`/`2192`): `Util.memoize` lookups on the render thread per frame, by memo instance (calls, last-key hit rate, distinct identical and equal keys, first caller), logged every 5000 frames | `1a43c7d` (`git apply`; `-Pjvm.ruthenium.probe.memo=true`) |
| `s26-c6-check.patch` | 26th exactness check of `renderTypeCache` and `emptyCrossbowCharge` (`s26-c6-check`, logs `2193-2199`): every front cache hit also asks vanilla's memo and compares the instance, every skipped charge duration also calls vanilla's `getChargeDuration`; once per launch vanilla on synthetic empty stacks (`EMPTY`, air, a count 0 Quick Charge III crossbow) and a count 1 control | HEAD after both commits (`git apply`; `-Pjvm.ruthenium.c6Check=true`) |
| `s27-child-check.patch` | 27th exactness check of `partChildArrays` (`s27-alloc-check-child-*`, logs `2224-2226`): every render from the child array also iterates the children map and compares each child, and counts renders that fall back to the map | HEAD after the child array commit (`git apply`; `-Pjvm.ruthenium.childCheck=true`) |
| `s27-pose-check.patch` | 27th lifetime check of `pooledSubmitPoses` (`s27-alloc-close8-pose-*`, `s27-alloc-close8-pose2-*`, `s27-alloc-close8-fabcopy2-entities`, logs `2235-2237`, `2244`, `2245`, `2280`): at the start of each frame the last frame's pooled poses and matrices are set to NaN, so anything reading one after its frame draws nothing (frozen close-ups across the switch); logs the poses and matrices per frame | HEAD after the Fabric block model copy commit (`git apply`; `-Pjvm.ruthenium.poseCheck=true`) |
| `s27-probe-check.patch` | 27th exactness check of `noAllocLightProbe` (`s27-alloc-check-probe-*`, logs `2263`, `2264`): every computed probe position is compared, bit for bit, with vanilla's `getLightProbePosition` and its `BlockPos.containing` | HEAD after the light probe commit (`git apply`; `-Pjvm.ruthenium.probeCheck=true`) |
| `s27-name-check.patch` | 27th exactness check of `keepDisplayNames` (`s27-alloc-check-names-*`, logs `2267`, `2268`): every kept name tag also calls vanilla's `getNameTag` and compares (`equals`) | HEAD after the display name commit (`git apply`; `-Pjvm.ruthenium.nameCheck=true`) |
| `s27-light-localref.patch` | 27th, binned (`s27-alloc-light-mixed`, logs `2247-2252`): `EntityRendererLightM` takes the entity from a `@ModifyReceiver` on `entity.level()` instead of `@Local`, which MixinExtras passes to a `WrapOperation` as a `LocalRefImpl` per call (9-12% of the mixed scene's render thread allocation in JFR). 3 interleaved pairs: 67.1 vs 71.7 KB/frame static, no change; the `LocalRefImpl` is scalar replaced without JFR | `fe0ac8d` (`git apply`) |
| `s27-shadow-scratch.patch` | 27th, binned (`s27-alloc-toggle-shadow*-players`, logs `2277`, `2279`): `ShadowFeatureRenderer.shadowVertex` transforms into a scratch vector instead of a new one. Players static -15 KB/frame, but fps lower in 6 of 8 rounds in one launch and even in the other, GCs 70 vs 77; too small to resolve. Also add `render.feature.ShadowFeatureRendererM` to `ruthenium.mixins.json` | HEAD (`git apply`, then the mixin entry) |

The two old patches (5th, 7th) fail on HEAD because the code moved on; `git apply --3way` gets most of the way.
