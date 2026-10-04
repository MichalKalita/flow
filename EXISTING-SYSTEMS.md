# Existing systems related to the blackbox requirements

Status: Survey of publicly available products and platforms as of 2026-10-04. This note records whether anything already supplies behaviour similar to `BLACKBOX-REQUIREMENTS.md`. It is not a requirements change.

## Conclusion

No existing product implements the full combination in the draft:

- a self-contained installation on a basic VPS (about one shared CPU and 2 GB RAM)
- a two- or three-VPS high-availability deployment with quorum writes and a one-member degraded state
- one explicit application commit spanning SQL, files, key-value storage, queues, pub/sub, and audit records
- applications written for the platform contracts, with Docker/Kubernetes excluded as application deployment inputs
- HTTP plus application-defined TCP/UDP services, automatic HTTPS, logs, metrics, dashboard, and external backup

Several products cover individual layers. Using them still means assembling services and giving up all-or-nothing commit across SQL, files, and queues, or accepting a much heavier hosting model than the resource baseline.

## Distinctive combination in the draft

The draft is hard to match because the unusual parts arrive together:

1. Unified commit across SQL, replicated files, key-value storage, queues, pub/sub publication, and audit appends.
2. Self-containment on a cheap VPS, without a separately operated router, database, broker, or monitoring stack.
3. Small-cluster HA (intended scale two or three VPS instances), with acknowledged replicated writes surviving loss of one machine, and with the one-survivor state limited to reads plus local temporary key-value writes.
4. The same application code in local development and HA, with an explicit local-durability exception in development.
5. Custom network protocols, not only HTTP.
6. Compatible updates with client-visible continuity (otherwise fast requests complete within five seconds).
7. Explicit rejection of Kubernetes/container packaging as the application model.

## Closest systems by layer

### Unified transactions across more than one service

These are the nearest matches for the mandatory unified commit, and they still stop short of SQL plus files plus queues plus pub/sub plus audit.

| System | What it unifies | Gap versus the draft |
| --- | --- | --- |
| [Azure Service Fabric Reliable Collections](https://learn.microsoft.com/en-us/azure/service-fabric/service-fabric-reliable-services-reliable-collections) | One transaction over replicated dictionaries and queues inside a partition | No SQL, no file scopes, no small-VPS self-contained runtime. Enterprise cluster, not a 2 GB application platform. |
| [Oracle AQ / Transactional Event Queues](https://www.oracle.com/database/advanced-queuing/) | SQL changes and enqueue/dequeue in one database transaction | Heavy enterprise database. Not an application platform, files, HTTPS, custom protocols, or 2 GB VPS baseline. |
| [Spanner queues](https://cloud.google.com/blog/products/databases/spanner-queues-provide-native-transactional-messaging) | SQL state and enqueue in one Spanner transaction | Hosted Google service. Not self-contained, not a full application runtime. |
| [FoundationDB](https://apple.github.io/foundationdb/technical-overview.html) plus [Record Layer](https://foundationdb.github.io/fdb-record-layer) | Strict ACID over a distributed key-value store; SQL and queues can be built as layers (for example Apple QuiCK-style queues) | A storage substrate, not a complete application platform. Files, HTTP, certificates, dashboard, and custom protocols are out of scope. |
| Postgres with `SKIP LOCKED` queues, `LISTEN`/`NOTIFY`, and transactional outbox | SQL, queue rows, and event outbox in one database transaction | Files and external brokers remain outside the commit. HA, serving, and operations are still assembled separately. |

[Dapr transactional outbox](https://docs.dapr.io/developing-applications/building-blocks/state-management/howto-outbox) can commit a state-store write together with a pub/sub publication. It is a sidecar pattern, typically on Kubernetes, and does not span SQL, files, queues, and audit as one application commit.

### Application server on a small machine

| System | Fit | Gap versus the draft |
| --- | --- | --- |
| [Tarantool](https://www.tarantool.io/) | In-process Lua application server plus ACID storage, queues, and Raft/synchronous replication. Can run on small VPS instances. | No product-level unified commit with files. No first-class HTTPS certificates, administrator dashboard, custom TCP/UDP application services, or the draft's one-member degraded-state contract. |
| [PocketBase](https://pocketbase.io/) | Single-binary SQLite backend, files, realtime, admin UI; realistic on a small VPS | Official product is single-node. Community [pocketbase-ha](https://github.com/litesql/pocketbase-ha) replicates through NATS with last-writer-wins, which does not meet “acknowledge only after durable storage on more than one machine”. |
| SQLite HA pieces: LiteFS, Litestream, Turso/libSQL, rqlite, dqlite | Replicated or shipped SQLite | Database replication only. Not an application platform. |

### Write-to-the-platform runtimes

These match the draft's rule that applications must use platform contracts, and still miss unified commit or self-contained small-cluster HA.

| System | Fit | Gap versus the draft |
| --- | --- | --- |
| [Encore](https://encore.dev/) | SQL, pub/sub, object storage, cache, cron, and secrets declared in application code. Local `encore run` starts matching infrastructure. | Production maps those primitives onto AWS/GCP services (RDS, SQS/SNS, S3, and so on). No unified commit across services. Self-hosting means supplying that infrastructure yourself. |
| [Convex](https://www.convex.dev/) | Transactional reactive backend. Applications are written for its model. Self-hosting exists. | Document/reactive store rather than required SQL. No three-VPS HA contract as specified. |
| Classic Google App Engine | Datastore, memcache, task queues, logs; applications had to follow the platform model | Hosted only. Historical analogue, not a self-hosted VPS product. |
| Cloudflare Workers plus D1, R2, KV, Queues, and Durable Objects | Integrated primitives, applications written for the platform | Serverless and vendor-hosted. No unified transaction across those primitives, and not a self-contained VPS install. |

### Durable execution

These cover asynchronous work, retries, and recovery of in-flight logic. They are not a substitute for the data platform.

| System | Fit | Gap versus the draft |
| --- | --- | --- |
| [DBOS](https://www.dbos.dev/) | Open-source library on Postgres: workflows, queues, cron, and transactional steps checkpointed in the same database | HTTP serving, files, custom protocols, certificates, and HA clustering remain the operator's problem. Unified commit applies to Postgres state, not files. |
| [Restate](https://restate.dev/) | Single Rust binary with a replicated log, virtual objects, workflows, timers, and durable messaging | SQL and files stay outside the engine. Combining them requires sagas/compensation, not one platform commit. |
| [Temporal](https://temporal.io/) | Durable workflows with a separate cluster | Needs backing stores (Postgres/Cassandra, Elasticsearch). Not SQL, files, or a 2 GB self-contained platform. |

### Batteries-included BaaS

These resemble the list of integrated capabilities (HTTP, SQL or documents, files, functions, logs) without the transaction and small-cluster HA contracts.

| System | Fit | Gap versus the draft |
| --- | --- | --- |
| [Supabase](https://supabase.com/) | Postgres, file storage, realtime, functions, dashboard; self-hostable | Self-hosting is a multi-container stack, not a 2 GB baseline. Transactions stop at SQL. Storage and async work are separate. |
| [Appwrite](https://appwrite.io/), [Nhost](https://nhost.io/) | Similar BaaS shape, self-hostable | Same assembly and transaction limits. |
| Firebase | Hosted documents, functions, files, realtime | Hosted, no SQL requirement, no unified commit, no small-VPS HA. |

### Hosting and PaaS

Coolify, Dokku, CapRover, Cloudron, Railway, Fly.io, Render, and Heroku deploy or host applications. The draft excludes Dockerfiles, images, Compose, and Kubernetes manifests as application inputs, and excludes drop-in hosting of existing applications. These tools do not supply the application-facing contracts.

CockroachDB, Yugabyte, and Patroni/Postgres provide SQL HA. They still leave files, queues, pub/sub, serving, and operations as separate systems.

Erlang/OTP with Mnesia, Microsoft Orleans, and Akka Cluster can host distributed applications with replicated state. They are runtimes and libraries, not the integrated platform described in the draft.

## Practical compositions if the full platform is not built

These are the shortest paths that reuse existing software. Each gives up at least one confirmed requirement.

1. **Tarantool as the runtime**, with a custom layer for HTTP, files, certificates, and operations. Closest to a small self-contained HA application server. Unified commit with files and the degraded-state contract would still be original work.
2. **Postgres plus DBOS** for SQL, queues, workflows, and cron, plus Cockroach or Patroni for SQL HA, plus MinIO or local files. Transactions hold only what lives in Postgres.
3. **Restate** for durable messaging and virtual objects, plus a separate SQL database. Strong exactly-once execution; weak unified commit with SQL and files.
4. **Encore** if the priority is the same application code locally and in production, and HA is delegated to AWS or GCP primitives.

## Requirement coverage

| Draft requirement | Nearest existing coverage | Remaining gap |
| --- | --- | --- |
| One commit across SQL, files, KV, queues, pub/sub, audit | Service Fabric (KV + queue); Oracle/Spanner (SQL + queue); FoundationDB as a substrate | Files and audit in that same commit are not a product |
| Self-contained on ~2 GB RAM | Tarantool, PocketBase, Restate engine | Not the full capability set, and not the HA acknowledgement contract |
| Two or three VPS, quorum write, one member read-only for replicated data | Cockroach, FoundationDB, Tarantool synchronous replication, Patroni | Not packaged as an application platform with files and queues |
| Applications written to platform contracts | Encore, Convex, DBOS, Restate, App Engine | Either cloud primitives or execution-only |
| Custom TCP/UDP plus HTTP, certificates, dashboard, backup | No single product | Would have to be assembled or written |
| Compatible update with five-second request continuity | Some service meshes and rolling-deploy platforms, under extra infrastructure | Not on the resource baseline, and not tied to the unified data contract |
| Reject Docker/Kubernetes as application inputs | PocketBase, Tarantool, classic PaaS buildpacks in spirit | Those products do not provide the rest of the contract |

## Why a complete equivalent is missing

The unusual demand is not “SQL plus HA” or “BaaS on a VPS”. It is all-or-nothing commit across SQL, files, and queues, acknowledged only after multi-machine durability, on a two- or three-node 2 GB VPS cluster, with five-second request continuity and custom protocols, without Kubernetes as the application model.

Existing products split that problem:

- databases unify SQL and sometimes queues, then leave files and serving outside
- BaaS products unify APIs, then leave transactions at the database boundary
- durable-execution engines unify retries, then leave SQL and files to the operator
- PaaS products unify deployment of arbitrary apps, which the draft excludes

Building the draft therefore means writing a new platform, or composing existing pieces and weakening the unified-commit and resource-baseline requirements.
