<#
.SYNOPSIS
  Memory-budget benchmark (ticket 05, ADR 0002) for Windows.

.DESCRIPTION
  Builds the release app, runs it against the fake ACP agent in benchmark mode (the app drives its
  own scenario: no keystrokes or clicks), and measures the editor's own processes:
  agent-editor.exe plus its WebView2 processes. The fake agent, Node and claude are excluded.

  Windows has no PSS, so memory is each process's private working set (pages only it uses), the
  closest Windows equivalent. Budget (ADR 0002): <= 250 MB with 5 Tabs, <= 15 MB per extra Tab,
  near 0% idle CPU (here: <= 1% of one core).

.EXAMPLE
  pnpm bench:memory                                              # build, run, report; exit 1 on a miss
  powershell -ExecutionPolicy Bypass -File scripts/bench-memory.ps1 -SkipBuild   # reuse the last build
#>
param(
    [switch]$SkipBuild,
    [int]$Messages = 2000
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$budget = @{ TotalMB = 250; PerTabMB = 15; IdleCpuPercent = 1.0 }

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

# A scratch repository, a fake-agent script (a short turn, then a long transcript), a marker file.
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("agent-editor-bench-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$repo = Join-Path $work 'repo'
New-Item -ItemType Directory -Force $repo | Out-Null
git -C $repo init --quiet
git -C $repo -c user.name=Bench -c user.email=bench@example.com commit --quiet --allow-empty -m init
$paragraph = 'The Agent explains a change in a sentence or two, the way real replies read, so the transcript has realistic weight. '
$long = 1..$Messages | ForEach-Object { "Message $_. $paragraph" }
$script = @{ turns = @(@{ chunks = @('Short reply.') }, @{ messages = $long }) } | ConvertTo-Json -Depth 4 -Compress
$scriptPath = Join-Path $work 'script.json'
[System.IO.File]::WriteAllText($scriptPath, $script, (New-Object System.Text.UTF8Encoding $false))
$markers = Join-Path $work 'markers.txt'

function Get-Tree([int]$rootId) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name
    $excluded = 'node.exe', 'fake-acp-agent.exe', 'claude.exe'
    $ids = [System.Collections.Generic.List[int]]::new(); $ids.Add($rootId)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($p in $all | Where-Object { $_.ParentProcessId -eq $ids[$i] -and $excluded -notcontains $_.Name }) { $ids.Add([int]$p.ProcessId) }
    }
    $ids
}

function Measure-Editor([int]$rootId) {
    $ids = Get-Tree $rootId
    $perf = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Property IDProcess, Name, WorkingSetPrivate, WorkingSet |
        Where-Object { $ids -contains [int]$_.IDProcess }
    [pscustomobject]@{
        PrivateMB   = [math]::Round(($perf | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB, 1)
        WorkingSetMB = [math]::Round(($perf | Measure-Object WorkingSet -Sum).Sum / 1MB, 1)
        Processes   = ($perf | ForEach-Object { "$($_.Name)=$([math]::Round($_.WorkingSetPrivate / 1MB, 1))" }) -join ', '
        Ids         = $ids
    }
}

function Wait-Marker([string]$name, [int]$seconds = 180) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path $markers) -and ((Get-Content $markers) -contains $name)) { return }
        if ($script:app.HasExited) { throw "the editor exited before '$name'" }
        Start-Sleep -Milliseconds 250
    }
    throw "timed out waiting for '$name'"
}

$env:AGENT_EDITOR_BENCH = $markers
$env:AGENT_EDITOR_WORKSPACE = $repo
$env:AGENT_EDITOR_ACP_ADAPTER = $fake
$env:FAKE_ACP_SCRIPT = $scriptPath
$script:app = Start-Process -FilePath $exe -PassThru
try {
    Wait-Marker 'one-tab'
    Start-Sleep -Seconds 2
    $one = Measure-Editor $app.Id

    Wait-Marker 'five-tabs'
    Start-Sleep -Seconds 2
    $five = Measure-Editor $app.Id
    $cores = [Environment]::ProcessorCount
    $cpuStart =(Get-Process -Id $five.Ids -ErrorAction SilentlyContinue | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum
    $window = 10
    Start-Sleep -Seconds $window
    $cpuEnd = (Get-Process -Id $five.Ids -ErrorAction SilentlyContinue | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum
    $idleCpu = [math]::Round(($cpuEnd - $cpuStart) / ($window * 1000) * 100, 2)
} finally {
    if (-not $app.HasExited) {
        Get-Tree $app.Id | Sort-Object -Descending | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
        Get-CimInstance Win32_Process -Filter "Name = 'fake-acp-agent.exe'" | Where-Object { $_.ExecutablePath -eq $fake } |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
    Remove-Item Env:AGENT_EDITOR_BENCH, Env:AGENT_EDITOR_WORKSPACE, Env:AGENT_EDITOR_ACP_ADAPTER, Env:FAKE_ACP_SCRIPT -ErrorAction SilentlyContinue
}

$perTab = [math]::Round(($five.PrivateMB - $one.PrivateMB) / 4, 1)
$results = @(
    [pscustomobject]@{ Check = 'Total, 5 Tabs (private MB)'; Value = $five.PrivateMB; Budget = "<= $($budget.TotalMB)"; Pass = $five.PrivateMB -le $budget.TotalMB }
    [pscustomobject]@{ Check = 'Per extra Tab (private MB)'; Value = $perTab; Budget = "<= $($budget.PerTabMB)"; Pass = $perTab -le $budget.PerTabMB }
    [pscustomobject]@{ Check = 'Idle CPU (% of one core)'; Value = $idleCpu; Budget = "<= $($budget.IdleCpuPercent)"; Pass = $idleCpu -le $budget.IdleCpuPercent }
)
""
"Agent Editor memory benchmark (Windows, $Messages-message transcript, $cores logical cores)"
"  1 Tab : $($one.PrivateMB) MB private ($($one.WorkingSetMB) MB working set)  [$($one.Processes)]"
"  5 Tabs: $($five.PrivateMB) MB private ($($five.WorkingSetMB) MB working set)  [$($five.Processes)]"
$results | Format-Table -AutoSize | Out-String
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
if ($results.Pass -contains $false) { "MISS against ADR 0002's budget."; exit 1 }
"Within ADR 0002's budget."
