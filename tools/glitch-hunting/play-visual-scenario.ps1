[CmdletBinding()]
param(
    [ValidateSet("eye-shredder", "intro-first-exit", "intro-cutscene", "boot-start-smoke")]
    [string]$Scenario = "eye-shredder",

    [string]$DvdPath,

    [string]$Preset = "windows-clang-debug",

    [switch]$SkipBuild,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

$runner = $null
$runnerParameters = @{
    Preset = $Preset
}
switch ($Scenario) {
    "eye-shredder" {
        $runner = Join-Path $PSScriptRoot "run-eye-shredder.ps1"
        $runnerParameters.Visual = $true
        $runnerParameters.SkipBuild = $true
    }
    "boot-start-smoke" {
        $runner = Join-Path $PSScriptRoot "play-visual-tas.ps1"
        $runnerParameters.TapePath = Join-Path $repoRoot "build\boot-start-smoke.tape"
    }
    "intro-first-exit" {
        $runner = Join-Path $PSScriptRoot "play-visual-tas.ps1"
        $runnerParameters.TapePath = Join-Path $repoRoot "build\intro-first-exit.tape"
    }
    "intro-cutscene" {
        $runner = Join-Path $PSScriptRoot "play-visual-tas.ps1"
        $runnerParameters.TapePath = Join-Path $repoRoot "build\intro-cutscene.tape"
    }
}

if (-not [string]::IsNullOrWhiteSpace($DvdPath)) {
    $runnerParameters.DvdPath = $DvdPath
}

if ($DryRun) {
    [ordered]@{
        scenario = $Scenario
        runner = [System.IO.Path]::GetFullPath($runner)
        parameters = $runnerParameters
    } | ConvertTo-Json -Depth 3
    return
}

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot "prepare-visual-tas.ps1") -Preset $Preset
}

& $runner @runnerParameters
