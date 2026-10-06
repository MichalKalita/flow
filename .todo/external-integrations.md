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

Extend this foundation deliberately for durable asynchronous jobs and external events. The existing synchronous transaction model must remain available. External effects need explicit acceptance, retry, deduplication, and failure semantics; they cannot inherit SQLite rollback guarantees. Entity-change and scheduled triggers remain design work, not implemented capabilities.
