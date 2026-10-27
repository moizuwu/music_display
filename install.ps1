<#
.SYNOPSIS
  Builds music_display from source on Windows and installs music_display.exe.

.DESCRIPTION
  Checks for git and Rust, clones the source, runs `cargo build --release --locked`
  and copies music_display.exe (and a short copy named mdis.exe) to a folder on your
  user PATH. The Linux counterpart is install.sh.

    irm https://raw.githubusercontent.com/moizuwu/music_display/main/install.ps1 | iex

  or download it, read it, then run:  .\install.ps1 -DryRun

.PARAMETER DryRun   Print the steps, change nothing.
.PARAMETER Ref      Install this branch or tag instead of main.
.PARAMETER Force    Rebuild even when already up to date.
.PARAMETER Prefix   Install folder (default %LOCALAPPDATA%\Programs\music_display).
.PARAMETER Portable Only build and copy music_display.exe into this folder: no PATH change, no mdis.exe.
.PARAMETER NoDeps   Install nothing (no rustup), only check that the tools are there.
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [string]$Ref = "main",
    [switch]$Force,
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA "Programs\music_display"),
    [string]$Portable,
    [switch]$NoDeps
)

$ErrorActionPreference = "Stop"
$RepoUrl = if ($env:MDIS_REPO_URL) { $env:MDIS_REPO_URL } else { "https://github.com/moizuwu/music_display" }
$MinRust = [version]"1.90"
$SrcDir = Join-Path $env:LOCALAPPDATA "music_display\src"

function Say($msg) { Write-Host "==> $msg" }
function Die($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

# Run (or just print, with -DryRun) a command; a failing exit code stops the script.
function Run([string]$exe, [string[]]$arguments) {
    if ($DryRun) { Write-Host "    would run: $exe $($arguments -join ' ')"; return }
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { Die "'$exe $($arguments -join ' ')' failed (exit code $LASTEXITCODE)" }
}

function Have($name) { [bool](Get-Command $name -ErrorAction SilentlyContinue) }

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

# --- git -------------------------------------------------------------------
if (-not (Have "git")) {
    if ($NoDeps -or -not (Have "winget")) {
        Die "git is required. Install it from https://git-scm.com/download/win and rerun."
    }
    Say "git is missing; installing it with winget"
    Run "winget" @("install", "--id", "Git.Git", "-e", "--source", "winget")
    if (-not $DryRun) { Die "git was installed; open a new terminal so it is on PATH, then rerun this script." }
}

# --- Rust (and the MSVC linker it needs) ------------------------------------
if (-not (RustOk)) {
    $found = RustVersion
    Say "Rust $MinRust or newer is needed (found: $(if ($found) { $found } else { 'none' }))."
    if ($NoDeps) { Die "Rust $MinRust or newer is required. Install it from https://rustup.rs and rerun." }
    if (-not $DryRun -and -not [Environment]::UserInteractive) { Die "Rust is missing; install it from https://rustup.rs and rerun." }
    if (Have "rustup") {
        Run "rustup" @("update", "stable")
    } elseif (Have "winget") {
        Run "winget" @("install", "--id", "Rustlang.Rustup", "-e", "--source", "winget")
    } else {
        Die "Install Rust from https://rustup.rs and rerun."
    }
    if (-not $DryRun) {
        # rustup puts cargo in %USERPROFILE%\.cargo\bin; this session may not know yet
        $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
        if (-not (RustOk)) { Die "Rust finished installing but $MinRust or newer is still not found. Open a new terminal and rerun." }
    }
}
if (-not $DryRun -and -not (Have "link.exe") -and -not (Test-Path "$env:ProgramFiles (x86)\Microsoft Visual Studio\Installer\vswhere.exe")) {
    Write-Host "warning: the MSVC build tools were not found. If the build fails, install them:" -ForegroundColor Yellow
    Write-Host "         winget install Microsoft.VisualStudio.2022.BuildTools --override `"--add Microsoft.VisualStudio.Workload.VCTools`""
}

# --- source -------------------------------------------------------------------
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

# --- build -----------------------------------------------------------------------
$exe = Join-Path $SrcDir "target\release\music_display.exe"
$after = if ($DryRun) { $null } else { (& git -C $SrcDir rev-parse HEAD) }
if ($Force -or $before -ne $after -or -not (Test-Path $exe)) {
    Say "Building (this takes a few minutes the first time)"
    Push-Location $(if ($DryRun) { "." } else { $SrcDir })
    try { Run "cargo" @("build", "--release", "--locked") } finally { Pop-Location }
} else {
    Say "Already up to date"
}

# --- install -------------------------------------------------------------------------
$dest = if ($Portable) { $Portable } else { $Prefix }
Say "Installing into $dest"
if (-not $DryRun) {
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item $exe (Join-Path $dest "music_display.exe") -Force
    if (-not $Portable) { Copy-Item $exe (Join-Path $dest "mdis.exe") -Force }
} else {
    Write-Host "    would copy $exe to $dest"
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
