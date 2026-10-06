# HTTPS, certificates, and public routing

## Requested outcome

Automatically obtain and renew Let's Encrypt certificates. Bind project endpoints to configured domains and allow multiple projects on one domain, including an account project at `example.com/auth` and commerce at `example.com/`.

## Current implementation

Public HTTP and WebSocket routes use the project folder prefix `/<project>/...`. MQTT selects a project in CONNECT username. There is no built-in TLS or certificate automation. Admin uses a separate protected listener, loopback by default. See [hosted routing](../projects/README.md), [HTTP routing](../runtime/src/http.rs), and [listener isolation tests](../runtime/tests/admin_bind.rs).

## Implementation options and recommendation

1. A reverse proxy can supply TLS during development/deployment, but alone does not deliver the requested server-managed certificates and routing.
2. Add built-in TLS termination and ACME certificate management. Recommend this as the target, using a maintained TLS/ACME implementation after evaluation rather than hand-writing cryptographic protocols.
3. Offer a managed proxy integration as an optional deployment mode with explicit trusted-proxy configuration.

Certificate automation should follow [ACME](https://www.rfc-editor.org/rfc/rfc8555.html). Persist certificate/account material securely, schedule bounded renewal work, and retain usable active certificates when renewal fails. Do not expose certificate keys in the admin UI or logs.

## Proposed route model

Keep the existing `/<project>/...` routes as canonical project routes. Add explicit domain/path aliases pointing to a project and local endpoint prefix; this is a general configuration mechanism rather than an exception for the account project.

For example, `example.com/auth/...` selects the configured account project and `example.com/...` selects commerce. Use explicit most-specific-prefix routing, enforce path-segment boundaries, and reject ambiguous mappings before activation. `/authentic` must not match `/auth`. Define whether the external prefix is stripped or rewritten into the project's canonical prefix consistently for HTTP, WebSocket, file URLs, and redirects.

Resolve the target project before authentication and permission evaluation. A domain alias never shares databases or identities. Normalize and validate Host/SNI and route declarations, reject undeclared hosts, and prevent client-supplied forwarded headers from choosing a project unless a trusted proxy is configured.

Apply complete routing candidates atomically; failed reload retains the last working program/routes and writes the project error file. New routes requiring a certificate must remain inactive until usable TLS is ready. Keep the protected admin listener separate and outside public alias routing.

## HA peer transport

The user requires explicit server pairing through mutually configured public keys and a reachable endpoint on at least one member. The primary/standby pair must work when only one member accepts inbound connections; the other initiates an outbound, bidirectional authenticated channel. Peer authentication is separate from public domain certificates: possession of a Let's Encrypt certificate alone does not grant membership. Use a protected encrypted inter-server surface with verification against configured peer identity keys; retain separation from public project routes and the protected admin listener. See [key generation and pairing](variables-and-secrets.md#confirmed-server-pairing).

Public application ingress is a separate deployment contract: define how traffic reaches the standby if the only publicly reachable member fails. A stable public proxy/tunnel or another reachable fallback may be used; outbound peer synchronization alone does not solve that routing case. Listener/dialer roles must not be tied to primary/standby roles.

All HA members host the same software and project route inventory. Coordinate TLS material and certificate job ownership with the full-server primary/standby model. Define certificate readiness and ingress behavior for promotion alongside the replication contract; do not run conflicting independent renewal controllers for the same deployment. Avoid conflicting certificate controllers or implicit copying of private keys.

## Open decisions

- **TLS-1:** Are aliases public alongside the existing project-prefixed paths? Recommend preserving canonical paths for compatibility and configuring additional aliases explicitly.
- **TLS-2:** Are wildcard domains required initially? Recommend ordinary explicit hostnames first; design optional DNS-based challenge credentials for wildcard support.
- **TLS-3:** Where should routing be configured? Recommend declarative project requests plus host-admin ownership/approval of domains, preventing one project from taking another's hostname.

Answers: Pending conversation. Domain ownership and alias configuration are not implemented yet.

## Dependencies and verification

Depends on [secure live configuration](variables-and-secrets.md). Certificate scheduling and failure handling should use the common system policy; source-specific ACME behavior belongs in an adapter. Public login and mailbox TLS depend on this work.

Verify alias precedence, boundary matching, unknown hosts, HTTP/WebSocket equivalence, absolute URL generation, certificate acquisition/renewal/restart, expired certificates, invalid candidate rollback, and admin listener isolation. Use a local certificate test service or ACME staging in explicit integration runs, not production issuance for every test.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
