#!/bin/sh
# Fetches Moira (MIT, https://github.com/dirkwhoffmann/Moira) into
# tests/moira/fetched, which git ignores, for the cycles of the differential
# harness of the 68010 dialect:
#
#     cargo test --release --features moira differential -- --ignored
#
# The `moira` feature's build script compiles its core, Moira/, with the `cc`
# crate and tests/moira/shim.cpp; nothing fetches or compiles it otherwise. The
# commit is pinned. A folder given as the first argument is used instead, and
# the build script reads it from S68K_MOIRA.
set -eu

commit=ce28239b0507ebfe1bf2ad2b9c9252dc2f2cc031
root=$(cd "$(dirname "$0")/.." && pwd)
target=${1:-"$root/tests/moira/fetched"}

if [ -d "$target/.git" ] && [ "$(git -C "$target" rev-parse HEAD)" = "$commit" ]; then
    echo "Moira is already at $commit in $target."
    exit 0
fi

rm -rf "$target"
mkdir -p "$target"
git -C "$target" init --quiet
git -C "$target" remote add origin https://github.com/dirkwhoffmann/Moira
git -C "$target" fetch --quiet --depth 1 origin "$commit"
git -C "$target" checkout --quiet FETCH_HEAD
echo "Fetched Moira at $commit into $target."
