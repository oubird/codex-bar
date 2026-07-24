#Requires -Version 5.1
<#
.SYNOPSIS
    Build the CodexPlus release executable and copy it as the only file
    under the project root's release/ directory.

.DESCRIPTION
    This script owns a single responsibility: produce the per-commit
    distributable for CodexPlus on Windows x64. It is intentionally simple.

    * When -SkipBuild is not supplied, `cargo build --release --locked`
      runs first so the artifact always matches the current source tree.
    * The resulting target/release/codexplus.exe is copied into the fixed
      <repo>/release/ directory as codexplus.exe. The directory itself is
      recreated every run, so its previous contents are always overwritten.
    * The script does not zip anything, does not embed README.md in the
      archive, and does not expose an OutputDir / Version parameter. Git
      tag or Release title carries the version number; only the EXE is
      uploaded to GitHub / Gitee Release assets.

.PARAMETER SkipBuild
    Skip `cargo build --release --locked`. Use only when the calling
    pipeline has already produced a known-good target/release/codexplus.exe.

.EXAMPLE
    .\scripts\package-release.ps1

.EXAMPLE
    .\scripts\package-release.ps1 -SkipBuild
#>

param([switch]$SkipBuild)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$SourceExe = Join-Path $RepoRoot "target\release\codexplus.exe"
$ReleaseDir = Join-Path $RepoRoot "release"
$ReleaseExe = Join-Path $ReleaseDir "codexplus.exe"

# ----------------------------------------------------------------------------
# Build helpers
# ----------------------------------------------------------------------------

function Require-File {
    param(
        [string]$Path,
        [string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "$Label not found: $Path"
    }
}

function Get-Sha256 {
    param([string]$Path)
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Invoke-CargoBuild {
    param([string]$RepoRoot)

    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        throw "cargo is required to build the release executable."
    }

    $buildLog = Join-Path $env:TEMP "codexplus-release-build.log"
    if (Test-Path -LiteralPath $buildLog) { Remove-Item -LiteralPath $buildLog -Force }
    Write-Host "Running cargo build --release --locked. Log: $buildLog"

    $previousEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $proc = Start-Process -FilePath $cargo.Source `
            -ArgumentList @("build", "--release", "--locked") `
            -WorkingDirectory $RepoRoot `
            -NoNewWindow -PassThru `
            -RedirectStandardOutput $buildLog `
            -Wait
    } finally {
        $ErrorActionPreference = $previousEAP
    }

    $exitCode = $proc.ExitCode
    if ($null -eq $exitCode) {
        $exitCode = -1
    }

    if ($exitCode -ne 0) {
        Write-Host "cargo build --release --locked failed with exit code $exitCode."
        Write-Host "Last 200 log lines:"
        if (Test-Path -LiteralPath $buildLog) {
            Get-Content -LiteralPath $buildLog -Tail 200
        }
        throw "cargo build --release --locked exited with code $exitCode"
    }
}

# ----------------------------------------------------------------------------
# Main
# ----------------------------------------------------------------------------

if (-not $SkipBuild) {
    Invoke-CargoBuild -RepoRoot $RepoRoot
}

Require-File -Path $SourceExe -Label "Release executable"

if (Test-Path -LiteralPath $ReleaseDir) {
    Remove-Item -LiteralPath $ReleaseDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
Copy-Item -LiteralPath $SourceExe -Destination $ReleaseExe -Force

Require-File -Path $ReleaseExe -Label "Published executable"

$size = (Get-Item -LiteralPath $ReleaseExe).Length
$hash = Get-Sha256 -Path $ReleaseExe

Write-Host ""
Write-Host "Release executable ready:"
Write-Host "  Path  : $ReleaseExe"
Write-Host "  Size  : $size bytes"
Write-Host "  SHA256: $hash"