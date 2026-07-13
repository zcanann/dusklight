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
        "--input-tape-end", "hold",
        "--exit-after-tape",
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
        $arguments += "--automation-oracle-continue-on-pass"
    } else {
        $arguments += "--headless"
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

        $signature = [ordered]@{
            status = $result.status
            tape_frame = $result.tape_frame
            character_index = $result.actual.character_index
            original_offset = $result.actual.original_offset
            fresh_usa_gc_cached_address = $result.actual.fresh_usa_gc_cached_address
            bytes = @($result.actual.bytes)
            renderer_xf_num_chans_raw = $result.stages.renderer.telemetry.xf_num_chans_raw
            renderer_bp_num_chans_raw = $result.stages.renderer.telemetry.bp_num_chans_raw
            renderer_mismatch_draw_count = $result.stages.renderer.telemetry.mismatch_draw_count
        } | ConvertTo-Json -Compress
        $signatures += $signature
        $gcAddress = "0x{0:X8}" -f [uint32]$result.actual.fresh_usa_gc_cached_address
        Write-Host "  PASS memory_frame=$($result.stages.memory.tape_frame) GC=$gcAddress bytes=$(@($result.actual.bytes) -join ' ') renderer=XF$($result.stages.renderer.telemetry.xf_num_chans_raw)/BP$($result.stages.renderer.telemetry.bp_num_chans_raw)" -ForegroundColor Green
    } finally {
        Remove-ContainedDirectory -Path $state -Base $stateBase
    }
}

if (@($signatures | Select-Object -Unique).Count -ne 1) {
    throw "Eye Shredder runs passed individually but produced different oracle signatures."
}

Write-Host "`nPASS: Eye Shredder ($Runs identical run$(if ($Runs -eq 1) { '' } else { 's' }))" -ForegroundColor Green
Write-Host "Artifacts: $artifactRoot" -ForegroundColor DarkGray
