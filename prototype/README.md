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

Admin panel: <http://127.0.0.1:4000/admin>. Server naslouchá pouze na loopbacku. `PORT` mění port, `DATABASE_PATH` cestu databáze (výchozí `data/order_lab.sqlite3`). Při prvním startu se vytvoří schéma a data deklarovaná pomocí SEED v application.flow. Další start změněné záznamy nepřepíše a smazané neobnoví.

Na macOS při problému s CA nastavte `MIX_CACERTS_PATH=/etc/ssl/cert.pem` a `HEX_CACERTS_PATH=/etc/ssl/cert.pem`. Pokud projekt používá lokální `.mix` a `.hex`, nastavte stejné `MIX_HOME` a `HEX_HOME` i pro `mix deps.get`; `bin/start` je rozpozná automaticky.

## Co vyzkoušet

- Petra / CZ + produkt skladem: commit, platební odkaz, simulovaný email.
- Webkamera: nedostatek skladu, rollback bez volání pluginů.
- Nora / BR: platební plugin odmítne zemi; objednávka a sklad se rollbacknou, záznam pluginu zůstane.
- David / US + bankovní převod: nepodporovaná platební metoda.
- Simulovaná chyba emailu: objednávka zůstane potvrzená; tři pokusy s odstupem jedné sekundy, pak ponechání neúspěšné úlohy. V adminu ji lze opravit a zopakovat.

V přehledu otevřete detail požadavku: obsahuje vstupní JSON, HTTP výsledek a všechny vstupy/výstupy pluginů. Sekce Pluginy ukazuje i chyby a jednotlivé emailové pokusy. Výchozí režim je lokální simulace emailu a platby. Volitelné PAYMENT_PROVIDER_URL zapne skutečné HTTP volání pro synchronní Payment.create_url; poskytovatel musí podporovat stabilní Idempotency-Key a vracet typovaný výsledek. E2E používají samostatný lokální HTTP provider.

`./bin/demo` odešle čtyři skutečné demo požadavky (úspěch, sklad, platba, email). Díky pevným idempotency klíčům při opakování nevytvoří další objednávky. Neplatný JSON, nesprávný content type i příliš velké tělo se také zaznamenají; u velkého těla se uchová jen přijatý fragment s označením `truncated`.

## Deklarativní tok

Celá aplikace je v jediném `priv/workflows/application.flow`: typy s rozsahy, tabulky, typované MQTT zdroje, HTTP scénáře a WebSocket odběry. Obecný parser, typová kontrola a interpreter vykonávají dotazy, podmínky, iterace, CRUD, transakce a registrované pluginy. Objednávková logika je ve Flow. `FLOW_PATH` mění cestu jediného souboru; úprava vyžaduje restart. Syntaxi a hranice implementace popisuje [dokumentace jazyka](docs/LANGUAGE.md).

Jedna SQLite transakce vytvoří cenový snapshot, podmíněně odečte sklad, získá demo payment URL a vloží emailovou úlohu. Selhání před commitem vše vrátí. Jeden Store proces serializuje operace; to je záměrné zjednodušení lokálního prototypu. SQLite má zapnuté WAL, foreign keys a `synchronous=FULL`.

Email worker čte pouze potvrzené úlohy. Diagnostiky úspěšně potvrzeného prefixu se ukládají společně s jeho commitem. Při běžné chybě se zbývající diagnostiky uloží po rollbacku, takže chyba nezmaže historii pokusů. Uchovává demo osobní údaje; admin nemá přihlášení a je určený jen pro lokální použití.

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

Příkaz sekačce pošlete přes `POST /api/devices/mower1/commands` s JSON `{"token":"demo-petra","action":"stop"}`. Endpoint ve Flow ověří přístup a vloží typovaný `PUBLISH DeviceCommand(:device_id)` do transakční fronty. HTTP 202 potvrzuje přijetí; worker jej odešle na `devices/mower1/command`. Admin → Živá zařízení obsahuje tlačítka start/stop a historii odchozích MQTT zpráv. Zprávy přežijí restart a rollback je neodešle. Při pádu po přenosu může přijít opakovaná kopie; zařízení může deduplikovat podle command_id. Nekompatibilní čekající zprávu lze po opravě kontraktu zopakovat v adminu.

`POST /api/language/check` validuje Flow zaslaný jako text bez spuštění.

`GET /api/admin`, `GET /api/requests/:id`, `GET /api/orders/:id`, `POST /api/email-jobs/:id/retry`, `POST /api/requests/:id/resume`, `GET /health`.

## Pouze E2E testy

Žádné unit testy. Testy spouštějí skutečnou Elixir aplikaci na portu 4100 s vlastní dočasnou SQLite databází, používají HTTP, skutečné MQTT TCP spojení na portu 18830 a ovládají admin přes skutečný Chrome. Instalovaný Google Chrome je potřeba; na jiném systému lze v `playwright.config.js` změnit `channel` a nainstalovat Chromium.

```sh
cd prototype/e2e
npm ci
npm test
```

Ověřují úspěšnou objednávku, cenový snapshot, rollback skladu a platby, validaci vstupů, idempotenci, souběžné objednávky, emailové retry a ruční opravu, admin panel, mobilní layout a persistenci po restartu. Další E2E scénáře ve stejném jediném souboru ověřují obecné CRUD, rozsahy typů a typované výpočty, podmíněné filtry, INNER/LEFT JOIN, podmíněné projekce, EXISTS, savepointy, MQTT QoS 1, DUP a retained subscriptions, WebSocket kopie pro více klientů, izolaci zařízení, odhlášení a živý admin panel; také skutečné dekódování a resize obrázků a transakční rollback souborů. Screenshoty a log serveru jsou v `e2e/test-results/`.

## Hranice prototypu

Neobsahuje HA, skutečné platby, přihlášení do HTTP adminu ani hot updates. WebSocket používá deklarativně ověřené lokální bearer tokeny. Interpreter má trvalé checkpointy a po restartu obnoví bezpečné pokračování za posledním COMMIT. Po pádu mezi commitem objednávky a uložením odpovědi se tak neopakuje objednávka, sklad ani platba; klient se stejným klíčem získá obnovený výsledek.

Externí CALL bez bezpečného retry kontraktu nelze slepě zopakovat. Pokud takový CALL zůstává v pokračování, požadavek se automaticky neobnoví a stejný klíč vrátí `outcome_unknown`. Změněný zdroj rovněž nesmí převzít checkpoint původního programu. Diagnostiky nepotvrzeného externího pokusu mohou při tvrdém pádu zůstat neuložené. Simulovaný email může po pádu mezi provedením a potvrzením úlohy běžet znovu; skutečný poskytovatel by potřeboval idempotenci nebo jiný explicitní kontrakt. Podrobnosti popisuje [dokumentace obnovy](docs/LANGUAGE.md#obnova-vykonávání-po-pádu).


Payment CALL má trvalou evidenci `<DATABASE_PATH>.effects`, která přežije rollback objednávky. Dokončený výsledek se znovu použije; nejasný pokus se opakuje se stejným klíčem a vstupem. Bez podpory poskytovatele nejde o garanci právě jednoho externího účinku. Admin → Pluginy ukazuje tento deník a umožňuje obnovit pending požadavek. Obnova hlídá změny zdrojových dat a případný konflikt, aby nepřepsala novější sklad. Hlavní databáze i deník musí zůstat pohromadě pro obnovu; asynchronní email a kompenzace mají nadále popsaná omezení.
