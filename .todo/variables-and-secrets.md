# Variables and secrets

## Requested outcome

Projects declare typed variables and secrets, including persistent generated defaults. Administrators can change project settings while the server is running, without SSH. Secrets must not be stored as plaintext.

## Current implementation

`project.json` maps JWT adapters and event actors to environment variable names. The hosted loader reads those values into runtime configuration. There is no general variable declaration, encrypted secret store, or settings editor. Changing process environment values requires a restart. See [project configuration](../projects/README.md) and [the loader](../runtime/src/projects.rs).

## Implementation options and recommendation

1. Extend environment-only configuration. This is simple, but cannot satisfy persistent generated defaults and live administration on its own.
2. Add a protected, versioned configuration store with encrypted secrets and keep environment references as an import/override option. Recommend this approach.
3. Support an external secret manager through an optional adapter later. It must not become required infrastructure on the minimum host.

Keep project namespaces isolated. Separate application settings from host controls such as worker counts and global disk budgets. Use a bootstrap encryption key supplied independently of the encrypted store; copying a store together with its key must not be presented as protection against host compromise. Encryption, key rotation, and recovery must use reviewed cryptographic APIs rather than custom encryption.

## Proposed declaration and lifecycle

The original idea `[secret TOKEN [default [randomString [length 20]]]]` is illustrative syntax, not supported Flow syntax. Declare type, required/optional status, and default generation explicitly. Generate a missing default once using cryptographic randomness, persist it, and reuse it across reloads and restarts. Choose entropy by intended use; a generic length of 20 is not automatically suitable for every signing key.

Validate a complete candidate before activation. Failed candidate validation must preserve the active configuration and last working program. Concurrent initialization must not generate multiple committed defaults. Coordinate persisted configuration versions, program activation, and recovery so a crash cannot expose a half-applied candidate.

Represent a secret as an opaque capability usable only by authorized native/plugin consumers. Do not expose it through ordinary query projection, errors, logs, cached source, or admin read APIs. The editor should show presence/version and support replacement, not display plaintext by default. Key rotation requires an explicit overlap/revocation policy for credentials already in use.

## Confirmed decision

- **CFG-1:** Supply one bootstrap encryption key at deployment, for example through an environment variable. Manage ordinary project secrets through the protected administration interface without SSH. Back up the bootstrap key separately from encrypted data.

## Open decisions
- **CFG-2:** Which settings must apply live? Recommend live application settings, connector credentials, and logging settings; label settings requiring listener/process restart clearly.
- **CFG-3:** Should an administrator ever reveal a stored secret? Recommend write-only management, with a one-time display only for newly issued credentials when necessary.

Remaining answers: Pending conversation. Unanswered recommendations are not approved decisions.

## Dependencies and verification

This is the first foundation for [routing/TLS](https.md), [external integrations](external-integrations.md), and [authentication](external%20auth.md). The protected admin listener remains the management surface; its HTTP console keeps ordinary application permissions.

Verify generated-default reuse, concurrent initialization, encryption-at-rest, project isolation, invalid-candidate rollback, restart recovery, rotation, redaction, and version conflicts. Test the settings flow against the real runtime with an isolated database. Keep application mutation audit atomic; record configuration administration with redacted metadata and never secret values.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
