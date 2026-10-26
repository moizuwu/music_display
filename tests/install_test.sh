#!/usr/bin/env bash
# Tests install.sh without touching the real system: fake pacman / apt-get / dnf / zypper / sudo /
# doas / yay / paru / git / curl / cargo / rustc in an isolated PATH, a scratch HOME and a
# scratch os-release. Only a short list of real utilities (awk, sed, sort...) is on that PATH.
#
#   bash tests/install_test.sh
set -uo pipefail

HERE=$(cd "$(dirname "$0")/.." && pwd)
INSTALL="$HERE/install.sh"
BASH_BIN=$(command -v bash)
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT
LIB="$ROOT/lib"
REAL="$ROOT/real"
mkdir -p "$LIB" "$REAL"

[ "$(id -u)" != 0 ] || { echo "run as a normal user (the root paths use sudo / doas)"; exit 2; }

# The only real programs the script under test can see.
for t in awk sed sort head tail id readlink ln mkdir dirname basename cat sh cp chmod touch grep tr rm mv ls date sleep wc env tee cut; do
    p=$(command -v "$t") || { echo "missing real tool: $t"; exit 2; }
    ln -s "$p" "$REAL/$t"
done

# ── fake programs (templates; "installing" a package copies one into the fake bin) ──────────
mkfake() { printf '%s\n' "#!/bin/bash" "$2" > "$LIB/$1"; chmod +x "$LIB/$1"; }
for n in cc playerctl cava; do mkfake "$n" 'echo "$(basename "$0") $*" >> "$LOG/tools.log"'; done
mkfake rustc 'v=$(cat "$0.ver"); echo "rustc $v (fake)"'
mkfake cargo 'echo "cargo $*" >> "$LOG/tools.log"
case "$1" in
  --version) echo "cargo 1.99.0" ;;
  build) mkdir -p target/release
         printf "#!/bin/sh\necho \"music_display %s\"\n" "$(cat .git/rev)" > target/release/music_display
         chmod +x target/release/music_display ;;
esac'
mkfake git 'echo "git $*" >> "$LOG/tools.log"
dir=""; if [ "$1" = -C ]; then dir=$2; shift 2; fi
case "$1" in
  clone) ref=""; last=""; prev=""
         for a in "$@"; do [ "$prev" = --branch ] && ref=$a; prev=$a; last=$a; done
         mkdir -p "$last/.git"; cp "$REMOTE/rev" "$last/.git/rev"; echo "$ref" > "$last/.git/ref" ;;
  status) [ -e "$dir/DIRTY" ] && echo " M file"; exit 0 ;;
  fetch) cp "$REMOTE/rev" "$dir/.git/fetch_head" ;;
  rev-parse) case "$2" in HEAD) cat "$dir/.git/rev" ;; FETCH_HEAD) cat "$dir/.git/fetch_head" ;; esac ;;
  merge) [ -e "$REMOTE/nonff" ] && exit 1; cp "$dir/.git/fetch_head" "$dir/.git/rev" ;;
esac'
mkfake curl 'echo "curl $*" >> "$LOG/tools.log"
case "$*" in
  *sh.rustup.rs*) cat <<"EOS"
FBIN="$HOME/.cargo/bin"; . "$LIB/provide.sh"; mkdir -p "$FBIN"; mk cargo; mk rustc 1.99.0
EOS
  ;;
esac'
cat > "$LIB/provide.sh" <<'EOF'
mk() { cp "$LIB/$1" "$FBIN/$1"; chmod +x "$FBIN/$1"; if [ -n "${2:-}" ]; then echo "$2" > "$FBIN/$1.ver"; fi; return 0; }
provide() {
    for p in "$@"; do
        case "$p" in
            gcc | build-essential) mk cc ;;
            git | curl | playerctl | cava) mk "$p" ;;
            rust | rustc | cargo) mk cargo; mk rustc "${FAKE_PKG_RUSTC:-1.99.0}" ;;
        esac
    done
    return 0
}
EOF
mkfake pacman '. "$LIB/provide.sh"
case "$1" in
  -Si) [ -n "${FAKE_NODB:-}" ] && exit 1; shift
       for p in "$@"; do case ",${FAKE_NOT_IN_REPOS:-}," in *",$p,"*) exit 1 ;; esac; done; exit 0 ;;
  -S) echo "pacman $*" >> "$LOG/pm.log"; shift; pk=()
      for a in "$@"; do case "$a" in -*) ;; *) pk+=("$a") ;; esac; done; provide "${pk[@]}" ;;
esac'
mkfake pm '. "$LIB/provide.sh"
echo "$(basename "$0") $*" >> "$LOG/pm.log"; pk=()
for a in "$@"; do case "$a" in -* | install) ;; *) pk+=("$a") ;; esac; done; provide "${pk[@]}"'
mkfake priv 'echo "$(basename "$0") $*" >> "$LOG/priv.log"; exec "$@"'
mkfake aur '. "$LIB/provide.sh"
echo "$(basename "$0") $*" >> "$LOG/aur.log"; eval "last=\${$#}"; provide "$last"'

# ── scenario helpers ────────────────────────────────────────────────────────────────────────
PASSES=0
FAILS=0
check() { # check "what" cmd args...
    local d=$1; shift
    if "$@" >/dev/null 2>&1; then PASSES=$((PASSES + 1)); else FAILS=$((FAILS + 1)); echo "  FAIL: $d"; fi
}
has() { grep -qF -- "$2" "$1" 2>/dev/null; }
hasnt() { ! has "$1" "$2"; }
count() { grep -cF -- "$2" "$1" 2>/dev/null || true; }
isfile() { [ -f "$1" ]; }
nofile() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
eq() { [ "$1" = "$2" ]; }
ne() { [ "$1" != "$2" ]; }

world() { # world NAME OS_ID [ID_LIKE] : a fresh fake machine; no tools installed yet
    echo "== $1"
    W="$ROOT/$1"; rm -rf "$W"
    mkdir -p "$W/home" "$W/bin" "$W/log" "$W/remote"
    echo rev1 > "$W/remote/rev"
    printf 'ID=%s\nID_LIKE="%s"\n' "$2" "${3:-}" > "$W/os-release"
    unset FAKE_NODB FAKE_NOT_IN_REPOS FAKE_PKG_RUSTC
}
give() { local n; for n in "$@"; do cp "$LIB/$n" "$W/bin/$n"; done; }
give_pm() { cp "$LIB/$1" "$W/bin/$2"; }
give_rust() { give cargo rustc; echo "$1" > "$W/bin/rustc.ver"; }
give_all() { give cc git curl playerctl cava; give_rust "${1:-1.99.0}"; }

inst() { # inst ARGS... : run install.sh in the fake machine; output in $W/out, status in $RC
    env -i HOME="$W/home" PATH="$W/bin:$REAL" LOG="$W/log" LIB="$LIB" FBIN="$W/bin" REMOTE="$W/remote" \
        MDIS_OS_RELEASE="$W/os-release" MDIS_NO_TTY=1 MDIS_REPO_URL=https://example.invalid/m.git \
        FAKE_NODB="${FAKE_NODB:-}" FAKE_NOT_IN_REPOS="${FAKE_NOT_IN_REPOS:-}" FAKE_PKG_RUSTC="${FAKE_PKG_RUSTC:-1.99.0}" \
        "$BASH_BIN" "$INSTALL" "$@" >"$W/out" 2>&1
    RC=$?
}
OUT() { echo "$W/out"; }
SRC() { echo "$W/home/.local/share/music_display/src"; }
BINLINK() { echo "$W/home/.local/bin/$1"; }
installed_ok() { # the build exists and both commands link to it
    check "$1: binary built" isfile "$(SRC)/target/release/music_display"
    check "$1: music_display linked" test -L "$(BINLINK music_display)"
    check "$1: mdis linked" test -L "$(BINLINK mdis)"
    check "$1: links point at the build" eq "$(readlink "$(BINLINK mdis)")" "$(SRC)/target/release/music_display"
}

bash -n "$INSTALL" || { echo "syntax error in install.sh"; exit 1; }
if command -v shellcheck >/dev/null 2>&1; then
    shellcheck "$INSTALL" && echo "shellcheck: clean" || FAILS=$((FAILS + 1))
else
    echo "(shellcheck is not installed: skipped)"
fi

# ── Arch: nothing installed, sudo ───────────────────────────────────────────────────────────
world arch_sudo arch
give_pm pacman pacman; give_pm priv sudo
inst --yes
check "arch: exit 0" eq "$RC" 0
check "arch: one pacman call, --needed, no sync (-y)" has "$W/log/pm.log" "pacman -S --needed --noconfirm gcc git curl playerctl rust"
check "arch: optional cava asked and installed" has "$W/log/pm.log" "pacman -S --needed --noconfirm cava"
check "arch: no -Sy / -y partial upgrade" hasnt "$W/log/pm.log" " -Sy"
check "arch: went through sudo" has "$W/log/priv.log" "sudo pacman -S"
check "arch: cloned shallow main" has "$W/log/tools.log" "git clone --depth 1 --branch main https://example.invalid/m.git"
check "arch: cargo build --release --locked" has "$W/log/tools.log" "cargo build --release --locked"
installed_ok arch
check "arch: prints the version" has "$(OUT)" "Installed: music_display rev1"
check "arch: PATH hint" has "$(OUT)" "not on your PATH"

# ── Arch with doas only ─────────────────────────────────────────────────────────────────────
world arch_doas arch
give_pm pacman pacman; give_pm priv doas
inst --yes
check "doas: exit 0" eq "$RC" 0
check "doas: used doas" has "$W/log/priv.log" "doas pacman -S"
check "doas: sudo never named" hasnt "$W/log/priv.log" "sudo"

# ── no sudo, no doas ────────────────────────────────────────────────────────────────────────
world no_priv arch
give_pm pacman pacman
inst --yes
check "no root helper: fails" ne "$RC" 0
check "no root helper: says so" has "$(OUT)" "needs root"
check "no root helper: nothing installed" nofile "$W/log/pm.log"

# ── Debian / Fedora / openSUSE ──────────────────────────────────────────────────────────────
world ubuntu ubuntu debian
give_pm pm apt-get; give_pm priv sudo
FAKE_PKG_RUSTC=1.75.0
inst --yes
check "apt: exit 0" eq "$RC" 0
check "apt: apt-get install -y with the right packages" has "$W/log/pm.log" "apt-get install -y build-essential git curl playerctl rustc cargo"
check "apt: old distro rust -> rustup offered and used" has "$W/log/tools.log" "curl --proto =https --tlsv1.2 -fsSL https://sh.rustup.rs"
installed_ok apt

world fedora fedora
give_pm pm dnf; give_pm priv sudo
inst --yes
check "dnf: install -y" has "$W/log/pm.log" "dnf install -y gcc git curl playerctl rust cargo"
installed_ok dnf

world suse opensuse-tumbleweed "suse opensuse"
give_pm pm zypper; give_pm priv sudo
inst --yes
check "zypper: non-interactive" has "$W/log/pm.log" "zypper --non-interactive install gcc git curl playerctl rust cargo"
installed_ok zypper

# ── unsupported distro ──────────────────────────────────────────────────────────────────────
world alpine alpine
inst --yes
check "unsupported: fails" ne "$RC" 0
check "unsupported: prints the manual list" has "$(OUT)" "playerctl"
check "unsupported: changed nothing" nofile "$W/home/.local"
world alpine_ok alpine
give_all
inst --no-deps
check "--no-deps with everything present works" eq "$RC" 0
installed_ok alpine_ok
world alpine_missing alpine
give cc git
inst --no-deps
check "--no-deps lists what is missing" has "$(OUT)" "missing (--no-deps)"
check "--no-deps changed nothing" nofile "$W/home/.local"

# ── rust too old ────────────────────────────────────────────────────────────────────────────
world old_rust arch
give_pm pacman pacman; give_pm priv sudo; give_all 1.75.0
inst
check "old rust, no yes: declined, fails" ne "$RC" 0
check "old rust, no yes: says what is needed" has "$(OUT)" "Rust 1.90 or newer is required"
check "old rust, no yes: nothing downloaded" hasnt "$W/log/tools.log" "sh.rustup.rs"
check "old rust, no yes: no checkout" nofile "$W/home/.local/share"
inst --yes
check "old rust, --yes: rustup, then builds" eq "$RC" 0
check "old rust, --yes: rustup is minimal and leaves shell files alone" has "$W/log/tools.log" "https://sh.rustup.rs"
installed_ok old_rust

# ── nothing missing: no package manager, no root ────────────────────────────────────────────
world complete arch
give_pm pacman pacman; give_pm priv sudo; give_all
inst
check "complete: exit 0" eq "$RC" 0
check "complete: package manager not called" nofile "$W/log/pm.log"
check "complete: root not used" nofile "$W/log/priv.log"

# ── re-run: up to date, update, force, dirty, no fast-forward ───────────────────────────────
inst
check "rerun: already up to date" has "$(OUT)" "already up to date"
check "rerun: built once" eq "$(count "$W/log/tools.log" "cargo build")" 1
inst --force
check "--force rebuilds" eq "$(count "$W/log/tools.log" "cargo build")" 2
echo rev2 > "$W/remote/rev"
inst
check "update: exit 0" eq "$RC" 0
check "update: fast-forwarded and rebuilt" eq "$(cat "$(SRC)/.git/rev")" rev2
check "update: new binary" has "$(OUT)" "Installed: music_display rev2"
echo rev3 > "$W/remote/rev"; touch "$(SRC)/DIRTY"
inst
check "dirty checkout: stops" ne "$RC" 0
check "dirty checkout: says why" has "$(OUT)" "local changes"
check "dirty checkout: nothing reset" eq "$(cat "$(SRC)/.git/rev")" rev2
check "dirty checkout: still there" isfile "$(SRC)/DIRTY"
rm -f "$(SRC)/DIRTY"; touch "$W/remote/nonff"
inst
check "no fast-forward: stops" ne "$RC" 0
check "no fast-forward: says so, resets nothing" has "$(OUT)" "Nothing was reset or deleted"
check "no fast-forward: checkout unchanged" eq "$(cat "$(SRC)/.git/rev")" rev2
rm -f "$W/remote/nonff"

# ── link collisions ─────────────────────────────────────────────────────────────────────────
world links arch
give_pm pacman pacman; give_pm priv sudo; give_all
mkdir -p "$W/home/.local/bin"
echo "mine" > "$W/home/.local/bin/mdis"
ln -s /usr/bin/true "$W/home/.local/bin/music_display"
inst
check "collisions: exit 0 (warns)" eq "$RC" 0
check "collisions: regular file untouched" eq "$(cat "$W/home/.local/bin/mdis")" mine
check "collisions: foreign symlink untouched" eq "$(readlink "$W/home/.local/bin/music_display")" /usr/bin/true
check "collisions: told the user" has "$(OUT)" "left alone"

# ── dry run ─────────────────────────────────────────────────────────────────────────────────
world dry arch
give_pm pacman pacman; give_pm priv sudo
inst --dry-run
check "dry run: exit 0" eq "$RC" 0
check "dry run: shows commands" has "$(OUT)" "would run: sudo pacman -S --needed --noconfirm gcc git curl playerctl rust"
check "dry run: shows the build" has "$(OUT)" "cargo build --release --locked"
check "dry run: installed nothing" nofile "$W/log/pm.log"
check "dry run: created nothing" nofile "$W/home/.local"
check "dry run: no rustup download" nofile "$W/home/.cargo"

# ── AUR fallback ────────────────────────────────────────────────────────────────────────────
world aur_no_flag arch
give_pm pacman pacman; give_pm priv sudo; give_pm aur yay; give cc git curl cava; give_rust 1.99.0
FAKE_NOT_IN_REPOS=playerctl
inst --yes
check "aur: --yes alone does not build from the AUR" nofile "$W/log/aur.log"
check "aur: says it is skipped and how to allow it" has "$(OUT)" "pass --aur"
check "aur: explains what the AUR is" has "$(OUT)" "written by a third party"
check "aur: still installs the rest" eq "$RC" 0

world aur_flag_doas arch
give_pm pacman pacman; give_pm priv doas; give_pm aur yay; give cc git curl cava; give_rust 1.99.0
FAKE_NOT_IN_REPOS=playerctl
inst --yes --aur
check "aur: --aur builds with yay and doas" has "$W/log/aur.log" "yay --sudo doas -S --needed playerctl"
check "aur: playerctl now there" isfile "$W/bin/playerctl"

world aur_flag_sudo arch
give_pm pacman pacman; give_pm priv sudo; give_pm aur paru; give cc git curl cava; give_rust 1.99.0
FAKE_NOT_IN_REPOS=playerctl
inst --aur
check "aur: paru, plain with sudo" has "$W/log/aur.log" "paru -S --needed playerctl"
check "aur: no --sudo doas when sudo exists" hasnt "$W/log/aur.log" "--sudo"

world aur_no_helper arch
give_pm pacman pacman; give_pm priv sudo; give cc git curl cava; give_rust 1.99.0
FAKE_NOT_IN_REPOS=playerctl
inst --yes --aur
check "aur: no helper -> manual hint" has "$(OUT)" "neither yay nor paru"
check "aur: never installs a helper" nofile "$W/log/aur.log"

world nodb arch
give_pm pacman pacman; give_pm priv sudo
FAKE_NODB=1
inst --yes
check "pacman db missing: fails" ne "$RC" 0
check "pacman db missing: tells to upgrade fully" has "$(OUT)" "pacman -Syu"
check "pacman db missing: nothing installed" nofile "$W/log/pm.log"

# ── options ─────────────────────────────────────────────────────────────────────────────────
world ref arch
give_pm pacman pacman; give_pm priv sudo; give_all
inst --ref v1.2.3
check "--ref is cloned" has "$W/log/tools.log" "--branch v1.2.3"
inst --help
check "--help exits 0" eq "$RC" 0
inst --bogus
check "unknown option fails" ne "$RC" 0

echo
echo "passed: $PASSES  failed: $FAILS"
[ "$FAILS" = 0 ]
