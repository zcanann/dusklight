[CmdletBinding()]
param(
    [string]$Preset = "windows-clang-debug"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$bootFixture = Join-Path $repoRoot "tests\fixtures\automation\boot_start_smoke.tas"
$bootOutput = Join-Path $repoRoot "build\boot_start_smoke.tape"
$eyeShredderFixture = Join-Path $repoRoot "tests\fixtures\automation\eye_shredder.tas"
$eyeShredderOutput = Join-Path $repoRoot "build\eye_shredder.tape"
$introRouteFixture = Join-Path $repoRoot "tests\fixtures\automation\intro_route.tas"
$introRouteOutput = Join-Path $repoRoot "build\intro_route.tape"
$introFirstExitFixture = Join-Path $repoRoot "tests\fixtures\automation\intro_first_exit.tas"
$introFirstExitOutput = Join-Path $repoRoot "build\intro_first_exit.tape"
$manifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"
$debugState = [System.IO.Path]::GetFullPath(
    (Join-Path $repoRoot "build\automation-state\vscode-debug"))
$automationStateRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $repoRoot "build\automation-state"))

function Invoke-Checked {
    param([string]$FilePath, [string[]]$Arguments)

    Write-Host "`n> $FilePath $($Arguments -join ' ')" -ForegroundColor Cyan
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath"
    }
}

Push-Location $repoRoot
try {
    Invoke-Checked "cmake" @(
        "--preset", $Preset,
        "-DDUSK_ENABLE_CODE_MODS=OFF"
    )
    Invoke-Checked "cmake" @(
        "--build", "--preset", $Preset,
        "--target", "dusklight", "--", "-j$([Environment]::ProcessorCount)"
    )

    New-Item -ItemType Directory -Path (Split-Path $bootOutput) -Force | Out-Null
    Invoke-Checked "cargo" @(
        "run", "--quiet", "--manifest-path", $manifest, "--",
        "tape", "compile", $bootFixture, $bootOutput
    )
    Invoke-Checked "cargo" @(
        "run", "--quiet", "--manifest-path", $manifest, "--",
        "tape", "compile", $introRouteFixture, $introRouteOutput
    )
    Invoke-Checked "cargo" @(
        "run", "--quiet", "--manifest-path", $manifest, "--",
        "tape", "compile", $introFirstExitFixture, $introFirstExitOutput
    )
    Invoke-Checked "cargo" @(
        "run", "--quiet", "--manifest-path", $manifest, "--",
        "tape", "compile", $eyeShredderFixture, $eyeShredderOutput
    )

    $debugStateWithSeparator = $debugState.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    $stateRootWithSeparator = $automationStateRoot.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $debugStateWithSeparator.StartsWith(
            $stateRootWithSeparator, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to reset debug state outside the automation-state root: $debugState"
    }
    if (Test-Path -LiteralPath $debugState) {
        Remove-Item -LiteralPath $debugState -Recurse -Force
    }
    New-Item -ItemType Directory -Path $debugState -Force | Out-Null
    $normalConfig = Join-Path $env:APPDATA "TwilitRealm\Dusklight\config.json"
    if (Test-Path -LiteralPath $normalConfig -PathType Leaf) {
        $configuredDvd = (Get-Content -Raw -LiteralPath $normalConfig | ConvertFrom-Json).'backend.isoPath'
        if (-not [string]::IsNullOrWhiteSpace($configuredDvd)) {
            $debugConfig = [ordered]@{
                'backend.isoPath' = [string]$configuredDvd
            } | ConvertTo-Json
            [System.IO.File]::WriteAllText(
                (Join-Path $debugState "config.json"),
                $debugConfig,
                [System.Text.UTF8Encoding]::new($false))
        }
    }

    Write-Host "`nVisual TAS tapes ready:" -ForegroundColor Green
    Write-Host "  $eyeShredderOutput" -ForegroundColor Green
    Write-Host "  $introFirstExitOutput" -ForegroundColor Green
    Write-Host "  $introRouteOutput" -ForegroundColor Green
    Write-Host "  $bootOutput" -ForegroundColor DarkGray
} finally {
    Pop-Location
}
