# rqlite source review: SQLite integration, replication, and code size

Reviewed on 2026-10-05 by downloading and reading source code, not just product documentation. No build, failure test, or performance benchmark was run.

## Reviewed revisions

- rqlite: commit `812963feb363ea655521885dfd32212ebfb17983`, whose changelog starts with v10.5.2, dated October 4, 2026.
- SQLite driver: `github.com/rqlite/go-sqlite3` v1.51.0, commit `cee84f443e3b95fdd08e5853c797a02148e90380`.
- Consensus library: `github.com/hashicorp/raft` v1.8.0, commit `0f543c056e7e577a54b819a62116335c9661a13a`.
- The driver contains SQLite amalgamation version 3.53.4.

The pinned `go.mod` replaces the nominal `github.com/mattn/go-sqlite3` dependency with rqlite's fork. Counts below refer to these downloaded revisions, not a timeless estimate. [Dependency definitions](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/go.mod)

## Implementation stack

```text
HTTP API
  -> SQL parser/rewriter
  -> leader-forwarding proxy
  -> store.Store
  -> protobuf command encoding
  -> HashiCorp Raft
       -> durable Raft log in bbolt
       -> ordered committed entries
  -> store.FSM.Apply
  -> Store.fsmApply
  -> CommandProcessor.Process
  -> db.SwappableDB
  -> db.DB
  -> Go database/sql
  -> rqlite/go-sqlite3 via cgo
  -> SQLite C implementation
```

rqlite's own orchestration is Go. SQLite is C. The Go driver contains C-call bridges and the SQLite amalgamation. There is no separately started SQLite database server.

The public HTTP interface is distinct from the embedded `store` package. Request forwarding is implemented by the proxy and cluster code, not by SQLite itself. [Proxy implementation](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/proxy/proxy.go)

## What is actually replicated

A normal write log entry contains an encoded SQL request, not the resulting row changes. The inner request carries:

- The transaction flag.
- An ordered list of SQL statements.
- Typed, optionally named parameters, including byte values for BLOBs.
- Database timeout and other request options.

The outer command carries a command type, inner payload bytes, and a compression flag. Protobuf supplies serialization; the request marshaler optionally gzip-compresses the inner payload when its thresholds and compression benefit warrant it. [Command schema](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/command/proto/command.proto), [Marshaller](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/command/marshal.go)

One batch is one Raft command, but it is only one atomic database transaction when its transaction flag is set. Without that flag, individual statements can commit separately even though they occupy one Raft entry.

## Exact normal write path

1. `http.Service.execute` parses SQL and parameters, invokes `command/sql.Process`, and constructs `ExecuteRequest`.
2. `proxy.Proxy.Execute` tries the local store. If it receives `ErrNotLeader`, it forwards to the current leader through the internal cluster client.
3. `Store.Execute` checks disallowed PRAGMAs, cancellation, leadership, and readiness, then calls `Store.execute`.
4. `Store.execute` serializes the request, wraps it in a `COMMAND_TYPE_EXECUTE` envelope, and calls `s.raft.Apply`.
5. HashiCorp Raft persists and replicates the entry, determines commitment, and sends committed entries to its FSM worker.
6. `FSM.Apply` delegates to `Store.fsmApply`. That calls `CommandProcessor.Process`, which decodes the command and invokes the database wrapper.
7. `db.executeWithConn` opens a local SQLite transaction if requested, executes the ordered statements with their parameters, rolls back on error, or commits on success.
8. The FSM result completes the leader's apply future. `Store.execute` returns that result and the Raft index; the proxy and HTTP layer return it to the caller.

SQL is executed after the Raft command is committed. A committed log entry may therefore contain an SQL request that subsequently fails and rolls back; Raft commitment is not the same as SQL success. Replicas may apply later than the leader. Success does not require every replica to finish executing the batch.

The pinned source locations are [HTTP request handling](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/http/service.go#L1391), [Store.execute](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/store/store.go#L1502), [command dispatch](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/store/command_processor.go#L48), and [local transaction execution](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/db/db.go#L1171).

## How SQLite is wrapped

`db.DB` owns two `database/sql` handles for the same database file: one read/write handle restricted to one open connection, and a separate read-only pool. `SwappableDB` adds locking so restoration can replace the database without callers using a closed handle. [Database opening](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/db/db.go#L249), [SwappableDB](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/db/swappable_db.go)

The custom driver registration installs connection hooks. On each new connection, including replacements, it disables SQLite's automatic WAL checkpointing. The normal driver also disables checkpoint-on-close; rqlite decides when database pages and WAL files can be checkpointed and retained. [Driver setup](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/db/driver.go)

The driver invokes SQLite C functions through cgo. Local transaction commit is ultimately an SQL `COMMIT` executed against SQLite. The wrapper does not intercept a normal application commit and magically turn it into Raft replication: replication has already happened before the wrapper starts executing the request. [SQLite driver](https://github.com/rqlite/go-sqlite3/blob/cee84f443e3b95fdd08e5853c797a02148e90380/sqlite3.go)

## Two different logs and the recovery mechanism

The Raft log and SQLite WAL serve different purposes:

| Log | Contains | Role |
| --- | --- | --- |
| Raft log, stored in bbolt | Ordered replicated command payloads and Raft metadata | Durable consensus history |
| SQLite WAL | Locally changed database pages | SQLite transaction storage and incremental snapshot input |

The bbolt store is an embedded dependency, not an etcd server or a second public application data service. [Raft log store](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/store/log/log.go)

In steady state rqlite uses SQLite WAL mode with `synchronous=OFF`. Durability depends on the replicated Raft log and retained snapshots. Snapshot creation switches SQLite to `FULL`, performs coordinated checkpointing, then restores `OFF`.

Snapshots can start from a complete database and later incorporate compacted WAL data. The checkpoint manager handles bounded waits, incomplete checkpoints, and WAL resets. Restart either reuses a verified database corresponding to a clean snapshot or restores from snapshot and replays subsequent log entries. Snapshot restoration uses a temporary file before swapping the database. This recovery machinery explains much of the code beyond the small FSM dispatcher. [Snapshot creation and restoration](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/store/store.go#L2612), [Checkpoint manager](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/db/checkpoint_manager.go), [Snapshot restore](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/snapshot/restore.go)

## Correctness details that matter when embedding it

- SQL rewriting happens in the HTTP layer before the store call. Calling `Store.Execute` directly bypasses that step; an embedding application must generate deterministic commands itself or explicitly perform the required processing. [SQL processing](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/command/sql/processor.go)
- Ordinary SQL errors are returned as operation results. Node-local failures, such as disk-full, I/O, corruption, or escaped write-lock failures, are classified as fatal; application of the committed command stops the process to avoid replica divergence. [Error classification](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/db/db_errors.go)
- The apply-future timeout is a queueing/start timeout in HashiCorp Raft, not an overall deadline proving rollback. Leadership loss can leave the caller uncertain whether its write survived. A platform still needs a retry and deduplication contract. [Raft Apply API](https://github.com/hashicorp/raft/blob/0f543c056e7e577a54b819a62116335c9661a13a/api.go#L853)
- CDC captures changes during local execution and emits them afterward. It is not the normal replication mechanism. [FSM application and CDC](https://github.com/rqlite/rqlite/blob/812963feb363ea655521885dfd32212ebfb17983/store/store.go#L2529)

## Changeset support already present in the driver

The pinned driver has `sqlite3_session.go`, with **137 code lines**. It exposes session creation, table attachment, changeset generation, and iteration over old and new row values. [Session binding](https://github.com/rqlite/go-sqlite3/blob/cee84f443e3b95fdd08e5853c797a02148e90380/sqlite3_session.go)

The reviewed rqlite production code does not call `CreateSession` or `NewChangeset` in its replication path. The reviewed driver's Go files also provide no changeset-apply binding. The underlying SQLite amalgamation contains the C extension, but that is not a complete replicated changeset runtime.

This gives our proposed runtime a useful starting point for capturing transaction results. We would still implement changeset application, conflict handling, schema-version handling, and the safe prepare/replicate/apply lifecycle.

## Measured code size

Measured with `cloc 2.10`, counting code lines rather than comments or blank lines. Duplicate checking was disabled. This is repository source volume, not linked binary size or the minimal reachable implementation. Raw measurements and classification are recorded in `RQLITE-CODE-SIZE.json`.

| rqlite category | Go code lines |
| --- | ---: |
| Authored production source | 25,073 |
| Unit/integration test files outside support directories | 47,219 |
| Generated Go source | 3,558 |
| System-test directory, including its helpers | 5,425 |
| Test fixtures, including large embedded fixture literals | 16,021 |
| Tool source and tool tests | 223 |
| All counted Go source | 97,519 |

Authored production source is 153 files and 33,326 physical lines including comments and blanks. The complete Go source is 116,858 physical lines. Source outside Go and dependencies are not included in this table.

| Production package, including its subpackages | Code lines |
| --- | ---: |
| `db`: SQLite wrapper, WAL processing, hooks, utilities | 4,000 |
| `store`: Raft/SQLite integration and lifecycle | 3,525 |
| `snapshot`: snapshot storage, compaction, restore | 3,201 |
| `command`: encoding, SQL processing, chunking | 1,025 |
| Subtotal of these four areas | **11,751** |
| HTTP API | 2,344 |
| Cluster communication and forwarding service | 1,772 |
| TCP transport utilities | 486 |
| Leader proxy | 232 |
| Remaining production areas | 8,488 |
| Total | **25,073** |

The four-area subtotal is a useful scale reference for the storage integration, not a self-contained extraction: it imports internal utilities, dependencies, and transport interfaces.

| Particularly relevant file | Code lines |
| --- | ---: |
| `store/store.go` | 2,516 |
| `db/db.go` | 1,929 |
| `db/swappable_db.go` | 206 |
| `command/sql/processor.go` | 207 |
| `command/marshal.go` | 175 |
| `db/checkpoint_manager.go` | 131 |
| `db/driver.go` | 129 |
| `store/command_processor.go` | 129 |
| `store/fsm.go` | 66 |

Separate dependency measurements:

| Dependency | Code lines | Scope |
| --- | ---: | --- |
| HashiCorp Raft v1.8.0 runtime source | 6,158 | Excludes tests, benchmarks, compatibility package, fuzzy harness, and testing helpers |
| HashiCorp Raft test files | 6,367 | `_test.go` files |
| rqlite SQLite driver v1.51.0 | 4,057 | Non-test Go source, excluding examples and upgrade tool; includes optional build variants |
| SQLite C amalgamation, `sqlite3-binding.c` | 170,376 | SQLite engine bundled in the driver |

The driver Go count excludes C code inside cgo comment preambles under cloc's Go classification. Other dependencies, including bbolt, protobuf, the SQL parser, compression libraries, and cloud SDKs, were not counted. Adding the table values does not give the exact source or size of a compiled binary.

## Implications for our platform

rqlite is a relatively compact Go application, but durable SQLite/Raft integration is several coordinated subsystems. Its 66-line FSM bridge is not evidence that replication correctness takes only dozens of lines.

For SQL batches and table-backed KV, audit, queues, and BLOB files, its existing command path is directly relevant. For running application logic inside a preparatory SQLite transaction and replicating resulting changesets, we need a new command path and transaction lifecycle. That is a substantive change, not merely a new JSON envelope around existing `Store.Execute`.

Reuse the Raft library, driver, encoding patterns, error handling, and carefully reviewed recovery mechanisms. Keep one clearly chosen replication model. Decide whether to retain SQL-command replication or implement changeset replication before integrating application execution deeply into storage.
