param(
    [switch]$FrameGen,
    [switch]$Res720p,
    [switch]$SeriesX,
    [switch]$NoHud,
    [switch]$Stats,
    [switch]$LowMemory,
    [string]$SkipMovies = "end",
    [int]$CardBuffersMB = 0,
    [double]$Ambient = 0.4
)

$GLeafDir = "C:\Users\gabri\Downloads\victory-gleaf6"
$LabDir   = "C:\Users\gabri\Downloads\victory-klbf-lab39"
$GameExe  = "C:\Users\gabri\Downloads\GTA6.Xbox.v1.0.2.1-CYBERLEEK\game_scarlett_master.exe"
$Exe      = Join-Path $GLeafDir "target\release\victory.exe"
$FfxDir   = "C:\Users\gabri\Downloads\victory-data\ffx-sdk-2.4"
$CacheDir = "C:\Users\gabri\Downloads\victory-data\cache-gleaf-ka1qux"
$HudScript = Join-Path $LabDir "victory-hud.py"

$Console = if ($SeriesX) { "series-x" } else { "series-s" }
$Res = if ($Res720p) { "1280x720" } else { "1920x1080" }
$Tag = if ($SeriesX) { "gleaf-ka1qux-sx-$Res" } elseif ($FrameGen) { "gleaf-ka1qux-ss-fg-$Res" } else { "gleaf-ka1qux-ss-fsr-$Res" }
$LogFile = "C:\Users\gabri\Downloads\victory-data\run-$Tag.log"

if (-not (Test-Path $Exe)) {
    Write-Host "Build release GLeaf-6 nao encontrado em $Exe" -ForegroundColor Red
    exit 1
}

# Limpa variaveis VICTORY_* antigas
Get-ChildItem Env:VICTORY_* -ErrorAction SilentlyContinue | ForEach-Object { Remove-Item "Env:$($_.Name)" -ErrorAction SilentlyContinue }

$env:VICTORY_CONFIG = Join-Path $GLeafDir "victory.toml"
$env:VICTORY_LOG = if ($Stats) { "info" } else { "info" }

# Perfil medido gleaf-6 (RX 6600 / RX 7600 8 GB) + Shaders Ka1qux + Fix Sombras Roxas YukiRyuu + Fix BC7
$env:VICTORY_DISPLAY_MODE = $Res
$env:VICTORY_TEXTURE_NO_WAIT = "1"
$env:VICTORY_RELAY = "2"
$env:VICTORY_RELAY_COPY = "1"
$env:VICTORY_WIDE_BACK = "1"
$env:VICTORY_CARD_REHASH_RUNS = "1"
$env:VICTORY_JOIN_COPIES = "1"
$env:VICTORY_INDIRECT_WINDOW = "1"
$env:VICTORY_CARD_STEPS = "2"
$env:VICTORY_INDIRECT_BATCH = "2"
$env:VICTORY_MAP_THREADS = "1"
$env:VICTORY_TITLE_PRIORITIES = "0"
$env:VICTORY_CARD_AWAKE = "1"
$env:VICTORY_SHOW_AFTER = "1"
$env:VICTORY_JOIN_PARTS = "8"
$env:VICTORY_UPLOAD_AHEAD = "1"
$env:VICTORY_LEVELS_ON_DEMAND = "0"
$env:VICTORY_WINDOW_KB = "4096"
$env:VICTORY_CARD_ROOM_FOR_TARGETS_MB = if ($SeriesX) { "3072" } else { "2048" }
# Sempre 24ms (igual ao play-rx7600.ps1 oficial do gleaf-6): 500ms trava o 1o frame do mundo por >30s e dispara o watchdog +0x23cf683!
$env:VICTORY_PIPELINE_WAIT_MS = "24"
$env:VICTORY_AMBIENT = "constant:$Ambient"
$env:VICTORY_SKIP_PIPELINES = "none"
$env:VICTORY_CARD_BUFFERS_MIN_KB = "64"
$env:VICTORY_CARD_BUFFERS_MB = if ($CardBuffersMB -gt 0) { "$CardBuffersMB" } else { "auto" }
$env:VICTORY_TEXTURE_FILLS_MB = "96"
# Desliga PIPELINES_FIRST (que congelava o loading em 0.65 FPS / 1512ms por frame)
$env:VICTORY_PIPELINES_FIRST = "0"
# Cap 512px apenas com -LowMemory (Graficos Leves no Launcher); desligado no padrao para manter perfil fiel do gleaf-6
if ($LowMemory) {
    $env:VICTORY_TEXTURE_MAX_DIM = "512"
    $env:VICTORY_TEXTURE_MAX_DIM_KEEP_UI = "1"
}
if ($SkipMovies -and $SkipMovies -ne "0" -and $SkipMovies -ne "off") {
    $env:VICTORY_SKIP_MOVIES = $SkipMovies
}

# FidelityFX Upscaler (Series S) + AMD Frame Generation
if (-not $SeriesX -and (Test-Path (Join-Path $FfxDir "amd_fidelityfx_upscaler_dx12.dll"))) {
    $env:VICTORY_FIDELITYFX_DIR = $FfxDir
    # Upscaler grava direto na command list real (Thin=0)
    $env:VICTORY_THIN = "0"
    if ($FrameGen) {
        $env:VICTORY_FRAMEGEN = "1"
    }
} else {
    $env:VICTORY_THIN = "1"
}

$hudProc = $null
if (-not $NoHud -and (Test-Path $HudScript)) {
    $hudProc = Start-Process -FilePath "pythonw.exe" -ArgumentList "`"$HudScript`"" -PassThru -ErrorAction SilentlyContinue
}

try {
    [System.Diagnostics.Process]::GetCurrentProcess().PriorityClass = [System.Diagnostics.ProcessPriorityClass]::High
} catch {}

Set-Location $GLeafDir
$CacheOk = Test-Path (Join-Path $CacheDir "precompiled.ok")
$CacheLabel = if ($CacheOk) { "Cache Ka1qux Pre-Compilado" } else { "Cache Ka1qux Frio (rode PreCompilar-Shaders-GLeaf-Ka1qux-AMD.bat)" }
Write-Host "Iniciando GTA VI [MODO GLEAF-6 + SHADERS KA1QUX | $Console $Res | FG=$FrameGen]" -ForegroundColor Magenta
Write-Host "CARD_STEPS=2 | RELAY=2 | BC7 Fix | Purple Shadow Fix (YukiRyuu) | $CacheLabel" -ForegroundColor Cyan

$ErrorActionPreference = "Continue"
try {
    & $Exe run --console $Console $GameExe --execute 2> $LogFile
} finally {
    if ($hudProc -and -not $hudProc.HasExited) {
        Stop-Process -Id $hudProc.Id -Force -ErrorAction SilentlyContinue
    }
}
