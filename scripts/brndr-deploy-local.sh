#!/usr/bin/env bash
# Local brndr build + deploy without GitHub Actions runners.
#
# Builds artifacts via brndr-publish-local.sh, installs on this Mac and herm-b
# over SSH/scp. Publish to GitHub Releases is a separate explicit step.
#
# Examples:
#   HERM_HOST=herm-b.example ./scripts/brndr-deploy-local.sh all
#   HERM_HOST=... ./scripts/brndr-deploy-local.sh all --skip-build
#   ./scripts/brndr-deploy-local.sh laptop --skip-build
#   ./scripts/brndr-deploy-local.sh publish
#
# Env:
#   HERM_HOST      SSH host for herm-b (required for vps/all)
#   HERM_REPO      Local herm checkout (default ../herm)
#   BRNDR_OUT_DIR  Artifact directory (default dist/brndr-publish)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PUBLISH_SCRIPT="$ROOT/scripts/brndr-publish-local.sh"
OUT_DIR="${BRNDR_OUT_DIR:-$ROOT/dist/brndr-publish}"
HERM_REPO="${HERM_REPO:-$ROOT/../herm}"
REMOTE_HERM_REPO="${REMOTE_HERM_REPO:-/opt/repos/github.com/bean-la/herm}"
REMOTE_INSTALL_SCRIPT="$REMOTE_HERM_REPO/scripts/remote/brndr-install-artifact.sh"
REMOTE_RESTART_SCRIPT="$REMOTE_HERM_REPO/scripts/local/restart-brndr.sh"

GITHUB_SHA="$(git -C "$ROOT" rev-parse HEAD)"
SKIP_BUILD=0
DO_RESTART=0
COMMAND="build"

usage() {
  cat <<EOF
Usage: $(basename "$0") [build|laptop|vps|all|publish] [options]

  build     build macOS + Linux artifacts (default)
  laptop    install macOS artifact to ~/.local/bin/brndr-bin
  vps       scp Linux artifact to HERM_HOST and install
  all       build, laptop, vps
  publish   upload artifacts to GitHub releases (optional)

Options:
  --skip-build   use existing artifacts in $OUT_DIR
  --sha <sha>    override commit SHA (default: HEAD)
  --restart      restart brndr on herm-b after vps install (opt-in)
EOF
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      build|laptop|vps|all|publish)
        COMMAND="$1"
        shift
        ;;
      --skip-build)
        SKIP_BUILD=1
        shift
        ;;
      --restart)
        DO_RESTART=1
        shift
        ;;
      --sha)
        GITHUB_SHA="${2:?--sha requires a value}"
        shift 2
        ;;
      -h|--help|help)
        usage
        exit 0
        ;;
      *)
        echo "unknown argument: $1" >&2
        usage >&2
        exit 1
        ;;
    esac
  done
}

require_artifact() {
  local path="$1"
  local label="$2"
  if [[ ! -f "$path" ]]; then
    echo "ERROR: missing $label at $path (run build first or drop --skip-build)" >&2
    exit 1
  fi
}

do_build() {
  if [[ "$SKIP_BUILD" == "1" ]]; then
    echo "==> Skipping build (--skip-build)"
    return
  fi
  echo "==> Building artifacts"
  bash "$PUBLISH_SCRIPT" build
}

do_laptop() {
  echo "==> Installing macOS artifact on laptop"
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "laptop install skipped: not macOS ($(uname -s))"
    return
  fi

  local artifact="$OUT_DIR/herdr-macos-aarch64"
  require_artifact "$artifact" "macOS artifact"

  if ! file "$artifact" | grep -qE "Mach-O.*(arm64|aarch64)"; then
    echo "ERROR: macOS artifact is not aarch64 Mach-O" >&2
    exit 1
  fi

  local install_dir="${BRNDR_INSTALL_DIR:-${HOME}/.local/bin}"
  mkdir -p "$install_dir"
  install -m 0755 "$artifact" "$install_dir/brndr-bin"
  ln -sf "$install_dir/brndr-bin" "$install_dir/herdr"

  local homebrew_herdr="/opt/homebrew/bin/herdr"
  if [[ "${PATH}" == */opt/homebrew/bin:* ]]; then
    ln -sfn "$install_dir/brndr-bin" "$homebrew_herdr" 2>/dev/null \
      || echo "NOTE: cannot update $homebrew_herdr" >&2
  fi

  echo "  installed → $install_dir/brndr-bin: $("$install_dir/brndr-bin" --version 2>/dev/null || echo '?')"
}

do_vps() {
  echo "==> Installing Linux artifact on herm-b"
  if [[ -z "${HERM_HOST:-}" ]]; then
    echo "ERROR: HERM_HOST is required for vps deploy" >&2
    exit 1
  fi

  local linux_artifact="$OUT_DIR/herdr-linux-x86_64"
  local build_info="$OUT_DIR/BUILD_INFO-linux.txt"
  require_artifact "$linux_artifact" "Linux artifact"
  require_artifact "$build_info" "BUILD_INFO-linux.txt"

  local remote_artifact="/tmp/brndr-artifact-${GITHUB_SHA}"
  local local_install_script="$HERM_REPO/scripts/remote/brndr-install-artifact.sh"
  if [[ ! -f "$local_install_script" ]]; then
    echo "ERROR: local install script not found at $local_install_script" >&2
    exit 1
  fi

  local remote_build_info="/tmp/brndr-build-info-${GITHUB_SHA}.txt"
  local remote_install="/tmp/brndr-install-artifact-${GITHUB_SHA}.sh"

  echo "  scp → $HERM_HOST"
  scp "$linux_artifact" "$HERM_HOST:$remote_artifact"
  scp "$build_info" "$HERM_HOST:$remote_build_info"
  if ssh "$HERM_HOST" "test -x '$REMOTE_INSTALL_SCRIPT'" 2>/dev/null; then
    remote_install="$REMOTE_INSTALL_SCRIPT"
  else
    scp "$local_install_script" "$HERM_HOST:$remote_install"
  fi

  ssh "$HERM_HOST" \
    "BRNDR_BUILD_INFO='$remote_build_info' HERM_REPO='$REMOTE_HERM_REPO' bash '$remote_install' '$GITHUB_SHA' '$remote_artifact'"

  ssh "$HERM_HOST" "rm -f '$remote_artifact' '$remote_build_info' '/tmp/brndr-install-artifact-${GITHUB_SHA}.sh'"

  if [[ "$DO_RESTART" == "1" ]]; then
    echo "==> Restarting brndr on herm-b (--restart)"
    ssh "$HERM_HOST" \
      "sudo -u herm env PATH='/home/herm/.local/bin:/usr/bin:/bin' BRNDR_BIN=/home/herm/.local/bin/brndr /bin/bash '$REMOTE_RESTART_SCRIPT'"
  else
    echo "  restart skipped (pass --restart to restart brndr on herm-b)"
  fi
}

do_publish() {
  bash "$PUBLISH_SCRIPT" publish
}

main() {
  parse_args "$@"
  cd "$ROOT"

  case "$COMMAND" in
    build)
      do_build
      ;;
    laptop)
      do_laptop
      ;;
    vps)
      do_vps
      ;;
    all)
      do_build
      do_laptop
      do_vps
      ;;
    publish)
      do_publish
      ;;
    *)
      usage >&2
      exit 1
      ;;
  esac
}

main "$@"
