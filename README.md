<p align="center"><img src="docs/klbf/logo.svg" width="120" alt="rodrion-vi"></p>
<h1 align="center">rodrion-vi — VIctory AMD RDNA 2 / RX 6600 Unified Edition</h1>
<p align="center">
  <img src="https://img.shields.io/badge/base-KLBF%20Lab%2039%20%2B%207900XT%20Shaders-ff7a00" alt="Base: KLBF Lab 39 + 7900XT">
  <img src="https://img.shields.io/badge/tested%20on-RX%206600%208GB%20%2B%20SAM%2FReBAR-e11d48" alt="Tested on RX 6600 8GB">
  <img src="https://img.shields.io/badge/memory-Tiled%20Physical--Page%20Pool%20%2B%20UMA-06b6d4" alt="Tiled Physical-Page Pool + UMA">
  <img src="https://img.shields.io/badge/license-GPL--3.0-22c55e" alt="License: GPL-3.0">
</p>

> [!CAUTION]
> **⚠️ NENHUM ARQUIVO DE JOGO ESTÁ INCLUÍDO / NO GAME FILES INCLUDED**
>
> Este repositório contém **apenas o código-fonte da camada de compatibilidade, scripts e launcher**. Nenhum executável, asset, shader, textura ou dado proprietário de jogo é distribuído aqui. **Cada usuário deve fornecer sua própria cópia legalmente obtida** e configurar os caminhos localmente em `victory.toml` (baseado em `victory.example.toml`).
>
> *No game files, dumps, logs, or proprietary assets are included. You must supply your own legally obtained copy.*

---

## Médias de Teste Reais (RX 6600 8 GB + Ryzen 5 4500)

Testado em **Windows 11**, **AMD Radeon RX 6600 8 GB** (Above 4G Decoding + **SAM / ReBAR** ativados, `TiledResourcesTier = 4`, RDNA 2 Wave64 nativo), **AMD Ryzen 5 4500 (6C/12T, 8 MB L3)** e **16 GB DDR4** — modo **Series S + FSR 2.4 (720p)**:

| Métrica | Resultado Medido (Mundo Aberto) | Observações |
|---|---|---|
| **FPS real (cenas ~2.000–4.300 draws)** | **4.5 – 5.9 FPS** (`168 – 222 ms/frame`) | Limitado pela CPU (Ryzen 5 4500); menu em **28.5 FPS** (`35 ms`) |
| **FPS real (cenas densas ~5.000–6.000 draws)** | **3.8 – 4.9 FPS** (`205 – 263 ms/frame`) | Antes das otimizações + SAM: `2.6 – 2.8 FPS` (`352 – 381 ms`) |
| **Fluidez com AMD FSR FrameGen** | **~8 – 12 FPS exibidos** | Usa a folga da RX 6600 sem perda de FPS real |
| **Tempo na GPU (RX 6600)** | **4.7 – 15.7 ms/frame** (~51% uso) | Shaders RDNA nativos (`rdna=true`, `compute=64`) |
| **VRAM (RX 6600 8 GB)** | **5.5 GB / 6.7 GB** (~1.2 GB livre) | **0 evictions/s** (antes: 182 evictions/s e 52 GB de re-uploads) |
| **Tiled Physical-Page Pool (`VICTORY_TILED_POOL=1`)** | **739–822 janelas (3.5–4.2 GB virtuais) em ~1.15 GB físicos** | **50.5% de RAM economizada** em *NULL hole tiles*, sem `device removed` |
| **Offload GPU com SAM/ReBAR (`rebar_hw=true`)** | **239 cópias (1.110 MB) em VRAM** • **380 hash + 125 compare passes/frame na GPU** | Comparação na CPU caiu **87%** (`17.3 MB` → `2.2 MB`; buffers na CPU: `0.0 MB`) |

---

## Principais Melhorias nesta Edição

### 1. Memória Unificada — Tiled Physical-Page Pool (`VICTORY_TILED_POOL=1`)
- Usa **`CreateReservedResource` + `UpdateTileMappings`** sobre *chunks* de 64 MiB em `D3D12_HEAP_TYPE_CUSTOM (L0, Write-Back)` com mapeamento CPU persistente.
- Espelha páginas físicas de 64 KiB do console (`xmem`) diretamente nos *tiles* D3D12, economizando **~50% de RAM** em buracos esparsos (`NULL tiles` = 0 bytes) com *refcount* seguro e descarte diferido (`host::retire`).
- Invalidação por faixa de endereço (`VIEW_LOG` ring-buffer em `inplace.rs`), evitando re-escaneamento global em cada `MapView`/`UnmapView`.
- Barreira VEH lock-free em `xmem.rs` para remapeamentos concorrentes sem *access violations*.

### 2. Aproveitamento Completo de AMD SAM / ReBAR (`D3D12_HEAP_TYPE_GPU_UPLOAD`)
- Uploads dinâmicos de até **32 MiB** vão direto para a VRAM GDDR6 via PCIe BAR (sem dupla cópia em RAM) quando **Above 4G Decoding + Resizable BAR / SAM** estão ativados na BIOS.
- Combina `VICTORY_CARD_WINDOWS_MB=512` + `VICTORY_CARD_BUFFERS_MIN_KB=256` (hash e comparação via **Compute Shaders na GPU**) com o **Tiled Pool** para o restante das janelas.
- Gestão calibrada de VRAM para placas de 8 GB (`VICTORY_VRAM_IDLE_MS=3000`, `VICTORY_TEXTURE_SHORT_SIDE=128`, `VICTORY_TEXTURE_CATCHUP=1`), eliminando *eviction thrashing* de texturas.

### 3. Tradutor de Shaders RDNA Nativo (`YukiRyuu/VIctory-7900XT`)
- Detecção automática de GPUs AMD RDNA (`set_host_rdna(true)`, Wave64 nativo + `VICTORY_RDNA_FLOAT` emitindo `v_max_f32`, `v_mul_legacy_f32`, `v_med3_f32`, `v_fract_f32` em 1 instrução).
- Correções de *QuadRead* com *helper lanes* (remove artefatos em pintura, vidro e pele na AMD), índices de primitiva de 9 bits em *mesh shaders* e paridade 100% entre pré-compilação offline e execução em tempo real (`pipeline/host.rs`).
- Proteção contra `device removed` em *root views* extensas (`VICTORY_ROOT_WINDOW_MB=12`).

### 4. Otimizações de CPU (Hot Paths)
- **Hash rápido de texturas (`fast_hash64` 4 vias `u64`):** substitui `BLAKE3` criptográfico na detecção de mudança de texturas (~6–8x mais rápido) + `VICTORY_TEXTURE_HASH_EVERY=32`.
- **Pool persistente (`wide_run`)** no refresh de texturas (`texture.rs`), no `find_all` de janelas Tiled (`shadow.rs`) e em lotes no `Batch::flush` (`host.rs`).
- **`back_piece` e `changed_runs` vetorizados (32/8 bytes)** sem *zero-fills* redundantes.

### 5. Launcher Gráfico, Controles e Qualidade de Vida (`launcher/Program.cs`)
- **Mapeamento customizável Xbox → Teclado (`VICTORY_KEYMAP`):** interface interativa para remapear todos os 24 botões/eixos do Xbox (ex.: `X = K`) com resolução automática de conflitos.
- **Câmera no Mouse (`VICTORY_MOUSE_CAMERA=1`)** com sensibilidade ajustável, inversão de eixo Y e botões de mira/tiro (`F9` pausa/libera o mouse).
- **Cursor oculto ao clicar na janela do jogo** (`lifecycle.rs`), reaparecendo automaticamente com `F9` ou `Alt+Tab`.
- **Toggle Memória Unificada vs Modo Clássico AMD** (`-NoUnifiedMemory`) e **Modo Gráfico Leve / Lite** (`-LiteGraphics`, `VICTORY_LITE_GFX=1`, `VICTORY_AMBIENT=0.2`).
- Suporte a **AMD FSR 2.4** e **AMD FSR Frame Generation** (FidelityFX SDK 2.4).

---

## Como Compilar e Rodar

### Pré-requisitos
- **Windows 11** + **Rust** (toolchain em `rust-toolchain.toml`)
- GPU DirectX 12 com Shader Model 6.6 e Tiled Resources Tier 3/4 (recomendado: **Above 4G Decoding + SAM/ReBAR ativados na BIOS**).
- Sua própria cópia legalmente obtida dos arquivos do título, fora da pasta deste repositório.

### 1. Configurar caminhos locais
Copie `victory.example.toml` para `victory.toml` (ignorado pelo Git) e aponte `game_dir`, `cache_dir` e `data_dir` para suas pastas locais (sempre **fora** do repositório).

### 2. Compilar o `victory.exe`
```powershell
cargo build --release --bin victory
```

### 3. (Opcional) Compilar o Launcher Windows Forms
```powershell
& 'C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe' /nologo /target:winexe /optimize+ `
  /out:GTA6-Launcher.exe /r:System.Windows.Forms.dll /r:System.Drawing.dll launcher\Program.cs
```
*(Ajuste os caminhos no topo de `run-klbf-lab39.ps1` e `launcher\Program.cs` para o diretório da sua instalação.)*

### 4. Pré-compilar os Shaders (recomendado na primeira vez)
```powershell
$env:VICTORY_CONFIG = ".\victory.toml"
$env:VICTORY_SHADER_OPT = "0"
$env:VICTORY_SKIP_PIPELINES = (Get-Content docs\klbf\skip-amd.txt -Raw).Trim()
.\target\release\victory.exe translate --cache --pipelines --jobs 8 "<sua_pasta_do_jogo>\shaders.rpf"
```

### 5. Executar
```powershell
.\run-klbf-lab39.ps1 -Res720p
# Com AMD FSR Frame Generation:
.\run-klbf-lab39.ps1 -FrameGen -Res720p
# Modo Clássico AMD (sem Tiled Pool / UMA):
.\run-klbf-lab39.ps1 -Res720p -NoUnifiedMemory
```

---

## Referências, Créditos e Licença

Este projeto é um fork experimental de **VIctory** sob licença **GPL-3.0** (veja [LICENSE](LICENSE)), construindo sobre o trabalho incrível da comunidade:

- **Upstream VIctory** & **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)** — base de renderização, tradução D3D12.X e melhorias de texturas/view-heap.
- **[PastiiJay/klbf-lab](https://github.com/PastiiJay/klbf-lab)** / **[klbf-lab-beta](https://github.com/PastiiJay/klbf-lab-beta)** (Lab 39) — física de água, menus/HUD Series S, FSR Frame Generation, pré-compilação resumível de pipelines e correções AMD.
- **[YukiRyuu/VIctory-7900XT](https://github.com/YukiRyuu/VIctory-7900XT)** — tradutor de shaders RDNA nativo (`set_host_rdna`, `VICTORY_RDNA_FLOAT`, QuadRead helper lanes, mesh 9-bit), `root_window` (`VICTORY_ROOT_WINDOW_MB`) e otimizações AMD.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)** — `VICTORY_DYNAMIC_SLOTS`, `VICTORY_MERGE_DWORDS`, `VICTORY_PIPELINE_GATE_S`, mips parciais e divisão de dispatches >65.535 grupos.
- **[GPUOpen-LibrariesAndSDKs/FidelityFX-SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK)** — AMD FidelityFX SDK 2.4 (FSR + Frame Generation).
- **rorodev6 / shadowdog00** — presets `VICTORY_LATE_WAIT=3`, `VICTORY_TEXTURE_BATCH` e `VICTORY_GROUPED_MESH_CLAMP`.
- **ShakiraRED** — `VICTORY_COPY_PLAIN`, `VICTORY_WRITE_PLAIN` e `VICTORY_STAGING_POOL`.
- **Trexies** — `VICTORY_CLAMP_WAITS=0` (eliminação de stalls UAV nos indirect clamps).
- **triippywhale** — clamp do view-heap AMD e alocação dinâmica de slots.
- **Geckomacabre/vx-layer** & **MiamiGame/XTP** (tabelas do `addrlib` do Mesa) — física de água e modos XOR swizzle.
- **Belobug/VIctory-Belobug-AMD** — `ambient.rs` (`VICTORY_AMBIENT`).
- **JappertjeV**, **PalmVIDev**, **avatarmanz**, **ka1qux**, **clems206** — diagnósticos e correções integradas ao código.

> Nomes de produtos e marcas são mencionados apenas para indicar compatibilidade e pertencem aos seus respectivos proprietários. **Nenhum arquivo de jogo é incluído neste repositório.**
