[CmdletBinding()]
param(
    [string]$DvdPath,
    [string]$Preset = "windows-clang-debug",
    [ValidateRange(250, 30000)]
    [int]$ProbeDelayMilliseconds = 1500,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$game = Join-Path $repoRoot "build\$Preset\dusklight.exe"
$tape = Join-Path $repoRoot "build\boot-start-smoke.tape"
$fixture = Join-Path $repoRoot "tests\fixtures\automation\boot_start_smoke.tas"
$huntctl = Join-Path $repoRoot "tools\huntctl\Cargo.toml"

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

if (-not $IsWindows -and $null -ne $IsWindows) {
    throw "The native taskbar/window regression probe currently requires Windows."
}

if (-not $SkipBuild) {
    & cmake --build --preset $Preset --target dusklight --parallel $([Environment]::ProcessorCount)
    if ($LASTEXITCODE -ne 0) { throw "Dusklight build failed." }
}
if (-not (Test-Path -LiteralPath $game -PathType Leaf)) { throw "Missing game executable: $game" }
if (-not (Test-Path -LiteralPath $tape -PathType Leaf)) {
    & cargo run --quiet --manifest-path $huntctl -- tape compile $fixture $tape
    if ($LASTEXITCODE -ne 0) { throw "Could not compile the headless probe tape." }
}

$resolvedDvd = ConvertTo-AbsolutePath $DvdPath
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    $resolvedDvd = Get-ConfiguredDvdPath
}
if ($null -eq $resolvedDvd -or -not (Test-Path -LiteralPath $resolvedDvd -PathType Leaf)) {
    throw "Headless window test requires a valid DVD image or prior Dusklight selection."
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
$artifactRoot = Join-Path $repoRoot "build\test-results\headless-window\$stamp"
$state = Join-Path $artifactRoot "state"
New-Item -ItemType Directory -Force $state | Out-Null

if (-not ("DusklightWindowProbe" -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class DusklightWindowProbe {
    private delegate bool EnumProc(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc proc, IntPtr state);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);

    public static int VisibleTopLevelWindows(uint wantedPid) {
        int count = 0;
        EnumWindows((hwnd, _) => {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            if (pid == wantedPid && IsWindowVisible(hwnd)) count++;
            return true;
        }, IntPtr.Zero);
        return count;
    }
}
'@
}

$arguments = @(
    "--dvd", $resolvedDvd,
    "--input-tape", $tape,
    "--input-tape-end", "hold",
    "--automation-data-root", $state,
    "--cvar", "backend.wasPresetChosen=true",
    "--headless"
)
$argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
$process = Start-Process -FilePath $game -ArgumentList $argumentLine `
    -WorkingDirectory $repoRoot -PassThru

try {
    Start-Sleep -Milliseconds $ProbeDelayMilliseconds
    $process.Refresh()
    if ($process.HasExited) {
        throw "Headless probe exited before inspection: exit=$($process.ExitCode)"
    }

    $visibleWindows = [DusklightWindowProbe]::VisibleTopLevelWindows([uint32]$process.Id)
    $result = [ordered]@{
        schema = "dusklight-headless-window-test/v1"
        pid = $process.Id
        main_window_handle = [int64]$process.MainWindowHandle
        visible_top_level_windows = $visibleWindows
        passed = $process.MainWindowHandle -eq 0 -and $visibleWindows -eq 0
    }
    [System.IO.File]::WriteAllText(
        (Join-Path $artifactRoot "result.json"),
        ($result | ConvertTo-Json -Depth 4),
        [System.Text.UTF8Encoding]::new($false))
    if (-not $result.passed) {
        throw "Headless process exposed a desktop/taskbar window; see $artifactRoot"
    }
    Write-Host "PASS: headless process has no visible top-level window or taskbar surface." -ForegroundColor Green
    $result | ConvertTo-Json -Depth 4
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    try { $process.WaitForExit() } catch { }
}
