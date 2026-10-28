#!/bin/sh
# Fetches the whole SingleStepTests/m68000 suite (MIT, 127 files, 317,500
# tests, about 140 MB) into tests/m68000/fetched, which git ignores, for
#
#     cargo test --release --features conformance conformance:: -- --ignored
#
# The commit is pinned, so a run checks the same tests as the vendored subset
# in tests/m68000 was cut from; change it here and in tests/m68000/README.md
# together. A folder given as the first argument is used instead, and the
# runner reads it from S68K_M68000_TESTS (its v1 folder).
set -eu

commit=64b253116a3de04aaac4346c43680960dc9b67e5
root=$(cd "$(dirname "$0")/.." && pwd)
target=${1:-"$root/tests/m68000/fetched"}

if [ -d "$target/.git" ] && [ "$(git -C "$target" rev-parse HEAD)" = "$commit" ]; then
    echo "The suite is already at $commit in $target."
    exit 0
fi

rm -rf "$target"
mkdir -p "$target"
git -C "$target" init --quiet
git -C "$target" remote add origin https://github.com/SingleStepTests/m68000
git -C "$target" fetch --quiet --depth 1 origin "$commit"
git -C "$target" checkout --quiet FETCH_HEAD
echo "Fetched the suite at $commit into $target."
