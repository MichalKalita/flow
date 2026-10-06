# Flow

Business software on a server you own.

Flow is built for companies that want their applications to work and their data to stay safe. It should be usable by someone who knows nothing about server infrastructure.

The ambition is simple: put it on Alpine Linux and let it do its job for twenty years. Good defaults, a short setup guide, dependable backups, and a clear way to recover when something breaks. Running the business should take more attention than running the server.

Reliability comes first. Keeping data comes first. Peak performance comes after that.

## What runs today

Flow runs applications described in `.flow` files. Each project has its own data, users, and permissions. Applications can expose HTTP APIs, communicate with devices over MQTT, and send live updates over WebSocket.

Production uses one Rust executable with SQLite and the admin dashboard built in. The dashboard shows traffic, logs, and mutation history, and provides tools for managing projects. Application changes and their audit records are saved together.

The baseline production host is a small server with one shared CPU and 2 GB RAM. Larger machines can use more resources. Alpine Linux with OpenRC is the reference deployment.

## Where it is going

The finished system should handle ordinary server work without asking the user to become an expert:

- A setup guide that asks for a few necessary details and gives the rest sensible defaults.
- A main server and a backup server with the same software, all the same projects, and complete database copies. Automatic takeover keeps the system running while protecting saved data.
- Configurable backups that actually restore, using ordinary disks, network shares, or object storage.
- One log explorer that combines server logs and archived history, including logs stored remotely.
- Built-in login, application screens, integrations, and a full email server using the same application rules and event mechanism.

These are planned features, not a claim that the current release already does all of them. The [implementation plan](.todo/implementation-plan.md) records the order, dependencies, and decisions.

## Running on Alpine Linux

The production deployment runs the compiled executable as an unprivileged service. Application data stays outside the release directory, so replacing the program does not replace the data. The server does not need Rust or Bun installed to run a prepared release.

See the [Alpine deployment guide](deploy/alpine/README.md) for the existing ARM64/OpenRC setup, release updates, and recovery steps. Once the service is installed:

```sh
rc-service flow start
rc-service flow status
```

## Local development

From the repository root:

```sh
cp .env.example .env
openssl rand -hex 32
```

Put the generated value in `FLOW_ADMIN_TOKEN` in `.env`. For an unprivileged local HTTP listener, set `FLOW_HTTP_BIND='127.0.0.1:8080'`. Then run:

```sh
./start.sh
```

This builds the dashboard and starts the runtime, so the development machine needs Rust and Bun. Open the admin dashboard at `http://127.0.0.1:9090` and enter the admin token.

Hosted applications live in `projects/`; their databases are stored separately under `data/projects/`. A sample endpoint is `http://127.0.0.1:8080/demo/api/products`. The local `.env` and generated data are excluded from Git. The sample credentials are for development.

## More detail

- [Runtime behavior and current limitations](runtime/README.md)
- [Application language](runtime/LANGUAGE.md)
- [Hosted projects and permissions](projects/README.md)
- [Runnable application example](runtime/application.flow)
- [Traffic and capacity checks](k6/README.md)
- [Planned work](.todo/implementation-plan.md)

Older architecture documents and other-language examples are design references, not the implemented runtime API.
