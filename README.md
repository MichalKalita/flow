# Flow

Minimalistický deklarativní runtime v Rustu: Flow soubor → SQLite + HTTP/MQTT/WebSocket + centrální permissions.

```sh
cd runtime
cargo run -- --check application.flow
FLOW_JWT_SECRET='development-key-32-bytes-minimum-123456' cargo run -- application.flow flow.sqlite 127.0.0.1:8080
```

[Spuštění, funkce a omezení](runtime/README.md) · [Syntaxe](runtime/LANGUAGE.md) · [Spustitelná aplikace](runtime/application.flow) · [Plný návrhový příklad](examples/application.flow) · [Permission scénáře](examples/PERMISSIONS.md)

Elixir prototyp posloužil jako reference a byl odstraněn; poslední ověřená implementace je v commitu `4cda224`. Aktuální verze obsluhuje HTTP, MQTT a WebSocket nad SQLite. Původní architektonické dokumenty a příklady jiných jazyků jsou návrhy, nikoliv implementované API.
