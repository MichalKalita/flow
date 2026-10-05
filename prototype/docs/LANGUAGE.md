# OrderLab Flow — jazyk scénářů

Flow deklaruje HTTP rozhraní, typy dat, dotazy, pravidla a účinky. Implementace databáze a pluginů patří hostitelskému Elixiru. Scénář nemůže volat libovolné Elixir funkce, spouštět SQL ani definovat vlastní funkce.

## Stav implementace

Celá aplikace je v jediném `priv/workflows/application.flow`: typy, tabulky, počáteční data, MQTT zdroje, HTTP scénáře a autorizované WebSocket odběry. Runtime jej při startu parsuje a typově kontroluje. `FLOW_PATH` nastaví cestu jiného jediného souboru. Změna vyžaduje restart. Lexer, parser, typová kontrola, validátor hodnot a interpreter jsou v `lib/order_lab/language/`. Objednávky i další CRUD scénáře používají stejný interpreter. `POST /api/language/check` přijímá zdroj jako text a vrací diagnostiku bez spuštění.

## Typy a vstupy

```text
TYPE Rating = Number WHERE value BETWEEN 1.0 AND 5.0
TYPE Quantity = Int WHERE value BETWEEN 1 AND 100
TYPE Item = {product_id: String, quantity: Quantity}
TYPE Cart = List<Item> WHERE length(value) BETWEEN 1 AND 50

FILTER ProductFilter
    rating Rating?
    min_price Int?
    in_stock Bool

HTTP POST /api/search
INPUT filter ProductFilter
INPUT limit Int = 20
```

Typy jsou `String`, `Int`, `Number`, `Float`, `Bool`, `JSON`, `List<T>`, záznam `{pole: Typ}` a volitelný `Typ?`. `Float` nyní přijímá libovolné JSON číslo stejně jako `Number`. `TYPE` může odkazovat na jiný pojmenovaný typ. Cyklické a neznámé typy se odmítají. Refinement používá proměnnou `value`; predikát musí vrátit `true`. Záznamy odmítají neznámá pole a vyžadují všechna nepovinná pole. Chybějící volitelné pole a `null` jsou povolené. Známé konstanty (i uvnitř záznamu s jinými dynamickými poli) se kontrolují už při kompilaci. Vnější data a výpočty se kontrolují při validaci hodnot; zatím nejde o matematický důkaz správnosti celého programu.

`INPUT` deklaruje JSON vstup nebo výchozí výraz. `:filter` a `filter` odkazují na stejnou proměnnou. Výchozí hodnoty mohou používat dříve deklarované vstupy. Není povolena implicitní konverze řetězce na číslo či boolean.

## Výrazy a dotazy

```text
products = FROM Products AS p
    WHERE p.price_cents >= 100
    WHEN :filter.min_price IS PRESENT
        WHERE p.price_cents >= :filter.min_price
    WHEN :filter.in_stock
        WHERE p.stock > 0
    ORDER BY p.price_cents ASC
    ORDER BY p.id ASC
    LIMIT :limit
    SELECT {id: p.id, name: p.name, price: p.price_cents}

available = EXISTS p IN Products WHERE p.stock > 0
valid = ALL item IN :items SATISFY item.quantity > 0
total = SUM item IN :items OF item.price_cents * item.quantity
RETURN {products: products, available: available, total: total}
```

`FROM` pracuje s kolekcí z registrovaného zdroje nebo s výrazem vracejícím seznam. Alias je povinný. `WHERE` se skládají konjunkcí; `WHEN` přidává filtr jen při splnění podmínky. Nejprve se provádějí JOIN, potom se filtruje a řadí a omezuje počet, nakonec promítá `SELECT`. Více klíčů řazení se zapisuje opakováním `ORDER BY`. `ONE FROM` vrací jeden záznam nebo `null`; více výsledků je chyba. `FIRST FROM` a `LAST FROM` vrací první/poslední výsledek nebo `null`.

Operátory: `OR`, `AND`, `NOT`, `=`, `==`, `!=`, `<`, `<=`, `>`, `>=`, `IN`, `BETWEEN … AND …`, `IS PRESENT`, `IS NOT PRESENT`, `+`, `-`, `*`, `/`, `%`. Logické operátory vyžadují boolean a vyhodnocují se zkráceně. Porovnání pořadí vyžaduje dvě čísla nebo dva řetězce. Dělení nulou je chyba. Kvantifikátor v kombinaci s vnějším logickým výrazem uzavři do závorek.

Registrované vestavěné operace: `count`, `length`, `contains`, `lower`, `upper`, `starts_with`, `concat(List<String>)`, `merge(record, record)`, `coalesce(value, default)`, `min`, `max`, `round`, `distinct`, `group_sum(rows, "key", "amount")`, `uuid("prefix")`, `now()`. `group_sum` seskupuje podle pole a sčítá druhé číselné pole; zachovává pořadí prvního výskytu skupin.

## Iterace, podmínky a pravidla

```text
snapshots = FOR EACH item IN :items
    product = ONE FROM Products AS p WHERE p.id = item.product_id
    REQUIRE product IS PRESENT ELSE 404 product_not_found "Produkt neexistuje"
    RETURN {id: product.id, price: product.price_cents, quantity: item.quantity}

WHEN :send_email
    CALL Email.send_confirmation WITH {
        order_id: order.id, to: :email, subject: "Potvrzení", payment_url: payment.url,
        total_cents: order.total_cents, items: snapshots, simulate_failure: false
    }
ELSE
    REQUIRE :email IS PRESENT ELSE 422 missing_email "Chybí email"
```

Iterace s vazbou sbírá hodnoty `RETURN` do seznamu; každá iterace musí vrátit hodnotu. Iterace bez vazby slouží k účinkům. Proměnné v iteraci a podmínkové větvi jsou lokální. `RETURN` ve větvi ukončí aktuální blok/scénář; uvnitř iterace ukončí aktuální položku. `REQUIRE` při nepravdě vyvolá deklarovanou chybu. Samostatný `FAIL 409 code "zpráva"` vyvolá chybu bez podmínky. Bez HTTP statusu je výchozí hodnota 422.

## Transakce, tabulky a pluginy

```text
TABLE Notes = {id: String, text: String}

HTTP POST /api/notes
INPUT text String
TRANSACTION
    note = INSERT Notes WITH {id: uuid("note"), text: :text}
    UPDATE Notes AS n WHERE n.id = note.id SET {text: upper(n.text)}
    DELETE FROM Notes AS n WHERE n.text = ""
    payment = CALL Payment.create_url WITH {
        order_id: note.id, amount_cents: 100, currency: "CZK",
        country: "CZ", method: "card"
    }
    job = QUEUE Email.send_confirmation WITH {
        order_id: note.id, to: "demo@example.test", payment_url: payment.url,
        subject: "Potvrzení", total_cents: 100, items: [], simulate_failure: false
    } POLICY 3 ATTEMPTS DELAY 1000 ms RETAIN
    COMMIT
RESPONSE 201 WITH {note: note, payment: payment, job: job}
```

Zápisy, `QUEUE` a `PUBLISH` vyžadují aktivní transakci. `COMMIT` musí být právě jednou, mimo iteraci; po něm se ve stejném transakčním bloku nesmí provádět další účinky. Chyba před commitem vrací databázi do původního stavu. Vnořená transakce není podporována. `INSERT` vrací vložený záznam, `UPDATE` seznam změněných záznamů a `DELETE` seznam výsledků hostitelského adaptéru. Změny jsou hodnoty a nevymění automaticky dříve navázaný snapshot.

`CALL` provádí registrovanou externí operaci. Databázový rollback sám nevrací externí účinky; host musí respektovat kontrakt operace a případně zajistit kompenzaci. `QUEUE` deklaruje odložené volání, počet pokusů, prodlevu a osud po posledním neúspěchu (`RETAIN`/`DELETE`). Trvalá fronta `flow_jobs` se zapisuje ve stejné transakci jako obchodní změny. Worker provádí registrované operace a zaznamenává pokusy. Kontrakty jsou v `Native.operations/0`; vstupy a úspěšné výstupy se validují.

```text
TRY
    payment = CALL Payment.create_url WITH :payment_request
CATCH error
    FAIL 422 payment_unavailable error.message
```

`CATCH` zachytí obchodní chybu operace nebo `REQUIRE`/`FAIL`, nikoliv chybu jazyka. V transakci používá savepoint a vrací zápisy uvnitř `TRY`; nezruší předchozí změny. Chyba má pole `code`, `message`, `status`, `details`. Vazby vzniklé v úspěšném `TRY` jsou dostupné dál; vazby handleru jsou lokální. Commit uvnitř `TRY` typová kontrola odmítá. Vazby ze success větve lze použít dál pouze tehdy, když handler vždy ukončí scénář; jinak by na chybové větvi chyběly.

## Zápis zdroje

Řetězce používají dvojité uvozovky a JSON escapování. `#` zahajuje komentář. Bloky používají mezery; tabulátory jsou chyba. Výrazy v závorkách, seznamech a záznamech mohou pokračovat na dalších řádcích. Vlastní funkce a libovolné nativní volání nejsou součástí jazyka. Runtime nyní používá pořadí deklarací jako konkrétní prováděcí plán; optimalizace musí zachovat datové závislosti a význam účinků.

## MQTT zdroje

```text
TYPE DeviceID = String WHERE length(value) BETWEEN 1 AND 64
TYPE Latitude = Number WHERE value BETWEEN -90 AND 90
TYPE Longitude = Number WHERE value BETWEEN -180 AND 180
MQTT DeviceStatus
    TOPIC "devices/{device_id}/status"
    PARAM device_id DeviceID
    PAYLOAD {online: Bool, battery: Int}
    HISTORY 24 HOURS
MQTT DevicePosition
    TOPIC "devices/{device_id}/position"
    PARAM device_id DeviceID
    PAYLOAD {latitude: Latitude, longitude: Longitude}
    HISTORY 5 MINUTES

HTTP GET /api/devices/:device_id
INPUT device_id DeviceID
status = LAST FROM DeviceStatus(:device_id) AS s
positions = FROM DevicePosition(:device_id) AS p
    WHERE p.received_at >= ago(5, "minutes")
    ORDER BY p.received_at ASC
RETURN {status: status, positions: positions}
```

Scénář používá jméno typovaného zdroje a parametry. `PARAM` odpovídá `{parametru}` v topicu a obsazuje celý segment. Překrývající se šablony se odmítají. Payload je striktní záznam; chybné typy, neznámá pole a nesplněné refinements se odmítnou před uložením. Zdroj přidává rezervované pole `received_at`: UTC ISO 8601 čas přijetí.

Zdroj vrací historii a případně poslední starší hodnotu. `HISTORY` určuje uchovávané okno, ale poslední zpráva a retained zprávy se uchovávají i mimo něj. Přesné okno proto vyžaduje explicitní `WHERE`. `LAST FROM` vrátí poslední známý stav. Dotazy zahrnují pouze zprávy přijaté do `request.time`; Store serializuje celý scénář. `ago(5, "minutes")` počítá od času vyhodnocení a podporuje `ms`, `seconds`, `minutes`, `hours`.

MQTT 3.1.1 server na `127.0.0.1:1883` (`MQTT_PORT`) podporuje CONNECT, PUBLISH QoS 0/1, SUBSCRIBE (doručení QoS 0), UNSUBSCRIBE, retained zprávy, PING a DISCONNECT. QoS 1 potvrzuje až po validaci a uložení. Posledních 100 QoS 1 identifikátorů deduplikuje při DUP v rámci spojení. Neplatný payload nebo nedeklarovaný topic ukončí spojení bez PUBACK. Prázdný retained payload ruší retained stav. SQLite uchová historii i retained stav přes restart. Broker zatím nemá persistent sessions, QoS 2, will messages, TLS ani autentizaci. MQTT slouží také jako spouštěč `ON MQTT`; odchozí typovaný PUBLISH používá transakční frontu popsanou níže.

## Tabulky, HTTP a ověření

`Products`, `Users`, `Orders` mají registrované SQLite adaptéry. Jiná `TABLE` se ukládá jako typované JSON záznamy v `flow_records`; nový scénář ani tabulka nevyžadují změnu Elixiru. Tabulka musí mít `id: String`; `UPDATE` nesmí měnit identifikátor. Vložené a změněné záznamy se validují včetně refinements. `RETAIN` ponechá neúspěšnou úlohu, `DELETE` ji po posledním neúspěšném pokusu smaže; diagnostické záznamy pokusů zůstávají.

HTTP cesty mají parametry `:name` deklarované pomocí `INPUT`. GET a DELETE čtou query string; ostatní metody JSON objekt. Čísla a boolean v query stringu se dekódují jako JSON; záznamový filtr se předává jako JSON v jedné query hodnotě. Cestový parametr má přednost. Admin, diagnostika, fake platební stránka a retry jsou hostitelská rozhraní. Všechny HTTP scénáře používají stejnou obsluhu idempotence: `Idempotency-Key` se vztahuje k metodě a konkrétní cestě. Stejný vstup vrací uložený výsledek bez opakování účinků; jiný vstup vrací 409. Přijaté i odmítnuté požadavky se zaznamenávají pod skutečnou metodou a cestou.

`cd prototype/e2e && npm test` spouští pouze E2E testy. Ty připojí další CRUD scénáře do stejného jediného souboru, nastartují skutečný Elixir proces a provádějí HTTP, MQTT přes TCP i ovládání adminu v Chrome. Ověřují rozsahy typů, podmíněné filtry, EXISTS, savepoint rollback, MQTT QoS 1, DUP, retained subscriptions, izolaci zařízení, odmítnutí neplatných payloadů a persistenci po restartu. Objednávkové testy pokrývají sklad, platby, frontu pluginů, idempotenci a souběh.


## Scénáře spouštěné MQTT

```text
TABLE DeviceAlerts = {id: String, device_id: DeviceID, kind: String, created_at: String}
ON MQTT DeviceStatus
    WHEN :message.battery < 20
        TRANSACTION
            INSERT DeviceAlerts WITH {id: uuid("alert"), device_id: :device_id, kind: "low_battery", created_at: request.time}
            COMMIT
    RETURN {accepted: true}
```

`ON MQTT JménoZdroje` dostává automaticky parametry topicu a `message` s přesným typem PAYLOAD; tyto INPUT se znovu nedeklarují. Tělo používá stejné příkazy jako HTTP. Zpráva se nejprve validuje a uloží, pak spustí scénář. Transportní PUBACK potvrzuje přijetí zprávy, nikoliv úspěch obchodní operace. Chyba scénáře vrátí jeho transakční změny; přijatá zpráva a diagnostika zůstanou. Admin ji eviduje s metodou `MQTT`, skutečným topicem a vstupní zprávou. Vyhrazená metadata jsou `request` a `message`; MQTT parametry mají zatím řetězcový základ typu. Jeden zdroj může mít jediný ON handler. Handler se provádí synchronně ve Store; automatické opakování selhaného handleru zatím není součástí prototypu.


## Typované vazby a kontrola konstrukcí

```text
TYPE Rating = Number WHERE value BETWEEN 1.0 AND 5.0
HTTP POST /ratings
INPUT rating Rating
INPUT delta Number = 0
value: Rating = :rating + :delta
RETURN {rating: value}
```

Zápis `jméno: Typ = …` platí pro výraz, INSERT, CALL a iteraci s vazbou. Kompilátor kontroluje strukturální kompatibilitu. Známé konstanty ověřuje včetně refinements: `value: Rating = 7` nebo `INPUT rating Rating = 7` odmítne před spuštěním. Stejně kontroluje konstantní pole v INSERT a SET i při dynamických ostatních polích. Dynamický výsledek musí po vyhodnocení projít validátorem; nesplnění vyvolá obchodní chybu `422 type_constraint_failed`, kterou může zachytit TRY/CATCH. Uvnitř transakce vrátí její změny. Typovaný výsledek neslibuje, že libovolná aritmetika zachová původní rozsah; každý výpočet se znovu ověřuje.

## JOIN a podmíněná propojení

```text
rows = FROM Products AS p
    WHEN :show_reviews
        LEFT JOIN Reviews AS r ON r.product_id = p.id
    LEFT JOIN Users AS u ON r IS PRESENT AND u.id = r.user_id
    ORDER BY p.id ASC
    SELECT {product: p, review: r, author: u}
```

`JOIN` a `INNER JOIN` znamenají vnitřní propojení; `LEFT JOIN` zachová levý řádek a při chybějící shodě naváže alias na `null`. Více shod rozmnoží řádky. JOIN se vyhodnocují v pořadí datových závislostí před WHERE, ORDER BY, LIMIT a SELECT. Výchozí výsledek bez SELECT je původní levý záznam. Všechny aliasy musí být v dotazu jedinečné.

JOIN může číst tabulku, typovaný MQTT zdroj i kolekci závislou na předchozím aliasu: `JOIN o.items AS item ON true`. Dotaz může mít více JOIN a zapisovat je na jednom řádku nebo odsazenými klauzulemi. `WHEN` nad JOIN řídí jeho zahrnutí podle vstupního podmínkového výrazu; při false má alias hodnotu null a řádky se nerozmnoží. Alias z LEFT či podmíněného JOIN má volitelný typ. Přístup k jeho polím vyžaduje `IS PRESENT`, například ve zkráceném AND v ON nebo ve WHERE; tento filtr zpřesní typ pro následující klauzule a SELECT. Runtime zatím provádí propojení nad kolekcemi v paměti, nikoliv SQL optimalizátorem.


## Podmíněné hodnoty a výsledky větví

```text
SELECT {author_name: IF u IS PRESENT THEN u.name ELSE null}

values = FOR EACH number IN :numbers
    WHEN number > 0
        half = number / 2
        RETURN {value: half}
    ELSE
        original = number
        RETURN {value: original}
```

`IF podmínka THEN výraz ELSE výraz` je čistý výraz. Vyhodnotí pouze vybranou větev; na obou stranách musí být kompatibilní typy. Kombinace čísla a null vytvoří volitelný číselný typ, kombinace Int a Number vytvoří Number. Totéž platí pro číselné položky seznamu a návratové hodnoty iterace. Každá větev má vlastní vazby a typové zúžení: `IS PRESENT` umožní přístup k polím ve THEN; `IS NOT PRESENT` jej umožní ve ELSE. Také odvozování výsledku `FOR EACH` respektuje lokální vazby jednotlivých větví.

Refinement predikáty musí být deterministické. `uuid`, `now` a `ago` se v TYPE WHERE odmítají. Při čtení uložených záznamů a MQTT historie se znovu ověřuje aktuální kontrakt; stará data se po změně schématu nesmí vydávat za nový typ bez validace.

## WebSocket odběr typovaných zpráv

WebSocket endpoint se deklaruje ve stejném aplikačním souboru jako HTTP a MQTT:

```text
WEBSOCKET /ws
    INPUT token String
    AUTHORIZE EXISTS access IN DeviceAccess WHERE access.token = :token AND EXISTS user IN Users WHERE user.id = access.user_id
    SOURCE DeviceStatus WHERE EXISTS access IN DeviceAccess WHERE access.token = :token AND access.device_id = :device_id
    SOURCE DevicePosition WHERE EXISTS access IN DeviceAccess WHERE access.token = :token AND access.device_id = :device_id
```

`INPUT` deklaruje přesně typovaná přihlašovací data. `AUTHORIZE` je Bool predikát pro ověření přihlášení; `SOURCE … WHERE` je Bool predikát pro přístup ke konkrétním parametrům zdroje. Tyto predikáty mohou číst tabulky pomocí EXISTS a dalších dotazů, ale nesmějí používat nedeterministické uuid/now/ago. Kompilátor kontroluje jejich typy i názvy polí.

`SOURCE` je seznam povolených, již deklarovaných MQTT zdrojů. Kompilátor odmítne neznámý zdroj, duplicitní endpoint či kolizi se stejnou HTTP GET cestou. Zatím jsou podporovány statické WebSocket cesty. Klient nejprve pošle přihlašovací zprávu; token se nedává do URL:

```json
{"action":"authenticate","input":{"token":"demo-petra"}}
```

Úspěch vrátí `{"type":"authenticated"}`. Chybějící či neplatná přihlašovací data nepovolí žádný odběr. Přihlášené spojení nemůže měnit identitu; pro jiný token se musí připojit znovu. Endpoint bez INPUT a s implicitním AUTHORIZE true může být veřejný.

Následně klient posílá JSON příkazy:

```json
{"action":"subscribe","source":"DeviceStatus","params":{"device_id":"mower1"},"latest":true}
```

Server ověří parametry podle typu zdroje a oba autorizační predikáty, potvrdí odběr zprávou `type: "subscribed"` a při `latest: true` pošle i poslední uloženou zprávu, pokud existuje. Každá další přijatá a validovaná MQTT zpráva pro toto zařízení vytvoří kopii pro každého odběratele:

```json
{"type":"message","source":"DeviceStatus","params":{"device_id":"mower1"},"payload":{"online":true,"battery":88},"received_at":"2026-10-05T12:00:00.000000Z"}
```

Odhlášení používá stejný zdroj a parametry s `action: "unsubscribe"`; odpověď má `type: "unsubscribed"`. Opakovaný subscribe nevytváří duplicitní odběr. Neplatné příkazy vrátí `type: "error"` a spojení zůstává použitelné. Nelze odebírat libovolné topic stringy ani wildcardy. Jedno spojení má nejvýše 16 odběrů a příkaz nejvýše 8 KiB. Odpojení klienta odběry automaticky odstraní; pomalý klient s přeplněnou procesní schránkou se odpojí kódem 1013.

Kopie se odesílají po uložení MQTT zprávy, nezávisle na následném úspěchu jejího ON MQTT scénáře. Neplatný payload se neuloží ani nerozešle. Zachycená retransmise QoS1 DUP se znovu nerozešle. Odběr je živý: nemá potvrzování jednotlivých kopií ani trvalou frontu pro odpojeného klienta; `latest` poskytuje poslední známou hodnotu, ne obnovu celé historie. Autorizace probíhá před subscribe, před vrácením latest i před každou živou kopií. Odebrání přístupu vrátí `forbidden` nebo `unauthorized` a odstraní příslušný aktivní odběr, aniž odešle payload. Chyba čtení pravidel přístup nepovolí. Unsubscribe zůstává možný i po odebrání práv. Admin → Živá zařízení umožňuje zadat token a odebírat stav a polohu povolené sekačky.

Demo používá v DeviceAccess tokeny `demo-petra` pro uživatele u1 a zařízení mower1 a `demo-david` pro u2 a mower2. Jde o lokální bearer tokeny uložené v tabulce, nikoliv integraci s OAuth/JWT poskytovatelem. Známé demo tokeny v tomto příkladu jsou určené pro lokální zkoušení. Admin a ostatní HTTP endpointy nejsou tímto pravidlem chráněny.

## Ověřené soubory a obrázky

`File` a `Image` jsou nativní typy s ověřeným obsahem, nikoliv libovolné JSON záznamy. HTTP vstup obsahuje base64 v `data` a volitelný `name`. Runtime dopočítá `media_type`, `byte_size` a `sha256`; Image navíc `format`, `width` a `height`. Pokud klient dodá metadata, musí přesně odpovídat obsahu. Image vyžaduje skutečné dekódování PNG nebo JPEG přes libvips; PDF, poškozený obrázek a falešná přípona tento typ nesplní.

```text
TYPE ProductPhoto = Image WHERE value.byte_size <= 10485760 AND value.width <= 8000 AND value.height <= 8000
HTTP POST /photos
INPUT photo ProductPhoto
TRANSACTION
    resized: ProductPhoto = CALL Image.resize WITH {image: :photo, width: 800, height: 600}
    stored = CALL Files.put WITH {file: resized}
    COMMIT
RETURN stored
```

Image je kompatibilní s File. File se musí ověřit přes `CALL Image.decode WITH {file: :file}`, než může být vstupem Image.resize. Resize zachová poměr stran a původní PNG/JPEG formát, vejde se do zadaného obdélníku a nezvětšuje malé obrázky. Šířka a výška cíle mají rozsah 1–8192. Vstupní soubory mají limit 10 MiB; dekódovaný obrázek nejvýše 8192 × 8192 a současně 20 milionů pixelů. HTTP používá JSON/base64, multipart zatím nepodporuje.

`Files.put` vrací `{id, url, name, media_type, byte_size, sha256}`. `Files.read WITH {id: ...}` vrací File a `Files.delete WITH {id: ...}` vrací `{id, deleted}`. Obsah je uložen v SQLite; put/delete musí být uvnitř TRANSACTION a vracejí se společně s aplikačními záznamy při rollbacku. URL `/files/:id` vrací skutečné bajty souboru. Tyto operace ani Image.resize/decode nelze vložit do QUEUE. Příklad produktu s fotografií je součástí `application.flow`.


## Počáteční záznamy

```text
TABLE DeviceAccess = {id: String, user_id: UserID, token: String, device_id: DeviceID}
SEED DeviceAccess WITH [{id: "petra-mower1", user_id: "u1", token: "demo-petra", device_id: "mower1"}]
```

SEED je top-level deklarace deterministického seznamu záznamů, typovaného podle tabulky. Kompilátor odmítne neznámou tabulku, chybný záznam či duplicitní id i napříč více SEED stejné tabulky. Runtime při startu vloží dosud neinicializované klíče v jedné SQLite transakci. Již existující záznam nepřepíše. Trvalá evidence klíčů zabrání obnovení později smazaného záznamu při restartu; odebrané oprávnění se tak samo nevrátí. Změna hodnoty v SEED nemění existující data; jejich úprava patří do explicitního UPDATE/DELETE scénáře. Stejná pravidla platí pro Products a Users: demo data jsou deklarována v application.flow, runtime je neobsahuje. Při přechodu ze starší databáze se existující klíče pouze označí jako inicializované a jejich hodnoty se zachovají. Před tímto prvním označením nelze rozlišit dosud nevytvořený záznam od záznamu smazaného starší verzí bez evidence SEED. Aplikace bez SEED startuje bez demo dat.


## Obnova vykonávání po pádu

Interpreter ukládá do SQLite trvalé pokračování: validované vstupy, proměnné, stav větví a transakcí a dosud nevykonanou část scénáře. První checkpoint vzniká před tělem scénáře. Každý COMMIT zapisuje nový checkpoint a dosavadní pluginové diagnostiky do stejné transakce jako obchodní změny a frontu. Pokud proces spadne po commitu, pokračuje za touto hranicí; potvrzené INSERT, odečet skladu, souborový zápis ani QUEUE se neopakují. Platba provedená před takto potvrzeným commitem se také znovu nevolá.

Při startu Store vyhledá rozpracované požadavky s checkpointem a obnoví ty, které lze bezpečně provést. Platí to i bez Idempotency-Key; klíč pouze umožňuje klientovi znovu získat výsledek. Původní `request.id`, `request.time`, hodnoty proměnných a zvolená větev se zachovají. Vytvoření odpovědi, její uložení do requests a odstranění checkpointu dokončí požadavek. Finální diagnostiky a výsledek se ukládají v jedné SQLite transakci. Restart před jejím potvrzením znovu použije poslední checkpoint.

Obnova podporuje podmíněné COMMIT, lokální rozsah větví, RETURN uvnitř transakce i další navazující transakce. Každý již potvrzený prefix zůstává potvrzený; pozdější chyba nemůže zpětně vrátit dřívější COMMIT. Pád před prvním commitem vrátí otevřenou SQLite transakci; čistě nativní scénář lze obnovit od počátečního checkpointu.

Automatická obnova je povolena, pokud zbývající pokračování obsahuje jen výrazy, čtení, nativní transakční zápisy, QUEUE, PUBLISH a operace deklarované jako pure/read nebo s kontraktem retry: idempotent. Pokud by pokračování mohlo znovu vykonat externí CALL bez bezpečného retry kontraktu, runtime jej automaticky nespustí. Požadavek zůstane running; opakování se stejným klíčem vrátí `409 outcome_unknown`. Toto omezení se týká pluginů bez bezpečného kontraktu opakování, například přímého CALL Email.send_confirmation. Payment.create_url má nyní trvalý deník a idempotentní kontrakt popsaný níže. SQLite rollback nesmaže případný účinek u externího poskytovatele; automatická kompenzace zatím není implementována.

Checkpoint obsahuje fingerprint zdroje, verze formátu a registru operací. Změněný program nesmí automaticky převzít staré pokračování; obnova vyžaduje původní odpovídající program. Migrace rozpracovaných scénářů zatím není implementovaná. Syntaktická změna zdroje včetně komentáře mění fingerprint. Staré running požadavky bez checkpointu nemají automatickou obnovu. Nejde o HA ani o obnovu živých WebSocket spojení.

E2E testy zastavují skutečný server pomocí SIGKILL přesně před a po COMMIT, restartují jej se stejnou databází a ověřují stav SQL, odpověď, pluginy a frontu. Testovací marker se aktivuje pouze explicitním nastavením `FLOW_E2E_CRASH_MARKER`, `FLOW_E2E_CRASH_PHASE` a `FLOW_E2E_CRASH_ROUTE`; normální běh tyto proměnné nepoužívá.


## Typované odchozí MQTT zprávy

PUBLISH používá tentýž typovaný zdroj jako příjem a dotazy; aplikace neskládá raw topic string:

```text
TYPE MowerAction = String WHERE value IN ["start", "stop"]
MQTT DeviceCommand
    TOPIC "devices/{device_id}/command"
    PARAM device_id DeviceID
    PAYLOAD {command_id: String, action: MowerAction}
    HISTORY 24 HOURS

HTTP POST /api/devices/:device_id/commands
INPUT device_id DeviceID
INPUT action MowerAction
INPUT token String
REQUIRE EXISTS access IN DeviceAccess WHERE access.token = :token AND access.device_id = :device_id AND EXISTS user IN Users WHERE user.id = access.user_id ELSE 403 forbidden "Přístup k zařízení je zamítnut."
TRANSACTION
    outgoing = PUBLISH DeviceCommand(:device_id) WITH {command_id: uuid("command"), action: :action}
    COMMIT
RESPONSE 202 WITH outgoing
```

PUBLISH lze použít bez vazby nebo jako `jméno = PUBLISH …`; vrací `{id: String, state: String}` se stavem queued. Parametry mají přesné typy MQTT PARAM, payload má přesný typ PAYLOAD. Kompilátor odmítne neznámý zdroj, chybnou aritu, pole, typ či známou konstantu mimo rozsah. Runtime znovu validuje vypočítaný payload i hodnoty parametrů; `/`, `+`, `#` a NUL v parametru topicu odmítá. Zpráva včetně topicu má limit 64 kB. Neplatná hodnota vrátí transakci a neodešle žádnou kopii.

Volitelný suffix `RETAIN BoolVýraz` řídí retained stav, výchozí je false. Například `PUBLISH DeviceStatus(:device_id) WITH {online: true, battery: 50} RETAIN true` uloží poslední hodnotu i pro pozdější MQTT subscriber. Příkaz se provádí pouze v aktivní TRANSACTION, stejně jako INSERT a QUEUE. V jedné transakci může být více PUBLISH i PUBLISH uvnitř FOR EACH. Rollback odstraní jejich frontové záznamy; síťový přenos se před commitem neprovádí.

Worker zpracovává potvrzenou SQLite frontu mqtt_outbox. Stavy jsou queued → accepted → sent. Ve stavu accepted již existuje validovaná zpráva v nativní MQTT historii a případný retained stav; toto potvrzení je atomické se změnou stavu fronty. Po restartu se nativní záznam znovu nevloží. Navazující ON MQTT scénář používá stabilní idempotency klíč `mqtt-outbox:<id>`, takže potvrzené obchodní změny handleru se neopakují. Transportní zpráva se rozešle přes lokální broker a typované WebSocket odběry; výsledek handleru je samostatná obchodní operace, ne potvrzení fyzického provedení příkazu na zařízení.

sent znamená dokončení lokálního rozeslání, nikoliv potvrzení od zařízení. Přenos připojeným MQTT subscriberům je QoS 0. Pád mezi rozesláním a označením sent může po obnově vytvořit další živou kopii; konzument potřebuje vlastní deduplikaci, například podle command_id z příkladu. Odpojení klienti mají k dispozici pouze retained hodnotu, pokud je povolená; fronta není trvalou frontou pro každého MQTT či WebSocket klienta. PUBLISH zatím necílí na externí broker.

Před zpracováním se čekající zpráva znovu ověří podle aktuálního kontraktu. Retained hodnoty z historie se při novém MQTT subscribe také ověří; záznamy neplatné podle aktuálního kontraktu se neodešlou. Nekompatibilní změna zdroje nebo payloadu označí čekající záznam failed a uloží chybu, aniž odešle neplatná data. Admin → Živá zařízení ukazuje topic, přesný payload, stav, chybu a původní HTTP/MQTT požadavek. Po opravě kontraktu lze failed záznam obnovit tlačítkem nebo `POST /api/mqtt-outbox/:id/retry`; již sent záznam nelze tímto API zopakovat. Dočasné selhání přenosu ponechá accepted záznam pro další pokus.

Stejný admin panel odesílá příkazy start/stop podle vybraného zařízení a tokenu. E2E testy používají skutečné MQTT subscriptions, WebSocket i Chrome a SIGKILL v queued a accepted fázi. Ověřují rollback, typy, autorizaci HTTP příkazu, native history, retained zprávu, ON MQTT a opravu odmítnuté zprávy po změně kontraktu.


## Trvalý deník externích CALL

Registrovaná operace může deklarovat `retry: :idempotent`. Takový plugin implementuje `call(input, context)` a získá stabilní `context["idempotency_key"]`. Tento kontrakt znamená, že opakování stejného klíče a stejného vstupu neprovede druhý logický účinek a vrátí původní výsledek. Runtime tuto vlastnost poskytovatele nemůže sám zajistit. Operace bez tohoto kontraktu se při obnově slepě neopakují. Aplikační CALL syntaxe se nemění:

```text
payment = CALL Payment.create_url WITH {order_id: order.id, amount_cents: order.total_cents, currency: "CZK", country: user.country, method: :payment_method}
```

Vedle hlavní databáze vzniká soubor `<DATABASE_PATH>.effects` se samostatným SQLite deníkem, WAL a synchronous=FULL. Proto zápis deníku není součástí objednávkové transakce a přežije její rollback. Deník zaznamená před provedením pending operaci se stabilním id, místem v programu, pořadím volání a hashem vstupu. Po úspěšném výsledku nebo definitivní obchodní chybě uloží completed výsledek a diagnostiku.

Při obnově runtime rozlišuje dvě situace. completed vrátí uložený výsledek bez dalšího requestu poskytovateli, včetně uložené obchodní chyby pro TRY/CATCH. pending zopakuje volání se stejným idempotency klíčem a vstupem. Poskytovatel musí původní výsledek uchovat; samotná existence deníku negarantuje právě jeden externí účinek. Deník má identitu svázanou s hlavní databází; chybějící či zaměněný soubor se nesmí potichu vytvořit jako náhrada a startup takový nesoulad odmítne. Expirace klíčů u poskytovatele ruší kontrakt bezpečného opakování.

Deník zachovává hodnoty uuid/now/ago, včetně UUID vznikajících v nativním QUEUE a PUBLISH. Pozice deníku jsou součástí každého checkpointu a rozlišují také opakovaná volání ve FOR EACH. Obnova necommitnutého prefixu tak rekonstruuje stejné identifikátory a použije stejné vstupy externích operací. Čtené zdrojové hodnoty se rovněž evidují; při jejich změně vrátí obnova `409 journal_source_conflict` místo přepsání novějšího skladu. Odlišná struktura volání nebo jiný vstup vrátí `409 journal_conflict`. Případné již vzniklé externí účinky při takovém konfliktu zůstávají k inspekci a ručnímu řešení.

Nejasný výsledek HTTP poskytovatele vyvolá ExternalUnknown. Nativní transakce se vrátí, odpověď má `503 outcome_unknown` a původní požadavek zůstává running s checkpointem. Tato nejistota se nezaměňuje za definitivní obchodní chybu a nelze ji zachytit běžným TRY/CATCH. Opakovaný HTTP požadavek se stejným Idempotency-Key může obnovu spustit bez restartu. Stejnou obnovu nabízí `POST /api/requests/:id/resume` a tlačítko v adminu → Pluginy. Admin ukazuje id operace, původní požadavek, vstup, pending/completed a počet pokusů.

Payment.create_url používá tento kontrakt. Výchozí režim zůstává lokální demo. Volitelné `PAYMENT_PROVIDER_URL` zapne pro CALL i QUEUE skutečný HTTP POST s JSON vstupem a hlavičkou Idempotency-Key; úspěšná odpověď musí mít `{url: String, provider: String, method: String}`. Tento endpoint musí implementovat výše uvedený kontrakt, nejde o automatické připojení konkrétní platební služby. Změna endpointu je součástí fingerprintu a blokuje převzetí starého checkpointu.

Stejný deník pokrývá také QUEUE operací s kontraktem retry: idempotent. Každá potvrzená úloha má vlastní prostor deníku odvozený z původního požadavku a id úlohy; dvě úlohy ve stejné iteraci proto mají různé klíče. Worker volá call(input, context), ověřuje vstup a výstup a uloží stav úlohy spolu s diagnostikou v jedné hlavní transakci. Pád před tímto potvrzením znovu použije dokončený výsledek nebo zopakuje pending volání se stejným klíčem. Neplatná úspěšná odpověď poskytovatele se nepotvrdí jako completed; případný externí účinek je nejasný a čeká na opravu odpovědi.

Retry politika platí i pro nejasné výsledky. Po vyčerpání pokusů zůstane failed úloha s outcome_unknown zachována i při DELETE; nejistý účinek nelze smazáním úlohy prohlásit za neprovedený. Totéž platí pro chybu kontraktu. Definitivní obchodní chybu lze podle DELETE odstranit, ale její diagnostika a completed deník zůstanou. U idempotentního pluginu se definitivní výsledek při dalších pokusech vrací z deníku, včetně chyby; nová obchodní operace vyžaduje novou úlohu. Retry neznamená vytvoření nového klíče.

POST /api/jobs/:id/retry obnoví failed úlohu se stejným id, vstupem a externím klíčem. Admin → Pluginy ukazuje pro pending frontovou operaci tlačítko Zopakovat úlohu, jakmile automatická politika skončí. Původní HTTP požadavek může být již committed: obnova úlohy ho znovu neprovádí. /api/email-jobs/:id/retry zůstává kompatibilní alias; pouze u simulovaného emailu navíc vypíná simulate_failure. Změna konfigurace poskytovatele či kontraktu již zapsané frontové operace způsobí konflikt a zabrání volání nového poskytovatele; po vrácení původní konfigurace lze úlohu obnovit. Před prvním pokusem úloha používá aktuální registrovaný kontrakt a konfiguraci, které se připnou při zápisu do deníku.

Email zůstává simulovaný a bez garance právě jednoho externího provedení. Automatická kompenzace, migrace rozpracovaných operací a úklid deníku zatím chybí. Záloha potřebuje konzistentní hlavní databázi i její deník; samotný hlavní SQLite soubor nestačí k bezpečné obnově externích operací.

E2E testy ověřují také frontové volání, oddělené klíče úloh, SIGKILL ve workeru, obnovu přes Chrome, změnu poskytovatele, neplatný výstup a rozdíl mezi nejasnou a definitivní chybou při DELETE. Provozují samostatný skutečný HTTP poskytovatel s idempotentními účtenkami, přeruší spojení nebo provedou SIGKILL před a po zapsání externího výsledku. Ověřují jednu účtenku i při opakovaném requestu, stejné vstupy a klíč, obnovu v iteraci, TRY/CATCH, změnu skladových dat i tlačítko obnovy v Chrome.
