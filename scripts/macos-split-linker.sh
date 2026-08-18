#!/usr/bin/env bash
# macos-split-linker.sh — split linker for macOS builds of the herdr fork.
#
# zig 0.15.2 (required by the vendored libghostty-vt) writes the mach-o
# archive's compiler_rt.o at a non-8-byte-aligned offset, which Apple's
# ld64 rejects at final link ("64-bit mach-o member not 8-byte aligned").
# Linking with zig's lld tolerates it — but zig's lld breaks rustc build
# scripts (undefined std symbols) and rejects ld64's -exported_symbols_list
# on proc-macro dylibs, so a wholesale CARGO_TARGET_*_LINKER=zig swap fails
# on cold builds.
#
# This wrapper routes by content: ONLY the final brndr link references
# libghostty-vt.a — it goes to zig cc (lld, alignment-tolerant). Every other
# link (build scripts, proc-macro dylibs) goes to Apple cc, exactly like a
# stock build. Verified on a cold aarch64-apple-darwin release build.
set -euo pipefail
if [[ "$*" == *"libghostty-vt.a"* ]]; then
  exec zig cc "$@"
else
  exec /usr/bin/cc "$@"
fi
