[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$CandidateTape,

    [string]$CandidateId,

    [string]$DvdPath,

    [string]$Preset = "windows-clang-debug",

    [string]$Stage = "F_SP103,1,1,3",

    [string]$ArtifactRoot,

    [switch]$KeepState,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$manifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"

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

function Write-Utf8Json([string]$Path, $Value) {
    [System.IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth 12),
        [System.Text.UTF8Encoding]::new($false))
}

$resolvedTape = ConvertTo-AbsolutePath $CandidateTape
if ($null -eq $resolvedTape -or -not (Test-Path -LiteralPath $resolvedTape -PathType Leaf)) {
    throw "Candidate tape does not exist: $resolvedTape"
}
$resolvedDvd = ConvertTo-AbsolutePath $DvdPath
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    $resolvedDvd = Get-ConfiguredDvdPath
}
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    throw "Candidate evaluation requires a valid GCN USA DVD image or prior Dusklight selection."
}

$game = Join-Path $repoRoot "build\$Preset\dusklight.exe"
if (-not (Test-Path -LiteralPath $game -PathType Leaf)) {
    throw "Missing game executable: $game"
}

$inspectText = & cargo run --quiet --manifest-path $manifest -- tape inspect $resolvedTape
if ($LASTEXITCODE -ne 0) { throw "Could not inspect candidate tape: $resolvedTape" }
$tapeSummary = (($inspectText -join [Environment]::NewLine) | ConvertFrom-Json)
if ([int]$tapeSummary.wait_frame_count -ne 0) {
    throw "Candidate is not an absolute TAS: found $($tapeSummary.wait_frame_count) reactive frame(s)."
}

if ([string]::IsNullOrWhiteSpace($CandidateId)) {
    $CandidateId = [System.IO.Path]::GetFileNameWithoutExtension($resolvedTape)
}
$safeCandidateId = $CandidateId -replace '[^A-Za-z0-9_.-]', '_'
if ([string]::IsNullOrWhiteSpace($safeCandidateId)) { $safeCandidateId = "candidate" }
if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
    $ArtifactRoot = Join-Path $repoRoot "build\test-results\route-search\$safeCandidateId-$stamp"
}
$resolvedArtifactRoot = ConvertTo-AbsolutePath $ArtifactRoot
New-Item -ItemType Directory -Force $resolvedArtifactRoot | Out-Null

$state = Join-Path $resolvedArtifactRoot "worker-state"
$tracePath = Join-Path $resolvedArtifactRoot "gameplay.trace"
$traceSummaryPath = Join-Path $resolvedArtifactRoot "trace.summary.json"
$resultPath = Join-Path $resolvedArtifactRoot "evaluation.json"
New-Item -ItemType Directory -Force $state | Out-Null

$arguments = @(
    "--dvd", $resolvedDvd,
    "--stage", $Stage,
    "--input-tape", $resolvedTape,
    "--input-tape-end", "hold",
    "--automation-data-root", $state,
    "--gameplay-trace", $tracePath,
    "--cvar", "game.instantSaves=true",
    "--cvar", "backend.cardFileType=1",
    "--cvar", "backend.wasPresetChosen=true",
    "--cvar", "game.enableMenuPointer=false",
    "--headless", "--fixed-step", "--exit-after-tape"
)

if ($DryRun) {
    [ordered]@{
        schema_version = 1
        candidate_id = $CandidateId
        tape = $resolvedTape
        tape_sha256 = (Get-FileHash -LiteralPath $resolvedTape -Algorithm SHA256).Hash.ToLowerInvariant()
        stage = $Stage
        program = $game
        arguments = $arguments
        artifact_root = $resolvedArtifactRoot
    } | ConvertTo-Json -Depth 6
    return
}

$processExitCode = $null
$launchError = $null
$summary = $null
try {
    $argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
    $process = Start-Process -FilePath $game -ArgumentList $argumentLine `
        -WorkingDirectory $repoRoot -Wait -PassThru
    $processExitCode = $process.ExitCode
} catch {
    $launchError = $_.Exception.Message
}

if (Test-Path -LiteralPath $tracePath -PathType Leaf) {
    $summaryText = & cargo run --quiet --manifest-path $manifest -- trace inspect $tracePath
    if ($LASTEXITCODE -eq 0) {
        $summaryText = $summaryText -join [Environment]::NewLine
        [System.IO.File]::WriteAllText(
            $traceSummaryPath, $summaryText, [System.Text.UTF8Encoding]::new($false))
        $summary = $summaryText | ConvertFrom-Json
    } elseif ($null -eq $launchError) {
        $launchError = "Could not inspect gameplay trace."
    }
} elseif ($null -eq $launchError) {
    $launchError = "Game process produced no gameplay trace."
}

$deepestMilestone = "none"
$firstHitTick = $null
$firstHitTapeFrame = $null
$transitionTick = $null
$success = $false
if ($null -ne $summary) {
    if ($null -ne $summary.route_control -and
        $summary.route_control.location.stage_name -eq "F_SP103" -and
        [int]$summary.route_control.location.room -eq 1 -and
        [int]$summary.route_control.location.point -eq 1) {
        $deepestMilestone = "fsp103_route_control"
    }
    if ($null -ne $summary.first_loading_trigger) {
        $deepestMilestone = "fsp103_exit_activated"
    }
    if ($null -ne $summary.first_loading_trigger -and
        $null -ne $summary.first_loading_transition -and
        $summary.first_loading_transition.location.stage_name -eq "F_SP104" -and
        [int]$summary.first_loading_transition.location.point -eq 0) {
        $deepestMilestone = "fsp104_point0"
        $success = $true
        $firstHitTick = [uint64]$summary.first_loading_trigger.simulation_tick
        $firstHitTapeFrame = [uint64]$summary.first_loading_trigger.tape_frame
        $transitionTick = [uint64]$summary.first_loading_transition.simulation_tick
    }
}

$preservedState = $null
if ($KeepState -or $null -ne $launchError -or ($null -ne $processExitCode -and $processExitCode -ne 0)) {
    $preservedState = $state
} elseif (Test-Path -LiteralPath $state) {
    Remove-Item -LiteralPath $state -Recurse -Force
}

$result = [ordered]@{
    schema_version = 1
    candidate_id = $CandidateId
    tape = $resolvedTape
    tape_sha256 = (Get-FileHash -LiteralPath $resolvedTape -Algorithm SHA256).Hash.ToLowerInvariant()
    goal = [ordered]@{
        kind = "scene_transition"
        source = [ordered]@{ stage = "F_SP103"; room = 1; point = 1 }
        destination = [ordered]@{ stage = "F_SP104"; point = 0 }
        score_tick = "source_exit_activation"
    }
    evaluation_status = if ($null -eq $launchError -and $processExitCode -eq 0) { "completed" } else { "worker_error" }
    success = $success
    first_hit_tick = $firstHitTick
    first_hit_tape_frame = $firstHitTapeFrame
    transition_tick = $transitionTick
    deepest_milestone = $deepestMilestone
    process_exit_code = $processExitCode
    error = $launchError
    artifacts = [ordered]@{
        root = $resolvedArtifactRoot
        trace = if (Test-Path -LiteralPath $tracePath) { $tracePath } else { $null }
        trace_summary = if (Test-Path -LiteralPath $traceSummaryPath) { $traceSummaryPath } else { $null }
        evaluation = $resultPath
        worker_state = $preservedState
    }
}
Write-Utf8Json $resultPath $result
$result | ConvertTo-Json -Depth 12
