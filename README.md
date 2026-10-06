# Flow

- Software for individuals and small and medium-sized businesses.
- Run it on your own Alpine Linux server.
- Use it without becoming a server expert.
- Reliability comes first.
- Keep your data safe.
- Good defaults avoid unnecessary overprovisioning.
- Designed for a $12/month server with 1 shared CPU and 2 GB RAM.
- Can also use a server with 32 CPUs and 128 GB RAM.
- Resource limits grow with available CPU and RAM.
- Linear performance scaling is a design goal.
- The long-term goal is twenty years of quiet operation.

## Available today

- One Rust executable.
- Built-in SQLite databases.
- Built-in admin dashboard.
- Separate data, users, and permissions for each project.
- Applications defined in `.flow` files.
- HTTP APIs, MQTT, and WebSocket.
- Application writes and audit records saved together.
- Logs, traffic charts, and project management.
- Alpine Linux deployment with OpenRC.

## Planned

- A short setup guide with good defaults.
- Automatic server keys and simple peer pairing.
- A main server and a standby with full copies of every project.
- Automatic takeover without losing acknowledged data.
- Configurable backups and working restoration.
- Storage on disks, network shares, and S3-compatible services.
- One log explorer for local and archived logs.
- Shared login, application screens, and external integrations.
- A complete built-in email server.
- One event mechanism for every event source.
- See the [implementation plan](.todo/implementation-plan.md).

## Run it

- Follow the [Alpine deployment guide](deploy/alpine/README.md).
- Start the installed service with `rc-service flow start`.
- Check it with `rc-service flow status`.
- Prepared releases need no Rust or Bun installation on the server.
- Local development uses `./start.sh` and requires Rust and Bun.
- Copy `.env.example` to `.env` before local development.
- Set `FLOW_ADMIN_TOKEN` to a value generated with `openssl rand -hex 32`.
- Use `FLOW_HTTP_BIND='127.0.0.1:8080'` for local development.
- Open the local dashboard at `http://127.0.0.1:9090`.
- Projects live in `projects/`.
- Databases live in `data/projects/`.
- Sample credentials are for development.

## Documentation

- [Runtime and current limitations](runtime/README.md)
- [Application language](runtime/LANGUAGE.md)
- [Hosted projects](projects/README.md)
- [Application example](runtime/application.flow)
- [Capacity checks](k6/README.md)
- Older architecture documents are design references.
