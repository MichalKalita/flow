# Implementation plan

## Purpose and decision status

This is the recommended dependency order for the full `.todo` scope. Prioritize reliable system operation, then public access and reusable runtime mechanisms, then applications and integrations. This is a design plan; no feature implementation is claimed by these documents.

Each topic distinguishes requested outcomes, current implementation, recommendations, and open decisions. Answer questions in conversation; record accepted decisions back in the corresponding English document. Unanswered recommendations must not be treated as approved product requirements.

## Confirmed decisions

- A complete built-in email server is required, modeled as a hosted project whose program controls application storage and reactions.
- Email input is an ordinary event, exactly like every other event. Only the source and typed payload differ. A source plugin handles mail-specific protocols, verification, and limits.
- Events are accepted and durably stored before handlers run. All sources use one runtime event lifecycle, dispatcher, permission model, and failure mechanism. Existing synchronous stream automations must be migrated explicitly to this target.
- Failure handling is configurable throughout the system, including retry limits and the terminal choice to retain or delete failed input/work.
- Email receipt limits, including individual message size, must be configurable.
- Supply one bootstrap encryption key at deployment; manage project secrets through protected administration without SSH. Back up the key separately from encrypted data.
- Protected administration may show a masked secret preview with a few leading and trailing characters. Do not send the full stored value to the browser for masking; mask short values fully where necessary.
- A short final pause of writes to the affected project is acceptable for schema migration cutover. Preparation happens while the old version remains active.

## Invariants for every phase

Preserve central application permissions, project isolation, positive numeric branded IDs, transactional allocation, and committed sequence high-water marks across restart and deletion. Every committed application mutation and its SQLite audit must share a transaction; failed attempts must not leave committed-change audit entries.

Validate complete candidates before replacing active programs/configuration/routes. Failed reloads retain the last working project and preserve data/audit while writing the project error file. Any new cross-project access uses an explicit general configuration mechanism.

Keep operational logs and telemetry outside application databases and exclude credentials, tokens, bodies, and plugin arguments. Keep the protected admin listener separate; its console uses normal application permissions. Maintain bounded memory, disk queues, retained work, and telemetry on a 1 CPU / 2 GB host, while allowing larger hosts to use more resources. No required external monitoring stack.

## Recommended sequence

| Step | Deliverable | Prerequisites | Completion evidence |
| --- | --- | --- | --- |
| 1 | [Variables and secrets](variables-and-secrets.md): typed declarations, encrypted store, generated defaults, validated live settings | Existing protected admin surface | Defaults persist once; secret values never leak; invalid activation and crash recovery preserve the working configuration |
| 2 | [Runtime logs](runtime%20logs.md): global host budget, project verbosity, severity histogram, admin settings | 1 | Combined project/host archives obey a shared limit; bounded search/histograms and redaction work under pressure |
| 3 | [Capacity guidance](benchmark.md): passive inventory and opt-in bounded calibration | 1, 2 | Isolated/cancellable diagnostics produce evidence-based suggestions; production data is untouched |
| 4 | [Local migrations and seeds](migrations.md): migration metadata, candidate validation, recovery, test/production groups | 1; use 3 for storage preflight | Old version runs during preparation; short write pause reconciles changes; failures retain working data/audit; sequences and seed history survive |
| 5 | [HTTPS and routing](https.md): built-in TLS/ACME, explicit domain/path aliases | 1; route candidate validation | Renewal/restart and shared-domain routes work; canonical project paths and protected admin isolation remain intact |
| 6 | [Universal event lifecycle](external-integrations.md#confirmed-unified-event-mechanism): common durable acceptance, dispatch, failures, typed source-plugin API | 1, 2, 4 | Existing streams and plugin sources use the same API and lifecycle; committed input survives failed handlers; retry/retain/delete recover across restart |
| 7 | [External effects and data-only endpoints](external-integrations.md#commit-and-external-effects): typed connector methods, outbox, outcome reconciliation, validated remote results | 6; 5 for public callbacks | Local writes/audit/intents are atomic; external ambiguity is explicit; permission-checked external-only endpoints validate responses |
| 8 | [Shared accounts](shared%20account.md): explicit trust, central identity project, local extensions | 1, 4, 5, 7 | Consumer profiles and roles remain local; explicit issuer/audience bindings and account lifecycle prevent implicit sharing |
| 9 | [External authentication](external%20auth.md): reusable provider login/linking, multiple providers | 1, 5, 7; 8 for cross-project login | Verified callbacks, safe linking, session/revocation behavior, and generic/named-provider support are covered |
| 10 | [Delegated permissions](subpermissions.md): bounded scopes, expiry, revocation, issuance rights | 4 and stable actor contract; 8 only for shared parents | No escalation across fields, records, plugins, or transports; parent permission loss and expiry affect active access |
| 11 | [S3-compatible storage](s3.md): signing, listing, upload lifecycle, ordinary public URLs by policy | 1, 4, 7 | Required providers pass compatibility checks; incomplete uploads are not ready files; cross-project access is rejected |
| 12 | [Generic frontend](generic%20frontend.md): project-controlled application UI and typed metadata | 5, stable metadata, 9 or another defined application login; 11 for S3 widgets | Enabled/disabled state, login, forms, validation, and permission changes pass E2E without admin privileges |
| 13 | [Cross-project migration](migrations.md#cross-project-transfer): explicit mappings, provenance, resumable copy, optional deletion phase | 4, 6, 7 and explicit cross-project configuration | Numeric IDs/references remap correctly; retries deduplicate; partial completion and separately authorized source deletion recover safely |
| 14 | [Complete email server](emails.md): source plugin, configurable receipt, submission/delivery, mailbox management, verification | 1, 2, 4, 5, 6, 7; 8 only if shared identities; 11 optional for object storage | Full receiving/sending/mailbox flows and project-defined handlers operate through universal events within bounded host resources |

This table is a delivery order, not a claim that all earlier features are hard prerequisites for every later feature. After step 7, S3 and local-account delegation can proceed independently of shared accounts. Cross-project transfer does not require shared login. Standalone external login can precede shared account federation. Email can start after its hard foundations and must not require an S3 service; the complete server remains a later milestone because of protocol and operational scope.

Certificate renewal can start with a small bounded implementation at step 5, but it must use the shared failure policy and join the common job/event infrastructure when step 6 is delivered. This is a staged integration, not a permanent second scheduler for email or certificates.

## Milestones and exit gates

### M1 — Reliable operating foundation: steps 1–4

Administrators manage settings securely; logs share a bounded host budget; capacity diagnostics are safe; incompatible schema changes have a validated, recoverable migration path. Demonstrate this on the minimum host, including audit growth and space reserved for migration/backup/event storage. Existing routing, permissions, and rollback remain covered.

### M2 — Secure public hosting: step 5

A configured shared domain routes an account prefix and an application root without ambiguity. Certificates acquire/renew and survive restart. Failed candidates keep active routes intact. HTTP/WebSocket remain project-isolated and admin APIs remain on the protected listener.

### M3 — One event and integration model: steps 6–7

Local sources and plugins share durable acceptance, dispatch, actors, retry limits, and terminal retain/delete. Update existing automation behavior explicitly: handler failure cannot undo a committed source event, but each failed handler attempt rolls back its own application mutations and audit. Verify restart at every transition, quotas, event-chain limits, schema-versioned queued payloads, and handler-version behavior.

A declared external-data-only endpoint authenticates, checks permissions, and validates the remote result without requiring local business entities. Outbox delivery never promises remote rollback or exactly-once execution without destination support.

### M4 — Identity and limited access: steps 8–10

A central identity project works with explicit consumer bindings and local extensions. Multiple configured providers can coexist. Delegated credentials are bounded by current parent permissions and revocation. Include all named providers in the delivery backlog; a generic OIDC prototype alone does not finish that topic.

### M5 — Application and storage workflows: steps 11–13

S3-compatible storage and an ordinary-user frontend are usable without admin credentials. Explicit cross-project transfer remaps references, resumes after failures, and records audit locally in each database. Optional source deletion is a separate reviewed operation, not an assumed distributed rollback.

### M6 — Full built-in mail and final capacity validation: step 14

Follow the email document's sub-milestones through receipt, submission/delivery, and complete mailbox management. A receive-only or SMTP-send-only prototype does not satisfy the full email server. Prove that email uses the same event infrastructure as other sources and that project configuration controls storage/handlers and receipt limits. Revalidate the whole system on the minimum host and scaling behavior on a larger host.

## Decision queue

Resolve the earliest choices first, while continuing design work independent of the answer:

1. **CFG-2:** live settings boundary. **CFG-1 is answered: a deployment-supplied bootstrap key and project secret administration. CFG-3 is answered: allow a masked preview showing a few first and last characters.**
2. **LOG-1–3 / CAP-1–3:** host retention policy, diagnostic safety, capacity targets, and applying guidance.
3. **MIG-2–3:** transfer deletion and seed-triggered events. **MIG-1 is answered: a short final write pause is allowed.**
4. **TLS-1–3:** aliases, wildcard requirements, and domain ownership/configuration.
5. **INT-1–3:** additional event sources, plugin extension trust, and common policy placement/overrides. The universal event/failure model itself is confirmed.
6. **ACC-1–3 / AUTH-1–3 / DEL-1–3:** provisioning, revocation, roles, provider/client priorities, and delegation defaults.
7. **OBJ-1–3 / UI-1–3:** deployment providers, public/multipart objects, frontend exposure/customization/registration.
8. **MAIL-1–3:** mailbox interfaces, raw-message retention, and direct/relay delivery. Complete built-in mail, the common event lifecycle, and configurable limits are already confirmed.

The detailed English questions/recommendations live in their topic files. Ask them in Czech in conversation, record answers, and revise dependencies when a decision changes the implementation contract. Do not ask the user to choose the general implementation order; derive it from system needs and dependencies.

## Verification and delivery workflow

Implement coherent parts and commit each with concrete English messages. Read relevant implementation/tests first and add meaningful integration coverage for behavior changes. Update runtime/language/project documentation when the feature is implemented; `.todo` designs alone do not document a shipped API.

For Rust changes, run in `runtime/` before delivery:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

For frontend changes, run in `admin-ui/`:

```sh
bun run format
bun run check
bun test src
bun run build
bun run test:e2e
```

Use the real runtime and isolated temporary databases for admin/application E2E. Commit small rebuilt assets in `runtime/src/admin-assets/` alongside frontend source. Do not commit databases, logs, secrets, or build directories.

Capacity and mail interoperability need explicit bounded operational validation in addition to unit/integration tests. Report the workload and environment; green functional tests alone do not prove production capacity or external-service compatibility.
