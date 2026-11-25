# Performance Notes (`fefw-recomp`)

Current performance status, shipped optimizations, key toggles, deferred ideas, and open targets as of **v0.2.13+** (late Nov 2026).

- **Historical v0.2.0 lab log:** [docs/PERF_NOTES_0.2.0.md](docs/PERF_NOTES_0.2.0.md) (archived chronological notes from `stephenjayakar/fefw-recomp` through v0.2.0 / 14 Nov).
- **Benchmarking & A/B harness:** [docs/PERF_ROUTES.md](docs/PERF_ROUTES.md) (`tools/perf/ab_run.py`, `perf_report.py`, `record_to_route.py`).
- **Full config & toggle reference:** [runtime/docs/CONFIG.md](runtime/docs/CONFIG.md).

---

## 1. Bottleneck Profile (Where Time Goes)

From profiling dense town scenes (Dagsion hub/street, ~2,400–2,640 draws/frame) and battle maps:

- **Main game thread (`main`):** ~78–86% of busy CPU samples are in lifted AArch64 game code, with a flat profile (top 40 functions ~25–37%). Roughly ~48% of lifted-code samples land on guest memory loads (pointer chasing: `mov rax, [rax]`), making the game thread largely **memory-latency bound** alongside instruction-fetch/cache pressure across a large `.text` (~147 MB).
  - Main-thread waits are primarily short chains of `Basic_Worker1-4` jobs, `GFX_Worker0-4`, `MOCull_Worker1/2` + Leader, and GPU sync waits when the render thread falls behind.
- **Worker threads:**
  - `Basic_Worker1-4` (~95% lifted code): led by box/plane culling (`f_7101980580`), `f_71005ef6e0`, `f_7100a831a0`, `f_7100a9bb60`, `f_7101978d50`.
  - `MOCull_Worker1/2` + Leader (~99% lifted code): dominated by masked occlusion culling (`f_71019730e0` ~80–84% of workers, a 16k-instruction NEON rasterizer; `f_7101976dc0` ~61% of Leader).
  - `GFX_Worker0-4`: ~42% lifted code (`f_7100a7c620`), rest is NVN command recording.
- **Render thread (NVN → Vulkan replay):** CPU-bound on descriptor set building (`build_set`), draw/buffer binding & compute-out overlap tracking, texture preparation, and Vulkan driver calls (`vkCmd*`, `vkUpdateDescriptorSets`).
- **Fast-travel / arrival frames:** ~290–350 ms of spread-out lifted game logic (allocation/containers, ~14% under `nn::mem::StandardAllocator::Allocate`); zstd decompression is already native.

---

## 2. CPU & Recompiler / Codegen (`recomp/`, `game-build/fefw/`)

### Shipped Optimizations
- **Local NZCV flags (`opt = ('flags',)` in `recomp/nxrecomp/opt.py`):** Keeps AArch64 `NZCV` in function-local `NxFlags` instead of spilling 4 fields to `Ctx` on every flag-setting instruction (per-function backward liveness check; ~-18% x86 instructions in hot functions, ~-12–14% main CPU time).
- **Whole-register vector stores (`opt = ('vstore',)` + `neon.h`):** Emits scalar-into-lane-0 results, `MOVI` pairs, and `c->v[n] = <V128>` as single 16-byte stores (`nx_vst`), eliminating partial-write store-forwarding stalls (-4.8% static instructions).
- **`NX_NOESC` (`noescape`) on `Ctx *c` (`recomp/include/nxrecomp/ctx.h`):** Marks `Ctx *` as `noescape` on lifted functions and runtime ABI entries, paired with `-mllvm -capture-tracking-max-uses-to-explore=100000` in `fefw_build.py` / `NxRecomp.cmake`. Prevents LLVM from reloading `Ctx` fields after every guest memory store (-20% context loads, -17% context stores, ~-8–9% main CPU).
- **Inlined indirect-call dispatch (`NX_INLINE_INDIRECT`, `recomp/include/nxrecomp/cxx.h`):** Expands `call_indirect` inline to `nx_call_indirect`, indexing the main module's flat offset table (`nx_fn_flat` in `runtime/src/core/dispatch.cpp`) right at each `BLR`/`BR` site so every call site keeps its own x86 branch predictor slot. Fallback uses a 1,024-entry per-thread cache + lock-free hash table.
- **Hot-code packing & cross-TU inlining (`recomp/games/fefw/hot.txt`, `inline.txt`):**
  - `hot_order` packs the 3,409 hottest functions (99.5% of samples) into `.text$nxh<rank>` sections (.text 189 → 147 MB).
  - `inline_edges` copies 1,039 small hot callees (≤60 insns) as `static inline` into their hot callers' TUs (measured neutral on its own, kept in config).
- **Native HLE replacements (`GameConfig.inline_replace` in `recomp/games/fefw/config.py`, `game-build/fefw/src/fefw_hle.cpp`):**
  - `nnSdk` libc routines: `memcpy`/`memmove`, `memset`, `strlen`, `memcmp`, `strcmp` (`cxx.h` `hle_*`).
  - `ZSTD_decompressMultiFrame` (`0x71000c6a60`): native zstd 1.5.7 with `NX_ORIG` fallback (`FEFW_HLE_ZSTD=0` disables). Eliminates ~250 ms loading-frame spikes.
  - Native culling tests (`0x71019786c0` view-box, `0x7101980580` cylinder-box, ported from ObeliskChamber `ab56dd1`): currently **off by default** (`FEFW_HLE_CULL=1` enables, `FEFW_HLE_CULL=2` verifies against lifted original).
- **Compiler & CPU targets (`game-build/release/fefw_build.py`, `recomp/cmake/NxRecomp.cmake`):**
  - Lifted code built with `-O2` (note: bundled `zig c++` maps `-O1`/`-O2`/`-O3` to clang `-cc1 -O2` with loop/SLP vectorization; MSVC source builds switched from `/O1` to `/O2` in `ffd542d` / `a19645f`), `-g0`, `-fomit-frame-pointer`, `-ffp-contract=off`.
  - Two CPU build variants (`--cpu fma` default: `x86_64+cx16+fma`; `--cpu compat`: `x86_64+cx16+sse4_2+popcnt` + musl `fefw_fma.c`).
  - Developer PGO pipeline (`fefw_build.py --pgo-gen` / `--pgo-use`, with ext-TSP block placement; measured ~-4.5% main CPU).
- **Per-thread ABI counters (`runtime/include/nxrt/tl_counts.h`):** Replaced shared atomic counters on `host_import`, HLE thunks, and SVCs (eliminated 8.9–23.7% main / ~50% worker cache-line bouncing during load spikes).

### Tried, Deferred, or Experimental
- **`opt.py` `'tregs'` (`x8..x17` in locals, `NXRECOMP_OPT=flags,vstore,tregs`):**
  - Measured ~-2.2% to -3.6% main CPU in v0.2.0 tests.
  - Turned **off** by default in `config.py` (`2df9df5`) pending full safety verification across the game. `4a6dce9` (ObeliskChamber PR #165) subsequently fixed syncing `x9..x17` across branches to non-ABI fragments/hook splits and added `recomp/tests/opt_tregs_test.py`. Ready to re-audit and re-test.
- **`opt.py` `'xregs'` / `'vregs'` (full guest registers in locals):** Eagerly loading all registers at entry and after every call stretched live ranges, caused host register spills, and grew code size (plus 3/30 e2e failures around jump tables/tail calls). Would require **lazy loading** (load on first use after a sync point) + dirty tracking.
- **Clang 23 vs bundled Zig Clang 22.1.8:** Only -1.1% static instructions on hot set; not worth breaking PGO format / release reproducibility.
- **`-mcpu=znver2`:** Only -0.1% static instructions; neutral in A/B runs.
- **Large pages (2 MiB) for guest heap:** `tlbbench` showed +11–22 ns/miss in L2/L3 (+65 ns past L3), but conflicts with per-4 KiB `VirtualProtect` write-watch and requires `SeLockMemoryPrivilege` + major `memory.cpp` rework. Needs actual hardware DTLB-miss profiling (e.g. AMD uProf / `perf stat`) before considering.
- **Dropping `c->fn` store on function entry:** Only -0.3% static instructions, hurts crash-report precision; skipped.

---

## 3. NVN Render-Thread Replay & GPU Overlap (`runtime/src/gfx/nvn/`)

### Shipped Optimizations (v0.2.0 → v0.2.13+)
- **GPU/CPU overlap & submission splitting:**
  - `[nvn] split_submit_ms = 4` (default on as of `0e10804`, `split_min_draws = 16`): flushes recorded commands at render-pass boundaries mid-submission so the GPU starts work while the render thread keeps recording.
  - `[nvn] submit_batch = auto` defaults to `1` on Windows and Wine+NVIDIA (`0e10804`), keeping `frame` on non-NVIDIA Wine.
- **Hardware-aware GPU barrier & shadow defaults (`e5fc38d`, `5dfc016`, `0c35aec`):**
  - `[nvn] lean_barriers = auto`: enables `dispatch_barriers = tracked`, `game_barriers = tracked`, `batch_ubo_updates`, `feedback_levels`, and `depth_ro_sticky` on NVIDIA and AMD GPUs.
  - `[nvn] narrow_pass_conflicts = true` (`ab56dd1`, ObeliskChamber): narrower Vulkan memory barrier for attachment pass conflicts.
  - `[nvn] compute_shadows = auto` (`5dfc016`): automatically off on unified-memory iGPUs (avoids redundant shadow copies/barriers), on for discrete GPUs.
  - `[gfx] igpu_compat`: turns off `lean_draws` and `residency` on Intel iGPUs/Arc where needed.
- **Render-thread replay hot paths (`c027497`, `a0d3b44`, `742d778`, `11f6727`, `8685a6a`, `0e10804`, `0fc708d`):**
  - **Descriptor sets & layouts:** 256-entry `layout_cache`, 64-entry `alpha_cache` + `ProgLayout::writes_storage` (EmblemWeave `0fc708d`), precomputed `ProgLayout` texture lists & lazy dummy views (`a0d3b44`), deduplicated graphics SSBO pool/mirror lookups (`3x → 1x resolve_read` in `build_set`, `0e10804`), TSR-jittered camera UBO caching (`a0d3b44`).
  - **Binding & command recording:** fused `Op::BindBuffers` / `Op::BindTextures` single-pass diff+update in `run_list`, skipped no-op bind op bodies & `out_before_op`, skipped `memcpy` on `raw_same`/`raw_list_same` hits, uninitialized `RawBytes` for `CmdList::bytes`, enlarged `textures_note_bound` and `g_gfx_fast` caches to 2,048 slots, 64-slot `compute_pipeline` fast cache, cached Vulkan blend state on `GState::blend_vk` (`0e10804`, `11f6727`, `c027497`).
  - **Compute-out & texture checks:** `out_masks = true` (walk only bound buffer slots), non-TLS `PoolGpu` and `OutOverlapCache` on render thread, memoised servable compute-out slots (`out_draw_read_slot`), fused `tex_bind_sampled` with `ViewRequest` cache + identity-swizzle fast path + folded undefined-layout check, fast-path compressed textures out of `is_color_feedback`, fast-path `attach_targets(false)` (`0e10804`, `8685a6a`, `11f6727`).
  - **Barriers & counters:** batched `ReportCounter`/`ReportValue`/`SignalEvent` writes, tracked texture copy barriers, narrowed transfer barriers (`742d778`).

### Experimental / Non-Default
- **`[nvn] replay_threads = 2` (`nvn_cmdq.cpp`, default `1`):** Offloads `vkCmd*` and `vkUpdateDescriptorSets` to a dedicated `nvn-rec` worker thread (`replay_batch = 16`). Measured 16.8 → 13.6–14.2 ms render-thread work p50 on RTX 3080 at 4K in v0.2.0; needs a fresh A/B on v0.2.13+ (especially after `0e10804` replay optimizations) before considering an `auto` default.

---

## 4. Memory, Pool Mirror, Write-Watch & VRAM Residency

- **Device-local pool mirrors (`[nvn] pool_mirror = on`, `mirror_hot_copy = true`, `storage_read_copy = 2`):** Serves vertices, indices, UBOs, and read-only graphics SSBOs from VRAM instead of host PCIe memory.
  - `gfx_ssbo_readonly = true` and `cs_ssbo_readonly = true` (`ebdb5a1`): SPIR-V reflection (`DescriptorBinding::written` / `atomic`) avoids unnecessary GPU-write invalidation, shadow copy-backs, and barriers on read-only SSBOs.
  - `compute_shadow_min_bytes = 4096` (`ebdb5a1`): skips per-dispatch shadow copy-in/out for small non-atomic buffers.
  - Wine section alias probe (`8398593`): enables `pool_mirror` on NVIDIA Linux under Wine/Proton when the driver supports coherent aliased `VK_EXT_external_memory_host` mappings.
- **Write-watch & mirror bookkeeping (`ebdb5a1`, `ab56dd1`, `11f6727`, `8685a6a`, `273f7ad`, `0e10804`):**
  - Lock-free `Region::has_written` + global `write_seq()` and `O(active)` `g_tv_mirrors`/`g_sig_mirrors`/`g_want_mirrors` lists in `mirror_begin_submit()`.
  - Sequential write-watch fault coalescing (`8685a6a`) and elimination of repeated compute-out guard faults (`273f7ad`).
  - Popcount valid-page summary (`write_watch::count_valid`), word-wise `used`/`gpu_inv` bitmaps, 600-present aging for `sig_pages`, `PoolGpu::has_copy_chunk` fast path, inline `page_valid`/`range_valid`, removal of hot-path `LOCK XADD` atomics.
- **VRAM residency & pressure (`ebdb5a1`, `f1c23e6`, `4a6dce9`, `e4c6ce9`):**
  - Proactive/on-demand LRU mirror chunk eviction (`[nvn] pool_mirror_evict_idle_s = 30`) and budgeted paced mirror/ASTC bursts (`273f7ad`, `f1c23e6`) to prevent post-Dagsion slowdowns and VRAM thrash.
  - `tex_residency_tick()` triggers on raw OS VRAM usage (`max(residency_percent + 5, 85)%`) so VMA fragmentation doesn't stall demotions; unified-memory iGPU & Wine `PagefileUsage` fixes (`d513153`, `e4c6ce9`).
  - `[memory] scale_limits = auto` (`4a6dce9`): scales caches/budgets on ≤20 GiB RAM or ≤8.5 GiB VRAM systems.

---

## 5. Shaders, Pipelines & Cold-Cache Stutter

- **Zero-disk-I/O shader loading:** Full resident SPIR-V preload pack (`[nvn] shader_pack_all = true`, `shader_pack_resident = true`, ~170 MB in RAM) + background `nvn-order-write` thread (eliminated 300–450 ms game-thread HDD/file-cache stalls on first-use shaders).
- **Async pipeline & shader compilation:**
  - `fefw.exe --prepare-shaders` (`0e1afa2`, `37dfcfe`): precompiles shader parts at Setup.
  - `[nvn] compute_async = prebuild`, `prewarm_wait = false`, `gpl_burst_threads = 2`, `gpl_link_cache = false`: cut cold-cache battle worst frame from 142 ms → 44 ms.
  - Frame-time-paced background speculative compiles (`compile_pace_after_ms = 20`, `compile_pace_ms = 8`) and `[nvn] gpl_cold = auto` (off on ≤8 hardware threads) (`ed1af52`).
- **ASTC GPU transcoding:** Enabled by default on Wine/RADV AMD iGPUs (`5dfc016`); slow-GPU sliding-window handoff (`AstcRateWindow`) and `[gfx] astc_gpu_cap_ms` (`4a6dce9`) protect against TDRs.

---

## 6. Kernel, Scheduling, Logging & Movies

- **Kernel & scheduler (`e5fc38d`):**
  - `[kernel] wait_spin = auto`: enables 30 µs wait/lock spinning on `main` and `Basic_Worker` on ≥8 physical cores (sleeping waits kept on ≤6-core CPUs / Steam Deck).
  - `[sched] starvation_lean = true`: cuts starvation guard CPU overhead from ~88 ms/s → ~24 ms/s.
- **Async logging (`ab56dd1`, `efc9a79`):** `[log] async` background writer (bounded 4 MiB queue) and `[log] repeat_limit` per-call-site throttling.
- **AV1 Cutscenes (`7c832b0`, `b14478f`):** `dav1d` built with NASM x86-64 AVX2/AVX-512 assembly (15.8 → 69.7+ fps offline decode; runtime warns if built C-only) plus zero-copy NV12 delivery and frame pacing (`#40`).

---

## 7. Benchmarking & Profiling Workflow

- **Reproducible A/B runs:** Use `tools/perf/ab_run.py` + `tools/perf/perf_report.py` with recorded `.tt` routes (see [docs/PERF_ROUTES.md](docs/PERF_ROUTES.md)). Use `--save-dir` so autosaves don't drift between runs.
- **Built-in runtime telemetry (`runtime/docs/CONFIG.md`):**
  - `[prof] replay_stats = N`, `[prof] gpu_stats = N`, `[prof] gpu_passes = true` (per-pass GPU timestamps, profiling only), `[nvn] out_prog_ab = 1`.
  - `[prof] sample_offsets` (supports plain function names as of `ab56dd1`).
  - GPU breadcrumbs around residency ticks, ASTC transcodes, and texture restores (`5a1617c`, `35d810f`).
  - `tools/perf/hitch_report.py`, `hitch_detail.py`, `prof_hitch.py`.

---

## 8. Open Opportunities / Next Targets

Ordered roughly by risk-adjusted payoff:

1. **Fresh v0.2.13+ baseline profile:** Re-profile Dagsion hub/street and battle maps on current `master` to see the new main-thread vs render-thread vs GPU balance after all v0.2.1–v0.2.13 NVN work.
2. **Validate & enable `FEFW_HLE_CULL`:** Native culling replacements (`0x71019786c0`, `0x7101980580`) are already implemented in [fefw_hle.cpp](game-build/fefw/src/fefw_hle.cpp) and include a verification mode (`FEFW_HLE_CULL=2`). Verify zero mismatches across gameplay and A/B measure worker/main-thread savings (especially `Basic_Worker` where `f_7101980580` was ~8–12% of samples).
3. **Validate & enable `opt.py` `'tregs'`:** Now that ObeliskChamber PR #165 (`4a6dce9`) fixed non-ABI branch syncing in `opt.temps_local` and added `opt_tregs_test.py`, re-run static liveness checks over `fefw` (`main` + `sdk`) and A/B test (~2–3.5% main CPU win when paired with a fresh PGO profile).
4. **Re-test `[nvn] replay_threads = 2`:** Now that render-thread replay has been heavily optimized (`c027497`, `0e10804`), A/B test `replay_threads = 2` across NVIDIA/AMD/Windows/Proton to decide whether it still yields a win worth making `auto` or whether it's redundant.
5. **Target remaining worker/main lifted hotspots for `inline_replace`:**
   - `f_71019730e0` (MOCull NEON rasterizer — 80–84% of `MOCull_Worker1/2`).
   - `f_71005ef6e0` / `f_7100a831a0` on `Basic_Worker1-4`.
6. **Further `opt.py` codegen wins:**
   - Vector temporaries `v16..v31` (AAPCS64 caller-saved, never synced at calls/returns; ~1% static win).
   - Lazy-loading `xregs` (callee-saved `x19..x28` across calls with dirty tracking, avoiding the live-range bloat of eager loading).
7. **Hardware DTLB-miss check:** Run one Dagsion street capture under `perf stat` (Linux) or AMD uProf (Windows) to measure actual L2 DTLB miss rate on `main` before deciding whether guest-heap 2 MiB large pages are worth pursuing.
