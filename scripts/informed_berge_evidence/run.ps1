param(
    [string]$RunDate = (Get-Date -Format 'yyyy-MM-dd'),
    [string]$ScratchRoot = (Join-Path ([IO.Path]::GetTempPath()) 'hymeko-informed-berge-evidence'),
    [switch]$Quick,
    [switch]$SkipQuietHostGate
)

$ErrorActionPreference = 'Stop'
$invariant = [Globalization.CultureInfo]::InvariantCulture
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$targetDir = Join-Path $ScratchRoot 'target'
$tempDir = Join-Path $ScratchRoot 'tmp'
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $ScratchRoot 'cargo-home' }
$evidenceRoot = Join-Path $repoRoot "reports\informed-berge-evidence\$RunDate"
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
    [IO.File]::WriteAllLines($path, $lines, [Text.UTF8Encoding]::new($false))
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
    if (-not $accepted) { throw 'quiet-host gate failed: five-sample average CPU remained above 25%' }
} else {
    $ambientAverage = 'SKIPPED'
    $ambientMaximum = 'SKIPPED'
}

Push-Location $repoRoot
try {
    $buildMessages = @(& cargo test -p hymeko_graph --release --test informed_berge_evidence --no-run --message-format=json-render-diagnostics |
        ForEach-Object { $_ | ConvertFrom-Json })
    if ($LASTEXITCODE -ne 0) { throw "informed_berge_evidence build failed with exit $LASTEXITCODE" }
} finally {
    Pop-Location
}

$artifacts = @($buildMessages | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'informed_berge_evidence' -and
    $_.target.kind -contains 'test' -and $_.executable
})
if ($artifacts.Count -ne 1) { throw "Expected one informed_berge_evidence executable; got $($artifacts.Count)" }
$exe = (Get-Item -LiteralPath $artifacts[0].executable).FullName
$exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLowerInvariant()

$processRows = [Collections.Generic.List[object]]::new()
$sampleRows = [Collections.Generic.List[object]]::new()
$cellRows = [Collections.Generic.List[object]]::new()
$phaseRows = [Collections.Generic.List[object]]::new()
$workRows = [Collections.Generic.List[object]]::new()
$resultRows = [Collections.Generic.List[object]]::new()

function Invoke-EvidenceProcess([string]$testName, [string]$workload, [string]$subject) {
    $processRun = "$testName-$workload-$subject"
    Write-Host "run $processRun"
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $exe
    $psi.Arguments = "$testName --exact --ignored --nocapture"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['INFORMED_BERGE_RUN'] = $processRun
    $psi.EnvironmentVariables['INFORMED_BERGE_WORKLOAD'] = $workload
    $psi.EnvironmentVariables['INFORMED_BERGE_SUBJECT'] = $subject

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
        } catch {}
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
        process_run = $processRun; test = $testName; workload = $workload; subject = $subject
        process_wall_ms = $clock.Elapsed.TotalMilliseconds.ToString('F3', $invariant)
        process_cpu_ms = if ($null -eq $processCpuMs) { 'UNMEASURED' } else { $processCpuMs.ToString('F3', $invariant) }
        external_peak_rss_b = [uint64]$externalPeak; exit_code = $exitCode
    })
    if ($exitCode -ne 0) {
        throw "cell $processRun failed with exit $exitCode`nSTDERR:`n$stderr`nSTDOUT:`n$stdout"
    }
    if ($stderr) { Write-Verbose $stderr }

    $evidenceLines = @($stdout -split "`r?`n" | Where-Object { $_ -like 'EVIDENCE_*' })
    if (@($evidenceLines | Where-Object { $_ -like 'EVIDENCE_CELL*' }).Count -ne 1) {
        throw "cell $processRun did not emit exactly one EVIDENCE_CELL"
    }
    if (@($evidenceLines | Where-Object { $_ -like 'EVIDENCE_RESULT*' }).Count -ne 1) {
        throw "cell $processRun did not emit exactly one EVIDENCE_RESULT"
    }
    foreach ($line in $evidenceLines) {
        $value = Parse-EvidenceLine $line
        switch ($value.kind) {
            'EVIDENCE_SAMPLE' {
                $sampleRows.Add([pscustomobject][ordered]@{
                    process_run=$processRun; run=$value.run; section=$value.section
                    workload=$value.workload; subject=$value.subject; ordinal=$value.ordinal
                    repetitions=$value.repetitions; wall_us=$value.wall_us; cpu_us=$value.cpu_us
                    checksum=$value.checksum
                })
            }
            'EVIDENCE_CELL' {
                $cellRows.Add([pscustomobject][ordered]@{
                    process_run=$processRun; run=$value.run; section=$value.section
                    workload=$value.workload; subject=$value.subject; status=$value.status
                    nodes=$value.nodes; arcs=$value.arcs; incidences=$value.incidences
                    warmups=$value.warmups; samples=$value.samples; repetitions=$value.repetitions
                    wall_median_us=$value.wall_median_us; wall_q1_us=$value.wall_q1_us
                    wall_q3_us=$value.wall_q3_us; wall_worst_us=$value.wall_worst_us
                    cpu_median_us=$value.cpu_median_us; cpu_q1_us=$value.cpu_q1_us
                    cpu_q3_us=$value.cpu_q3_us; cpu_worst_us=$value.cpu_worst_us
                    checksum=$value.checksum
                })
            }
            'EVIDENCE_PHASE' {
                $phaseRows.Add([pscustomobject][ordered]@{
                    process_run=$processRun; run=$value.run; workload=$value.workload
                    subject=$value.subject; phase=$value.phase; working_set_b=$value.working_set_b
                    self_peak_b=$value.self_peak_b; external_process_peak_b=[uint64]$externalPeak
                })
            }
            'EVIDENCE_WORK' {
                $workRows.Add([pscustomobject][ordered]@{
                    process_run=$processRun; run=$value.run; workload=$value.workload
                    subject=$value.subject; expanded_nodes=$value.expanded_nodes
                    expanded_arcs=$value.expanded_arcs; resolved_incidences=$value.resolved_incidences
                    pushes=$value.pushes; pops=$value.pops; stale_pops=$value.stale_pops
                    heuristic_evals=$value.heuristic_evals; reopened=$value.reopened
                })
            }
            'EVIDENCE_RESULT' {
                $resultRows.Add([pscustomobject][ordered]@{
                    process_run=$processRun; run=$value.run; section=$value.section
                    workload=$value.workload; subject=$value.subject; reachable=$value.reachable
                    cost_micros=$value.cost_micros; hops=$value.hops; path_hash=$value.path_hash
                    items=$value.items; bound=$value.bound; max_depth=$value.max_depth
                    max_results=$value.max_results; max_expansions=$value.max_expansions
                })
            }
        }
    }
}

$searchMatrix = [Collections.Generic.List[object]]::new()
function Add-Search([string]$workload, [string[]]$subjects) {
    foreach ($subject in $subjects) {
        $searchMatrix.Add([pscustomobject]@{ workload=$workload; subject=$subject })
    }
}

if ($Quick) {
    Add-Search 'grid200_d40' @('dijkstra_unit','astar_l1','astar_l2')
    Add-Search 'map32' @('dijkstra_weight','astar_metric','astar_discrete','astar_combined')
    Add-Search 'transport128' @('dijkstra_weight','astar_discrete')
    $preparationWorkloads = @('map32','transport128')
    $shortestWorkloads = @('path65k')
    $enumerationMatrix = @(
        [pscustomobject]@{workload='layered10';subject='paths_goal'},
        [pscustomobject]@{workload='parallel16';subject='cycles'}
    )
} else {
    foreach ($distance in @(20,40,80)) {
        Add-Search "grid200_d$distance" @('dijkstra_unit','astar_l1','astar_l2')
    }
    foreach ($map in @('map32','map128','map512','corridor128')) {
        Add-Search $map @('dijkstra_weight','astar_metric','astar_discrete','astar_combined')
    }
    Add-Search 'transport128' @('dijkstra_weight','astar_discrete')
    $preparationWorkloads = @('map32','map128','map512','corridor128','transport128')
    $shortestWorkloads = @('path65k','grid256','sts1023','random1e6','map512')
    $enumerationMatrix = @(
        [pscustomobject]@{workload='layered10';subject='paths_goal'},
        [pscustomobject]@{workload='grid6';subject='paths_goal'},
        [pscustomobject]@{workload='grid5';subject='cycles'},
        [pscustomobject]@{workload='sts31';subject='paths_all'},
        [pscustomobject]@{workload='sts31';subject='cycles'},
        [pscustomobject]@{workload='parallel16';subject='cycles'}
    )
}

foreach ($entry in $searchMatrix) {
    Invoke-EvidenceProcess 'search_cell' $entry.workload $entry.subject
}
foreach ($workload in $preparationWorkloads) {
    Invoke-EvidenceProcess 'preparation_cell' $workload 'discrete_hops'
}
foreach ($workload in $shortestWorkloads) {
    foreach ($subject in @('bfs_shortest','dijkstra_unit')) {
        Invoke-EvidenceProcess 'shortest_cell' $workload $subject
    }
}
foreach ($entry in $enumerationMatrix) {
    Invoke-EvidenceProcess 'enumeration_cell' $entry.workload $entry.subject
}

if ($processRows.Count -ne $cellRows.Count -or $cellRows.Count -ne $resultRows.Count) {
    throw "process/cell/result cardinality mismatch: $($processRows.Count)/$($cellRows.Count)/$($resultRows.Count)"
}
foreach ($process in $processRows) {
    if ([int]$process.exit_code -ne 0 -or [uint64]$process.external_peak_rss_b -eq 0) {
        throw "invalid process evidence for $($process.process_run)"
    }
}
foreach ($cell in $cellRows) {
    if ($cell.status -ne 'OK' -or [int]$cell.samples -ne 9) {
        throw "$($cell.process_run) is not a nine-sample OK cell"
    }
    $samples = @($sampleRows | Where-Object { $_.process_run -eq $cell.process_run })
    if ($samples.Count -ne 9) { throw "$($cell.process_run) raw sample count is $($samples.Count)" }
    if (@($samples | Select-Object -ExpandProperty checksum -Unique).Count -ne 1) {
        throw "$($cell.process_run) checksum drift"
    }
    $q1 = Parse-Number $cell.wall_q1_us; $median = Parse-Number $cell.wall_median_us
    $q3 = Parse-Number $cell.wall_q3_us; $worst = Parse-Number $cell.wall_worst_us
    if (-not ($q1 -le $median -and $median -le $q3 -and $q3 -le $worst)) {
        throw "$($cell.process_run) invalid wall quartiles"
    }
    if ($cell.cpu_median_us -eq 'NA') {
        throw "$($cell.process_run) failed to rise above the process-CPU timer resolution"
    }
    $result = $resultRows | Where-Object { $_.process_run -eq $cell.process_run } | Select-Object -First 1
    if ($result.path_hash -ne $cell.checksum) { throw "$($cell.process_run) result/cell hash mismatch" }
}

$informedResults = @($resultRows | Where-Object { $_.section -eq 'informed_search' })
foreach ($group in $informedResults | Group-Object workload) {
    $reachability = @($group.Group | Select-Object -ExpandProperty reachable -Unique)
    if ($reachability.Count -ne 1 -or $reachability[0] -ne 'true') {
        throw "$($group.Name) reachability mismatch"
    }
    if (@($group.Group | Select-Object -ExpandProperty cost_micros -Unique).Count -ne 1) {
        throw "$($group.Name) optimal-cost mismatch"
    }
}

$shortestResults = @($resultRows | Where-Object { $_.section -eq 'berge_shortest' })
foreach ($group in $shortestResults | Group-Object workload) {
    if ($group.Count -ne 2) { throw "$($group.Name) missing BFS/Dijkstra pair" }
    if (@($group.Group | Select-Object -ExpandProperty reachable -Unique).Count -ne 1 -or
        @($group.Group | Select-Object -ExpandProperty hops -Unique).Count -ne 1) {
        throw "$($group.Name) BFS/Dijkstra semantic mismatch"
    }
}

$expectedWorkRows = $informedResults.Count + $shortestWorkloads.Count
if ($workRows.Count -ne $expectedWorkRows) {
    throw "search-work row count $($workRows.Count) does not match $expectedWorkRows"
}
foreach ($work in $workRows) {
    foreach ($field in @('expanded_nodes','expanded_arcs','resolved_incidences','pushes','pops')) {
        if ([uint64]$work.$field -eq 0) { throw "$($work.process_run) has zero $field" }
    }
    if ([uint64]$work.reopened -ne 0) { throw "$($work.process_run) reopened under a consistent heuristic" }
}

$enumerationResults = @($resultRows | Where-Object { $_.section -eq 'berge_enumeration' })
foreach ($result in $enumerationResults) {
    if ($result.bound -notin @('none','depth','results','expansions')) {
        throw "$($result.process_run) invalid enumeration bound"
    }
    if ([uint64]$result.items -gt [uint64]$result.max_results) {
        throw "$($result.process_run) exceeded its result budget"
    }
    if ($result.bound -eq 'results' -and [uint64]$result.items -ne [uint64]$result.max_results) {
        throw "$($result.process_run) result-bound cardinality mismatch"
    }
}
$parallel = $enumerationResults | Where-Object { $_.workload -eq 'parallel16' } | Select-Object -First 1
if (-not $parallel -or [uint64]$parallel.items -ne 120 -or $parallel.bound -ne 'none') {
    throw 'parallel16 must expose all C(16,2)=120 cycles without truncation'
}

Write-Tsv (Join-Path $dataDir 'processes.tsv') $processRows @(
    'process_run','test','workload','subject','process_wall_ms','process_cpu_ms','external_peak_rss_b','exit_code'
)
Write-Tsv (Join-Path $dataDir 'samples.tsv') $sampleRows @(
    'process_run','run','section','workload','subject','ordinal','repetitions','wall_us','cpu_us','checksum'
)
Write-Tsv (Join-Path $dataDir 'cells.tsv') $cellRows @(
    'process_run','run','section','workload','subject','status','nodes','arcs','incidences','warmups','samples',
    'repetitions','wall_median_us','wall_q1_us','wall_q3_us','wall_worst_us','cpu_median_us','cpu_q1_us',
    'cpu_q3_us','cpu_worst_us','checksum'
)
Write-Tsv (Join-Path $dataDir 'rss.tsv') $phaseRows @(
    'process_run','run','workload','subject','phase','working_set_b','self_peak_b','external_process_peak_b'
)
Write-Tsv (Join-Path $dataDir 'search-work.tsv') $workRows @(
    'process_run','run','workload','subject','expanded_nodes','expanded_arcs','resolved_incidences',
    'pushes','pops','stale_pops','heuristic_evals','reopened'
)
Write-Tsv (Join-Path $dataDir 'results.tsv') $resultRows @(
    'process_run','run','section','workload','subject','reachable','cost_micros','hops','path_hash','items',
    'bound','max_depth','max_results','max_expansions'
)

$head = (& git -C $repoRoot rev-parse HEAD).Trim()
$rustc = (& rustc --version).Trim()
$cargo = (& cargo --version).Trim()
$os = Get-CimInstance Win32_OperatingSystem
$cpuInfo = Get-CimInstance Win32_Processor | Select-Object -First 1
$oracleBlob = (& git -C $repoRoot hash-object 'hymeko_graph/tests/informed_common/mod.rs').Trim()
$clientBlob = (& git -C $repoRoot hash-object 'hymeko_graph/tests/informed_berge_evidence.rs').Trim()
$command = "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/informed_berge_evidence/run.ps1 -RunDate $RunDate -ScratchRoot $ScratchRoot"
$manifestRows = @(
    [pscustomobject]@{key='run';value=$runName},
    [pscustomobject]@{key='source_commit';value=$head},
    [pscustomobject]@{key='evidence_client_blob';value=$clientBlob},
    [pscustomobject]@{key='independent_oracle_blob';value=$oracleBlob},
    [pscustomobject]@{key='binary_path';value=$exe},
    [pscustomobject]@{key='binary_sha256';value=$exeHash},
    [pscustomobject]@{key='profile';value='release'},
    [pscustomobject]@{key='warmups';value='2'},
    [pscustomobject]@{key='samples';value='9'},
    [pscustomobject]@{key='cpu_batch_target_us';value='250000'},
    [pscustomobject]@{key='search_contract';value='full path hash outside timed boundary; equal optimal cost per workload; deterministic SearchStats; consistent heuristics require zero reopenings'},
    [pscustomobject]@{key='shortest_contract';value='public BFS Berge shortest and unit-cost Dijkstra agree on reachability and hyperedge count'},
    [pscustomobject]@{key='enumeration_contract';value='full ordered output hash outside timing; finite depth/result/expansion budgets; explicit stopping reason'},
    [pscustomobject]@{key='dstar_status';value='NOT_IMPLEMENTED; repeated A* is not reported as D* or LPA*'},
    [pscustomobject]@{key='logical_processors';value="$([Environment]::ProcessorCount)"},
    [pscustomobject]@{key='ambient_cpu_avg_pct';value="$ambientAverage"},
    [pscustomobject]@{key='ambient_cpu_max_pct';value="$ambientMaximum"},
    [pscustomobject]@{key='os';value="$($os.Caption) build $($os.BuildNumber)"},
    [pscustomobject]@{key='cpu';value="$($cpuInfo.Name)"},
    [pscustomobject]@{key='ram_bytes';value="$($os.TotalVisibleMemorySize * 1KB)"},
    [pscustomobject]@{key='rustc';value=$rustc},
    [pscustomobject]@{key='cargo';value=$cargo},
    [pscustomobject]@{key='command';value=$command}
)
Write-Tsv (Join-Path $evidenceRoot 'manifest.tsv') $manifestRows @('key','value')
Write-Host "wrote informed Berge evidence to $evidenceRoot"
