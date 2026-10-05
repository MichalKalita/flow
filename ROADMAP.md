# Runtime roadmap

Implement and verify one step at a time. Each step should have a coherent commit,
updated documentation and meaningful integration tests.

1. Numeric entity IDs: implemented with branded runtime values, SQLite INTEGER
   keys, transactional allocation and numeric JSON. New databases are sufficient;
   do not migrate or erase existing text-ID databases.
2. Verified request author: expose `request.actor` and `request.actor.id` from
   authenticated server context, without a caller-supplied author parameter.
   Preserve normal permissions and reject attempts to spoof that context.
3. Multiple projects: use a root directory containing named project folders.
   Serve HTTP on one listener, initially port 80, under `/<folder-name>/...`.
   Keep databases and telemetry isolated. Admin should switch between a project
   and the entire system. Write load errors to `<project-name>-error.txt`.
   Automatic HTTPS certificates and port 443 are deferred.
4. Automatic reload: watch source changes, validate before switching, and retain
   the last working project on failure. Write validation errors to the project's
   error file; a failed initial load must not stop other projects.
5. Project integration: design shared identities and explicit cross-project
   contracts for a company interface. Do not infer identity equivalence from equal
   local numeric IDs in different databases.
6. Admin database editor: browse, search and edit application records as a fully
   privileged administrator. All writes must still validate data and references
   and record audit entries atomically. Keep access on the protected admin listener.

Steps 1–4 are implemented and covered by integration tests. Step 6 has a validated,
audited backend and a frontend undergoing end-to-end verification. Step 5 is
deferred at the user's request: future sharing must be explicit and general, not
automatic user sharing. HTTPS remains deferred.
