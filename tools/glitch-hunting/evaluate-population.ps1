[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Population,

    [string]$DvdPath,

    [string]$Preset = "windows-clang-debug",

    [ValidateRange(1, 64)]
    [int]$Workers = 4,

    [ValidateRange(1, 1000)]
    [int]$Repetitions = 3,

    [ValidateRange(10, 3600)]
    [int]$TimeoutSeconds = 300,

    [string]$ArtifactRoot,

    [string]$ResultsPath,

    [switch]$KeepState,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$huntctlManifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"

function ConvertTo-AbsolutePath([string]$Value, [string]$Base = $repoRoot) {
    if ([string]::IsNullOrWhiteSpace($Value)) { return $null }
    $clean = $Value.Trim().Trim('"').Trim("'")
    if (-not [System.IO.Path]::IsPathRooted($clean)) { $clean = Join-Path $Base $clean }
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

function Write-Utf8Json([string]$Path, $Value, [int]$Depth = 12) {
    $parent = Split-Path -Parent $Path
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Force $parent | Out-Null
    }
    [System.IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth $Depth),
        [System.Text.UTF8Encoding]::new($false))
}

function Read-NativeMilestones([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "worker produced no native milestone result"
    }
    $result = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
    if ($result.schema.name -ne "dusklight.automation.milestones" -or
        [int]$result.schema.version -ne 1) {
        throw "unsupported native milestone schema"
    }
    $hits = @{}
    foreach ($milestone in @($result.milestones)) {
        $hits[[string]$milestone.id] = $milestone
    }
    return [pscustomobject]@{ Result = $result; Hits = $hits }
}

function Get-AttemptScore([string]$Segment, $Native) {
    $ready = $Native.Hits["gameplay-ready-f-sp103"]
    if ($Segment -eq "boot_to_fsp103") {
        if ($null -ne $ready -and $ready.hit) {
            return [pscustomobject]@{
                Depth = 2
                ScoreTick = [uint64]$ready.sim_tick
                Deepest = "gameplay-ready-f-sp103"
            }
        }
        return [pscustomobject]@{ Depth = 0; ScoreTick = $null; Deepest = "none" }
    }

    $exit = $Native.Hits["exit-f-sp103-to-f-sp104"]
    $entered = $Native.Hits["entered-f-sp104"]
    if ($null -ne $entered -and $entered.hit) {
        if ($null -eq $exit -or -not $exit.hit) {
            throw "entered-f-sp104 hit without the required source-exit milestone"
        }
        # Loading completion contains host-I/O latency. Reaching F_SP104 proves the destination,
        # while the authoritative source-exit tick is the route-golf score.
        return [pscustomobject]@{
            Depth = 4
            ScoreTick = [uint64]$exit.sim_tick
            Deepest = "entered-f-sp104"
        }
    }
    if ($null -ne $exit -and $exit.hit) {
        return [pscustomobject]@{
            Depth = 3
            ScoreTick = [uint64]$exit.sim_tick
            Deepest = "exit-f-sp103-to-f-sp104"
        }
    }
    if ($null -ne $ready -and $ready.hit) {
        return [pscustomobject]@{
            Depth = 2
            ScoreTick = [uint64]$ready.sim_tick
            Deepest = "gameplay-ready-f-sp103"
        }
    }
    return [pscustomobject]@{ Depth = 0; ScoreTick = $null; Deepest = "none" }
}

$resolvedPopulation = ConvertTo-AbsolutePath $Population
if ($null -eq $resolvedPopulation -or
    -not (Test-Path -LiteralPath $resolvedPopulation -PathType Leaf)) {
    throw "Population manifest does not exist: $resolvedPopulation"
}
$populationRoot = Split-Path -Parent $resolvedPopulation
$manifest = Get-Content -Raw -LiteralPath $resolvedPopulation | ConvertFrom-Json
if ($manifest.schema -ne "dusklight-search-population/v1") {
    throw "Unsupported population schema: $($manifest.schema)"
}
if ($manifest.segment -notin @("boot_to_fsp103", "fsp103_to_fsp104")) {
    throw "Unsupported population segment: $($manifest.segment)"
}
$members = @($manifest.members)
if ($members.Count -eq 0) { throw "Population contains no candidates." }

$seen = @{}
foreach ($member in $members) {
    $candidateId = [string]$member.candidate_id
    if ([string]::IsNullOrWhiteSpace($candidateId) -or $seen.ContainsKey($candidateId)) {
        throw "Population contains a blank or duplicate candidate ID: $candidateId"
    }
    $seen[$candidateId] = $true
    $member | Add-Member -NotePropertyName resolved_tape -NotePropertyValue (
        ConvertTo-AbsolutePath ([string]$member.tape_file) $populationRoot)
    if (-not (Test-Path -LiteralPath $member.resolved_tape -PathType Leaf)) {
        throw "Candidate tape does not exist: $($member.resolved_tape)"
    }
}

$resolvedDvd = ConvertTo-AbsolutePath $DvdPath
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    $resolvedDvd = Get-ConfiguredDvdPath
}
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    throw "Population evaluation requires a valid GCN USA DVD image or prior Dusklight selection."
}

$game = Join-Path $repoRoot "build\$Preset\dusklight.exe"
if (-not (Test-Path -LiteralPath $game -PathType Leaf)) {
    throw "Missing game executable: $game"
}

if ([string]::IsNullOrWhiteSpace($ArtifactRoot)) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
    $ArtifactRoot = Join-Path $repoRoot "build\test-results\search\$($manifest.segment)\$stamp"
}
$resolvedArtifactRoot = ConvertTo-AbsolutePath $ArtifactRoot
New-Item -ItemType Directory -Force $resolvedArtifactRoot | Out-Null
if ([string]::IsNullOrWhiteSpace($ResultsPath)) {
    $ResultsPath = Join-Path $resolvedArtifactRoot "results.json"
}
$resolvedResultsPath = ConvertTo-AbsolutePath $ResultsPath

$milestoneList = if ($manifest.segment -eq "boot_to_fsp103") {
    "gameplay-ready-f-sp103"
} else {
    "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,entered-f-sp104"
}
$goal = if ($manifest.segment -eq "boot_to_fsp103") {
    "gameplay-ready-f-sp103"
} else {
    "entered-f-sp104"
}
$stage = if ($manifest.segment -eq "fsp103_to_fsp104") { "F_SP103,1,1,3" } else { $null }

$pending = [System.Collections.Generic.Queue[object]]::new()
foreach ($member in $members) {
    $safeId = ([string]$member.candidate_id) -replace '[^A-Za-z0-9_.-]', '_'
    for ($attempt = 1; $attempt -le $Repetitions; $attempt++) {
        $attemptRoot = Join-Path $resolvedArtifactRoot (
            "candidates\$safeId\attempt-{0:D3}" -f $attempt)
        $state = Join-Path $attemptRoot "state"
        New-Item -ItemType Directory -Force $state | Out-Null
        $pending.Enqueue([pscustomobject]@{
            CandidateId = [string]$member.candidate_id
            Tape = [string]$member.resolved_tape
            Attempt = $attempt
            Root = $attemptRoot
            State = $state
            Milestones = Join-Path $attemptRoot "milestones.json"
            Stdout = Join-Path $attemptRoot "stdout.txt"
            Stderr = Join-Path $attemptRoot "stderr.txt"
        })
    }
}

$plan = [ordered]@{
    schema = "dusklight-search-evaluation-plan/v1"
    segment = [string]$manifest.segment
    candidates = $members.Count
    repetitions = $Repetitions
    evaluations = $pending.Count
    workers = $Workers
    target = $goal
    route_score_tick = if ($manifest.segment -eq "fsp103_to_fsp104") {
        "exit-f-sp103-to-f-sp104"
    } else { "gameplay-ready-f-sp103" }
    stage = $stage
    dvd = $resolvedDvd
    program = $game
    artifact_root = $resolvedArtifactRoot
    results = $resolvedResultsPath
}
Write-Utf8Json (Join-Path $resolvedArtifactRoot "plan.json") $plan
if ($DryRun) {
    $plan | ConvertTo-Json -Depth 6
    return
}

$running = [System.Collections.ArrayList]::new()
$completed = [System.Collections.ArrayList]::new()
$total = $pending.Count
try {
    while ($pending.Count -gt 0 -or $running.Count -gt 0) {
        while ($pending.Count -gt 0 -and $running.Count -lt $Workers) {
            $trial = $pending.Dequeue()
            $arguments = @("--dvd", $resolvedDvd)
            if ($null -ne $stage) { $arguments += @("--stage", $stage) }
            $arguments += @(
                "--input-tape", $trial.Tape,
                "--input-tape-end", "hold",
                "--automation-data-root", $trial.State,
                "--milestones", $milestoneList,
                "--milestone-goal", $goal,
                "--milestone-result", $trial.Milestones,
                "--cvar", "game.instantSaves=true",
                "--cvar", "backend.cardFileType=1",
                "--cvar", "backend.wasPresetChosen=true",
                "--cvar", "game.enableMenuPointer=false",
                "--headless", "--fixed-step", "--exit-after-tape"
            )
            $argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
            try {
                $process = Start-Process -FilePath $game -ArgumentList $argumentLine `
                    -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru `
                    -RedirectStandardOutput $trial.Stdout -RedirectStandardError $trial.Stderr
                [void]$running.Add([pscustomobject]@{
                    Trial = $trial
                    Process = $process
                    Started = [DateTime]::UtcNow
                })
            } catch {
                [void]$completed.Add([pscustomobject]@{
                    Trial = $trial
                    ExitCode = $null
                    TimedOut = $false
                    LaunchError = $_.Exception.Message
                })
            }
        }

        $finishedThisPass = $false
        foreach ($job in @($running)) {
            $job.Process.Refresh()
            $timedOut = ([DateTime]::UtcNow - $job.Started).TotalSeconds -ge $TimeoutSeconds
            if (-not $job.Process.HasExited -and -not $timedOut) { continue }
            if ($timedOut -and -not $job.Process.HasExited) {
                try { $job.Process.Kill() } catch { }
            }
            $job.Process.WaitForExit()
            [void]$completed.Add([pscustomobject]@{
                Trial = $job.Trial
                ExitCode = $job.Process.ExitCode
                TimedOut = $timedOut
                LaunchError = $null
            })
            [void]$running.Remove($job)
            $finishedThisPass = $true
            $displayIdLength = [Math]::Min(12, $job.Trial.CandidateId.Length)
            $displayId = $job.Trial.CandidateId.Substring(0, $displayIdLength)
            Write-Host ("[{0}/{1}] {2} attempt {3}" -f
                $completed.Count, $total, $displayId, $job.Trial.Attempt) -ForegroundColor Cyan
        }
        if (-not $finishedThisPass -and $running.Count -gt 0) {
            Start-Sleep -Milliseconds 50
        }
    }
} finally {
    foreach ($job in @($running)) {
        if (-not $job.Process.HasExited) {
            try { $job.Process.Kill() } catch { }
        }
        try { $job.Process.WaitForExit() } catch { }
    }
}

$attempts = [System.Collections.ArrayList]::new()
foreach ($job in $completed) {
    $workerError = $job.LaunchError
    if ($job.TimedOut) { $workerError = "worker timed out after $TimeoutSeconds seconds" }
    $score = [pscustomobject]@{ Depth = 0; ScoreTick = $null; Deepest = "none" }
    if ($null -eq $workerError) {
        try {
            $native = Read-NativeMilestones $job.Trial.Milestones
            $score = Get-AttemptScore ([string]$manifest.segment) $native
        } catch {
            $workerError = $_.Exception.Message
        }
    }

    $preservedState = $null
    if ($KeepState -or $null -ne $workerError) {
        $preservedState = $job.Trial.State
    } elseif (Test-Path -LiteralPath $job.Trial.State) {
        $resolvedState = [System.IO.Path]::GetFullPath($job.Trial.State)
        $artifactPrefix = $resolvedArtifactRoot.TrimEnd('\', '/') +
            [System.IO.Path]::DirectorySeparatorChar
        if (-not ($resolvedState + [System.IO.Path]::DirectorySeparatorChar).StartsWith(
                $artifactPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove worker state outside $resolvedArtifactRoot"
        }
        Remove-Item -LiteralPath $resolvedState -Recurse -Force
    }
    [void]$attempts.Add([pscustomobject]@{
        CandidateId = $job.Trial.CandidateId
        Attempt = $job.Trial.Attempt
        Depth = [int]$score.Depth
        ScoreTick = $score.ScoreTick
        Deepest = $score.Deepest
        ExitCode = $job.ExitCode
        Error = $workerError
        Milestones = if (Test-Path -LiteralPath $job.Trial.Milestones) {
            $job.Trial.Milestones
        } else { $null }
        WorkerState = $preservedState
    })
}

$candidateResults = [ordered]@{}
foreach ($member in $members) {
    $candidateAttempts = @($attempts | Where-Object CandidateId -eq $member.candidate_id)
    $validAttempts = @($candidateAttempts | Where-Object { $null -eq $_.Error })
    $depth = if ($validAttempts.Count -gt 0) {
        [int](($validAttempts | Measure-Object -Property Depth -Maximum).Maximum)
    } else { 0 }
    $winningAttempts = if ($depth -gt 0) {
        @($validAttempts | Where-Object Depth -eq $depth)
    } else { @() }
    $winningAttempts = @($winningAttempts)
    $ticks = @($winningAttempts | ForEach-Object { [uint64]$_.ScoreTick })
    $candidateResults[[string]$member.candidate_id] = [ordered]@{
        milestone_depth = $depth
        attempts = $Repetitions
        successes = $winningAttempts.Count
        first_hit_ticks = $ticks
    }
}

$results = [ordered]@{
    schema = "dusklight-search-results/v1"
    segment = [string]$manifest.segment
    candidates = $candidateResults
}
Write-Utf8Json $resolvedResultsPath $results

$attemptSummary = [ordered]@{
    schema = "dusklight-search-evaluation/v1"
    population = $resolvedPopulation
    results = $resolvedResultsPath
    workers = $Workers
    repetitions = $Repetitions
    invalid_attempts = @($attempts | Where-Object { $null -ne $_.Error }).Count
    attempts = @($attempts | Sort-Object CandidateId, Attempt)
}
Write-Utf8Json (Join-Path $resolvedArtifactRoot "evaluation.json") $attemptSummary 16

if ($attemptSummary.invalid_attempts -gt 0) {
    throw "Population evaluation had $($attemptSummary.invalid_attempts) invalid worker attempt(s); see $(Join-Path $resolvedArtifactRoot 'evaluation.json')"
}

# This also proves that the generated document matches Rust's deny-unknown-fields schema.
& cargo run --quiet --manifest-path $huntctlManifest -- search rank `
    --population $resolvedPopulation --results $resolvedResultsPath | Out-Null
if ($LASTEXITCODE -ne 0) { throw "huntctl rejected generated search results: $resolvedResultsPath" }

$results | ConvertTo-Json -Depth 10
