# Flow project guidelines

- The runtime lives in `runtime/` and is implemented in Rust. Preserve centralized permissions and atomic transactions across all transports.
- Write all new or modified code, identifiers, comments, user-facing strings, documentation, and commit messages in English.
- Read the relevant implementation and tests before making changes. Cover behavior changes with meaningful integration tests and update the documentation.
- Every committed application mutation must have a SQLite audit record in the same transaction as the data. Rollbacks must not leave committed-change audit entries.
- Keep application logs and telemetry outside the application database. Never log credentials, tokens, request bodies, or plugin arguments.
- Keep the admin interface on a separate protected listener. Its HTTP console must use normal application authentication and permissions.
- Production target: a VPS with 1 shared CPU and 2 GB RAM must run the entire production system, including audit, logging, telemetry, and the dashboard. Use bounded buffers and history, batched writes, and lightweight built-in components. Do not introduce an external monitoring stack or unbounded individual latency samples.
- Format all code after changes, including HTML, CSS, and JavaScript. Use `cargo fmt` for Rust and Prettier for the dashboard HTML. Before finishing, run in `runtime/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
- Commit coherent parts of the work incrementally using concrete English commit messages. Do not include unrelated user changes, generated databases, logs, secrets, or build artifacts.
- Keep the user informed about meaningful findings and limitations in their preferred conversation language.
