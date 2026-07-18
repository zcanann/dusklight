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
$milestonePath = Join-Path $resolvedArtifactRoot "milestones.json"
$resultPath = Join-Path $resolvedArtifactRoot "evaluation.json"
New-Item -ItemType Directory -Force $state | Out-Null

$arguments = @(
    "--dvd", $resolvedDvd,
    "--stage", $Stage,
    "--input-tape", $resolvedTape,
    "--input-tape-end", "hold",
    "--automation-data-root", $state,
    "--gameplay-trace", $tracePath,
    "--milestones", "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,entered-f-sp104",
    "--milestone-goal", "entered-f-sp104",
    "--milestone-result", $milestonePath,
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
$workerError = $null
$diagnosticError = $null
$summary = $null
$milestoneResult = $null
try {
    $argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
    $process = Start-Process -FilePath $game -ArgumentList $argumentLine `
        -WorkingDirectory $repoRoot -Wait -PassThru
    $processExitCode = $process.ExitCode
} catch {
    $workerError = $_.Exception.Message
}

if (Test-Path -LiteralPath $milestonePath -PathType Leaf) {
    try {
        $milestoneResult = Get-Content -Raw -LiteralPath $milestonePath | ConvertFrom-Json
        if ($milestoneResult.schema.name -ne "dusklight.automation.milestones" -or
            [int]$milestoneResult.schema.version -notin @(1, 2)) {
            throw "Unsupported native milestone result schema."
        }
    } catch {
        if ($null -eq $workerError) { $workerError = $_.Exception.Message }
    }
} elseif ($null -eq $workerError) {
    $workerError = "Game process produced no native milestone result."
}

if (Test-Path -LiteralPath $tracePath -PathType Leaf) {
    $summaryText = & cargo run --quiet --manifest-path $manifest -- trace inspect $tracePath
    if ($LASTEXITCODE -eq 0) {
        $summaryText = $summaryText -join [Environment]::NewLine
        [System.IO.File]::WriteAllText(
            $traceSummaryPath, $summaryText, [System.Text.UTF8Encoding]::new($false))
        $summary = $summaryText | ConvertFrom-Json
    } else {
        $diagnosticError = "Could not inspect gameplay trace."
    }
} else {
    $diagnosticError = "Game process produced no gameplay trace."
}

$deepestMilestone = "none"
$milestoneDepth = 0
$deepestTick = $null
$firstHitTick = $null
$firstHitTapeFrame = $null
$exitActivationTick = $null
$transitionTick = $null
$goalHitTick = $null
$success = $false
if ($null -ne $milestoneResult) {
    $hits = @{}
    foreach ($hit in $milestoneResult.milestones) { $hits[[string]$hit.id] = $hit }
    $ready = $hits["gameplay-ready-f-sp103"]
    $exit = $hits["exit-f-sp103-to-f-sp104"]
    $entered = $hits["entered-f-sp104"]
    if ($null -ne $ready -and $ready.hit) {
        $deepestMilestone = "gameplay-ready-f-sp103"
        $milestoneDepth = 2
        $deepestTick = [uint64]$ready.sim_tick
    }
    if ($null -ne $exit -and $exit.hit) {
        $deepestMilestone = "exit-f-sp103-to-f-sp104"
        $milestoneDepth = 3
        $deepestTick = [uint64]$exit.sim_tick
        $exitActivationTick = [uint64]$exit.sim_tick
    }
    if ($null -ne $entered -and $entered.hit) {
        $deepestMilestone = "entered-f-sp104"
        $milestoneDepth = 4
        $deepestTick = [uint64]$entered.sim_tick
        $transitionTick = [uint64]$entered.sim_tick
    }
    if ($milestoneResult.goal -eq "entered-f-sp104" -and
        $milestoneResult.goal_reached -and $null -ne $entered -and $entered.hit) {
        $success = $true
        # Loading duration is host work after movement has finished. Require the
        # entered-map goal for validity, but rank successful routes by the native
        # source-exit hit which the candidate can actually improve.
        $firstHitTick = [uint64]$exit.sim_tick
        $firstHitTapeFrame = [uint64]$exit.tape_frame
        $goalHitTick = [uint64]$entered.sim_tick
    }
}

$searchFirstHitTicks = [System.Collections.Generic.List[uint64]]::new()
if ($milestoneDepth -gt 0) {
    $searchTick = if ($success) { $firstHitTick } else { $deepestTick }
    $searchFirstHitTicks.Add([uint64]$searchTick)
}

$preservedState = $null
if ($KeepState -or $null -ne $workerError) {
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
        native_milestone = "entered-f-sp104"
        score_tick = "verified_source_exit_first_hit"
    }
    # A valid native result with an unmet goal is an ordinary search sample;
    # the engine intentionally uses a nonzero exit code for that case.
    evaluation_status = if ($null -eq $workerError) { "completed" } else { "worker_error" }
    success = $success
    first_hit_tick = $firstHitTick
    first_hit_tape_frame = $firstHitTapeFrame
    exit_activation_tick = $exitActivationTick
    transition_tick = $transitionTick
    goal_hit_tick = $goalHitTick
    deepest_milestone = $deepestMilestone
    search_result = [ordered]@{
        milestone_depth = $milestoneDepth
        attempts = 1
        successes = if ($milestoneDepth -gt 0) { 1 } else { 0 }
        first_hit_ticks = $searchFirstHitTicks
    }
    process_exit_code = $processExitCode
    error = $workerError
    diagnostic_error = $diagnosticError
    artifacts = [ordered]@{
        root = $resolvedArtifactRoot
        native_milestones = if (Test-Path -LiteralPath $milestonePath) { $milestonePath } else { $null }
        trace = if (Test-Path -LiteralPath $tracePath) { $tracePath } else { $null }
        trace_summary = if (Test-Path -LiteralPath $traceSummaryPath) { $traceSummaryPath } else { $null }
        evaluation = $resultPath
        worker_state = $preservedState
    }
}
Write-Utf8Json $resultPath $result
$result | ConvertTo-Json -Depth 12
