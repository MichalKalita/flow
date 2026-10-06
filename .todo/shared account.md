# Shared accounts across projects

## Requested outcome

Use a dedicated account project for shared user accounts, with other projects extending accounts through their own local profiles and application data. The sharing mechanism must be explicit and general enough for different project arrangements.

## Current implementation

Projects have separate actor entities, databases, authentication configuration, and permissions. JWT subjects are external identity strings mapped to local numeric actor IDs. Equal numeric IDs or shared environment secrets do not imply account sharing. No cross-project account federation exists. See [hosted isolation](../projects/README.md), [authentication](../runtime/src/engine.rs), and [authorization/audit tests](../runtime/tests/project_authz_audit.rs).

## Implementation options and recommendation

1. A shared database/table with global users. This would undermine current isolation and makes local permissions and schema ownership difficult.
2. A dedicated identity project issuing audience-bound credentials, with explicit trust and local identity mappings in consumer projects. Recommend this.
3. General permission-checked cross-project queries can provide live profile information where needed, but should be an explicit optional sharing capability rather than implicit access to every account field.

Define a general project trust/binding configuration. It should specify the trusted issuer/project, intended audience, allowed claims or profile data, identity linkage, and revocation behavior. Do not hard-code a special project name such as `auth` into the runtime.

## Proposed identity and extension model

The account project owns login identifiers, linked providers, and account lifecycle. A consumer project owns its local actor and roles. Store a unique mapping from trusted issuer and external subject to a local numeric actor ID. Its profile references that local actor; other projects do not reuse its numeric ID as a branded local reference.

Keep source account provenance separate from local identity. Linking and unlinking require proof of control; never merge identities only because email addresses match. One project can trust multiple identity sources through explicit configuration.

Recommend local roles and application permissions evaluated from consumer data, not broad global roles supplied by an untrusted token. A shared account must not automatically grant access to every project. Define provisioning as invitation, explicit enrollment, or configured first-login creation; each option needs normal typed validation and atomic audit.

Account disablement and deletion need a documented propagation contract. Options are live validation on each request, short-lived credentials with bounded revocation delay, or a local status projection updated by common events. Do not claim immediate revocation from an asynchronous projection without an additional freshness check. Consumer outages must not silently break isolation or delete application history.

## Open decisions

- **ACC-1:** Should a first login automatically create a consumer project's local profile? Recommend explicit per-project provisioning policy, with invitation-only as the conservative default.
- **ACC-2:** Must central account disablement take effect immediately everywhere? Recommend defining a maximum delay; immediate enforcement requires an online/freshness check and a failure policy.
- **ACC-3:** Are roles shared or local? Recommend local application roles, with explicitly mapped shared claims only when a project chooses them.

Answers: Pending conversation.

## Dependencies and verification

Depends on [migrations](migrations.md), [secrets](variables-and-secrets.md), [public TLS/routing](https.md), and the common typed integration/event contract. It supports [external login](external%20auth.md), [delegated permissions](subpermissions.md), and [the generic frontend](generic%20frontend.md).

Verify two projects with the same local numeric ID but different accounts, audience/issuer rejection, explicit trust only, multi-issuer linkage, profile provisioning rollback/audit, local roles, disablement/deletion, unavailable identity service, and attempted cross-project escalation. UI/API surfaces must not reveal other projects' account data implicitly.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
