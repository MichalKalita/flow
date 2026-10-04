## Layer 1 — Minimal core: application runtime and state management

- Dependency: the server, its storage, and its network; no higher platform layer.
- Layers describe logical dependencies based on `BLACKBOX-REQUIREMENTS.md`, rather than prescribed modules, technologies, or implementation order. A higher layer uses the lower layers it needs, without necessarily depending on all of them.
- Runtime for application logic written specifically for the platform's native contracts; no programming language is prescribed.
- The same application code and operations locally, on a single development machine, and across multiple VPS instances; configuration and declared durability guarantees may differ.
- A unified application transaction with explicit commit, rollback, and participation by higher-layer services; all participating changes commit together or none do.
- Separation of pending changes from committed application state and completed work.
- Durable storage, recovery of interrupted commits, and prevention of duplicate application state changes when the platform internally retries the same logical operation.
- Replication: every healthy, synchronized replica holds a complete copy of the replicated dataset; the dataset must fit on one server.
- HA write acknowledgement only after durable storage on multiple machines; an acknowledged write survives the loss of one VPS.
- An explicit development mode that acknowledges local durable storage without waiting for another machine.
- Distinct replicated, local persistent, and local temporary state guarantees; transaction atomicity does not increase the durability of local data.
- Degraded HA operation with only one reachable member: reads and local temporary key-value writes remain available; SQL writes, replicated file writes, replicated key-value writes, and audit appends are not committed or acknowledged as successful.
- No automatic downgrade from replicated writes to local writes; a mixed transaction must not commit only its permitted local portion.
- An application-facing replica-status guard that can stop a protected operation; its exact condition, read-freshness guarantees, and development-mode behavior remain open. Commit requirements are enforced independently of an earlier guard check.
- Safe rejection of writes or new work when capacity is insufficient, with a clear reason and information for an administrator alert; committed data, accepted work, and atomicity are preserved.
- Guarantee boundary: native managed state follows its declared contracts; external APIs, direct unmanaged storage, and external side effects do not inherit transactional rollback.
- The complete platform must fit approximately one shared CPU and 2 GB of RAM per VPS; the intended small HA deployment uses two or three VPS instances, with the exact count and supported workload still open.
- Exact transaction isolation, read consistency, concurrent writes, transaction limits, caller-disconnection outcomes, and complete network-partition rules remain open.

## Layer 2 — Native data services

- Dependency: the runtime, transactions, persistence, replication, and capacity rules of layer 1.
- SQL database as a mandatory foundational service; exact SQL behavior and isolation rules remain open.
- Files in the directory scopes `replica`, `persistent server`, and `temporary server`; the destination directory determines write guarantees.
- `replica`: durable storage on multiple machines in HA operation and survival of one VPS loss; local durable storage in development mode.
- `persistent server`: durable storage on the current server and survival of restarts with intact storage; destruction of that server's storage may permanently lose the file.
- `temporary server`: local temporary files without a restart-durability guarantee; lifetime and cleanup remain open.
- Key-value storage with an explicit choice of replicated or local temporary scope and configurable TTL in both scopes.
- Local temporary key-value entries are not shared, replicated, or merged after reconnection; exact expiration, eviction, and behavior without TTL remain open.
- Append-only audit logging with transactional appends, replicated persistence, and mandatory backup under the configured policy; record contents, access controls, and retention remain open.
- Queues: transactional acceptance of new work, stored message state, and transactional acknowledgement of completed processing; receiving a message does not itself acknowledge completion.
- Every queue must declare its maximum total attempts, delay between attempts, and final retain-or-delete disposition; incomplete definitions are rejected.
- A retained message with an exhausted attempt budget remains stored and inspectable in a failed state, without further automatic processing; exact retention and manual replay interfaces remain open.
- Pub/sub for multiple independent subscribers; transactional publication occurs only after successful commit, and rollback publishes nothing. Delivery and subscriber processing need not finish before the publisher's commit completes.
- Native configuration and secrets available through the same application-facing contract in every deployment mode; protection, rotation, activation, backup, and transaction participation remain open.
- Concurrent file access, transactions mixing local and replicated scopes, message ordering, delivery guarantees, and counting interrupted attempts remain open.

## Layer 3 — Application serving and work execution

- Dependency: the application runtime and guarantees of layer 1, and native state and queues from layer 2.
- Integrated HTTP and HTTPS, multiple domains, URL routing, static files, and WebSockets; general-purpose HTTP streaming remains open.
- Automatic HTTPS certificate provisioning, renewal, and activation without interrupting request service; this is a mandatory foundational core capability rather than an optional plugin.
- Certificates require domain-validation and issuance prerequisites to be satisfied; issuance-failure behavior, local development, and certificate distribution in degraded states remain open.
- Application-defined TCP and UDP services on configurable public ports, such as MQTT or email protocols, without requiring built-in MQTT broker or email-server implementations.
- A stable public application address accessible through ordinary browsers and standard protocol clients; public DNS is permitted, and no special HA client is required.
- Request and protocol processing with access to native data services and unified commit.
- Workers and long-running asynchronous processing through queues; accepted work survives compatible releases and covered VPS failures under its declared policy.
- A failed transactional attempt leaves no committed application changes or successful message-completion acknowledgement; subsequent processing follows the queue's policy.
- Execution support for pub/sub publication and delivery after commit, with independent subscriber processing.
- Compatible build or module releases without an outage, including on a single machine; configuration or secret changes may trigger the same release process.
- HTTP request continuity during compatible releases and covered single-VPS failures: otherwise fast valid requests, including in-flight requests, complete successfully within five seconds without client retries or transition-induced errors.
- The one-member degraded state and capacity exhaustion retain their explicit exceptions; prohibited operations must not be falsely acknowledged.
- Long-lived connections may be disconnected during releases; recovery, custom protocols, unreachable network paths, and routing to a failed VPS need separate contracts.
- Only applications written for the platform's contracts are supported; Dockerfiles, container images, Docker Compose definitions, and Kubernetes manifests are unsupported deployment inputs.
- Routing, backend execution, database, file storage, messaging, and workers are included without separately operated services; permission to require external traffic routing has not been established.
- Custom-protocol security, release and data compatibility, release rollback, in-progress work, and simultaneous releases and failures remain open.

## Layer 4 — Scheduling, telemetry, and external recovery

- Dependency: data services from layer 2 and request, worker, and release execution from layer 3, under layer 1 guarantees.
- Cron scheduling through native queues; each definition has its own logically separate system-managed queue, independent failure policy, and independently observable backlog.
- Scheduled-task scope: once per application deployment by default, or independently on each server when explicitly selected.
- Mandatory overlap policy: `skip` discards that scheduled occurrence, `defer` queues every occurrence for sequential execution, and `allow overlap` permits concurrent executions; definitions without a selection are rejected.
- Configurable recovery of missed occurrences: skip them, run one catch-up execution, or replay all of them, while respecting the overlap policy.
- Observable pending scheduled executions, backlog growth, waiting time, and execution duration; the platform must not silently change the policy or discard deferred occurrences.
- Append-only diagnostic/runtime logs independent of commit and rollback; they are neither replicated nor backed up and may have gaps or be lost with their server.
- Application metrics collection enabled in a standard installation; it may be provided by an included plugin without an external monitoring stack.
- Transaction-associated metrics are applied only after successful commit as a best-effort step; rollback does not apply them, and losing a metric update after commit does not undo committed data.
- Independent operational metrics, such as request duration, memory usage, and failed transactions.
- Tracing of a logical operation across requests, SQL, queues, and asynchronous processing.
- Integrated backups to an external S3-compatible destination independent of application VPS instances, with configurable contents and automatic frequency; audit records are backed up, while runtime logs are excluded.
- Documented, repeatable restoration of selected backup contents to a coherent, usable state, including after all VPS instances are destroyed; newer data may be lost, and no fixed restoration deadline is required.
- Observable success or failure of backup and restoration; an external backup destination is not a prerequisite for serving ordinary application requests.
- Optional external S3-compatible application file storage with a separate contract; its operations are outside native transaction atomicity.
- A desired helper for best-effort compensation of external S3 operations after definitive transaction failure; compensation may fail and leave external effects. A missing commit response alone must not trigger compensation.
- Exact schedule expressions, time zones, catch-up rules, partition coordination, telemetry formats and retention, backup selection and retention, and restoration targets remain open.

## Layer 5 — Derived operational capabilities and limits

- Dependency: native state from layer 2, application operation from layer 3, and telemetry and scheduling from layer 4.
- Historical log retrieval and filtering, including periods such as the preceding 30 days; a mandatory minimum retention period has not been established.
- Distinction between complete results and available records with gaps; local logs from an unavailable VPS may be missing and may be permanently lost if its storage is destroyed.
- Derived status of servers, replication, degraded operation, request serving, storage, queues, scheduled tasks, releases, and backups.
- Operational access to metrics and traces; exact queries, aggregation, visualizations, and interfaces remain open.
- Traffic and resource limits using lower-level facilities such as key-value storage; these may be supplied by a plugin.
- Limit guarantees follow the underlying service: local temporary key-value storage does not provide application-wide coordination or a strict global limit.
- Administrative interfaces for controlling lower-layer capabilities while preserving their transaction, capacity, durability, and availability contracts; no specific APIs are prescribed.
- Limit scope, enforcement, permitted overshoot during disconnection, and availability of operational information remain open.

## Layer 6 — Alerting

- Dependency: status and operational queries from layer 5, metrics and scheduled-work information from layer 4, and capacity events from layer 1.
- Administrator-defined alert rules and evaluation of their conditions.
- Default warnings for scheduled-execution backlogs and scheduled tasks running too long.
- Alerts for capacity exhaustion that prevents safely accepting new writes or work.
- Distinction between observed state and missing, delayed, or incomplete information.
- Exact thresholds, evaluation intervals, notification channels, acknowledgement, alert recovery, and alerting availability remain open.

## Layer 7 — Administrator dashboard

- Dependency: operational views and controls from layer 5, alerting from layer 6, and managed capabilities from lower layers.
- Overview of servers, replication, degraded states, request serving, storage, queues, scheduled tasks, logs, metrics, traces, releases, and backups.
- Historical log retrieval and filtering with visible identification of missing or unavailable information.
- Configuration and secrets management, and initiation of releases, backups, and restoration.
- Queue and scheduled-task controls, inspection, and manual replay of retained failed work.
- Custom alert-rule management and alert display.
- Full administration of integrated capabilities while preserving their declared guarantees; the dashboard and alerting are higher-level capabilities requiring no separately operated monitoring services.
- Administrator authentication, roles, permissions, exact screens, and dashboard availability remain open.
- Built-in application identity, sessions, application-user authorization, and a separate workflow engine are not confirmed platform requirements.
