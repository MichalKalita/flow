# Generic application frontend

## Requested outcome

Generate or provide a generic frontend that each project can enable or disable for its users. Users log in and can perform actions according to application permissions.

## Current implementation

The embedded admin UI provides an endpoint inventory, HTTP console, and privileged database editor. The console obeys application permissions; the database editor is protected administrative tooling. Neither is a public generated application frontend. See [admin UI sources](../admin-ui/src), [admin APIs](../runtime/src/admin.rs), and [project documentation](../projects/README.md).

## Implementation options and recommendation

1. A runtime-served generic renderer using typed operation metadata and explicit UI declarations. Recommend this for automatic updates and a small deployment footprint.
2. Generate a standalone frontend at build time. Useful for customization/export, but introduces rebuild and schema-version coordination.
3. Offer both eventually, using one metadata contract. Do not require both to deliver the first usable frontend.

Recommend operation-first pages/forms, not unrestricted CRUD inferred from every entity. Projects explicitly publish suitable queries and mutations with labels, layout hints, field widgets, and navigation. Entity access and permissions may depend on ownership or input values, so a static list of allowed buttons is not an authorization decision.

## Proposed application behavior

Serve the frontend on a project-scoped public path or explicit [domain alias](https.md). Disabled projects expose no generated UI or private metadata. Project selection does not grant access to another project.

Use ordinary application authentication and the same declared APIs/central permission engine as other clients. Never include the host admin token, privileged editor endpoints, internal tables, or raw SQL. Return only metadata intentionally exposed for public/app use; permission expressions and hidden schema dependencies must not leak.

Render enum choices, bounded numbers/lists, optional fields, branded numeric references, validation errors, pagination, and file operations through typed metadata. The server remains authoritative for validation, hidden fields, row access, and conflicts. Do not expose generated secrets or internal identities as editable form fields.

Recommend explicit create/edit/delete actions and version-aware updates where operations support optimistic concurrency. Display permission loss or stale forms as normal recoverable states. Support accessible keyboard interaction, project-defined labels, and localization through data rather than embedding host administration details in user flows.

## Open decisions

- **UI-1:** Should the first frontend be operation-first or automatically expose entities? Recommend operation-first with explicit metadata and an optional permitted entity browser later.
- **UI-2:** How much customization is needed initially? Recommend labels, ordering, widget hints, navigation, and theme settings before arbitrary executable templates.
- **UI-3:** Should enabling the frontend imply public registration? Recommend separate controls for UI availability, authentication, and registration; none should enable the others implicitly.

Answers: Pending conversation.

## Dependencies and verification

Depends on a stable type/operation metadata API, secure public routing, and an application login flow. Shared accounts are optional for standalone projects. File widgets depend on the chosen local/S3 storage contract; email-specific screens can build on this frontend later.

Use Playwright E2E against the real runtime and isolated databases for anonymous, signed-in, restricted, and revoked users. Cover hidden fields, cross-project attempts, disabled UI, validation, numeric reference inputs, concurrent changes, and ordinary audited mutations. Run frontend format/check/tests/build and commit regenerated embedded assets if this frontend uses the runtime asset bundle.

## Catalog integration

The [predefined project catalog](catalog.md) must launch usable applications with one click. Templates should include the frontend metadata and a working owner/login path. Installing a template must not leave a non-expert with only an API or a requirement to write screens manually.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.

## Delivered initial slice

An initial explicit string-field CRUD renderer is implemented on the public project listener. It uses ordinary application credentials, version-checked mutations, and bounded cursor pages. Contacts exercises it in real-runtime browser tests. Arbitrary schema forms, relationships, broader authentication flows, and the full frontend specification remain outstanding.
