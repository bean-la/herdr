#!/usr/bin/env bash
# rebuild-brndr.sh — build the fork at a given SHA on herm-b WITH the
# HERDR_BUILD_ID stamp (fixes the "VPS reports 0.8.0" version ambiguity).
# Runs as herm. Usage: bash rebuild-brndr.sh <sha>
set -euo pipefail
export PATH="/home/herm/.cargo/bin:/home/herm/.local/zig-0.15.2:/usr/local/bin:/usr/bin:/bin"
SHA="${1:-f9cedabc}"
FORK=/home/herm/repos/github.com/bean-la/herdr
WORKTREE="/tmp/brndr-rebuild-${SHA:0:8}"

echo "== fetch =="
git -C "$FORK" fetch bean-la brndr
git -C "$FORK" worktree remove --force "$WORKTREE" 2>/dev/null || true
git -C "$FORK" worktree add --detach "$WORKTREE" "$SHA"
cd "$WORKTREE"

echo "== build_info stamp format =="
sed -n '40,70p' src/build_info.rs

echo "== build (HERDR_BUILD_ID=${SHA:0:8}) =="
HERDR_BUILD_ID="${SHA:0:8}" cargo build --release

echo "== version =="
./target/release/brndr --version
echo "== BUILT at $WORKTREE =="
