<#
.SYNOPSIS
  Memory-budget benchmark (ticket 05, ADR 0002) for Windows.

.DESCRIPTION
  Builds the release app, runs it against the fake ACP agent in benchmark mode (the app drives its
  own scenario from src/benchmark.ts: no keystrokes or clicks), and measures the editor's own
  processes: agent-editor.exe and its WebView2 processes. The fake agent, Node and claude (and
  anything under them) are excluded.

  Phases (marker names shared with src/benchmark.ts):
    one-tab         1 Tab after a short turn
    five-tabs       5 Tabs, each after a short turn          -> per-extra-Tab cost
    long-transcript then a long transcript in Tab 1, every Tab visited, idle -> total and idle CPU

  Windows has no PSS, so memory is each process's private working set (pages only it uses), the
  closest Windows equivalent. Budget (ADR 0002): <= 250 MB total, <= 15 MB per extra Tab, near 0%
  idle CPU (here: <= 1% of one core). The Manual editor isn't built yet (ticket 14 adds it).

.EXAMPLE
  pnpm bench:memory                                                              # build, run, report; exit 1 on a miss
  powershell -ExecutionPolicy Bypass -File scripts/bench-memory.ps1 -SkipBuild   # reuse the last build
#>
param(
    [switch]$SkipBuild,
    [int]$Messages = 2000,
    [int]$CpuWindowSeconds = 20
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$budget = @{ TotalMB = 250; PerTabMB = 15; IdleCpuPercent = 1.0 }
$excludedNames = 'node.exe', 'fake-acp-agent.exe', 'claude.exe'

if (-not $SkipBuild) {
    Push-Location $root
    try {
        pnpm tauri build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
        cargo build --release -p editor-core --bin fake-acp-agent
        if ($LASTEXITCODE -ne 0) { throw "building the fake agent failed" }
    } finally { Pop-Location }
}
$exe = Join-Path $root 'target\release\agent-editor.exe'
$fake = Join-Path $root 'target\release\fake-acp-agent.exe'
foreach ($f in $exe, $fake) { if (-not (Test-Path $f)) { throw "missing $f (run without -SkipBuild)" } }

function Get-EditorProcessIds([int]$rootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name
    $ids = [System.Collections.Generic.List[int]]::new(); $ids.Add($rootId)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($p in $all | Where-Object { $_.ParentProcessId -eq $ids[$i] -and $excludedNames -notcontains $_.Name }) {
            $ids.Add([int]$p.ProcessId)
        }
    }
    $ids
}

function Measure-Memory([int]$rootId) {
    $ids = Get-EditorProcessIds $rootId
    $perf = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Property IDProcess, Name, WorkingSetPrivate, WorkingSet |
        Where-Object { $ids -contains [int]$_.IDProcess }
    [pscustomobject]@{
        PrivateMB    = [math]::Round(($perf | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB, 1)
        WorkingSetMB = [math]::Round(($perf | Measure-Object WorkingSet -Sum).Sum / 1MB, 1)
        Processes    = ($perf | ForEach-Object { "$($_.Name)=$([math]::Round($_.WorkingSetPrivate / 1MB, 1))" }) -join ', '
    }
}

function Get-CpuMilliseconds([int[]]$ids) {
    $times = @{}
    foreach ($p in Get-Process -Id $ids -ErrorAction SilentlyContinue) { $times[$p.Id] = $p.TotalProcessorTime.TotalMilliseconds }
    $times
}

# Idle CPU over the window, as % of one core, counting only processes alive at both ends.
function Measure-IdleCpu([int]$rootId, [int]$seconds) {
    $ids = Get-EditorProcessIds $rootId
    $before = Get-CpuMilliseconds $ids
    Start-Sleep -Seconds $seconds
    $after = Get-CpuMilliseconds $ids
    $used = 0.0
    foreach ($id in $before.Keys) { if ($after.ContainsKey($id)) { $used += $after[$id] - $before[$id] } }
    [math]::Round($used / ($seconds * 1000) * 100, 2)
}

# A scratch repository, a fake-agent script (5 short turns, then a long transcript), a marker file.
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("agent-editor-bench-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$repo = Join-Path $work 'repo'
$markers = Join-Path $work 'markers.txt'
$app = $null
try {
    New-Item -ItemType Directory -Force $repo | Out-Null
    git -C $repo init --quiet
    git -C $repo -c user.name=Bench -c user.email=bench@example.com commit --quiet --allow-empty -m init
    $paragraph = 'The Agent explains a change in a sentence or two, the way real replies read, so the transcript has realistic weight. '
    $short = @{ chunks = @('Short reply.') }
    $turns = @($short, $short, $short, $short, $short, @{ messages = @(1..$Messages | ForEach-Object { "Message $_. $paragraph" }) })
    $agentScript = Join-Path $work 'script.json'
    [System.IO.File]::WriteAllText($agentScript, (@{ turns = $turns } | ConvertTo-Json -Depth 4 -Compress), (New-Object System.Text.UTF8Encoding $false))

    $env:AGENT_EDITOR_BENCH = $markers
    $env:AGENT_EDITOR_WORKSPACE = $repo
    $env:AGENT_EDITOR_ACP_ADAPTER = $fake
    $env:FAKE_ACP_SCRIPT = $agentScript
    $app = Start-Process -FilePath $exe -PassThru

    function Wait-Phase([string]$name, [int]$seconds = 240) {
        $deadline = (Get-Date).AddSeconds($seconds)
        while ((Get-Date) -lt $deadline) {
            if ((Test-Path $markers) -and ((Get-Content $markers) -contains $name)) { Start-Sleep -Seconds 2; return }
            if ($app.HasExited) { throw "the editor exited before '$name'" }
            Start-Sleep -Milliseconds 250
        }
        throw "timed out waiting for '$name'"
    }

    Wait-Phase 'one-tab'
    $one = Measure-Memory $app.Id
    Wait-Phase 'five-tabs'
    $five = Measure-Memory $app.Id
    Wait-Phase 'long-transcript'
    $full = Measure-Memory $app.Id
    $idleCpu = Measure-IdleCpu $app.Id $CpuWindowSeconds
} finally {
    if ($app -and -not $app.HasExited) { taskkill /T /F /PID $app.Id 2>&1 | Out-Null }
    # The fake agent can outlive a crashed editor; stop any instance of this build.
    Get-CimInstance Win32_Process -Filter "Name = 'fake-acp-agent.exe'" | Where-Object { $_.ExecutablePath -eq $fake } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Remove-Item Env:AGENT_EDITOR_BENCH, Env:AGENT_EDITOR_WORKSPACE, Env:AGENT_EDITOR_ACP_ADAPTER, Env:FAKE_ACP_SCRIPT -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

$perTab = [math]::Round(($five.PrivateMB - $one.PrivateMB) / 4, 1)
$results = @(
    [pscustomobject]@{ Check = 'Total, 5 Tabs + long transcript (private MB)'; Value = $full.PrivateMB; Budget = "<= $($budget.TotalMB)"; Pass = $full.PrivateMB -le $budget.TotalMB }
    [pscustomobject]@{ Check = 'Per extra Tab (private MB)'; Value = $perTab; Budget = "<= $($budget.PerTabMB)"; Pass = $perTab -le $budget.PerTabMB }
    [pscustomobject]@{ Check = "Idle CPU over $CpuWindowSeconds s (% of one core)"; Value = $idleCpu; Budget = "<= $($budget.IdleCpuPercent)"; Pass = $idleCpu -le $budget.IdleCpuPercent }
)
""
"Agent Editor memory benchmark (Windows, $Messages-message transcript, $([Environment]::ProcessorCount) logical cores; no Manual editor yet)"
"  1 Tab           : $($one.PrivateMB) MB private ($($one.WorkingSetMB) MB working set)  [$($one.Processes)]"
"  5 Tabs          : $($five.PrivateMB) MB private ($($five.WorkingSetMB) MB working set)  [$($five.Processes)]"
"  + long transcript: $($full.PrivateMB) MB private ($($full.WorkingSetMB) MB working set)  [$($full.Processes)]"
$results | Format-Table -AutoSize | Out-String
if ($results.Pass -contains $false) { "MISS against ADR 0002's budget."; exit 1 }
"Within ADR 0002's budget."
