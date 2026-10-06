# Built-in email server

## Confirmed direction

Provide a complete email server built into Flow, rather than only an SMTP sending integration. This remains a second-level feature built on the runtime foundations.

Email should be modeled as a hosted project, one of many projects. Its Flow program should define what email data is stored and what happens when a message arrives. Avoid a fixed application schema that every email project must use.

Retain the option to use an external SMTP server. Enabling email for individual projects or using a dedicated email project should be explicit configuration, preserving project isolation.

## Confirmed plugin boundary

Use an email source plugin (working terminology) for mail-specific protocols, parsing, verification, domain routing, and receipt limits. It supplies typed email events through the same runtime event-input API as every other source. An email event differs in its source and payload type, not in its lifecycle or processing mechanism.

The runtime owns the common durable event lifecycle, dispatch, permissions, retry policy, and terminal retention or deletion. The project owns its application schema, stored business data, and event handlers. The plugin does not implement a separate event queue, scheduler, or failure-policy engine.

The plugin may be implemented as an in-process module or a bundled component behind the same typed source interface. Packaging remains an implementation choice; the complete built-in server and unified event contract are requirements.

A complete design must cover incoming mail, authenticated submission, outgoing delivery and retries, mailbox access and message management, domains and accounts, attachments, TLS, abuse controls, and operational visibility. These are parts of the requested final scope, even if delivered in separate milestones.

## Existing automation foundation

The runtime already supports conditional event automations for newly created stream records. A mutation declares an event source with `[on ...]`, a verified actor, and an optional `[when ...]` condition. The runnable `LowBattery` example creates an alert when a device reports a battery level below 20.

Handlers enforce central permissions and run inside the originating SQLite transaction. A failed handler rolls back that transaction. Event processing is bounded to 256 events per transaction. See [the example](../runtime/application.flow), [the implementation](../runtime/src/engine.rs), and [integration tests](../runtime/tests/streams.rs).

This is a foundation, not an existing email service or a general background job system. General entity-change triggers, scheduled jobs, and durable asynchronous processing are not implemented yet. Evolve this foundation into one common event mechanism for all sources, including email.

## Confirmed event input contract

Provide a basic typed email API with an incoming-message event input. The email source plugin supplies this input; it is not a REST endpoint. The event represents a message that has already occurred, been accepted, and been durably stored at the input boundary.

Receipt must precede event handling. Project business rules run only after the input has been committed. A failed rule must not roll back an already accepted message. This ordering is the common contract for every event in the system, regardless of its source.

The project chooses the permanent application representation: full message, selected metadata, attachments, or derived business records. Durable input storage remains necessary even when the project does not keep the full message permanently. Its retention and cleanup follow the common configurable failure policy; exact defaults remain undecided.

The current synchronous stream automations run before the originating transaction commits. They therefore do not yet implement this accepted-input contract. Migrate them explicitly to the unified event lifecycle rather than maintaining a second event mechanism. Each source transaction and each handler transaction must still preserve atomic application mutations and audit, but a failed handler must not undo the committed source event. Document and test this change to existing automation semantics.

## Proposed implementation of the confirmed contract

1. The email plugin selects the destination project through explicit mail routing and validates mail-specific receipt constraints.
2. Use the common runtime event-input API to persist the accepted message and pending processing state durably before the plugin acknowledges successful receipt. The message and pending state must recover together; a crash must not leave an accepted message without discoverable work.
3. The common runtime dispatcher delivers the typed incoming-message event to the configured project handler using a trusted, permission-checked actor. Treat sender addresses and message headers as untrusted input, not authenticated Flow identities.
4. Commit the handler's local application changes, their audit records, and processing completion atomically. On failure, roll back that attempt and apply the common failure policy.
5. Release temporary input storage after successful processing or an explicit terminal deletion policy; retain it when the terminal policy selects storage. Outgoing delivery remains an external effect with separate failure semantics.

The exact storage layout, retry defaults, event envelope, and API syntax are proposals still to be specified. Preserve positive numeric, project-local IDs and branded references. For large messages, prefer a bounded metadata event with a controlled payload handle rather than loading the entire message into every handler.

## Configurable receipt limits

The email plugin must enforce configurable message size while receiving data, before accepting an oversized message. Project settings should also support recipient count, attachment count and size, parsing complexity, concurrent connections, receive deadlines, and accepted-input queue capacity.

Combine project-specific settings with host-wide resource ceilings so many projects cannot exhaust the minimum 1 CPU / 2 GB RAM host. Use bounded buffering and storage. Reject or temporarily defer new input when capacity is exhausted; never acknowledge receipt and silently discard accepted mail. Exact defaults and supported settings remain to be designed.

## Confirmed common failure policy

Email uses the same configurable failure mechanism as every other event and system mechanism. Configuration determines the retry limit and the final action after unsuccessful attempts: retain the failed input or delete it. These are supported policy choices, not an email-specific hard-coded behavior.

A processing failure cannot roll back receipt. Explicit deletion after exhausting the configured attempts is a separate lifecycle action and is allowed. Retention must remain bounded through configured quotas and lifecycle rules; deletion must not erase committed mutation audit history.

Retry delays, default attempt limits, configuration ownership, and manual recovery are still design decisions. See [the common failure contract](external-integrations.md#confirmed-common-failure-policy).

## Open decisions

- **MAIL-1:** Which mailbox-access interfaces are required for the complete server? Recommend standards-based mailbox access plus typed project operations; determine whether additional legacy interfaces are necessary. Incoming mail, outgoing mail, and mailbox management remain in the final scope.
- **MAIL-2:** What raw-message retention should apply after successful processing? Recommend project-configured retention, separate from failed-input disposition and permanent application storage.
- **MAIL-3:** Must outgoing delivery work directly to other mail servers, through a relay, or both? Recommend supporting both explicitly; delivery policies and credential requirements differ.

Answers: Pending conversation. Receipt before dispatch, plugin-specific mail work, the universal event lifecycle, configurable limits, bounded retries, and terminal retain/delete choices are confirmed. Remaining common policy defaults and configuration ownership are tracked in [external integrations](external-integrations.md).

## Delivery milestones

1. Implement the generic event/plugin foundation; do not introduce a provisional email-only engine.
2. Provide the receiving plugin with domain/recipient validation, configurable limits, durable acceptance through the common API, and project handlers controlling application storage.
3. Add authenticated submission, direct outgoing delivery and relay configuration, delivery-status handling, and recovery through the common failure mechanism.
4. Complete mailbox access, folders/message management, attachments, aliases/accounts, transport TLS, source verification/abuse controls, and administrative diagnosis. A receive-only or send-only release is an intermediate milestone, not completion of the requested full server.

Mail-specific transport identity, authentication, and verification belong to the plugin; business authorization belongs to central runtime permissions. Operational diagnostics must never store message bodies in logs. Capacity validation must include mail payload storage, delivery work, and spam/connection pressure.

## Dependencies and verification

Depends on secrets, certificates, durable jobs/external-effect semantics, schema migrations, and bounded host resource policies. Consult [external integrations](external-integrations.md) for the common event and delivery contract.

Integration coverage must prove that email uses the same event-input API, dispatcher, retry policy, and terminal disposition as other sources. It must also include project isolation, receipt across restart, processing failures and retries, duplicate delivery handling, mailbox permissions, atomic local audit, receipt and attachment limits, host-wide capacity enforcement, and delivery recovery. Verify that handlers never run before durable input acceptance and that handler failure cannot roll back acceptance. Verify both configured terminal outcomes: retention and deletion after retry exhaustion, including recovery across restart. Logs must exclude message bodies and credentials.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
