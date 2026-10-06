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

## Proposed receipt processing

Use a durable, bounded intake queue before acknowledging accepted mail. Then let project rules choose the permanent representation: full message, selected metadata, attachments, or business records derived from the message. Temporary intake storage and permanent project storage are separate policies.

Process accepted messages with retryable jobs. Keep local entity changes and their audit records atomic. Queue outgoing delivery as an external effect; do not claim that a database rollback can undo an email already sent.

This receipt model is a recommendation awaiting user confirmation. The alternative is to run required project rules before accepting the message and return a temporary delivery failure when processing fails.

## Pending decisions

Resolve acceptance timing and failure behavior in conversation before specifying the queue contract. Later decisions include mailbox protocol coverage, permanent raw-message retention, domain/account ownership, and direct delivery versus an external SMTP relay.

## Dependencies and verification

Depends on secrets, certificates, durable jobs/external-effect semantics, schema migrations, and bounded host resource policies. Consult [external integrations](external-integrations.md) for the common event and delivery contract.

Integration coverage must include project isolation, receipt across restart, processing failures and retries, duplicate delivery handling, mailbox permissions, atomic local audit, attachment limits, and delivery recovery. Logs must exclude message bodies and credentials.
