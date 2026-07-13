[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$automationStateRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $repoRoot "build\automation-state"))
$debugState = [System.IO.Path]::GetFullPath(
    (Join-Path $automationStateRoot "vscode-debug"))

$debugStateWithSeparator = $debugState.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
$stateRootWithSeparator = $automationStateRoot.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
if (-not $debugStateWithSeparator.StartsWith(
        $stateRootWithSeparator, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove debug state outside the automation-state root: $debugState"
}

if (Test-Path -LiteralPath $debugState) {
    Remove-Item -LiteralPath $debugState -Recurse -Force -ErrorAction Stop
    if (Test-Path -LiteralPath $debugState) {
        throw "Visual TAS debug state still exists after cleanup: $debugState"
    }
    Write-Host "Removed Visual TAS debug state: $debugState" -ForegroundColor DarkGray
}
