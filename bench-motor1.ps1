param(
    [string]$Tag = "v1.0.0-baseline",
    [int]$RunsPerArm = 1,
    [int]$WarmupSamples = 2,
    [int]$MeasureSamples = 8,
    [int]$TimeoutSec = 260,
    [hashtable]$BaseEnv = @{},
    [hashtable]$CompareEnv = $null,
    [string]$CompareName = "B"
)

$ErrorActionPreference = 'Stop'
$RepoDir = "C:\Users\gabri\Downloads\victory-klbf-lab39"
$GameExe = "C:\Users\gabri\Downloads\GTA6.Xbox.v1.0.2.1-CYBERLEEK\game_scarlett_master.exe"
$Exe = Join-Path $RepoDir "target\release\victory.exe"
$FfxDir = "C:\Users\gabri\Downloads\victory-data\ffx-sdk-2.4"
$SkipFile = Join-Path $RepoDir "docs\klbf\skip-amd.txt"
$BenchRoot = Join-Path "C:\Users\gabri\Downloads\victory-data\bench" $Tag
New-Item -ItemType Directory -Force -Path $BenchRoot | Out-Null

if (-not (Test-Path $Exe)) { throw "Missing binary: $Exe" }

function Get-Percentile([double[]]$arr, [double]$p) {
    if (-not $arr -or $arr.Count -eq 0) { return [double]::NaN }
    $sorted = $arr | Sort-Object
    if ($sorted.Count -eq 1) { return [double]$sorted[0] }
    $idx = ($sorted.Count - 1) * $p
    $lo = [math]::Floor($idx)
    $hi = [math]::Ceiling($idx)
    if ($lo -eq $hi) { return [double]$sorted[$lo] }
    return [double]$sorted[$lo] + ($idx - $lo) * ([double]$sorted[$hi] - [double]$sorted[$lo])
}

function Parse-VictoryLog([string]$LogPath, [int]$Warmup, [int]$MaxSamples) {
    $raw = [System.IO.File]::ReadAllText($LogPath) -replace "\x1b\[[0-9;]*m", "" -replace "`r?`n", " " -replace "\s+", " "
    $entries = $raw -split '(?=20\d\d-\d\d-\d\dT)'
    $samples = New-Object System.Collections.Generic.List[object]
    $cur = $null

    foreach ($e in $entries) {
        if ($e -match 'victory_gpu::host: ([\d.]+) fps, ([\d.]+) ms/frame \(([\d.]+) preparing: copies ([\d.]+) ms ([\d.]+) MB, textures ([\d.]+) ms ([\d.]+) MB; ([\d.]+) on the GPU; ([\d.]+) copying back ([\d.]+) MB\), (\d+) draws/frame, dropped (\d+) without a host pipeline.*?heap buffers [\d.]+ passes ([\d.]+) ms') {
            $draws = [int]$Matches[11]
            if ($draws -gt 100) {
                $cur = [pscustomobject]@{
                    fps = [double]$Matches[1]
                    total_ms = [double]$Matches[2]
                    prepare_ms = [double]$Matches[3]
                    copies_ms = [double]$Matches[4]
                    copies_mb = [double]$Matches[5]
                    textures_ms = [double]$Matches[6]
                    textures_mb = [double]$Matches[7]
                    gpu_ms = [double]$Matches[8]
                    copyback_ms = [double]$Matches[9]
                    copyback_mb = [double]$Matches[10]
                    draws = $draws
                    dropped = [int]$Matches[12]
                    heap_buffers_ms = [double]$Matches[13]
                    desc_sync_ms = 0.0
                    desc_pages = 0.0
                    desc_slots = 0.0
                    desc_mirrored = 0.0
                    desc_tabled = 0.0
                    settle_count = 0.0
                    settle_ms = 0.0
                    settle_read_ms = 0.0
                    mapping_ms = 0.0
                    lock_wait_ms = 0.0
                    gpu_idle_ms = 0.0
                    hash_passes = 0.0
                    cmp_passes = 0.0
                    tex_refresh_ms = 0.0
                    tex_watch_ms = 0.0
                    watch_fast_hits = 0.0
                    watch_slow_calls = 0.0
                    watch_slow_ms = 0.0
                    sync_take_ms = 0.0
                    sync_scan_ms = 0.0
                    sync_mirror_ms = 0.0
                    sync_pages_skipped = 0.0
                    sync_mb_saved = 0.0
                    sync_wide_calls = 0.0
                    batch_copies = 0.0
                    batch_kept_ref = 0.0
                    batch_flushed_ref = 0.0
                    batch_kept_tex = 0.0
                    tiled_fast_checks = 0.0
                    tiled_fast_skips = 0.0
                }
                $samples.Add($cur)
            } else {
                $cur = $null
            }
            continue
        }
        if ($null -eq $cur) { continue }
        if ($e -match 'descriptor records the title.*?per frame ([\d.]+) changed heap pages, ([\d.]+) slots looked at, ([\d.]+) whose record was new and was mirrored, ([\d.]+) left to.*?([\d.]+) ms') {
            $cur.desc_pages = [double]$Matches[1]
            $cur.desc_slots = [double]$Matches[2]
            $cur.desc_mirrored = [double]$Matches[3]
            $cur.desc_tabled = [double]$Matches[4]
            $cur.desc_sync_ms = [double]$Matches[5]
        }
        if ($e -match 'submissions sent ahead per frame: [\d.]+ in ([\d.]+) settles .*? ([\d.]+) ms waiting for the card') {
            $cur.settle_count = [double]$Matches[1]
            $cur.settle_ms = [double]$Matches[2]
        }
        if ($e -match 'of the waiting for the card in settles.*?title memory read ([\d.]+)') {
            $cur.settle_read_ms = [double]$Matches[1]
        }
        if ($e -match 'changes of title memory''s mapping per frame: [\d.]+, ([\d.]+) ms of them waiting.*?([\d.]+) ms under way') {
            $cur.lock_wait_ms = [double]$Matches[1]
            $cur.mapping_ms = [double]$Matches[2]
        }
        if ($e -match 'the card''s clock per frame: nothing of ours queued ([\d.]+) ms') {
            $cur.gpu_idle_ms = [double]$Matches[1]
        }
        if ($e -match 'our passes over copies in the card''s memory per frame: ([\d.]+) hash passes.*?([\d.]+) compare passes') {
            $cur.hash_passes = [double]$Matches[1]
            $cur.cmp_passes = [double]$Matches[2]
        }
        if ($e -match 'texture refreshes per frame, all threads: ([\d.]+) ms, of them [\d.]+ ms waiting.*?, ([\d.]+) ms asking the write watch') {
            $cur.tex_refresh_ms = [double]$Matches[1]
            $cur.tex_watch_ms = [double]$Matches[2]
        }
        if ($e -match 'watch::writes per frame \(FAST=\d, UNLOCKED=\d\): ([\d.]+) read-fast, ([\d.]+) slow \(([\d.]+) ms: ([\d.]+) GetWriteWatch, ([\d.]+) guard\)') {
            $cur.watch_fast_hits = [double]$Matches[1]
            $cur.watch_slow_calls = [double]$Matches[2]
            $cur.watch_slow_ms = [double]$Matches[3]
        }
        if ($e -match 'sync_heaps breakdown \(VICTORY_SYNC_SKIP_TABLED_PAGES=\d\): take ([\d.]+) ms, scan ([\d.]+) ms, mirror ([\d.]+) ms, ([\d.]+) pages skipped \(([\d.]+) MB copy saved\), ([\d.]+) wide_run calls/frame') {
            $cur.sync_take_ms = [double]$Matches[1]
            $cur.sync_scan_ms = [double]$Matches[2]
            $cur.sync_mirror_ms = [double]$Matches[3]
            $cur.sync_pages_skipped = [double]$Matches[4]
            $cur.sync_mb_saved = [double]$Matches[5]
            $cur.sync_wide_calls = [double]$Matches[6]
        }
        if ($e -match 'batch Step::Copy per frame \(VICTORY_BATCH_RANGE_FLUSH=\d\): ([\d.]+) copies, ([\d.]+) refreshes kept coalesced, ([\d.]+) flushed on overlap, ([\d.]+) textures kept cached') {
            $cur.batch_copies = [double]$Matches[1]
            $cur.batch_kept_ref = [double]$Matches[2]
            $cur.batch_flushed_ref = [double]$Matches[3]
            $cur.batch_kept_tex = [double]$Matches[4]
        }
        if ($e -match 'tiled lock-free sync per frame \(VICTORY_TILED_LOCK_BATCH=1\): ([\d.]+) lock-free checks \(([\d.]+) deduped') {
            $cur.tiled_fast_checks = [double]$Matches[1]
            $cur.tiled_fast_skips = [double]$Matches[2]
        }
    }

    # Drop the very last sample if its trailing sub-lines were cut off by process exit
    if ($samples.Count -gt 1 -and $samples[$samples.Count - 1].desc_sync_ms -eq 0.0 -and $samples[$samples.Count - 1].gpu_idle_ms -eq 0.0) {
        $samples.RemoveAt($samples.Count - 1)
    }

    $usable = @($samples | Select-Object -Skip $Warmup | Select-Object -First $MaxSamples)
    if ($usable.Count -eq 0) {
        return [pscustomobject]@{ ok = $false; total_world_samples = $samples.Count; used = 0 }
    }

    $stat = {
        param([string]$field)
        $vals = [double[]]@($usable | ForEach-Object { [double]$_.$field })
        [pscustomobject]@{
            median = [math]::Round((Get-Percentile $vals 0.50), 2)
            p10 = [math]::Round((Get-Percentile $vals 0.10), 2)
            p90 = [math]::Round((Get-Percentile $vals 0.90), 2)
            worst = [math]::Round(($vals | Measure-Object -Maximum).Maximum, 2)
            mean = [math]::Round(($vals | Measure-Object -Average).Average, 2)
        }
    }

    return [pscustomobject]@{
        ok = $true
        total_world_samples = $samples.Count
        used = $usable.Count
        prepare_ms = & $stat 'prepare_ms'
        total_ms = & $stat 'total_ms'
        fps = & $stat 'fps'
        draws = & $stat 'draws'
        copies_ms = & $stat 'copies_ms'
        copies_mb = & $stat 'copies_mb'
        textures_ms = & $stat 'textures_ms'
        copyback_ms = & $stat 'copyback_ms'
        copyback_mb = & $stat 'copyback_mb'
        heap_buffers_ms = & $stat 'heap_buffers_ms'
        desc_sync_ms = & $stat 'desc_sync_ms'
        desc_pages = & $stat 'desc_pages'
        desc_slots = & $stat 'desc_slots'
        desc_mirrored = & $stat 'desc_mirrored'
        desc_tabled = & $stat 'desc_tabled'
        settle_ms = & $stat 'settle_ms'
        settle_count = & $stat 'settle_count'
        settle_read_ms = & $stat 'settle_read_ms'
        mapping_ms = & $stat 'mapping_ms'
        lock_wait_ms = & $stat 'lock_wait_ms'
        gpu_ms = & $stat 'gpu_ms'
        gpu_idle_ms = & $stat 'gpu_idle_ms'
        hash_passes = & $stat 'hash_passes'
        cmp_passes = & $stat 'cmp_passes'
        tex_refresh_ms = & $stat 'tex_refresh_ms'
        tex_watch_ms = & $stat 'tex_watch_ms'
        watch_fast_hits = & $stat 'watch_fast_hits'
        watch_slow_calls = & $stat 'watch_slow_calls'
        watch_slow_ms = & $stat 'watch_slow_ms'
        sync_take_ms = & $stat 'sync_take_ms'
        sync_scan_ms = & $stat 'sync_scan_ms'
        sync_mirror_ms = & $stat 'sync_mirror_ms'
        sync_pages_skipped = & $stat 'sync_pages_skipped'
        sync_mb_saved = & $stat 'sync_mb_saved'
        sync_wide_calls = & $stat 'sync_wide_calls'
        batch_copies = & $stat 'batch_copies'
        batch_kept_ref = & $stat 'batch_kept_ref'
        batch_flushed_ref = & $stat 'batch_flushed_ref'
        batch_kept_tex = & $stat 'batch_kept_tex'
        tiled_fast_checks = & $stat 'tiled_fast_checks'
        tiled_fast_skips = & $stat 'tiled_fast_skips'
    }
}

function Invoke-OneRun([string]$ArmLabel, [int]$RunIdx, [hashtable]$Extra) {
    $logFile = Join-Path $BenchRoot ("{0}-run{1}.log" -f $ArmLabel, $RunIdx)
    $errFile = $logFile
    $outFile = Join-Path $BenchRoot ("{0}-run{1}.stdout.log" -f $ArmLabel, $RunIdx)
    Remove-Item $logFile, $outFile -ErrorAction SilentlyContinue

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = "run --console series-s `"$GameExe`" --execute"
    $psi.WorkingDirectory = $RepoDir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true

    # Clean VICTORY_* from child env
    foreach ($k in @($psi.EnvironmentVariables.Keys)) {
        if ($k -like 'VICTORY_*') { $psi.EnvironmentVariables.Remove($k) }
    }

    $skipList = if (Test-Path $SkipFile) { (Get-Content $SkipFile -Raw).Trim() } else { "39f0e9aeb156a61c" }
    # Exact v1.0.0 Motor 1 baseline environment (Series S 720p, FSR 2.4, No FrameGen for pure CPU prepare measurement)
    $envMap = @{
        VICTORY_CONFIG = (Join-Path $RepoDir "victory.toml")
        VICTORY_LOG = "info"
        VICTORY_SHADER_OPT = "0"
        VICTORY_SKIP_PIPELINES = $skipList
        VICTORY_LATE_WAIT = "3"
        VICTORY_LATE_WAIT_LISTS = "256"
        VICTORY_RELAY = "2"
        VICTORY_RELAY_COPY_QUEUE = "0"
        VICTORY_INDIRECT_WINDOW = "1"
        VICTORY_FOLD_BUFFERS = "1"
        VICTORY_WIDE_BACK = "1"
        VICTORY_WIDE_REFRESH = "1"
        VICTORY_WIDE_MERGE = "1"
        VICTORY_WIDE_THREADS = "8"
        VICTORY_JOIN_COPIES = "1"
        VICTORY_TEXTURE_NO_WAIT = "1"
        VICTORY_CARD_REHASH_RUNS = "1"
        VICTORY_ASK_ONCE = "1"
        VICTORY_TEXTURE_BATCH = "1"
        VICTORY_GROUPED_MESH_CLAMP = "1"
        VICTORY_CLAMP_WAITS = "0"
        VICTORY_CARD_FETCH_INFLIGHT = "1"
        VICTORY_CARD_FETCH_INFLIGHT_MB = "96"
        VICTORY_COPY_PLAIN = "1"
        VICTORY_WRITE_PLAIN = "1"
        VICTORY_STAGING_POOL = "1"
        VICTORY_WIDE_HEAP = "1"
        VICTORY_WIDE_SYNC = "1"
        VICTORY_HEAP_SCAN_EVERY = "48"
        VICTORY_ASK_QUIET = "8"
        VICTORY_UNDER_WRITES_RUNS = "65536"
        VICTORY_BIND_CACHE = "1"
        VICTORY_DECLARE_ONCE = "1"
        VICTORY_GPU_CBV = "1"
        VICTORY_POOL_COMMIT = "0"
        VICTORY_UMA_POOL = "0"
        VICTORY_TILED_POOL = "1"
        VICTORY_TILED_POOL_MB = "2560"
        VICTORY_INPLACE = "2"
        VICTORY_INPLACE_MB = "4608"
        VICTORY_DIRECT_DMA = "1"
        VICTORY_REBAR = "1"
        VICTORY_PAGE_WRITES = "1"
        VICTORY_TEXTURE_HEAPS = "0"
        VICTORY_PLACEMENT_HEAPS = "1"
        VICTORY_RANGED = "1"
        VICTORY_REFRESH_STAMP = "1"
        VICTORY_CARD_FLAGGED = "1"
        VICTORY_DROP_DXIL = "1"
        VICTORY_LIST_POOL_MOST = "2048"
        VICTORY_LIST_POOL_MB = "2048"
        VICTORY_WATCH_4K = "0"
        VICTORY_ASK_ONCE_VIEWS = "0"
        VICTORY_COPY_CHANGED = "0"
        VICTORY_QUIET_BACK = "0"
        VICTORY_INDIRECT_RAW = "0"
        VICTORY_ASYNC_SUBMIT = "0"
        VICTORY_ASYNC_PRESENT = "0"
        VICTORY_THIN = "0"
        VICTORY_DYNAMIC_SLOTS = "524288"
        VICTORY_MERGE_DWORDS = "1"
        VICTORY_SECTION_VIEWS = "0"
        VICTORY_PIPELINES_FIRST = "0"
        VICTORY_TEXTURE_LRU = "0"
        VICTORY_SKIP_MOVIES = "end"
        VICTORY_PIPELINE_WORKERS = "2"
        VICTORY_PIPELINE_WAIT_MS = "24"
        VICTORY_PIPELINE_GATE_S = "8"
        VICTORY_D3D12_BACKGROUND = "0"
        VICTORY_CARD_ROOM_FOR_BUFFERS = "1"
        VICTORY_CARD_ROOM_FOR_TARGETS_MB = "1800"
        VICTORY_CARD_BUFFERS_MIN_KB = "256"
        VICTORY_CARD_BUFFERS_MB = "1024"
        VICTORY_TEXTURE_STAGE_MB = "512"
        VICTORY_TEXTURE_STAGE_MS = "24"
        VICTORY_TEXTURE_CATCHUP = "0"
        VICTORY_ROOT_WINDOW_MB = "12"
        VICTORY_CARD_WINDOWS_MB = "512"
        VICTORY_CARD_WINDOWS_PERCENT = "78"
        VICTORY_TEXTURE_MAX_DIM = "0"
        VICTORY_TEXTURE_MAX_DIM_KEEP_UI = "1"
        VICTORY_TEXTURE_DROP_MIPS = "2"
        VICTORY_TEXTURE_SHORT = "1"
        VICTORY_TEXTURE_SHORT_PERCENT = "97"
        VICTORY_TEXTURE_SHORT_SIDE = "128"
        VICTORY_TEXTURE_ROOM_PERCENT = "90"
        VICTORY_TEXTURE_HASH_EVERY = "32"
        VICTORY_VRAM_IDLE_MS = "3000"
        VICTORY_BAKE_SOURCES_FIRST = "1"
        VICTORY_BAKE_ROOM_PERCENT = "96"
        VICTORY_AMBIENT = "0.6"
        VICTORY_DISPLAY_MODE = "1280x720"
        VICTORY_FFX_DIR = $FfxDir
        VICTORY_KEYS = "18:Enter,22:Enter,26:Enter,30:Enter,34:Enter,38:Enter,42:Enter,46:Enter,50:Enter,55:Enter,60:Enter,65:Enter"
    }
    foreach ($k in $BaseEnv.Keys) { $envMap[$k] = [string]$BaseEnv[$k] }
    if ($Extra) { foreach ($k in $Extra.Keys) { $envMap[$k] = [string]$Extra[$k] } }
    foreach ($k in $envMap.Keys) { $psi.EnvironmentVariables[$k] = $envMap[$k] }

    $targetCount = $WarmupSamples + $MeasureSamples
    Write-Host ("[{0}] Starting {1} run {2} (target: {3}+{4} world samples)..." -f (Get-Date -Format 'HH:mm:ss'), $ArmLabel, $RunIdx, $WarmupSamples, $MeasureSamples)

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    $fsErr = [System.IO.File]::Open($errFile, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::ReadWrite)
    $swErr = New-Object System.IO.StreamWriter($fsErr)
    $swErr.AutoFlush = $true

    $null = $proc.Start()
    $worldSeen = 0
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $readTask = $proc.StandardError.ReadLineAsync()
    $nullTask = $proc.StandardOutput.ReadToEndAsync()

    try {
        while (-not $proc.HasExited) {
            if ($sw.Elapsed.TotalSeconds -ge $TimeoutSec) {
                Write-Host "  TIMEOUT after ${TimeoutSec}s (world samples seen: $worldSeen)" -ForegroundColor Yellow
                break
            }
            if ($readTask.Wait(200)) {
                $line = $readTask.Result
                if ($null -eq $line) { break }
                $swErr.WriteLine($line)
                if ($line -match '(\d+) draws/frame' -and ([int]$Matches[1] -gt 100)) {
                    $worldSeen++
                    if ($worldSeen -ge $targetCount) {
                        Write-Host ("  Reached {0} world samples in {1:N1}s!" -f $worldSeen, $sw.Elapsed.TotalSeconds)
                        break
                    }
                }
                $readTask = $proc.StandardError.ReadLineAsync()
            }
        }
    } finally {
        if (-not $proc.HasExited) {
            try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch {}
            $proc.WaitForExit(5000) | Out-Null
        }
        $swErr.Dispose()
        $fsErr.Dispose()
        $proc.Dispose()
    }

    Start-Sleep -Seconds 3
    $res = Parse-VictoryLog $logFile $WarmupSamples $MeasureSamples
    $res | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $BenchRoot ("{0}-run{1}.json" -f $ArmLabel, $RunIdx))
    if ($res.ok) {
        Write-Host ("  => {0}#{1}: prepare={2}ms (p10={3} p90={4} max={5}) | total={6}ms ({7} fps) | draws={8} | copies={9}ms/{10}MB | descSync={11}ms | heapBuf={12}ms | settle={13}ms | copyBack={14}ms/{15}MB" -f `
            $ArmLabel, $RunIdx, $res.prepare_ms.median, $res.prepare_ms.p10, $res.prepare_ms.p90, $res.prepare_ms.worst, `
            $res.total_ms.median, $res.fps.median, $res.draws.median, $res.copies_ms.median, $res.copies_mb.median, `
            $res.desc_sync_ms.median, $res.heap_buffers_ms.median, $res.settle_ms.median, $res.copyback_ms.median, $res.copyback_mb.median)
    } else {
        Write-Host ("  => {0}#{1}: FAILED (world samples={2})" -f $ArmLabel, $RunIdx, $res.total_world_samples) -ForegroundColor Red
    }
    return $res
}

$armA = @()
$armB = @()
for ($i = 1; $i -le $RunsPerArm; $i++) {
    $rA = Invoke-OneRun "A" $i @{}
    if (-not $rA.ok) {
        Write-Host "  Retrying A run $i after boot failure..." -ForegroundColor Yellow
        $rA = Invoke-OneRun "A" $i @{}
    }
    $armA += $rA
    if ($null -ne $CompareEnv) {
        $rB = Invoke-OneRun $CompareName $i $CompareEnv
        if (-not $rB.ok) {
            Write-Host "  Retrying $CompareName run $i after boot failure..." -ForegroundColor Yellow
            $rB = Invoke-OneRun $CompareName $i $CompareEnv
        }
        $armB += $rB
    }
}

function Summarize-Arm([string]$name, $runs) {
    $ok = @($runs | Where-Object { $_.ok })
    if ($ok.Count -eq 0) { Write-Host "Arm ${name}: 0 valid runs"; return }
    $fields = @('prepare_ms','total_ms','fps','draws','copies_ms','copies_mb','desc_sync_ms','sync_take_ms','sync_scan_ms','sync_mirror_ms','sync_pages_skipped','sync_mb_saved','sync_wide_calls','watch_fast_hits','watch_slow_calls','watch_slow_ms','batch_copies','batch_kept_ref','batch_flushed_ref','batch_kept_tex','tiled_fast_checks','heap_buffers_ms','settle_ms','settle_count','copyback_ms','copyback_mb','mapping_ms','gpu_ms','gpu_idle_ms','tex_watch_ms')
    Write-Host "=== ARM $name (n=$($ok.Count)) ==="
    foreach ($f in $fields) {
        $vals = [double[]]@($ok | ForEach-Object { [double]$_.$f.median })
        $mean = ($vals | Measure-Object -Average).Average
        $min = ($vals | Measure-Object -Minimum).Minimum
        $max = ($vals | Measure-Object -Maximum).Maximum
        $spread = ($max - $min) / 2.0
        Write-Host ("  {0,-16}: mean={1,8:N2} +- {2,6:N2}  (min={3:N2}, max={4:N2})" -f $f, $mean, $spread, $min, $max)
    }
}

Summarize-Arm "A" $armA
if ($null -ne $CompareEnv) { Summarize-Arm $CompareName $armB }
