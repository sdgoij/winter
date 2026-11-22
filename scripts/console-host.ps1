# Copies the console host Windows Terminal ships beside our built binary.
#
# The inbox conhost.exe cannot forward an image sequence; the packaged
# OpenConsole.exe can. A non-packaged process cannot execute it in place, so
# the copy is required (ADR 055).
#
#   just console-host                      copies into target\debug and target\release
#   scripts\console-host.ps1 -Destination dist
#                                          copies into a release folder
#   scripts\console-host.ps1 -Source path\to\OpenConsole.exe
#                                          uses a host built or unpacked elsewhere

param(
    [string[]]$Destination = @(),
    [string]$Source = ''
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot

function Find-Host {
    if ($Source) {
        if (-not (Test-Path $Source)) {
            throw "No console host at $Source."
        }
        return $Source
    }
    if ($env:WINTERM_CONSOLE_HOST) {
        if (-not (Test-Path $env:WINTERM_CONSOLE_HOST)) {
            throw "WINTERM_CONSOLE_HOST points at $($env:WINTERM_CONSOLE_HOST), which does not exist."
        }
        return $env:WINTERM_CONSOLE_HOST
    }

    $base = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages'
    $package = Get-ChildItem $base -ErrorAction SilentlyContinue |
        Where-Object { $_.PSChildName -like 'Microsoft.WindowsTerminal_*' } |
        Select-Object -First 1

    if (-not $package) {
        throw "Windows Terminal is not installed, so there is no OpenConsole.exe to copy. Pass -Source or set WINTERM_CONSOLE_HOST."
    }

    $hostPath = Join-Path (Get-ItemProperty $package.PSPath).PackageRootFolder 'OpenConsole.exe'
    if (-not (Test-Path $hostPath)) {
        throw "No OpenConsole.exe in $($package.PSPath)."
    }
    return $hostPath
}

$hostPath = Find-Host
$notice = Join-Path (Split-Path -Parent $hostPath) 'NOTICE.html'

$targets = if ($Destination.Count -gt 0) {
    $Destination | ForEach-Object {
        if ([System.IO.Path]::IsPathRooted($_)) { $_ } else { Join-Path $root $_ }
    }
} else {
    @('debug', 'release') | ForEach-Object {
        Join-Path $root "target\$_"
    } | Where-Object { Test-Path $_ }
}

if (-not $targets) {
    throw "No destination: build first, or pass -Destination."
}

foreach ($directory in $targets) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    Copy-Item $hostPath (Join-Path $directory 'OpenConsole.exe') -Force
    if (Test-Path $notice) {
        Copy-Item $notice (Join-Path $directory 'OpenConsole.NOTICE.html') -Force
    }
    "copied $hostPath to $directory"
}
