param(
    [string]$RunDate = (Get-Date -Format 'yyyy-MM-dd'),
    [switch]$Quick,
    [switch]$SkipQuietHostGate
)

$ErrorActionPreference = 'Stop'
$invariant = [Globalization.CultureInfo]::InvariantCulture
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$taskRoot = 'D:\h\codex-graph-p34-audit'
$targetDir = Join-Path $taskRoot 'target'
$tempDir = Join-Path $taskRoot 'tmp'
$cargoHome = Join-Path $tempDir 'cargo-home'
$evidenceRoot = Join-Path $repoRoot "reports\performance-evidence\$RunDate"
$dataDir = Join-Path $evidenceRoot 'data'
$figureDir = Join-Path $evidenceRoot 'figures'
[void](New-Item -ItemType Directory -Force -Path $targetDir, $tempDir, $cargoHome, $dataDir, $figureDir)

$env:CARGO_HOME = $cargoHome
$env:CARGO_TARGET_DIR = $targetDir
$env:TEMP = $tempDir
$env:TMP = $tempDir
$runName = Get-Date -Format 'yyyyMMdd-HHmmssK'

function Parse-Number([string]$value) {
    if ($value -eq 'NA' -or [string]::IsNullOrWhiteSpace($value)) { return $null }
    return [double]::Parse($value, $invariant)
}

function Parse-EvidenceLine([string]$line) {
    $parts = $line -split "`t"
    $values = [ordered]@{ kind = $parts[0] }
    foreach ($part in $parts[1..($parts.Count - 1)]) {
        $pair = $part -split '=', 2
        if ($pair.Count -eq 2) { $values[$pair[0]] = $pair[1] }
    }
    return $values
}

function Write-Tsv([string]$path, [object[]]$rows, [string[]]$columns) {
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add(($columns -join "`t"))
    foreach ($row in $rows) {
        $values = foreach ($column in $columns) {
            $value = $row.$column
            if ($null -eq $value -or ($value -is [string] -and [string]::IsNullOrEmpty($value))) {
                'NA'
            } else {
                "$value" -replace "`t", ' ' -replace "`r?`n", ' '
            }
        }
        $lines.Add(($values -join "`t"))
    }
    $utf8 = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllLines($path, $lines, $utf8)
}

if (-not $SkipQuietHostGate) {
    $accepted = $false
    for ($attempt = 1; $attempt -le 3 -and -not $accepted; $attempt++) {
        $loads = @()
        for ($sample = 0; $sample -lt 5; $sample++) {
            $loads += [double]((Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average)
            if ($sample -lt 4) { Start-Sleep -Seconds 1 }
        }
        $ambientAverage = ($loads | Measure-Object -Average).Average
        $ambientMaximum = ($loads | Measure-Object -Maximum).Maximum
        Write-Host ("ambient CPU attempt {0}: avg={1:F1}% max={2:F1}%" -f $attempt, $ambientAverage, $ambientMaximum)
        $accepted = $ambientAverage -le 25.0
    }
    if (-not $accepted) {
        throw "quiet-host gate failed: five-sample average CPU remained above 25%"
    }
} else {
    $ambientAverage = 'SKIPPED'
    $ambientMaximum = 'SKIPPED'
}

Push-Location $repoRoot
try {
    & cargo test -p hymeko_graph --release --test performance_evidence --no-run
    if ($LASTEXITCODE -ne 0) { throw "performance_evidence build failed with exit $LASTEXITCODE" }
} finally {
    Pop-Location
}

$exe = (Get-ChildItem -LiteralPath (Join-Path $targetDir 'release\deps') -Filter 'performance_evidence-*.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
if (-not $exe) { throw 'compiled performance_evidence executable was not found' }
$exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLowerInvariant()
$logicalProcessors = [Environment]::ProcessorCount

$processRows = [Collections.Generic.List[object]]::new()
$sampleRows = [Collections.Generic.List[object]]::new()
$cellRows = [Collections.Generic.List[object]]::new()
$phaseRows = [Collections.Generic.List[object]]::new()
$retainedRows = [Collections.Generic.List[object]]::new()
$workRows = [Collections.Generic.List[object]]::new()

function Invoke-EvidenceProcess(
    [string]$testName,
    [string]$workload,
    [string]$subject,
    [int]$threads,
    [string]$representation = ''
) {
    $processRun = "$testName-$workload-$subject-t$threads"
    Write-Host "run $processRun"
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "$testName --exact --ignored --nocapture"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['EVIDENCE_RUN'] = $processRun
    $psi.EnvironmentVariables['EVIDENCE_WORKLOAD'] = $workload
    $psi.EnvironmentVariables['EVIDENCE_THREADS'] = "$threads"
    if ($testName -eq 'operation_cell') {
        $psi.EnvironmentVariables['EVIDENCE_OPERATION'] = $subject
    } else {
        $psi.EnvironmentVariables['EVIDENCE_REPRESENTATION'] = $representation
    }

    $process = [Diagnostics.Process]::Start($psi)
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    [double]$externalPeak = 0
    do {
        try {
            $process.Refresh()
            $externalPeak = [Math]::Max($externalPeak, [double]$process.WorkingSet64)
            $externalPeak = [Math]::Max($externalPeak, [double]$process.PeakWorkingSet64)
        } catch {
            # The child may exit between HasExited and Refresh; the last valid sample remains useful.
        }
        if (-not $process.HasExited) { Start-Sleep -Milliseconds 5 }
    } while (-not $process.HasExited)
    $process.WaitForExit()
    $clock.Stop()
    $stdout = $stdoutTask.Result
    $stderr = $stderrTask.Result
    $exitCode = $process.ExitCode
    try { $processCpuMs = $process.TotalProcessorTime.TotalMilliseconds } catch { $processCpuMs = $null }
    try {
        $process.Refresh()
        $externalPeak = [Math]::Max($externalPeak, [double]$process.PeakWorkingSet64)
    } catch {}

    $processRows.Add([pscustomobject][ordered]@{
        process_run = $processRun
        test = $testName
        workload = $workload
        subject = $subject
        threads = $threads
        process_wall_ms = $clock.Elapsed.TotalMilliseconds.ToString('F3', $invariant)
        process_cpu_ms = if ($null -eq $processCpuMs) { 'UNMEASURED' } else { $processCpuMs.ToString('F3', $invariant) }
        external_peak_rss_b = [uint64]$externalPeak
        exit_code = $exitCode
    })

    if ($exitCode -ne 0) {
        throw "cell $processRun failed with exit $exitCode`nSTDERR:`n$stderr`nSTDOUT:`n$stdout"
    }
    if ($stderr) { Write-Verbose $stderr }

    $evidenceLines = @($stdout -split "`r?`n" | Where-Object { $_ -like 'EVIDENCE_*' })
    if (-not ($evidenceLines | Where-Object { $_ -like 'EVIDENCE_CELL*' })) {
        throw "cell $processRun exited successfully without an EVIDENCE_CELL line"
    }
    foreach ($line in $evidenceLines) {
        $value = Parse-EvidenceLine $line
        switch ($value.kind) {
            'EVIDENCE_SAMPLE' {
                $sampleRows.Add([pscustomobject][ordered]@{
                    process_run = $processRun; run = $value.run; section = $value.section
                    workload = $value.workload; subject = $value.subject; threads = $value.threads
                    ordinal = $value.ordinal; repetitions = $value.repetitions
                    wall_us = $value.wall_us; cpu_us = $value.cpu_us; checksum = $value.checksum
                })
            }
            'EVIDENCE_CELL' {
                $cellRows.Add([pscustomobject][ordered]@{
                    process_run = $processRun; run = $value.run; section = $value.section
                    workload = $value.workload; subject = $value.subject; status = $value.status
                    threads = $value.threads; nodes = $value.nodes; arcs = $value.arcs
                    incidences = $value.incidences; warmups = $value.warmups; samples = $value.samples
                    repetitions = $value.repetitions; wall_median_us = $value.wall_median_us
                    wall_q1_us = $value.wall_q1_us; wall_q3_us = $value.wall_q3_us
                    wall_worst_us = $value.wall_worst_us; cpu_median_us = $value.cpu_median_us
                    cpu_q1_us = $value.cpu_q1_us; cpu_q3_us = $value.cpu_q3_us
                    cpu_worst_us = $value.cpu_worst_us; checksum = $value.checksum
                    speedup = 'NA'; efficiency = 'NA'; operation_busy_cores = 'NA'
                    reason = if ($value.Contains('reason')) { $value.reason } else { 'NA' }
                })
            }
            'EVIDENCE_PHASE' {
                $phaseRows.Add([pscustomobject][ordered]@{
                    process_run = $processRun; run = $value.run; workload = $value.workload
                    subject = $value.subject; phase = $value.phase
                    working_set_b = $value.working_set_b; self_peak_b = $value.self_peak_b
                    external_process_peak_b = [uint64]$externalPeak
                })
            }
            'EVIDENCE_RETAINED' {
                $retainedRows.Add([pscustomobject][ordered]@{
                    process_run = $processRun; run = $value.run; workload = $value.workload
                    representation = $value.representation; representation_b = $value.representation_b
                    workspace_b = $value.workspace_b; output_b = $value.output_b
                    semantic_loss = $value.semantic_loss
                })
            }
            'EVIDENCE_WORK' {
                $workRows.Add([pscustomobject][ordered]@{
                    process_run = $processRun; run = $value.run; workload = $value.workload
                    subject = $value.subject; expanded_nodes = $value.expanded_nodes
                    expanded_arcs = $value.expanded_arcs
                    resolved_incidences = $value.resolved_incidences; pushes = $value.pushes
                    pops = $value.pops; stale_pops = $value.stale_pops
                    heuristic_evals = $value.heuristic_evals; reopened = $value.reopened
                })
            }
        }
    }
}

if ($Quick) {
    $scaleWorkloads = @('sts511')
    $threadList = @(1, [Math]::Min(2, $logicalProcessors)) | Select-Object -Unique
    $serialWorkloads = @('path65k')
    $serialOperations = @('dijkstra', 'rank2', 'levi')
    $representationWorkloads = @('sts511')
} else {
    $scaleWorkloads = @('sts1023', 'random1e6', 'map512')
    $threadList = @(1, 2, 4, 8, 16, [Math]::Min(24, $logicalProcessors)) |
        Where-Object { $_ -le $logicalProcessors } | Select-Object -Unique
    $serialWorkloads = @('path65k', 'grid256', 'sts1023', 'random1e6', 'map512')
    $serialOperations = @('reachable', 'components', 'dijkstra', 'rank2', 'levi')
    $representationWorkloads = @(
        'sts1023', 'random1e6', 'map512',
        'arity2_1m', 'arity3_1m', 'arity8_1m', 'arity32_1m', 'arity128_1m'
    )
}

foreach ($workload in $scaleWorkloads) {
    foreach ($threads in $threadList) {
        Invoke-EvidenceProcess 'operation_cell' $workload 'independent_query_batch' $threads
    }
}
foreach ($workload in $serialWorkloads) {
    foreach ($operation in $serialOperations) {
        Invoke-EvidenceProcess 'operation_cell' $workload $operation 1
    }
}
$representationKinds = @('canonical_index', 'implicit_levi', 'sparse_star', 'clique_2section')
foreach ($workload in $representationWorkloads) {
    foreach ($kind in $representationKinds) {
        Invoke-EvidenceProcess 'representation_cell' $workload $kind 1 $kind
    }
}

$okCells = @($cellRows | Where-Object { $_.status -eq 'OK' })
foreach ($cell in $okCells) {
    if ([int]$cell.samples -ne 9) { throw "$($cell.process_run) did not emit nine samples" }
    $rawCount = @($sampleRows | Where-Object {
        $_.process_run -eq $cell.process_run -and $_.section -eq $cell.section
    }).Count
    if ($rawCount -ne 9) { throw "$($cell.process_run)/$($cell.section) raw sample count is $rawCount" }
    $q1 = Parse-Number $cell.wall_q1_us
    $median = Parse-Number $cell.wall_median_us
    $q3 = Parse-Number $cell.wall_q3_us
    $worst = Parse-Number $cell.wall_worst_us
    if (-not ($q1 -le $median -and $median -le $q3 -and $q3 -le $worst)) {
        throw "$($cell.process_run)/$($cell.section) has invalid wall quartile order"
    }
    $cpu = Parse-Number $cell.cpu_median_us
    if ($null -ne $cpu -and $median -gt 0) {
        $cell.operation_busy_cores = ($cpu / $median).ToString('F4', $invariant)
    }
}

foreach ($workload in $scaleWorkloads) {
    $rows = @($okCells | Where-Object {
        $_.section -eq 'operation' -and $_.workload -eq $workload -and
        $_.subject -eq 'independent_query_batch'
    })
    $baseline = $rows | Where-Object { [int]$_.threads -eq 1 } | Select-Object -First 1
    if (-not $baseline) { throw "missing one-thread batch baseline for $workload" }
    $checksum = $baseline.checksum
    foreach ($row in $rows) {
        if ($row.checksum -ne $checksum) { throw "batch checksum drift for $workload" }
        $speedup = (Parse-Number $baseline.wall_median_us) / (Parse-Number $row.wall_median_us)
        $row.speedup = $speedup.ToString('F6', $invariant)
        $row.efficiency = ($speedup / [int]$row.threads).ToString('F6', $invariant)
    }
}

foreach ($processRow in $processRows) {
    if ([uint64]$processRow.external_peak_rss_b -eq 0) {
        throw "$($processRow.process_run) has zero external peak RSS"
    }
}

$dijkstraCells = @($okCells | Where-Object {
    $_.section -eq 'operation' -and $_.subject -eq 'dijkstra'
})
if ($workRows.Count -ne $dijkstraCells.Count) {
    throw "Dijkstra work-row count $($workRows.Count) does not match cell count $($dijkstraCells.Count)"
}
foreach ($work in $workRows) {
    foreach ($field in @('expanded_nodes','expanded_arcs','resolved_incidences','pushes','pops')) {
        if ([uint64]$work.$field -eq 0) { throw "$($work.process_run) has zero $field" }
    }
    if ([uint64]$work.reopened -ne 0) {
        throw "$($work.process_run) reopened a node under the consistent zero heuristic"
    }
}

foreach ($workload in $representationWorkloads) {
    $rows = @($okCells | Where-Object {
        $_.workload -eq $workload -and $_.section -like 'representation_*'
    })
    foreach ($kind in $representationKinds) {
        $prep = @($rows | Where-Object { $_.subject -eq $kind -and $_.section -eq 'representation_preparation' })
        $query = @($rows | Where-Object { $_.subject -eq $kind -and $_.section -eq 'representation_query' })
        if ($prep.Count -ne 1 -or $query.Count -ne 1) { throw "missing representation pair: $workload/$kind" }
        if ($prep[0].checksum -ne $query[0].checksum) { throw "representation checksum drift: $workload/$kind" }
    }
    $checksums = @($rows | Select-Object -ExpandProperty checksum -Unique)
    if ($checksums.Count -ne 1) { throw "representation semantic mismatch for $workload" }
}

Write-Tsv (Join-Path $dataDir 'processes.tsv') $processRows @(
    'process_run','test','workload','subject','threads','process_wall_ms','process_cpu_ms',
    'external_peak_rss_b','exit_code'
)
Write-Tsv (Join-Path $dataDir 'samples.tsv') $sampleRows @(
    'process_run','run','section','workload','subject','threads','ordinal','repetitions',
    'wall_us','cpu_us','checksum'
)
Write-Tsv (Join-Path $dataDir 'cells.tsv') $cellRows @(
    'process_run','run','section','workload','subject','status','threads','nodes','arcs',
    'incidences','warmups','samples','repetitions','wall_median_us','wall_q1_us','wall_q3_us',
    'wall_worst_us','cpu_median_us','cpu_q1_us','cpu_q3_us','cpu_worst_us','checksum',
    'speedup','efficiency','operation_busy_cores','reason'
)
Write-Tsv (Join-Path $dataDir 'rss.tsv') $phaseRows @(
    'process_run','run','workload','subject','phase','working_set_b','self_peak_b',
    'external_process_peak_b'
)
Write-Tsv (Join-Path $dataDir 'representations.tsv') $retainedRows @(
    'process_run','run','workload','representation','representation_b','workspace_b','output_b',
    'semantic_loss'
)
Write-Tsv (Join-Path $dataDir 'search-work.tsv') $workRows @(
    'process_run','run','workload','subject','expanded_nodes','expanded_arcs',
    'resolved_incidences','pushes','pops','stale_pops','heuristic_evals','reopened'
)

$head = (& git -C $repoRoot rev-parse HEAD).Trim()
$rustc = (& rustc --version).Trim()
$cargo = (& cargo --version).Trim()
$os = Get-CimInstance Win32_OperatingSystem
$cpuInfo = Get-CimInstance Win32_Processor | Select-Object -First 1
$manifestRows = @(
    [pscustomobject]@{ key='run'; value=$runName },
    [pscustomobject]@{ key='source_commit'; value=$head },
    [pscustomobject]@{ key='binary_path'; value=$exe },
    [pscustomobject]@{ key='binary_sha256'; value=$exeHash },
    [pscustomobject]@{ key='profile'; value='release' },
    [pscustomobject]@{ key='warmups'; value='2' },
    [pscustomobject]@{ key='samples'; value='9' },
    [pscustomobject]@{ key='cpu_batch_target_us'; value='250000' },
    [pscustomobject]@{ key='random1e6_seed'; value='0x51eed' },
    [pscustomobject]@{ key='workload_definitions'; value='path65k=window_ir(65536,2);grid256=grid_ir(256,256);sts1023=steiner_triple_system(1023);random1e6=uniform_rank_k_ir(0x51eed,200000,333333,3);map512=map_grid_ir(512,512,map_obstacles);arityK_1m=uniform_rank_k_ir(0x51eed^K,200000,floor(1000000/K),K),K=2,3,8,32,128' },
    [pscustomobject]@{ key='logical_processors'; value="$logicalProcessors" },
    [pscustomobject]@{ key='ambient_cpu_avg_pct'; value="$ambientAverage" },
    [pscustomobject]@{ key='ambient_cpu_max_pct'; value="$ambientMaximum" },
    [pscustomobject]@{ key='os'; value="$($os.Caption) build $($os.BuildNumber)" },
    [pscustomobject]@{ key='cpu'; value="$($cpuInfo.Name)" },
    [pscustomobject]@{ key='ram_bytes'; value="$($os.TotalVisibleMemorySize * 1KB)" },
    [pscustomobject]@{ key='rustc'; value=$rustc },
    [pscustomobject]@{ key='cargo'; value=$cargo },
    [pscustomobject]@{ key='command'; value="powershell -NoProfile -ExecutionPolicy Bypass -File scripts/performance_evidence/run.ps1 -RunDate $RunDate" }
)
Write-Tsv (Join-Path $evidenceRoot 'manifest.tsv') $manifestRows @('key','value')

Write-Host "wrote performance evidence to $evidenceRoot"
