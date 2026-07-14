[CmdletBinding()]
param(
    [string]$DvdPath,
    [string]$Preset = "windows-clang-debug",
    [ValidateSet("first-exit", "intro-cutscene")]
    [string]$Goal = "intro-cutscene",
    [ValidateRange(1, 1000)]
    [int]$Runs = 3,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

function ConvertTo-AbsolutePath([string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) { return $null }
    $clean = $Value.Trim().Trim('"').Trim("'")
    if (-not [System.IO.Path]::IsPathRooted($clean)) { $clean = Join-Path $repoRoot $clean }
    return [System.IO.Path]::GetFullPath($clean)
}

function Get-ConfiguredDvdPath {
    $configPath = Join-Path $env:APPDATA "TwilitRealm\Dusklight\config.json"
    if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) { return $null }
    try {
        return ConvertTo-AbsolutePath (
            (Get-Content -Raw -LiteralPath $configPath | ConvertFrom-Json).'backend.isoPath')
    } catch { return $null }
}

function Quote-ProcessArgument([string]$Value) {
    if ($Value.Contains('"')) { throw "Arguments containing a double quote are unsupported: $Value" }
    return '"' + $Value + '"'
}

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot "prepare-visual-tas.ps1") -Preset $Preset
}

$resolvedDvd = ConvertTo-AbsolutePath $DvdPath
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    $resolvedDvd = Get-ConfiguredDvdPath
}
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    throw "Intro Route requires a valid GCN USA DVD image or prior Dusklight selection."
}

$game = Join-Path $repoRoot "build\$Preset\dusklight.exe"
$tapeName = if ($Goal -eq "first-exit") { "intro_first_exit.tape" } else { "intro_route.tape" }
$scenarioName = if ($Goal -eq "first-exit") { "intro-first-exit" } else { "intro-cutscene" }
$tape = Join-Path $repoRoot "build\$tapeName"
$manifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"
if (-not (Test-Path -LiteralPath $game -PathType Leaf)) { throw "Missing game executable: $game" }
if (-not (Test-Path -LiteralPath $tape -PathType Leaf)) { throw "Missing compiled tape: $tape" }

$inspect = & cargo run --quiet --manifest-path $manifest -- tape inspect $tape
if ($LASTEXITCODE -ne 0) { throw "Could not inspect the Intro Route tape." }
$tapeSummary = ($inspect -join [Environment]::NewLine) | ConvertFrom-Json
if ([int]$tapeSummary.wait_frame_count -ne 0) {
    throw "Intro Route is not a TAS: found $($tapeSummary.wait_frame_count) reactive frame(s)."
}

$stateBase = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "build\automation-state\$scenarioName"))
$stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
$artifactRoot = Join-Path $repoRoot "build\test-results\$scenarioName\$stamp"
New-Item -ItemType Directory -Force $stateBase, $artifactRoot | Out-Null
$triggerTicks = @()
$failures = @()

for ($run = 1; $run -le $Runs; $run++) {
    $runName = "run-{0:D3}" -f $run
    $failure = $null
    $state = Join-Path $stateBase ([Guid]::NewGuid().ToString("N"))
    $tracePath = Join-Path $artifactRoot "$runName.gameplay.trace"
    $summaryPath = Join-Path $artifactRoot "$runName.summary.json"
    New-Item -ItemType Directory -Force $state | Out-Null
    try {
        $arguments = @(
            "--dvd", $resolvedDvd,
            "--input-tape", $tape,
            "--input-tape-end", "hold",
            "--automation-data-root", $state,
            "--gameplay-trace", $tracePath,
            "--cvar", "game.instantSaves=true",
            "--cvar", "backend.cardFileType=1",
            "--cvar", "backend.wasPresetChosen=true",
            "--cvar", "game.enableMenuPointer=false",
            "--headless", "--exit-after-tape"
        )
        Write-Host "$scenarioName $runName/$Runs" -ForegroundColor Cyan
        $argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
        $process = Start-Process -FilePath $game -ArgumentList $argumentLine `
            -WorkingDirectory $repoRoot -Wait -PassThru
        if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $tracePath -PathType Leaf)) {
            throw "Intro Route process failed: exit=$($process.ExitCode), trace=$tracePath"
        }

        $summaryText = & cargo run --quiet --manifest-path $manifest -- trace inspect $tracePath
        if ($LASTEXITCODE -ne 0) { throw "Could not inspect $tracePath" }
        $summaryText = $summaryText -join [Environment]::NewLine
        [System.IO.File]::WriteAllText($summaryPath, $summaryText, [System.Text.UTF8Encoding]::new($false))
        $summary = $summaryText | ConvertFrom-Json
        $missedFirstExit = $null -eq $summary.route_control -or
            $null -eq $summary.first_loading_trigger -or
            $summary.first_loading_transition.location.stage_name -ne "F_SP104" -or
            [int]$summary.first_loading_transition.location.room -ne -1 -or
            [int]$summary.first_loading_transition.location.point -ne 0
        $missedIntro = $Goal -eq "intro-cutscene" -and (
            [int]$summary.post_load_playable.location.room -ne 1 -or
            $null -eq $summary.intro_cutscene -or
            [int]$summary.intro_cutscene.location.point -ne 26 -or
            [int]$summary.intro_cutscene.event_name_hash -ne 783959030)
        if ($missedFirstExit -or $missedIntro) {
            throw "$scenarioName missed a semantic milestone; see $summaryPath"
        }
        $triggerTicks += [uint64]$summary.first_loading_trigger.simulation_tick
        Write-Host "  PASS control=$($summary.route_control.simulation_tick) first-exit=$($summary.first_loading_trigger.simulation_tick) load=$($summary.first_loading_transition.simulation_tick) intro=$($summary.intro_cutscene.simulation_tick)" -ForegroundColor Green
    } catch {
        $failure = "$runName`: $($_.Exception.Message)"
        $failures += $failure
        Write-Host "  FAIL $failure" -ForegroundColor Red
    } finally {
        $resolvedState = [System.IO.Path]::GetFullPath($state)
        $basePrefix = $stateBase.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
        if (-not ($resolvedState + [System.IO.Path]::DirectorySeparatorChar).StartsWith(
                $basePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove Intro Route state outside $stateBase"
        }
        if (Test-Path -LiteralPath $resolvedState) {
            if ($null -ne $failure) {
                Copy-Item -LiteralPath $resolvedState `
                    -Destination (Join-Path $artifactRoot "$runName.failed-state") `
                    -Recurse -Force
            }
            Remove-Item -LiteralPath $resolvedState -Recurse -Force
        }
    }
}

$sortedTriggerTicks = @($triggerTicks | Sort-Object)
$minimumTriggerTick = if ($sortedTriggerTicks.Count -gt 0) { $sortedTriggerTicks[0] } else { $null }
$maximumTriggerTick = if ($sortedTriggerTicks.Count -gt 0) { $sortedTriggerTicks[-1] } else { $null }
$medianTriggerTick = if ($sortedTriggerTicks.Count -gt 0) {
    $sortedTriggerTicks[[math]::Floor(($sortedTriggerTicks.Count - 1) / 2)]
} else { $null }
$triggerSpread = if ($sortedTriggerTicks.Count -gt 0) {
    $maximumTriggerTick - $minimumTriggerTick
} else { $null }

$matrixSummary = [ordered]@{
    schema_version = 1
    scenario = $scenarioName
    requested_runs = $Runs
    passed_runs = $triggerTicks.Count
    failed_runs = $failures.Count
    first_exit_tick_min = $minimumTriggerTick
    first_exit_tick_median = $medianTriggerTick
    first_exit_tick_max = $maximumTriggerTick
    first_exit_tick_spread = $triggerSpread
    failures = $failures
}
$matrixSummaryPath = Join-Path $artifactRoot "matrix.summary.json"
[System.IO.File]::WriteAllText(
    $matrixSummaryPath,
    ($matrixSummary | ConvertTo-Json -Depth 4),
    [System.Text.UTF8Encoding]::new($false))

if ($failures.Count -gt 0) {
    throw "$scenarioName failed $($failures.Count)/$Runs run(s); see $matrixSummaryPath"
}

Write-Host "`nPASS: $scenarioName ($Runs run$(if ($Runs -eq 1) { '' } else { 's' }))" -ForegroundColor Green
Write-Host "First-exit ticks: min=$minimumTriggerTick median=$medianTriggerTick max=$maximumTriggerTick spread=$triggerSpread" -ForegroundColor Green
Write-Host "Artifacts: $artifactRoot" -ForegroundColor DarkGray
