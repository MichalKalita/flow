# External integrations and the universal event mechanism

## Requested outcome

Provide generic typed integration capabilities that the core can validate and plan around. Define commit, failure, retry, and recovery semantics explicitly for external services. Support endpoints using only external data, with mandatory validation of both input and returned data. Cover services such as S3, REST APIs, MQTT, and email.

## Current implementation

The compiler declares typed plugin inputs/outputs and method modes. Runtime execution accepts only the built-in `Payment.createUrl`, `Image.resize`, and `Files.put` implementations. The first two are local computations; `Files.put` stores data in the same SQLite transaction. These examples are not an existing remote payment or generic external-plugin framework. See [plugin declarations](../runtime/src/program.rs), [execution](../runtime/src/engine.rs), and [runtime integration tests](../runtime/tests/runtime.rs).

## Confirmed email use case and automation foundation

The user wants a complete built-in email server modeled as a hosted project. Projects must control the stored email data and reactions to incoming messages. This requires a reusable event mechanism rather than email-specific business logic in the runtime. See [the email design](emails.md).

Conditional automations already exist for newly created stream records: `[on ...]` selects the source and verified actor, and `[when ...]` selects when to execute. They run with central permissions inside the originating SQLite transaction; failures roll back the transaction. See [the compiler](../runtime/src/program.rs), [the executor](../runtime/src/engine.rs), and [stream integration tests](../runtime/tests/streams.rs).

Evolve this foundation into a unified durable event mechanism for every source. Existing synchronous stream automations must be migrated explicitly to this contract, with documentation and integration coverage for the changed handler transaction boundary. Source mutations and individual handler mutations must each retain atomic data and audit transactions. External effects need explicit acceptance, retry, deduplication, and failure semantics; they cannot inherit SQLite rollback guarantees. Entity-change and scheduled triggers remain design work, not implemented capabilities.

## Confirmed unified event mechanism

Every event in the system uses the same event-input API, durable lifecycle, dispatcher, permissions, failure policy, and retention/deletion mechanism. Only the source and typed payload differ. A handler operates on an event that has already occurred and been accepted; persist the event before dispatching its handlers. Email is an ordinary event source under this universal contract, not a separate class of event. Its plugin supplies incoming-message events rather than REST requests and applies mail-specific receipt constraints before acceptance.

Separate acceptance from the handler transaction. Handler failure cannot undo accepted input. Persist enough pending processing state to recover after restart, and keep each handler's local application writes and audit atomic. Retry limits and terminal retention or deletion use the common configurable failure policy below; exact defaults and recovery controls remain undecided.

Existing stream automations execute inside the originating SQLite transaction before commit, so they do not yet satisfy the target contract. Migrate them to the same lifecycle as plugin-sourced events; do not retain a parallel legacy event-processing engine as the final design. For events produced by a local mutation, commit the source changes, their audit, and pending event state atomically, then dispatch after commit. Handler failure rolls back the handler attempt, not the source transaction.

Bounded event storage, project quotas, and host-wide ceilings apply to the common runtime mechanism. An HTTP request remains a transport request, but any events it produces use the universal event lifecycle.

### Source plugin responsibilities

“Source plugin” is working terminology for an adapter that provides events. An email plugin implements email protocols, parsing, verification, domain routing, and mail-specific limits. Other source plugins implement their own protocol responsibilities and supply events through the same runtime interface. The project chooses its handlers and application storage mapping.

Keep event persistence and scheduling under the runtime's common contract. A source may acknowledge successful receipt only after the runtime has durably accepted the input. Source credentials and verification results do not bypass central project permissions or automatically turn untrusted sender fields into Flow identities. Plugin packaging and exact API syntax remain implementation decisions.

## Confirmed common failure policy

All events and system mechanisms must support configurable failure behavior. Email receipt and processing use this shared mechanism. Configure how many retries are allowed and what happens when processing still fails: retain the failed input/work item or delete it. Do not hard-code one outcome for all projects or implement separate email-only failure rules.

For a durably accepted event, acceptance is already committed. Each failed local processing attempt rolls back its application changes and audit entries. Retry exhaustion then applies the configured terminal disposition to the accepted input/work item; this is distinct from undoing acceptance or deleting previously committed application data. Retention and deletion must respect project isolation, permissions, and transactional audit for application mutations.

The shared configuration model must account for each mechanism's execution contract. A synchronous request still needs a response and cannot silently acquire deferred processing semantics; an external effect already performed cannot be undone by retry or deletion. Define replay safety and completion tracking before enabling retries for such mechanisms. The desired configurable failure handling applies throughout the system; these distinctions describe implementation constraints, not exclusions from that scope.

### Implementation recommendations awaiting decisions

Persist attempt count, next eligible attempt time, and terminal status with durable work so restarts do not reset the retry limit. Define whether the configured limit counts retries after the initial attempt or all attempts, and allow zero retries. Use bounded retry scheduling, bounded failed-input storage, and safe metadata-only diagnostics. Coordinate local application commit and job completion atomically wherever they share a transactional store.

Treat retention or deletion as an explicit terminal transition. A crash during cleanup must not resurrect deleted work or lose retained work. Deleting a failed input must not delete committed mutation audit history. Define idempotency and ambiguous external outcomes separately; retries alone do not guarantee exactly-once external effects.

Configuration placement, retry delays, defaults, manual replay, and the payload retention lifecycle remain to be specified. Integration tests must demonstrate identical event lifecycle and failure handling for existing stream sources and plugin sources, and cover zero retries, retry exhaustion, both terminal dispositions, restart recovery, project isolation, bounded storage, and atomic audit/rollback.

## Plugin contract and implementation options

1. Trusted in-process Rust implementations of a typed plugin/source API. Recommend this first for the smallest production footprint and integration with resource limits.
2. Bundled supervised components behind the same contract when protocol implementation warrants a separate service boundary. Account for process memory and crash recovery on the minimum host.
3. User-supplied external executable or remote plugins. Possible later, but require a trust, isolation, versioning, and capability model; do not quietly load arbitrary code as the initial implementation.

Declare version/capabilities, typed input/output or event payload, secret references, permitted destinations, resource/time limits, and effect semantics. The compiler rejects unknown capabilities, illegal effects in pure expressions, and missing configuration before activation. Plugin INVOKE and event actors remain subject to central permissions, and plugin output cannot bypass field/entity checks.

Separate local pure computation, local transactional work, remote reads, and remote effects. A remote read is not a deterministic pure calculation. An external write cannot inherit SQLite rollback merely by declaring itself transactional.

## Commit and external effects

Recommend a transactional outbox for effects: commit the local business state, its audit, and pending delivery intent together, then execute the external call with the common runtime scheduling and failure policy. Expose pending/succeeded/failed/unknown outcome explicitly. If an operation requires immediate external completion, define its timeout and partial-failure contract rather than promising atomic rollback.

Use stable idempotency identifiers when the destination supports them. A timeout after a remote success is an unknown outcome; check status or reconcile before blindly retrying. Compensation is another business operation, not a rollback guarantee. Deleting exhausted work cannot reverse an already performed effect.

Recheck the intended actor's current authority before a delayed effect; do not treat an old queue entry as permission to bypass revocation. Source-specific protocol behavior belongs to the plugin, while dispatch, attempt limits, and terminal policy remain common runtime mechanisms. Server-log archival uses the same connector/job infrastructure with explicit host-level configuration and separate credentials/namespace; it does not create project-specific log stores or a second scheduler.

## External-data-only endpoints

Bind a declared HTTP operation to a typed connector read or explicitly defined effect. A local application entity is not required to serve a validated external result. Still authenticate the caller, enforce operation/plugin grants, validate input, validate the full remote response against its declared type, and project only authorized output. Reject malformed, oversized, missing, or unexpected fields according to the declared contract.

Do network I/O outside the project's SQLite lock. Acquire a bounded permission/context snapshot where needed, perform the limited remote request, then revalidate current authority before delivering data or committing local writes. Never hold a database transaction open during an arbitrary network wait. Resolve any consistency requirement explicitly; do not assume the remote system and SQLite share a snapshot.

Configure trusted destinations, redirect policy, timeouts, maximum response bytes, and pagination limits. Do not accept arbitrary caller-selected URLs with server credentials. Return stable typed error categories without exposing remote bodies or secrets. Caching, stale fallback, and synchronous external writes require declared policy rather than hidden adapter behavior.

## HA event and effect ownership

The confirmed peer address/public-key pairing authenticates servers; replication and failover behavior still need decisions. If peers share a replicated project, the common event/job mechanism must coordinate ownership so only the authorized owner dispatches a given effect. Replicate accepted input, processing state, application data/audit, numeric sequence high-water marks, and completed effect state according to the chosen consistency contract.

Do not create one independent event queue per replica that sends the same email/payment twice. Worker takeover needs explicit ownership epochs or equivalent stale-owner protection and destination idempotency/reconciliation where available. HA recovery must use the same retry and terminal policy as single-server operation. Event/job/effect diagnostics include the executing server name and stable instance identity, while preserving the recorded origin of accepted events and committed mutations. Identity labels are diagnostic metadata, not authority grants. See [HA scope](variables-and-secrets.md#ha-scope-and-open-architecture).

## Real-world integration inventory

| Service | Typical use | Required behavior |
| --- | --- | --- |
| HTTP/REST APIs | Catalogs, CRM, ERP, geocoding | Typed reads/writes, bounded responses, auth, timeout and outcome handling |
| S3-compatible object storage | Uploads, downloads, attachments, server-log archives | Scoped signing, upload intents, completion verification, cleanup |
| MQTT brokers/devices | Device events and commands | Source verification, project routing, bounded connections, duplicate handling |
| Built-in mail / SMTP relay | Incoming messages and outgoing mail | Email source plugin, durable acceptance, delivery state, common failure policy |
| Mailbox access | Reading and managing mail | Typed mailbox operations, account permissions, protocol state |
| Payment providers | Payments, refunds, payment notifications | Idempotent requests where supported, verified notifications, reconciliation |
| Webhooks | Third-party notifications | Signature/replay verification, durable event acceptance, deduplication |
| SMS/push providers | Notifications and one-time codes | Delivery identifiers, expiry, recipient limits, external-outcome handling |
| Identity providers | Login and account linking | Verified identity assertions, explicit issuer trust, bounded key retrieval |
| Search/reporting services | Indexes and exports | Projection of authorized data, durable updates, rebuild/checkpoint policy |

These are concrete use cases for one contract, not a requirement to finish every connector in the first milestone. S3 and the full built-in email server have dedicated requested deliverables. MQTT already exists as a transport; external-broker integration still needs a distinct adapter implementation.

## Open decisions

- **INT-1:** What sources should be added after existing stream events and plugin inputs? Recommend entity create/update/delete and scheduled events through the same event contract, with explicit project subscriptions.
- **INT-2:** Should user-provided plugin code be supported initially? Recommend trusted built-in implementations first, retaining a versioned interface for later extension.
- **INT-3:** Where are common failure policies declared and overridden? Recommend declarative project defaults and per-handler overrides, with host ceilings and live administration of supported parameters.

Answers: Pending conversation. The universal event mechanism, plugin-specific source work, retry limits, and terminal retain/delete choices are confirmed; syntax, defaults, and override rules are not.

## Dependencies and verification

Depends on secure settings, schema migration/recovery, and bounded host resources. Deliver the universal event lifecycle before connectors that depend on it. The outbox and external-data endpoint contract follow, then S3 and the full email service.

Verify typed connector rejection, missing INVOKE grants, bad remote responses, no-entity endpoints, delayed authority loss, timeout after remote success, retry exhaustion, failed-item retention/deletion, restart at acceptance/dispatch/completion boundaries, and audit atomicity. Rework existing automation tests to prove the explicitly changed transaction boundary: committed source events survive handler failure, while each failed handler attempt leaves no committed application changes or audit. No separate event scheduler or failure-policy engine is allowed for an individual connector.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
