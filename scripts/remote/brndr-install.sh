#!/usr/bin/env bash
# brndr-install.sh — build + install the herdr fork's stable branch on herm-b.
#
# Herm-owned deploy helper, mirroring bean-la/herm's
# scripts/remote/baremetal-deploy.sh conventions. Invoked by
# .github/workflows/brndr-deploy.yml over ssh as `herm` on herm-b.
#
# - Builds in a CLEAN detached worktree under /tmp — never in a shared checkout.
# - Installs to ~/.local/bin as both `brndr` and `herdr` (boot.ts spawns
#   HERDR_BIN ?? brndr; keep the old name for compat), backing up existing
#   binaries first (the backups saved the 2026-08-14 brndr swap).
# - NEVER auto-restarts: restarting the server kills live remote panes, so that
#   stays an operator step. Instead it detects running servers on the replaced
#   binary (inode-compare via check-brndr-stale.sh) and NOTIFIES via a marker
#   file (/var/lib/brn/brndr-restart-required) + a best-effort mailbox note to
#   the operator lane.
#
# Usage: bash brndr-install.sh <sha>   # sha must be on origin/brndr
set -euo pipefail

SHA="${1:?usage: brndr-install.sh <sha>}"
FORK="${FORK:-/home/herm/repos/github.com/bean-la/herdr}"
HERM_REPO="${HERM_REPO:-/opt/repos/github.com/bean-la/herm}"
SOCK="${BRN_SOCKET_PATH:-/var/lib/brn/brn.sock}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
WORKTREE="/tmp/brndr-deploy-${SHA:0:12}"
MARKER="/var/lib/brn/brndr-restart-required"

# zig is installed under ~/.local (exported in herm's .bashrc, which non-login
# shells from ssh don't source); cargo under ~/.cargo/bin.
export PATH="$HOME/.local/zig:$HOME/.cargo/bin:/usr/local/bin:$PATH"

log() { printf '[brndr-install] %s\n' "$*"; }

# ── 1. fetch + clean detached worktree ─────────────────────────────────
log "fetch origin brndr in ${FORK}"
git -C "$FORK" fetch bean-la brndr
if [[ -d "$WORKTREE" ]]; then
  log "worktree exists at $WORKTREE — checking out $SHA (detached)"
  git -C "$WORKTREE" checkout -q --detach "$SHA" 2>/dev/null || true
else
  git -C "$FORK" worktree add --detach "$WORKTREE" "$SHA"
fi

# ── 2. build release ───────────────────────────────────────────────────
cd "$WORKTREE"
log "cargo build --release ($(cargo --version) | zig $(zig version))"
HERDR_BUILD_ID="${SHA:0:8}" cargo build --release   # stamp the version (build_info.rs option_env)

BUILT="$WORKTREE/target/release/brndr"
[[ -x "$BUILT" ]] || { log "ERROR: no release/brndr in build output"; exit 1; }
log "built: $("$BUILT" --version 2>/dev/null || echo 'brndr (no version string)')"

# ── 3. backup + install both names ─────────────────────────────────────
TS="$(date +%m%d)"
install_one() {
  local dst="$INSTALL_DIR/$1"
  if [[ -x "$dst" ]]; then
    local bak="$dst.bak-$TS"
    [[ -e "$bak" ]] || { cp -p "$dst" "$bak"; log "backed up $dst -> $bak"; }
  fi
  install -m 0755 "$BUILT" "$dst"
  log "installed $dst ($(stat -Lc %s "$dst" 2>/dev/null || stat -f %z "$dst") bytes)"
}
install_one brndr
install_one herdr

# ── 4. stale detection + notify (never auto-restart) ───────────────────
# Fetch the checker read-only via git show FETCH_HEAD (never builds/edits in
# the shared herm checkout; same trick as baremetal-deploy.sh).
log "fetch check-brndr-stale.sh (read-only)"
git -C "$HERM_REPO" fetch origin main 2>/dev/null || true
git -C "$HERM_REPO" show FETCH_HEAD:scripts/local/check-brndr-stale.sh > /tmp/check-brndr-stale.sh 2>/dev/null || true

RESTART_REQUIRED=0
if [[ -s /tmp/check-brndr-stale.sh ]]; then
  if bash /tmp/check-brndr-stale.sh --json > /tmp/brndr-stale.json 2>&1; then
    log "OK: no running server uses a replaced binary — no restart needed."
  else
    RESTART_REQUIRED=1
    log "RESTART REQUIRED: a running server uses the replaced binary."
    cat /tmp/brndr-stale.json
  fi
else
  log "WARN: check-brndr-stale.sh unavailable — stale check skipped"
fi

# Marker file (dash/agents + human eyes; cleared by the operator on restart).
printf 'sha=%s\ninstalled_at=%s\nrestart_required=%s\n' \
  "$SHA" "$(date -u +%FT%TZ)" "$RESTART_REQUIRED" > "$MARKER"
chmod 0644 "$MARKER"
log "marker: $MARKER"

# Best-effort mailbox note to the operator lane (dynamic recipient via
# presence.list; operator agents rotate keys every launch so never hardcode).
if [[ "$RESTART_REQUIRED" == "1" ]]; then
  TO_AGENT="$(BRN_SOCKET="$SOCK" timeout 6 node -e '
    const net = require("net");
    const s = net.connect(process.env.BRN_SOCKET);
    let buf = "";
    s.on("connect", () => s.write(JSON.stringify({jsonrpc:"2.0",id:1,method:"presence.list",params:{}})+"\n"));
    s.on("data", (d) => {
      buf += d.toString();
      if (!buf.includes("\n")) return;
      try {
        const r = JSON.parse(buf.split("\n")[0]);
        const ags = (r.result && r.result.agents) || [];
        const op = ags.find((a) => a.host === "sebluair") || ags[0];
        console.log(op ? op.agent_id : "");
      } catch (e) { /* ignore */ }
      process.exit(0);
    });
    s.on("error", () => process.exit(0));
    s.setTimeout(6000, () => process.exit(0));
  ' 2>/dev/null || true)"
  if [[ -n "$TO_AGENT" ]]; then
    BRN_SOCKET="$SOCK" TO_AGENT="$TO_AGENT" SHA_SHORT="${SHA:0:12}" timeout 6 node -e '
      const net = require("net");
      const s = net.connect(process.env.BRN_SOCKET);
      const p = {
        jsonrpc: "2.0", id: 2, method: "mailbox.send",
        params: {
          to_agent: process.env.TO_AGENT,
          subject: "brndr installed (" + process.env.SHA_SHORT + ") — server restart required",
          body: "Build " + process.env.SHA_SHORT + " installed to ~/.local/bin (brndr+herdr) on herm-b. " +
                "A running server still maps the replaced binary. Restart the brn session server " +
                "when convenient to load the new build — this deploy intentionally did NOT " +
                "auto-restart (it would kill live remote panes).",
        },
      };
      s.on("connect", () => s.write(JSON.stringify(p) + "\n"));
      s.on("data", () => process.exit(0));
      s.on("error", () => process.exit(0));
      s.setTimeout(6000, () => process.exit(0));
    ' 2>/dev/null || true
    log "mailbox notify sent to $TO_AGENT"
  else
    log "no operator agent resolved from presence.list — mailbox notify skipped"
  fi
fi

# ── 5. cleanup stale deploy worktrees ──────────────────────────────────
for wt in $(git -C "$FORK" worktree list --porcelain 2>/dev/null | awk '/^worktree \/tmp\/brndr-deploy-/ {print $2}'); do
  [[ "$wt" == "$WORKTREE" ]] && continue
  git -C "$FORK" worktree remove --force "$wt" 2>/dev/null && log "removed stale worktree $wt"
done
git -C "$FORK" worktree prune

# ── 6. machine-readable result for the workflow ────────────────────────
printf 'BRNDR_DEPLOY: sha=%s restart_required=%s\n' "$SHA" "$RESTART_REQUIRED"
