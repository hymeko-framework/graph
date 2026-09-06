# P3.4 §6 — isolated-subprocess CPU/RSS harness. Spawns the compiled p34_cell test binary once per
# (workload, operation, thread-count), measuring the CHILD PROCESS's TotalProcessorTime and
# PeakWorkingSet64 (process CPU, not host-wide %). Only `batch` is Rayon-parallel; serial ops are run at
# one thread (their wall is thread-invariant by construction). Writes a raw TSV.
#
# Usage: powershell -NoProfile -File scripts/p34_cpu_rss.ps1 [-Exe <path>] [-OutTsv <path>]
param(
    [string]$Exe,
    [string]$OutTsv = "reports/data/2026-09-03-p34-cpu-scaling.tsv"
)
$ErrorActionPreference = 'Stop'
$target = $env:CARGO_TARGET_DIR
if (-not $target) { $target = "D:\h\graph-matroid-p34\target" }
if (-not $Exe) {
    $Exe = (Get-ChildItem "$target\release\deps\p34_cell-*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
}
$hostMax = [Environment]::ProcessorCount
Write-Host "cell exe: $Exe   host cores: $hostMax"

function Invoke-Cell($workload, $op, $threads) {
    $rn = if ($threads -eq 0) { $hostMax } else { $threads }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = 'run_cell --exact --ignored --nocapture'
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['P34_WORKLOAD'] = $workload
    $psi.EnvironmentVariables['P34_OP'] = $op
    $psi.EnvironmentVariables['RAYON_NUM_THREADS'] = "$rn"
    $p = [System.Diagnostics.Process]::Start($psi)
    $out = $p.StandardOutput.ReadToEnd()
    $null = $p.StandardError.ReadToEnd()
    $p.WaitForExit()
    try { $cpu = $p.TotalProcessorTime.TotalSeconds } catch { $cpu = [double]::NaN }
    try { $rss = $p.PeakWorkingSet64 } catch { $rss = 0 }
    $wall = ($p.ExitTime - $p.StartTime).TotalSeconds
    $line = ($out -split "`r?`n" | Where-Object { $_ -match 'P34CELL' } | Select-Object -First 1)
    if (-not $line) { Write-Host "  !! no P34CELL line for $workload/$op/$rn"; return $null }
    $kv = @{}
    foreach ($tok in ($line -split "`t")) { if ($tok -match '=') { $k, $v = $tok -split '=', 2; $kv[$k] = $v } }
    [pscustomobject]@{
        workload = $workload; op = $op; threads = $rn
        nodes = [int]$kv['nodes']; arcs = [int]$kv['arcs']; inc = [int]$kv['inc']; iters = [int]$kv['iters']
        op_wall_s = [double]$kv['op_wall_s']; checksum = $kv['checksum']
        proc_cpu_s = $cpu; peak_rss_b = [double]$rss; proc_wall_s = $wall
    }
}

$rows = New-Object System.Collections.ArrayList

# (1) the parallel op 'batch' across all thread counts, on the heaviest workloads (the scaling table).
$scaleWorkloads = @('sts1023', 'random1e6', 'map512')
$threadsList = @(1, 2, 4, 8, 16, 0)
foreach ($w in $scaleWorkloads) {
    foreach ($t in $threadsList) {
        Write-Host "batch  $w  threads=$t"
        $r = Invoke-Cell $w 'batch' $t
        if ($r) { [void]$rows.Add($r) }
    }
}
# batch at host-max on the remaining workloads (single scaling point).
foreach ($w in @('sts511', 'mycielski', 'transport')) {
    $r = Invoke-Cell $w 'batch' 0
    if ($r) { [void]$rows.Add($r) }
}

# (2) the serial ops at one thread on every workload (thread-invariant by construction).
$serialOps = @('reachable', 'components', 'dijkstra', 'rank2', 'levi')
foreach ($w in @('sts511', 'sts1023', 'mycielski', 'random1e6', 'map512', 'transport')) {
    foreach ($op in $serialOps) {
        Write-Host "$op  $w  threads=1"
        $r = Invoke-Cell $w $op 1
        if ($r) { [void]$rows.Add($r) }
    }
}

# derived columns. Speedup for the parallel op is relative to its own workload's 1-thread wall.
$base = @{}
foreach ($r in $rows) { if ($r.op -eq 'batch' -and $r.threads -eq 1) { $base[$r.workload] = $r.op_wall_s } }

$header = "workload`top`tthreads`tnodes`tarcs`tinc`titers`top_wall_s`tproc_cpu_s`tproc_wall_s`tbusy_cores`tnorm_util`tspeedup`tefficiency`tinc_per_s`tpeak_rss_mb`tchecksum"
$lines = New-Object System.Collections.ArrayList
[void]$lines.Add($header)
foreach ($r in $rows) {
    $busy = if ($r.proc_wall_s -gt 0) { $r.proc_cpu_s / $r.proc_wall_s } else { 0 }
    $nu = if ($r.threads -gt 0) { $busy / $r.threads } else { 0 }
    $speed = if ($r.op -eq 'batch' -and $base.ContainsKey($r.workload) -and $r.op_wall_s -gt 0) { $base[$r.workload] / $r.op_wall_s } else { 1.0 }
    $eff = if ($r.threads -gt 0) { $speed / $r.threads } else { 0 }
    $workInc = if ($r.op -eq 'batch') { $r.inc * 64.0 } else { [double]$r.inc }
    $incPerS = if ($r.op_wall_s -gt 0) { $workInc / $r.op_wall_s } else { 0 }
    $rssMb = $r.peak_rss_b / 1MB
    $l = "{0}`t{1}`t{2}`t{3}`t{4}`t{5}`t{6}`t{7:F6}`t{8:F4}`t{9:F4}`t{10:F3}`t{11:F3}`t{12:F3}`t{13:F3}`t{14:F0}`t{15:F1}`t{16}" -f `
        $r.workload, $r.op, $r.threads, $r.nodes, $r.arcs, $r.inc, $r.iters, $r.op_wall_s, $r.proc_cpu_s, $r.proc_wall_s, $busy, $nu, $speed, $eff, $incPerS, $rssMb, $r.checksum
    [void]$lines.Add($l)
}
$outPath = Join-Path (Get-Location) $OutTsv
$lines | Out-File -FilePath $outPath -Encoding utf8
Write-Host "wrote $outPath  ($($rows.Count) cells)"
