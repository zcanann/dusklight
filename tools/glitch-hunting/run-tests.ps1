[CmdletBinding()]
param(
    [ValidateSet(
        "all",
        "native",
        "input-tape",
        "game-clock",
        "name-entry",
        "name-entry-trace",
        "rng",
        "eye-shredder-oracle",
        "eye-shredder",
        "aurora-card",
        "aurora-time",
        "rust",
        "rust-lint",
        "worker-smoke",
        "pool-smoke"
    )]
    [string]$Test = "all",

    [string]$Preset = "windows-clang-debug",

    [ValidateRange(1, 256)]
    [int]$Jobs = [Environment]::ProcessorCount
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$buildRoot = Join-Path $repoRoot "build\$Preset"
$huntctlManifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"

function Invoke-Checked {
    param(
        [Parameter(Mandatory)]
        [string]$FilePath,

        [Parameter()]
        [string[]]$Arguments = @()
    )

    Write-Host "`n> $FilePath $($Arguments -join ' ')" -ForegroundColor Cyan
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath"
    }
}

function Initialize-NativeBuild {
    Invoke-Checked "cmake" @(
        "--preset", $Preset,
        "-DDUSK_ENABLE_CODE_MODS=OFF"
    )
}

function Build-NativeTargets {
    param([Parameter(Mandatory)][string[]]$Targets)

    Initialize-NativeBuild
    $arguments = @("--build", "--preset", $Preset, "--target") + $Targets + @("--", "-j$Jobs")
    Invoke-Checked "cmake" $arguments
}

function Invoke-NativeExecutable {
    param([Parameter(Mandatory)][string]$RelativePath)

    $executable = Join-Path $buildRoot $RelativePath
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Expected test executable was not built: $executable"
    }
    Invoke-Checked $executable
}

$nativeTests = [ordered]@{
    "input-tape"       = @("dusk_input_tape_test", "dusk_input_tape_test.exe")
    "game-clock"       = @("dusk_game_clock_test", "dusk_game_clock_test.exe")
    "name-entry"       = @("dusk_name_entry_observer_test", "dusk_name_entry_observer_test.exe")
    "name-entry-trace" = @("dusk_name_entry_trace_test", "dusk_name_entry_trace_test.exe")
    "rng"              = @("dusk_rng_test", "dusk_rng_test.exe")
    "eye-shredder-oracle" = @("dusk_eye_shredder_oracle_test", "dusk_eye_shredder_oracle_test.exe")
    "aurora-card"      = @("card_tests", "extern\aurora\tests\card_tests.exe")
    "aurora-time"      = @("os_time_tests", "extern\aurora\tests\os_time_tests.exe")
}

function Invoke-NativeTests {
    param([Parameter(Mandatory)][string[]]$Names)

    $targets = foreach ($name in $Names) {
        $nativeTests[$name][0]
    }
    Build-NativeTargets $targets
    foreach ($name in $Names) {
        Invoke-NativeExecutable $nativeTests[$name][1]
    }
}

function Invoke-RustTests {
    Invoke-Checked "cargo" @("test", "--manifest-path", $huntctlManifest)
}

function Invoke-RustLint {
    Invoke-Checked "cargo" @("fmt", "--manifest-path", $huntctlManifest, "--check")
    Invoke-Checked "cargo" @(
        "clippy", "--manifest-path", $huntctlManifest,
        "--all-targets", "--", "-D", "warnings"
    )
}

function Invoke-WorkerSmoke {
    param([switch]$Pool)

    Build-NativeTargets @("dusklight")
    $worker = Join-Path $buildRoot "dusklight.exe"
    if ($Pool) {
        Invoke-Checked "cargo" @(
            "run", "--quiet", "--manifest-path", $huntctlManifest, "--",
            "pool", "health", "--worker", $worker,
            "--worker-arg", "--automation-worker", "--workers", "2", "--checks", "6"
        )
    } else {
        Invoke-Checked "cargo" @(
            "run", "--quiet", "--manifest-path", $huntctlManifest, "--",
            "hello", "--worker", $worker, "--worker-arg", "--automation-worker"
        )
    }
}

Push-Location $repoRoot
try {
    switch ($Test) {
        "all" {
            Invoke-NativeTests @($nativeTests.Keys)
            Invoke-RustTests
            Invoke-RustLint
            Invoke-WorkerSmoke -Pool
        }
        "native" {
            Invoke-NativeTests @($nativeTests.Keys)
        }
        "rust" {
            Invoke-RustTests
        }
        "rust-lint" {
            Invoke-RustLint
        }
        "worker-smoke" {
            Invoke-WorkerSmoke
        }
        "pool-smoke" {
            Invoke-WorkerSmoke -Pool
        }
        "eye-shredder" {
            & (Join-Path $repoRoot "tools\glitch-hunting\run-eye-shredder.ps1") -Preset $Preset
        }
        default {
            Invoke-NativeTests @($Test)
        }
    }

    Write-Host "`nPASS: $Test" -ForegroundColor Green
} finally {
    Pop-Location
}
