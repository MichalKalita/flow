# Požadavky na typy a konečné kolekce v aplikačním jazyce

Status: Potvrzené požadavky z návrhu jazyka. Tento dokument určuje požadované chování, nikoliv současnou implementaci prototypu. V oblasti číselných typů a kolekcí má přednost před staršími ilustrativními příklady v [DECLARATIVE-LANGUAGE.md](DECLARATIVE-LANGUAGE.md). Nevyžaduje Lean ani znalost fyzikálních jednotek.

## 1. Povinné brandované číselné typy

- Každá číselná hodnota v business logice musí mít pojmenovaný nominální typ (brand). Holé `Int`, `Float`, `Number` ani jiné obecné číselné typy nesmějí být typem pole, parametru, proměnné nebo výsledku.
- `INTEGER` a `DECIMAL` jsou konstrukce pro definici pojmenovaného typu; nesmějí se použít přímo jako typ hodnoty.
- Dvě samostatné deklarace vytvářejí odlišné typy, i když mají stejnou reprezentaci a rozsah. `OrderId` nelze použít místo `ProductId` v dotazu, porovnání, přiřazení ani volání pluginu.
- Značka se musí zachovat přes vstupy, výrazy, kolekce, databázové sloupce, dotazy, kontrakty pluginů a odpovědi. Interní SQL reprezentace nesmí značku ztratit z typového schématu aplikace.
- Jazyk nesmí hodnotu implicitně rozbalit na obecné číslo ani přeznačit na jiný brand. Nevyžaduje automatické převody jednotek, rozměrovou analýzu ani katalog vztahů mezi značkami.
- Číselný literál lze použít, pokud kontext jednoznačně určí brand a hodnota splňuje jeho omezení. Literál bez určitelného významového typu se odmítne.

## 2. Konečné číselné rozsahy

Každý číselný typ musí definovat konečnou množinu povolených hodnot:

```text
TYPE ProductId = INTEGER 1..10^9
TYPE OrderId = INTEGER 1..10^9
TYPE ProductCount = INTEGER 0..10^6
TYPE ByteValue = INTEGER 0..(2^8 - 1)
TYPE Weight = DECIMAL 0..10^6 SCALE 3
TYPE Rating = DECIMAL 1..5 SCALE 1
```

- Dolní a horní mez jsou povinnou součástí syntaxe. Slovo `RANGE` se nepoužívá; samotné `INTEGER` nebo `DECIMAL` je neplatná deklarace.
- `DECIMAL` musí navíc určit konečnou desetinnou přesnost pomocí `SCALE`. Například `Rating` obsahuje pouze `1.0, 1.1, …, 5.0`, nikoliv libovolná reálná čísla mezi mezemi.
- Meze mohou být přesné konstantní výrazy s mocninou `^`, například `10^9` nebo `2^8 - 1`. Vyhodnocují se při kontrole definice, nezávisle na runtime datech.
- Vstupy, výstupy pluginů a výsledky výpočtů musí splňovat brand, rozsah i přesnost. Přetečení, obalení hodnoty a tiché zaokrouhlení nejsou přípustné. Porušení kontraktu musí skončit definovanou chybou.

## 3. Povinné limity kolekcí

- Každá kolekce musí mít při kontrole programu známou konečnou horní mez počtu prvků. Platí to i pro vstupní seznamy, výsledky pluginů, databázové dotazy, historii MQTT, vytvořené a vnořené kolekce.
- Nelze načíst všechny objednávky uživatele bez omezení počtu. Samotný filtr ani konečné časové okno nejsou důkazem maximálního počtu výsledků.
- Dotaz musí mít explicitní `LIMIT`, nebo čerpat z kontraktu, který už zaručuje konečnou horní mez. Výběr nejvýše jedné položky má mez jedna.
- Dynamický limit je přípustný pouze s typem, jehož horní mez je známá. Limit `PageSize` z následujícího příkladu znamená nejvýše 100 položek:

```text
TYPE PageSize = INTEGER 1..100
PARAM pageSize PageSize

orders = FROM Orders
    WHERE user = :userId
    LIMIT :pageSize
```

- Stránka smí obsahovat `items`, `hasMore` a volitelný `nextCursor`. Informace o další stránce nezvyšuje limit vrácené kolekce.
- Automatické načítání dalších stránek nesmí obejít omezení. Iterace přes stránky musí mít konečný celkový limit položek nebo stránek, z něhož lze odvodit počet položek.
- Filtry, spojování, rozšiřování, mapování a iterace musí zachovávat nebo odvozovat horní meze výsledků. Pokud mez nelze určit, program se odmítne, případně musí autor dodat limit.
- `EXISTS` může vrátit boolean bez načtení celé kolekce do aplikace. Nesmí sloužit jako skrytý způsob předání neomezené kolekce.

Limit výsledku neznamená automaticky limit počtu prohledaných databázových řádků ani omezení celkového počtu uložených záznamů. Tento požadavek stanovuje velikost kolekcí a rozsah práce deklarovaných aplikačním scénářem.

## 4. Bezpečné agregace a výpočty

- Kontrola programu musí z mezí hodnot a maximálního počtu prvků odvodit bezpečný rozsah agregace. Také odvozený výsledek musí mít významový brand.
- Pro přesně `N` hodnot v `1..10^9` leží součet v `N..N × 10^9`. Pro nanejvýš `N` hodnot včetně prázdné kolekce leží v `0..N × 10^9`.
- Například součet nejvýše 1 000 takových hodnot vyžaduje rozsah `0..10^12`. Cílový typ musí pojmout celý možný výsledek; jinak je nutné odmítnout program nebo explicitně ošetřit kontrolu užšího rozsahu jako možnou chybu.
- Samotná agregace nesmí obejít limit načítání: ani `SUM` nad neomezeným počtem objednávek není přípustný.
- Výpočet mezí musí být přesný. Runtime musí použít reprezentaci, která bezpečně pojme odvozený rozsah; nesmí spoléhat na přetečení strojového čísla.
- Limity iterací musí umožnit odvodit maximální počet aplikačních operací a volání pluginů v jednom průchodu scénářem. Neomezená smyčka přes další stránky není povolená.

## Stav prototypu

Současný Elixir prototyp tyto požadavky zatím plně nevynucuje: podporuje obecné číselné typy, strukturální aliasy a kolekce bez povinných limitů. Tento dokument je podkladem pro budoucí změnu jazyka, nikoliv tvrzením, že je tato syntaxe již implementovaná. Aktuální podporu popisuje [dokumentace prototypu](prototype/docs/LANGUAGE.md).
