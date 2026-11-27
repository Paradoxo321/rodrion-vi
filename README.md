<p align="center"><img src="docs/klbf/logo.svg" width="120" alt="rodrion-vi"></p>
<h1 align="center">rodrion-vi — VIctory AMD RDNA 2 / RX 6600 Unified Edition</h1>
<p align="center">
  <img src="https://img.shields.io/badge/base-KLBF%20Lab%2039%20%2B%207900XT%20Shaders-ff7a00" alt="Base: KLBF Lab 39 + 7900XT">
  <img src="https://img.shields.io/badge/tested%20on-RX%206600%208GB%20%2B%20SAM%2FReBAR-e11d48" alt="Tested on RX 6600 8GB">
  <img src="https://img.shields.io/badge/memory-Tiled%20Physical--Page%20Pool%20%2B%20UMA-06b6d4" alt="Tiled Physical-Page Pool + UMA">
  <img src="https://img.shields.io/badge/license-GPL--3.0-22c55e" alt="License: GPL-3.0">
</p>

> [!CAUTION]
> **⚠️ NO GAME FILES INCLUDED**
>
> This repository contains **only the source code for the compatibility layer, launch scripts, and launcher**. No executables, assets, shaders, textures, logs, or proprietary game data are distributed here. **Every user must supply their own legally obtained copy** and configure local paths in `victory.toml` (copied from `victory.example.toml`).

---

## Real-World Benchmarks (RX 6600 8 GB + Ryzen 5 4500)

Tested on **Windows 11**, **AMD Radeon RX 6600 8 GB** (**Above 4G Decoding + SAM / ReBAR enabled**, `TiledResourcesTier = 4`, native RDNA 2 Wave64), **AMD Ryzen 5 4500 (6C/12T, 8 MB L3)**, and **16 GB DDR4** — **Series S + FSR 2.4 + FrameGen (720p)** mode, **0 crashes / 0 TDRs / 0 panics**:

| Version | Overall Median Frametime | Real FPS (Med) | Presented FPS w/ FrameGen | 2,000–4,000 draws | 8,000+ draws | Key Changes |
|---|---|---|---|---|---|---|
| **v1.2.0** | `282.8 ms` | `~3.5 FPS` | `~7.0 FPS` | — | — | CPU↔GPU Boost baseline (`BATCH_RANGE_FLUSH`, `WATCH_READ_FAST`, `GUARD_ABSENT_FAST`, `TILED_LOCK_BATCH`, `COPY_ON_GPU`, `CARD_SYNC_INLINE`) |
| **v1.3.0** | `167.7 ms` | `6.0 FPS` | `12.3 FPS` | `150.8 ms` | — | `VICTORY_HEAP_RECORD_FAST=1` + `VICTORY_TEXTURE_ADAPTIVE=1` (`heap_textures` 24.8 → 5.4 ms) |
| **v1.4.0** | `180.7 ms` | `5.5 FPS` | `11.8 FPS` | `150.3 ms` | — | 64-sharded `Notes` (`VICTORY_PORT_JOURNAL=1`) + `VICTORY_WATCH_UNLOCKED_ASK=1` + pipeline worker panic recovery |
| **v1.5.0** | `152.9 ms` | `6.5 FPS` | `13.7 FPS` | `137.9 ms` | `459.3 ms` | `VICTORY_TABLE_MEMO=1` (81–84.5% hits on ~87k–145k table entries/frame) |
| **v1.6.0** | **`125.2 ms`** (`p10 93.0`) | **`8.0 FPS`** (`p90 10.8`) | **`16.9 FPS`** | **`119.1 ms`** (`8.4 FPS`) | **`265.2 ms`** (`−42%`) | `LOOSE` `RwLock`, `VICTORY_PORT_CONSTANTS=1` (185 MB 32-bit heap in-place), `VICTORY_REFRESH_COALESCE=1`, `VICTORY_ROOT_REACH_MS=1000` |

### v1.6.0 Scene Density Breakdown (Open World)
- **100 – 2,000 draws:** `86.8 ms` (**11.6 FPS real** / ~23 FPS FG)
- **2,000 – 4,000 draws:** `119.1 ms` (**8.4 FPS real** / ~17 FPS FG)
- **4,000 – 8,000 draws:** `187.1 ms` (**5.3 FPS real** / ~11 FPS FG)
- **8,000+ draws:** `265.2 ms` (**3.8 FPS real** — `heap_buffers` cut from `200.7 ms` → `75.1 ms`, −63%)

---

## Key Improvements in This Edition

### 1. Unified Memory — Tiled Physical-Page Pool (`VICTORY_TILED_POOL=1`)
- Uses **`CreateReservedResource` + `UpdateTileMappings`** backed by 64 MiB `D3D12_HEAP_TYPE_CUSTOM (L0, Write-Back)` chunks with persistent CPU `Map()`.
- Maps console 64 KiB physical pages (`xmem`) directly to D3D12 tiles, saving **~50% RAM** on sparse holes (`NULL` tiles = 0 bytes) with per-page reference counting and deferred retirement (`host::retire`).
- Range-based mapping invalidation (`VIEW_LOG` ring buffer in `inplace.rs`), eliminating unnecessary global rescans on unrelated `MapView`/`UnmapView` calls.
- Lock-free VEH transit barrier in `xmem.rs` for safe concurrent page remaps without access violations.

### 2. Full AMD SAM / ReBAR Utilization (`D3D12_HEAP_TYPE_GPU_UPLOAD`)
- Dynamic buffer and texture uploads up to **32 MiB** write directly into GDDR6 VRAM over PCIe BAR (zero extra RAM staging copy) when **Above 4G Decoding + Resizable BAR / SAM** are enabled in BIOS.
- Combines `VICTORY_CARD_WINDOWS_MB=512` + `VICTORY_CARD_BUFFERS_MIN_KB=256` (GPU compute shader hashing & comparison) with the **Tiled Pool** for remaining windows.
- Tuned 8 GB VRAM management (`VICTORY_VRAM_IDLE_MS=3000`, `VICTORY_TEXTURE_SHORT_SIDE=128`, `VICTORY_TEXTURE_CATCHUP=1`) to eliminate texture eviction thrashing.

### 3. Native RDNA Shader Translator (`YukiRyuu/VIctory-7900XT`)
- Automatic AMD RDNA detection (`set_host_rdna(true)`, native Wave64 + `VICTORY_RDNA_FLOAT` emitting single-instruction `v_max_f32`, `v_mul_legacy_f32`, `v_med3_f32`, `v_fract_f32`).
- `QuadRead` helper-lane fixes (resolves artifacts on paint, glass, and skin on AMD), 9-bit mesh primitive indices, and 100% fingerprint parity between offline shader pre-compilation and runtime (`pipeline/host.rs`).
- Extended root-view windowing (`VICTORY_ROOT_WINDOW_MB=12`) preventing `device removed` faults.

### 4. CPU Hot-Path & Sync Optimizations (v1.2.0 – v1.6.0)
- **64-sharded write-watch journal (`VICTORY_PORT_JOURNAL=1`)** + lock-free read-fast / guard-absent paths (`VICTORY_WATCH_READ_FAST=1`, `VICTORY_GUARD_ABSENT_FAST=1`, `VICTORY_WATCH_UNLOCKED_ASK=1`).
- **Per-command-list descriptor table memoization (`VICTORY_TABLE_MEMO=1`)** — 81–84.5% hit rate across ~87k–145k descriptor table entries per frame.
- **32-bit graphics heap in-place (`VICTORY_PORT_CONSTANTS=1`)** + `LOOSE` `RwLock` + **`Step::Refresh` coalescing (`VICTORY_REFRESH_COALESCE=1`)** + **`VirtualQuery` reach cache (`VICTORY_ROOT_REACH_MS=1000`, 98.8% hit rate)**.
- **Fast 12-byte `[tag, i, gen]` heap buffer pass (`VICTORY_HEAP_RECORD_FAST=1`)**, adaptive texture upload budget (`VICTORY_TEXTURE_ADAPTIVE=1`), and lock-free Tiled Pool sync (`VICTORY_TILED_LOCK_BATCH=1`).
- **Fast 4-lane `u64` texture hashing (`fast_hash64`)**, persistent worker pool (`wide_run`), and vectorized `back_piece` & `changed_runs`.
- **Pipeline worker panic recovery** (`pipeline/make.rs`) ensuring shader worker panics never stall submission threads.

### 5. Custom Launcher, Controls, Discord RPC & Quality of Life (`launcher/Program.cs`)
- **Dual-engine & version selector** (Motor 1 / Motor 2) with PT-BR / EN-US language toggle.
- **Discord Rich Presence (`VICTORY_DISCORD=1`)** over local Discord IPC pipe with zero extra crates.
- **Customizable Xbox → Keyboard remapper (`VICTORY_KEYMAP`)**, **Mouse Camera (`VICTORY_MOUSE_CAMERA=1`)**, and click-to-hide cursor (`F9` toggle).
- **CPU↔GPU Boost toggle**, **Unified Memory vs Classic AMD Mode**, **Lite Graphics Mode**, and **AMD FSR 2.4 + Frame Generation**.

---

## Build & Run

### Prerequisites
- **Windows 11** + **Rust** (toolchain specified in `rust-toolchain.toml`)
- DirectX 12 GPU with Shader Model 6.6 and Tiled Resources Tier 3/4 (recommended: **Above 4G Decoding + SAM/ReBAR enabled in BIOS**).
- Your own legally obtained game files stored **outside** this repository.

### 1. Configure Local Paths
Copy `victory.example.toml` to `victory.toml` (git-ignored) and set `game_dir`, `cache_dir`, and `data_dir` to your local directories (always outside the repository).

### 2. Build `victory.exe`
```powershell
cargo build --release --bin victory
```

### 3. (Optional) Build the Windows Forms Launcher
```powershell
& 'C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe' /nologo /target:winexe /optimize+ `
  /out:GTA6-Launcher.exe /r:System.Windows.Forms.dll /r:System.Drawing.dll launcher\Program.cs
```
*(Update the local directory paths at the top of `run-klbf-lab39.ps1` and `launcher\Program.cs` to match your setup.)*

### 4. Pre-Compile Shaders (Recommended First Run)
```powershell
$env:VICTORY_CONFIG = ".\victory.toml"
$env:VICTORY_SHADER_OPT = "0"
$env:VICTORY_SKIP_PIPELINES = (Get-Content docs\klbf\skip-amd.txt -Raw).Trim()
.\target\release\victory.exe translate --cache --pipelines --jobs 8 "<your_game_dir>\shaders.rpf"
```

### 5. Launch
```powershell
.\run-klbf-lab39.ps1 -Res720p
# With AMD FSR Frame Generation:
.\run-klbf-lab39.ps1 -FrameGen -Res720p
# Classic AMD Mode (without Tiled Pool / UMA):
.\run-klbf-lab39.ps1 -Res720p -NoUnifiedMemory
```

---

## References, Credits & License

This project is an experimental fork of **VIctory** licensed under **GPL-3.0** (see [LICENSE](LICENSE)), building upon work from the community:

- **Upstream VIctory** & **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)** — core D3D12.X translation, rendering, and texture/view-heap work.
- **[PastiiJay/klbf-lab](https://github.com/PastiiJay/klbf-lab)** / **[klbf-lab-beta](https://github.com/PastiiJay/klbf-lab-beta)** (Lab 39) — water physics, Series S menus/HUD, FSR Frame Generation, resumable pipeline pre-build, and AMD fixes.
- **[YukiRyuu/VIctory-7900XT](https://github.com/YukiRyuu/VIctory-7900XT)** — native RDNA shader translator (`set_host_rdna`, `VICTORY_RDNA_FLOAT`, QuadRead helper lanes, 9-bit mesh indices), `root_window` (`VICTORY_ROOT_WINDOW_MB`), and AMD optimizations.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)** — `VICTORY_DYNAMIC_SLOTS`, `VICTORY_MERGE_DWORDS`, `VICTORY_PIPELINE_GATE_S`, partial mip streaming, and >65,535-group dispatch splitting.
- **[GPUOpen-LibrariesAndSDKs/FidelityFX-SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK)** — AMD FidelityFX SDK 2.4 (FSR + Frame Generation).
- **rorodev6 / shadowdog00 & [rorodev6/victory-megafork-amd](https://github.com/rorodev6/victory-megafork-amd)** — `VICTORY_LATE_WAIT=3`, `VICTORY_TEXTURE_BATCH`, `VICTORY_GROUPED_MESH_CLAMP`, Port 7 64-sharded journal (`VICTORY_PORT_JOURNAL`), and pipeline worker panic recovery.
- **[Op-lite/victory-scarlett](https://github.com/Op-lite/victory-scarlett)** — `VICTORY_TABLE_MEMO` (per-list descriptor table memoization) and `CARD_AWAKE=0` tuning.
- **[gleaf-6/VIctory-7900XTX-](https://github.com/gleaf-6/VIctory-7900XTX-)** — `LOOSE` `RwLock`, `VICTORY_PORT_CONSTANTS`, `VICTORY_REFRESH_COALESCE`, and `VICTORY_ROOT_REACH_MS`.
- **realblastex/flamingo** — Discord Rich Presence local IPC integration (`apps/victory/src/discord.rs`).
- **ShakiraRED** — `VICTORY_COPY_PLAIN`, `VICTORY_WRITE_PLAIN`, and `VICTORY_STAGING_POOL`.
- **Trexies** — `VICTORY_CLAMP_WAITS=0` (removing global UAV stalls on indirect clamps).
- **triippywhale** — AMD view-heap clamping and dynamic descriptor slot scaling.
- **Geckomacabre/vx-layer** & **MiamiGame/XTP** (Mesa `addrlib` tables) — ocean water physics and XOR swizzle addressing.
- **Belobug/VIctory-Belobug-AMD** — `ambient.rs` (`VICTORY_AMBIENT`).
- **JappertjeV**, **PalmVIDev**, **avatarmanz**, **ka1qux**, **clems206** — diagnostics and upstream fixes.

> Product names and trademarks are referenced solely to describe compatibility and belong to their respective owners. **No game files are included in this repository.**
