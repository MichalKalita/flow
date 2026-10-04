# OrderLab Flow — jazyk scénářů

Flow deklaruje HTTP rozhraní, typy dat, dotazy, pravidla a účinky. Implementace databáze a pluginů patří hostitelskému Elixiru. Scénář nemůže volat libovolné Elixir funkce, spouštět SQL ani definovat vlastní funkce.

## Stav implementace

Celá aplikace je v jediném `priv/workflows/application.flow`: typy, tabulky, MQTT zdroje a HTTP scénáře. Runtime jej při startu parsuje a typově kontroluje. `FLOW_PATH` nastaví cestu jiného jediného souboru. Změna vyžaduje restart. Lexer, parser, typová kontrola, validátor hodnot a interpreter jsou v `lib/order_lab/language/`. Objednávky i další CRUD scénáře používají stejný interpreter. `POST /api/language/check` přijímá zdroj jako text a vrací diagnostiku bez spuštění.

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

Zápisy a `QUEUE` vyžadují aktivní transakci. `COMMIT` musí být právě jednou, mimo iteraci; po něm se nesmí provádět další účinky. Chyba před commitem vrací databázi do původního stavu. Vnořená transakce není podporována. `INSERT` vrací vložený záznam, `UPDATE` seznam změněných záznamů a `DELETE` seznam výsledků hostitelského adaptéru. Změny jsou hodnoty a nevymění automaticky dříve navázaný snapshot.

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

MQTT 3.1.1 server na `127.0.0.1:1883` (`MQTT_PORT`) podporuje CONNECT, PUBLISH QoS 0/1, SUBSCRIBE (doručení QoS 0), UNSUBSCRIBE, retained zprávy, PING a DISCONNECT. QoS 1 potvrzuje až po validaci a uložení. Posledních 100 QoS 1 identifikátorů deduplikuje při DUP v rámci spojení. Neplatný payload nebo nedeklarovaný topic ukončí spojení bez PUBACK. Prázdný retained payload ruší retained stav. SQLite uchová historii i retained stav přes restart. Broker zatím nemá persistent sessions, QoS 2, will messages, TLS ani autentizaci. MQTT slouží také jako spouštěč `ON MQTT`; odchozí PUBLISH ještě není implementované.

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
