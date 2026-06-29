#!/usr/bin/env bash
# setup.sh — stand up a standalone NCC + private local relay on THIS machine,
# non-conflicting with anything else running, and self-documenting.
#
# It: picks a free relay port, uses an isolated SQLite DB file + per-squad dirs,
# generates its own keys, wires the NCC at the local relay, and writes a
# SETUP-MANIFEST.md recording every key / port / env var it set up. Safe to run
# next to other NCCs / relays on the same host.
#
# Storage: the relay runs on SQLite (compiled in — no database server to install
# or run; the .db file is created automatically). To use Postgres instead, build
# the relay with default features and pass a postgres:// DATABASE_URL by hand —
# see SETUP.md. Standalone defaults to SQLite.
#
# Usage:   ./setup.sh [squad-name]
#   squad-name : lowercase letters/digits/hyphens. Default: myteam.
# Env overrides (optional):
#   RELAY_PORT_BASE     first port to scan for the relay (default 8431)
#   RELAY_DB_FILE       sqlite db path (default ~/.local/share/relay-<squad>/relay.db)
#
# It does NOT seed cards or launch AI agents — those are repo-specific; see SETUP.md §6–7.
set -euo pipefail

log() { printf '[setup] %s\n' "$*" >&2; }
die() { printf '[setup] ERROR: %s\n' "$*" >&2; exit 1; }

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NCC_REPO="$KIT/nexus-control-plane"
RELAY_REPO="$KIT/nexus-relay"

SQUAD="${1:-myteam}"
[[ "$SQUAD" =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "squad name must be lowercase alphanumeric/hyphens (got '$SQUAD')"
RELAY_PORT_BASE="${RELAY_PORT_BASE:-8431}"
RELAY_DB_FILE="${RELAY_DB_FILE:-$HOME/.local/share/relay-${SQUAD}/relay.db}"
DBURL="sqlite://${RELAY_DB_FILE}"
RELAY_LOG="${TMPDIR:-/tmp}/relay-${SQUAD}.log"

log "=== standing up squad '$SQUAD' (isolated, non-conflicting) ==="

# --- prerequisites (no Postgres: the relay's SQLite backend is compiled in) ---
for c in cargo node npm git curl; do command -v "$c" >/dev/null 2>&1 || die "missing prerequisite: $c"; done
[[ -f "$RELAY_REPO/relay-api/Cargo.toml" ]] || die "relay repo not found at $RELAY_REPO (run from the kit root)"
[[ -f "$NCC_REPO/deploy/standup-local.sh" ]] || die "NCC repo not found at $NCC_REPO (run from the kit root)"

# --- identity preflight (warn-only; nothing here is set for you) ---
# The NCC/relay never touch your git identity or GitHub auth. The (removed) org
# bootstrap used to set git user.name/email; now it's yours to set. Surface gaps up
# front so a commit or a GitHub-clone doesn't fail later, deep inside a session.
GIT_NAME="$(git config --global user.name 2>/dev/null || true)"
GIT_EMAIL="$(git config --global user.email 2>/dev/null || true)"
if [[ -z "$GIT_NAME" || -z "$GIT_EMAIL" ]]; then
  log "WARNING: git identity is not fully configured — commits inside agent sessions will fail until you set it:"
  [[ -z "$GIT_NAME"  ]] && log "    git config --global user.name  \"Your Name\""
  [[ -z "$GIT_EMAIL" ]] && log "    git config --global user.email \"you@example.com\""
fi
if command -v gh >/dev/null 2>&1; then
  gh auth status >/dev/null 2>&1 || log "NOTE: 'gh' is installed but not logged in — needed ONLY for GitHub-source cards ('gh auth login'). Local-folder cards work without it."
else
  log "NOTE: no 'gh' (GitHub CLI) — needed ONLY to clone GitHub repos as cards. Point cards at your own local clones (any host) and you don't need it. See SETUP.md §6."
fi

# --- pick a FREE relay port (don't collide with anything listening) ---
port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1 || (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
RELAY_PORT="$RELAY_PORT_BASE"
while port_busy "$RELAY_PORT"; do RELAY_PORT=$((RELAY_PORT+1)); [[ "$RELAY_PORT" -le 65535 ]] || die "no free port from $RELAY_PORT_BASE"; done
log "relay port: $RELAY_PORT (scanned from $RELAY_PORT_BASE)"

# --- isolated SQLite DB file (refuse to clobber an existing one) ---
if [[ -e "$RELAY_DB_FILE" ]]; then
  die "SQLite DB '$RELAY_DB_FILE' already exists — pick a different squad name, or 'rm -f ${RELAY_DB_FILE}*' to reuse it"
fi
mkdir -p "$(dirname "$RELAY_DB_FILE")"
log "relay SQLite DB: $RELAY_DB_FILE (migrations create it on init)"

# --- build relay (SQLite backend — no DB server needed) ---
log "building relay (cargo build --release, SQLite backend; first build takes a few minutes)…"
( cd "$RELAY_REPO" && cargo build --release --no-default-features --features backend-sqlite --bin relay-api --bin relay-bootstrap >/dev/null )

# --- bootstrap: migrations + root token, then namespace (admin + operator keys) ---
log "initializing relay DB (migrations + root token)…"
INIT_OUT="$(cd "$RELAY_REPO" && DATABASE_URL="$DBURL" target/release/relay-bootstrap init 2>&1)"
ROOT_TOKEN="$(printf '%s' "$INIT_OUT" | grep -oE 'nrr_[a-f0-9]+' | head -1)"
[[ -n "$ROOT_TOKEN" ]] || die "could not parse root token from: $INIT_OUT"
log "creating relay namespace '$SQUAD'…"
NS_OUT="$(cd "$RELAY_REPO" && ROOT_TOKEN="$ROOT_TOKEN" DATABASE_URL="$DBURL" target/release/relay-bootstrap create-namespace "$SQUAD" 2>&1)"
ADMIN_KEY="$(printf '%s' "$NS_OUT" | grep -oE 'nra_[a-f0-9]+' | head -1)"
OPERATOR_KEY="$(printf '%s' "$NS_OUT" | grep -oE 'nrp_[a-f0-9]+' | head -1)"
[[ -n "$ADMIN_KEY" && -n "$OPERATOR_KEY" ]] || die "could not parse namespace keys from: $NS_OUT"
log "generated keys: root(nrr_) admin(nra_) operator(nrp_)"

# --- run the relay (localhost only) ---
( cd "$RELAY_REPO" && DATABASE_URL="$DBURL" LISTEN_ADDR="127.0.0.1:$RELAY_PORT" \
    nohup target/release/relay-api > "$RELAY_LOG" 2>&1 & )
for i in $(seq 1 15); do curl -fsS -o /dev/null "http://127.0.0.1:$RELAY_PORT/health" 2>/dev/null && break; sleep 1; done
curl -fsS -o /dev/null "http://127.0.0.1:$RELAY_PORT/health" 2>/dev/null || die "relay failed health check (see $RELAY_LOG)"
log "relay UP: http://127.0.0.1:$RELAY_PORT  (dashboard: /dashboard, log: $RELAY_LOG)"

# --- run the NCC, wired to the local relay (own token, own free port via standup-local) ---
log "installing NCC web-UI deps (npm install)…"
( cd "$NCC_REPO" && npm install >/dev/null 2>&1 )
NCC_TOKEN="$(openssl rand -hex 24 2>/dev/null || (head -c16 /dev/urandom | xxd -p))"
log "starting NCC (standup-local.sh builds the binary on first run; auto-picks a free port)…"
# NCC_SKIP_BOOTSTRAP=1: the auto-bootstrap provisioned org-internal tooling that a
# standalone install neither has nor needs; skipping it avoids a harmless boot-time error.
NCC_OUT="$(cd "$NCC_REPO" && \
  NCC_BOOTSTRAP_TOKEN="$NCC_TOKEN" NCC_SKIP_BOOTSTRAP=1 \
  RELAY_ADMIN_KEY="$ADMIN_KEY" RELAY_URL="http://127.0.0.1:$RELAY_PORT" \
  ./deploy/standup-local.sh "$SQUAD" "$SQUAD" 2>&1)"
printf '%s\n' "$NCC_OUT" | sed 's/^/[standup] /' >&2
NCC_URL="$(printf '%s' "$NCC_OUT" | grep -oE 'http://localhost:[0-9]+' | head -1)"
[[ -n "$NCC_URL" ]] || die "could not parse NCC url from standup-local output"
NCC_PORT="${NCC_URL##*:}"
curl -fsS -o /dev/null "$NCC_URL/health" 2>/dev/null || die "NCC failed health check"
log "NCC UP: $NCC_URL"

# --- write the self-documenting manifest ---
MANIFEST="$KIT/SETUP-MANIFEST.md"
cat > "$MANIFEST" <<EOF
# SETUP MANIFEST — squad '$SQUAD'

Written by setup.sh. **Local sandbox credentials — do NOT commit or share this file.**
(Already covered by the kit .gitignore, but double-check before pushing anywhere.)

## Open these
- NCC board:        $NCC_URL            ← paste the login token below
- Relay dashboard:  http://127.0.0.1:$RELAY_PORT/dashboard   ← paste the OPERATOR key below

## Keys
| Key | Value | Use |
|---|---|---|
| NCC login token | \`$NCC_TOKEN\` | log into the NCC board |
| Relay OPERATOR (nrp_) | \`$OPERATOR_KEY\` | view the relay dashboard / read+send as operator |
| Relay ADMIN (nra_) | \`$ADMIN_KEY\` | manage relay participants (the NCC uses this) |
| Relay ROOT (nrr_) | \`$ROOT_TOKEN\` | create more relay namespaces (rarely needed) |

## How this instance is wired (env / paths)
- Squad / NCC_NAME / relay namespace: \`$SQUAD\`
- Relay:  \`LISTEN_ADDR=127.0.0.1:$RELAY_PORT\`  \`DATABASE_URL=$DBURL\`  log: \`$RELAY_LOG\`
- NCC:    \`RELAY_URL=http://127.0.0.1:$RELAY_PORT\`  \`RELAY_ADMIN_KEY=<the nra_ above>\`  \`NCC_PORT=$NCC_PORT\`
- NCC data dir:       \`$HOME/.local/share/ncc-$SQUAD\`  (pid: \`ncc.pid\`)
- NCC workspace root: \`$HOME/.skynexus-sessions/$SQUAD\`
- SQLite DB file:     \`$RELAY_DB_FILE\` (isolated — created by this run; WAL sidecars alongside)

## Next steps (not automated — repo-specific)
1. Seed cards from your repos / local folders — see SETUP.md §6.
2. Start each card's session so it registers on the relay — SETUP.md §7.
3. Launch Claude in a session to make it a live agent.

## Operate
- Restart / teardown commands — see SETUP.md "Operate". Teardown:
  \`kill \$(cat $HOME/.local/share/ncc-$SQUAD/ncc.pid); kill \$(lsof -ti :$RELAY_PORT); rm -f ${RELAY_DB_FILE}*\`
EOF
chmod 600 "$MANIFEST" 2>/dev/null || true
log "wrote manifest (with all keys/env): $MANIFEST"

cat >&2 <<EOF

[setup] ============================ DONE ============================
[setup]  NCC board:       $NCC_URL     token: $NCC_TOKEN
[setup]  Relay dashboard: http://127.0.0.1:$RELAY_PORT/dashboard   operator: $OPERATOR_KEY
[setup]  Squad/namespace: $SQUAD       SQLite DB: $RELAY_DB_FILE
[setup]  Everything (keys, env, paths) is in: $MANIFEST
[setup]  relay CLI: add the kit's bin/ to PATH so you and your sessions can message:
[setup]      export PATH="$KIT/bin:\$PATH"
[setup]  Next: seed cards + start sessions  →  SETUP.md §6–7
[setup] =============================================================
EOF
