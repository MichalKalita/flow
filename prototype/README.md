# Order Lab

Lokální Elixir prototyp deklarativního objednávkového toku. Bez Leanu a replikace. SQLite uchovává produkty, uživatele, objednávky, přijaté požadavky, volání pluginů a emailovou frontu.

## Spuštění

Vyžaduje Elixir 1.18+ a kompatibilní Erlang/OTP.

```sh
cd prototype
mix local.hex --force
mix deps.get
./bin/start
```

Admin panel: <http://127.0.0.1:4000/admin>. Server naslouchá pouze na loopbacku. `PORT` mění port, `DATABASE_PATH` cestu databáze (výchozí `data/order_lab.sqlite3`). Při prvním startu se vytvoří schéma a demo data; další start je nemaže.

Na macOS při problému s CA nastavte `MIX_CACERTS_PATH=/etc/ssl/cert.pem` a `HEX_CACERTS_PATH=/etc/ssl/cert.pem`. Pokud projekt používá lokální `.mix` a `.hex`, nastavte stejné `MIX_HOME` a `HEX_HOME` i pro `mix deps.get`; `bin/start` je rozpozná automaticky.

## Co vyzkoušet

- Petra / CZ + produkt skladem: commit, platební odkaz, simulovaný email.
- Webkamera: nedostatek skladu, rollback bez volání pluginů.
- Nora / BR: platební plugin odmítne zemi; objednávka a sklad se rollbacknou, záznam pluginu zůstane.
- David / US + bankovní převod: nepodporovaná platební metoda.
- Simulovaná chyba emailu: objednávka zůstane potvrzená; tři pokusy s odstupem jedné sekundy, pak ponechání neúspěšné úlohy. V adminu ji lze opravit a zopakovat.

V přehledu otevřete detail požadavku: obsahuje vstupní JSON, HTTP výsledek a všechny vstupy/výstupy pluginů. Sekce Pluginy ukazuje i chyby a jednotlivé emailové pokusy. Nejde o skutečná externí HTTP volání: oba pluginy jsou lokální Elixir moduly. Žádný email ani platba se reálně neodesílá.

`./bin/demo` odešle čtyři skutečné demo požadavky (úspěch, sklad, platba, email). Díky pevným idempotency klíčům při opakování nevytvoří další objednávky. Neplatný JSON, nesprávný content type i příliš velké tělo se také zaznamenají; u velkého těla se uchová jen přijatý fragment s označením `truncated`.

## Deklarativní tok

Celá aplikace je v jediném `priv/workflows/application.flow`: typy s rozsahy, tabulky, typované MQTT zdroje, HTTP scénáře a WebSocket odběry. Obecný parser, typová kontrola a interpreter vykonávají dotazy, podmínky, iterace, CRUD, transakce a registrované pluginy. Objednávková logika je ve Flow. `FLOW_PATH` mění cestu jediného souboru; úprava vyžaduje restart. Syntaxi a hranice implementace popisuje [dokumentace jazyka](docs/LANGUAGE.md).

Jedna SQLite transakce vytvoří cenový snapshot, podmíněně odečte sklad, získá demo payment URL a vloží emailovou úlohu. Selhání před commitem vše vrátí. Jeden Store proces serializuje operace; to je záměrné zjednodušení lokálního prototypu. SQLite má zapnuté WAL, foreign keys a `synchronous=FULL`.

Email worker čte pouze potvrzené úlohy. Diagnostická historie se ukládá mimo objednávkovou transakci, takže rollback ji nemaže. Uchovává demo osobní údaje; admin nemá přihlášení a je určený jen pro lokální použití.

## HTTP API

```sh
curl -s http://127.0.0.1:4000/api/orders \
  -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: demo-order-1' \
  -d '{"user_id":"u1","items":[{"product_id":"p1","quantity":1}],"payment_method":"card"}'
```

`quantity` je celé číslo 1–100; vstup má 1–50 řádků a neznámá pole se odmítají. Opakované řádky stejného produktu se sloučí. Ceny jsou v haléřích, vždy ze serverové databáze. `Idempotency-Key` je volitelný: stejný klíč a stejné JSON hodnoty vrátí původní dokončený výsledek bez nových efektů; změněný vstup vrátí 409.

`GET /api/devices/:device_id` vrací poslední MQTT stav a polohy za posledních pět minut. MQTT 3.1.1 server naslouchá na `127.0.0.1:1883` (`MQTT_PORT` mění port). Přijímá striktně typovaný JSON na `devices/:device_id/status` a `devices/:device_id/position`; poskytuje QoS 0/1, subscriptions a retained zprávy. `ON MQTT DeviceStatus` spouští deklarovaný scénář nízké baterie; jeho požadavky a výsledky jsou vidět v adminu.

Admin → **Živá zařízení** otevírá WebSocket `/ws` a odebírá stav i polohu povolené sekačky. Nejprve pošlete `{"action":"authenticate","input":{"token":"demo-petra"}}`; Petra smí sledovat `mower1`, token `demo-david` smí sledovat `mower2`. Odběr používá typ zdroje a parametry: `{"action":"subscribe","source":"DeviceStatus","params":{"device_id":"mower1"},"latest":true}`. Každý připojený odběratel dostává vlastní kopii validovaných zpráv. Práva jsou deklarována ve stejném Flow souboru a ověřují se i před každou kopií zprávy. Odpojení ukončí odběry; historie se automaticky nedoplňuje.

`POST /api/products/:product_id/photo` přijímá ověřený PNG/JPEG jako JSON/base64, zmenší jej a uloží spolu se záznamem v jedné transakci.

`POST /api/language/check` validuje Flow zaslaný jako text bez spuštění.

`GET /api/admin`, `GET /api/requests/:id`, `GET /api/orders/:id`, `POST /api/email-jobs/:id/retry`, `GET /health`.

## Pouze E2E testy

Žádné unit testy. Testy spouštějí skutečnou Elixir aplikaci na portu 4100 s vlastní dočasnou SQLite databází, používají HTTP, skutečné MQTT TCP spojení na portu 18830 a ovládají admin přes skutečný Chrome. Instalovaný Google Chrome je potřeba; na jiném systému lze v `playwright.config.js` změnit `channel` a nainstalovat Chromium.

```sh
cd prototype/e2e
npm ci
npm test
```

Ověřují úspěšnou objednávku, cenový snapshot, rollback skladu a platby, validaci vstupů, idempotenci, souběžné objednávky, emailové retry a ruční opravu, admin panel, mobilní layout a persistenci po restartu. Další E2E scénáře ve stejném jediném souboru ověřují obecné CRUD, rozsahy typů a typované výpočty, podmíněné filtry, INNER/LEFT JOIN, podmíněné projekce, EXISTS, savepointy, MQTT QoS 1, DUP a retained subscriptions, WebSocket kopie pro více klientů, izolaci zařízení, odhlášení a živý admin panel; také skutečné dekódování a resize obrázků a transakční rollback souborů. Screenshoty a log serveru jsou v `e2e/test-results/`.

## Hranice prototypu

Neobsahuje HA, obnovu libovolného přerušeného workflow, skutečné platby, autentizaci ani hot updates. Po dokončené operaci data přežijí restart. Pád mezi commitem a uložením HTTP výsledku může zanechat požadavek `running`; další pokus se stejným klíčem vrátí `outcome_unknown` místo slepého zopakování. Diagnostická historie není atomická s obchodním commitem. Simulovaný email může po pádu mezi provedením a potvrzením úlohy běžet znovu; skutečný poskytovatel by potřeboval idempotenci nebo jiný explicitní kontrakt.
