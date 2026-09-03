param(
    [string]$RunDate = (Get-Date -Format 'yyyy-MM-dd')
)

$ErrorActionPreference = 'Stop'
$invariant = [Globalization.CultureInfo]::InvariantCulture
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$evidenceRoot = Join-Path $repoRoot "reports\performance-evidence\$RunDate"
$dataDir = Join-Path $evidenceRoot 'data'
$figureDir = Join-Path $evidenceRoot 'figures'
[void](New-Item -ItemType Directory -Force -Path $figureDir)
$cells = Import-Csv -Delimiter "`t" -LiteralPath (Join-Path $dataDir 'cells.tsv')
$rss = Import-Csv -Delimiter "`t" -LiteralPath (Join-Path $dataDir 'rss.tsv')
$utf8 = New-Object Text.UTF8Encoding($false)

function F([double]$value, [string]$pattern = '0.0') {
    $value.ToString($pattern, $invariant)
}

function Write-Svg([string]$name, [Collections.Generic.List[string]]$lines) {
    $lines.Add('</svg>')
    [IO.File]::WriteAllLines((Join-Path $figureDir $name), $lines, $utf8)
}

$colors = @{
    sts1023 = '#287a52'; random1e6 = '#b45f2d'; map512 = '#356cb6'
    canonical_index = '#287a52'; implicit_levi = '#b45f2d'
    sparse_star = '#356cb6'; clique_2section = '#873b9a'
}

# Fixed-work independent-query batch scaling, with IQR whiskers derived from wall-time quartiles.
$scale = @($cells | Where-Object {
    $_.section -eq 'operation' -and $_.subject -eq 'independent_query_batch' -and $_.status -eq 'OK'
})
$svg = [Collections.Generic.List[string]]::new()
$svg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="540" font-family="sans-serif">')
$svg.Add('<rect width="100%" height="100%" fill="white"/>')
$svg.Add('<text x="450" y="28" text-anchor="middle" font-size="18" font-weight="bold">Fixed 64-query batch scaling</text>')
$svg.Add('<text x="450" y="48" text-anchor="middle" font-size="11">median speedup; whiskers propagate each cell&apos;s wall-time IQR</text>')
$left = 75.0; $right = 850.0; $top = 65.0; $bottom = 465.0
$maxThreads = ($scale | ForEach-Object { [int]$_.threads } | Measure-Object -Maximum).Maximum
$maxSpeedup = [Math]::Max(2.0, ($scale | ForEach-Object { [double]$_.speedup } | Measure-Object -Maximum).Maximum * 1.12)
$xOf = { param([int]$thread) $left + [Math]::Log($thread, 2) / [Math]::Log($maxThreads, 2) * ($right - $left) }
$yOf = { param([double]$speedup) $bottom - $speedup / $maxSpeedup * ($bottom - $top) }
for ($tick = 0; $tick -le [Math]::Ceiling($maxSpeedup); $tick += 2) {
    $y = & $yOf $tick
    $svg.Add(('<line x1="{0}" y1="{1}" x2="{2}" y2="{1}" stroke="#e8e8e8"/>' -f (F $left), (F $y), (F $right)))
    $svg.Add(('<text x="68" y="{0}" text-anchor="end" font-size="10">{1}x</text>' -f (F ($y + 4)), $tick))
}
foreach ($thread in @($scale | ForEach-Object { [int]$_.threads } | Sort-Object -Unique)) {
    $x = & $xOf $thread
    $svg.Add(('<line x1="{0}" y1="{1}" x2="{0}" y2="{2}" stroke="#f1f1f1"/>' -f (F $x), (F $top), (F $bottom)))
    $svg.Add(('<text x="{0}" y="485" text-anchor="middle" font-size="10">{1}</text>' -f (F $x), $thread))
}
$legendY = 72
foreach ($workload in @('sts1023','random1e6','map512')) {
    $rows = @($scale | Where-Object { $_.workload -eq $workload } | Sort-Object { [int]$_.threads })
    if ($rows.Count -eq 0) { continue }
    $baselineMedian = [double]($rows | Where-Object { [int]$_.threads -eq 1 } | Select-Object -First 1).wall_median_us
    $points = [Collections.Generic.List[string]]::new()
    foreach ($row in $rows) {
        $x = & $xOf ([int]$row.threads)
        $speedup = [double]$row.speedup
        $y = & $yOf $speedup
        $low = $baselineMedian / [double]$row.wall_q3_us
        $high = $baselineMedian / [double]$row.wall_q1_us
        $yLow = & $yOf $low
        $yHigh = & $yOf $high
        $svg.Add(('<line x1="{0}" y1="{1}" x2="{0}" y2="{2}" stroke="{3}"/>' -f (F $x), (F $yHigh), (F $yLow), $colors[$workload]))
        $points.Add(('{0},{1}' -f (F $x), (F $y)))
        $svg.Add(('<circle cx="{0}" cy="{1}" r="4" fill="{2}"/>' -f (F $x), (F $y), $colors[$workload]))
    }
    $svg.Add(('<polyline points="{0}" fill="none" stroke="{1}" stroke-width="2.5"/>' -f ($points -join ' '), $colors[$workload]))
    $svg.Add(('<rect x="690" y="{0}" width="12" height="12" fill="{1}"/>' -f $legendY, $colors[$workload]))
    $svg.Add(('<text x="708" y="{0}" font-size="11">{1}</text>' -f ($legendY + 11), $workload))
    $legendY += 20
}
$svg.Add(('<line x1="{0}" y1="{1}" x2="{2}" y2="{3}" stroke="#999" stroke-dasharray="5 4"/>' -f (F (& $xOf 1)), (F (& $yOf 1)), (F (& $xOf $maxThreads)), (F (& $yOf $maxThreads))))
$svg.Add('<line x1="75" y1="465" x2="850" y2="465" stroke="#333"/>')
$svg.Add('<line x1="75" y1="65" x2="75" y2="465" stroke="#333"/>')
$svg.Add('<text x="462" y="518" text-anchor="middle" font-size="12">worker threads (log2 position)</text>')
$svg.Add('<text x="20" y="265" text-anchor="middle" font-size="12" transform="rotate(-90 20 265)">speedup vs one thread</text>')
Write-Svg 'batch-scaling.svg' $svg

# RSS stages for the Levi operation; phase observations and the independently polled process peak.
$memoryRows = @($rss | Where-Object { $_.subject -eq 'levi' })
$phaseOrder = @('process_baseline','canonical_ir','incidence_index','prepared','measured','external_peak')
$svg = [Collections.Generic.List[string]]::new()
$svg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540" font-family="sans-serif">')
$svg.Add('<rect width="100%" height="100%" fill="white"/>')
$svg.Add('<text x="480" y="28" text-anchor="middle" font-size="18" font-weight="bold">Fresh-process Levi resident memory</text>')
$svg.Add('<text x="480" y="48" text-anchor="middle" font-size="11">self-observed phase working set plus independently polled child peak</text>')
$left = 80.0; $right = 920.0; $top = 65.0; $bottom = 450.0
$maxMiB = 1.0
$seriesMemory = @{}
foreach ($workload in @($memoryRows | Select-Object -ExpandProperty workload -Unique)) {
    $rows = @($memoryRows | Where-Object { $_.workload -eq $workload })
    $values = [ordered]@{}
    foreach ($phase in $phaseOrder[0..4]) {
        $row = $rows | Where-Object { $_.phase -eq $phase } | Select-Object -First 1
        if ($row -and $row.working_set_b -ne 'NA') { $values[$phase] = [double]$row.working_set_b / 1MB }
    }
    $values['external_peak'] = [double]$rows[0].external_process_peak_b / 1MB
    $seriesMemory[$workload] = $values
    $candidate = ($values.Values | Measure-Object -Maximum).Maximum
    $maxMiB = [Math]::Max($maxMiB, $candidate)
}
$maxMiB *= 1.12
$xStep = ($right - $left) / ($phaseOrder.Count - 1)
$yMemory = { param([double]$mib) $bottom - $mib / $maxMiB * ($bottom - $top) }
for ($tick = 0; $tick -le $maxMiB; $tick += [Math]::Max(25, [Math]::Ceiling($maxMiB / 6 / 25) * 25)) {
    $y = & $yMemory $tick
    $svg.Add(('<line x1="80" y1="{0}" x2="920" y2="{0}" stroke="#e8e8e8"/>' -f (F $y)))
    $svg.Add(('<text x="72" y="{0}" text-anchor="end" font-size="10">{1}</text>' -f (F ($y + 4)), (F $tick '0')))
}
for ($i = 0; $i -lt $phaseOrder.Count; $i++) {
    $x = $left + $i * $xStep
    $label = $phaseOrder[$i] -replace '_',' '
    $svg.Add(('<text x="{0}" y="475" text-anchor="middle" font-size="10">{1}</text>' -f (F $x), $label))
}
$legendY = 72
foreach ($workload in @('path65k','grid256','sts1023','random1e6','map512')) {
    if (-not $seriesMemory.ContainsKey($workload)) { continue }
    $color = if ($colors.ContainsKey($workload)) { $colors[$workload] } else { @{'path65k'='#555';'grid256'='#9a7635'}[$workload] }
    $points = [Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $phaseOrder.Count; $i++) {
        $phase = $phaseOrder[$i]
        if (-not $seriesMemory[$workload].Contains($phase)) { continue }
        $x = $left + $i * $xStep
        $y = & $yMemory ([double]$seriesMemory[$workload][$phase])
        $points.Add(('{0},{1}' -f (F $x), (F $y)))
        $svg.Add(('<circle cx="{0}" cy="{1}" r="3.5" fill="{2}"/>' -f (F $x), (F $y), $color))
    }
    $svg.Add(('<polyline points="{0}" fill="none" stroke="{1}" stroke-width="2.2"/>' -f ($points -join ' '), $color))
    $svg.Add(('<rect x="790" y="{0}" width="12" height="12" fill="{1}"/>' -f $legendY, $color))
    $svg.Add(('<text x="808" y="{0}" font-size="10">{1}</text>' -f ($legendY + 10), $workload))
    $legendY += 18
}
$svg.Add('<line x1="80" y1="450" x2="920" y2="450" stroke="#333"/>')
$svg.Add('<line x1="80" y1="65" x2="80" y2="450" stroke="#333"/>')
$svg.Add('<text x="20" y="260" text-anchor="middle" font-size="12" transform="rotate(-90 20 260)">resident memory (MiB)</text>')
Write-Svg 'process-memory.svg' $svg

# Lifecycle-equivalent representation totals and dominance relative to canonical_index.
$totalRows = [Collections.Generic.List[object]]::new()
$representationWorkloads = @($cells | Where-Object { $_.section -eq 'representation_query' } |
    Select-Object -ExpandProperty workload -Unique)
foreach ($workload in $representationWorkloads) {
    $canonicalPrep = [double]($cells | Where-Object {
        $_.workload -eq $workload -and $_.section -eq 'representation_preparation' -and $_.subject -eq 'canonical_index'
    } | Select-Object -First 1).wall_median_us
    $canonicalQuery = [double]($cells | Where-Object {
        $_.workload -eq $workload -and $_.section -eq 'representation_query' -and $_.subject -eq 'canonical_index'
    } | Select-Object -First 1).wall_median_us
    foreach ($kind in @('canonical_index','implicit_levi','sparse_star','clique_2section')) {
        $prep = [double]($cells | Where-Object {
            $_.workload -eq $workload -and $_.section -eq 'representation_preparation' -and $_.subject -eq $kind
        } | Select-Object -First 1).wall_median_us
        $query = [double]($cells | Where-Object {
            $_.workload -eq $workload -and $_.section -eq 'representation_query' -and $_.subject -eq $kind
        } | Select-Object -First 1).wall_median_us
        if ($kind -eq 'canonical_index') {
            $dominance = 'BASELINE'
        } elseif ($prep -le $canonicalPrep -and $query -le $canonicalQuery) {
            $dominance = 'ALWAYS_BETTER'
        } elseif ($prep -ge $canonicalPrep -and $query -ge $canonicalQuery) {
            $dominance = 'NEVER_BETTER'
        } elseif ($query -lt $canonicalQuery) {
            $threshold = [Math]::Floor(($prep - $canonicalPrep) / ($canonicalQuery - $query)) + 1
            $dominance = "BETTER_FROM_Q>=$([Math]::Max(1, $threshold))"
        } else {
            $last = [Math]::Ceiling(($canonicalPrep - $prep) / ($query - $canonicalQuery)) - 1
            $dominance = "BETTER_UNTIL_Q<=$([Math]::Max(0, $last))"
        }
        $totalRows.Add([pscustomobject][ordered]@{
            workload = $workload; representation = $kind
            prep_us = $prep.ToString('F3', $invariant); query_us = $query.ToString('F3', $invariant)
            total_q1_us = ($prep + $query).ToString('F3', $invariant)
            total_q16_us = ($prep + 16 * $query).ToString('F3', $invariant)
            total_q256_us = ($prep + 256 * $query).ToString('F3', $invariant)
            dominance_vs_canonical = $dominance
        })
    }
}
$columns = @('workload','representation','prep_us','query_us','total_q1_us','total_q16_us','total_q256_us','dominance_vs_canonical')
$lines = [Collections.Generic.List[string]]::new(); $lines.Add(($columns -join "`t"))
foreach ($row in $totalRows) { $lines.Add((($columns | ForEach-Object { $row.$_ }) -join "`t")) }
[IO.File]::WriteAllLines((Join-Path $dataDir 'representation-totals.tsv'), $lines, $utf8)

$plotWorkload = if ($representationWorkloads -contains 'sts1023') { 'sts1023' } else { $representationWorkloads[0] }
$plotRows = @($totalRows | Where-Object { $_.workload -eq $plotWorkload })
$queries = @(1,16,256)
$maximumTotal = ($plotRows | ForEach-Object { [double]$_.total_q256_us } | Measure-Object -Maximum).Maximum
$minimumTotal = ($plotRows | ForEach-Object { [double]$_.total_q1_us } | Measure-Object -Minimum).Minimum
$logMin = [Math]::Log10([Math]::Max(1.0, $minimumTotal)); $logMax = [Math]::Log10($maximumTotal)
$svg = [Collections.Generic.List[string]]::new()
$svg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="540" font-family="sans-serif">')
$svg.Add('<rect width="100%" height="100%" fill="white"/>')
$svg.Add(('<text x="450" y="28" text-anchor="middle" font-size="18" font-weight="bold">Lifecycle-equivalent component count — {0}</text>' -f $plotWorkload))
$svg.Add('<text x="450" y="48" text-anchor="middle" font-size="11">total(Q) = representation preparation + Q × recomputed component-count query</text>')
$left = 85.0; $right = 840.0; $top = 70.0; $bottom = 455.0
$xPosition = @{1=130.0;16=445.0;256=760.0}
$yTotal = { param([double]$value) $bottom - ([Math]::Log10($value) - $logMin) / ($logMax - $logMin) * ($bottom - $top) }
foreach ($queryCount in $queries) {
    $x = $xPosition[$queryCount]
    $svg.Add(('<line x1="{0}" y1="70" x2="{0}" y2="455" stroke="#eee"/>' -f (F $x)))
    $svg.Add(('<text x="{0}" y="480" text-anchor="middle" font-size="11">Q={1}</text>' -f (F $x), $queryCount))
}
$legendY = 72
foreach ($row in $plotRows) {
    $kind = $row.representation
    $values = @([double]$row.total_q1_us,[double]$row.total_q16_us,[double]$row.total_q256_us)
    $points = [Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $queries.Count; $i++) {
        $x = $xPosition[$queries[$i]]; $y = & $yTotal $values[$i]
        $points.Add(('{0},{1}' -f (F $x), (F $y)))
        $svg.Add(('<circle cx="{0}" cy="{1}" r="4" fill="{2}"/>' -f (F $x), (F $y), $colors[$kind]))
    }
    $svg.Add(('<polyline points="{0}" fill="none" stroke="{1}" stroke-width="2.5"/>' -f ($points -join ' '), $colors[$kind]))
    $svg.Add(('<rect x="650" y="{0}" width="12" height="12" fill="{1}"/>' -f $legendY, $colors[$kind]))
    $svg.Add(('<text x="668" y="{0}" font-size="10">{1}</text>' -f ($legendY + 10), ($kind -replace '_',' ')))
    $legendY += 18
}
$svg.Add('<line x1="85" y1="455" x2="840" y2="455" stroke="#333"/>')
$svg.Add('<line x1="85" y1="70" x2="85" y2="455" stroke="#333"/>')
$svg.Add('<text x="20" y="260" text-anchor="middle" font-size="12" transform="rotate(-90 20 260)">total time (µs, log scale)</text>')
Write-Svg 'representation-total.svg' $svg

Write-Host "wrote performance-evidence figures to $figureDir"
