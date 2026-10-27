#!/usr/bin/env bash
# Cut a release: scripts/release.sh X.Y.Z [--dry-run]
# Bumps Cargo.toml and Cargo.lock, turns the Unreleased section of CHANGELOG.md into X.Y.Z with
# today's date, commits "Release X.Y.Z" and tags vX.Y.Z. Nothing is pushed; do that yourself
# (`git push && git push --tags`) once you have looked at the commit.
set -euo pipefail

new="${1:-}"
dry=0
[[ "${2:-}" == "--dry-run" ]] && dry=1
if [[ ! "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "usage: $0 X.Y.Z [--dry-run]" >&2
    exit 2
fi

cd "$(dirname "$0")/.."
tag="v$new"
today="$(date +%F)"
repo="https://github.com/moizuwu/music_display"

[[ -z "$(git status --porcelain)" ]] || { echo "working tree is not clean; commit or stash first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "tag $tag already exists" >&2; exit 1; }
grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "CHANGELOG.md has no [Unreleased] section" >&2; exit 1; }
# something must be listed under Unreleased
body="$(awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f' CHANGELOG.md | grep -c '^- ' || true)"
[[ "$body" -gt 0 ]] || { echo "the [Unreleased] section is empty" >&2; exit 1; }

old="$(awk -F'"' '/^version *=/{print $2; exit}' Cargo.toml)"
echo "releasing $old -> $new ($today)"
[[ "$dry" == 1 ]] && { echo "dry run: nothing changed"; exit 0; }

# Cargo.toml: the first `version =` is the package's
sed -i "0,/^version = \"$old\"/s//version = \"$new\"/" Cargo.toml
# Cargo.lock: only the entry of this package
awk -v old="$old" -v new="$new" '
    $0 == "name = \"music_display\"" { hit = 1 }
    hit && $0 == "version = \"" old "\"" { print "version = \"" new "\""; hit = 0; next }
    { print }' Cargo.lock > Cargo.lock.tmp && mv Cargo.lock.tmp Cargo.lock
grep -A1 '^name = "music_display"$' Cargo.lock | grep -q "^version = \"$new\"" || { echo "could not update Cargo.lock" >&2; git checkout -- Cargo.toml Cargo.lock; exit 1; }

# CHANGELOG.md: Unreleased becomes the release, a fresh Unreleased goes on top, links follow
awk -v new="$new" -v today="$today" '
    /^## \[Unreleased\]/ { print; print ""; print "## [" new "] - " today; next }
    { print }' CHANGELOG.md > CHANGELOG.tmp && mv CHANGELOG.tmp CHANGELOG.md
sed -i "s|^\[Unreleased\]: .*|[Unreleased]: $repo/compare/$tag...HEAD|" CHANGELOG.md
prev="v$old"
printf '[%s]: %s/compare/%s...%s\n' "$new" "$repo" "$prev" "$tag" > link.tmp
# new link goes right above the previous release's link
awk -v prev="[$old]:" 'FNR==NR{l=$0; next} index($0, prev)==1 && !d {print l; d=1} {print}' link.tmp CHANGELOG.md > CHANGELOG.tmp \
    && mv CHANGELOG.tmp CHANGELOG.md && rm link.tmp

git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -q -m "Release $new"
git tag -a "$tag" -m "Release $new"
echo "committed and tagged $tag. Push with: git push && git push origin $tag"
