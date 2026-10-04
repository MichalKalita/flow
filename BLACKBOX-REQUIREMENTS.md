# Self-contained high-availability application platform

Status: Working draft, revised after twenty-five requirements discussions. Open questions are not accepted requirements. Applications must be written specifically for the platform's contracts.

## Purpose and scope

Define the externally observable behavior of a self-contained application platform that supplies the recurring building blocks of application software: request serving, business logic execution, SQL, files, messaging, key-value storage, logging, metrics, asynchronous work, availability, and recovery.

A developer should be able to supply application business logic and its configuration without separately assembling or operating these foundational services. The application may perform extensive asynchronous processing and implement network services beyond HTTP, including MQTT or an email server. The specification imposes no programming-language choice.

This specification describes what users, clients, and operators can observe and rely on. It does not prescribe programming languages, internal architecture, storage engines, or a particular supported build or module-release mechanism. The explicit application compatibility and deployment exclusions below apply regardless of implementation choices.

## Mandatory application model and explicit exclusions

**This platform is NOT a replacement for Kubernetes, a general-purpose container orchestrator, or a drop-in hosting environment for existing applications.**

Applications MUST be written specifically to comply with the platform's application-facing contracts, including its native state services, explicit unified commits, storage scopes, asynchronous execution, replication requirements, and external-side-effect boundaries. Application code that does not follow those contracts cannot be expected to function correctly and is outside the supported application model.

Existing applications that do not comply MUST be rewritten for this model. Merely packaging, uploading, or configuring an existing application is not sufficient to make it compliant or give it the platform's transaction and HA guarantees.

**The platform MUST NOT accept Dockerfiles, container images, Docker Compose definitions, or Kubernetes manifests as application deployment inputs.** Running an arbitrary existing container or server workload is outside the required capabilities. Freedom to choose the application programming language does not remove the requirement to use the platform's application model.

These exclusions define what application developers may deploy. They do not prescribe the platform's internal implementation. Supported compliant applications may still be released as complete builds or modules, as stated elsewhere in this specification.

## Confirmed requirements

### Resource baseline and self-containment

The platform must support a complete installation on a single basic VPS with approximately one shared CPU and 2 GB of RAM.

The platform must also support a deployment across multiple VPS instances that continues serving the application after the loss of one VPS. A small deployment of two or three VPS instances is the intended scale; the exact required number is not prescribed at this stage. Each instance must fit the same basic VPS resource baseline. The supported workload remains to be defined.

The complete application dataset must fit within the storage capacity of one server. Each healthy, synchronized replica must hold a complete copy of the platform-managed replicated dataset; distributing different subsets across servers is not required. Increasing the number of replicas provides redundancy rather than increasing the required dataset capacity. Server-local persistent files, temporary files, and runtime logs retain their explicitly non-replicated policies. Files stored in an optional external S3-compatible service follow that service's separately agreed contract.

### Application prerequisites and guarantee boundary

The complete server-side dataset must fit on a single server. This is a prerequisite of the intended deployment model, not a requirement to partition the dataset across servers or keep all stored data in RAM.

All server-side application state must use the platform's native capabilities: SQL, file directory scopes, key-value storage, queues, pub/sub, configuration, secrets, logs, and metrics as applicable. Their declared policies determine transaction participation, replication, retention, and recovery. Using a native capability does not remove its explicitly weaker guarantees, such as server-local files or runtime logs.

The platform's transaction, rollback, HA, and recovery guarantees apply only to operations and state governed by those native contracts. Application code must not assume that direct unmanaged storage access or an external API call inherits these guarantees.

External integrations are permitted, but must be used deliberately with an explicit understanding of their independent effects. A transaction rollback does not inherently undo an external request. A compensation helper may itself fail or leave external effects behind; retrying application execution may also repeat an external action. These limitations must remain visible in the application-facing contract rather than be hidden behind a claim of global atomicity.

The permitted external S3 file and backup destinations retain their separately defined contracts. They do not expand the native transaction guarantee to arbitrary external services.

The same application code must run on a developer's computer, in a single-machine development environment, and in a multiple-VPS deployment. A separate application implementation for development or high availability must not be required. Deployment configuration may differ. Application-facing operations must remain the same, with explicitly declared durability differences between single-machine development and HA operation.

Compatible updates must meet the no-outage guarantee even in a single-machine installation. Availability after complete machine failures is specified separately from update availability; continued service after the loss of the only machine is not a confirmed requirement.

All required application-serving capabilities must be included in the platform. No separately operated application router, backend, database, file store, message broker, key-value service, job runner, or log service may be required. Public DNS is an allowed external dependency. Permission to require any externally operated traffic-routing service has not been established.

An external backup destination is explicitly permitted. External backup must not become a prerequisite for serving ordinary application requests; configured backup obligations apply separately.

An external S3-compatible service from any provider is also permitted as an optional application file-storage destination. The platform must still support self-contained file storage without that dependency.

### Integrated capabilities

The platform must provide:

- Integrated HTTP request routing and serving, including HTTPS, multiple domains, routing by URL path, static files, and WebSocket connections. General-purpose HTTP streaming remains to be confirmed.
- Server-side execution of application business logic and data processing.
- Application-defined network services and protocols beyond HTTP and WebSockets, such as MQTT or email protocols. These examples require extensibility, not built-in MQTT broker or email-server implementations.
- A SQL database as a foundational capability. SQL is required, rather than merely one acceptable storage option. The platform must not require a MongoDB-style database. Required SQL behavior and transaction semantics remain to be defined.
- A mandatory unified transaction capability with explicit application commit across integrated data services, as defined below.
- Application file storage and retrieval.
- A message queue for asynchronous application work.
- Key-value storage in explicitly selected replicated or local temporary modes, with configurable TTL. TTL may be configured per key or per collection/bucket; neither granularity is prescribed.
- Execution of asynchronous background work and scheduled tasks.
- Publish/subscribe events for multiple independent subscribers.
- Integrated application configuration and secrets management.
- Automatic HTTPS certificate provisioning and renewal as a core capability.
- Integrated append-only logging with distinct audit and diagnostic use cases, as defined below.
- Integrated collection of application-reported metrics, enabled by default. This may be supplied by a platform plugin rather than the core runtime.
- Tracing to follow application operations across requests and asynchronous processing.
- An administrator dashboard with operational views and configurable alerts, delivered as a higher-level capability over native platform state and telemetry.
- Backup to an external S3-compatible destination and restoration from those backups.

### Foundational and derived capabilities

The platform must distinguish foundational capabilities from higher-level features that use them, without requiring a particular module layout or implementation language.

Traffic and resource limits may be delivered as plugins using lower-level facilities such as key-value storage. Operational status and health views may be derived from metrics and other platform state. These features do not have to reside in the core runtime. Their failure behavior must account for the guarantees of their underlying capabilities; local temporary storage does not provide application-wide coordination or a strict global limit.

Cron scheduling may also be delivered as a higher-level capability using the native queue service. It does not require a separate core execution engine. A system scheduling request such as "enqueue this work every minute" is an acceptable intended interface. Each cron definition must have its own logically separate system-managed queue. The specification does not prescribe the internal scheduling mechanism or physical queue storage layout.

Certificate management is explicitly a core capability. Application-level user identity, sessions, and authorization are not confirmed platform requirements. Their scope remains open because a centrally prescribed application identity model may be inappropriate for some applications. Access control for platform administration must be discussed separately.

### Configuration and secrets

The platform must provide application configuration and secrets management so application code can obtain required settings and credentials without embedding environment-specific values in the code.

The same application-facing capability must be available locally, in a single-machine development installation, and in HA deployments. Live configuration or secret changes without a release are not required. Changing configuration or secrets may initiate a release, which must satisfy the same compatible-update availability contract as an application-code release.

The activation boundary for new values, protection of secrets, access controls, credential rotation, and inclusion of secrets in backups remain to be defined.

### Automatic HTTPS certificates

For configured public domains, the platform must obtain and renew valid HTTPS certificates automatically when domain validation and certificate issuance prerequisites are satisfied. Developers must not need to operate a separate certificate-management service or manually replace certificates during normal operation.

Certificate renewal and activation must not interrupt HTTP request service. Required local-development behavior, certificate authority dependencies, issuance-failure reporting, and certificate distribution in degraded states remain to be defined.

### Public client access

The application must be accessible through a stable public application address using ordinary browsers and standard clients for its implemented protocols. Installing special platform-specific client software or requiring a platform-specific HA client library must not be a prerequisite for ordinary access. A protocol's normal client, such as an MQTT or email client, is acceptable.

Public DNS may be used. Multiple DNS A records are a suggested option, not a mandated mechanism or a substitute for meeting the request-continuity contract. The behavior when a client has selected an address belonging to a failed VPS remains an explicit acceptance case to resolve. No DNS arrangement is assumed to satisfy that contract merely by listing several addresses.

### Application-defined protocols

HTTP and WebSocket support must not limit the kinds of applications that can run on the platform. Application code must be able to expose its own network services and process protocol exchanges while using the integrated SQL, files, queues, key-value storage, logs, and background execution capabilities.

Application-defined services must support TCP and UDP with configurable public listening ports. Connection security and routing of non-HTTP traffic remain to be defined. Availability criteria for protocol sessions must describe connection interruption, resumption, and message outcomes rather than assuming every interaction is an HTTP request. The five-second HTTP continuity requirement must not be silently generalized to all custom protocols before those semantics are agreed.

Preserving already-open WebSocket and other long-lived connections during a release is desirable, but is not a hard requirement. A release may disconnect such a connection. Client reconnection behavior, session recovery, and loss or duplication of application messages remain to be defined, as does connection behavior during unexpected VPS failures.

### Files, messaging, and asynchronous work

File storage, message queuing, key-value operations, and task execution must be accessible to application business logic as integrated platform capabilities in both single-machine and multiple-VPS deployments.

Background jobs and scheduled tasks must survive compatible releases and covered VPS failures without losing accepted work. Intentional deletion after exhausting the explicitly configured failure policy is permitted and is not an accidental-loss violation. Job acceptance, scheduling, cancellation, and inspection details remain to be defined, as do message ordering and delivery semantics.

### Required queue failure policy

Every queue definition MUST explicitly specify its processing-failure policy. A queue definition missing any required policy field must be rejected rather than silently use an implicit policy.

The required policy fields are:

- The maximum number of processing attempts. The interface must clearly distinguish total attempts from retries; the count must not be ambiguous.
- The delay between failed processing and a subsequent attempt. Whether the platform must support variable delays or backoff in addition to a fixed delay remains open.
- The final disposition after the attempt budget is exhausted: retain the failed message or delete it.

After a failed attempt, the message must remain recoverable and be retried according to the configured delay and remaining attempt budget. Failed transactional processing must not leave committed application effects or a successful completion acknowledgement.

If the final disposition is **retain**, the message must remain stored in a failed state and must not continue automatic processing after the configured attempts are exhausted. It must be possible to inspect the retained message and its failure information. Its retention period, manual replay, and manual deletion interfaces remain to be defined.

If the final disposition is **delete**, the message may be removed after the configured attempts are exhausted. No retained copy is required by this queue policy. Independent runtime logs, audit records, and backups follow their own policies.

Queue recovery after a release or VPS failure must preserve the policy and message state. The treatment of an interrupted attempt, timeout, or uncertain commit outcome in the attempt count remains to be specified. Unknown commit outcomes must not be blindly treated as a definitive processing failure that permits duplicate effects.

### Scheduled tasks and overlap policy

Scheduled work may use system-managed cron queues: the scheduling capability produces due work items in the queue belonging to that cron definition, and the queue executes them under its declared processing and failure policies. Scheduled items must use the native platform capabilities and remain subject to their transaction, durability, and degraded-state contracts.

Each cron definition must have an independent queue policy, including the required attempt limit, retry delay, and final retain-or-delete disposition. The scheduler must preserve that cron's execution scope and overlap policy. Backlog and failure state must be inspectable independently for each cron; one shared policy for all cron definitions is not the intended contract.

Every scheduled task must support a configurable execution scope. The default is once per application deployment, irrespective of replica count. An explicit per-server mode must also be available, with one independently scheduled execution on each participating server. Required per-server use cases are not prescribed.

Every scheduled-task definition MUST explicitly select one of three overlap policies. A definition without an overlap policy must be rejected:

| Policy | Required behavior when the previous execution is still running |
| --- | --- |
| Skip | Do not start or defer the new scheduled execution. That occurrence is skipped. |
| Defer | Do not overlap executions. Queue each due occurrence and process the pending occurrences sequentially after the current execution finishes. Do not silently merge or discard pending occurrences. |
| Allow overlap | Start the new execution when scheduled, regardless of whether a previous execution is still running. |

The overlap policy applies within the task's execution scope. In per-server mode, an execution on one server does not prevent the corresponding execution on another server merely because they belong to the same task definition.

In defer mode, if executions take longer than the scheduling interval, the backlog is expected to grow. For example, a task scheduled every minute that takes three minutes cannot keep pace when overlap is forbidden and every occurrence must run. The platform must expose this backlog and warn the administrator; it must not silently change the selected policy to hide the backlog. When no capacity remains for new queued work, the capacity-exhaustion contract applies. Exact limits and treatment of the unaccepted scheduled occurrence remain to be specified.

Missed schedules during downtime must have a configurable recovery policy supporting skipping missed occurrences, running one catch-up execution, or replaying all missed occurrences. Replay must still follow the selected overlap policy. The default recovery policy, maximum catch-up history, and interaction with already queued work remain to be specified.

Application-wide scope must not silently become per-server execution after a release or failure. Exact coordination guarantees during network partitions and handling of interrupted executions remain to be specified. Supported schedule expressions, time zones, and daylight-saving behavior also remain open.

### Asynchronous execution and publication

Long-running asynchronous work is required, but may be expressed through scheduled tasks and queued jobs. A separate workflow engine is not a confirmed requirement. Progress tracking, cancellation, checkpointing, and resumption granularity remain open.

Publish/subscribe must support delivery of one application event to multiple independent subscribers. Its retention, replay, subscription lifetime, delivery, and degraded-mode guarantees remain to be specified independently of ordinary worker queues.

Events published within an application transaction must remain pending until that transaction successfully commits. Subscribers must not receive them before commit. A rolled-back or definitively failed transaction must not publish its events. Event publication must participate in the unified transaction; actual delivery and subscriber processing need not finish before the publisher's commit completes.

The no-duplicate requirement for internal retries applies to application state changes. Exactly-once effects in external systems, such as sending an email or charging a payment, have not been defined. The final contract must distinguish retrying task execution from duplicating its observable effects.

SQL changes, accepting new queued work, and acknowledging completion of consumed messages must participate in the mandatory unified transaction capability. Receiving a message is distinct from irrevocably acknowledging its completion. If a processing transaction is rolled back, its message must remain recoverable for processing.

### Unified application transactions — mandatory core requirement

The platform MUST provide one application transaction spanning its integrated data services. Application code must be able to make SQL changes, write files, change key-value entries, append audit records, enqueue messages, publish pub/sub events, and acknowledge processed messages, then explicitly request a single commit. It must not require the developer to coordinate separate per-service commits or implement compensating application operations to obtain all-or-nothing behavior.

All participating changes MUST commit as one logical operation or none of them may take effect. The platform must not expose a partially committed result across the participating services. Uncommitted changes must not be treated as published application state or completed work by other clients and workers. Read-your-own-pending-changes and the exact concurrent-read isolation contract remain to be specified.

A successful commit response MUST mean that every participating change has committed under its declared storage guarantee. If the transaction is rolled back or definitively fails before committing, none of its changes may become committed application state. A failure during commit must not leave a permanent mixture of committed and uncommitted effects across participating services.

The same transaction-facing application code MUST work in single-machine development and HA operation. The explicit local durability exception in development mode remains in force. Atomicity does not upgrade a `persistent server` or `temporary server` file into replicated data; visibility and failure semantics for transactions mixing different file scopes remain to be finalized.

In the one-member degraded state, a transaction containing any prohibited SQL write, replicated-file write, replicated key-value write, or audit append MUST NOT partially commit its permitted local temporary changes. Transactions containing only permitted operations must retain all-or-nothing commit behavior locally. Local temporary key-value entries are not merged or replicated after connectivity returns.

This requirement covers platform-managed data operations, including transactional pub/sub publication. External S3 operations are outside the atomic commit guarantee; runtime logs are independent of it, and transaction-associated metric updates follow the post-commit policy below. The boundary for configuration and secret changes remains open. Direct actions in external systems, such as sending an email, issuing an HTTP response, or charging a payment, do not acquire an agreed atomic rollback guarantee merely because application code performs them inside a transaction.

The platform is responsible for recovering interrupted commits and preserving atomicity and the existing no-duplicate guarantee for internal retries. Ordinary application code must not need to coordinate recovery across services or add a separate commit-status lookup to its normal workflow.

A dedicated application-facing API to query transaction status is not mandated. Applications may inspect their business state when appropriate. A missing success response must not automatically be interpreted as proof of rollback, and inspecting business state must not be treated as a general replacement for the platform's atomic recovery guarantees. The returned outcome when the caller itself disconnects, and the treatment of independently resubmitted client requests, remain open.

### File storage policies

Application file storage must expose three distinct directory scopes. The destination directory selects the storage guarantee; a separate durability flag on every write is not required. The names below identify the requested scopes, not fixed absolute filesystem paths.

| Directory scope | Observable write and storage guarantee |
| --- | --- |
| `replica` | In HA operation, a successful write completion is returned to application code only after the file is durably stored on multiple machines. The file survives the loss of any one VPS in a multiple-VPS deployment, just like an acknowledged SQL write. In configured single-machine development mode, completion follows local durable storage without waiting for another machine. |
| `persistent server` | A successful write completion confirms durable storage on the current server only. The file survives application and server restarts when that server's storage remains intact. It is not replicated, may be unavailable while the server is offline, and may be permanently lost if that storage is destroyed. |
| `temporary server` | The file is temporary and local to the current server. No replicated or restart-durability guarantee is provided. Its lifetime and cleanup rules remain to be defined. |

In HA operation, application execution that depends on committed `replica` data must wait for confirmed replication. Reporting committed storage while replication is still pending is not acceptable. Within a transaction, preparing a file write is distinct from successfully committing it: the write remains a pending change until the unified commit completes. How much work is performed before commit is not prescribed. In configured single-machine development mode, committed storage requires local durability only; no peer is required. The application uses the same directory and operations in both modes, without code changes.

The directory distinction must be visible and unambiguous to the application developer. A local persistent write must not be described as an HA-replicated write. Policies for moving files between scopes, concurrent access, and partial writes remain to be defined.

For example, an application may receive a CSV import as a local file, process it, commit replicated SQL changes, and then return an HTTP success response. The temporary source file need not be replicated; the committed database changes must meet the SQL durability contract. Whether an interrupted import must be resumable or replayable remains to be defined.

An application may choose an external S3-compatible file service instead. The meaning of that service's acknowledgement and its durability obligations must be specified rather than assuming that an S3 interface alone establishes them.

External S3 operations do not participate in the platform's all-or-nothing transaction guarantee. A system helper for best-effort compensation is a desired capability: if the platform transaction definitively fails, it can attempt the compensating action associated with an external operation. This is compensation rather than an atomic rollback. Compensation itself may fail, leaving externally visible residual effects. Guaranteed recovery from failed compensation is not required; reporting, inspection, retry, and manual-repair interfaces remain to be specified. A missing commit response alone must not be treated as definitive failure and trigger compensation of a potentially committed operation.

Other required file operations and upload interruption behavior remain to be specified. All-or-nothing transactions spanning SQL, files, key-value entries, audit appends, and queued work are mandatory as defined above.

### Key-value storage policies and expiration

Applications must explicitly select one of two key-value storage scopes:

| Scope | Required behavior |
| --- | --- |
| Replicated | Entries follow the same replicated persistence, unified transaction, and acknowledgement guarantees as SQL data. A committed write survives one VPS failure. Writes cannot succeed in the one-member HA state. |
| Local temporary | Entries exist only on the current server and may be lost. They are not shared, replicated, or merged with another server's entries. Writes remain available on an isolated server under their local contract, like temporary server files. |

Both scopes must support configurable TTL for cache use. TTL governs expiration and does not determine replication policy. Expiration is intentional removal, rather than a durability failure. Behavior without TTL, expiration timing, TTL replacement, and early eviction remain to be defined. Replicated key-value storage uses the single-machine development durability exception just like SQL and replicated files.

There is no disconnected multi-writer cache with last-write-wins reconciliation in the current requirements. A local temporary value must not be presented as shared application state. The application-facing way to select a scope remains to be defined without prescribing a language or storage engine.

### Application data correctness

Once a client receives a successful acknowledgement of an application data write covered by the HA persistence contract, that write must survive the failure of any one VPS in a multiple-VPS deployment. It must not silently disappear as a consequence of that failure. SQL writes, `replica` file writes, replicated key-value writes, and audit appends carry this contract. Explicitly server-local files and disposable data carry their separately declared weaker guarantees.

For any operation carrying the platform's replicated persistence guarantee, success must not be acknowledged until the data is stored on more than one machine. This applies to SQL, replicated files, replicated key-value entries, audit records, and other capabilities when they promise that guarantee. Single-machine development has the explicit local exception defined in this document. Local temporary entries do not promise replicated persistence. Mere acceptance into a local processing queue is not equivalent to confirmed replicated persistence.

Configured single-machine development mode is an explicit exception to multi-machine acknowledgement: persistence operations must be usable without another machine, and `replica` file writes are confirmed after local durable storage. This mode does not promise survival of the loss of that machine, and must not require changes to application code.

When only one member of a three-VPS HA deployment is reachable, SQL writes, replicated-file writes, replicated key-value writes, and audit appends must not be committed or acknowledged as successful. Reading remains a supported operation, and local temporary key-value writes are explicitly permitted. There is no automatic downgrade of replicated storage to local storage.

A complete replica may lack recent changes committed by the other two members. Application code decides which operations may use degraded reads, including authentication and authorization. A supported local read must not silently claim to reflect the latest global state.

### Application-required replica status

The platform must expose an application-facing guard, illustratively named `requireReplicaStatusOk`, that application code can call before continuing an operation requiring an acceptable replication state. This name does not prescribe an API language or error mechanism.

If the required condition is not satisfied, the guard must return failure or raise an error so application code can stop the protected operation. The application can place this guard only where its business rules require it; ordinary reads need not universally require it.

The precise meaning of acceptable replica status remains to be agreed: sufficient peers for replicated writes, synchronization of the serving replica, and freshness of the protected read are distinct properties. A status check must not falsely imply a read-freshness guarantee it does not provide. The configured single-machine development behavior must retain the same application-facing operation.

Passing the guard must not bypass the platform's write or commit requirements. Those requirements must still be enforced when committing, even if replication state changes after the guard passed. The scope of any stronger guarantee for protected reads remains to be defined.

Internal retries used to preserve request continuity must not cause the same logical operation to be applied twice. For example, retrying request processing after a failure must not create a second order for the original request. Guarantees for independently repeated client requests and external side effects remain to be defined.

Application data durability is distinct from the weaker loss allowances for diagnostic logs and local temporary key-value entries. Recovery of acknowledged data after destruction of multiple VPS instances remains to be specified; a complete copy does not imply that every acknowledged change had already reached every replica.

### Integrated logs

Log records are append-only: adding records is supported; editing or overwriting existing records is not supported. Retention and deletion policies remain to be defined.

The platform must support two logging use cases with different loss expectations:

- **Audit records:** The secure audit log is a replicated persistent service with the same acknowledgement and single-VPS-failure durability requirements as SQL data. Success must be acknowledged only after durable storage on multiple machines. External backup is required according to the configured backup policy. A single-machine development installation must expose the same audit capability under its explicit local durability exception. Loss should be exceptionally unlikely; tolerated failures beyond the replicated acknowledgement contract remain to be specified.
- **Diagnostic/runtime logs:** These logs must operate without replication or backup. Gaps caused by failures are acceptable. Uninterrupted collection during a failure is not required.

Runtime logs are independent of application transactions. They may describe pending, failed, rolled-back, or committed operations. Runtime logging does not participate in commit or rollback and has no influence on the transaction's result. Rolling back an application transaction must not remove its diagnostic records.

Audit appends in HA operation must not report successful completion while the required replication is unavailable. There is no local-only acknowledgement exception in the one-member production state. Audit writes inside an application transaction follow the unified commit contract, including rollback of uncommitted appends. Locally prepared but uncommitted audit records must not be presented as successfully committed audit history.

Duplicate record handling, ordering, acknowledgement thresholds, and guarantees after multiple storage failures remain to be agreed. No single global event order is implied merely by append-only storage.

An example application scenario is login without SQL writes: read the user record, validate credentials, issue a JWT, append an audit record, and store a TTL-bound token entry in cache. If successful login requires a committed audit record, it cannot complete successfully in the one-member HA state. Application code may additionally use the replica-status guard to restrict credential checks under degraded reads. Issuing JWTs here is an example of application logic, not a requirement for built-in identity management.

A developer must be able to retrieve and filter historical logs using capabilities included in the platform. Retrieving logs for a period such as the preceding 30 days is a required use case. Whether 30 days is the mandatory minimum retention period, and which filters and access interface are required, remain to be confirmed.

Historical diagnostic logs on an unavailable VPS may be unavailable until it returns. If that VPS and its storage are permanently destroyed, its diagnostic log history may be permanently lost. Queries may return available records with gaps; the interface for identifying incomplete results remains to be defined. This allowance does not relax application data or audit record requirements.

### Metrics

Application code must be able to report metrics to the platform. The platform must collect those metrics by default without requiring separately operated Prometheus, Grafana, or equivalent services.

Metrics are a required platform capability, but may be delivered through an included plugin rather than implemented in the core runtime. This packaging choice must not require the application developer to assemble an external monitoring stack.

Metric types, labels, query and visualization capabilities, retention, aggregation across machines, and loss, replication, and backup policies remain to be defined.

Application metric changes associated with a transaction must be deferred until that transaction has successfully committed. A transaction that rolls back or definitively fails must not apply its associated metric changes. For example, a counter of created orders must not increase for an order that did not commit.

Applying those metric changes is a best-effort post-commit step, rather than part of the atomic data transaction. A metric update may be missed if that step fails or the process stops after commit. Such a failure must not roll back committed data or turn the successful data commit into a failed transaction. Exactly-once metric delivery is not required; retry and duplicate-update behavior remain to be specified.

Operational measurements outside an application transaction, such as request duration, memory usage, or failed-transaction counts, may be reported independently. They do not imply that the measured application operation committed successfully.

### Tracing, limits, and operational status

The platform must support tracing so a developer or operator can follow a logical operation through request handling, SQL activity, queued work, and asynchronous execution. Application instrumentation responsibilities, trace sampling, retrieval, and retention remain to be defined.

Traffic and resource limiting must be available as a higher-level capability, potentially through a plugin using cache. Limit scope, enforcement behavior, and allowed overshoot during disconnection remain to be specified.

### Administrator dashboard and alerts

An administrator must have a dashboard for inspecting the state of the platform and its integrated capabilities, including servers, replication and degraded states, request serving, storage, queues, scheduled tasks, logs, metrics, traces, releases, and backup status. The dashboard must support the previously required historical log retrieval and filtering.

The dashboard must provide full administrative operation of all platform capabilities, rather than inspection alone. This includes managing configuration and secrets, initiating releases, backups and restores, controlling queues and scheduled tasks, inspecting and replaying retained failed work, and managing alerts. Administrator actions remain subject to the declared transaction, durability, availability, and capacity contracts. Exact interfaces and administrator authentication remain to be defined.

The dashboard and alerting form a higher-level capability built on existing native state, metrics, logs, and tracing. They need not be part of the core runtime and must not require separately operated monitoring services. Their availability guarantees remain to be specified separately from application request serving.

Administrators must be able to define custom alert rules. Default rules must include warnings for a backlog of scheduled executions and scheduled tasks running too long. Backlog depth, growth, waiting time, and execution duration must be observable. Exact default thresholds, evaluation intervals, notification channels, acknowledgement, and recovery behavior remain to be defined.

The interface must distinguish observable healthy state from degraded, unavailable, or incomplete information. It must not imply that unavailable server-local logs or delayed telemetry have been observed successfully. Administrator authentication, any additional roles or permissions, and detailed control interfaces remain to be defined.

### Capacity exhaustion

When storage is full or another applicable capacity limit prevents accepting additional writes, the platform MUST reject new writes or new queued work that cannot be safely accepted. It must expose a clear capacity-related reason and an administrator alert. It must not falsely acknowledge successful storage or silently discard previously accepted work to admit new work.

Already committed data and accepted work must be preserved under their declared policies. Explicit expiry, retention, and configured deletion of exhausted failed messages remain permitted; capacity pressure must not silently invent an additional deletion policy.

A transaction rejected for insufficient capacity must preserve the unified all-or-nothing guarantee. Capacity exhaustion is an explicit limit of successful-request availability, not permission to partially commit. Reads and work that can safely proceed within remaining resources should remain available. Exact resource limits, client response formats, and recovery behavior after capacity is freed remain to be defined.

### External backup and restore

The platform must provide an integrated capability to back up data to an external destination through an S3-compatible interface. The destination must be independent of the VPS instances running the application, so that destruction of those instances does not also destroy the backup.

Backup contents must be configurable, subject to the log policies: audit records require backup, while runtime logs are excluded. The operator must be able to select which other supported categories of information to include. The available categories and selection granularity remain to be defined; this draft does not require every backup to contain all application data, files, queued work, code, and configuration together. Audit backup behavior before an external destination is configured remains to be defined.

Automatic backup frequency must be configurable. No universal backup interval or fixed maximum loss window is prescribed. A default schedule, backup retention, and whether on-demand backups are also required remain to be defined.

If all application VPS instances and their local storage are destroyed, loss of data newer than the latest usable external backup is acceptable. Recovery in that situation is a backup-and-restore guarantee, rather than a continued-availability or zero-data-loss guarantee. The actual recovery point depends on successful completed backups, not merely the configured schedule.

Restoring from an external backup must be reliably achievable through a documented, repeatable procedure. No fixed restoration-time target is required at this stage. The restoration target and whether code and configuration must be installed separately remain open; restoration must recover the selected backup contents to a coherent, usable state.

Backup and restore must be specified as observable operations with clear success or failure outcomes. A completed backup must be usable to recover the agreed contents to a coherent state. The treatment of incomplete or failed backups remains to be defined.

### Availability during interruption

In a multiple-VPS deployment, the system as a whole must continue providing its required service when any one VPS is unexpectedly stopped or loses network connectivity to the other VPS instances.

The network-disconnection case must be covered even if the disconnected VPS remains running. The behavior of requests reaching that VPS and the required data guarantees during disconnection remain to be specified.

The application must also remain operational when only one VPS survives, including the case where two out of three VPS instances stop. The survivor supports reads and local temporary key-value writes, while SQL writes, replicated-file writes, replicated key-value writes, and audit appends are unavailable. Application-defined replica-status guards may further restrict sensitive operations. Full write availability is not required. Response delays greater than five seconds are acceptable in this degraded state.

The same restrictions apply to an isolated one-member side of a network partition. Local temporary key-value writes may be accepted there even if the other two VPS instances remain running; they remain local after reconnection. SQL, audit, replicated key-value, and replicated-file write behavior on the two-member side, degraded background-job and queue behavior, and availability of server-local file writes still require a complete contract. Behavior during a release combined with a VPS failure also remains to be defined.

### Availability during updates

Correctly prepared and compatible application updates must complete without a service outage. The required release availability target is 100% uptime throughout the update.

This guarantee applies when application changes follow an agreed compatibility procedure. Removing an endpoint that an active client still requires is not, by itself, a platform availability failure. The compatibility procedure and responsibilities must be specified before acceptance criteria can be finalized.

The guarantee must not depend on whether a release replaces a complete build or updates a module. It must not depend on whether application business logic is delivered as one module or several modules.

### Client-visible continuity

An update or a covered failure must not cause an otherwise valid request to fail with HTTP 500 or time out. Temporary unavailability must not be disguised as another error response: the required outcome is successful completion of the request.

Internal pauses, buffering, and retries are acceptable, provided the client-visible contract is met. For otherwise fast requests within the agreed workload, the maximum total response time during a compatible update or a single-VPS failure is five seconds. Lower transition latency is not a priority. The one-survivor degraded state described above may exceed this deadline.

This guarantee covers both requests already in progress and requests arriving during the transition, without requiring the client to retry. Long-lived connections, streaming, intrinsically slow operations, and unreachable network paths still require separate definitions.

This availability guarantee concerns errors caused by updates or covered failures. It does not require invalid requests or application-level failures to return successful responses.

The one-member degraded state explicitly excludes successful completion of operations requiring SQL writes, replicated-file writes, replicated key-value writes, or committed audit records, and operations rejected by an application replica-status guard. Such operations must not be falsely acknowledged; their client-visible unavailable or deferred outcome remains to be defined. The general no-failed-request guarantee must be interpreted within the supported operations of each availability state.

Persistent data changes required by a release must be included in the final update contract.

## Questions to resolve

### Next discussion: observable semantics of the building blocks

1. What condition must the replica-status guard require: enough connected replicas, confirmed synchronization and read freshness, or a selectable requirement? How does it behave in single-machine development mode?
2. What token revocation and cache-miss behavior is required? What acknowledgement threshold and multiple-failure loss policy apply to audit records?
3. What TTL defaults, expiration timing, early eviction, and local temporary lifetime rules are required?

### Subsequent discussion

- What application workload must fit within the resource baseline: concurrent clients, request rate, processing time, stored data size, and log growth?
- How must queue attempts be counted after interruption or timeout? Are fixed retry delays sufficient, and what retention and manual replay controls are required for failed messages?
- What is the default missed-schedule recovery policy and catch-up limit? What failure and partition behavior preserves application-wide scope? What happens to scheduled occurrences that cannot be accepted because capacity is exhausted?
- What system scheduling and per-cron queue-management interfaces are required?
- What schedule expressions, time zones, and daylight-saving behavior are required?
- What metric queries and visualizations must the dashboard provide, and what history and loss policy applies?
- What failure guarantees apply to transactions mixing local and replicated file storage? Which compensation reporting and inspection capabilities are required for external S3 operations?
- Must configuration and secrets participate in application transactions? Can audit records about an aborted operation be deliberately appended outside that operation's transaction?
- What retry and duplicate-update policy applies to best-effort post-commit metrics?
- What concurrent-read isolation, read-your-own-writes, TTL start time, caller-disconnection outcomes, and transaction-size or duration limits are required?
- What transaction and lifetime guarantees apply to local temporary entries, particularly when mixed with replicated data?
- What configuration activation, secret access, rotation, backup, and degraded-mode guarantees are required?
- What platform-administration access controls are required, and should any application identity capability be included?
- What tracing interfaces, limit scopes, dashboard views and actions, alert thresholds, notification channels, and monitoring availability guarantees are required? Which derived capabilities are supplied by default?
- Which backup categories, selection granularity, default schedule, retention policy, and restoration target are required? Must an operator also be able to trigger backups on demand?
- Which infrastructure dependencies are allowed, and which dependency failures are covered?
- What guarantees apply to application writes after multiple VPS failures, audit acknowledgements, and writes made in the one-survivor state?
- Which write and conflict policy should apply during network partitions, and what must clients observe after reconnection?
- What consistency, transaction, uniqueness, and concurrent-update behavior must clients observe?
- What measurable audit loss tolerance is acceptable, and under which failures? What must the log record, which filters are required, who may read it, and may records ever expire or be deleted?
- What HTTP streaming and WebSocket recovery, message-loss, and duplication guarantees are required?
- What key-value durability modes, no-TTL behavior, eviction rules, and expiration timing are required?
- When are temporary server files removed: task completion, application restart, server restart, age limits, or another configured policy? What guarantees apply to moving files between scopes and partial or concurrent writes?
- How must background work behave during failure and updates, including work already in progress and external side effects?
- What must an operator observe when an update succeeds, fails, or is cancelled? What behavior is required during recovery or rollback?
- What compatibility obligations apply to old and new clients and persistent data during a release?
- What exact capacity limits, rejection responses, alert behavior, and recovery controls are required? What operations remain supported when too few functioning instances remain?

## Acceptance criteria

The following scenarios capture the confirmed direction. Workload limits, data guarantees, routing assumptions, and degraded-state limits must be resolved before these are executable acceptance tests.

| Scenario | Action | Required observable result |
| --- | --- | --- |
| Complete single-VPS installation | Run the application on one VPS within the resource baseline. | HTTP/WebSocket serving, custom network services, business logic, SQL, files, messaging, key-value storage, tasks, logs, and metrics are available without separately operated services for those capabilities. Single-machine durability semantics remain to be specified. |
| Same application code | Run the same application code locally, on one development machine, and across multiple VPS instances. | The application uses the same operations without an alternate implementation. Configured development mode acknowledges persistence locally; HA mode requires multi-machine durability. |
| Unsupported application deployment | Attempt to deploy an existing non-compliant application through a Dockerfile, container image, Docker Compose definition, or Kubernetes manifest. | These are unsupported application deployment inputs. The application must be rewritten to comply with the platform's contracts; packaging alone does not confer compatibility or HA guarantees. |
| Application-defined network service | Supply application code implementing a non-HTTP service using TCP or UDP on a configured port. | Standard clients for that protocol can use the service, and the application can use the integrated platform capabilities. Security and session-continuity requirements remain to be specified. |
| Compatible update | Apply a correctly prepared release on one machine or multiple VPS instances while clients send valid requests. | Otherwise fast requests, including in-flight requests, complete successfully within five seconds, without client retries or update-induced errors or timeouts. |
| One VPS stops | Unexpectedly stop any one VPS in a multiple-VPS deployment while clients send valid requests. | Otherwise fast requests complete successfully within five seconds without client retries or failure-induced errors or timeouts, except where the one-survivor degraded-state allowance applies. |
| One VPS loses peer connectivity | Disconnect any one VPS from the other instances while it remains running. | The system continues serving the application; client routing and data correctness requirements during disconnection still need to be specified. |
| Only one VPS survives | Stop two VPS instances in a three-VPS deployment. | Reads and local temporary key-value writes remain supported. SQL writes, replicated-file writes, replicated key-value writes, and audit appends are not committed or acknowledged as successful. Application guards may restrict sensitive reads. Response times may exceed five seconds. |
| Replica-status guard | Call the application guard while its required replica condition is not satisfied. | The guard returns failure or raises an error so the protected application operation does not proceed. Its precise predicate and development-mode behavior remain to be specified. |
| Audited degraded-mode login | Attempt login on an isolated VPS where the application requires a committed audit record before returning success. | Login is not falsely reported successful while its mandatory audit append cannot commit. Its client-visible unavailable outcome remains to be specified. |
| Acknowledged HA application write | Receive a successful response for an HA-persistent write, then lose any one VPS. | The acknowledged change remains part of application state. Explicitly server-local files are outside this guarantee. |
| Directory-selected file policy | Write files to each of the three directory scopes in HA mode. | `replica` reports completion only after durable multi-machine storage; `persistent server` confirms durable storage on the current server; `temporary server` provides local temporary storage. |
| Single-machine development write | Write to `replica` in configured single-machine development mode with no peers available. | The write is confirmed after local durable storage, without waiting for peers and without changes to application code. |
| File survival | Complete writes to `replica` and `persistent server`, restart the receiving server with intact storage, then destroy that server and its storage. | Both files survive the intact-storage restart. The replicated file also survives destruction of that one server; the server-local persistent file may be lost. Temporary file cleanup remains to be defined. |
| Key-value TTL | Store a cache entry with a configured TTL. | The platform supports expiration of that entry; exact timing and early-eviction rules remain to be specified. |
| Key-value scope | Write one replicated entry and one local temporary entry, then lose the receiving VPS. | The committed replicated entry survives that one VPS failure. The local temporary entry may be lost and is not reconstructed by merging copies. |
| Default metrics collection | Report application metrics in a standard installation. | The platform collects the metrics without a separately operated monitoring stack; query, visualization, retention, and failure behavior remain to be specified. |
| Configuration and secrets | Run the same application in development and HA environments with different settings and credentials. | Application code obtains configuration and secrets through the platform without embedding environment-specific values in code. Access and update rules remain to be specified. |
| Automatic HTTPS | Configure a publicly validatable domain, obtain a certificate, and reach its renewal period. | The platform provisions and renews HTTPS certificates without a separately operated certificate-management service; activation does not interrupt request service. Issuance prerequisites and failure outcomes remain to be specified. |
| Transactional pub/sub event | Prepare an event for multiple independent subscribers inside a transaction, then commit or roll back. | Subscribers cannot receive the event before successful commit. Rollback publishes no event. Exact delivery and recovery guarantees after publication remain to be specified. |
| Trace asynchronous processing | Handle a request that initiates queued background work. | The platform supports following the logical operation across those stages; instrumentation, query, and retention requirements remain to be specified. |
| Unified transaction commit | Prepare SQL changes, a file write, key-value changes, an audit append, queued work, and completion acknowledgement of a consumed message, then explicitly commit once. | All participating changes commit as one logical operation under their declared guarantees, or none do. No application-managed per-service commit sequence is required. |
| Unified transaction rollback | Prepare changes across multiple services, then roll back or cause definitive failure before commit. | None of the changes becomes committed application state; newly prepared messages are not delivered and the consumed message is not acknowledged as completed. |
| Independent runtime diagnostics | Write runtime diagnostics while preparing a transaction, then roll it back. | Diagnostic records are not rolled back, and logging does not affect the transaction outcome. Their normal retention and loss policy still applies. |
| Metric update after rollback | Prepare an order creation and an associated order-counter increment, then roll back. | Neither the order nor the associated counter increment is committed or applied. Runtime diagnostics about the attempt may remain. |
| Metric failure after successful commit | Commit an order, then interrupt or fail its post-commit metric update. | The order remains committed. The counter increment may be missing; no rollback or transaction failure is caused by metric delivery failure. |
| External S3 compensation | Perform an external S3 operation with an associated compensation action, then definitively fail the platform transaction. | External S3 is outside atomicity. If the desired helper is provided, it attempts compensation; failure of that action may leave residual external effects without guaranteed automatic recovery. |
| Interrupted unified commit | Stop a VPS during commit of a cross-service transaction. | Platform recovery does not leave a permanent partial commit or duplicate effects from internal retries. Ordinary application code does not need separate service-recovery steps or a mandatory commit-status API call. Caller-disconnection outcomes remain to be specified. |
| Degraded mixed transaction | On the one-member side, prepare a local temporary key-value change together with a prohibited SQL write, replicated-file write, replicated key-value write, or audit append, then request commit. | The transaction does not partially commit the local temporary change while rejecting the prohibited operation. The exact unavailable outcome remains to be specified. |
| Internal retry | Interrupt request processing so that the platform retries the same logical operation. | The operation completes without duplicate application effects, such as a second order. |
| Background work survives transition | Accept background work, then apply a compatible update or lose one VPS. | Accepted work is not lost and remains processable; retry, completion, and external-effect semantics remain to be specified. |
| Scheduled-task scope | Define a task with default scope, then define one with explicit per-server scope in a multiple-VPS deployment. | Default execution is once per application deployment; per-server execution schedules work independently on each participating server. Partition and recovery guarantees remain to be specified. |
| Queue-backed scheduled work | Register two system scheduling requests with different schedules and failure policies. | Each cron definition has its own logical queue, independent failure policy, and inspectable backlog. The scheduler preserves each task's declared scope and overlap policy. Scheduling timing and recovery details remain to be specified. |
| Required overlap policy | Define a scheduled task without selecting skip, defer, or allow overlap. | The definition is rejected. |
| Skip overlapping occurrence | Keep a scheduled execution running beyond the next due time with skip selected. | The next occurrence does not start and is not deferred. |
| Defer overlapping occurrence | Keep a scheduled execution running beyond several due times with defer selected. | Each due occurrence is queued and runs sequentially; pending occurrences are not silently merged or discarded. |
| Allow overlapping occurrence | Keep a scheduled execution running beyond the next due time with allow overlap selected. | The new occurrence starts according to its schedule regardless of the previous execution. |
| Missed scheduled occurrences | Recover after downtime with several missed due times and an explicit recovery policy. | The platform skips missed occurrences, runs one catch-up execution, or replays all missed occurrences according to the selected policy, while respecting overlap rules. Defaults and catch-up limits remain to be specified. |
| Growing scheduled backlog | Schedule a non-overlapping deferred task every minute while each execution takes three minutes. | Pending occurrences accumulate, backlog and durations are visible, and the default alert rules warn the administrator without silently changing the policy. Thresholds and capacity behavior remain to be specified. |
| Administrator overview | Open the dashboard during normal and degraded operation. | The administrator can inspect platform state, logs, metrics, and related operational information, with unavailable or incomplete information clearly identified. Exact views and dashboard availability remain to be specified. |
| Administrator controls | Use the dashboard to manage configuration, initiate a release or backup/restore operation, control scheduled work, or replay retained failed work. | The dashboard provides the required administrative actions while preserving each capability's declared guarantees. Authentication and detailed interfaces remain to be specified. |
| Capacity exhaustion | Fill storage or exhaust a configured capacity limit, then attempt new writes or enqueue work. | The platform rejects work it cannot safely accept with a clear reason and an alert, preserves already committed data and accepted work under their policies, and does not partially commit a rejected transaction. |
| Custom alert | Define an administrator alert rule and make its condition occur. | The higher-level alerting capability evaluates the rule and exposes the alert. Evaluation timing and notification delivery remain to be specified. |
| Incomplete queue definition | Define a queue without an attempt limit, retry delay, or final failure disposition. | The definition is rejected; the platform does not silently choose missing failure behavior. |
| Queue retry policy | Repeatedly fail processing of a message in a queue with an explicit attempt budget and delay. | Processing follows the configured attempt and delay rules. Definitively failed transactional attempts leave no committed application effects. Interrupted-attempt counting remains to be specified. |
| Retain exhausted message | Exhaust the attempt budget in a queue configured to retain failed messages. | The message remains stored and inspectable in a failed state, with no further automatic processing. Retention and manual actions remain to be specified. |
| Delete exhausted message | Exhaust the attempt budget in a queue configured to delete failed messages. | The message is removed according to the explicit policy; no retained copy is required by that policy. |
| Audit and runtime log policies | Generate both categories of logs in a multiple-VPS deployment with external backup configured. | Audit records are replicated and backed up; runtime logs are neither replicated nor backed up. Loss guarantees and backup timing remain to be specified. |
| Historical log query | Retrieve and filter logs for the preceding 30 days while one VPS is unavailable. | The platform provides filtering and retrieval of available diagnostic records. Records from the unavailable VPS may be absent; its history may be lost permanently if its storage is destroyed. Retention limits remain to be specified. |
| Configurable backup | Select backup contents and an automatic backup frequency. | The platform creates backups according to those settings; the supported selection granularity and retention policy remain to be specified. |
| External backup and recovery | Complete a backup to an independent S3-compatible destination, then destroy all application VPS instances and restore from the backup. | The selected contents are reliably recovered to a coherent, usable state through a repeatable procedure. Data newer than that backup may be lost. No fixed restoration deadline is required; the restoration target remains to be specified. |

The draft does not yet define a complete availability contract or sufficient acceptance criteria.
