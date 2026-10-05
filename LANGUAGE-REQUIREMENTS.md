# Požadavky na typy a konečné kolekce v aplikačním jazyce

Status: Potvrzené požadavky z návrhu jazyka. Tento dokument určuje požadované chování, nikoliv současnou implementaci prototypu. V oblasti číselných typů a kolekcí má přednost před staršími ilustrativními příklady v [DECLARATIVE-LANGUAGE.md](DECLARATIVE-LANGUAGE.md). Nevyžaduje Lean ani znalost fyzikálních jednotek.

## 1. Povinné brandované číselné typy

- Každá číselná hodnota v business logice musí mít pojmenovaný nominální typ (brand). Holé `Int`, `Float`, `Number` ani jiné obecné číselné typy nesmějí být typem pole, parametru, proměnné nebo výsledku.
- `integer` a `decimal` jsou konstrukce pro definici pojmenovaného typu; nesmějí se použít přímo jako typ hodnoty.
- Dvě samostatné deklarace vytvářejí odlišné typy, i když mají stejnou reprezentaci a rozsah. `OrderId` nelze použít místo `ProductId` v dotazu, porovnání, přiřazení ani volání pluginu.
- Značka se musí zachovat přes vstupy, výrazy, kolekce, databázové sloupce, dotazy, kontrakty pluginů a odpovědi. Interní SQL reprezentace nesmí značku ztratit z typového schématu aplikace.
- Jazyk nesmí hodnotu implicitně rozbalit na obecné číslo ani přeznačit na jiný brand. Nevyžaduje automatické převody jednotek, rozměrovou analýzu ani katalog vztahů mezi značkami.
- Číselný literál lze použít, pokud kontext jednoznačně určí brand a hodnota splňuje jeho omezení. Literál bez určitelného významového typu se odmítne.

## 2. Konečné číselné rozsahy

Každý číselný typ musí definovat konečnou množinu povolených hodnot:

```text
[type ProductId [integer [range 1 [pow 10 9]]]]
[type OrderId [integer [range 1 [pow 10 9]]]]
[type ProductCount [integer [range 0 [pow 10 6]]]]
[type ByteValue [integer [range 0 [sub [pow 2 8] 1]]]]
[type Weight [decimal [range 0 [pow 10 6]] [scale 3]]]
[type Rating [decimal [range 1 5] [scale 1]]]
```

- Dolní a horní mez jsou povinnou součástí kontraktu `[range min max]`. Chybějící rozsah odmítne budoucí validátor, nikoliv obecný parser seznamů.
- `decimal` musí navíc určit konečnou desetinnou přesnost pomocí `[scale n]`. Například `Rating` obsahuje pouze `1.0, 1.1, …, 5.0`, nikoliv libovolná reálná čísla mezi mezemi.
- Meze mohou být přesné prefixové konstantní výrazy, například `[pow 10 9]` nebo `[sub [pow 2 8] 1]`. Vyhodnocují se při kontrole definice, nezávisle na runtime datech.
- Vstupy, výstupy pluginů a výsledky výpočtů musí splňovat brand, rozsah i přesnost. Přetečení, obalení hodnoty a tiché zaokrouhlení nejsou přípustné. Porušení kontraktu musí skončit definovanou chybou.

## 3. Povinné limity kolekcí

- Každá kolekce musí mít při kontrole programu známou konečnou horní mez počtu prvků. Platí to i pro vstupní seznamy, výsledky pluginů, databázové dotazy, historii MQTT, vytvořené a vnořené kolekce.
- Nelze načíst všechny objednávky uživatele bez omezení počtu. Samotný filtr ani konečné časové okno nejsou důkazem maximálního počtu výsledků.
- Dotaz musí mít explicitní `[first kolekce limit]` nebo `[last kolekce limit]`, nebo čerpat z kontraktu, který už zaručuje konečnou horní mez. Výběr nejvýše jedné položky má mez jedna.
- Dynamický limit je přípustný pouze s typem, jehož horní mez je známá. Limit `PageSize` z následujícího příkladu znamená nejvýše 100 položek:

```text
[type PageSize [integer [range 1 100]]]
[query Orders
  [input userId UserID]
  [input pageSize PageSize]
  [output [list OrderSummary [max 100]]]
  [user [entity $userId]]
  [result [first user.orders $pageSize]]]
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

Současný Elixir projekt je pouze parser jednotného hranatého zápisu do AST. Nekontroluje doménové typy, brandy, rozsahy, limity kolekcí ani platnost propojení. Požadavky tohoto dokumentu patří budoucímu validátoru a runtime. Implementovanou syntaxi popisuje [dokumentace parseru](prototype/docs/LANGUAGE.md).
