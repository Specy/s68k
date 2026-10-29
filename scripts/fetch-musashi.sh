#!/bin/sh
# Fetches Musashi (MIT, https://github.com/kstenerud/Musashi) into
# tests/musashi/fetched, which git ignores, for the differential harness of the
# 68010 dialect:
#
#     cargo test --release --features musashi differential -- --ignored
#
# The `musashi` feature's build script compiles it with the `cc` crate; nothing
# fetches or compiles it otherwise. The commit is pinned to the one the vendored
# legality table and self-checking programs in tests/musashi were taken from
# (tests/musashi/NOTICE). A folder given as the first argument is used instead,
# and the build script reads it from S68K_MUSASHI.
set -eu

commit=313ebf1bd9f4d0d93341eb5ce21fd8a119e9dbdd
root=$(cd "$(dirname "$0")/.." && pwd)
target=${1:-"$root/tests/musashi/fetched"}

if [ -d "$target/.git" ] && [ "$(git -C "$target" rev-parse HEAD)" = "$commit" ]; then
    echo "Musashi is already at $commit in $target."
    exit 0
fi

rm -rf "$target"
mkdir -p "$target"
git -C "$target" init --quiet
git -C "$target" remote add origin https://github.com/kstenerud/Musashi
git -C "$target" fetch --quiet --depth 1 origin "$commit"
git -C "$target" checkout --quiet FETCH_HEAD
echo "Fetched Musashi at $commit into $target."
