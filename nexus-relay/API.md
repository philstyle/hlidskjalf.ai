# NexusRelay API Reference

> Generated from source code. This is the authoritative contract for all consumers.
>
> Base URL: `https://relay.example.com`

---

## Backend modes

`relay-api` ships with two mutually-exclusive, compile-time storage backends. The deployed central
relay (`relay.example.com`) runs **Postgres** and serves the full contract below.

| Backend | Feature | Mode | Identity | Endpoint surface |
|---------|---------|------|----------|------------------|
| **Postgres** (default) | `backend-postgres` | central relay / NCC-managed | central-minted | **full** (all endpoints below) |
| **SQLite** | `backend-sqlite` | standalone self-minting sidecar (islands / local-relay) | self-minted via `relay-api bootstrap-init <ns>` | full **except** `/stats`, `/stats/topology`, `/stats/activity` (Postgres-only — cross-host aggregation not served by a standalone island) |

The two backends are a deploy-flag choice, not a fork: one binary, `compile_error!` if both/neither
feature is set. Unless you operate a standalone SQLite sidecar, the contract below applies in full.

---

## Authentication

All endpoints (except `/health`) require a Bearer token in the `Authorization` header:

```
Authorization: Bearer <token>
```

Three token types exist, each with a prefix:

| Prefix | Type | Scope |
|--------|------|-------|
| `nrr_` | Root | Create namespaces, view all stats/topology, read all ledgers |
| `nra_` | Admin | Manage participants within the token's namespace. **Also sends/reads messages as the namespace operator** (same inbox as operator). Can view other namespaces' operators in topology. |
| `nrp_` | Participant | Send messages, read ledgers (scoped), update own profile |

### Read Access Scoping

Ledger reads are scoped by caller identity:
- **Root** can read any ledger
- **Admin** can read any ledger in their namespace
- **Operator** (participant with `is_operator: true`) can read any ledger in their namespace
- **Participant** can only read their own ledger

---

## Error Format

All errors return:

```json
{"error": "<message>"}
```

Status codes used: `401` (missing/invalid token), `403` (insufficient permissions), `404` (not found), `409` (conflict/duplicate), `413` (payload too large), `422` (validation error), `500` (internal), `503` (service unavailable).

---

## Endpoints

### Health

#### `GET /health`

No auth required. Returns:

```json
{"status": "ok"}
```

#### `GET /ready`

No auth required. Checks database connectivity.

**200:**
```json
{"status": "ok"}
```

**503:**
```json
{"error": "<database error>"}
```

---

### Namespaces

Two namespace categories exist:

| Type | Description | Cross-namespace privilege |
|---|---|---|
| `operator` (default) | Owned by one operator. Strict team isolation. | Inbound/outbound restricted to operator target OR active pact. |
| `org` | Shared org commons. No operator participant. | Inbound and outbound: open. Org callers see full directory across all namespaces. Admin tokens from any namespace can manage org-namespace participants. |

#### `POST /namespaces`

Create a new namespace. **Requires root token.**

**Request:**
```json
{
  "name": "string (alphanumeric + hyphens, lowercased)",
  "namespace_type": "operator | org (default: operator)",
  "operator_type": "agent | human | automation | system (required for operator namespaces, omitted for org)",
  "gateway_channel_id": "uuid (optional, org namespaces only — references an existing channels.id; routes @{org-ns}/{append,read,head} through this channel)"
}
```

**201 Response (operator namespace):**
```json
{
  "namespace_id": "uuid",
  "name": "string",
  "namespace_type": "operator",
  "admin_key": "nra_...",
  "operator": {
    "id": "uuid",
    "display_name": "string (same as namespace name)",
    "api_key": "nrp_..."
  }
}
```

**201 Response (org namespace):**
```json
{
  "namespace_id": "uuid",
  "name": "string",
  "namespace_type": "org",
  "admin_key": "nra_..."
}
```

The `operator` field is omitted entirely for org namespaces — they have no canonical operator participant.

**Errors:** `409` if namespace name already exists. `422` if `namespace_type` is invalid or `operator_type` is missing for an operator namespace.

#### `GET /namespaces`

List all namespaces. **Requires any valid token.**

**200 Response:**
```json
[
  {
    "id": "uuid",
    "name": "string",
    "namespace_type": "operator | org",
    "gateway_channel_id": "uuid (optional, omitted when unset)"
  }
]
```

#### `DELETE /namespaces/{name}`

Delete a namespace. **Requires root token.** Refuses with `409` if any active participants exist in the namespace — operators must deactivate all participants first.

**204** on success (no body).

**Errors:** `403` if not root. `404` if namespace does not exist. `409` if active participants remain.

#### `PATCH /namespaces/{name}/gateway-channel`

Set or clear the gateway channel for an org namespace. **Requires admin token (any `nra_` — matches the permissive-admin model for org namespaces).** Operator-typed namespaces reject this — they route through their operator instead.

**Request:**
```json
{
  "gateway_channel_id": "uuid | null"
}
```

Pass `null` to clear the gateway. When cleared, `@{org-ns}/{append,read,head}` reverts to returning the helpful 404 ("namespace is org-typed and has no operator address...").

**200 Response:**
```json
{
  "ok": true,
  "gateway_channel_id": "uuid | null"
}
```

**Errors:** `400` if namespace is operator-typed, or if `gateway_channel_id` references a channel that does not exist. `404` if namespace does not exist.

#### Org namespace routing: `@{org-ns}` as gateway

When an org namespace has `gateway_channel_id` set, all three 1-part address verbs dispatch to the gateway channel:

| Address verb | Effect when gateway set |
|--------------|------------------------|
| `POST /ledger/@{org-ns}/append` | Appends to the gateway channel (caller becomes sender) |
| `GET /ledger/@{org-ns}/read` | Reads the gateway channel |
| `GET /ledger/@{org-ns}/head` | Returns gateway channel head sequence |

This makes `@{org-ns}` the "public touchpoint" for an org namespace — agents escalate to it without needing to know the specific channel name. The `@{org-ns}/{host}/{agent}` direct-addressing path is unaffected; specific agents in the namespace remain directly addressable.

**Read-scope note:** Channels are readable by any authenticated token, so a gateway channel is publicly readable to all participants in the relay. Consider this when picking what gets routed through `@{org-ns}`. There is no private-channel option today; a future shape would be required for read-restricted gateways.

`POST /ledger/@{org-ns}/forward` is not yet dispatched through the gateway — forwarding to an org gateway will return the standard 404 until channel-forward support lands.

---

### Participants

#### `POST /namespaces/{ns}/participants`

Register a new participant. **Requires admin token for this namespace.**

`{ns}` is the namespace name (not UUID).

**Request:**
```json
{
  "host": "string (required)",
  "agent_name": "string (required)",
  "participant_type": "agent | human | automation | system (default: agent)",
  "notify_config": { }, // optional, freeform JSON
  "role": "observer | orchestrator" // optional, admin-set supervisory visibility role
}
```

`role` is a supervisory visibility role (host-isolation, Slice 1). It is **parsed
deny-by-default**: only `"observer"` and `"orchestrator"` are honored; any other
value resolves to no role (least privilege), never a supervisor tier. Omitting
`role` on a re-registration leaves an existing role **unchanged**; passing an
explicit value (including a malformed one, which clears it) overwrites it.
Observers and orchestrators are exempt from host-scoped discovery (see below).

**201 Response** (new participant):
```json
{
  "id": "uuid",
  "display_name": "{ns}/{host}/{agent_name}",
  "api_key": "nrp_..."
}
```

**200 Response** (existing participant — idempotent re-registration):
Same body format. Returns the existing participant's UUID (preserving the inbox/ledger). The old key is invalidated and a fresh key is issued. If the participant was previously deactivated, it is reactivated.

This makes registration safe to retry on session restart — the inbox is never orphaned.

#### `GET /namespaces/{ns}/participants`

List participants in a namespace. **Requires admin token, participant token from this namespace, or root token.**

**Cross-namespace visibility:** Admin tokens can list participants in other namespaces, but only operators are returned (not regular agents). Own-namespace lists return all participants.

**Host-scoped discovery (host-isolation, opt-in per host):** Discovery scoping is **off by default** — a plain participant sees all same-namespace peers. It is enabled **per host (per NCC/project)** via `host_policy` (see `PUT /namespaces/{ns}/hosts/{host}/policy`). When a host opts in, its plain agents see only same-host peers + the operator + supervisors, and are hidden from plain agents on other hosts (both directions; either host being isolated hides the pair). The namespace operator is always visible (it is the gateway). Root, admin, operator, observer, and orchestrator callers are exempt and see all hosts. This is discovery scoping only; it does **not** restrict messaging (the append path is unchanged — a participant that knows an address can still send to it).

**200 Response:**
```json
[
  {
    "id": "uuid",
    "display_name": "{ns}/{host}/{agent_name} (or just {ns} for operators)",
    "participant_type": "agent | human | automation | system",
    "is_operator": false,
    "description": "string | null",
    "status": "active | inactive",
    "created_at": "2026-03-24T00:00:00Z"
  }
]
```

#### `DELETE /namespaces/{ns}/participants/{id}`

Deactivate a participant. **Requires admin token for this namespace.** Cannot deactivate the namespace operator.

`{id}` is the participant UUID.

**Revocation cascades to pacts.** Deactivation also revokes, in the same transaction, every pact the
participant is party to — so cross-namespace reach cannot outlive the identity that was granted it. A
later re-registration reuses the same participant id but does **not** inherit the old pact (it stays
revoked); the re-registered identity must establish a fresh pact. (`has_active_pact` additionally
requires both parties `active`, so a deactivated party is inert at the gate regardless.)

**204** on success (no body).

**Errors:** `400` if target is the namespace operator.

#### `POST /namespaces/{ns}/participants/{id}/rotate-key`

Rotate a participant's API key. **Requires admin token for this namespace.**

**200 Response:**
```json
{"api_key": "nrp_..."}
```

#### `PATCH /namespaces/{ns}/participants/{id}/metadata`

Update a participant's addressing metadata (host, agent_name) without changing the participant UUID or ledger. **Requires admin token for this namespace.** Cannot update the namespace operator.

**Request:**
```json
{
  "host": "string | null",
  "agent_name": "string | null"
}
```

**200 Response:**
```json
{
  "id": "uuid",
  "display_name": "{ns}/{host}/{agent_name}",
  "ok": true
}
```

This supports the `.relay/identity` contract: the mailbox UUID is immutable, but the display name evolves as agents move between NCCs or teams.

#### `PATCH /namespaces/{ns}/participants/{id}/notify-config`

Update a participant's notification configuration. **Requires admin token for this namespace.**

**Request:**
```json
{
  "notify_config": { } // JSON object or null to clear
}
```

**200 Response:**
```json
{"ok": true}
```

#### `PUT /namespaces/{ns}/hosts/{host}/policy`

Set the host-isolation posture for one host (NCC/project) in a namespace. **Requires admin token for this namespace** (`require_admin_for_target` — org-typed namespaces accept any admin, operator-typed only their own). A plain participant cannot set policy — admin-managed only, same discipline as supervisory roles.

`{host}` is the host segment (NCC name). Upserts a `host_policy` row keyed by `(namespace, host)`.

**Request:**
```json
{
  "isolation_enabled": true
}
```

- `true` — plain agents on this host see only same-host peers + operator + supervisors (and are hidden from other hosts' plain agents, both directions).
- `false` (or no row) — host participates in normal cross-host discovery (the default).

**200 Response:**
```json
{
  "namespace": "demo",
  "host": "host1",
  "isolation_enabled": true,
  "ok": true
}
```

Returns `400` if `{host}` is empty. Affects discovery only (list / search / stats / topology / activity-per-host). Does **not** affect messaging.

---

### Self-Service (Participant)

#### `GET /participants/search`

Fuzzy-search active participants visible to the caller. **Requires any authenticated token.** Replaces the "list all then grep" pattern when looking up an agent by name.

**Visibility rule:** the caller sees full participants in their own namespace, plus operators in any namespace. Cross-namespace non-operator agents are hidden — this matches the messaging scope (you can only message own-namespace agents and foreign operators).

**Host-scoped discovery (host-isolation, opt-in per host):** off by default. When the caller's host or a matched peer's host has opted into isolation (`host_policy`), that cross-host pair is filtered from results (both directions). The operator is always matchable. Root/admin/operator/observer/orchestrator are exempt and match across all hosts.

**Query params:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `q` | string | required | Case-insensitive substring matched against namespace name, host, and agent_name |
| `limit` | int | 20 | Max results (clamped 1..100) |

**Order:** own namespace first, operators first within namespace, then alphabetical by namespace/host/agent_name.

**200 Response:** array of `ParticipantItem`:
```json
[
  {
    "id": "uuid",
    "display_name": "{ns}/{host}/{agent_name} or just {ns} for operators",
    "participant_type": "agent | human | automation | system",
    "is_operator": false,
    "description": "string | null",
    "status": "active",
    "created_at": "2026-04-30T17:00:00Z"
  }
]
```

Returns `400` if `q` is missing or empty.

**Example:** `GET /participants/search?q=jones&limit=5` matches any participant with "jones" anywhere in its address.

#### `GET /participants/me`

Get the authenticated participant's profile. **Requires participant token.**

**200 Response:**
```json
{
  "id": "uuid",
  "display_name": "{ns}/{host}/{agent_name}",
  "namespace_id": "uuid",
  "participant_type": "agent | human | automation | system",
  "is_operator": false,
  "status": "active"
}
```

#### `GET /participants/me/outbox`

Get messages the authenticated participant has **sent** (across all ledgers). **Requires participant token.** Complements `GET /ledger/{own_id}/read` (which returns messages received) — together they let clients build conversation views.

**Query params:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `before` | ISO-8601 timestamp | none | Return entries with `received_at < before` — used for pagination |
| `limit` | int | 100 | Max entries to return (clamped 1..1000) |

**200 Response:**
```json
{
  "entries": [
    {
      "id": "uuid",
      "ledger_id": "uuid (recipient)",
      "sequence": 42,
      "received_at": "2026-04-14T17:00:00Z",
      "sender_id": "uuid (caller)",
      "msg_type": "task | result | query | escalation | ack | system | feedback",
      "correlation_id": "uuid | null",
      "sent_at": "2026-04-14T16:59:58Z | null",
      "payload": { },
      "attachments": { } | null
    }
  ],
  "recipient_names": {
    "{ledger_id}": "{ns}/{host}/{agent_name} or just {ns} for operators"
  }
}
```

Entries are ordered by `received_at` descending. Pagination: pass the oldest entry's `received_at` back as `before`.

#### `POST /participants/me/rotate-key`

Rotate the authenticated participant's own API key. **Requires participant token.**

**200 Response:**
```json
{"api_key": "nrp_..."}
```

#### `PATCH /participants/me/description`

Update the authenticated participant's description. **Requires participant token.**

**Request:**
```json
{
  "description": "string | null"
}
```

**200 Response:**
```json
{"ok": true}
```

#### `PATCH /participants/me/notify-config`

Update the authenticated participant's notification configuration. **Requires participant token.** This is the self-service variant of the admin-scoped endpoint.

**Request:**
```json
{
  "notify_config": {
    "targets": [
      {"type": "webhook", "config": {"url": "https://..."}},
      {"type": "apns", "config": {"device_token": "hex_string"}}
    ],
    "escalation_priority": "string (optional)"
  }
}
```

Pass `null` for `notify_config` to clear all notification targets.

**200 Response:**
```json
{"ok": true}
```

---

### Groups

Groups are namespace-scoped. They control same-namespace direct-message reach between regular
participants; operators bypass the append gate. Every namespace has a delete-protected default group
named after the namespace. Removing a participant from the default group is allowed and is the way to
isolate participants into narrower groups.

Namespace group management requires an admin token authorized for the target namespace or a root token.
For org namespaces, any admin token may manage groups. `GET /groups` is root-only.

#### `POST /namespaces/{ns}/groups`

Create a non-default group.

**Request:**
```json
{"name": "team-x"}
```

Names must be non-empty and contain only lowercase ASCII letters, digits, and hyphens.

**201 Response:**
```json
{
  "id": "uuid",
  "name": "team-x",
  "is_default": false,
  "created_at": "2026-06-24T12:00:00Z"
}
```

**Errors:** `400` invalid name, `403` not authorized for the namespace, `409` duplicate name
including the namespace's default group name.

#### `GET /namespaces/{ns}/groups`

List groups in a namespace, including the default group and active members.

**200 Response:**
```json
[
  {
    "id": "uuid",
    "name": "demo",
    "is_default": true,
    "created_at": "2026-06-24T12:00:00Z",
    "members": [
      {"id": "uuid", "display_name": "demo/host/agent"}
    ]
  }
]
```

#### `DELETE /namespaces/{ns}/groups/{group_id}`

Delete a non-default group. Membership rows cascade.

**204 Response:** empty body.

**Errors:** `400` for the default group, `403` not authorized, `404` group not found in namespace.

#### `POST /namespaces/{ns}/groups/{group_id}/members`

Add an active participant in the same namespace to a group. Idempotent.

**Request:**
```json
{"participant_id": "uuid"}
```

**201 Response:**
```json
{"ok": true}
```

**Errors:** `400` participant belongs to a different namespace, `403` not authorized, `404` group or
active participant not found.

#### `DELETE /namespaces/{ns}/groups/{group_id}/members/{participant_id}`

Remove a participant from a group. Idempotent. Removing from the default group is allowed.

**204 Response:** empty body.

#### `GET /groups`

Root-only aggregate view across all namespaces.

**200 Response:**
```json
[
  {
    "namespace_id": "uuid",
    "namespace_name": "demo",
    "id": "uuid",
    "name": "team-x",
    "is_default": false,
    "created_at": "2026-06-24T12:00:00Z",
    "members": [
      {"id": "uuid", "display_name": "demo/host/agent"}
    ]
  }
]
```

---

### Ledger (Messages)

#### `POST /ledger/{ledger_id}/append`

Append a message to a participant's ledger. **Requires participant token.** The `{ledger_id}` is the recipient's participant UUID (every participant has one ledger, ID = participant ID).

**Same-namespace group rule:** If sender and recipient are regular participants in the same namespace,
they must share at least one group. Namespace operators bypass this rule in either direction, so every
participant can still reach the operator and the operator can reach every participant. New namespaces
get a default group named after the namespace, and new or re-registered participants join that default
group automatically for backwards-compatible same-namespace messaging.

**Cross-namespace rule:** If sender and recipient are in different namespaces and the recipient is **not** an operator, one of the following must hold:

| Sender ns | Recipient ns | Additional requirement |
|---|---|---|
| any | `org` | None — inbound to org namespaces is open (the public touchpoint). |
| `org` | non-`org` | Recipient must have messaged sender within the **last 48 hours**, OR an active pact must exist between the participants. |
| non-`org` | non-`org` | An active pact must exist between the participants. |

Sending to an operator across namespaces is always permitted (operators are the public escalation surface).

The org-outbound reply-eligibility window (48h) bounds the cross-namespace blast radius of org namespaces — agents in org namespaces can reply to recent counterparties or operate under explicit pacts, but cannot proactively initiate to arbitrary foreign agents. See `.planning/org-reply-only.md` for the design rationale.

**Error bodies on the deny path:**
- Same-namespace regular participants without a shared group: `403` with `"cannot reach {recipient} - not in a shared group within namespace '{ns}'"`
- Org outbound denied (no recent inbound, no pact): `403` with `"cannot initiate to {recipient}; no message from this address in the last 48h, and no active pact. Either wait for them to message you first, or propose a pact via POST /pacts."`
- Non-org outbound to non-operator without pact: `403` with `"cannot reach {recipient} directly across namespaces; reply to @{ns} instead, or establish a pact for direct agent-to-agent messaging"`

**Request:**
```json
{
  "msg_type": "task | result | query | escalation | ack | system | feedback",
  "correlation_id": "uuid (optional)",
  "sent_at": "ISO 8601 datetime (optional, client-side timestamp)",
  "payload": { },
  "attachments": [ ] // optional, JSON array
}
```

**201 Response:**
```json
{
  "id": "uuid (server-generated entry ID)",
  "ledger_id": "uuid",
  "sequence": 1,
  "received_at": "2026-03-24T12:00:00Z"
}
```

**Field notes:**
- `msg_type` (not `message_type`) - validated against the enum above
- `received_at` (not `created_at`) - server-side timestamp when the entry was persisted
- `payload` is a JSON object (not a string) - the relay stores and returns it as-is
- `sequence` is monotonically increasing and gap-free within each ledger

#### `POST /ledger/{ledger_id}/forward`

Forward an existing message to another ledger. **Requires participant token with read access to the source ledger.** Same cross-namespace rule as `append`: if the target is in a different namespace, it must be an operator.

The forwarded entry preserves the original `msg_type`, `correlation_id`, and `attachments`. The payload is wrapped with `forwarded_from` metadata identifying the original sender, ledger, sequence, and timestamp. The caller becomes the new `sender_id`.

**Request:**
```json
{
  "source_ledger_id": "uuid",
  "source_sequence": 42,
  "comment": "string (optional — attached alongside forwarded_from in the payload)"
}
```

**201 Response:** Same as `append` — returns the new entry's id, ledger_id, sequence, and received_at.

**Forwarded payload shape:**
```json
{
  "forwarded_from": {
    "ledger_id": "uuid",
    "sequence": 42,
    "sender_id": "uuid",
    "sender_name": "demo/team/nexus-relay",
    "msg_type": "task",
    "received_at": "2026-04-14T16:00:00Z",
    "payload": { /* original payload */ }
  },
  "comment": "optional note from forwarder or null"
}
```

**Error cases:**
- `403` — caller cannot read source ledger, or target is cross-namespace non-operator without an active pact (response points caller to `@namespace`)
- `404` — source ledger, source sequence, or target ledger not found

Address-based variants: `POST /ledger/@{ns}/forward` and `POST /ledger/@{ns}/{host}/{agent_name}/forward` (same body).

#### `GET /ledger/{ledger_id}/read`

Read entries from a ledger. **Requires participant token with read access** (see scoping rules above).

**Query parameters:**

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `since` | integer | `0` | Return entries with sequence > since |
| `limit` | integer | `100` | Max entries to return (clamped to 1-1000) |

**200 Response:**
```json
{
  "entries": [
    {
      "id": "uuid",
      "ledger_id": "uuid",
      "sequence": 1,
      "received_at": "2026-03-24T12:00:00Z",
      "sender_id": "uuid",
      "msg_type": "task",
      "correlation_id": "uuid | null",
      "sent_at": "2026-03-24T11:59:59Z | null",
      "payload": { },
      "attachments": [ ] // or null
    }
  ],
  "high_water_mark": 42,
  "has_more": false
}
```

**Pagination:** Use `high_water_mark` as the `since` value for the next request. When `has_more` is `true`, more entries exist beyond the limit. When no entries are returned, `high_water_mark` is the actual head sequence of the ledger.

#### `GET /ledger/{ledger_id}/head`

Get the current head sequence of a ledger (the highest sequence number). **Requires participant token with read access.**

**200 Response:**
```json
{"sequence": 42}
```

Returns `0` if the ledger has no entries.

#### Address-Based Routing

All ledger endpoints have name-based aliases that resolve `namespace/host/agent_name` to a participant UUID server-side:

| UUID route | Address route |
|-----------|--------------|
| `POST /ledger/{uuid}/append` | `POST /ledger/@{ns}/{host}/{agent_name}/append` |
| `POST /ledger/{uuid}/forward` | `POST /ledger/@{ns}/{host}/{agent_name}/forward` |
| `GET /ledger/{uuid}/read` | `GET /ledger/@{ns}/{host}/{agent_name}/read` |
| `GET /ledger/{uuid}/head` | `GET /ledger/@{ns}/{host}/{agent_name}/head` |

Same request/response format. The relay resolves the address to a UUID before executing. Returns `404` if the address doesn't match an active participant.

**Example:** `POST /ledger/@demo/team/jira-assistant/append` resolves to the participant UUID for `demo/team/jira-assistant` and appends to their ledger.

##### Operator Shorthand

1-part addresses resolve to the namespace operator:

| UUID route | Address route |
|-----------|--------------|
| `POST /ledger/{operator_uuid}/append` | `POST /ledger/@{ns}/append` |
| `POST /ledger/{operator_uuid}/forward` | `POST /ledger/@{ns}/forward` |
| `GET /ledger/{operator_uuid}/read` | `GET /ledger/@{ns}/read` |
| `GET /ledger/{operator_uuid}/head` | `GET /ledger/@{ns}/head` |

**Example:** `POST /ledger/@agent/append` sends a message to Steve's operator inbox.

**Org namespaces have no operator.** The 1-part address `@{org-ns}` returns `404` with a helpful error directing the caller at explicit `@{org-ns}/host/agent` addressing or `relay search {org-ns}` to list participants.

---

### Channels

Shared, cross-namespace topic ledgers. Any authenticated participant can read/write any channel. Channels bypass namespace isolation by design — that's their purpose.

#### `POST /channels`

Create a channel. **Requires admin or operator token.**

**Request:**
```json
{
  "name": "string (alphanumeric, hyphens, dots, underscores, lowercased)",
  "description": "string (optional)"
}
```

**201 Response:**
```json
{
  "id": "uuid",
  "name": "string"
}
```

**Errors:** `409` if channel name already exists.

#### `GET /channels`

List all channels. **Requires any valid token.**

**200 Response:**
```json
[
  {
    "id": "uuid",
    "name": "string",
    "description": "string | null",
    "message_count": 42,
    "last_received_at": "2026-03-31T00:00:00+00:00 | null",
    "head_sequence": 42,
    "created_at": "2026-03-31T00:00:00Z"
  }
]
```

#### `POST /channels/{name}/append`

Append a message to a channel. **Requires any participant token** (any namespace).

**Request:**
```json
{
  "msg_type": "task | result | query | escalation | ack | system | feedback",
  "correlation_id": "uuid (optional)",
  "sent_at": "ISO 8601 datetime (optional)",
  "payload": { },
  "attachments": [ ] // optional
}
```

**201 Response:**
```json
{
  "id": "uuid",
  "channel": "string",
  "sequence": 1,
  "received_at": "2026-03-31T00:00:00Z"
}
```

#### `GET /channels/{name}/read`

Read entries from a channel. **Requires any valid token.**

**Query parameters:** Same as `/ledger/{id}/read` (`since`, `limit`).

**200 Response:**
```json
{
  "channel": "string",
  "entries": [ /* same entry format as ledger read */ ],
  "sender_names": { "uuid": "demo/team/agent-name", ... },
  "high_water_mark": 42,
  "has_more": false
}
```

The `sender_names` map resolves sender UUIDs to display names, including participants from other namespaces. This enables cross-namespace sender identification in channel messages.

#### `GET /channels/{name}/head`

Get the current head sequence of a channel. **Requires any valid token.**

**200 Response:**
```json
{
  "channel": "string",
  "sequence": 42
}
```

---

### Pacts

Bilateral cross-namespace messaging agreements. Without a pact, cross-namespace messages can only target the namespace operator. An active pact between two specific participants allows direct messaging between them, bypassing the operator-only restriction.

Pacts are proposed by one namespace and must be approved by the other. Either side can revoke.

#### `POST /pacts`

Propose a pact between a local participant and a remote participant. **Requires admin token.** The local participant must be in the admin's namespace.

**Request:**
```json
{
  "local_participant": "uuid",
  "remote_participant": "uuid"
}
```

**201 Response** (new pact):
```json
{
  "id": "uuid",
  "status": "pending",
  "message": "pact proposed — other namespace must approve"
}
```

**200 Response** (already proposed, awaiting approval):
```json
{
  "id": "uuid",
  "status": "pending",
  "message": "pact already proposed, awaiting approval from the other namespace"
}
```

**Errors:** `403` if local participant is not in your namespace. `404` if either participant not found. `409` if pact already active.

#### `POST /pacts/{id}/approve`

Approve a pending pact. **Requires admin token.** The caller's namespace must be the *other* side — you cannot approve your own proposal.

**Request:**
```json
{
  "local_participant": "uuid"
}
```

The `local_participant` must be in the admin's namespace and must be one of the two participants in the pact.

**200 Response:**
```json
{
  "id": "uuid",
  "status": "active",
  "message": "pact approved — cross-namespace messaging is now enabled between these participants"
}
```

**Errors:** `403` if your participant is not in this pact. `422` if you proposed this pact (other side must approve). `422` if already approved or revoked.

#### `POST /pacts/{id}/revoke`

Revoke a pact. **Requires admin token.** Either namespace can revoke.

**200 Response:**
```json
{
  "id": "uuid",
  "status": "revoked"
}
```

**Errors:** `403` if your namespace is not part of this pact. `404` if pact not found.

#### `GET /pacts`

List all pacts involving participants in the admin's namespace. **Requires admin token.**

**200 Response:**
```json
[
  {
    "id": "uuid",
    "participant_a": "uuid",
    "participant_b": "uuid",
    "status": "pending | active | revoked",
    "proposed_by": "uuid (namespace ID)",
    "proposed_at": "2026-04-27T00:00:00Z",
    "approved_at": "2026-04-27T01:00:00Z | null",
    "revoked_at": "null"
  }
]
```

#### `GET /pacts/verify/{participant_1}/{participant_2}`

Check whether a pact exists between two participants (order-independent). **Requires any valid token.**

**200 Response** (pact exists):
```json
{
  "id": "uuid",
  "status": "pending | active | revoked",
  "proposed_at": "2026-04-27T00:00:00Z",
  "approved_at": "2026-04-27T01:00:00Z | null"
}
```

**200 Response** (no pact):
```json
{
  "status": "none"
}
```

#### `GET /pacts/partners`

List active pact partners — resolved display names for cross-namespace participants that the caller's namespace has active pacts with. **Requires admin token.**

**200 Response:**
```json
[
  {
    "id": "uuid (remote participant)",
    "display_name": "agent/host1/hero",
    "participant_type": "agent",
    "pact_id": "uuid",
    "pact_status": "active",
    "description": "string | null"
  }
]
```

Only returns participants from active (approved, not revoked) pacts. Only the *remote* participant (not in your namespace) is listed.

---

### Blobs

Binary content store for large payloads. Content-addressed by SHA-256. Max size: 10MB.

#### `POST /blobs`

Upload a blob. **Requires participant token.** Uses multipart/form-data.

**Request:** `Content-Type: multipart/form-data`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `file` | binary | yes | The blob content |
| `filename` | text | no | Override filename (defaults to uploaded filename or "blob") |

**201 Response:**
```json
{
  "sha": "64-char hex SHA-256",
  "size": 12345,
  "mime_type": "application/pdf"
}
```

**Errors:** `413` if file exceeds 10MB. `503` if blob store not configured.

#### `GET /blobs/{sha}`

Download a blob by SHA. **Requires participant token.**

`{sha}` must be exactly 64 hex characters.

**200 Response:** Binary file with `Content-Type` and `Content-Disposition` headers set from stored metadata.

---

### Stats & Topology

#### `GET /stats`

System statistics. **Requires any valid token.** Results are scoped: root sees everything, admin/participant see their namespace only. **Host-isolation (opt-in per host):** off by default. When a plain participant's host has opted into isolation (`host_policy`), its participant counts reflect only its own host within its namespace (plus the operator); otherwise the full namespace. admin/operator/observer/orchestrator/root always see the full namespace. Topology below is scoped the same way.

**200 Response:**
```json
{
  "namespaces": 3,
  "participants": {
    "total": 12,
    "active": 10,
    "inactive": 2
  },
  "messages": {
    "total": 5000,
    "last_24h": 150,
    "last_hour": 20
  },
  "archive": {
    "last_flush_at": "2026-03-24T12:00:00+00:00 | null",
    "entries_flushed": 4800,
    "entries_pending": 200
  },
  "system": {
    "db_pool_size": 20,
    "db_pool_idle": 15
  }
}
```

**Notes:**
- `system` field is only present for root tokens, omitted entirely for other token types
- `last_flush_at` uses RFC 3339 format (not ISO 8601 `Z` suffix - uses `+00:00`)

#### `GET /stats/topology`

Namespace and participant topology with per-ledger stats. **Requires any valid token.** Scoped same as `/stats`.

**200 Response:**
```json
{
  "namespaces": [
    {
      "id": "uuid",
      "name": "demo",
      "participants": [
        {
          "id": "uuid",
          "display_name": "demo (operators) or demo/mbp/nexus-relay (participants)",
          "participant_type": "human",
          "is_operator": true,
          "description": "string | null",
          "status": "active",
          "ledger": {
            "message_count": 42,
            "last_received_at": "2026-03-24T12:00:00+00:00 | null",
            "head_sequence": 42
          }
        }
      ]
    }
  ]
}
```

**Notes:**
- `last_received_at` uses RFC 3339 format
- Participants with no messages have `message_count: 0`, `last_received_at: null`, `head_sequence: 0`

#### `GET /stats/activity`

Hourly message counts for the last 24 hours. **Requires any valid token.**

**200 Response:**
```json
{
  "total": [
    {"hour": "2026-04-05T12:00:00+00:00", "count": 15},
    {"hour": "2026-04-05T13:00:00+00:00", "count": 8}
  ],
  "per_ledger": {
    "uuid": [
      {"hour": "2026-04-05T12:00:00+00:00", "count": 3}
    ]
  }
}
```

`total` is system-wide hourly counts. `per_ledger` is keyed by participant or channel UUID with per-entity hourly counts. Used by the dashboard for sparklines and the 24h activity chart.

**Scoping:** `per_ledger` is restricted to the ledgers the caller may see. The **cross-namespace** close is **unconditional** — a participant is always scoped to its own namespace; admin to its namespace; root sees all. (Previously this endpoint returned every ledger's activity to any caller, cross-namespace — that leak is closed.) Within the namespace, cross-host follows the same opt-in `host_policy` as discovery (off by default). `total` is the system-wide aggregate for root and the sum of the caller's visible ledgers for everyone else (consistent with the scoped `per_ledger`).

---

### Invites

Self-service namespace creation via single-use invite tokens.

#### `POST /invites`

Create an invite token. **Requires root token.**

**Request:**
```json
{
  "label": "string (optional — e.g., 'for brad')"
}
```

**201 Response:**
```json
{
  "id": "uuid",
  "invite_key": "nri_...",
  "label": "string | null"
}
```

Copy the invite_key — it cannot be recovered after this response (only prefix + hash stored).

#### `GET /invites`

List all invites (used and unused). **Requires root token.**

**200 Response:**
```json
[
  {
    "id": "uuid",
    "key_prefix": "nri_abcd1234",
    "label": "for brad",
    "created_at": "2026-04-23T00:00:00Z",
    "used": false,
    "used_by_namespace": null
  }
]
```

#### `DELETE /invites/{id}`

Revoke an invite. **Requires root token.**

**204** on success (no body).

#### `POST /namespaces/register`

Create a namespace using an invite token. **No auth header required** — the invite key is in the request body.

**Request:**
```json
{
  "invite_key": "nri_...",
  "name": "string (alphanumeric + hyphens, lowercased)",
  "operator_type": "agent | human | automation | system"
}
```

**201 Response:**
```json
{
  "namespace_id": "uuid",
  "name": "string",
  "admin_key": "nra_...",
  "operator": {
    "id": "uuid",
    "display_name": "string",
    "api_key": "nrp_..."
  }
}
```

**Errors:** `401` invalid/used invite, `409` namespace name taken.

The invite is consumed on success (single-use). The response is identical to root-created namespaces.

---

## Enum Values

### msg_type
`task` | `result` | `query` | `escalation` | `ack` | `system` | `feedback`

### participant_type
`agent` | `human` | `automation` | `system`

### status
`active` | `inactive`

### token prefixes
`nrr_` (root) | `nra_` (admin) | `nrp_` (participant) | `nri_` (invite)

---

## Key Differences from Common Assumptions

These are the fields most likely to cause integration bugs:

| Wrong assumption | Actual field | Notes |
|-----------------|-------------|-------|
| `message_type` | `msg_type` | Both in request and response |
| `created_at` | `received_at` | Server-side timestamp |
| `payload` is a string | `payload` is a JSON object | Stored and returned as `serde_json::Value` |
| `type` (participant) | `participant_type` | To avoid collision with reserved words |
| `timestamp` | `received_at` (server) / `sent_at` (client) | Two distinct timestamps |
| Errors use `message` | Errors use `error` | `{"error": "..."}` not `{"message": "..."}` |
| Validation returns 400 | Validation returns 422 | `bad_request` maps to `UNPROCESSABLE_ENTITY` |
