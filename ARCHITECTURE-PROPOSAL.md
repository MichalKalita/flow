# Minimal SQLite application platform

Status: Architecture proposal based on the requirements and the preference for one executable. This is a design recommendation, not a tested implementation or a revision of `BLACKBOX-REQUIREMENTS.md`.

## Recommended composition

Build the platform in Go. Reuse rqlite's SQLite/Raft storage implementation inside the executable, and implement application capabilities as tables and small runtime services around that storage. Do not add etcd for application key-value storage.

The first storage integration should vendor a reviewed rqlite release behind our own adapter. rqlite exposes a Go `store` package with database execution, replication, membership, barriers, and backup operations. That makes an in-process integration a credible starting point, but does not establish a supported, stable embedding API. Building and operating that integration must be the first technical spike; upstream changes may require maintaining a small fork. [rqlite store package](https://pkg.go.dev/github.com/rqlite/rqlite/v9/store)

Use these components:

| Concern | Proposed implementation |
| --- | --- |
| Application state | One logical SQLite database, with a complete copy on each replica |
| Replication and recovery | rqlite storage and its Raft implementation, embedded behind an adapter |
| Application transactions | Explicit commit of a prepared, atomic SQL batch |
| Audit | Application-owned SQL table in the same database |
| Replicated KV | Indexed SQL table with expiration timestamps |
| Local temporary KV | Bounded in-memory map on each server |
| Queues | SQL tables and a worker dispatcher |
| Pub/sub | SQL event rows and independent subscription cursors |
| Cron | SQL schedule rows that produce queue messages |
| Replicated files | File metadata and BLOB chunks in SQLite |
| HTTP, TCP, UDP | Go networking and embedded routing |
| Automatic HTTPS | CertMagic library |
| Application execution | Go handlers compiled into the executable; worker mode for release isolation |
| Runtime diagnostics | Local files, excluded from replication and backups |
| Metrics and traces | Bounded local collectors with explicitly weaker guarantees |
| Dashboard | HTML, CSS, and JavaScript embedded in the executable |
| Backups | Consistent database export plus a manifest uploaded to S3-compatible storage |

This is one deployed executable, not necessarily one process or one physical data file. SQLite journals, Raft logs, snapshots, configuration, and local diagnostics still need storage. Application data should share one logical database; internal replication files need not be forced into it.

## Why SQLite also handles KV, audit, and messaging

Separating SQL, KV, and queues into different storage systems creates a distributed commit problem. Keeping their durable state in one database turns that into an ordinary database transaction. This is the central design choice.

For example, one committed batch can insert an order, append its audit record, update a KV entry, enqueue work, publish an event row, and acknowledge the message that caused the operation. Either the entire batch becomes application state or none of it does. rqlite supports atomic batches through its transaction option. [rqlite transaction API](https://rqlite.io/docs/api/api/)

Proposed table families:

- Application tables, including any application-owned `audit_log`.
- `_platform_kv(namespace, key, value, expires_at)`.
- `_platform_queue_definitions` and `_platform_messages`.
- `_platform_events` and `_platform_subscriptions`.
- `_platform_schedules` and `_platform_schedule_occurrences`.
- `_platform_files` and `_platform_file_chunks`.
- `_platform_operations` for operation identities and committed results.
- `_platform_revision` and `_platform_schema_versions`.
- Versioned configuration and encrypted secrets, if managed in the database.

These names describe the schema responsibilities, not a finalized schema.

The application supplies the audit record's business meaning and performs the insert through the platform transaction API. The platform supplies storage, replication, transaction handling, and backup. There is no need for a separate audit storage engine. Append-only restrictions and any special access rules still need enforcement; an ordinary table alone does not provide a tamper-proof audit trail.

KV expiration uses an indexed `expires_at`. Reads treat expired entries as absent; a maintenance job removes expired rows later. For replicated decisions, the evaluation timestamp is chosen once and passed as a command value. Local temporary KV remains a separate map because its isolation, availability, and disposable lifetime are deliberately different.

etcd provides transactions within its own KV store; those transactions do not encompass our SQLite database. Adding it would require another commit protocol or weaker cross-service guarantees. This is an architectural inference from the two independent stores, not a claim that etcd is unreliable. [etcd transaction API](https://etcd.io/docs/v3.6/learning/api/)

## Transaction contract for the first version

The application-facing transaction is a builder for one commit batch. It is not an unrestricted SQLite connection held open while application code makes arbitrary calls.

1. Read required state from one consistent snapshot through the adapter, obtaining its revision.
2. Compute business decisions without external side effects, allocating stable IDs and timestamps outside replicated SQL.
3. Prepare SQL writes and platform operations without publishing them.
4. Explicitly call `commit` with the operation ID, expected revision, and prepared changes.
5. In one replicated SQL batch, check the revision, claim the operation ID, apply all changes, advance the revision, and store the committed result.
6. Return success only after the replicated commit succeeds and the leader has applied it.

The adapter must implement an actual failing revision check, such as a controlled constraint failure, not an UPDATE that quietly affects zero rows. Every durable writer, including workers, TTL cleanup, scheduling, and migrations, must go through the adapter and participate in revision tracking. Revision and data reads must belong to the same snapshot. A global revision is a simple first implementation; it can cause unnecessary conflicts and should later become more selective if measurements justify it.

A revision conflict restarts the business decision from fresh state. An uncertain commit retries or resolves the same operation identity rather than creating a new operation. Operation records include an input fingerprint and enough result data to distinguish a retry from misuse of an ID. Deduplication retention must exceed the supported retry window.

This model preserves an explicit application commit and atomic changes across the tables. It limits interactive SQL: reading arbitrary pending writes and branching on intermediate write results are not first-version promises. Application-generated IDs and commit-time SQL expressions cover many ordinary cases, but this is a proposed clarification of the original open transaction semantics.

rqlite does not define application-controlled `BEGIN`, `COMMIT`, `ROLLBACK`, or savepoints across separate requests. Wrapping those calls around its API would not solve the problem. [rqlite transaction API](https://rqlite.io/docs/api/api/)

Replication must be deterministic. Pin SQLite and extension versions across nodes; prevent node-specific functions, arbitrary extension loading, and nondeterministic triggers or queries. Pass generated values as parameters. rqlite rewrites some nondeterministic SQL functions, but our contract must be narrower than “any SQLite statement is safe to replay.” [rqlite nondeterministic functions](https://rqlite.io/docs/api/non-deterministic/)

SQLite's session extension is a possible later route to replicating row changes instead of SQL. It is not the first recommendation: it excludes virtual tables, requires declared primary keys, and does not by itself provide consensus, schema migration, or crash recovery. [SQLite session extension](https://www.sqlite.org/sessionintro.html)

## Replication and degraded operation

Use three voting nodes for production HA and an explicitly separate one-node development configuration. A two-voter cluster cannot continue quorum writes after losing one voter; three voters can tolerate one failure. Raft is already included in rqlite, so do not run a second consensus cluster for coordination. [HashiCorp Raft](https://github.com/hashicorp/raft)

Each node serves public traffic and sends storage operations to the current leader through authenticated internal connections. Leader discovery and forwarding belong to the adapter. Strong reads require a quorum-backed read path and applied-state synchronization; merely noticing that a node believes it is leader is insufficient.

When quorum is unavailable:

- Replicated writes, queue claims, message acknowledgements, and scheduler mutations stop.
- Explicit stale reads may use the local applied replica through a controlled read-only path.
- Local temporary KV remains available.
- Sensitive reads can require the stronger replica guard.
- No production node bootstraps itself as a new one-node cluster to restore write availability.

The storage spike must prove the selected rqlite version's local-read path under leader loss, forwarding behavior, durable acknowledgement, uncertain outcomes, and snapshot recovery. These are integration acceptance gates, not assumptions about library packaging.

## Workers, events, and scheduling

A dispatcher transaction claims a message with a lease and a monotonically changing fencing token. After that claim commits, a worker executes application logic. Completion checks the token and atomically commits business changes together with the message acknowledgement. An expired worker cannot later commit using an obsolete claim.

Task execution can repeat after crashes. Database effects are protected by operation IDs, fencing, and atomic completion; external email, payments, or APIs need their own idempotency keys or compensation. No runtime can automatically roll back an already completed external request.

Failure updates are transactions too. Persist total attempt counts, next eligible time, the latest failure, and the explicit retain-or-delete disposition. Interrupted and uncertain attempts need a defined counting rule before implementation is accepted.

Pub/sub stores the event in the publishing transaction. Independent subscription cursors or delivery rows track each subscriber. Network notifications are wake-up hints; stored event state is authoritative. A crash between commit and notification must not lose the event.

Cron inserts a queue message and records its occurrence in one transaction. A unique identity based on schedule, execution scope, and occurrence prevents duplicate scheduling after leader changes. Skip, defer, and overlap behavior is implemented around committed claims. Per-server schedules include the server identity; application-wide schedules do not.

## Files and the atomicity boundary

Initially, store replicated file content in SQLite BLOB chunks. Present a platform file API, not a promise that these are ordinary operating-system paths. This keeps file publication inside the same transaction as SQL, KV, and messages.

Small files can be inserted directly in the commit batch. Large uploads require bounded chunks and explicit limits. A later staging path can replicate hidden immutable chunks first and atomically publish metadata referencing them; uncommitted chunks remain inaccessible, and garbage collection removes abandoned staging data. Reference protection and collection must use the same transactional storage rules.

Putting file bytes outside the database while committing only a pathname does not make the bytes atomic or durably replicated. If content-addressed files on disk are introduced later, replication, publication, and garbage collection need an additional proven protocol.

Local persistent and temporary files keep their weaker scope. First-version transactions must reject combinations that would promise atomicity across replicated SQLite and ordinary local filesystem writes. This is an explicit reduction of the unresolved mixed-scope contract, not a silent downgrade.

## Serving, certificates, and releases

Use Go's HTTP and socket facilities. Embed dashboard assets using Go's `embed` package. CertMagic provides certificate issuance and renewal as a Go library. Implement its shared certificate storage and locking against platform-managed state, including challenge routing and cached-certificate operation during quorum loss. [Go embed](https://pkg.go.dev/embed), [CertMagic](https://github.com/caddyserver/certmagic)

For the first application model, compile business logic into the Go executable. Run the executable in supervisor mode and worker mode. The supervisor owns public connections, certificates, and the storage engine; workers invoke native state operations through private IPC.

For an application release, the existing supervisor starts workers from the new executable, checks readiness, routes new requests to them, and lets old workers finish accepted work. Keep old and new code/schema contracts compatible during overlap. An already-running job can stay with its old worker generation. This is one executable per release with temporary process and version overlap, not a container platform.

Updating the supervisor itself needs a separate listener and storage-ownership handoff. Do not claim that application-worker switching automatically proves outage-free platform upgrades. Custom protocol handling requires equivalent delegation, and long-lived connection behavior remains explicitly scoped.

The SQLite Go driver documents static Linux builds with musl, making a self-contained executable a practical packaging target. Verify the actual integrated build's runtime dependencies rather than assuming every Go build is static. [go-sqlite3 build documentation](https://github.com/mattn/go-sqlite3)

## HTTP availability that storage cannot supply

Three replicated databases do not preserve a TCP connection terminated on a destroyed VPS. TCP connections belong to endpoints and their connection state; DNS listing additional addresses does not transfer that state. The resulting limitation is an architectural inference from TCP, not a database limitation. [TCP specification, RFC 9293](https://www.rfc-editor.org/rfc/rfc9293.html)

Choose an honest first-version contract: data and application service survive one node loss, while requests whose public connection dies may fail and require reconnection or retry. Internal retries can protect requests whose serving connection remains alive. Five seconds can be a measured target for defined failure scenarios, not a universal guarantee over unreachable paths.

If the original no-client-retry requirement remains mandatory, a separately surviving ingress and carefully scoped request buffering/replay contract are needed. That adds infrastructure and still requires explicit ingress-failure exclusions. Neither Raft nor an external load balancer alone proves uninterrupted completion of every request.

## Logs, metrics, backup, and resource limits

Keep diagnostic logs and routine telemetry outside the replicated application database. They have weaker guarantees and high write volume; forcing them through consensus would compete with business writes. Audit remains ordinary durable application data in the main database.

For the first backup implementation, export the complete application database consistently, then upload it with a versioned manifest, checksums, and required recovery metadata. A full database backup naturally includes application audit tables. Use the embedded storage's supported backup path; SQLite's backup API describes why a consistent database copy needs a database-aware operation. [SQLite backup API](https://www.sqlite.org/backup.html)

Whole-database backups are a proposed initial scope. The original configurable category selection needs later logical export and dependency rules, especially if audit tables are application-owned. Runtime logs remain excluded. Secrets need encryption with a recovery key independent of the destroyed cluster; backup inclusion and key recovery must be explicit.

Budget memory for SQLite caches, workers, replication buffers, file staging, and telemetry. Keep all queues and buffers bounded. Preserve disk reserve for replication logs and recovery, and reject new work before accepted data is endangered. The 2 GB and shared-CPU target is plausible as a design goal, but remains unproven until measured against a declared workload.

## Why not choose dqlite as the default

Dqlite is a genuine embeddable SQLite replication library, with quorum commit and static-linking support. It is the strongest alternative when conventional interactive SQL transactions are essential. [dqlite repository](https://github.com/canonical/dqlite)

However, its documented VFS keeps database images in process memory, and the Go bindings explicitly mark disk mode as unsupported from dqlite 1.18.3 onward. That conflicts with a potentially disk-sized dataset, SQLite file BLOBs, and 2 GB of RAM. It also targets Linux, complicating the same local platform on macOS. The marketing description of disk persistence must not be confused with bounded-memory database pages. [dqlite replication](https://canonical.com/dqlite/docs/explanation/replication), [go-dqlite disk-mode source](https://github.com/canonical/go-dqlite/blob/v3/node.go)

If datasets were explicitly capped to comfortably fit RAM and local development could run in Linux, dqlite would deserve another evaluation. Under the current requirements, start with disk-backed rqlite storage and the restricted transaction builder.

## Implementation sequence and proof gates

1. Embed the selected rqlite storage in a minimal executable. Prove one-node development, three-node replication, durable commits, consistent backups, local stale reads, and bounded memory with a dataset larger than the cache budget.
2. Implement the transaction adapter with operation IDs, atomic batches, revision checks, deterministic SQL restrictions, and input validation. Prove rollback and uncertain-commit retry behavior before adding convenience services.
3. Add KV, application audit examples, queues, events, and small replicated files as tables. Prove one commit changes them all or none, including consumed-message acknowledgement.
4. Add worker claims, fencing, failure policies, and cron. Prove accepted work survives worker and leader crashes, and stale workers cannot commit.
5. Add HTTP/TLS, application worker generations, configuration, secrets, and custom-protocol delegation. Verify release overlap and single-machine application-release continuity.
6. Add backup/restore, local telemetry, log queries, alerts, and the dashboard. Restore a fresh cluster from a completed external backup.

The decisive tests kill the leader before log persistence, after quorum commit, during database application, and after commit but before returning success. Also test partitions, full disks, snapshots, schema-version overlap, and duplicate operation IDs. After recovery, compare business state and completed operations across replicas.

Success means a measured working platform under the chosen contract. It does not mean silently satisfying the original impossible network guarantee or treating this proposal as production validation.
