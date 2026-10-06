# External authentication

## Requested outcome

Projects can enable multiple external login providers simultaneously, including Apple, Google, Facebook, Microsoft, and generic standards-based providers.

## Current implementation

The runtime verifies configured HS256 JWTs and API keys and maps credentials to local actor entities. Roles are read from SQLite rather than accepted from client claims. The admin can issue existing-identity JWTs; this is not a public login service or provider callback implementation. There is no OIDC browser login, provider key discovery, or account-linking flow. See [authentication](../runtime/src/engine.rs), [adapter declarations](../runtime/src/program.rs), and [HTTP tests](../runtime/tests/http.rs).

## Implementation options and recommendation

1. Implement browser login independently in each project. Flexible, but duplicates provider flows and linking logic.
2. Provide a reusable identity plugin/API, usable by a standalone project or the dedicated account project. Recommend this with explicit project configuration.
3. Allow a trusted external identity broker as another standards-based provider. Do not make a broker mandatory to use Flow.

Use [OpenID Connect](https://openid.net/specs/openid-connect-core-1_0.html) for authentication where supported, with the authorization-code flow and OAuth protections from [RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html). Provider-specific adapters may be required; do not assume all named providers expose an identical OIDC interface. Evaluate their official contracts when implementing each adapter.

## Proposed login flow

Declare provider alias, issuer/endpoints, client identifier, secret reference, allowed scopes, and exact callback URL. Validate the whole configuration before activation. Protect browser flows with short-lived single-use state, PKCE, and OIDC nonce/claim validation as applicable. Provider/network work must have bounded timeouts and response limits.

Validate signatures, issuer, audience, expiry, and provider-specific requirements before linking to a local account. Cache trusted keys with bounded lifetime and a safe refresh policy. Do not let a token choose arbitrary discovery URLs or algorithms.

Use issuer plus provider subject as the identity key. Never auto-link accounts by email alone. Require authenticated linking/unlinking and ensure removing a provider does not accidentally lock out the account. Store any provider tokens only if a declared feature needs them, using encrypted secrets and minimal scopes.

Recommended browser sessions use secure HTTP-only cookies with explicit CSRF protection; API clients can use separately defined credentials. Session duration, refresh, logout, revocation, and cookie/domain scope need decisions. Authentication proves identity; project permissions remain local and central.

If a login/provider operation emits an event, it uses the universal committed-event mechanism. Protocol request/response handling belongs to the plugin; project handlers do not reinterpret an unverified callback as an authenticated event.

## Open decisions

- **AUTH-1:** Which providers must ship in the first milestone? Recommend generic OIDC plus the first provider actually required, then add all named providers in explicit follow-up milestones.
- **AUTH-2:** Should external login live in a central account project or also be usable independently? Recommend the reusable plugin supports both, with no automatic trust between projects.
- **AUTH-3:** Which clients need login: browser, mobile, CLI, or all? Recommend defining browser sessions and API credentials separately before adding native-client callback flows.

Answers: Pending conversation. The named-provider scope remains in the final plan even if delivered incrementally.

## Dependencies and verification

Depends on [secret storage](variables-and-secrets.md), [TLS and callback routing](https.md), typed outbound networking, and an account-link model. Shared login between projects also depends on [explicit account trust](shared%20account.md).

Test multiple providers, mismatched issuer/audience, invalid signatures, stale keys, replayed callbacks, state/nonce/PKCE failures, linking collisions, provider outages, logout/revocation, and project isolation. Use local provider fixtures and admin/application E2E flows, excluding production credentials and provider tokens from logs.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
