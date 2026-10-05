# Flow

Minimalistický deklarativní runtime v Rustu: Flow soubor → SQLite + HTTP/MQTT/WebSocket + centrální permissions.

Run from the repository root using local configuration:

```sh
./start.sh
```

If `.env` is missing, copy `.env.example` to `.env` and fill in `FLOW_ADMIN_TOKEN`
with a random secret of at least 32 bytes (`openssl rand -hex 32`). The script
loads `.env`, creates the data directories, builds the admin UI with Bun, and
runs a release build with one cargo job. Relative configuration paths are
resolved from the repository root. Bun must be on `PATH`.
Application HTTP defaults to `127.0.0.1:8080`, MQTT to `127.0.0.1:1883`, and the
admin dashboard to `127.0.0.1:9090`. The local `.env` and generated `data/` are
excluded from Git. Credentials in the example are for development.

Direct Cargo invocation:

```sh
cd runtime
cargo run -- --check application.flow
export FLOW_ADMIN_TOKEN="$(openssl rand -hex 32)"
FLOW_JWT_SECRET='development-key-32-bytes-minimum-123456' FLOW_AUTOMATION_KEY='automation-key-long-enough-123456789' cargo run -- application.flow flow.sqlite 127.0.0.1:8080
```

[Spuštění, funkce a omezení](runtime/README.md) · [Syntaxe](runtime/LANGUAGE.md) · [Spustitelná aplikace](runtime/application.flow) · [Plný návrhový příklad](examples/application.flow) · [Permission scénáře](examples/PERMISSIONS.md)

Elixir prototyp posloužil jako reference a byl odstraněn; poslední ověřená implementace je v commitu `4cda224`. Aktuální verze obsluhuje HTTP, MQTT a WebSocket nad SQLite. Původní architektonické dokumenty a příklady jiných jazyků jsou návrhy, nikoliv implementované API.
