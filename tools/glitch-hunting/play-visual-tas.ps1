[CmdletBinding()]
param(
    [string]$DvdPath,

    [string]$TapePath = "build/boot_start_smoke.tape",

    [string]$Preset = "windows-clang-debug",

    [switch]$ExitAfterTape
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

function ConvertTo-AbsolutePath {
    param([string]$Value)

    $clean = $Value.Trim()
    if ($clean.Length -ge 2 -and
        (($clean[0] -eq '"' -and $clean[$clean.Length - 1] -eq '"') -or
         ($clean[0] -eq "'" -and $clean[$clean.Length - 1] -eq "'"))) {
        $clean = $clean.Substring(1, $clean.Length - 2).Trim()
    }
    if ([string]::IsNullOrWhiteSpace($clean)) {
        return $null
    }
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
        $config = Get-Content -Raw -LiteralPath $configPath | ConvertFrom-Json
        return ConvertTo-AbsolutePath $config.'backend.isoPath'
    } catch {
        Write-Warning "Could not read Dusklight's configured DVD path from '$configPath': $_"
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

$resolvedDvd = ConvertTo-AbsolutePath $DvdPath
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    $configuredDvd = Get-ConfiguredDvdPath
    if ($null -ne $configuredDvd -and (Test-Path -LiteralPath $configuredDvd -PathType Leaf)) {
        if ($null -ne $resolvedDvd) {
            Write-Warning "Prompted DVD path does not exist: $resolvedDvd"
        }
        $resolvedDvd = $configuredDvd
        Write-Host "Using Dusklight's last manually selected DVD image." -ForegroundColor Yellow
    } else {
        $displayPath = if ($null -eq $resolvedDvd) { "<blank>" } else { $resolvedDvd }
        throw "DVD image does not exist: $displayPath. Enter a valid path or select one once in Dusklight."
    }
}

$resolvedTape = ConvertTo-AbsolutePath $TapePath
if ($null -eq $resolvedTape -or -not (Test-Path -LiteralPath $resolvedTape -PathType Leaf)) {
    $displayPath = if ($null -eq $resolvedTape) { "<blank>" } else { $resolvedTape }
    throw "Input tape does not exist: $displayPath"
}

$game = Join-Path $repoRoot "build\$Preset\dusklight.exe"
if (-not (Test-Path -LiteralPath $game -PathType Leaf)) {
    throw "Dusklight executable does not exist: $game. Run the prepare task first."
}

$gameArguments = @(
    "--dvd", $resolvedDvd,
    "--input-tape", $resolvedTape,
    "--input-tape-end", "release",
    "--develop",
    "--console"
)
if ($ExitAfterTape) {
    $gameArguments += "--exit-after-tape"
}

Write-Host "DVD:  $resolvedDvd" -ForegroundColor Cyan
Write-Host "Tape: $resolvedTape" -ForegroundColor Cyan
Write-Host "Starting visible TAS playback..." -ForegroundColor Green

$argumentLine = ($gameArguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
$process = Start-Process -FilePath $game -ArgumentList $argumentLine `
    -WorkingDirectory $repoRoot -Wait -PassThru
if ($process.ExitCode -ne 0) {
    throw "Dusklight exited with code $($process.ExitCode). Check the newest log under '$env:APPDATA\TwilitRealm\Dusklight\logs'."
}
