# Hlíðskjálf — setup runbook

**You are an AI agent. Read this whole file, then execute it top to bottom.** It
stands up, on this machine, a self-contained **NCC** — the **Nexus Control Center**, a
control plane for AI coding sessions — plus a **private local relay** (a message bus) so a
small squad of agents can talk to each other locally — with **zero dependency on any
external/org service**.

At the end you'll have:
- An **NCC board** at `http://localhost:<NCC_PORT>` (paste a printed token to log in).
- A **relay dashboard** at `http://127.0.0.1:<RELAY_PORT>/dashboard` (a message ledger).
- A squad of cards (repos) whose sessions can `/relay` each other privately.

Work step by step. **After each step, run its check and confirm it passed before
continuing.** If a check fails, see *Troubleshooting* at the bottom.

---

## Quick path (recommended): run the script

The kit ships a script that does the whole infra setup **non-conflictingly** (it scans
for a free relay port, creates an isolated Postgres DB + per-squad dirs, generates its
own keys, wires the NCC at the local relay) and **writes down exactly what it set up**:

```bash
./setup.sh myteam        # 'myteam' = any lowercase squad name
```
It logs each step and, when done, writes **`SETUP-MANIFEST.md`** containing every key,
port, URL, and env var for this instance — that file is your record of how it's wired.
It is safe to run alongside other NCCs / relays / Postgres DBs on the host.

After it finishes, **skip to §6** to seed your cards (that part is repo-specific and not
automated). Sections 0–5 below explain the same steps manually — read them to understand
or customize what the script does.

> If you're an agent: run `./setup.sh`, then **read `SETUP-MANIFEST.md`** and use it as
> your source of truth for the NCC URL/token + relay dashboard/keys in everything that
> follows. If you set up manually instead, write the same details into a manifest so the
> next agent (or you, later) can operate this instance.

---

## 0. Prerequisites — verify these first

```bash
rustc --version      # Rust toolchain (https://rustup.rs) — needed to build both services
cargo --version
node --version       # Node.js — needed to build the NCC's web UI
psql --version       # PostgreSQL client — the relay REQUIRES Postgres
pg_isready           # Postgres must be RUNNING and accept local connections
python3 --version    # used by the bundled `relay` CLI (bin/relay) + the snippets in §6–7
git --version
gh auth status       # GitHub CLI, authenticated — only if you'll clone GitHub repos as cards
claude --version     # Claude Code CLI — only needed to run actual AI agents in the sessions
```

- If `pg_isready` fails, start Postgres (macOS: `brew services start postgresql`, or
  `postgres -D <datadir>`; Linux: `sudo systemctl start postgresql`). You need to be
  able to `createdb`. If your Postgres needs a user/password, note its connection
  string — you'll use it as `DATABASE_URL` below (e.g.
  `postgres://user:pass@localhost/<db>`); the examples assume passwordless local.
- macOS or Linux. (PTYs are used; native Windows is not supported — use WSL.)

**Pick your values now** (used throughout):
- `SQUAD` = a short name for this experiment, e.g. `myteam` (letters/hyphens).
- `RELAY_PORT` = a free port for the relay, e.g. `8431`.
- `RELAY_DB` = an isolated Postgres DB name, e.g. `relay_myteam`.

---

## 1. Layout of this kit

```
standalone-ncc-kit/
├── SETUP.md                 ← this file
├── setup.sh                 ← one-command infra setup (the Quick path above)
├── bin/relay                ← bundled relay CLI (send/read messages; no org tooling needed)
├── nexus-control-plane/     ← the NCC (Rust + a vanilla-JS web UI)
└── nexus-relay/             ← the relay message bus (Rust + Postgres)
```

Set convenience paths (and put the bundled `relay` CLI on PATH so you **and your
agent sessions** can message over the relay):
```bash
KIT="$(cd "$(dirname "$0" 2>/dev/null || echo .)" && pwd)"   # or: KIT=/path/to/standalone-ncc-kit
NCC_REPO="$KIT/nexus-control-plane"
RELAY_REPO="$KIT/nexus-relay"
export PATH="$KIT/bin:$PATH"     # makes `relay` available; add to your shell rc to persist
```

---

## 2. Build the relay

```bash
cd "$RELAY_REPO"
cargo build --release --bin relay-api --bin relay-bootstrap
```
**Check:** `ls target/release/relay-api target/release/relay-bootstrap` both exist.
(First build pulls many crates — minutes is normal.)

---

## 3. Create the relay DB + tokens (one-time)

```bash
createdb "$RELAY_DB"
export DATABASE_URL="postgres://localhost/$RELAY_DB"

# migrations + a ROOT token (nrr_...) — SAVE the printed token
"$RELAY_REPO/target/release/relay-bootstrap" init

# create a namespace for your squad — SAVE the printed ADMIN_KEY (nra_) and OPERATOR_KEY (nrp_)
ROOT_TOKEN=<nrr_... from previous step> \
  "$RELAY_REPO/target/release/relay-bootstrap" create-namespace "$SQUAD"
```
**Save these three keys somewhere safe** (you'll write them into a SQUAD-OPS.md at the end):
- `nrr_...` ROOT — creates namespaces (rarely needed again)
- `nra_...` ADMIN — the NCC uses this to register agents
- `nrp_...` OPERATOR — you use this to view the dashboard / read the ledger

**Check:** the output shows `Operator ID`, `ADMIN_KEY=nra_...`, `OPERATOR_KEY=nrp_...`.

---

## 4. Run the relay (localhost only)

```bash
cd "$RELAY_REPO"
DATABASE_URL="postgres://localhost/$RELAY_DB" LISTEN_ADDR="127.0.0.1:$RELAY_PORT" \
  nohup target/release/relay-api > /tmp/relay-$SQUAD.log 2>&1 &
sleep 2
```
**Check:** `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:$RELAY_PORT/health`
prints `200`, and `/tmp/relay-$SQUAD.log` shows `listening on 127.0.0.1:<RELAY_PORT>`.

---

## 5. Run the NCC, pointed at the local relay

The NCC's `deploy/standup-local.sh` builds the NCC binary (and its web UI) on first
run, picks a free port, and prints a login token. Pass the relay env so it wires to
your local relay instead of any default.

```bash
cd "$NCC_REPO"
npm install          # one-time: installs esbuild for the web-UI build (standup-local.sh runs `node pwa/build.mjs`)
RELAY_ADMIN_KEY=<nra_... from step 3> \
RELAY_URL="http://127.0.0.1:$RELAY_PORT" \
NCC_SKIP_BOOTSTRAP=1 \
  ./deploy/standup-local.sh "$SQUAD" "$SQUAD"
```
This prints a block with **`url`** (`http://localhost:<NCC_PORT>`) and a **`login token`**.
**Save both.** It should also say **`relay: ENABLED`**.

`NCC_SKIP_BOOTSTRAP=1` skips an auto-provisioning step that only applied to the
original org deployment (it installed internal tooling from private repos). A
standalone install neither has nor needs it — everything you need is set up by
these steps. (Omit the flag and the NCC still runs; it just logs one harmless
"bootstrap" error.)

**Check:** `curl -s -o /dev/null -w '%{http_code}\n' http://localhost:<NCC_PORT>/health`
prints `200`.

> Tip: to run the NCC **without** any relay (fully standalone), just omit
> `RELAY_ADMIN_KEY`/`RELAY_URL` and the namespace arg: `./deploy/standup-local.sh "$SQUAD"`.

---

## 6. Seed the squad (create cards)

Set a token + base, and get a lane id:
```bash
T="Authorization: Bearer <login token from step 5>"
B="http://localhost:<NCC_PORT>"
LANE=$(curl -s -H "$T" $B/lanes | python3 -c "import sys,json;print(json.load(sys.stdin)[0]['id'])")
```

Create a card per repo. **From a GitHub repo** (needs `gh auth`):
```bash
curl -s -X POST $B/cards -H "$T" -H 'Content-Type: application/json' \
  -d "{\"name\":\"my-agent\",\"lane_id\":\"$LANE\",\"source_type\":\"github\",\"repo_full_name\":\"<owner/repo>\",\"notes\":\"#orchestrator\"}"
```
**Or from a local folder** (no GitHub needed):
```bash
curl -s -X POST $B/cards -H "$T" -H 'Content-Type: application/json' \
  -d "{\"name\":\"my-agent\",\"lane_id\":\"$LANE\",\"source_type\":\"local\",\"local_path\":\"/abs/path/to/folder\",\"notes\":\"#orchestrator\"}"
```
- `notes:"#orchestrator"` makes that card an **orchestrator**: the NCC injects
  `NCC_AUTH_TOKEN` + `NCC_PORT` into its session so it can drive the API to spawn/manage
  the others. Give it to your lead/coordinator card; omit it for plain worker cards.
- New cards are relay-enabled by default, so they auto-join your private relay.

**Check:** `curl -s -H "$T" $B/sessions | python3 -c "import sys,json;[print(c['card_name']) for c in json.load(sys.stdin)]"`
lists your cards. The card-create response includes each `card_id` — save them.

---

## 7. Start sessions + verify private relay works

Starting a session registers the card on your local relay and injects its relay creds:
```bash
curl -s -X POST $B/cards/<card_id>/session -H "$T" -d '{}'   # repeat per card
```
**Check registration:** `psql -d $RELAY_DB -tAc "SELECT host||'/'||agent_name FROM participants;"`
lists your cards under namespace `$SQUAD`.

**Prove agent-to-agent relay** (replace IDs with two of your participants — get a
target's id from its workspace `.relay/identity.json`):
```bash
WSROOT="$HOME/.skynexus-sessions/$SQUAD"      # default workspace root
TARGET=$(python3 -c "import json;print(json.load(open('$WSROOT/<cardB>/.relay/identity.json'))['participant_id'])")
( cd "$WSROOT/<cardA>" && env -u RELAY_API_KEY RELAY_URL="http://127.0.0.1:$RELAY_PORT" \
    relay send "$TARGET" --type task --body "hello from cardA" )
( cd "$WSROOT/<cardB>" && env -u RELAY_API_KEY RELAY_URL="http://127.0.0.1:$RELAY_PORT" \
    relay inbox --all | tail -3 )
```
The `relay` CLI is **bundled with this kit** at `bin/relay` (put it on PATH per §1).
It reads `RELAY_URL`/`RELAY_API_KEY` from the env, or falls back to the workspace's
`.relay/state.json` — so `cd <workspace> && relay inbox` works inside any session.
Useful commands: `relay send <id|@ns/host/agent> --body "…"`, `relay inbox`,
`relay participants`, `relay whoami`. This is also what gives each agent session a
working relay channel to message the rest of the squad.

---

## 8. Use it

- **NCC board:** open `http://localhost:<NCC_PORT>`, paste the login token.
- **Relay ledger:** open `http://127.0.0.1:$RELAY_PORT/dashboard`; if it asks for a
  token, paste the **OPERATOR key** (`nrp_...`).
- **Make a card into a live AI agent:** in the board, open the card's terminal and run
  your Claude launch command (the orchestrator card already has API access). The agent
  inherits the local `/relay`, so it can message the squad privately.

**Write a `SQUAD-OPS.md`** next to your orchestrator's workspace recording: the NCC url +
login token, the relay dashboard url + operator key, the admin/root keys, the card ids,
and the start/stop commands below — so an agent can help operate this later. **Keep keys
local; don't commit them** (add the file to `.git/info/exclude` in that repo).

---

## Operate (status / restart / teardown)

```bash
# Status
cat ~/.local/share/ncc-$SQUAD/ncc.pid ; curl -s http://localhost:<NCC_PORT>/health
lsof -ti :$RELAY_PORT ; curl -s http://127.0.0.1:$RELAY_PORT/health

# Restart relay (if it died) — binary already built
cd "$RELAY_REPO" && DATABASE_URL="postgres://localhost/$RELAY_DB" LISTEN_ADDR="127.0.0.1:$RELAY_PORT" \
  nohup target/release/relay-api > /tmp/relay-$SQUAD.log 2>&1 &

# Restart NCC (KILLS its sessions) — relay must be up first
kill "$(cat ~/.local/share/ncc-$SQUAD/ncc.pid)"
cd "$NCC_REPO" && RELAY_ADMIN_KEY=<nra_...> RELAY_URL="http://127.0.0.1:$RELAY_PORT" \
  ./deploy/standup-local.sh "$SQUAD" "$SQUAD"
# then re-POST a /session per card so they re-register

# Tear it all down
kill "$(cat ~/.local/share/ncc-$SQUAD/ncc.pid)" ; kill "$(lsof -ti :$RELAY_PORT)"
dropdb "$RELAY_DB"
# workspaces under ~/.skynexus-sessions/$SQUAD and data under ~/.local/share/ncc-$SQUAD persist until removed
```

---

## Troubleshooting

- **`send returns "session is not idle"`** — the send endpoint waits for the agent to be
  idle. For a programmatic clear/command, add `"force":true` to bypass.
- **Relay won't start / `DATABASE_URL is required`** — Postgres isn't running or the URL
  is wrong. Check `pg_isready`; set `DATABASE_URL` to a reachable DB you can write.
- **Port already in use** — pick a different `RELAY_PORT`; the NCC auto-picks its own free
  port (read it from the standup-local output).
- **`bootstrap.sh: No such file`** in NCC log — harmless. That's optional org tooling.
- **Cards show no relay participants** — registration happens on **session start**; make
  sure the relay was up and `RELAY_URL` was set when you (re)started the NCC, then POST a
  `/session` for each card.
- **Order matters** — relay up first, then the NCC (it reads `RELAY_URL` at startup).
- **No `claude`** — the NCC + relay still run; sessions are just shells until you install
  and launch an agent CLI in them.
