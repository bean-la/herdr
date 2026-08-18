#!/usr/bin/env bash
# macos-split-linker.sh — split linker for macOS builds of the herdr fork.
#
# zig 0.15.2 (required by the vendored libghostty-vt) writes the mach-o
# archive's compiler_rt.o at a non-8-byte-aligned offset, which Apple's
# ld64 rejects at final link ("64-bit mach-o member not 8-byte aligned").
# Linking executables with zig's lld tolerates it — but zig's lld does NOT
# accept ld64's -exported_symbols_list (used for proc-macro dylibs), so a
# wholesale CARGO_TARGET_*_LINKER=zig swap breaks the build-script dylibs.
#
# This wrapper routes by flag: dylib links (rustc passes
# -exported_symbols_list) go to Apple cc; executable links go to zig cc
# (lld). Requires zig on PATH and /usr/bin/cc (macOS runner default).
set -euo pipefail
if [[ "$*" == *"-exported_symbols_list"* ]]; then
  exec /usr/bin/cc "$@"
else
  exec zig cc "$@"
fi
