#Requires -Version 5.1
<#
.SYNOPSIS
    Package the CodexPlus Windows release into a side-by-side folder and zip.

.DESCRIPTION
    After `cargo build --release` has produced target/release/codexplus.exe, this
    script copies that binary together with the project README.md into a
    per-version staging folder under target/release/ and additionally produces
    a matching zip archive. The result mirrors the layout expected by GitHub /
    Gitee Releases:

        target/release/
            codexplus-vX.Y.Z-windows-x64/
                codexplus.exe
                README.md
            codexplus-vX.Y.Z-windows-x64.zip

    Both artifacts contain the same files; the folder is the unzipped view of
    the archive so either form can be uploaded.

.PARAMETER Version
    Release version. Accepts both `v0.2.0` and `0.2.0`; the leading `v` is
    stripped when naming the staging folder. Required.

.PARAMETER ExePath
    Override the source executable. Defaults to `<repo>/target/release/codexplus.exe`.

.PARAMETER ReadmePath
    Override the README source. Defaults to `<repo>/README.md`.

.PARAMETER OutputDir
    Override the parent output directory. Defaults to `<repo>/target/release`.

.PARAMETER RepoRoot
    Override the repository root. Defaults to the script's parent directory.

.EXAMPLE
    .\scripts\package-release.ps1 -Version v0.2.0

.EXAMPLE
    .\scripts\package-release.ps1 -Version 0.2.1 -ExePath target\release\codexplus.exe
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$ExePath,
    [string]$ReadmePath,
    [string]$OutputDir,
    [string]$RepoRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

function Require-File {
    param(
        [string]$Path,
        [string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "$Label not found: $Path"
    }
}

if (-not $ExePath) {
    $ExePath = Join-Path $RepoRoot "target\release\codexplus.exe"
}
if (-not $ReadmePath) {
    $ReadmePath = Join-Path $RepoRoot "README.md"
}
if (-not $OutputDir) {
    $OutputDir = Join-Path $RepoRoot "target\release"
}

Require-File -Path $ExePath -Label "Release executable"
Require-File -Path $ReadmePath -Label "README.md"

$versionTag = $Version.Trim()
if ($versionTag.StartsWith("v")) {
    $versionTag = $versionTag.Substring(1)
}
if ($versionTag -notmatch '^[0-9]+(\.[0-9]+){0,2}([\-+][0-9A-Za-z.\-]+)?$') {
    throw "Version '$Version' is not a valid release identifier (e.g. v0.2.0)."
}

$folderName = "codexplus-v$versionTag-windows-x64"
$stagingDir = Join-Path $OutputDir $folderName
$zipPath = Join-Path $OutputDir "$folderName.zip"

if (Test-Path -LiteralPath $stagingDir) {
    Remove-Item -LiteralPath $stagingDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null

Copy-Item -LiteralPath $ExePath -Destination (Join-Path $stagingDir "codexplus.exe") -Force
Copy-Item -LiteralPath $ReadmePath -Destination (Join-Path $stagingDir "README.md") -Force

if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory(
    $stagingDir,
    $zipPath,
    [System.IO.Compression.CompressionLevel]::Optimal,
    $false
)

$exeSize = (Get-Item -LiteralPath (Join-Path $stagingDir "codexplus.exe")).Length
$zipSize = (Get-Item -LiteralPath $zipPath).Length

Write-Host ""
Write-Host "Release artifacts ready:"
Write-Host "  Folder : $stagingDir"
Write-Host "  Zip    : $zipPath"
Write-Host "  EXE    : $exeSize bytes"
Write-Host "  ZIP    : $zipSize bytes"
Write-Host ""
Write-Host "Contents:"
Get-ChildItem -LiteralPath $stagingDir |
    Sort-Object Name |
    Select-Object Name, Length |
    Format-Table -AutoSize