# One-click project catalog

## Confirmed outcome

Provide a predefined catalog of ready-to-use projects. A user can select a project and start using it with literally one click.

The catalog is for individuals and small and medium-sized businesses without infrastructure expertise. Installation must not require editing Flow files, running shell commands, or tuning technical settings.

## Current implementation

The repository already includes sample hosted projects for bookstores, clinics, delivery, gyms, hotels, jobs, libraries, restaurants, ridesharing, schools, social networks, and tracking. They provide starting material, not an existing one-click catalog or verified production templates. See [hosted projects](../projects/README.md).

## Recommended implementation

Ship a curated, versioned list of project templates with the server. The catalog can work offline and does not require a separate marketplace service. Each entry has a plain-language name, short description, version, and a button such as “Use this project.”

A template includes the application program, compatible schema, frontend metadata, production bootstrap data, and declared configuration/dependencies. Validate supported versions and template contents before making an entry installable. Never install shared development credentials or demo identities as production defaults.

## One-click installation

Use good defaults for the project name, paths, storage, and ordinary settings. Allocate a fresh project instance and database, generate required local secrets once, and establish the installing user's access through an explicit identity binding. Preserve normal project permissions and numeric entity IDs.

The click must create a usable application, not merely copy files and then require a technical setup wizard. Ordinary catalog entries should work immediately using available server capabilities. If an optional external feature needs credentials or a domain, collect those through the shared server setup first or leave that optional feature disabled; do not fake its configuration or block basic application use.

Stage and validate the complete candidate before publishing the project. Record bootstrap application writes and audit atomically. A failed install must not expose a half-created application, overwrite an existing project, or lose user data. Deduplicate retries and double clicks so they do not accidentally create duplicate instances. Show progress and open the ready application after success.

Installation requires permission to create a hosted project. The catalog must not expose the protected admin credential to application clients. Explicitly declared account or project dependencies use the general sharing/configuration mechanism, not implicit shared identities.

## Lifecycle and HA

Record the installed template/version and configuration so backups can restore the instance and migrations can update it. Template updates must preserve data and local changes, validate compatibility, and retain the working version on failure. Removing an application must not silently delete its data.

In HA, install through the primary and replicate the full project/program/database state to every member under the common durability contract. Do not perform independent installations or execute bootstrap events again on standbys.

## Open decisions

- **CAT-1:** Which applications belong in the initial catalog? Recommend a small polished selection of real everyday tools, using reviewed sample projects as starting points.
- **CAT-2:** May users install multiple instances of one template? Recommend yes, with generated unique project names and fully isolated data.
- **CAT-3:** Should catalog updates arrive with server releases or from an optional remote catalog? Recommend bundled releases initially; remote updates can later use the same verified versioned template format.

Remaining answers: Pending conversation. A predefined catalog and actual one-click use are confirmed; the initial application list and distribution/update policy are not.

## Dependencies and verification

The initial bundled catalog entry belongs in the first usable release, after a small working frontend and verified backup/restore. It does not need every external identity provider or storage adapter. Full catalog/version/update coverage still follows its dependencies.

Depends on [good defaults and secret generation](variables-and-secrets.md), [migrations and seeds](migrations.md), [application login](external%20auth.md), and [the generic frontend](generic%20frontend.md). Templates can declare optional [integration](external-integrations.md) requirements. Installed projects participate in [backup/restore](backups.md) and full-server HA.

Use real-runtime integration and Playwright E2E coverage for one-click installation to first usable screen, isolated repeated instances, owner permissions, generated credentials, double clicks/retries, invalid templates, failed activation, version updates, backup restoration, and HA replication without duplicate bootstrap effects. Validate the catalog's representative applications on the minimum host.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Keep installation choices behind suitable defaults; advanced customization remains optional.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.

## Delivered initial slice

The bundled Contacts template now installs from protected administration with one click and opens a usable permission-checked CRUD frontend. Install retries are idempotent; separate instances have isolated databases, identities, and generated keys. Runtime integration and browser tests cover installation, edits, backup, restoration, and restart. Additional templates, update/migration lifecycle, remote catalogs, and HA remain outstanding.
