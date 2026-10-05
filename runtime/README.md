# Flow runtime v Rustu

Jeden proces načte Flow soubor, zkontroluje deklarace, otevře SQLite a automaticky zaregistruje HTTP trasy z `[http ...]`. Business operace zůstávají deklarativní. Permissions se vyhodnocují při čtení a před zápisem celé transakce; žádná route nemá vlastní kopii autorizačních pravidel.

## Spuštění

```sh
cd runtime
cargo run -- --check application.flow
FLOW_JWT_SECRET='development-key-32-bytes-minimum-123456' cargo run -- application.flow flow.sqlite 127.0.0.1:8080
```

Výchozí argumenty jsou `application.flow`, `flow.sqlite` a `127.0.0.1:8080`. Server používá HTTP. SQLite je přibalené do Rust závislosti, není potřeba databázový server. Program a databáze jsou svázané otiskem zdroje; změna schématu vyžaduje explicitní migraci nebo novou databázi. Seed se doplní pouze pro dosud neexistující ID, při restartu se data nepřepisují.

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

`application.flow` je spustitelná HTTP varianta: produkty, uživatelé, objednávky, sklad, historie zařízení a uložení příkazů. Obsahuje devět HTTP operací. Vytvoření objednávky seskupí duplicitní položky košíku, sníží sklad a vytvoří objednávku v jedné transakci. Payment URL je pouze lokální výpočet.

Původní širší deklarace z Elixir prototypu je zachována v [examples/application.flow](../examples/application.flow). Pluginy, obrázky, fronta, MQTT, WebSocket a eventové automaty nejsou součástí této minimální HTTP verze. Jejich operace se při kompilaci odmítnou; `[publish]` v HTTP variantě uloží streamový záznam do SQLite, neposílá MQTT zprávu. Certifikátová autentizace se přes nezabezpečené HTTP nepřijímá. Elixir checkpoint je dostupný v historii Gitu.

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

Integrační testy pokrývají objednávky, rollback, vlastnictví, autentizaci, práva zařízení, přesná čísla, úzkou projekci a restart SQLite. Formát syntaxe popisuje [LANGUAGE.md](LANGUAGE.md).
