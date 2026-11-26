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

Tested on **Windows 11**, **AMD Radeon RX 6600 8 GB** (**Above 4G Decoding + SAM / ReBAR enabled**, `TiledResourcesTier = 4`, native RDNA 2 Wave64), **AMD Ryzen 5 4500 (6C/12T, 8 MB L3)**, and **16 GB DDR4** — **Series S + FSR 2.4 (720p)** mode:

| Metric | Measured Result (Open World) | Notes |
|---|---|---|
| **Real FPS (~2,000–4,300 draws/frame)** | **4.5 – 5.9 FPS** (`168 – 222 ms/frame`) | CPU-bound on Ryzen 5 4500; pause menu at **28.5 FPS** (`35 ms`) |
| **Real FPS (heavy ~5,000–6,000 draws/frame)** | **3.8 – 4.9 FPS** (`205 – 263 ms/frame`) | Before optimizations + SAM: `2.6 – 2.8 FPS` (`352 – 381 ms`) |
| **Perceived fluidity with AMD FSR FrameGen** | **~8 – 12 displayed FPS** | Uses RX 6600 headroom with no loss of real FPS |
| **GPU time (RX 6600)** | **4.7 – 15.7 ms/frame** (~51% busy) | Native RDNA shaders (`rdna=true`, `compute=64`) |
| **VRAM usage (RX 6600 8 GB)** | **5.5 GB / 6.7 GB** (~1.2 GB free) | **0 evictions/s** (previously 182 evictions/s & 52 GB re-uploads) |
| **Tiled Physical-Page Pool (`VICTORY_TILED_POOL=1`)** | **739–822 windows (3.5–4.2 GB virtual) in ~1.15 GB physical** | **50.5% RAM saved** via `NULL` hole tiles, zero `device removed` |
| **GPU offload with SAM/ReBAR (`rebar_hw=true`)** | **239 copies (1,110 MB) in VRAM** • **380 hash + 125 compare passes/frame on GPU** | CPU write-compare cut by **87%** (`17.3 MB` → `2.2 MB`; CPU buffers: `0.0 MB`) |

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

### 4. CPU Hot-Path Optimizations
- **Fast 4-lane `u64` texture hashing (`fast_hash64`):** replaces cryptographic `BLAKE3` for texture change detection (~6–8x faster) + configurable `VICTORY_TEXTURE_HASH_EVERY=32`.
- **Persistent worker pool (`wide_run`)** for texture refreshes (`texture.rs`), parallel Tiled Pool `find_all` comparisons (`shadow.rs`), and coarse-chunked `Batch::flush` (`host.rs`).
- **Vectorized `back_piece` & `changed_runs` (32/8-byte skips)** without redundant zero-fills.

### 5. Custom Launcher, Controls & Quality of Life (`launcher/Program.cs`)
- **Customizable Xbox → Keyboard remapper (`VICTORY_KEYMAP`):** interactive UI to rebind all 24 Xbox buttons/axes (e.g. `X = K`) with automatic conflict resolution.
- **Mouse Camera (`VICTORY_MOUSE_CAMERA=1`)** with adjustable sensitivity, Y-axis invert, and aim/shoot mouse button presets (`F9` toggles mouse capture).
- **Click-to-hide mouse cursor** inside the game window (`lifecycle.rs`), automatically restored on `F9` or `Alt+Tab`.
- **Unified Memory vs Classic AMD Mode toggle** (`-NoUnifiedMemory`) and **Lite Graphics Mode** (`-LiteGraphics`, `VICTORY_LITE_GFX=1`, `VICTORY_AMBIENT=0.2`).
- Support for **AMD FSR 2.4** and **AMD FSR Frame Generation** (FidelityFX SDK 2.4).

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
- **rorodev6 / shadowdog00** — `VICTORY_LATE_WAIT=3`, `VICTORY_TEXTURE_BATCH`, and `VICTORY_GROUPED_MESH_CLAMP`.
- **ShakiraRED** — `VICTORY_COPY_PLAIN`, `VICTORY_WRITE_PLAIN`, and `VICTORY_STAGING_POOL`.
- **Trexies** — `VICTORY_CLAMP_WAITS=0` (removing global UAV stalls on indirect clamps).
- **triippywhale** — AMD view-heap clamping and dynamic descriptor slot scaling.
- **Geckomacabre/vx-layer** & **MiamiGame/XTP** (Mesa `addrlib` tables) — ocean water physics and XOR swizzle addressing.
- **Belobug/VIctory-Belobug-AMD** — `ambient.rs` (`VICTORY_AMBIENT`).
- **JappertjeV**, **PalmVIDev**, **avatarmanz**, **ka1qux**, **clems206** — diagnostics and upstream fixes.

> Product names and trademarks are referenced solely to describe compatibility and belong to their respective owners. **No game files are included in this repository.**
