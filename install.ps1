<#
.SYNOPSIS
  Installs music_display.exe on Windows (prebuilt release, or built from source as a fallback).

.DESCRIPTION
  Downloads the latest prebuilt music_display.exe from GitHub Releases and copies it
  (plus a short copy named mdis.exe) to a folder on your user PATH. If the download
  fails, or with -FromSource, it builds from source instead, which needs git, Rust
  and the MSVC C++ build tools. The Linux counterpart is install.sh.

    irm https://raw.githubusercontent.com/moizuwu/music_display/main/install.ps1 | iex

  or download it, read it, then run:  .\install.ps1 -DryRun

.PARAMETER DryRun     Print the steps, change nothing.
.PARAMETER Ref        Install this release tag (or build this branch/tag) instead of the latest.
.PARAMETER Force      Rebuild even when already up to date (source builds only).
.PARAMETER Prefix     Install folder (default %LOCALAPPDATA%\Programs\music_display).
.PARAMETER Portable   Only copy music_display.exe into this folder: no PATH change, no mdis.exe.
.PARAMETER NoDeps     Install nothing (no rustup), only check that the tools are there (source builds).
.PARAMETER FromSource Skip the prebuilt download and build from source.
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [string]$Ref = "main",
    [switch]$Force,
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA "Programs\music_display"),
    [string]$Portable,
    [switch]$NoDeps,
    [switch]$FromSource
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # the progress bar makes Invoke-WebRequest very slow on Windows PowerShell 5.1
$RepoUrl = if ($env:MDIS_REPO_URL) { $env:MDIS_REPO_URL } else { "https://github.com/moizuwu/music_display" }
$Asset = "music_display.exe"               # file name of the release asset
$MinRust = [version]"1.90"
$SrcDir = Join-Path $env:LOCALAPPDATA "music_display\src"
$VcComponent = "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
$script:Exe = $null                        # path of the exe to install, set by Download or Build

function Say($msg) { Write-Host "==> $msg" }
# throw (not exit) so that running through `irm | iex` doesn't close the user's terminal
function Die($msg) { Write-Host "error: $msg" -ForegroundColor Red; throw $msg }

# Run (or just print, with -DryRun) a command; a failing exit code stops the script.
function Run([string]$exe, [string[]]$arguments) {
    if ($DryRun) { Write-Host "    would run: $exe $($arguments -join ' ')"; return }
    & $exe @arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { Die "'$exe $($arguments -join ' ')' failed (exit code $LASTEXITCODE)" }
}

function Have($name) { [bool](Get-Command $name -ErrorAction SilentlyContinue) }

function RefreshPath {
    $machine = [Environment]::ExpandEnvironmentVariables([Environment]::GetEnvironmentVariable("Path", "Machine"))
    $user = [Environment]::ExpandEnvironmentVariables([Environment]::GetEnvironmentVariable("Path", "User"))
    $env:PATH = "$machine;$user;$env:USERPROFILE\.cargo\bin"
}

function RustVersion {
    if (-not (Have "rustc")) { return $null }
    $m = [regex]::Match((& rustc --version), '(\d+\.\d+)')
    if ($m.Success) { return [version]$m.Value }
    return $null
}

function RustOk {
    $v = RustVersion
    return ($null -ne $v -and $v -ge $MinRust)
}

function HasMsvc {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { return $false }
    return [bool](& $vswhere -latest -products * -requires $VcComponent -property installationPath)
}

# --- prebuilt download ---------------------------------------------------------
# Returns $true and sets $script:Exe when a valid exe was downloaded.
function Download {
    $url = if ($Ref -eq "main") { "$RepoUrl/releases/latest/download/$Asset" }
           else { "$RepoUrl/releases/download/$Ref/$Asset" }
    Say "Downloading the prebuilt $Asset"
    if ($DryRun) { Write-Host "    would download $url"; $script:Exe = "<downloaded $Asset>"; return $true }
    $tmp = Join-Path ([IO.Path]::GetTempPath()) "music_display-$([guid]::NewGuid().ToString('N')).exe"
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $tmp
        # sanity check: a real Windows executable is large and starts with "MZ"
        $head = [IO.File]::ReadAllBytes($tmp) | Select-Object -First 2
        if ((Get-Item $tmp).Length -lt 100KB -or $head[0] -ne 0x4D -or $head[1] -ne 0x5A) {
            throw "the downloaded file is not a valid Windows executable"
        }
        $script:Exe = $tmp
        return $true
    } catch {
        Write-Host "    download failed: $($_.Exception.Message)" -ForegroundColor Yellow
        Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        return $false
    }
}

# --- build from source -----------------------------------------------------------
# Sets $script:Exe to the built exe.
function Build {
    # git
    if (-not (Have "git")) {
        if ($NoDeps -or -not (Have "winget")) {
            Die "git is required to build from source. Install it from https://git-scm.com/download/win and rerun."
        }
        Say "git is missing; installing it with winget"
        Run "winget" @("install", "--id", "Git.Git", "-e", "--source", "winget",
                       "--accept-package-agreements", "--accept-source-agreements")
        if (-not $DryRun) {
            RefreshPath
            if (-not (Have "git")) { Die "git was installed but is not on PATH yet. Open a new terminal and rerun this script." }
        }
    }

    # MSVC C++ build tools (the linker Rust needs on Windows)
    if (-not $DryRun -and -not (HasMsvc)) {
        Die ("The MSVC C++ build tools were not found, and the build would fail without them.`n" +
             "  If 'Build Tools for Visual Studio' is already installed: open Visual Studio Installer, click Modify, tick 'Desktop development with C++'.`n" +
             "  Otherwise: winget install Microsoft.VisualStudio.2022.BuildTools --override `"--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive`"`n" +
             "  Then open a new terminal and rerun. (Or skip all this: rerun without -FromSource to use the prebuilt exe.)")
    }

    # Rust
    if (-not (RustOk)) {
        $found = RustVersion
        Say "Rust $MinRust or newer is needed (found: $(if ($found) { $found } else { 'none' }))."
        if ($NoDeps) { Die "Rust $MinRust or newer is required. Install it from https://rustup.rs and rerun." }
        if (-not $DryRun -and -not [Environment]::UserInteractive) { Die "Rust is missing; install it from https://rustup.rs and rerun." }
        if (Have "rustup") {
            Run "rustup" @("update", "stable")
        } elseif (Have "winget") {
            Run "winget" @("install", "--id", "Rustlang.Rustup", "-e", "--source", "winget",
                           "--accept-package-agreements", "--accept-source-agreements")
        } else {
            Die "Install Rust from https://rustup.rs and rerun."
        }
        if (-not $DryRun) {
            RefreshPath
            if (Have "rustup") { Run "rustup" @("default", "stable") }
            if (-not (RustOk)) { Die "Rust finished installing but $MinRust or newer is still not found. Open a new terminal and rerun." }
        }
    }

    # source
    $before = $null
    if (Test-Path (Join-Path $SrcDir ".git")) {
        Say "Updating $SrcDir"
        $before = (& git -C $SrcDir rev-parse HEAD)
        if (-not $DryRun -and (& git -C $SrcDir status --porcelain)) {
            Die "$SrcDir has local changes; leaving it alone."
        }
        Run "git" @("-C", $SrcDir, "fetch", "--depth", "1", "origin", $Ref)
        Run "git" @("-C", $SrcDir, "merge", "--ff-only", "FETCH_HEAD")
    } else {
        Say "Cloning $RepoUrl ($Ref) into $SrcDir"
        if (-not $DryRun) { New-Item -ItemType Directory -Force (Split-Path $SrcDir) | Out-Null }
        Run "git" @("clone", "--depth", "1", "--branch", $Ref, $RepoUrl, $SrcDir)
    }

    # build
    $exe = Join-Path $SrcDir "target\release\music_display.exe"
    $after = if ($DryRun) { $null } else { (& git -C $SrcDir rev-parse HEAD) }
    if ($Force -or $before -ne $after -or -not (Test-Path $exe)) {
        Say "Building (this takes a few minutes the first time)"
        Push-Location $(if ($DryRun) { "." } else { $SrcDir })
        try { Run "cargo" @("build", "--release", "--locked") } finally { Pop-Location }
    } else {
        Say "Already up to date"
    }
    $script:Exe = $exe
}

# --- get the exe: prebuilt first, source as fallback ------------------------------
if ($FromSource) {
    Build
} elseif (-not (Download)) {
    Say "Falling back to building from source"
    Build
}

# --- install -------------------------------------------------------------------------
$dest = if ($Portable) { $Portable } else { $Prefix }
Say "Installing into $dest"
if (-not $DryRun) {
    New-Item -ItemType Directory -Force $dest | Out-Null
    try {
        Copy-Item $script:Exe (Join-Path $dest "music_display.exe") -Force
        if (-not $Portable) { Copy-Item $script:Exe (Join-Path $dest "mdis.exe") -Force }
    } catch {
        Die "Could not copy the exe into $dest. If music_display is running, close it and rerun. ($($_.Exception.Message))"
    }
    if ($script:Exe -like "$([IO.Path]::GetTempPath())*") { Remove-Item $script:Exe -Force -ErrorAction SilentlyContinue }
} else {
    Write-Host "    would copy $script:Exe to $dest"
}

if (-not $Portable) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $onPath = ($userPath -split ';') -contains $dest
    if (-not $onPath) {
        Say "Adding $dest to your user PATH"
        if (-not $DryRun) { [Environment]::SetEnvironmentVariable("Path", "$userPath;$dest", "User") }
        Write-Host "    open a new terminal for the change to take effect"
    }
}

Say "Done. Run: mdis   (a media player must be playing; Windows Terminal is recommended)"
