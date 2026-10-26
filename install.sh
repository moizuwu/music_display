#!/usr/bin/env bash
# music_display installer: checks what is missing, installs it (asking first), builds the
# program from source and links `music_display` and `mdis` into ~/.local/bin.
#
#   curl -fsSL https://raw.githubusercontent.com/moizuwu/music_display/main/install.sh | bash
#   (or download it, read it, run it:  bash install.sh --dry-run)
#
# Options: --dry-run  --yes  --no-deps  --ref <branch|tag>  --force  --aur  --help
set -euo pipefail

REPO_URL="${MDIS_REPO_URL:-https://github.com/moizuwu/music_display}"
MIN_RUST="1.90"   # what the locked dependencies need (checked against Cargo.lock)
SRC_DIR="$HOME/.local/share/music_display/src"
BIN_DIR="$HOME/.local/bin"
OS_RELEASE="${MDIS_OS_RELEASE:-/etc/os-release}"

DRY=0 YES=0 NODEPS=0 FORCE=0 AUR=0 REF=main
PM=""
PRIV=()

say() { printf '%s\n' "$*"; }
warn() { printf 'install.sh: %s\n' "$*" >&2; }
die() { warn "$*"; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# Run a command, or only show it with --dry-run.
run() {
    if [ "$DRY" = 1 ]; then say "would run: $*"; else "$@"; fi
}

usage() {
    cat <<'EOF'
Usage: install.sh [options]

  --dry-run     show what would be done, change nothing
  --yes         answer yes to the questions (AUR builds still need --aur)
  --no-deps     don't install packages or Rust, only check that they are there
  --ref REF     branch or tag to install (default: main)
  --force       rebuild even when the checkout is already up to date
  --aur         allow building a missing dependency from the AUR (Arch only)
  --help        this text

Installs into ~/.local/share/music_display/src and ~/.local/bin (music_display, mdis).
EOF
}

# A yes/no question. Reads the terminal (stdin is the script when piped from curl); without
# one the answer is no. --yes answers yes; --dry-run assumes yes to show the whole plan.
ask() {
    if [ "$YES" = 1 ]; then say "$1 [y/N] y (--yes)"; return 0; fi
    ask_strict "$1"
}

# Like ask, but --yes does not count.
ask_strict() {
    local a=""
    if [ "$DRY" = 1 ]; then say "$1 [y/N] (dry run: would ask)"; return 0; fi
    if [ "${MDIS_NO_TTY:-0}" != 1 ] && { : </dev/tty; } 2>/dev/null; then
        read -r -p "$1 [y/N] " a </dev/tty || a=""
    else
        say "$1 -> no terminal to ask on, assuming no"
        return 1
    fi
    case "$a" in y | Y | yes | YES) return 0 ;; *) return 1 ;; esac
}

# version_ge A B: A >= B
version_ge() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}

rust_version() {
    have rustc || return 1
    rustc --version | awk '{print $2}' | sed 's/-.*//'
}

rust_ok() {
    have cargo || return 1
    local v
    v=$(rust_version) || return 1
    version_ge "$v" "$MIN_RUST"
}

have_cc() { have cc || have gcc || have clang; }

detect_pm() {
    local id like
    [ -r "$OS_RELEASE" ] || return 1
    id=$(. "$OS_RELEASE"; printf '%s' "${ID:-}")
    like=$(. "$OS_RELEASE"; printf '%s' "${ID_LIKE:-}")
    case " $id $like " in
        *" arch "* | *" artix "* | *" manjaro "* | *" cachyos "* | *" endeavouros "*) PM=pacman ;;
        *" debian "* | *" ubuntu "*) PM=apt-get ;;
        *" fedora "* | *" rhel "* | *" centos "*) PM=dnf ;;
        *" opensuse "* | *" suse "* | *" opensuse-leap "* | *" opensuse-tumbleweed "*) PM=zypper ;;
        *) return 1 ;;
    esac
    have "$PM"
}

manual_list() {
    cat <<EOF
Install these yourself, then rerun with --no-deps:
  - a C compiler and linker (gcc or clang)
  - git, curl
  - playerctl
  - Rust $MIN_RUST or newer (https://rustup.rs)
  - optional: cava (the visualiser)
EOF
}

# The package(s) a tool comes in on this system.
pkgs_for() {
    case "$PM:$1" in
        apt-get:cc) echo build-essential ;;
        *:cc) echo gcc ;;
        apt-get:rust) echo rustc cargo ;;
        dnf:rust | zypper:rust) echo rust cargo ;;
        *:rust) echo rust ;;
        *) echo "$1" ;;
    esac
}

# Root for installing packages: nothing when root, else sudo, else doas.
find_priv() {
    PRIV=()
    [ "$(id -u)" = 0 ] && return 0
    if have sudo; then PRIV=(sudo)
    elif have doas; then PRIV=(doas)
    else return 1
    fi
}

priv_or_die() {
    find_priv || die "installing packages needs root: run as root, or install sudo or doas. Command: $*"
}

install_with_pm() {
    priv_or_die "$PM install $*"
    case "$PM" in
        pacman) run ${PRIV[@]+"${PRIV[@]}"} pacman -S --needed --noconfirm "$@" ;;
        apt-get) run ${PRIV[@]+"${PRIV[@]}"} apt-get install -y "$@" ;;
        dnf) run ${PRIV[@]+"${PRIV[@]}"} dnf install -y "$@" ;;
        zypper) run ${PRIV[@]+"${PRIV[@]}"} zypper --non-interactive install "$@" ;;
    esac
}

# A package the official repos don't have: built from the AUR with yay / paru, but ONLY when
# --aur is given (never offered as a question, never implied by --yes).
aur_install() {
    local p="$1" helper="" h
    if [ "$AUR" != 1 ]; then
        warn "$p is not in the official repos. It would come from the AUR: a package built on this machine from a PKGBUILD written by a third party. Not building it: pass --aur to allow that, or install it yourself (https://aur.archlinux.org/packages/$p)."
        return 1
    fi
    for h in yay paru; do
        if have "$h"; then helper="$h"; break; fi
    done
    if [ -z "$helper" ]; then
        warn "$p is not in the official repos and neither yay nor paru is installed. Get it yourself, e.g. from https://aur.archlinux.org/packages/$p"
        return 1
    fi
    say "$p is not in the official repos: building it from the AUR with $helper (--aur given), a PKGBUILD written by a third party."
    local extra=()
    # no sudo on this machine: tell the helper to use doas
    if [ "$(id -u)" != 0 ] && ! have sudo && have doas; then extra=(--sudo doas); fi
    run "$helper" ${extra[@]+"${extra[@]}"} -S --needed "$p"
}

install_pacman() {
    local repo=() aurs=() p
    for p in "$@"; do
        if pacman -Si "$p" >/dev/null 2>&1; then
            repo+=("$p")
        elif ! pacman -Si pacman >/dev/null 2>&1; then
            die "pacman's package databases are missing. Run a full system upgrade first (sudo pacman -Syu), then rerun. Nothing was installed."
        else
            aurs+=("$p")
        fi
    done
    if [ ${#repo[@]} -gt 0 ]; then
        priv_or_die "pacman -S --needed ${repo[*]}"
        ask "Install ${repo[*]} with ${PRIV[*]:-root} pacman -S --needed?" || die "declined: ${repo[*]} are needed"
        install_with_pm "${repo[@]}"
    fi
    for p in ${aurs[@]+"${aurs[@]}"}; do
        aur_install "$p" || true
    done
}

install_deps() {
    local need=() cava=0 t
    have_cc || need+=("$(pkgs_for cc)")
    for t in git curl playerctl; do
        have "$t" || need+=("$(pkgs_for "$t")")
    done
    # Rust: install the distro's only when there is none; an old one goes to rustup below
    # shellcheck disable=SC2207
    if ! have cargo || ! have rustc; then need+=($(pkgs_for rust)); fi
    have cava || cava=1
    if [ ${#need[@]} -gt 0 ]; then
        say "Missing: ${need[*]}"
        if [ "$PM" = pacman ]; then install_pacman "${need[@]}"; else
            priv_or_die "$PM install ${need[*]}"
            ask "Install ${need[*]} with ${PRIV[*]:-root} $PM?" || die "declined: ${need[*]} are needed"
            install_with_pm "${need[@]}"
        fi
    fi
    if [ "$cava" = 1 ] && ask "cava (the optional visualiser) is not installed. Install it?"; then
        if [ "$PM" = pacman ]; then install_pacman cava || true; else
            install_with_pm cava || warn "cava could not be installed (optional, skipped)"
        fi
    fi
}

install_rustup() {
    local v
    v=$(rust_version 2>/dev/null || echo "none")
    say "Rust $MIN_RUST or newer is needed (found: $v)."
    ask "Install it with rustup into your home directory (minimal profile, shell files untouched)?" \
        || die "Rust $MIN_RUST or newer is required. Install it (https://rustup.rs) and rerun."
    if [ "$DRY" = 1 ]; then
        say "would run: curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path"
        return 0
    fi
    have curl || die "curl is needed to fetch rustup"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
    PATH="$HOME/.cargo/bin:$PATH"
    hash -r
    rust_ok || die "rustup finished but Rust $MIN_RUST or newer is still not found"
}

# Clone, or bring the checkout up to date. Sets UPTODATE=1 when there is nothing to build.
UPTODATE=0
checkout() {
    if [ ! -e "$SRC_DIR" ]; then
        run mkdir -p "$(dirname "$SRC_DIR")"
        run git clone --depth 1 --branch "$REF" "$REPO_URL" "$SRC_DIR"
        return 0
    fi
    [ -d "$SRC_DIR/.git" ] || die "$SRC_DIR exists but is not a git checkout. Move it away and rerun; nothing was changed."
    if [ -n "$(git -C "$SRC_DIR" status --porcelain)" ]; then
        die "$SRC_DIR has local changes. Commit or move them and rerun; nothing was changed."
    fi
    if [ "$DRY" = 1 ]; then
        say "would run: git -C $SRC_DIR fetch origin $REF, then fast-forward if it changed"
        return 0
    fi
    git -C "$SRC_DIR" fetch origin "$REF"
    local head new
    head=$(git -C "$SRC_DIR" rev-parse HEAD)
    new=$(git -C "$SRC_DIR" rev-parse FETCH_HEAD)
    if [ "$head" = "$new" ]; then
        if [ "$FORCE" = 0 ] && [ -x "$SRC_DIR/target/release/music_display" ]; then
            UPTODATE=1
        fi
        return 0
    fi
    git -C "$SRC_DIR" merge --ff-only FETCH_HEAD \
        || die "cannot fast-forward $SRC_DIR to $REF. Nothing was reset or deleted; fix the checkout by hand and rerun."
}

build() {
    if [ "$DRY" = 1 ]; then
        say "would run (in $SRC_DIR): cargo build --release --locked"
        return 0
    fi
    (cd "$SRC_DIR" && cargo build --release --locked)
}

# ~/.local/bin/<name> -> the built binary; never replaces a regular file or someone else's link.
link_bin() {
    local bin="$SRC_DIR/target/release/music_display" name t
    run mkdir -p "$BIN_DIR"
    for name in music_display mdis; do
        t="$BIN_DIR/$name"
        if [ -L "$t" ] && [ "$(readlink "$t")" = "$bin" ]; then
            say "$t already points at the build"
        elif [ -e "$t" ] || [ -L "$t" ]; then
            warn "$t already exists and is not ours: left alone (skipped)"
        else
            run ln -s "$bin" "$t"
        fi
    done
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) say "Note: $BIN_DIR is not on your PATH. Add it, e.g. in your shell's rc file: export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
    esac
}

main() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --dry-run) DRY=1 ;;
            --yes | -y) YES=1 ;;
            --no-deps) NODEPS=1 ;;
            --force) FORCE=1 ;;
            --aur) AUR=1 ;;
            --ref) [ $# -ge 2 ] || die "--ref needs a branch or tag"; REF="$2"; shift ;;
            --ref=*) REF="${1#--ref=}" ;;
            -h | --help) usage; exit 0 ;;
            *) usage >&2; die "unknown option: $1" ;;
        esac
        shift
    done
    if [ "$DRY" = 1 ]; then say "Dry run: nothing will be changed."; fi

    local missing=() t
    have_cc || missing+=("a C compiler")
    for t in git curl playerctl; do
        have "$t" || missing+=("$t")
    done

    if [ "$NODEPS" = 1 ]; then
        rust_ok || missing+=("Rust $MIN_RUST or newer")
        [ ${#missing[@]} -eq 0 ] || die "missing (--no-deps): ${missing[*]}"
    else
        if ! detect_pm; then
            if [ ${#missing[@]} -eq 0 ] && rust_ok; then
                : # everything is there already; only the distro is unknown
            else
                warn "unsupported system (no pacman, apt, dnf or zypper found for this distro). Nothing was changed."
                manual_list >&2
                exit 1
            fi
        else
            install_deps
        fi
        if ! rust_ok; then install_rustup; fi
        if [ "$DRY" = 0 ]; then
            have git || die "git is still missing"
            have_cc || die "a C compiler is still missing"
            have playerctl || warn "playerctl is not installed: music_display needs it to read and control players. Install it later."
        fi
    fi

    checkout
    if [ "$UPTODATE" = 1 ]; then
        say "music_display ($REF) is already up to date. Use --force to rebuild."
        exit 0
    fi
    build
    link_bin
    if [ "$DRY" = 0 ]; then
        say "Installed: $("$SRC_DIR/target/release/music_display" --version)"
    fi
}

main "$@"
