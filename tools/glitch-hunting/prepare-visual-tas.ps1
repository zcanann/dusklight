[CmdletBinding()]
param(
    [string]$Preset = "windows-clang-debug"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$fixture = Join-Path $repoRoot "tests\fixtures\automation\boot_start_smoke.json"
$output = Join-Path $repoRoot "build\boot_start_smoke.tape"
$manifest = Join-Path $repoRoot "tools\huntctl\Cargo.toml"

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

    New-Item -ItemType Directory -Path (Split-Path $output) -Force | Out-Null
    Invoke-Checked "cargo" @(
        "run", "--quiet", "--manifest-path", $manifest, "--",
        "tape", "compile", $fixture, $output
    )

    Write-Host "`nVisual TAS build ready: $output" -ForegroundColor Green
} finally {
    Pop-Location
}
