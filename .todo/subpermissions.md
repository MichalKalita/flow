# Delegated permissions

## Requested outcome

A user can issue a limited credential for AI tools or integrations. It expires and allows only a subset of the user's permissions, potentially restricted to read access to selected records. Decide lifetime limits and who may issue such credentials.

## Current implementation

Permissions are centralized across transports with entity, field, relationship, and plugin INVOKE checks. Roles and identity are pinned to the pre-mutation state to prevent granting oneself permissions within a transaction. JWTs currently represent ordinary actors; the admin issuer's TTL range is not an existing user-delegation policy. WebSocket deliveries recheck permissions. See [permission execution](../runtime/src/engine.rs), [permission tests](../runtime/tests/permissions.rs), and [stream tests](../runtime/tests/streams.rs).

## Implementation options and recommendation

1. Store opaque random credentials as hashes with a server-side delegation record. Recommend this for immediate revocation, usage inspection, and simple bounded scopes.
2. Use signed capability tokens, still consulting revocation/parent state. This can reduce lookup needs in some environments but adds claim and key-rotation complexity.
3. A caveat-based attenuation format may support delegation chains later, but must not introduce a second permission engine.

Effective access is the intersection of the user's current application permissions, the credential's declared scope, and every parent scope if chains are enabled. A grant issued earlier must not survive the user's later permission loss. Issuing a token cannot confer permissions the user could not exercise. Never allow arbitrary permission expressions supplied by the client.

## Proposed bounded scope

Bind a credential to one project, user, audience/purpose, expiry, and allowed operations/actions. Support explicit entity/field restrictions and selected numeric record IDs using normal runtime brands. Default to read-only, with explicit plugin INVOKE grants where needed. Do not let INVOKE indirectly widen entity or output access.

Use trusted server time for expiration and bounded clock tolerance. Require a dedicated permission for issuance. Hash opaque token secrets, display the secret once, and record redacted issuance/revocation metadata. Audit mutations under both the parent actor and delegation identifier without recording the token.

Recheck expiry, revocation, parent status, and permissions for HTTP calls, MQTT sessions, WebSocket deliveries, event jobs, and outbound plugin dispatch. Already released data or an external effect cannot be recalled; enforce the scope before each new access/effect. No admin credential can be delegated through this application feature.

## Open decisions

- **DEL-1:** Who may issue delegated credentials? Recommend disabled unless the project grants an explicit delegation permission; optionally offer it to selected users/roles.
- **DEL-2:** What lifetime should be allowed? Recommend a 15-minute default and a one-hour project maximum initially, with explicit longer-lived integration policies if required.
- **DEL-3:** May a delegated credential issue another? Recommend no chains initially; if enabled later, intersect all parent scopes and expirations and bound depth.

Answers: Pending conversation. Lifetime values are proposals, not current runtime limits or approved settings.

## Dependencies and verification

Depends on [credential configuration](variables-and-secrets.md), local schema migrations, and a stable actor/account contract. [Shared accounts](shared%20account.md) are needed only when the parent identity is shared, not for local-only delegation.

Integration tests must prove no escalation through roles, fields, ownership changes, plugin arguments/results, cross-project use, guessed IDs, or transport differences. Include issuance beyond scope, expiration/revocation on active streams, parent permission loss, restart, concurrent revoke/use, and atomic mutation audit/rollback.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
