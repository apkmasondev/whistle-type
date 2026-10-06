<#
.SYNOPSIS
  First-run test with the production build: empty data folder → setup window → Download (via UI Automation)
  → Cancel mid-way → Download again (must resume) → SHA-256 verified → model loaded. Needs internet.
  Run with Windows PowerShell 5.1.
#>
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$exe = Join-Path $root 'target\release\WhistleType.exe'
$work = Join-Path $env:TEMP ("wt-firstrun-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Force $data | Out-Null
Set-Content (Join-Path $data 'settings.json') '{"ui_language":"en"}' -Encoding ASCII   # assertions use the English captions
$log = Join-Path $data 'logs\whistletype.log'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$results = @()
function Record($n, $ok, $d) { $script:results += [pscustomobject]@{ test = $n; ok = [bool]$ok; detail = $d }; Write-Host ("[{0}] {1}: {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $n, $d) }
function Log-Text() { if (Test-Path $log) { Get-Content $log -Raw -Encoding UTF8 } else { '' } }
function Wait-For([scriptblock]$cond, [int]$ms) { $sw = [Diagnostics.Stopwatch]::StartNew(); while ($sw.ElapsedMilliseconds -lt $ms) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 200 }; return $false }

function Find-Setup() {
    $rootEl = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.AndCondition(
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'WhistleType.Setup')),
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$script:p.Id)))
    return $rootEl.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
}
function Click($win, [string]$name) {
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $name)
    $b = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if (-not $b) { throw "button '$name' not found" }
    $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

$env:WHISTLETYPE_DATA_DIR = $data
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process $exe -PassThru
try {
    $ok = Wait-For { $null -ne (Find-Setup) } 10000
    Record 'setup window opens on first run (no model)' $ok "after $($sw.ElapsedMilliseconds) ms"
    $win = Find-Setup
    Record 'nothing downloaded before the user clicks' (-not (Test-Path (Join-Path $data 'models'))) ''

    Click $win 'Download'
    $part = Join-Path $data 'models\whistle-2.0.0\whistle.cact.part'
    $got = Wait-For { (Test-Path $part) -and (Get-Item $part).Length -gt 3MB } 120000
    Click $win 'Cancel'
    Start-Sleep 2
    $partial = if (Test-Path $part) { (Get-Item $part).Length } else { 0 }
    Record 'cancel keeps the partial file' ($got -and $partial -gt 0 -and (Log-Text) -match 'Download cancelled|cancelled') "$partial bytes"

    $win = Find-Setup
    $t = [Diagnostics.Stopwatch]::StartNew()
    Click $win 'Try again'
    $ready = Wait-For { (Log-Text) -match 'model ready' } 300000
    $resumed = (Log-Text) -match 'resuming at (\d+) bytes'
    Record 'download resumes after cancel' $resumed $Matches[0]
    Record 'model verified (SHA-256) and loaded' $ready "$($t.ElapsedMilliseconds) ms"
    $model = Join-Path $data 'models\whistle-2.0.0\whistle.cact'
    $hash = if (Test-Path $model) { (Get-FileHash $model -Algorithm SHA256).Hash.ToLower() } else { '' }
    Record 'file on disk matches the pinned hash' ($hash -eq 'b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb') $hash
    Record 'no .part file left' (-not (Test-Path $part)) ''

    # restart: must start offline-ready, without the setup window
    Stop-Process -Id $p.Id -Force; Start-Sleep 1
    $p = Start-Process $exe -ArgumentList '--background' -PassThru
    $from = (Get-Content $log).Count
    $ok = Wait-For { $l = @(Get-Content $log); ($l.Count -gt $from) -and (($l[$from..($l.Count - 1)] -join "`n") -match 'model ready') } 15000
    Start-Sleep 1
    Record 'second start loads the model without network/setup' ($ok -and $null -eq (Find-Setup)) ''

    # corrupted model: must be detected, never crash
    Stop-Process -Id $p.Id -Force; Start-Sleep 1
    $bytes = [IO.File]::ReadAllBytes($model); $bytes[5000000] = $bytes[5000000] -bxor 0xFF; [IO.File]::WriteAllBytes($model, $bytes)
    $p = Start-Process $exe -PassThru
    $ok = Wait-For { (Log-Text) -match 'SHA-256 mismatch' } 15000
    Start-Sleep 1
    Record 'corrupted model is detected (no crash, setup offered)' ($ok -and -not $p.HasExited -and $null -ne (Find-Setup)) ''
}
finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Copy-Item $log (Join-Path $root 'tests\results\first-run-app.log') -ErrorAction SilentlyContinue
    $results | ConvertTo-Json | Set-Content (Join-Path $root 'tests\results\first-run.json') -Encoding UTF8
    Write-Host ("{0} / {1} passed" -f @($results | Where-Object ok).Count, $results.Count)
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
