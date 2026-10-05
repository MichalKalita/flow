# Flow runtime v Rustu

Jeden proces načte Flow soubor, zkontroluje deklarace, otevře SQLite a automaticky zaregistruje HTTP trasy, MQTT streamy a WebSocket odběry z `[http ...]`. Business operace zůstávají deklarativní. Permissions se vyhodnocují při čtení a před zápisem celé transakce; žádná route nemá vlastní kopii autorizačních pravidel.

## Spuštění

```sh
cd runtime
cargo run -- --check application.flow
export FLOW_ADMIN_TOKEN="$(openssl rand -hex 32)"
FLOW_JWT_SECRET='development-key-32-bytes-minimum-123456' FLOW_AUTOMATION_KEY='automation-key-long-enough-123456789' cargo run -- application.flow flow.sqlite 127.0.0.1:8080
```

Výchozí argumenty jsou `application.flow`, `flow.sqlite`, HTTP `127.0.0.1:8080` a MQTT `127.0.0.1:1883`. Čtvrtý argument mění MQTT adresu. WebSocket používá stejný listener jako HTTP. Server nepoužívá TLS. SQLite je přibalené do Rust závislosti, není potřeba databázový server. Program a databáze jsou svázané otiskem zdroje; změna schématu vyžaduje explicitní migraci nebo novou databázi. Seed se doplní pouze pro dosud neexistující ID, při restartu se data nepřepisují.

```sh
curl http://127.0.0.1:8080/api/products
curl http://127.0.0.1:8080/api/users
```

Produkty jsou veřejné. Anonymní čtení uživatelů vrátí `[]`; přihlášený uživatel vidí pouze sebe. JWT používá HS256, klíč z `FLOW_JWT_SECRET`, issuer `https://identity.example.com`, audience `application` a subject `idp:u1`, `idp:u2` nebo `idp:u3` pro seedované identity. Kontroluje se podpis, algoritmus, issuer, audience, expirace a případné `nbf`. Identita a role se načítají z databáze, nikoliv z klientských parametrů. Neplatný token nikdy nespadne do anonymního přístupu. API klíče používají `Authorization: ApiKey ...` a SHA-256 lookup podle deklarace v `[auth]`.

```sh
curl -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"userId":"u1","items":[{"productId":"p1","quantity":2}],"paymentMethod":"CARD"}' \
  http://127.0.0.1:8080/api/orders
```

`application.flow` je spustitelná HTTP varianta: produkty, uživatelé, objednávky, sklad, historie zařízení a uložení příkazů. Obsahuje deset HTTP operací a dva WebSocket odběry a eventový automat. Vytvoření objednávky seskupí duplicitní položky košíku, sníží sklad a vytvoří objednávku v jedné transakci. Payment URL je pouze lokální výpočet.

Původní širší deklarace z Elixir prototypu je zachována v [examples/application.flow](../examples/application.flow). Externí fronta ještě není přenesena. Volání externích pluginů se zatím odmítá. Nativní pluginy `Payment.createUrl`, `Image.resize` a `Files.put` už fungují podle deklarovaných kontraktů. `[publish]` uloží streamový záznam do SQLite, odkud se doručí aktuálně autorizovaným MQTT/WebSocket odběrům. Certifikátová autentizace se přes nezabezpečené HTTP nepřijímá. Elixir checkpoint je dostupný v historii Gitu.

## Fotografie a pluginy

`POST /api/products/{productId}/photo` přijímá JSON s `photo` jako base64 PNG/JPEG, volitelně `width` a `height`. Rozměry a byte limit se kontrolují skutečným dekódováním. `Image.resize` zachová poměr stran, `Files.put` uloží PNG BLOB spolu s entitami `File` a `Photo` v jedné SQLite transakci. Pluginy mají vlastní pozitivní `INVOKE` granty; právo na volání nepřeskočí práva k entitám ani polím argumentů.

Seedovaný katalogový administrátor má JWT subject `idp:catalog-admin`. Uživatel bez role `CATALOG_ADMIN` nemůže fotografii uložit. Výsledné `File.url` vede na `/api/files/{id}`; download znovu ověří aktuální `READ File` před čtením BLOBu. Neautorizovaný upload nezanechá metadata ani soubor.

## MQTT a WebSocket

MQTT listener implementuje MQTT 3.1.1 s čistou session: CONNECT, PUBLISH s QoS 0/1, SUBSCRIBE, UNSUBSCRIBE, PING a DISCONNECT. Odběry doručuje s QoS 0, podporuje `+` a `#`. Username je alias adaptéru z `[auth]`, password jeho credential. Seedované zařízení `mower1` používá alias `deviceKey` a vývojový klíč `device-key-32-bytes-minimum-123456789`. Může publikovat do `devices/mower1/status` a `devices/mower1/position` a číst své příkazy. Topic určuje brandovanou vazbu `device`; nesmí odporovat payloadu. ID a čas příjmu generuje runtime. Retence respektuje deklarovanou dobu a maximum zpráv pro konkrétní topic.

WebSocket upgrade `/ws` přijímá stejný `Authorization` header jako HTTP, ale ověřuje adaptéry povolené transportem `WebSocket`. Klient posílá například:

```json
{"id":"status","query":"LiveDeviceStatus","input":{"deviceId":"mower1"}}
```

Odpověď je `{"id":"status","data":{"online":true,"battery":12}}`. Ukončení odběru používá `{"id":"status","unsubscribe":true}`. Odběr doručí dostupnou historii a potom nové zprávy. Interní ID rozlišují i zprávy se stejným obsahem, do výstupu se přitom vybírají pouze pole výstupního typu.

Oba transporty čtou stejnou SQLite databázi každých 100 ms a při každém průchodu znovu ověřují credentials a aktuální permissions. Odebrání přístupu zastaví další doručování; WebSocket vrátí chybu pro zasažený odběr. Odběry mají omezený počet a paměť. Durable MQTT sessions, QoS 2, retain flag a Last Will zatím nejsou implementované.

## Eventové automaty

`[on DeviceStatus [actor [service "device-automation"]]]` spouští operaci po vytvoření streamového záznamu. `[when ...]` určuje čistou podmínku. Runtime používá samostatné credentials z důvěryhodné konfigurace `Config.event_credentials`, s klíčem `service:device-automation`; klient je nemůže dodat v payloadu. Service se znovu autentizuje a její aktuální permissions se vynucují pro čtení eventu i všechny změny automatu. Konfigurace a vazba na deklarované ID se ověří už při startu.

Hlavní aplikace obsahuje `LowBattery`: pod 20 % vytvoří `DeviceAlert`. Seedovaná service má vývojový klíč `automation-key-long-enough-123456789`, předaný CLI přes `FLOW_AUTOMATION_KEY`. Stream a navazující automat se potvrzují atomicky; odmítnutí permissions vrátí celou publikaci zpět. Eventové řetězení má limit 256 událostí na transakci.

## Jádro

- Obecný parser hranatých forem, JSON řetězce, limity vstupu, uzlů a zanoření.
- Brandovaná ID, nominální deklarace typů, konečné rozsahy čísel, scale a limity seznamů. Čísla používají přesnou aritmetiku s velkými integers; neukládají se jako float.
- Typované vstupy a výstupy, odmítnutí neznámých vstupních polí, defaulty, inverse a streamové reference.
- Pozitivní granty ze strany aktéra, více grantů jako OR, explicitní `[includes ...]`, vlastnictví, role, vztahy, pole a předchozí/navržený stav transakce. Bez grantu je přístup odepřen. Permissions mohou číst potřebné závislosti bez zpřístupnění jejich hodnot klientovi.
- Při autorizaci se identita a její vazby připnou k původnímu stavu. Nově napsaná role tedy nemůže autorizovat zápis ve stejné transakci. Zápisy se provedou až po autorizaci a ověření výstupu, jinak se transakce vrátí zpět.
- Projekce načítá jednotlivé potřebné sloupce a závislosti pravidel, nikdy `SELECT *`. Neprovádí optimalizaci na minimální počet SQL dotazů. SQLite požadavky se serializují.

Kontrola programu nyní ověřuje schéma, podporované konstrukce, arity, reference a cykly vazeb. Úplné statické odvozování všech typů výrazů ještě není implementováno; dynamické výsledky se kontrolují za běhu a jejich chyba nesmí způsobit částečný zápis. Negativní odkazy na další permissions se konzervativně odmítají. `Context` obsahuje pouze důvěryhodný aktuální čas. Nativní `USE` s pluginovým release není zatím přeneseno.

## Ověření

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Integrační testy pokrývají objednávky, rollback, vlastnictví, autentizaci, práva zařízení, přesná čísla, úzkou projekci a restart SQLite, streamovou retenci, přenos MQTT → WebSocket a odebrání práv u aktivních odběrů. Formát syntaxe popisuje [LANGUAGE.md](LANGUAGE.md).

## Built-in observability and administration

The production target is a VPS with one shared CPU and 2 GB RAM. HTTP, MQTT,
SQLite, the log writer, telemetry and the dashboard run in one process. No
Prometheus server, exporter, external dashboard or monitoring database is required.

Set `FLOW_ADMIN_TOKEN` to a secret of at least 32 bytes before starting the server.
The admin dashboard listens on `127.0.0.1:9090`; `FLOW_ADMIN_BIND` changes its
address, but the CLI requires a loopback IP. For a remote VPS, forward that port:

```sh
ssh -L 9090:127.0.0.1:9090 user@server
```

Open `http://127.0.0.1:9090` and enter the token. The token is stored in the
tab's `sessionStorage`, so a reload keeps the session and closing the tab or
using Disconnect admin clears it. The public application listener does not
expose admin APIs.
The Preact control plane has a left navigation menu with Overview, Traffic &
latency, Streaming, Runtime & storage, Log explorer, Mutation audit, HTTP console,
and Access tokens. Interactive SVG charts show request volume, errors, latency
percentiles, event throughput, connection counts, process memory and CPU. Select
15 minutes, one hour or six hours and control automatic refresh.

Endpoint inventory links directly to filtered logs and the HTTP console.
The console calls local declared endpoints with normal application credentials
and permissions; mutations change production data and are audited. The token
issuer signs HS256 application JWTs for existing identities, using configured
adapter issuer, audience and key. TTL is limited to 60 seconds through 24 hours.
Signing keys never leave the server and credentials stay in page memory.
Possession of the admin token allows issuing application credentials, so protect
it accordingly. Tokens still have the selected identity's normal permissions.

WebSocket latency measures the HTTP upgrade, not subscription lifetime.
Unmatched routes and overload rejections use fixed metric labels to bound
cardinality. Connection and subscription gauges release on session exit.
MQTT delivery counters measure successful socket writes, not broker acknowledgments.
Event and automation counters advance only after the enclosing transaction commits.

### Application logs

`FLOW_OBSERVABILITY_DIR` defaults to `data/observability`. `application.jsonl`
contains structured request logs (request ID, endpoint template, status and
elapsed time) and timed SQLite I/O and plugin spans with success/failure.
SQLite operation spans cover the entire operation, including authentication,
transaction execution and commit/rollback; nested read/apply spans give more
specific timings. Request IDs are returned in `X-Request-ID`. Logs exclude
request bodies, credentials and plugin arguments. The dashboard retains the
last 200 events in memory as a live cache; older entries remain in rotating disk files.
Log explorer filters by text, kind, level, endpoint, status and time, and pages
back through archives. Each query scans at most 4 MiB and returns at most 200 rows;
a continuation cursor resumes bounded scans. Archive rotation preserves cursors
until the underlying file is removed. Details show request IDs for correlation.
Audit filters entity, action and transport, with before/after snapshots decoded
to their logical JSON values, including records created by older versions.

A single background writer uses a bounded 512-event in-memory queue and a 64 KiB write
buffer, flushed every second. That queue is backpressure, not log retention: it holds
events waiting to hit disk. At 256 MiB, the log rotates through three archives
(approximately 1 GiB total, plus at most one event). A full queue drops disk log
entries instead of blocking request execution; the dashboard reports dropped
logs and storage errors. Logging failures do not undo successful business data.

### Metrics and resource usage

Telemetry stores counters, sums and fixed latency histograms in memory. It does
not keep individual request samples. p50/p95/p99 are upper bucket boundaries,
not exact percentiles; a percentile above 60 seconds is shown as `>60000`.
Counts and histograms are cumulative across restarts. The chart retains up to
360 one-minute aggregates per endpoint (six hours), with count, errors and mean
latency and histogram buckets. Charts aggregate histogram counts across endpoints
rather than averaging percentiles. HTTP 4xx and 5xx responses count as errors.
I/O and plugin spans have separate bounded latency metrics. Streaming counters
are cumulative, with six hours of minute history and 60 seconds of throughput
history. Active connection gauges reset on restart. A five-second sampler records
RSS and computes process CPU percentage from cumulative CPU time.

The writer atomically replaces `metrics.json` every 30 seconds and flushes it
on normal Ctrl-C shutdown. An abrupt termination can lose up to 30 seconds of
metrics and one second of buffered logs. Audit records remain transactional in
SQLite. A malformed metrics snapshot fails startup; back up or remove the
snapshot explicitly if resetting telemetry is intended. Do not share a telemetry
directory between multiple runtime processes.

The dashboard reports process RSS (and peak RSS on Linux) against the host RAM
limit: cgroup `memory.max` when the process is constrained, otherwise physical
memory (`/proc/meminfo` MemTotal on Linux, `sysctl hw.memsize` on macOS). It also
shows SQLite page-cache, schema and statement allocations, and estimated memory
used by metrics, history, the dashboard log cache and writer queue. Linux reads
`/proc/self/status`; macOS uses `ps` for development. Component estimates include
collection capacity and payloads, but exclude allocator bookkeeping, fragmentation,
thread stacks, active requests and other shared allocations; they do not sum to RSS.
SQLite disk usage includes the main database, WAL and SHM files. Logical page
size and telemetry/log disk usage are displayed separately.

The CLI caps Tokio at two async workers and two blocking workers, plus one log
writer. The public router admits at most 16 simultaneous requests and the admin
API at most four, rejecting additional requests with HTTP 503. Database work is
serialized. These limits bound concurrent request buffering, but large uploads
and application queries can still dominate memory. Validate the actual workload
on the target VPS; local tests are not a capacity guarantee.

### Transactional audit

`_flow_audit` stores every committed INSERT, UPDATE and DELETE on application
entity/stream tables and binary file records. SQLite triggers include seeds,
automations, stream retention and file cascades. Each entry contains timestamp,
transaction ID, operation, transport, authenticated actor, entity/ID, action and
before/after state. File contents are represented by their byte count. Entity
snapshots preserve raw SQL column representations, including exact decimals.
Automation entries identify their service actor and share the initiating
transaction ID. Queries and rolled-back writes create no committed-change audit
entries. Audit write failure aborts the application transaction.

Existing databases gain audit tables and triggers on their next successful open;
previous mutations cannot be reconstructed. The audit has no automatic deletion:
its retained history grows with mutation volume and is included in SQLite disk
usage. Operational logs and metrics never write to SQLite.

### Formatting and validation

```sh
cargo fmt
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cd ../admin-ui
bun install --frozen-lockfile
bun run format
bun run check
bun test src
bun run build
bunx playwright install chromium
bun run test:e2e
```

The frontend ships as small embedded HTML, CSS and JS assets with no CDN, chart
framework or frontend server. Bun, Tailwind and Playwright are development tools;
production needs only the Rust executable. Commit regenerated `src/admin-assets/`
alongside frontend source changes. E2E tests start the real runtime on isolated
ports and a temporary database; they never use the project's `.env` or production data.
