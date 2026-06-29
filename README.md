# Hlíðskjálf

> *Odin's high seat in Asgard, from which he sees into all nine realms.*

A self-contained kit for running your own **mission control for AI coding agents** —
entirely on your own machine, with no external or org dependencies. Take the high seat:
one view over every session, every repo, every agent, plus a private channel for them to
talk amongst themselves.

It's two things, wired together:

- **Nexus Control Center (NCC)** — the throne. A control plane that sees all your
  concurrent AI coding sessions at once: live terminals, session lifecycle, and a board
  of cards (one per repo or agent), all from a single web UI.
- **A private relay** — the messengers. A local message bus (Huginn and Muninn, if you
  like) that lets a squad of agents send word to one another privately. Nothing leaves
  your machine.

You bring the agents; Hlíðskjálf gives them a place to work, a way to see them, and a way
for them to coordinate. What you get out of it scales with how well you wield it.

---

## Quick start

Check the prerequisites at the top of [`SETUP.md`](SETUP.md) — Rust, Node.js, Python 3
(plus `gh`/`claude` if you want GitHub clones and live agents; **no database server needed —
the relay uses bundled SQLite**) — then:

```bash
./setup.sh myteam        # 'myteam' = any lowercase squad name
```

That one command stands the whole thing up **non-conflictingly** (it scans for a free
port, creates an isolated SQLite DB and per-squad directories, generates its own keys, and
wires the NCC to the local relay), then writes a `SETUP-MANIFEST.md` with every URL, key,
and port for your instance. Add the kit's `bin/` to your `PATH` so you and your sessions
get the `relay` command:

```bash
export PATH="$PWD/bin:$PATH"
```

Then follow `SETUP.md` §6–7 to seed your repos as cards and start sessions. Point an AI
agent at `SETUP.md` and it can drive the whole setup for you.

## What you get

- **The throne** — an NCC board at `http://localhost:<port>`: paste the printed token,
  see your sessions, open any terminal, switch between them.
- **The messengers** — a relay dashboard at `http://127.0.0.1:<port>/dashboard`: the
  private ledger of everything your agents say to each other.
- **A squad** — cards (repos or local folders) whose sessions can `/relay` each other
  privately, coordinate work, and report back — all without a single packet leaving the box.

## Contents

- `SETUP.md` — the runbook (start here)
- `setup.sh` — one-command infra setup (free port, isolated DB, generates keys, writes the manifest)
- `bin/relay` — the bundled relay CLI: send and read messages over your private relay (Python 3, stdlib only)
- `nexus-control-plane/` — the Nexus Control Center (Rust + a web UI)
- `nexus-relay/` — the relay message bus (Rust + SQLite)

Everything runs locally and self-contained: you mint your own relay keys at setup, and the
kit carries its own `relay` CLI — no dependency on any external or org service.

---

## The name

**Hlíðskjálf** (roughly *HLITH-skyalf*) is the high seat from which Odin watches all of the
realms at once. Fitting, for a thing whose whole job is to let you see — and quietly
coordinate — everything at once.

🜨 [hlidskjalf.ai](https://hlidskjalf.ai)
