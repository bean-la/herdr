#!/usr/bin/env bash
# Local equivalent of .github/workflows/brndr-deploy.yml.
# Builds Linux (Docker) + macOS (native) artifacts and publishes releases.
# For build + laptop/VPS deploy without GitHub runners, use brndr-deploy-local.sh.
set -euo pipefail

REPO="${BRNDR_REPO:-bean-la/herdr}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${BRNDR_OUT_DIR:-$ROOT/dist/brndr-publish}"
ZIG_VERSION="${BRNDR_ZIG_VERSION:-0.16.0}"
RUST_VERSION="${BRNDR_RUST_VERSION:-1.96.1}"

cd "$ROOT"
GITHUB_SHA="$(git rev-parse HEAD)"
SHORT_SHA="${GITHUB_SHA:0:8}"
TAG="brndr-${GITHUB_SHA}"

export LIBGHOSTTY_VT_OPTIMIZE="${LIBGHOSTTY_VT_OPTIMIZE:-ReleaseSafe}"
export LIBGHOSTTY_VT_SIMD="${LIBGHOSTTY_VT_SIMD:-false}"
export HERDR_BUILD_ID="$SHORT_SHA"

mkdir -p "$OUT_DIR"

setup_zig_macos() {
  local zig_dir="$ROOT/.local/zig-${ZIG_VERSION}"
  if [[ ! -x "$zig_dir/zig" ]]; then
    mkdir -p "$ROOT/.local"
    local tarball="zig-aarch64-macos-${ZIG_VERSION}.tar.xz"
    curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/${tarball}" -o "/tmp/${tarball}"
    tar -xJf "/tmp/${tarball}" -C "$ROOT/.local"
    rm -rf "$zig_dir"
    mv "$ROOT/.local/zig-aarch64-macos-${ZIG_VERSION}" "$zig_dir"
  fi
  export PATH="$zig_dir:$PATH"
}

build_macos() {
  echo "==> Building macOS artifact"
  setup_zig_macos
  command -v cmake >/dev/null
  command -v ninja >/dev/null
  export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$ROOT/scripts/macos-split-linker.sh"
  cargo build --release --locked --target aarch64-apple-darwin

  cp "target/aarch64-apple-darwin/release/brndr" "$OUT_DIR/herdr-macos-aarch64"
  file "$OUT_DIR/herdr-macos-aarch64" > "$OUT_DIR/BUILD_INFO.txt"
  {
    echo "version=0.9.0"
    echo "protocol=22"
    echo "commit=$GITHUB_SHA"
    echo "target=aarch64-apple-darwin"
    echo "libghostty_vt_optimize=$LIBGHOSTTY_VT_OPTIMIZE"
    echo "libghostty_vt_simd=$LIBGHOSTTY_VT_SIMD"
  } >> "$OUT_DIR/BUILD_INFO.txt"
  python3 -c "import hashlib, pathlib; artifact = pathlib.Path('$OUT_DIR/herdr-macos-aarch64'); print(f'sha256={hashlib.sha256(artifact.read_bytes()).hexdigest()}  {artifact.name}')" >> "$OUT_DIR/BUILD_INFO.txt"
}

build_linux() {
  echo "==> Building Linux artifact (Docker)"
  command -v docker >/dev/null
  docker run --rm --platform linux/amd64 \
    -v "$ROOT:/work" \
    -w /work \
    -e HERDR_BUILD_ID="$SHORT_SHA" \
    -e LIBGHOSTTY_VT_OPTIMIZE="$LIBGHOSTTY_VT_OPTIMIZE" \
    -e LIBGHOSTTY_VT_SIMD="$LIBGHOSTTY_VT_SIMD" \
    -e ZIG_VERSION="$ZIG_VERSION" \
    -e RUST_VERSION="$RUST_VERSION" \
    -e OUT_DIR="/work/dist/brndr-publish" \
    -e GITHUB_SHA="$GITHUB_SHA" \
    ubuntu:24.04 \
    bash -euxo pipefail -c '
      apt-get update
      apt-get install -y curl ca-certificates build-essential cmake ninja-build pkg-config python3 git
      curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain "$RUST_VERSION"
      source "$HOME/.cargo/env"
      curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz" -o /tmp/zig.tar.xz
      tar -xJf /tmp/zig.tar.xz -C /opt
      export PATH="/opt/zig-x86_64-linux-${ZIG_VERSION}:$PATH"
      export CARGO_TARGET_DIR="/work/target/docker-linux-amd64"
      zig version
      # Host macOS builds write into vendor/libghostty-vt/zig-out on the bind mount.
      # Clear vendored Zig outputs so the Linux link sees amd64 libghostty-vt.
      rm -rf vendor/libghostty-vt/zig-out vendor/libghostty-vt/.zig-cache
      # Force build.rs to rebuild vendored libghostty-vt after clearing zig-out.
      cargo clean -p herdr
      cargo build --release --locked
      mkdir -p "$OUT_DIR"
      cp "$CARGO_TARGET_DIR/release/brndr" "$OUT_DIR/herdr-linux-x86_64"
      chmod 0755 "$OUT_DIR/herdr-linux-x86_64"
      (
        cd "$OUT_DIR"
        {
          echo "version=0.9.0"
          echo "protocol=22"
          echo "commit=${GITHUB_SHA}"
          sha256sum herdr-linux-x86_64
        } > BUILD_INFO-linux.txt
      )
    '
}

publish() {
  echo "==> Publishing releases to $REPO"
  if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "Release $TAG exists; uploading assets with --clobber"
    gh release upload "$TAG" --repo "$REPO" --clobber \
      "$OUT_DIR/herdr-linux-x86_64" \
      "$OUT_DIR/BUILD_INFO-linux.txt" \
      "$OUT_DIR/herdr-macos-aarch64" \
      "$OUT_DIR/BUILD_INFO.txt"
  else
    gh release create "$TAG" --repo "$REPO" --target "$GITHUB_SHA" \
      --title "$TAG" --notes "Immutable brndr build for ${GITHUB_SHA} (local publish)." \
      "$OUT_DIR/herdr-linux-x86_64" \
      "$OUT_DIR/BUILD_INFO-linux.txt" \
      "$OUT_DIR/herdr-macos-aarch64" \
      "$OUT_DIR/BUILD_INFO.txt"
  fi

  if ! gh release view brndr-latest --repo "$REPO" >/dev/null 2>&1; then
    gh release create brndr-latest --repo "$REPO" --latest=false \
      --title "brndr rolling builds" \
      --notes "Rolling herdr fork builds for the settled commit."
  fi
  gh release upload brndr-latest --repo "$REPO" --clobber \
    "$OUT_DIR/herdr-linux-x86_64" \
    "$OUT_DIR/BUILD_INFO-linux.txt" \
    "$OUT_DIR/herdr-macos-aarch64" \
    "$OUT_DIR/BUILD_INFO.txt"
}

usage() {
  cat <<EOF
Usage: $(basename "$0") [build|all|macos|linux|publish]

  build    build macOS + Linux artifacts (default)
  all      build, then publish to GitHub releases
  macos    build macOS artifact only
  linux    build Linux artifact in Docker only
  publish  publish existing artifacts from $OUT_DIR
EOF
}

main() {
  local step="${1:-build}"
  case "$step" in
    build)
      build_macos
      build_linux
      ;;
    all)
      build_macos
      build_linux
      publish
      ;;
    macos) build_macos ;;
    linux) build_linux ;;
    publish) publish ;;
    -h|--help|help) usage ;;
    *) usage; exit 1 ;;
  esac
}

main "$@"
