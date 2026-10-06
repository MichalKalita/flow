# Variables and secrets

## Requested outcome

Projects declare typed variables and secrets, including persistent generated defaults. Administrators can change project settings while the server is running, without SSH. Secrets must not be stored as plaintext.

## Current implementation

`project.json` maps JWT adapters and event actors to environment variable names. The hosted loader reads those values into runtime configuration. There is no general variable declaration, encrypted secret store, or settings editor. Changing process environment values requires a restart. See [project configuration](../projects/README.md) and [the loader](../runtime/src/projects.rs).

## Implementation options and recommendation

1. Extend environment-only configuration. This is simple, but cannot satisfy persistent generated defaults and live administration on its own.
2. Add a protected, versioned configuration store with encrypted secrets and keep environment references as an import/override option. Recommend this approach.
3. Support an external secret manager through an optional adapter later. It must not become required infrastructure on the minimum host.

Keep project namespaces isolated. Separate application settings from host controls such as worker counts and global disk budgets. On the first start, generate the server's own cryptographic key material using cryptographic randomness. Persist it with restricted access and reuse it on subsequent starts; do not generate replacement keys on every restart. Keep private key material separate from ordinary project configuration. Encryption, key rotation, and recovery must use reviewed cryptographic APIs rather than custom encryption.

## Confirmed defaults and guided setup

All server settings have defined defaults, and a setup/configuration wizard recommends suitable values. This includes server naming/identity, local log retention and chunk size, optional S3 archival, resource limits, peer/HA settings, and enabled feature configuration. Project settings participate through their declared types, defaults, constraints, and required inputs.

Provide a valid standalone starting configuration. Generate server identity and key material automatically. Optional external capabilities can default to disabled/unconfigured until required peer addresses, public keys, service destinations, or credentials are supplied; never invent valid external credentials or trusted peers. The exact numeric defaults are implementation choices still to be calibrated.

The wizard shows the current/default value, recommended value, and a short reason. Derive resource/storage recommendations from detected CPU, memory, free space, and optional workload measurements. Offer ordinary safe presets where the workload is unknown, labeled as estimates. The administrator can accept or change recommendations within validated limits.

Use one versioned typed settings/defaults contract for startup, persistent configuration, admin forms, and the wizard. Initialize persistent generated defaults once rather than recomputing secrets/identity on each wizard run. Record explicit overrides so routine restart or re-running the wizard does not silently replace them. Validate the complete candidate and apply supported settings live; clearly identify any change requiring restart.

## Proposed declaration and lifecycle

The original idea `[secret TOKEN [default [randomString [length 20]]]]` is illustrative syntax, not supported Flow syntax. Declare type, required/optional status, and default generation explicitly. Generate a missing default once using cryptographic randomness, persist it, and reuse it across reloads and restarts. Choose entropy by intended use; a generic length of 20 is not automatically suitable for every signing key.

Validate a complete candidate before activation. Failed candidate validation must preserve the active configuration and last working program. Concurrent initialization must not generate multiple committed defaults. Coordinate persisted configuration versions, program activation, and recovery so a crash cannot expose a half-applied candidate.

Represent a secret as an opaque capability usable only by authorized native/plugin consumers. Do not expose the full value through ordinary query projection, errors, logs, cached source, or admin read APIs. The protected settings editor supports replacement and a masked preview showing a few leading and trailing characters, with the middle hidden. Generate the preview on the server; do not send the full secret to the browser merely to mask it there. Fully mask short values when showing both ends would reveal the entire value or too much of it. Keep previews out of operational logs and ordinary application APIs. Key rotation requires an explicit overlap/revocation policy for credentials already in use.

## Confirmed decisions

- **CFG-1 (revised):** The server generates its own encryption key material on first startup and persists it for later starts. An administrator-supplied bootstrap key is not required. Manage ordinary project secrets through protected administration without SSH. Preserve protected key backups for recovery.

- **CFG-3:** Allow a simple masked preview of stored project secrets in protected administration: a few initial and final characters, with the middle hidden. Full-value reveal is not required by this decision. The exact preview length is an implementation detail; short secrets must remain fully masked where necessary.

## Confirmed server pairing

For an HA deployment, configure the address and public key of server B on server A, and the address and public key of server A on server B. This explicit reciprocal trust pairs the servers. Each server retains its own private keys; no private key needs to be entered on the peer.

Display the server's public identity key in full for copying into peer configuration. It is public pairing material, unlike the masked preview of project secrets. Authenticate possession of the corresponding private key before admitting the configured peer; an address or unverified presented public key alone is not sufficient.

## Confirmed server identification requirement

Every server must be clearly identifiable by name/identity in logs and throughout system diagnostics and administration. Multiple servers may archive to the same S3 destination, so each archive must have an unambiguous originating server. The user has not selected hashing a key as the identifier scheme.

Recommend a human-readable configurable server name plus a generated persistent instance identifier. The name is for operators; the identifier distinguishes instances even if names are reused. Display both where useful, and retain the instance identifier across restarts, renaming, and key rotation. A public-key fingerprint is separate peer-verification metadata, not the only log identity.

The instance identifier is host/protocol metadata, not a branded application entity ID. Existing Flow entity IDs and references remain positive numeric values. Choose the exact identifier encoding during implementation; do not introduce textual type prefixes into application IDs.

Persist instance identity with first-start state. A new server gets its own identity; restoring the same server preserves it. Define an explicit clone/new-instance operation so copying a deployment cannot silently make two active servers claim one identity. Reject duplicate identities during peer configuration/admission. Use the stable instance identifier for S3 namespaces; readable names remain metadata, so renaming does not split or overwrite archive ownership.

## Proposed key and peer lifecycle

Use a stable server identity key pair for peer authentication and appropriately separated local encryption keys for stored secrets. The exact algorithms and key hierarchy are implementation choices to evaluate with reviewed libraries. Do not reuse one raw key for incompatible cryptographic purposes.

Initialize keys and the encrypted store safely, with concurrent-start protection and recoverable initialization state. If existing encrypted data is present but its key is missing or invalid, report a recovery error instead of silently generating a new key that cannot decrypt the data. Support explicit key rotation while preserving decryptability and configured peer trust.

Establish an encrypted peer channel with mutual proof against the configured public keys. Keep this on a protected inter-server surface, separate from public application routes and the existing protected admin UI. Validate peer configuration candidates before activation; do not trust discovery results automatically. Pairing config, connection status, and fingerprints belong in protected server administration.

If configuration or secrets are replicated after the HA scope is decided, transfer them only over authenticated encrypted channels and protect them under the receiving server's own storage keys. Pairing is a host-level trust relationship; it does not implicitly grant cross-project application identities, permissions, or entity access.

## HA scope and open architecture

Reciprocal pairing is confirmed. The availability/replication model is not: decide whether nodes hold the same project data, which node may write, how failover happens, and what state is synchronized. Recommend one authoritative writer per replicated project as an initial design option; multiple servers can own different projects if explicitly configured.

Data replication must preserve application mutations together with audit, committed numeric ID sequences, durable events, and effect completion state. Prevent concurrent ownership and duplicate external effects during disconnects/recovery. Program/schema migration and secret rotation must coordinate with the chosen replica model. Address/public-key exchange establishes trust, not data replication or leader election by itself.

## Open decisions

- **CFG-2:** Which settings must apply live? Recommend live application settings, connector credentials, and logging settings; label settings requiring listener/process restart clearly.
- **HA-1:** Should both peers serve the same projects concurrently, or should one act as the active server and the other as standby? Recommend one writer per replicated project first, with explicit ownership and safe takeover.
- **HA-2:** Which state should pairing synchronize? Define projects/programs, application data/audit, ID sequences, event/effect state, configuration/secrets, and TLS material explicitly. Server operational logs remain local to each server unless remote access is explicitly configured.
- **HA-3:** What data-loss and outage tolerance is acceptable for failover? Decide acknowledgement/replication and disconnect behavior before choosing automatic takeover.

Remaining answers: Pending conversation. Unanswered recommendations are not approved decisions.

## Dependencies and verification

This is the first foundation for [routing/TLS](https.md), [external integrations](external-integrations.md), and [authentication](external%20auth.md). The protected admin listener remains the management surface; its HTTP console keeps ordinary application permissions.

Verify server name/identity persistence, rename and key rotation without identity changes, distinct instances sharing one archive bucket, clone/duplicate-identity handling, server-key generation/reuse, concurrent first startup, missing/corrupt-key recovery, rejected unknown or mismatched peers, mutual authentication, restart and rotation, generated-default reuse, concurrent initialization, encryption-at-rest, project isolation, invalid-candidate rollback, restart recovery, rotation, masked-preview behavior for short and long values, absence of full secrets from preview responses, redaction, and version conflicts. Test the settings flow against the real runtime with an isolated database. Keep application mutation audit atomic; record configuration administration with redacted metadata and never secret values.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
