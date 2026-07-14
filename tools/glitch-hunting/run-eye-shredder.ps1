[CmdletBinding()]
param(
    [string]$DvdPath,

    [string]$Preset = "windows-clang-debug",

    [ValidateRange(1, 1000)]
    [int]$Runs = 3,

    [switch]$Visual,

    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

function ConvertTo-AbsolutePath {
    param([string]$Value)

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $null
    }
    $clean = $Value.Trim().Trim('"').Trim("'")
    if (-not [System.IO.Path]::IsPathRooted($clean)) {
        $clean = Join-Path $repoRoot $clean
    }
    return [System.IO.Path]::GetFullPath($clean)
}

function Get-ConfiguredDvdPath {
    $configPath = Join-Path $env:APPDATA "TwilitRealm\Dusklight\config.json"
    if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
        return $null
    }
    try {
        return ConvertTo-AbsolutePath (
            (Get-Content -Raw -LiteralPath $configPath | ConvertFrom-Json).'backend.isoPath')
    } catch {
        return $null
    }
}

function Quote-ProcessArgument {
    param([string]$Value)

    if ($Value.Contains('"')) {
        throw "Process arguments containing a double quote are not supported: $Value"
    }
    return '"' + $Value + '"'
}

function Remove-ContainedDirectory {
    param(
        [string]$Path,
        [string]$Base
    )

    $resolvedPath = [System.IO.Path]::GetFullPath($Path)
    $resolvedBase = [System.IO.Path]::GetFullPath($Base)
    $pathWithSeparator = $resolvedPath.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    $baseWithSeparator = $resolvedBase.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $pathWithSeparator.StartsWith(
            $baseWithSeparator, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove Eye Shredder state outside its root: $resolvedPath"
    }
    if (Test-Path -LiteralPath $resolvedPath) {
        Remove-Item -LiteralPath $resolvedPath -Recurse -Force -ErrorAction Stop
    }
    if (Test-Path -LiteralPath $resolvedPath) {
        throw "Eye Shredder state still exists after cleanup: $resolvedPath"
    }
}

if ($Visual -and -not $PSBoundParameters.ContainsKey('Runs')) {
    $Runs = 1
}

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot "prepare-visual-tas.ps1") -Preset $Preset
    & (Join-Path $PSScriptRoot "cleanup-visual-tas.ps1")
}

$resolvedDvd = ConvertTo-AbsolutePath $DvdPath
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    $resolvedDvd = Get-ConfiguredDvdPath
}
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    throw "Eye Shredder requires a valid GCN USA DVD image path or a valid prior Dusklight selection."
}

$game = Join-Path $repoRoot "build\$Preset\dusklight.exe"
$tape = Join-Path $repoRoot "build\eye_shredder.tape"
if (-not (Test-Path -LiteralPath $game -PathType Leaf)) {
    throw "Dusklight executable does not exist: $game"
}
if (-not (Test-Path -LiteralPath $tape -PathType Leaf)) {
    throw "Eye Shredder tape does not exist: $tape"
}

$manifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"
$inspectOutput = & cargo run --quiet --manifest-path $manifest -- tape inspect $tape
if ($LASTEXITCODE -ne 0) {
    throw "Could not inspect the compiled Eye Shredder tape."
}
$tapeSummary = ($inspectOutput -join [Environment]::NewLine) | ConvertFrom-Json
if ([int]$tapeSummary.wait_frame_count -ne 0) {
    throw "Eye Shredder is not a TAS: its compiled tape contains $($tapeSummary.wait_frame_count) reactive condition frame(s)."
}
if ([int]$tapeSummary.nominal_frame_count -ne 869) {
    throw "Eye Shredder tape length drifted: expected 869 absolute frames, got $($tapeSummary.nominal_frame_count)."
}

$stateBase = [System.IO.Path]::GetFullPath(
    (Join-Path $repoRoot "build\automation-state\eye-shredder"))
$runStamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
$artifactRoot = Join-Path $repoRoot "build\test-results\eye-shredder\$runStamp"
New-Item -ItemType Directory -Path $stateBase -Force | Out-Null
New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null

$signatures = @()
for ($run = 1; $run -le $Runs; $run++) {
    $runName = "run-{0:D3}" -f $run
    $state = Join-Path $stateBase ([Guid]::NewGuid().ToString("N"))
    $resultPath = Join-Path $artifactRoot "$runName.oracle.json"
    $tracePath = Join-Path $artifactRoot "$runName.name-entry.trace.json"
    New-Item -ItemType Directory -Path $state -Force | Out-Null

    $arguments = @(
        "--dvd", $resolvedDvd,
        "--input-tape", $tape,
        "--input-tape-end", $(if ($Visual) { "release" } else { "hold" }),
        "--automation-data-root", $state,
        "--cursor-breakout-shadow",
        "--name-entry-trace", $tracePath,
        "--automation-oracle", "eye-shredder",
        "--automation-oracle-result", $resultPath,
        "--cvar", "game.instantSaves=true",
        "--cvar", "backend.cardFileType=1",
        "--cvar", "backend.wasPresetChosen=true",
        "--cvar", "game.enableMenuPointer=false",
        "--console"
    )
    if ($Visual) {
        $arguments += @("--fixed-step", "--automation-oracle-continue-on-pass")
    } else {
        $arguments += @("--headless", "--exit-after-tape")
    }

    $mode = if ($Visual) { "visual" } else { "headless" }
    Write-Host "Eye Shredder $runName/$Runs ($mode)" -ForegroundColor Cyan
    try {
        $argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
        $process = Start-Process -FilePath $game -ArgumentList $argumentLine `
            -WorkingDirectory $repoRoot -Wait -PassThru

        if (-not (Test-Path -LiteralPath $resultPath -PathType Leaf)) {
            throw "Eye Shredder did not emit an oracle result (process exit $($process.ExitCode))."
        }
        $result = Get-Content -Raw -LiteralPath $resultPath | ConvertFrom-Json
        if ($process.ExitCode -ne 0 -or $result.status -ne "pass") {
            throw "Eye Shredder failed: status=$($result.status), exit=$($process.ExitCode), reason=$($result.reason)"
        }

        $trace = Get-Content -Raw -LiteralPath $tracePath | ConvertFrom-Json
        $sessionStart = @($trace.event_stream.events | Where-Object { $_.kind -eq "session_started" })[0]
        $sessionEnd = @($trace.event_stream.events | Where-Object { $_.kind -eq "session_ended" })[0]
        $gameplayTick = [uint64]$result.stages.gameplay.sim_tick
        $gameplayFrame = [uint64]$result.stages.gameplay.tape_frame
        $timingMatches =
            [uint64]$result.sim_tick -eq $gameplayTick -and
            [uint64]$result.tape_frame -eq $gameplayFrame -and
            [uint64]$result.stages.memory.sim_tick -eq 692 -and
            [uint64]$result.stages.memory.tape_frame -eq 692 -and
            [uint64]$result.stages.renderer.sim_tick -eq 694 -and
            [uint64]$result.stages.renderer.tape_frame -eq 693 -and
            $gameplayTick -ge 867 -and $gameplayTick -le 868 -and
            $gameplayFrame -eq $gameplayTick -and
            [string]$result.stages.gameplay.telemetry.stage_name -eq "F_SP103" -and
            [int]$result.stages.gameplay.telemetry.room -eq 1 -and
            [int]$result.stages.gameplay.telemetry.point -eq 1 -and
            [int]$result.stages.gameplay.telemetry.layer -eq -1 -and
            [int]$result.stages.gameplay.telemetry.player_actor_name -eq 253 -and
            [bool]$result.stages.gameplay.telemetry.player_actor_present -and
            [bool]$result.stages.gameplay.telemetry.player_is_link -and
            -not [bool]$result.stages.gameplay.telemetry.event_running -and
            [uint64]$sessionStart.sim_tick -eq 334 -and
            [uint64]$sessionEnd.sim_tick -eq 758 -and
            [uint64]$sessionEnd.tape_frame -eq 758 -and
            [int]$result.actual.attempt -eq 2 -and
            [int]$trace.event_stream.drained_count -eq 113 -and
            [int]$trace.event_stream.dropped_count -eq 0 -and
            -not [bool]$trace.snapshot.active -and
            [uint64]$trace.snapshot.sim_tick -eq 727 -and
            [uint64]$trace.snapshot.tape_frame -eq 727 -and
            [int]$trace.snapshot.logical_cursor -eq 5 -and
            [int]$trace.snapshot.last_logical_cursor -eq 114 -and
            [int]$trace.snapshot.name_length -eq 5 -and
            [int]$trace.snapshot.selection_procedure -eq 8 -and
            [int]$trace.snapshot.character_column -eq 12
        if (-not $timingMatches) {
            throw "Eye Shredder reached the corruption through a different timeline; refusing to mask TAS drift."
        }

        $traceSha256 = (Get-FileHash -LiteralPath $tracePath -Algorithm SHA256).Hash

        $signature = [ordered]@{
            status = $result.status
            memory_sim_tick = $result.stages.memory.sim_tick
            memory_tape_frame = $result.stages.memory.tape_frame
            renderer_sim_tick = $result.stages.renderer.sim_tick
            renderer_tape_frame = $result.stages.renderer.tape_frame
            write_attempt = $result.actual.attempt
            character_index = $result.actual.character_index
            original_offset = $result.actual.original_offset
            fresh_usa_gc_cached_address = $result.actual.fresh_usa_gc_cached_address
            bytes = @($result.actual.bytes)
            renderer_xf_num_chans_raw = $result.stages.renderer.telemetry.xf_num_chans_raw
            renderer_bp_num_chans_raw = $result.stages.renderer.telemetry.bp_num_chans_raw
            renderer_mismatch_draw_count = $result.stages.renderer.telemetry.mismatch_draw_count
            gameplay_stage_name = $result.stages.gameplay.telemetry.stage_name
            gameplay_room = $result.stages.gameplay.telemetry.room
            gameplay_point = $result.stages.gameplay.telemetry.point
            gameplay_layer = $result.stages.gameplay.telemetry.layer
            gameplay_player_actor_name = $result.stages.gameplay.telemetry.player_actor_name
            gameplay_player_actor_present = $result.stages.gameplay.telemetry.player_actor_present
            gameplay_player_is_link = $result.stages.gameplay.telemetry.player_is_link
            gameplay_event_running = $result.stages.gameplay.telemetry.event_running
            trace_sha256 = $traceSha256
        } | ConvertTo-Json -Compress
        $signatures += $signature
        $gcAddress = "0x{0:X8}" -f [uint32]$result.actual.fresh_usa_gc_cached_address
        Write-Host "  PASS memory=$($result.stages.memory.sim_tick)/$($result.stages.memory.tape_frame) renderer=$($result.stages.renderer.sim_tick)/$($result.stages.renderer.tape_frame) gameplay=$gameplayTick/$gameplayFrame stage=$($result.stages.gameplay.telemetry.stage_name) attempt=$($result.actual.attempt) GC=$gcAddress bytes=$(@($result.actual.bytes) -join ' ') XF$($result.stages.renderer.telemetry.xf_num_chans_raw)/BP$($result.stages.renderer.telemetry.bp_num_chans_raw)" -ForegroundColor Green
    } finally {
        Remove-ContainedDirectory -Path $state -Base $stateBase
    }
}

if (@($signatures | Select-Object -Unique).Count -ne 1) {
    throw "Eye Shredder runs passed individually but diverged in simulation timing or oracle state."
}

Write-Host "`nPASS: Eye Shredder ($Runs identical run$(if ($Runs -eq 1) { '' } else { 's' }))" -ForegroundColor Green
Write-Host "Artifacts: $artifactRoot" -ForegroundColor DarkGray
