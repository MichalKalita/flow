# Deklarativní jazyk pro skládání aplikací

Status: Koncepční návrh. Název, syntaxe a úplná sémantika zatím nejsou určeny. Dokument doplňuje [BLACKBOX-REQUIREMENTS.md](BLACKBOX-REQUIREMENTS.md), jeho potvrzené požadavky nemění.

Aktualizované závazné požadavky na brandované číselné typy, konečné rozsahy a limity kolekcí jsou v [LANGUAGE-REQUIREMENTS.md](LANGUAGE-REQUIREMENTS.md). V těchto oblastech nahrazují starší ilustrativní příklady níže.

## Účel a principy

Uživatel nebo AI píše aplikační logiku jako deklarativní propojení hotových operací: endpointů, dotazů, transformací, změn stavu a služeb. Jazyk je lepidlo; implementace operací dodává platforma, pluginy a konektory napsané v běžných programovacích jazycích. Autor aplikace propojí objednávku, sklad, platbu a email, aniž by implementoval jejich technické provedení.

- Nelze deklarovat vlastní funkce; schopnosti dodávají pluginy a platforma.
- Zápis má být čitelný, s řádkovým členěním a odsazováním podobným Pythonu.
- Program deklaruje výsledky, podmínky, operace a závislosti.
- Runtime volí provedení, ale musí zachovat význam programu včetně efektů, transakčních hranic a pravidel selhání.
- Typy nesou důkazy omezení hodnot a vztahů mezi nimi, inspirované Leanem; kontrakty odmítají neplatná propojení.
- Chyby, rollback, obnova a HA patří do společného modelu platformy.

Podmínka nemusí být instrukce větvení; runtime ji může přeložit například do dotazu. Pořadí vyplývá z datových závislostí a explicitních omezení. Musí zachovat i pozorovatelné efekty.

## Typy, omezení a kontrakty

Typový model je inspirovaný Leanem: hodnota musí splňovat podmínky svého typu. Zahrnuje refinement typy (podtypy s predikátem) a vztahy mezi hodnotami. Nejde pouze o označení `Decimal` nebo `File`.

Ilustrativní deklarace:

```text
TYPE Rating = Decimal WHERE 1.0 <= value <= 5.0
TYPE PositiveQuantity = Integer WHERE value > 0
TYPE PriceRange = { min Money<CZK>, max Money<CZK> }
    WHERE 0 <= min <= max

TYPE ProductPhoto = Image
    WHERE format IN { JPEG, PNG }
    WHERE byteSize <= 10MiB
    WHERE 1 <= width <= 8000 AND 1 <= height <= 8000

FILTER ProductFilter
    minRating Rating?
    price PriceRange?

HTTP POST /products/:id/photo
INPUT id Product.ID
INPUT photo ProductPhoto

result = image.resize WITH photo, width = 800, height = 600
RETURN result
```

`Image` označuje obsah úspěšně ověřený podporovaným dekodérem, nikoliv příponu či tvrzení klienta. PDF nesplňuje vstup `ProductPhoto`. Typ váže ověřené vlastnosti ke konkrétnímu obsahu souboru; změna obsahu vyžaduje nové ověření. Kontrakt transformace určuje zaručený výstupní typ a vlastnosti, které je nutné znovu prokázat.

Známý literál `Rating = 7.0` se odmítne před spuštěním. Vnější data z HTTP, MQTT nebo souboru získají omezený typ až po úspěšné validaci. Každé vytvoření hodnoty musí doložit podmínky typu statickým důkazem, odvozením z kontraktů nebo ověřením za běhu. Například `rating + 1` není automaticky `Rating`. Přesný důkazový mechanismus zůstává otevřený; plná kompatibilita s Leanem není rozhodnuta.

Plugin dodává implementaci a kontrakt: typované vstupy a výstupy včetně zaručených podmínek, chyby, účinky a možnosti opakování nebo kompenzace. Typy mají znemožnit neplatná propojení; provozní selhání a nesprávně zvolená obchodní pravidla zůstávají možné.

## Iterace, existence a agregace

Jazyk umožní iterovat nad existujícími kolekcemi a vyjadřovat `EXISTS`, `NOT EXISTS`, `ALL`, filtrování a agregace. `FOR EACH` deklaruje operaci pro každý prvek, nikoliv povinnou sekvenční smyčku. Runtime může použít dávku, dotaz či souběh při zachování kontraktů. Nejde o neomezené `WHILE` ani rekurzi.

Ilustrativní syntaxe:

```text
# Operace pro každý existující prvek kolekce
FOR EACH item IN cart.items
    orders.prepareItem WITH order.ID, item.product, item.quantity

# Existuje alespoň jeden nedostupný produkt?
IF EXISTS item IN cart.items WHERE item.quantity > item.product.stock
    FAIL insufficientStock

# Žádná aktivní objednávka zákazníka
REQUIRE NOT EXISTS order IN Orders
    WHERE order.customer = currentUser.ID AND order.status = active

# Všechny položky splňují obchodní omezení
REQUIRE ALL item IN cart.items SATISFY item.quantity <= item.product.maxPerOrder

# Výběr a agregace
items = FROM cart.items WHERE product.active
quantity = COUNT items
amount = SUM item IN items OF item.quantity * item.price
```

Přesná syntaxe není rozhodnuta. Kolekce, jejich konzistence a souběžné efekty musí mít definovaný kontrakt; samotné `EXISTS` nenahrazuje atomickou kontrolu a odečet skladu.

## Příklad: typovaná MQTT data přes HTTP

MQTT zdroje mají striktní kontrakt: název typu zdroje, typ parametru a přesné schéma zpráv. Například `DeviceStatus(Device.ID)` a `DevicePosition(Device.ID)` se mapují na `/devices/{id}/status` a `/devices/{id}/position`. Mapování a validace zpráv patří do definice zdroje; aplikace používá jeho typované rozhraní a přímo pojmenovaná pole.

Ilustrativní syntaxe endpointu; závorky vybírají zdroj podle parametru, nejde o deklaraci uživatelské funkce:

```text
HTTP GET /device/:id
INPUT id Device.ID

status = LAST FROM DeviceStatus(:id)
    WHERE receivedAt <= request.time
    ORDER BY receivedAt ASC

positions = FROM DevicePosition(:id)
    WHERE receivedAt BETWEEN request.time - 5min AND request.time
    ORDER BY receivedAt ASC

RETURN {
    device: :id,
    online: status.online,
    battery: status.battery,
    positions: FROM positions SELECT latitude, longitude, receivedAt
}
```

Schéma může určit `online: Bool`, `battery: Decimal` v rozsahu 0 až 100, `latitude` v rozsahu −90 až 90 a `longitude` v rozsahu −180 až 180. Přístup k polím a jejich kompatibilita se kontrolují před spuštěním; příchozí hodnoty se validují. `receivedAt` je metadata příjmu poskytovaná zdrojem. Zprávy neodpovídající schématu nesmí vstoupit do typované kolekce; politika jejich odmítnutí zůstává otevřená.

Platforma nebo plugin zajišťuje příjem a historii, kterou samotné MQTT automaticky neposkytuje. Kontrakt určuje retenci, časovou sémantiku, pořadí a dostupnost. `request.time` je společný časový bod požadavku; později přijaté zprávy se nezahrnou. Chování při chybějícím stavu, prázdné historii a konzistence mezi zdroji zůstávají otevřené.

## Příklad objednávky

Ilustrativní zápis, nikoliv schválená syntaxe:

```text
HTTP POST /orders
INPUT cart Cart.ID
INPUT paymentMethod PaymentMethod

TRANSACTION
    order = orders.prepare FROM cart
        SNAPSHOT product, quantity, price
    stock = inventory.prepareDecrease FROM order.items
        REQUIRE remaining >= 0
    payment = payments.createLink
        WITH paymentMethod, order.total, order.ID
        AFTER stock
    QUEUE email.orderConfirmation WITH order, payment.url
    COMMIT

RESPONSE 201 WITH order.ID, payment.url
```

Připraví se kopie produktů a cen, odečet skladu a platební odkaz. Nedostatek kusů nebo definitivní odmítnutí platby před commitem znamená rollback nativních změn: objednávka ani odečet se nepotvrdí. Kontrola a odečet skladu musí respektovat souběžné objednávky.

Emailová úloha se přijme s objednávkou a provede po commitu. Odpověď zde potvrzuje přijetí úlohy, nikoliv doručení emailu.

## Transakce, chyby a HA

Nativní SQL, účastnící se souborové zápisy, KV, audit, fronty a pub/sub podléhají jednotnému explicitnímu commitu podle požadavků platformy. Účastnící se změny se potvrdí společně, nebo žádná. Runtime obnovuje přerušený commit a zabraňuje duplicitním změnám aplikačního stavu při interním opakování stejné operace.

Externí služby atomicitu automaticky nezískají. Příklad předpokládá nativní sklad. Pokud platební odkaz vznikne a selže commit, může odkaz zůstat. Kompenzace může selhat; odeslaný email běžně nelze odvolat. Timeout nemusí znamenat, že akce neproběhla. Plugin musí určit bezpečné opakování a řešení nejistého výsledku.

Stejný program funguje lokálně i v HA podle garancí platformy. V degradovaném stavu se replikovaný zápis nesmí změnit na lokální ani se commitnout jen povolená část transakce. Obnova sama nezaručuje právě jedno provedení externího efektu.

## Kontext a otevřené otázky

Pracovní označení je **typovaný deklarativní orchestrační DSL s transakčním a obnovitelným runtime**. Spojuje deklarativní programování, dataflow, refinement a závislé typy inspirované Leanem, efektové systémy, durable execution a transakční zpracování.

Inspirace: [NoFlo](https://noflojs.org/) pro skládání komponent, [Ash + Reactor](https://reactor.hexdocs.pm/readme.html) pro závislosti a kompenzace, [Ballerina](https://ballerina.io/use-cases/integration/) a [Open Workflow Specification](https://github.com/open-workflow-specification/specification) pro integrace, [Unison](https://www.unison-lang.org/docs/language-reference/abilities-and-ability-handlers/) pro typované efekty a [Temporal](https://docs.temporal.io/tasks) pro obnovu provádění. Žádný zde není považován za hotovou realizaci celé kombinace požadavků platformy.

Jazyk navazuje na [BLACKBOX-LAYERS.md](BLACKBOX-LAYERS.md); propojení s [ARCHITECTURE-PROPOSAL.md](ARCHITECTURE-PROPOSAL.md) zůstává otevřené. Dopracovat je nutné syntaxe, kompozice, kontrakty pluginů, konzistence kolekcí, externí retry, význam odpovědí a aktualizace rozpracovaných operací.

Implementace prototypu: [syntax a aktuální podpora](prototype/docs/LANGUAGE.md). Celá aplikace je v jediném [application.flow](prototype/priv/workflows/application.flow), včetně typů, MQTT zdrojů a HTTP scénářů.
