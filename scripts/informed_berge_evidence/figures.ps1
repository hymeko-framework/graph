param([string]$RunDate = (Get-Date -Format 'yyyy-MM-dd'))

$ErrorActionPreference = 'Stop'
$invariant = [Globalization.CultureInfo]::InvariantCulture
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$evidenceRoot = Join-Path $repoRoot "reports\informed-berge-evidence\$RunDate"
$dataDir = Join-Path $evidenceRoot 'data'
$figureDir = Join-Path $evidenceRoot 'figures'
[void](New-Item -ItemType Directory -Force -Path $figureDir)
$cells = @(Import-Csv (Join-Path $dataDir 'cells.tsv') -Delimiter "`t")
$work = @(Import-Csv (Join-Path $dataDir 'search-work.tsv') -Delimiter "`t")
$results = @(Import-Csv (Join-Path $dataDir 'results.tsv') -Delimiter "`t")
$utf8 = [Text.UTF8Encoding]::new($false)

function F([double]$value, [string]$pattern = '0.0') {
    $value.ToString($pattern, $invariant)
}

function Write-Tsv([string]$name, [object[]]$rows, [string[]]$columns) {
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add(($columns -join "`t"))
    foreach ($row in $rows) {
        $lines.Add((($columns | ForEach-Object { $row.$_ }) -join "`t"))
    }
    [IO.File]::WriteAllLines((Join-Path $dataDir $name), $lines, $utf8)
}

function Write-Svg([string]$name, [Collections.Generic.List[string]]$lines) {
    $lines.Add('</svg>')
    [IO.File]::WriteAllLines((Join-Path $figureDir $name), $lines, $utf8)
}

$searchRows = [Collections.Generic.List[object]]::new()
foreach ($group in @($cells | Where-Object { $_.section -eq 'informed_search' } | Group-Object workload)) {
    $baseline = $group.Group | Where-Object { $_.subject -like 'dijkstra_*' } | Select-Object -First 1
    $baselineWork = $work | Where-Object {
        $_.workload -eq $group.Name -and $_.subject -eq $baseline.subject
    } | Select-Object -First 1
    $prep = $cells | Where-Object {
        $_.section -eq 'heuristic_preparation' -and $_.workload -eq $group.Name
    } | Select-Object -First 1
    foreach ($cell in $group.Group) {
        $cellWork = $work | Where-Object {
            $_.workload -eq $group.Name -and $_.subject -eq $cell.subject
        } | Select-Object -First 1
        $usesDiscrete = $cell.subject -in @('astar_discrete','astar_combined')
        $prepUs = if ($usesDiscrete -and $prep) { [double]$prep.wall_median_us } else { 0.0 }
        $queryUs = [double]$cell.wall_median_us
        $searchRows.Add([pscustomobject][ordered]@{
            workload = $group.Name; subject = $cell.subject
            query_us = F $queryUs '0.000'; prep_us = F $prepUs '0.000'
            first_query_total_us = F ($queryUs + $prepUs) '0.000'
            query_speedup = F ([double]$baseline.wall_median_us / $queryUs) '0.000000'
            expanded_nodes = $cellWork.expanded_nodes
            expanded_node_reduction = F ([double]$baselineWork.expanded_nodes / [double]$cellWork.expanded_nodes) '0.000000'
            resolved_incidences = $cellWork.resolved_incidences
            stale_pops = $cellWork.stale_pops; reopened = $cellWork.reopened
        })
    }
}
Write-Tsv 'search-comparison.tsv' $searchRows @(
    'workload','subject','query_us','prep_us','first_query_total_us','query_speedup',
    'expanded_nodes','expanded_node_reduction','resolved_incidences','stale_pops','reopened'
)

$shortestRows = [Collections.Generic.List[object]]::new()
foreach ($group in @($cells | Where-Object { $_.section -eq 'berge_shortest' } | Group-Object workload)) {
    $bfs = $group.Group | Where-Object { $_.subject -eq 'bfs_shortest' } | Select-Object -First 1
    $dijkstra = $group.Group | Where-Object { $_.subject -eq 'dijkstra_unit' } | Select-Object -First 1
    $result = $results | Where-Object {
        $_.section -eq 'berge_shortest' -and $_.workload -eq $group.Name
    } | Select-Object -First 1
    $shortestRows.Add([pscustomobject][ordered]@{
        workload=$group.Name; hops=$result.hops
        bfs_us=$bfs.wall_median_us; dijkstra_us=$dijkstra.wall_median_us
        bfs_speedup=F ([double]$dijkstra.wall_median_us / [double]$bfs.wall_median_us) '0.000000'
    })
}
Write-Tsv 'shortest-comparison.tsv' $shortestRows @('workload','hops','bfs_us','dijkstra_us','bfs_speedup')

$enumerationRows = [Collections.Generic.List[object]]::new()
foreach ($cell in @($cells | Where-Object { $_.section -eq 'berge_enumeration' })) {
    $result = $results | Where-Object { $_.process_run -eq $cell.process_run } | Select-Object -First 1
    $enumerationRows.Add([pscustomobject][ordered]@{
        workload=$cell.workload; subject=$cell.subject; median_us=$cell.wall_median_us
        q1_us=$cell.wall_q1_us; q3_us=$cell.wall_q3_us; worst_us=$cell.wall_worst_us
        items=$result.items; bound=$result.bound; max_depth=$result.max_depth
        max_results=$result.max_results; max_expansions=$result.max_expansions
        us_per_item=if ([uint64]$result.items -gt 0) { F ([double]$cell.wall_median_us / [uint64]$result.items) '0.000000' } else { 'NA' }
    })
}
Write-Tsv 'enumeration-summary.tsv' $enumerationRows @(
    'workload','subject','median_us','q1_us','q3_us','worst_us','items','bound',
    'max_depth','max_results','max_expansions','us_per_item'
)

# Grid A*: latency and expanded-node speedup relative to Dijkstra.
$grid = @($searchRows | Where-Object { $_.workload -like 'grid200_*' -and $_.subject -ne 'dijkstra_unit' })
$svg = [Collections.Generic.List[string]]::new()
$svg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="540" font-family="sans-serif">')
$svg.Add('<rect width="100%" height="100%" fill="white"/>')
$svg.Add('<text x="450" y="28" text-anchor="middle" font-size="18" font-weight="bold">Grid A* relative to Dijkstra</text>')
$svg.Add('<text x="450" y="48" text-anchor="middle" font-size="11">query latency speedup and expanded-node reduction; higher is better</text>')
$left=80.0; $right=850.0; $top=70.0; $bottom=460.0
$maxValue=[Math]::Max(2.0,($grid | ForEach-Object {[Math]::Max([double]$_.query_speedup,[double]$_.expanded_node_reduction)} | Measure-Object -Maximum).Maximum*1.1)
$yOf={param([double]$value)$bottom-$value/$maxValue*($bottom-$top)}
$distances=@(20,40,80); $xOf=@{20=180.0;40=450.0;80=720.0}
for($tick=0;$tick-le$maxValue;$tick+=[Math]::Max(20,[Math]::Ceiling($maxValue/6/10)*10)){$y=&$yOf $tick;$svg.Add(('<line x1="80" y1="{0}" x2="850" y2="{0}" stroke="#eee"/>'-f(F $y)));$svg.Add(('<text x="72" y="{0}" text-anchor="end" font-size="10">{1}x</text>'-f(F($y+4)),(F $tick '0')))}
$styles=@{astar_l1=@('#287a52','#7bc49d');astar_l2=@('#356cb6','#8ab2e5')}
$legendY=75
foreach($subject in @('astar_l1','astar_l2')){
    $lat=[Collections.Generic.List[string]]::new();$exp=[Collections.Generic.List[string]]::new()
    foreach($distance in $distances){$row=$grid|Where-Object{$_.workload-eq"grid200_d$distance"-and$_.subject-eq$subject}|Select-Object -First 1;$x=$xOf[$distance];$yl=&$yOf([double]$row.query_speedup);$ye=&$yOf([double]$row.expanded_node_reduction);$lat.Add("$(F $x),$(F $yl)");$exp.Add("$(F $x),$(F $ye)");$svg.Add(('<circle cx="{0}" cy="{1}" r="4" fill="{2}"/>'-f(F $x),(F $yl),$styles[$subject][0]));$svg.Add(('<rect x="{0}" y="{1}" width="7" height="7" fill="{2}"/>'-f(F($x-3.5)),(F($ye-3.5)),$styles[$subject][1]))}
    $svg.Add(('<polyline points="{0}" fill="none" stroke="{1}" stroke-width="2.5"/>'-f($lat-join' '),$styles[$subject][0]));$svg.Add(('<polyline points="{0}" fill="none" stroke="{1}" stroke-width="2" stroke-dasharray="5 3"/>'-f($exp-join' '),$styles[$subject][1]));$svg.Add(('<text x="650" y="{0}" font-size="10">{1}: circle=latency, square=expanded nodes</text>'-f$legendY,$subject));$legendY+=18
}
foreach($distance in $distances){$svg.Add(('<text x="{0}" y="482" text-anchor="middle" font-size="11">d={1}</text>'-f(F $xOf[$distance]),$distance))}
$svg.Add('<line x1="80" y1="460" x2="850" y2="460" stroke="#333"/>');$svg.Add('<line x1="80" y1="70" x2="80" y2="460" stroke="#333"/>')
Write-Svg 'grid-astar.svg' $svg

# Weighted-map query speedup. Values below one make adverse cases explicit.
$mapRows=@($searchRows|Where-Object{$_.workload-notlike'grid200_*'-and$_.subject-like'astar_*'})
$labels=@($mapRows|ForEach-Object{"$($_.workload)/$($_.subject-replace'astar_','')"})
$svg=[Collections.Generic.List[string]]::new();$height=120+32*$mapRows.Count
$svg.Add(('<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="{0}" font-family="sans-serif">' -f $height));$svg.Add('<rect width="100%" height="100%" fill="white"/>');$svg.Add('<text x="500" y="28" text-anchor="middle" font-size="18" font-weight="bold">Weighted-map A* query speedup</text>');$svg.Add('<text x="500" y="48" text-anchor="middle" font-size="11">relative to same-workload Dijkstra; red line is parity</text>')
$left=260.0;$right=940.0;$max=([Math]::Max(1.1,($mapRows|ForEach-Object{[double]$_.query_speedup}|Measure-Object -Maximum).Maximum*1.1));$xOf={param([double]$v)$left+$v/$max*($right-$left)};$parity=&$xOf 1.0;$svg.Add(('<line x1="{0}" y1="60" x2="{0}" y2="{1}" stroke="#b33" stroke-dasharray="5 4"/>'-f(F $parity),($height-35)))
$i=0;foreach($row in $mapRows){$y=78+32*$i;$x=&$xOf([double]$row.query_speedup);$color=if([double]$row.query_speedup-ge1){'#287a52'}else{'#b45f2d'};$svg.Add(('<text x="250" y="{0}" text-anchor="end" font-size="10">{1}/{2}</text>'-f($y+4),$row.workload,($row.subject-replace'astar_','')));$svg.Add(('<line x1="{0}" y1="{1}" x2="{2}" y2="{1}" stroke="{3}" stroke-width="7"/>'-f(F $left),(F $y),(F $x),$color));$svg.Add(('<text x="{0}" y="{1}" font-size="10">{2}x</text>'-f(F($x+6)),($y+4),(F([double]$row.query_speedup)'0.00')));$i++}
Write-Svg 'weighted-map-astar.svg' $svg

# BFS vs unit-cost Dijkstra shortest Berge path.
$svg=[Collections.Generic.List[string]]::new();$svg.Add('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="500" font-family="sans-serif">');$svg.Add('<rect width="100%" height="100%" fill="white"/>');$svg.Add('<text x="450" y="28" text-anchor="middle" font-size="18" font-weight="bold">Berge shortest-path engine comparison</text>');$svg.Add('<text x="450" y="48" text-anchor="middle" font-size="11">Dijkstra median / BFS median; both return the same hop distance</text>');$left=80.0;$right=850.0;$top=70.0;$bottom=420.0;$max=([Math]::Max(1.0,($shortestRows|ForEach-Object{[double]$_.bfs_speedup}|Measure-Object -Maximum).Maximum*1.15));$yOf={param([double]$v)$bottom-$v/$max*($bottom-$top)};$count=$shortestRows.Count;$step=($right-$left)/[Math]::Max(1,$count)
for($i=0;$i-lt$count;$i++){$row=$shortestRows[$i];$x=$left+($i+0.5)*$step;$y=&$yOf([double]$row.bfs_speedup);$svg.Add(('<rect x="{0}" y="{1}" width="{2}" height="{3}" fill="#356cb6"/>'-f(F($x-$step*0.25)),(F $y),(F($step*0.5)),(F($bottom-$y))));$svg.Add(('<text x="{0}" y="442" text-anchor="middle" font-size="10">{1}</text>'-f(F $x),$row.workload));$svg.Add(('<text x="{0}" y="{1}" text-anchor="middle" font-size="10">{2}x</text>'-f(F $x),(F($y-6)),(F([double]$row.bfs_speedup)'0.00')))}
$svg.Add('<line x1="80" y1="420" x2="850" y2="420" stroke="#333"/>');Write-Svg 'berge-shortest.svg' $svg

$artifactLines=[Collections.Generic.List[string]]::new();$artifactLines.Add("path`tsha256")
$artifacts=@(Get-ChildItem -LiteralPath $dataDir -Filter '*.tsv';Get-ChildItem -LiteralPath $figureDir -Filter '*.svg')|Sort-Object FullName
foreach($artifact in $artifacts){$relative=$artifact.FullName.Substring($repoRoot.Length+1).Replace('\','/');$hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $artifact.FullName).Hash.ToLowerInvariant();$artifactLines.Add("$relative`t$hash")}
[IO.File]::WriteAllLines((Join-Path $evidenceRoot 'artifact-sha256.tsv'),$artifactLines,$utf8)
Write-Host "wrote informed Berge figures to $figureDir"
