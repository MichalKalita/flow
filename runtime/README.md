# Flow runtime in Rust

A single process loads a Flow program, validates declarations, opens SQLite and
registers HTTP, MQTT and WebSocket operations. Business operations are declarative.
Central permissions govern reads and complete transactions across all transports.

## Running

Run `./start.sh` from the repository root. The script loads `.env`, builds the
embedded admin frontend and starts a release runtime. The default database is
`data/projects/<name>.sqlite`, with hosted programs in `projects/`. See
[hosted project configuration](../projects/README.md) for routing, reload and admin
data management. Single-file mode remains available for tests and direct invocation:

```sh
cd runtime
cargo run -- --check application.flow
export FLOW_ADMIN_TOKEN="$(openssl rand -hex 32)"
FLOW_JWT_SECRET='development-key-32-bytes-minimum-123456' FLOW_AUTOMATION_KEY='automation-key-long-enough-123456789' cargo run -- application.flow flow-numeric.sqlite 127.0.0.1:8080
```

Arguments are the program, database, HTTP bind address and optional MQTT bind
address. HTTP defaults to `0.0.0.0:80`, MQTT to `127.0.0.1:1883`; WebSocket
uses the HTTP listener. SQLite is bundled. TLS is not implemented. Program and
schema versions are checked at startup; incompatible databases fail without
rewriting application data. Seed IDs are initialized once and do not restore deleted rows.

## Numeric IDs

Every application entity ID and reference is a positive integer between 1 and
9,007,199,254,740,991, stored in SQLite as `INTEGER`. This limit preserves exact
values in JavaScript clients. JSON input and output use numbers, not quoted IDs.
HTTP path/query parameters and MQTT topic segments are parsed as numeric IDs.
JWT subjects remain external identity strings, independent of local entity IDs.
WebSocket subscription labels and audit transaction correlation tokens are also
protocol metadata rather than entity IDs.

IDs retain their entity brand inside the runtime: `UserID(1)` and `ProductID(1)`
are different typed values. A plain integer is branded by its declared input or
field type. A value already carrying another entity's brand is rejected. This
check does not depend on textual prefixes. Full static inference for every
expression is still pending; computed values are validated at runtime.

`[new OrderID]` reserves an integer using a SQLite sequence in the enclosing
transaction, before records are inserted. Multiple reservations are distinct.
Committed allocations survive restarts and deletion, and rollback also rolls
back allocation. Sequences start above existing seeded IDs. `request.id` is a
numeric time-based correlation value; it is not an entity sequence.

Legacy text-ID databases are not migrated in this step. Use a new database;
existing databases remain available unchanged. `start.sh` and `.env.example`
use isolated `data/projects/<name>.sqlite` files to keep previous databases separate.

```sh
curl http://127.0.0.1:8080/api/products
curl -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"items":[{"productId":1,"quantity":2}],"paymentMethod":"CARD"}' \
  http://127.0.0.1:8080/api/orders
```

Products are public. Anonymous user reads return `[]`; a signed-in user sees only
that user's record. HS256 JWT validation checks the configured key, issuer,
audience, expiration and optional `nbf`. The example uses issuer
`https://identity.example.com`, audience `application` and subjects `idp:u1`,
`idp:u2`, `idp:u3` and `idp:catalog-admin`. User IDs are respectively 1, 2, 3 and 4.
Roles come from SQLite, not client claims or request parameters. API keys use
`Authorization: ApiKey ...` and a SHA-256 lookup through the declared adapter.

The runnable example covers products, users, orders, stock, device histories,
commands and an event automation. Orders group duplicate cart rows, decrease
stock and insert the order atomically. Payment URLs are local calculations.
Native plugins implement `Payment.createUrl`, `Image.resize` and `Files.put`;
external plugin implementations are not yet supported. Certificate authentication
is rejected over the unsecured public transport.

## Photos and plugins

`POST /api/products/{productId}/photo` accepts a base64 PNG/JPEG `photo` and optional `width` and `height`. Decoding checks dimensions and byte limits. `Image.resize` preserves the aspect ratio. `Files.put` commits the PNG blob and its File/Photo entities in one SQLite transaction. Plugin INVOKE grants do not bypass entity or argument-field permissions.

The seeded catalog administrator has subject `idp:catalog-admin`. Upload requires CATALOG_ADMIN. `File.url` points to `/api/files/{id}`, where downloads recheck READ File. Denied uploads leave no metadata or file.

## MQTT and WebSocket

MQTT 3.1.1 supports clean sessions, CONNECT, QoS 0/1 PUBLISH, SUBSCRIBE,
UNSUBSCRIBE, PING and DISCONNECT. Delivery uses QoS 0 and `+`/`#` topic filters.
Username is the auth adapter alias and password its credential. Seeded device 1
uses `deviceKey` and the development key
`device-key-32-bytes-minimum-123456789`. It can publish `devices/1/status` and
`devices/1/position` and read its commands. Numeric topic references must agree
with the payload. Stream IDs and receipt timestamps are generated by the runtime.

WebSocket upgrade `/ws` uses the Authorization header and adapters permitted by
its transport declaration. Subscribe with:

```json
{ "id": "status", "query": "LiveDeviceStatus", "input": { "deviceId": 1 } }
```

Replies contain `{"id":"status","data":{"online":true,"battery":12}}`.
Unsubscribe with `{"id":"status","unsubscribe":true}`. The subscription label
is client protocol metadata; the `deviceId` is a numeric, branded application ID.
History precedes live messages; current permissions are rechecked for delivery.

## Event automations

`[on DeviceStatus [actor [service 1]]]` runs a mutation after a stream record is
created. `[when ...]` supplies a pure condition. The verified service credential
comes from trusted `Config.event_credentials` under `service:1`, never from the
message payload. Identity and permissions are checked for the automation, and
its actor configuration is validated at startup.

The example `LowBattery` creates a DeviceAlert below 20% battery. The seeded
service key is `automation-key-long-enough-123456789`, configured by
`FLOW_AUTOMATION_KEY`. Stream and automation commit atomically; denied automation
permissions roll back the entire publish. Event chains are capped at 256 per
transaction.

## Runtime core

The parser handles bracket forms and JSON strings with input, node, and nesting limits. Numeric IDs carry entity brands. Numbers use exact rational arithmetic, finite ranges, and declared scale. Inputs/outputs reject unknown fields and validate defaults, inverses, and stream references.

Permissions are positive grants, combined with OR, covering ownership, roles, relations, fields, and previous/proposed transaction state. Missing grants deny access. Permission dependencies can be read without exposing their values to clients. Authentication and roles are pinned before writes, so a newly assigned role cannot authorize the same transaction. Authorization and output validation precede commit; failures roll back. Projection reads required columns and rule dependencies rather than SELECT *. SQLite work is serialized per project.

Compilation checks supported forms, arity, schema references, and relation cycles. Complete static inference remains pending; dynamic outputs are validated at runtime. Negative permission references are conservatively rejected. Context contains a trusted current time. Native USE with versioned plugin releases is not yet implemented.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Integration tests cover transactions, rollback, ownership, authentication, device permissions, exact numbers, narrow projection, SQLite restart, stream retention, MQTT/WebSocket interoperability, and revoked active subscriptions. See [LANGUAGE.md](LANGUAGE.md).

## Built-in observability and administration

The minimum host is a VPS with one shared CPU and 2 GB RAM. HTTP, MQTT,
SQLite, the log writer, telemetry and the dashboard run in one process. No
Prometheus server, exporter, external dashboard or monitoring database is required.
On larger machines Tokio worker and blocking threads follow CPU count, and HTTP
admission scales with CPU and RAM (16 in-flight per CPU, capped by 16 per 2 GiB),
so a 2 GiB host stays at 16 concurrent application requests. Override with
`FLOW_TOKIO_WORKERS`, `FLOW_TOKIO_BLOCKING` and `FLOW_HTTP_ADMISSION`.
Set `FLOW_HTTP_ADMISSION=unlimited` to disable the application HTTP admission
semaphore. Numeric values retain a configured concurrent-request limit, and
omitting the setting retains CPU/RAM-based admission.

Use the local setup wizard, or explicitly set `FLOW_ADMIN_TOKEN` to a secret of at least 32 bytes.
The admin dashboard listens on `127.0.0.1:9090`; `FLOW_ADMIN_BIND` changes its
address. Non-loopback addresses require the explicit opt-in
`FLOW_ADMIN_ALLOW_REMOTE=1`; token authentication and the separate admin listener
remain required. Use a trusted network or a TLS proxy when enabling network access.
For a remote VPS, forward the default loopback listener:

```sh
ssh -L 9090:127.0.0.1:9090 user@server
```

Open `http://127.0.0.1:9090` and enter the token. The token is stored in the
tab's `sessionStorage`, so a reload keeps the session and closing the tab or
using Disconnect admin clears it. The public application listener does not
expose admin APIs.
The Preact control plane has a left navigation menu with Overview, Traffic &
latency, Streaming, Runtime & storage, Log explorer, Mutation audit, HTTP console,
and Access tokens. Interactive SVG charts show request volume, errors, latency
percentiles, event throughput, connection counts, process memory and CPU. Select
15 minutes, one hour or six hours and control automatic refresh.

Endpoint inventory links directly to filtered logs and the HTTP console.
The console calls local declared endpoints with normal application credentials
and permissions; mutations change production data and are audited. The token
issuer signs HS256 application JWTs for existing identities, using configured
adapter issuer, audience and key. TTL is limited to 60 seconds through 24 hours.
Signing keys never leave the server and credentials stay in page memory.
Possession of the admin token allows issuing application credentials, so protect
it accordingly. Tokens still have the selected identity's normal permissions.

WebSocket latency measures the HTTP upgrade, not subscription lifetime.
Unmatched routes and overload rejections use fixed metric labels to bound
cardinality. Connection and subscription gauges release on session exit.
MQTT delivery counters measure successful socket writes, not broker acknowledgments.
Event and automation counters advance only after the enclosing transaction commits.

### Application logs

`FLOW_OBSERVABILITY_DIR` defaults to `data/observability`. `application.jsonl`
contains structured request logs (request ID, endpoint template, status and
elapsed time) and timed SQLite I/O and plugin spans with success/failure.
SQLite operation spans cover the entire operation, including authentication,
transaction execution and commit/rollback; nested read/apply spans give more
specific timings. Request IDs are returned in `X-Request-ID`. Logs exclude
request bodies, credentials and plugin arguments. The dashboard retains the
last 200 server events in memory as a live cache; older entries remain in server-owned disk segments. Projects keep separate telemetry, but send new operational logs to one server writer. Existing project archives remain available read-only through project log queries.
Log explorer filters by text, kind, level, endpoint, status and time, and pages
back through archives. Each query scans at most 4 MiB and returns at most 200 rows;
a continuation cursor resumes bounded scans. Archive rotation preserves cursors
until the underlying file is removed. Details show request IDs for correlation.
Audit filters entity, action and transport, with before/after snapshots decoded
to their logical JSON values, including records created by older versions.

One server log writer uses a 512-event queue with an independent 8 MiB memory cap, a 64 KiB write buffer, and a one-second flush/cleanup cadence. Individual records are bounded to 64 KiB. The default local target is 1 GiB and the independent chunk threshold is 50 MiB. Closed segments have immutable names; cleanup removes oldest closed chunks toward the target. Brief overshoot is permitted. Server settings can lower the target during operation; idle cleanup rotates an oversized active chunk too. Full queues drop operational records and report the loss rather than blocking application commits.

Every new log includes project, stable server instance, and readable server name at emission. Renaming does not rewrite history. Application responses include X-Flow-Server alongside X-Request-ID. Server settings show actual oldest local timestamp/age. The explorer provides a filtered one-minute histogram over 15 minutes, one hour, or six hours; bounded incomplete scans are labeled partial. Remote archive upload/search and peer aggregation remain planned. Legacy project archives remain outside the new server target until a later explicit migration.

### Metrics and resource usage

Telemetry stores counters, sums and fixed latency histograms in memory. It does
not keep individual request samples. p50/p95/p99 are upper bucket boundaries,
not exact percentiles; a percentile above 60 seconds is shown as `>60000`.
Counts and histograms are cumulative across restarts. The chart retains up to
360 one-minute aggregates per endpoint (six hours), with count, errors and mean
latency and histogram buckets. Charts aggregate histogram counts across endpoints
rather than averaging percentiles. HTTP 4xx and 5xx responses count as errors.
I/O and plugin spans have separate bounded latency metrics. Streaming counters
are cumulative, with six hours of minute history and 60 seconds of throughput
history. Active connection gauges reset on restart. A five-second sampler records
RSS and computes process CPU percentage from cumulative CPU time.

The writer atomically replaces `metrics.json` every 30 seconds and flushes it
on normal Ctrl-C shutdown. An abrupt termination can lose up to 30 seconds of
metrics and one second of buffered logs. Audit records remain transactional in
SQLite. A malformed metrics snapshot fails startup; back up or remove the
snapshot explicitly if resetting telemetry is intended. Do not share a telemetry
directory between multiple runtime processes.

The dashboard reports process RSS (and peak RSS on Linux) against the host RAM
limit: cgroup `memory.max` when the process is constrained, otherwise physical
memory (`/proc/meminfo` MemTotal on Linux, `sysctl hw.memsize` on macOS). It also
shows SQLite page-cache, schema and statement allocations, and estimated memory
used by metrics, history, the dashboard log cache and writer queue. Linux reads
`/proc/self/status`; macOS uses `ps` for development. Component estimates include
collection capacity and payloads, but exclude allocator bookkeeping, fragmentation,
thread stacks, active requests and other shared allocations; they do not sum to RSS.
SQLite disk usage includes the main database, WAL and SHM files. Logical page
size and telemetry/log disk usage are displayed separately.

The CLI sizes Tokio workers and blocking threads from CPU count (at least two
blocking threads), plus one log writer. By default, the public router admits at least 16
simultaneous requests and scales with CPU and RAM as above; the admin API admits
four. With bounded admission, extra application requests return HTTP 503;
`FLOW_HTTP_ADMISSION=unlimited` removes that admission check. Each project serializes
database work on its own lock, so several projects can run in parallel on extra
cores. These limits bound concurrent request buffering, but large uploads and
application queries can still dominate memory. Validate the actual workload on
the host; local tests are not a capacity guarantee.

### Transactional audit

`_flow_audit` stores every committed INSERT, UPDATE and DELETE on application
entity/stream tables and binary file records. SQLite triggers include seeds,
automations, stream retention and file cascades. Each entry contains timestamp,
transaction ID, operation, transport, authenticated actor, entity/ID, action and
before/after state. File contents are represented by their byte count. Entity
snapshots preserve raw SQL column representations, including exact decimals.
Automation entries identify their service actor and share the initiating
transaction ID. Queries and rolled-back writes create no committed-change audit
entries. Audit write failure aborts the application transaction.

Existing databases gain audit tables and triggers on their next successful open;
previous mutations cannot be reconstructed. The audit has no automatic deletion:
its retained history grows with mutation volume and is included in SQLite disk
usage. Operational logs and metrics never write to SQLite.

### Formatting and validation

```sh
cargo fmt
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cd ../admin-ui
bun install --frozen-lockfile
bun run format
bun run check
bun test src
bun run build
bunx playwright install chromium
bun run test:e2e
```

The frontend ships as small embedded HTML, CSS and JS assets with no CDN, chart
framework or frontend server. Bun, Tailwind and Playwright are development tools;
production needs only the Rust executable. Commit regenerated `src/admin-assets/`
alongside frontend source changes. E2E tests start the real runtime on isolated
ports and a temporary database; they never use the project's `.env` or production data.

## Persistent server settings and first setup

The runtime initializes a private server state directory on first startup. It generates a stable instance identity, an Ed25519 peer identity, an AES-256-GCM secret-storage key, and an administrative credential. Identity and key material persist across restarts. Missing or invalid keys for existing encrypted state fail startup instead of silently replacing keys.

`FLOW_SERVER_DIR` overrides the directory. By default it is `server/` beside the configured database directory/file; a bare database filename uses `data/server/`. `server.keys` and the containing directory have restricted permissions. `server.sqlite` stores a versioned authenticated-encrypted settings document; it is separate from application databases and mutation audit. Protect keys and recoverable encrypted state. Owner enrollment also creates `recovery.keys`, a password-encrypted recovery package using an independent salt/key derivation. It can recover the original key material using the owner password without placing unencrypted keys in a backup.

Without `FLOW_ADMIN_TOKEN`, open the loopback admin dashboard for first setup. Choose a server name and an owner password of at least 10 characters. The generated administrative credential is returned only to that setup session and remains in the tab's session storage. Later sign-ins verify the salted PBKDF2-HMAC-SHA256 owner password. A bounded server-wide login attempt budget protects the password verifier. Secrets and passwords are never written to operational logs.

Initial enrollment is available only on a loopback admin listener and rejects non-local hosts/cross-origin requests. Explicit existing admin tokens remain supported, including configured network opt-in. An explicit token keeps the legacy token login path; it does not expose unauthenticated enrollment. Public application routes never expose setup/settings APIs.

Server settings support name, bounded local-backup retention, revision-checked updates, and masked secret previews. Ordinary secrets are namespaced by project. A manifest can reference encrypted storage instead of an environment variable:

```json
{ "jwt_secrets": { "user": { "secret": "signing" } } }
```

The protected `POST /api/settings/secret` API sets a secret using `project`, `name`, and `value`. Reading settings returns only masked previews, never full values or password hashes. Environment-variable mappings continue to work. Peer identity is preparation for the planned HA feature; generation of keys alone does not implement replication or failover.

## Application catalog and frontend

The protected Applications page installs the bundled Contacts template with one click. Each installation has its own folder, database, owner identity, and generated signing key. Retry identifiers prevent duplicate installations. Templates are compiled and validated before publication. Contacts supports creation, editing, deletion, and bounded numeric cursor pagination. Writes use normal application permissions and transactional audit; stale edits return HTTP 409.

The application opens at `/<project>/app` on the public listener. Administration passes a short-lived ordinary application credential to the opened window without placing it in the URL. The public application never receives the admin credential. A project can opt into the small explicit frontend contract in `project.json`: title, item label, string fields, and declared CRUD operation aliases. This initial renderer supports text, email, telephone, and multiline text fields; it is not yet a complete arbitrary-schema frontend. Launch links currently use the browser's server hostname and the public listener port.

## Local backup and recovery

Set up an owner password before creating backups. The Backups page captures all active hosted projects, their validated programs/manifests, complete SQLite databases (including audit, allocation sequences, and blobs), encrypted server settings, and password-encrypted recovery keys. All project locks define a common capture boundary. Completed backups are published only after files are synchronized. A signed versioned manifest contains sizes and SHA-256 checksums. Local copies live in `backups/` beside the server directory; the default retains seven completed backups. Business databases in this directory are not encrypted: protect the private directory and destination disk. Secret settings and recovery keys remain encrypted.

“Check restore” reconstructs and validates an isolated temporary copy using the owner password from capture time. It checks signatures, checksums, supported runtime version, database integrity, schemas, types/references, and secret recovery. “Restore this backup” shows the selected capture time and requires confirmation. It prepares a current backup, pauses application work, preserves current directories as recovery copies, and restarts the runtime into the validated generation. A durable journal allows interrupted activation to complete or roll back on startup. Known committed ID sequence boundaries are carried forward so historical restoration does not reuse later allocated IDs. Logs are not replaced by business restoration.

For recovery to an empty directory on a replacement host:

```sh
flow-runtime --restore /safe/backups/BACKUP_ID /safe/owner-password.txt /safe/restored
FLOW_SERVER_DIR=/safe/restored/server flow-runtime /safe/restored/projects /safe/restored/databases 127.0.0.1:8080
```

The password file contains the capture-time owner password. Protect it separately. Do not run two active installations with the same recovered server identity. A local exclusive runtime lease prevents competing processes sharing the same server state location. Historical restore intentionally reverts business state; it cannot recover writes newer than the available backup or undo external actions. Remote destinations, scheduling, off-server continuous history, HA recovery, and single-project restoration remain planned. Resource acceptance on the minimum production host still needs a deployment recovery drill.
