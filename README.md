# VItoriaBr

<a id="english"></a>

## Overview

**VItoriaBr** runs **an Xbox Series X|S title** natively on Windows from files the user supplies.

The title's x86-64 code runs unchanged inside our process; **VItoriaBr** provides the underlying console platform and translates shaders/resources (D3D12.X / GFX10 → host D3D12 / DXIL). See [docs/architecture.md](docs/architecture.md). For an illustrated introduction to why an Xbox game cannot run directly on a PC and how each part of the project addresses this, see the [project overview](docs/visao-geral.md).

> **Upstream & License:** Derived from [Marvisak/VIctory](https://github.com/Marvisak/VIctory) with GPL-3.0 world-rendering, lighting, relay/VRAM, and DXIL optimizer improvements integrated from [fxceless/victory-fork](https://github.com/fxceless/victory-fork), licensed under **GPL-3.0-or-later** ([LICENSE](LICENSE)). **No game files or derived assets are shipped or allowed in this repository** ([docs/asset-policy.md](docs/asset-policy.md)).

## Status & Progress (2026-11-25)

**Active development — 3D open world rendering live!** Boot, menus, intro video with audio, profile/saves, and loading work at 30 fps; inside the 3D open world on RTX 5060 Ti it runs with **zero TDR** at **up to `6 fps`** (`3,500–4,400 draws/frame`), with depth-bounds lighting, ocean/water, vehicles, HUD, and subtitles.

### Real Hardware Test Results (NVIDIA Blackwell)

Tested on: **Windows 11 Pro (build 26300)** · **Ryzen 7 5700X** · **40 GB RAM** · **NVIDIA RTX 5060 Ti 16 GB** (SM 6.6 + Mesh Shaders, driver `621.24`), Xbox Series build `1.0.2.1`.

| Stage | Status |
|-------|--------|
| Boot & first-boot screens (accessibility, language, settings, legal) | ✅ Works — **30 fps** |
| Intro video with audio | ✅ Works |
| Start screen, account selector, profile & persistent saves | ✅ Works |
| Loading into the world (~50 s) | ✅ Works |
| 3D world (opening cutscene, first room & open world) | 🟢 **Rendering live on RTX 5060 Ti (up to `6 fps`, zero TDR, `1,700–4,400 draws/frame`)** — photorealistic skin, depth-shaded ambient light (`VICTORY_AMBIENT=0.6`), terrain/foliage and shadows working |

### Measured Shader & PSO Coverage on RTX 5060 Ti (`1.0.2.1`)

Measured via `victory translate --validate --pipelines` (`dxil.dll` Windows SDK `10.0.26100.0` + NVIDIA driver `621.24`):
- **Containers scanned:** `26,599` (`26,148` programs translated & 100% structured — `40.6M` IR instructions).

| Stage | DXIL Emitted | Validated (`dxil.dll`) | Host PSOs (RTX 5060 Ti) | PSO Rate |
|-------|--------------|------------------------|-------------------------|----------|
| **Vertex + Pixel** | `14,248 / 14,259` | `14,248` | **`14,248`** | **100.0%** |
| **Mesh** | `8,765 / 8,765` | `8,694` | **`8,765`** | **100.0%** |
| **Compute** | `2,728 / 2,734` | `2,728` | **`2,331`** | **85.4%** |
| **Total** | **`25,741 / 25,758` (99.9%)** | **`25,670`** | **`25,344`** | **98.5%** |

> The `397` refused Compute PSOs are wave64-only shaders (NVIDIA GPUs run wave32).

### VItoriaBr Improvements

- **`fxceless/victory-fork` synchronization (`4f9a19d`, `b401954`), stability fixes, and community PRs (`32d7848`, `aa85bb0`, `faca57e`, `d396d11`, `06161b9`):**
  - Depth-bounds testing (`#82`), complete first-room rendering (`a26859f`), inexpensive descriptor-sync fingerprints (`9376fd6`), and `VICTORY_RELAY=2` with a `relay_land()` race fix (`aa85bb0`).
  - Time/byte guards (`PASS_BUDGET`, `faca57e`) in `heap_textures` prevent the 30-second watchdog from firing on the first 3D frame.
  - Upstream stability fixes cover loader-lock deadlocks, XMA wrap/continuation, dynamic descriptor wraparound, and the `SHADER_HEAPS` use-after-free in `mirror()`/`make_waiting()`.
  - Community PRs: **`#6` (`victory warm`)**, **`#3` (portable Linux CI and the project overview)**, **`#7` (16-byte stride fix in `SLOT_VIEWS`)**, **`#10` (RDNA2 LDS bounds checks)**, **`#11` (texture formats 189–191)**, and **`#12` (packed `dot4AddPacked` products)**.
- **Signature and interpolation reconciliation** (`c0b60a0`, `crates/victory-shader/src/emit/{shader,container}.rs`):
  - Restored `reconcile_row_interp`, which the large merge had removed: elements packed into the same row share one interpolation mode, fixing `dxil.dll` failures in **~250 shaders**.
  - `ISG1`/`OSG1` elements now follow console order instead of register order, resolving **33 input-signature failures** and **39 output-signature failures**.
  - The v7 result was **`25,588` validated shaders**, up from `25,413`.
- **Computed-index table descriptors read as data (`SLOT_VIEWS`)** (`b62bbc0`, `18df163`, `77ad907`, `crates/victory-shader/src/emit/lower.rs` and `crates/victory-gpu`):
  - The new `SLOT_VIEWS` root SRV (252) exposes each heap slot's buffer-view address, length, and stride. The emitter uses it when the console reads a **descriptor address** selected by a runtime-computed index.
  - Array-texture `calculateLOD` now leaves the slice coordinate `undef`, as DXC requires, eliminating the three `out of bound coord must be undef` failures.
  - The v13 result was **`25,741 / 25,758` DXIL shaders (99.9%)**, **`25,670` validated**, and **`25,344` PSOs**. The `descriptor read only as data` (40), `out of bound coord` (3), and `descriptor chosen by control flow` (40) failure categories were **cleared**.
- **Two immediate samplers selected by a branch** (`67bb579`, `c248547`):
  - The phi between constant samplers becomes a **static sampler range** handle with a dynamic index. Selection uses a **path phi at the join** (true from the second predecessor), not `reach[pred]`, which defined the value in one predecessor without dominating the join and invalidated the bitcode.
  - All **40** affected shaders (`37 PS_TexturedLit` and `3 MST_DS`) now emit, validate, and create PSOs.
- **Full `Shadow` buffer D3D12 resource state tracking in `HostList`** (`crates/victory-gpu/src/{shadow,host,indirect,draw}.rs`):
  - Previously, `indirect::done()` left shared title buffers stuck in `D3D12_RESOURCE_STATE_UNORDERED_ACCESS`, causing `IASetVertexBuffers` and barrier mismatch (`id=527`) errors in the D3D12 Debug Layer.
  - Each `Shadow` buffer now tracks its state across lists via `prelude()` and intra-list barriers (`GENERIC_READ` for VB/IB/CBV/SRV/indirect `args`+`counter`, `UNORDERED_ACCESS` for UAV/clamp), **eliminating all buffer state errors in the D3D12 Debug Layer** and significantly extending 3D world execution on the RTX 5060 Ti.
- **`MIMG` `a16` support and LDS byte/half operations** (`2471a62`, `crates/victory-shader/src/emit/lower.rs`):
  - `image_sample_b`, `_cl`/`_o`, `image_gather4_l`, plus thread-safe byte/half reads and writes in LDS (`ds_read_{u8,i8,u16,i16}`, `ds_write_{b8,b16}{,_d16_hi}`).
- **Per-row DXIL interpolation mode reconciliation on the signature** (`5943771`, `crates/victory-shader/src/emit/shader.rs`):
  - Elements sharing the same signature row now emit the same interpolation mode, **clearing ~48 `dxil.dll` validator errors**.
- **Immediate recycling of dynamic descriptor-heap slots** (`a65d054`, `bd33acc`, `crates/victory-gpu/src/{descriptor/host,draw,list,queue}.rs`):
  - Slots return to the heap as soon as the synchronous `host.execute` completes, and `MIN_DYNAMIC` rises to **`2^18` (262,144)** on capped heaps (NVIDIA, 1,000,000-view limit), **eliminating the 16,384+ dropped draws/dispatches** in the 3D world.
- **Cubemap `image_sample_d_cl`** (`77b4b4c`, `crates/victory-shader/src/emit/lower.rs`):
  - Uses `grads = 2` for `deriv_dwords` on cubemaps (`kind == 5`), unblocking +27 shaders.
- **Mesh Shader, LDS, and barrier safety on NVIDIA Blackwell (wave32)** (`bd33acc`, `a13a719`, `1ece3a6`, `911267c`, `808a209`, `crates/victory-shader/src/emit/lower.rs`):
  - `mesh_prologue` caps `setMeshOutputCounts` to thread count so no uninitialized primitives are left; `emitIndices`, `storeVertexOutput`, and `storePrimitiveOutput` enforce `max_vertices`/`max_primitives` bounds.
  - All `LDS` reads, writes, and atomics enforce RDNA2-accurate OOB semantics (read `0`, drop writes) to prevent NVIDIA shared-memory traps.
  - `s_barrier` in non-post-dominating, loop, or post-`abort_block` blocks emits a memory-only fence (`TGSM_FENCE`/`UAV_FENCE`), preventing `BAR.SYNC` deadlocks on wave32.
- **Anti-TDR submission chunking & indirect bounds** (`1ca533d`, `8a75883`, `6f99f75`, `crates/victory-gpu/src/{host,indirect}.rs`):
  - `host::execute()` submits one command list per fence wait; `indirect.rs` zeroes out-of-range `DispatchMesh` arguments and caps valid dispatches (`4096` groups, `BUDGET = 2^22`).
  - `VICTORY_SHADER_LOOP_LIMIT` enabled in `release` builds as well (default `4096`).

> **Warning:** Entering the 3D world on a real GPU can still trigger a display driver reset (TDR). Use a `diag` build with `VICTORY_D3D12_WARP=1` to inspect safely on the software adapter.

## Quick Start

**Requirements:** Windows 10/11 x64, an AVX2-capable CPU, a Direct3D 12 GPU with Shader Model 6.6 and Mesh Shaders, Rust (see `rust-toolchain.toml`, using the `x86_64-pc-windows-msvc` toolchain), VS 2022 MSVC, and the Windows SDK.

> The title requires several GiB of available Windows commit memory. If `crashes.log` reports error `1455`, close other applications or increase the paging-file size.

```powershell
copy victory.example.toml victory.toml   # set paths outside the repository
cargo run -p victory -- config
cargo test --workspace
cargo build --release -p victory
.\target\release\victory.exe warm <game_dir>   # optional: pre-translate shaders into the cache first

# Recommended preset tested on RTX 5060 Ti 16 GB (photorealistic skin/water/foliage, zero black skin, zero TDR)
$env:VICTORY_RELAY = "2"
$env:VICTORY_QUEUE2 = "1"
$env:VICTORY_ASYNC_SUBMIT = "1"
$env:VICTORY_COPY_PLAIN = "1"
$env:VICTORY_TEXTURE_NO_WAIT = "1"
$env:VICTORY_CARD_BUFFERS_MB = "2048"
$env:VICTORY_TEXTURE_BUDGET_MB = "10240"
$env:VICTORY_PAGE_WRITES = "slots"
$env:VICTORY_AMBIENT = "0.6"
$env:VICTORY_WIDE_REFRESH = "1"
$env:VICTORY_WIDE_MERGE = "1"
$env:VICTORY_JOIN_COPIES = "1"
$env:VICTORY_DISPLAY_MODE = "1920x1080"

.\target\release\victory.exe run --execute <game_dir>\<title_exe>
```

For shader precompilation, its scope, and how to verify cache use in logs, see the [shader precompilation guide](docs/precompilacao-shaders.md).

Before playing, `.\target\release\victory.exe warm --jobs 4` caches archive
translations and compiles compute PSOs into the same pipeline library used by
the game. `--no-pipelines` retains translation-only warming. No shaders are
dispatched during warming; graphics PSOs still need the title's draw state.
The portable preload and shared loading-window behaviour follows
[DexNom/VIctory](https://github.com/DexNom/VIctory/tree/2e01ab4760db91820ef5f9a9f85761f83cd6b5d0),
without importing AMD-specific renderer paths.

Pipeline waits now share elapsed time across recording threads, independently
for compute and graphics (12 seconds per kind by default). This avoids dropping
one-time loading passes just because another recording thread is waiting.
`VICTORY_PIPELINE_WAIT_SHARED=reserve` restores the old reservation scheme;
lower `VICTORY_PIPELINE_WAIT_MS` values may again lose one-time passes.

### Tuning and Diagnostics Findings

- **Black skin or ground (VRAM pressure / streaming):** on the tested 16 GB card, `VICTORY_CARD_BUFFERS_MB=2048` and `VICTORY_TEXTURE_BUDGET_MB=10240` (`1024` / `5120` on 8 GB cards) avoid the allocation limit that blocked new skin/terrain textures. `VICTORY_PAGE_WRITES=slots` requeues placed textures uploaded before the title finished writing their bytes, recovering 7,000+ textures per session.
- **`VICTORY_QUEUE2=1`, `VICTORY_ASYNC_SUBMIT=1`, and `VICTORY_COPY_PLAIN=1`:** reduce `copying back` by **~3×** (`~205 → ~65–78 ms`) through the dedicated copy queue, handle ~12,400 in-place copies/frame (`~290 MB/frame`) without kernel syscalls, and halve texture work (`~40 → ~20 ms`) in those measurements.
- **`VICTORY_AMBIENT=0.6` (default):** enables depth-shaded ambient lighting, ported from `Belobug/VIctory-Belobug-AMD` under GPL-3.0, to light characters, skin, vehicles, water, and interiors while full RTGI is not yet working. Use `0` to disable ambient lighting.
- **`VICTORY_TEXTURE_NO_WAIT=1`:** avoids title-memory read-lock stalls during texture refreshes (`textures` dropped from `900–3,700 ms` to `6–25 ms` in the recorded runs).
- **`VICTORY_DISPLAY_MODE=1920x1080`:** reports a 1080p display mode to the title; it is not a direct override of the title's internal rendering resolution.
- **`VICTORY_CARD_FETCH_INFLIGHT` (leave off / `0`):** enabling it reduced `copying back` from `~162 ms` to `~45 ms`, but overloaded the GPU (`on the GPU` increased from `~50–65 ms` to `~200–330 ms`), lowering overall RTX 5060 Ti performance (`1.9–2.6 fps` versus `3.5–6.0 fps`).
- **`VICTORY_BVH_BUILDER=1` (experimental RTGI):** reaches **50/53** compute shaders on wave32. The remaining shaders (`39321a95223001ed`, `77360df21e12ea43`, `9fc111d3be79364f`) fail in divergent `v_readfirstlane_b32` scans. `VICTORY_FIRSTLANE_OWN=1` reaches **53/53** and activates the builder, but corrupts AABB offsets in memory and crashes after ~18 seconds. Leave it off until exchange between wave64 halves is fully implemented.

Texture passes alternate new uploads and retries to prevent starvation. Set `VICTORY_TEXTURE_FAIR_QUEUE=0` for the previous ordering. Inspired by [DexNom/VIctory](https://github.com/DexNom/VIctory/commit/5f4c3df44197eb03b82205d2da871ded8d0ccf16).

For an experimental copy-back comparison, set `VICTORY_BACK_OWN_VIEW=1` before launching. It can reduce redundant texture and buffer refreshes by writing shader results through a separate view of title memory. It is off by default and has not been validated in gameplay on the RTX 4070; disable it if rendering or stability worsens.

To experiment with shorter texture-staging passes, set `VICTORY_TEXTURE_CATCHUP=0` and `VICTORY_TEXTURE_STAGE_MS=20`. This is a per-pass, best-effort limit, not a frame-time guarantee; it may delay texture uploads and increase missing textures. Both settings are optional.

Streamed single-layer tiled 2D textures now reuse unchanged detiled mip levels; packed tail mips are checked together. Physical-page aliases also reuse section-wide commitment instead of asking Windows to commit the same pages again. These optimizations are automatic, preserve the existing VRAM limits, and do not enable experimental on-demand texture allocation.

System-memory cache reclamation considers both available physical RAM and
Windows commit headroom. Below 4 GiB of either, unused snapshot/upload-buffer
pools shrink to one quarter of their normal limits; below 2 GiB they are
emptied by the next sweep. Referenced resources and GPU-written texture
contents remain protected; required rendering allocations are not capped.

Uploads and CPU copies wait for overlapping in-flight GPU writes by default;
independent ranges can still be prepared ahead. `VICTORY_FILL_WAIT=1` makes
this explicit. The experimental value `0` permits speculative reads of old
data: detecting the conflict afterwards cannot repair a submitted draw.

The experimental `VICTORY_WAVE_WIDE=1` retry lets wave32 GPUs such as the
RTX 4070 run compute groups consisting of one 64-lane console wave as two
hardware warps with shared-memory exchanges. It reconverges branches,
shared-memory accesses, and return paths before inserting synchronization,
and refuses the retry if it cannot produce an exact translation, rather
than falling back to approximate half-wave results. This is GPU
work within one process, not two emulator processes. The retry remains
off by default pending broader gameplay validation. Divergent groups
containing multiple console waves are not yet supported by this retry;
enabling it does not establish full rendering compatibility.

### Diagnostics Build (`diag`)

```powershell
cargo build --profile diag -p victory --features diagnostics
```

Key diagnostics environment variables:
- `VICTORY_D3D12_WARP=1`: forces the software WARP adapter (slow, avoids GPU-driver TDR).
- `VICTORY_D3D12_DRED=1`: records DRED breadcrumbs in `crashes.log` after `DEVICE_HUNG`.
  The startup log confirms whether breadcrumb/page-fault reporting was enabled.
- `VICTORY_D3D12_DEBUG=1` / `VICTORY_D3D12_GBV=1`: enables the D3D12 Debug Layer and GPU-Based Validation.
- `VICTORY_CAPTURE=<directory>` / `VICTORY_CAPTURE_FRAMES=<list>` / `VICTORY_DUMP_TARGETS=1`: saves PNG frames and render targets.
- `VICTORY_SHADER_LOOP_LIMIT=<n>`: limits loop iterations in translated shaders.
- `VICTORY_SKIP_PIPELINES=<hash>,...`: skips draws/dispatches for specific hashes.

### Controls (Keyboard When No XInput Controller Is Connected)

| Keys | Controller |
|------|------------|
| `W A S D` / `I J K L` | Left / right analog stick |
| Arrow keys | D-pad |
| `Enter`, `Backspace`, `X`, `Y` | A, B, X, Y |
| `Q` / `E`, `Z` / `C` | LB/RB, LT/RT |
| `F` / `H` | L3 / R3 |
| `Esc`, `Tab` | Menu, View |

### Persistent Runtime Settings

Copy `victory.example.toml` to the git-ignored `victory.toml` in the working
directory (or select another file with `VICTORY_CONFIG`). Set `[paths].game_exe`
to launch with `victory run --execute` without repeating the executable path.
The existing positional path still overrides this default.

Set `[paths].shader_cache_dir` to share a shader/pipeline cache between `run`,
`warm`, and `translate`; it defaults to `<cache_dir>/shaders`.
Set `[paths].log_dir` to append plain-text tracing output to `victory.log`
there, while keeping console output. Leave it unset for console-only logging.
`VICTORY_GAME_EXE`, `VICTORY_SHADER_CACHE_DIR`, and `VICTORY_LOG_DIR` override
these paths. All output paths must remain outside the repository.

Put quoted runtime flag values in `[environment]`, for example
`VICTORY_RELAY = "2"`. They load before logging, shaders, and GPU initialization.
Only uppercase `VICTORY_*` keys are accepted; `VICTORY_CONFIG` must be set
outside the file. Inherited environment variables override file defaults;
path variables in this table override the corresponding `[paths]` settings.
Restart the emulator after editing the file; settings are not hot-reloaded.
Some debug switches (including `VICTORY_D3D12_WARP`, `VICTORY_D3D12_DEBUG`,
`VICTORY_D3D12_DRED`, and `VICTORY_GPU_TIMING`) test only whether a variable
exists: omit them to keep them off, rather than setting them to `"0"`.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
