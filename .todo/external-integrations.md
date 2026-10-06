# External integrations

This system should allow generic external integration

Core should be able to use it, plan with it.

External services should have exact behavior how its handled, like commit and rollback. In external services is not possible to rollback by definition, find some generic ways how to solve it.
I want to find a way to work how standards works.

Write list of real life needed external services, S3, REST API, MQTT, SMTP, ....

It should be possible to write a rest endpoint, where only external data will be used, validation MUST be there in some way, find way how to implement it.

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
