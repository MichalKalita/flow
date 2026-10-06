# Built-in email server

## Confirmed direction

Provide a complete email server built into Flow, rather than only an SMTP sending integration. This remains a second-level feature built on the runtime foundations.

Email should be modeled as a hosted project, one of many projects. Its Flow program should define what email data is stored and what happens when a message arrives. Avoid a fixed application schema that every email project must use.

Retain the option to use an external SMTP server. Enabling email for individual projects or using a dedicated email project should be explicit configuration, preserving project isolation.

## Implementation options

- Put mail protocol handling and delivery infrastructure in the runtime, with explicit domain-to-project routing. Keep the email application schema, storage mapping, permissions, and event rules in the project.
- Alternatively, bundle a mail service component behind a typed runtime adapter. This could reduce protocol implementation work, but must still provide the requested built-in deployment and project-controlled behavior. Component boundaries have not been decided.

A complete design must cover incoming mail, authenticated submission, outgoing delivery and retries, mailbox access and message management, domains and accounts, attachments, TLS, abuse controls, and operational visibility. These are parts of the requested final scope, even if delivered in separate milestones.

## Existing automation foundation

The runtime already supports conditional event automations for newly created stream records. A mutation declares an event source with `[on ...]`, a verified actor, and an optional `[when ...]` condition. The runnable `LowBattery` example creates an alert when a device reports a battery level below 20.

Handlers enforce central permissions and run inside the originating SQLite transaction. A failed handler rolls back that transaction. Event processing is bounded to 256 events per transaction. See [the example](../runtime/application.flow), [the implementation](../runtime/src/engine.rs), and [integration tests](../runtime/tests/streams.rs).

This is a foundation, not an existing email service or a general background job system. General entity-change triggers, scheduled jobs, and durable asynchronous processing need a separate design. Extend the common automation mechanism so email uses the same model as other event sources.

## Confirmed event input contract

Provide a basic typed email API with an incoming-message event input. This input belongs to the email transport adapter; it is not a REST endpoint. The event represents a message that has already occurred, been accepted, and been durably stored at the input boundary.

Receipt must precede event handling. Project business rules run only after the input has been committed. A failed rule must not roll back an already accepted message. This ordering is a general principle for accepted external event inputs, not an email-only exception.

The project chooses the permanent application representation: full message, selected metadata, attachments, or derived business records. Durable input storage remains necessary even when the project does not keep the full message permanently. Its retention and cleanup policy still need a decision.

The current synchronous stream automations run before the originating transaction commits. They therefore do not yet implement this accepted-input contract. Add a distinct durable event-input path; preserve existing transactional automations without silently changing their guarantees.

## Proposed implementation of the confirmed contract

1. Select the destination project through explicit mail routing and validate receipt constraints.
2. Persist the accepted message and pending processing state durably before acknowledging successful receipt. The message and pending state must recover together; a crash must not leave an accepted message without discoverable work.
3. Deliver a typed incoming-message event to the configured project handler using a trusted, permission-checked actor. Treat sender addresses and message headers as untrusted input, not authenticated Flow identities.
4. Commit the handler's local application changes, their audit records, and processing completion atomically. Failure leaves the accepted input available for recovery.
5. Release temporary input storage only according to the decided retention and recovery policy. Outgoing delivery remains an external effect with separate failure semantics.

The exact storage layout, retry policy, event envelope, and API syntax are proposals still to be specified. Preserve positive numeric, project-local IDs and branded references. For large messages, prefer a bounded metadata event with a controlled payload handle rather than loading the entire message into every handler.

## Configurable receipt limits

Message size must be configurable and enforced while receiving data, before accepting an oversized message. Project settings should also support recipient count, attachment count and size, parsing complexity, concurrent connections, receive deadlines, and accepted-input queue capacity.

Combine project-specific settings with host-wide resource ceilings so many projects cannot exhaust the minimum 1 CPU / 2 GB RAM host. Use bounded buffering and storage. Reject or temporarily defer new input when capacity is exhausted; never acknowledge receipt and silently discard accepted mail. Exact defaults and supported settings remain to be designed.

## Pending decisions

Resolve retry, failed-message retention, and administrator recovery behavior in conversation. Later decisions include mailbox protocol coverage, permanent raw-message retention, domain/account ownership, and direct delivery versus an external SMTP relay.

## Dependencies and verification

Depends on secrets, certificates, durable jobs/external-effect semantics, schema migrations, and bounded host resource policies. Consult [external integrations](external-integrations.md) for the common event and delivery contract.

Integration coverage must include project isolation, receipt across restart, processing failures and retries, duplicate delivery handling, mailbox permissions, atomic local audit, receipt and attachment limits, host-wide capacity enforcement, and delivery recovery. Verify that handlers never run before durable input acceptance and that handler failure cannot remove an accepted message. Logs must exclude message bodies and credentials.
