param(
    [string]$InputTsv = "reports/data/2026-09-03-codex-p34-rerun.tsv",
    [string]$OutputDir = "reports/figures"
)

$ErrorActionPreference = 'Stop'
$rows = Import-Csv -Delimiter "`t" -LiteralPath $InputTsv
$invariant = [Globalization.CultureInfo]::InvariantCulture

function F([double]$value) {
    $value.ToString('0.0', $invariant)
}

$colors = @{
    sts1023 = '#285'
    random1e6 = '#b63'
    map512 = '#397'
}
$threadX = @{ 1 = 70.0; 2 = 213.0; 4 = 357.0; 8 = 500.0; 16 = 644.0; 24 = 727.0 }
$scaleSvg = [Collections.Generic.List[string]]::new()
$scaleSvg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="800" height="500" font-family="sans-serif">')
$scaleSvg.Add('<text x="400" y="26" text-anchor="middle" font-size="17" font-weight="bold">Independent P3.4 rerun: 64-query batch scaling</text>')
$scaleSvg.Add('<line x1="70" y1="430" x2="740" y2="430" stroke="#333"/>')
$scaleSvg.Add('<line x1="70" y1="50" x2="70" y2="430" stroke="#333"/>')
foreach ($tick in @(1, 2, 4, 8, 16, 24)) {
    $x = $threadX[$tick]
    $scaleSvg.Add(('<line x1="{0}" y1="430" x2="{0}" y2="435" stroke="#333"/>' -f (F $x)))
    $scaleSvg.Add(('<text x="{0}" y="452" text-anchor="middle" font-size="11">{1}</text>' -f (F $x), $tick))
}
foreach ($tick in @(0, 2, 4, 6, 8, 10, 12, 14)) {
    $y = 430.0 - $tick / 14.0 * 380.0
    $scaleSvg.Add(('<line x1="65" y1="{0}" x2="740" y2="{0}" stroke="#eee"/>' -f (F $y)))
    $scaleSvg.Add(('<text x="60" y="{0}" text-anchor="end" font-size="10">{1}x</text>' -f (F ($y + 4)), $tick))
}
$legendY = 58
foreach ($workload in @('sts1023', 'random1e6', 'map512')) {
    $points = $rows | Where-Object { $_.section -eq 'batch_scaling' -and $_.workload -eq $workload }
    $coords = foreach ($row in $points) {
        $thread = [int]($row.variant -split '_')[0]
        $x = $threadX[$thread]
        $y = 430.0 - [double]$row.value / 14.0 * 380.0
        '{0},{1}' -f (F $x), (F $y)
    }
    $color = $colors[$workload]
    $scaleSvg.Add(('<polyline points="{0}" fill="none" stroke="{1}" stroke-width="2.5"/>' -f ($coords -join ' '), $color))
    foreach ($coord in $coords) {
        $xy = $coord -split ','
        $scaleSvg.Add(('<circle cx="{0}" cy="{1}" r="3.5" fill="{2}"/>' -f $xy[0], $xy[1], $color))
    }
    $scaleSvg.Add(('<rect x="610" y="{0}" width="12" height="12" fill="{1}"/>' -f $legendY, $color))
    $scaleSvg.Add(('<text x="628" y="{0}" font-size="11">{1}</text>' -f ($legendY + 11), $workload))
    $legendY += 19
}
$scaleSvg.Add('<text x="405" y="480" text-anchor="middle" font-size="12">Rayon threads (log-spaced)</text>')
$scaleSvg.Add('<text x="18" y="240" text-anchor="middle" font-size="12" transform="rotate(-90 18 240)">speedup vs 1 thread</text>')
$scaleSvg.Add('</svg>')
[IO.File]::WriteAllLines((Join-Path $OutputDir 'codex-p34-batch-scaling.svg'), $scaleSvg)

$variants = @(
    @{ Key = 'direct_cached_count'; Label = 'direct'; Color = '#285' },
    @{ Key = 'levi_cached_count'; Label = 'Levi'; Color = '#b63' },
    @{ Key = 'sparse_star_cached_count'; Label = 'sparse star'; Color = '#397' },
    @{ Key = 'clique_cached_count'; Label = 'clique'; Color = '#936' }
)
$groups = @('sts1023', 'random1e6', 'map512')
$barSvg = [Collections.Generic.List[string]]::new()
$barSvg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="500" font-family="sans-serif">')
$barSvg.Add('<text x="450" y="26" text-anchor="middle" font-size="17" font-weight="bold">Like-for-like cached component count: preparation only</text>')
$barSvg.Add('<line x1="70" y1="420" x2="860" y2="420" stroke="#333"/>')
$barSvg.Add('<line x1="70" y1="55" x2="70" y2="420" stroke="#333"/>')
foreach ($tick in @(0, 10, 20, 30, 40, 50, 60)) {
    $y = 420.0 - $tick / 65.0 * 350.0
    $barSvg.Add(('<line x1="65" y1="{0}" x2="860" y2="{0}" stroke="#eee"/>' -f (F $y)))
    $barSvg.Add(('<text x="60" y="{0}" text-anchor="end" font-size="10">{1}</text>' -f (F ($y + 4)), $tick))
}
for ($g = 0; $g -lt $groups.Count; $g++) {
    $group = $groups[$g]
    $baseX = 115.0 + $g * 250.0
    for ($v = 0; $v -lt $variants.Count; $v++) {
        $variant = $variants[$v]
        $row = $rows | Where-Object { $_.section -eq 'component_count_prep' -and $_.workload -eq $group -and $_.variant -eq $variant.Key } | Select-Object -First 1
        $ms = [double]$row.median_us / 1000.0
        $height = $ms / 65.0 * 350.0
        $x = $baseX + $v * 42.0
        $y = 420.0 - $height
        $barSvg.Add(('<rect x="{0}" y="{1}" width="34" height="{2}" fill="{3}"/>' -f (F $x), (F $y), (F $height), $variant.Color))
        $barSvg.Add(('<text x="{0}" y="{1}" text-anchor="middle" font-size="9">{2}</text>' -f (F ($x + 17)), (F ($y - 4)), ([math]::Round($ms, 1))))
    }
    $barSvg.Add(('<text x="{0}" y="442" text-anchor="middle" font-size="11">{1}</text>' -f (F ($baseX + 80)), $group))
}
$legendX = 190
foreach ($variant in $variants) {
    $barSvg.Add(('<rect x="{0}" y="466" width="12" height="12" fill="{1}"/>' -f $legendX, $variant.Color))
    $barSvg.Add(('<text x="{0}" y="477" font-size="10">{1}</text>' -f ($legendX + 16), $variant.Label))
    $legendX += 145
}
$barSvg.Add('<text x="18" y="240" text-anchor="middle" font-size="12" transform="rotate(-90 18 240)">median preparation (ms)</text>')
$barSvg.Add('</svg>')
[IO.File]::WriteAllLines((Join-Path $OutputDir 'codex-p34-like-for-like-components.svg'), $barSvg)

Write-Host "wrote corrected audit figures to $OutputDir"
