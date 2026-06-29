#!/usr/bin/env bash
set -euo pipefail

# standup-local.sh — stand up a local NCC instance that does NOT conflict with
# any other NCC already running on this machine (including the agent/instance
# orchestrating the setup).
#
# Model: build-once, run-many. One nexus-headless binary, N instances, each with
# its own NCC_NAME / data dir / workspace root / auto-picked free port.
#
# Usage:
#   ./deploy/standup-local.sh [flags] <name> [namespace]
#
#   <name>       instance name -> NCC_NAME, and derives data dir + workspace root.
#                Alphanumeric + hyphens (no leading hyphen). Must be unique on this host.
#   [namespace]  optional RELAY_NAMESPACE. Relay only enables if RELAY_ADMIN_KEY is
#                also exported in the environment (both are required by the binary).
#
# Location flags (optional — run from anywhere, put state where you want):
#   --data-dir <path>        DB/config dir (default: ~/.local/share/ncc-<name>)
#   --workspace-root <path>  where sessions/cards live (default: ~/.skynexus-sessions/<name>)
#   --base <dir>             shorthand: data dir = <dir>/data, workspaces = <dir>/workspaces
#                            (either of the two above overrides its half of --base)
#   These are FLAGS, not env vars — they're read from the command line, so the NCC_* scrub
#   below can't accidentally inherit a parent NCC's path (which is what corrupts session
#   state when two NCCs share a data dir). The binary honors NCC_DATA_DIR/NCC_WORKSPACE_ROOT;
#   this just lets standup-local set them deliberately. Relative paths resolve against $PWD.
#
# Honored env (all optional):
#   NCC_PORT_BASE        first port to try when scanning for a free one (default 4242)
#   RELAY_ADMIN_KEY      relay admin key (nra_...); with [namespace] enables relay
#   RELAY_URL            relay API URL (default https://relay.example.com)
#   NCC_GITHUB_ORG       github org/account bootstrap pulls org tooling from
#   NCC_SKIP_BOOTSTRAP   forwarded across the scrub (local dev: skip auto-bootstrap)
#   NCC_BOOTSTRAP_TOKEN  pre-seed the PWA login token (forwarded across the scrub).
#                        If unset, a secure random token is generated and printed.
#   SKIP_BUILD=1         do not build the binary even if missing (fail instead)
#
# Safe-from-inside-an-NCC: this script SCRUBS all inherited NCC_* and RELAY_NAMESPACE
# before launch and derives the new instance's data dir / workspace root / name from
# <name>, so running it from within an orchestrating NCC session cannot make the new
# instance share the orchestrator's data dir or relay identity.
#
# Conflict-safety: the port is the only shared resource that can collide; we scan
# upward from NCC_PORT_BASE and pick the first port nothing is LISTENing on, so a
# new instance never steps on a running one. Name/data-dir/workspace-root are
# derived per-name and checked for an already-running instance of the same name.

# --- locate repo (script lives in <repo>/deploy/) ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BIN="$REPO_DIR/target/release/nexus-headless"

log() { echo "[standup-local] $*"; }
die() { echo "[standup-local] ERROR: $*" >&2; exit 1; }

# --- args: optional location flags + positional <name> [namespace] ---
DATA_DIR_OVERRIDE=""
WS_ROOT_OVERRIDE=""
BASE_OVERRIDE=""
POSITIONAL=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --data-dir)         DATA_DIR_OVERRIDE="${2:-}"; shift 2 ;;
        --workspace-root)   WS_ROOT_OVERRIDE="${2:-}"; shift 2 ;;
        --base)             BASE_OVERRIDE="${2:-}"; shift 2 ;;
        --data-dir=*)       DATA_DIR_OVERRIDE="${1#*=}"; shift ;;
        --workspace-root=*) WS_ROOT_OVERRIDE="${1#*=}"; shift ;;
        --base=*)           BASE_OVERRIDE="${1#*=}"; shift ;;
        --) shift; while [[ $# -gt 0 ]]; do POSITIONAL+=("$1"); shift; done ;;
        -*) die "unknown flag: $1 (see usage at top of script)" ;;
        *)  POSITIONAL+=("$1"); shift ;;
    esac
done
NAME="${POSITIONAL[0]:-}"
NAMESPACE="${POSITIONAL[1]:-}"
[[ -n "$NAME" ]] || die "usage: $0 [--data-dir P] [--workspace-root P] [--base D] <name> [namespace]"
[[ "$NAME" =~ ^[a-zA-Z0-9][a-zA-Z0-9-]*$ ]] || die "name must be alphanumeric/hyphen, no leading hyphen: '$NAME'"

# Resolve relative override paths against the invoking $PWD (so "run from any folder" is
# predictable regardless of where the repo lives). Empty stays empty.
abspath() { case "$1" in ""|/*) printf '%s' "$1" ;; *) printf '%s/%s' "$PWD" "$1" ;; esac; }
DATA_DIR_OVERRIDE="$(abspath "$DATA_DIR_OVERRIDE")"
WS_ROOT_OVERRIDE="$(abspath "$WS_ROOT_OVERRIDE")"
BASE_OVERRIDE="$(abspath "$BASE_OVERRIDE")"

# --- capture the few knobs we forward, THEN scrub all inherited NCC_*/RELAY_NAMESPACE ---
# Run from inside an NCC session, the env already carries NCC_DATA_DIR/NCC_NAME/
# NCC_INSTANCE_ID/NCC_SESSION_ID/etc. Inheriting any of them makes the new instance
# share the orchestrator's data dir or relay identity. Forward only explicit knobs;
# derive everything else from <name>.
PORT_BASE="${NCC_PORT_BASE:-4242}"
GH_ORG_FWD="${NCC_GITHUB_ORG:-}"
# These two are local-dev pass-throughs (not instance identity) — forward them across
# the scrub so a dev can skip/seed bootstrap without the scrub eating them.
SKIP_BOOT_FWD="${NCC_SKIP_BOOTSTRAP:-}"
BOOT_TOKEN_FWD="${NCC_BOOTSTRAP_TOKEN:-}"
# Substrate integration knob (NCC_*, so it'd be eaten by the scrub below) — capture
# + re-export so an operator can wire this instance to a co-located substrate's
# admin API. SUBSTRATE_ADMIN_ADDR is not NCC_*/RELAY_NAMESPACE so it survives the
# scrub and is inherited as-is; only this one needs rescuing.
SUBSTRATE_ENABLED_FWD="${NCC_SUBSTRATE_ENABLED:-}"
# Default Agent Wake ON so a fresh instance "just works" — cards register on the relay
# and idle sessions get tapped on the shoulder when a message arrives. Override with
# NCC_WAKE_ENABLED=0. (Captured here so the NCC_* scrub below doesn't eat it.)
WAKE_ENABLED_FWD="${NCC_WAKE_ENABLED:-1}"
for v in $(env | sed -n 's/^\(NCC_[A-Za-z0-9_]*\)=.*/\1/p'); do unset "$v"; done
unset RELAY_NAMESPACE 2>/dev/null || true

# --- ensure a PROPER login/bootstrap token ---
# Pre-seeds a paired device token used to log into the PWA. If none was provided,
# mint a secure random one (never a hardcoded placeholder) and print it in the summary,
# so every instance has a real, unique token the operator can see and paste.
TOKEN_GENERATED=""
if [ -z "$BOOT_TOKEN_FWD" ]; then
    BOOT_TOKEN_FWD="$(openssl rand -hex 24 2>/dev/null || (uuidgen 2>/dev/null | tr -d '-' | tr 'A-Z' 'a-z') || echo "ncc$(date +%s)$$")"
    TOKEN_GENERATED=1
fi

# --- derive per-instance identity from <name>, honoring explicit location flags ---
# Precedence: --data-dir / --workspace-root win; else --base/{data,workspaces}; else the
# $HOME/<name> defaults. Flags came from argv (never the scrubbed env), so they can't
# reintroduce the shared-data-dir corruption the scrub guards against.
if [[ -n "$BASE_OVERRIDE" ]]; then
    DATA_DIR="${DATA_DIR_OVERRIDE:-$BASE_OVERRIDE/data}"
    WS_ROOT="${WS_ROOT_OVERRIDE:-$BASE_OVERRIDE/workspaces}"
else
    DATA_DIR="${DATA_DIR_OVERRIDE:-$HOME/.local/share/ncc-$NAME}"
    WS_ROOT="${WS_ROOT_OVERRIDE:-$HOME/.skynexus-sessions/$NAME}"
fi

# --- refuse to double-start the same named instance ---
if [[ -f "$DATA_DIR/ncc.pid" ]] && kill -0 "$(cat "$DATA_DIR/ncc.pid")" 2>/dev/null; then
    die "instance '$NAME' already running (pid $(cat "$DATA_DIR/ncc.pid"), data dir $DATA_DIR). Stop it first or pick another name."
fi

# --- portable "is this TCP port being listened on?" ---
port_in_use() {
    local p="$1"
    if command -v lsof >/dev/null 2>&1; then
        lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1
    elif command -v ss >/dev/null 2>&1; then
        ss -tlnH "sport = :$p" 2>/dev/null | grep -q .
    else
        # last resort: a successful connect means something is listening
        (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null && { exec 3>&- 3<&- 2>/dev/null; return 0; } || return 1
    fi
}

find_free_port() {
    local p="$PORT_BASE"
    while port_in_use "$p"; do
        p=$((p + 1))
        [[ "$p" -le 65535 ]] || die "no free port found from $PORT_BASE upward"
    done
    echo "$p"
}

# --- build the binary if it isn't there (PWA is embedded, build it first) ---
if [[ ! -x "$BIN" ]]; then
    [[ "${SKIP_BUILD:-}" != "1" ]] || die "binary missing and SKIP_BUILD=1: $BIN"
    command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust toolchain"
    command -v node  >/dev/null 2>&1 || die "node not found — needed to build the embedded PWA"
    log "binary not built; building (node pwa/build.mjs && cargo build --release -p nexus-headless)..."
    ( cd "$REPO_DIR" && node pwa/build.mjs && cargo build --release -p nexus-headless )
    [[ -x "$BIN" ]] || die "build finished but binary still missing: $BIN"
fi

PORT="$(find_free_port)"
mkdir -p "$DATA_DIR" "$WS_ROOT"

# --- relay enablement is opt-in and needs BOTH namespace + admin key ---
# host == NCC_NAME == this instance's <name>; cards register at {namespace}/{name}/{card}.
if [[ -n "$NAMESPACE" && -n "${RELAY_ADMIN_KEY:-}" ]]; then
    RELAY_NOTE="relay: ENABLED — host=$NAME · cards register as $NAMESPACE/$NAME/<card>"
elif [[ -n "$NAMESPACE" ]]; then
    RELAY_NOTE="relay: DISABLED — namespace '$NAMESPACE' given but RELAY_ADMIN_KEY is not set in the environment"
    log "WARNING: $RELAY_NOTE"
else
    RELAY_NOTE="relay: DISABLED — to enable, pass a namespace arg AND export RELAY_ADMIN_KEY (would register host=$NAME)"
fi

# --- launch in background, logging to the data dir ---
LOG="$DATA_DIR/ncc.log"
log "starting '$NAME' on port $PORT ..."
(
    cd "$REPO_DIR"
    export NCC_NAME="$NAME"
    export NCC_PORT="$PORT"
    export NCC_DATA_DIR="$DATA_DIR"
    export NCC_WORKSPACE_ROOT="$WS_ROOT"
    export NEXUSLINK_DEV=1
    [[ -n "$GH_ORG_FWD" ]] && export NCC_GITHUB_ORG="$GH_ORG_FWD"
    [[ -n "$SKIP_BOOT_FWD" ]] && export NCC_SKIP_BOOTSTRAP="$SKIP_BOOT_FWD"
    [[ -n "$BOOT_TOKEN_FWD" ]] && export NCC_BOOTSTRAP_TOKEN="$BOOT_TOKEN_FWD"
    [[ -n "$SUBSTRATE_ENABLED_FWD" ]] && export NCC_SUBSTRATE_ENABLED="$SUBSTRATE_ENABLED_FWD"
    export NCC_WAKE_ENABLED="$WAKE_ENABLED_FWD"
    [[ -n "$NAMESPACE" ]] && export RELAY_NAMESPACE="$NAMESPACE"
    nohup "$BIN" >"$LOG" 2>&1 &
    echo $! > "$DATA_DIR/ncc.pid"
)
PID="$(cat "$DATA_DIR/ncc.pid")"

# --- confirm it actually came up (bind happens early; give it a moment) ---
sleep 1
if ! kill -0 "$PID" 2>/dev/null; then
    die "instance '$NAME' exited immediately — see $LOG"
fi

cat <<SUMMARY

[standup-local] instance up:
  name            $NAME
  url             http://localhost:$PORT
  pid             $PID   (stop: kill \$(cat $DATA_DIR/ncc.pid))
  data dir        $DATA_DIR
  workspace root  $WS_ROOT
  log             $LOG
  login token     $BOOT_TOKEN_FWD${TOKEN_GENERATED:+  (auto-generated)}
                  ^ paste this into the "Paste bootstrap token" field to log in
  $RELAY_NOTE

  bootstrap (configures hooks/org tooling):  curl -s http://localhost:$PORT/bootstrap -X POST
SUMMARY
