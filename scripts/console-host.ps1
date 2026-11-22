# Copies the console host Windows Terminal ships beside our built binary.
#
# The inbox conhost.exe cannot forward an image sequence; the packaged
# OpenConsole.exe can. A non-packaged process cannot execute it in place, so
# the copy is required (ADR 055).

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$base = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages'
$package = Get-ChildItem $base -ErrorAction SilentlyContinue |
    Where-Object { $_.PSChildName -like 'Microsoft.WindowsTerminal_*' } |
    Select-Object -First 1

if (-not $package) {
    throw "Windows Terminal is not installed, so there is no OpenConsole.exe to copy. Place one beside winterm.exe yourself."
}

$source = Join-Path (Get-ItemProperty $package.PSPath).PackageRootFolder 'OpenConsole.exe'
if (-not (Test-Path $source)) {
    throw "No OpenConsole.exe in $($package.PSPath)."
}

foreach ($prof in @('debug', 'release')) {
    $directory = Join-Path $root "target\$prof"
    if (Test-Path $directory) {
        Copy-Item $source (Join-Path $directory 'OpenConsole.exe') -Force
        "copied $source to $directory"
    }
}
