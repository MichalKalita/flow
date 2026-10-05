# Flow: nejmenší jednotný zápis do AST

Aktuální implementace pouze parsuje syntaxi. Nemá doménový validátor, typovou kontrolu ani runtime. Návrhové doménové konvence jsou ve [workflow](../priv/workflows/application.flow) a [permissions](../../examples/PERMISSIONS.md).

## Jedna struktura

```text
[název argument argument [vnořený argument] ...]
```

Program je posloupnost hranatých seznamů. Řádky a odsazení nemají význam. Žádné `{}`, `()`, čárky, dvojtečky, anotace nebo infixové operátory nejsou strukturální syntaxí; pokud se vyskytnou v nequoted tokenu, jsou jeho textem. Jedinými delimitery jsou `[` a `]`, uvozovky, whitespace a `#` pro komentář.

Každá doménová konstrukce používá stejný seznam:

```flow
[type Quantity [integer [range 1 100]]]
[entity User [field id UserID] [field orders [list Order] [inverse Order.user]]]
[permissions User [Order [READ [when [eq target.user actor]]]]]
[query Orders [input userId UserID] [output [list OrderSummary [max 10]]]
  [http GET "/users/{userId}/orders"]
  [user [entity $userId]]
  [result [order [last user.orders 10] Order.createdAt DESC]]]
```

Výrazy jsou prefixové: `[add a b]`, `[eq a b]`, `[and podmínka podmínka]`. Reference `user.orders`, `actor`, `$userId` a názvy operací jsou obyčejné symboly. Parser nemusí znát operátory, jejich prioritu, aritu ani prostředí proměnných.

Strukturální jádro má jediný složený uzel: seznam. Ten současně vyjadřuje deklaraci, argumenty, hodnotu kolekce, záznam i výraz. Žádná nová doménová schopnost nevyžaduje změnu gramatiky nebo další parser funkci. Zbývající lexikální pravidla slouží pouze k jednoznačnému rozlišení textu, přesného čísla a hranic struktury.

Prázdné seznamy `[]`, prázdný program, opakované deklarace i neznámé keywords jsou syntakticky přijatelné. Zda představují platnou aplikaci, určí až validátor. Parser například přijme `[type Quantity [integer [range 10 1]]]`; významově neplatné meze nejsou syntaktická chyba.

## Gramatika

Gramatika pracuje s tokeny; mezery a komentáře jsou trivia:

```ebnf
document = { list }, EOF ;
list     = "[", { value }, "]" ;
value    = list | symbol | number | string ;
```

Lexikální pravidla:

- Whitespace jsou ASCII mezera, tab, LF a CR. CRLF se počítá jako jeden newline.
- `#` mimo řetězec zahajuje komentář do CR, LF nebo konce souboru. Komentáře nevstupují do AST.
- Bare token je maximální neprázdná posloupnost Unicode scalar values mimo whitespace, hranaté závorky, `#` a `"`. Řídicí kódy U+0000–U+001F a U+007F–U+009F mimo řetězce nejsou povolené.
- Bare token začínající číslicí nebo znaménkem bezprostředně následovaným číslicí musí být číslo dle následující gramatiky. Ostatní bare tokeny jsou symboly. Například `.field`, `+`, `-`, `true`, `false` a `null` jsou symboly; jejich význam parser neřeší.
- Číslo má standardní JSON zápis: volitelné `-`, celá část `0` nebo nenulová číslice a další číslice, volitelná desetinná část a exponent. Plus na začátku, úvodní nuly, `1.`, `1e`, `1..2` nebo `10^9` nejsou čísla. Mocnina se zapisuje `[pow 10 9]`.
- Řetězce používají přesnou syntaxi JSON: `"..."`, escapes `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, `\uXXXX`. Parser dekóduje surrogate pairs a odmítá osamocený surrogate. Unicode lze zapsat přímo jako UTF-8.
- Sousední bare tokeny nebo řetězce vyžadují oddělovač. Hranatá závorka i komentář jsou oddělovače, například `[a[b]"c"]` a `[][x]` jsou platné. `[a"b"]`, `["a"b]` a `["a""b"]` jsou neplatné.
- Platné UTF-8 se vyžaduje i uvnitř komentářů. Raw řídicí znaky U+0000–U+001F uvnitř řetězců musí být escaped. UTF-8 BOM nemá zvláštní význam a není whitespace.

```ebnf
number   = [ "-" ], integer, [ ".", digits ], [ exponent ] ;
integer  = "0" | nonzero, { digit } ;
exponent = ( "e" | "E" ), [ "+" | "-" ], digits ;
digits   = digit, { digit } ;
```

## AST

Každý uzel má stejné tři klíče:

```elixir
%{
  kind: :document | :list | :symbol | :number | :string,
  value: list_of_nodes_or_text,
  span: %{
    start: %{offset: byte_offset, line: line_number, column: column_number},
    end: %{offset: byte_offset, line: line_number, column: column_number}
  }
}
```

`:document` obsahuje deklarace, `:list` obsahuje svoje děti. Symbol je původní text, řetězec dekódovaný text, číslo přesné původní znění. `0.01`, `2490.00` nebo `1e1000000000` se nikdy nepřevádějí na float ani strojový integer.

Pozice jsou half-open: konec je první pozice za uzlem. Offset začíná nulou a počítá UTF-8 byty. Řádek a sloupec začínají jedničkou; sloupec počítá Unicode code points, tab je jeden code point. Span seznamu zahrnuje jeho závorky, span stringu uvozovky, span dokumentu celý vstup včetně trivia.

AST zachovává pořadí, prázdné seznamy a duplicity. Nepřevádí pole ani deklarace do slovníků; takový převod by ztratil část vstupu a předčasně řešil jeho význam. Neobsahuje Elixir AST, closures ani vyhodnocené výrazy. Jediné atomy jsou pevně dané klíče a druhy uzlů.

## Chyby a limity

`Flow.Parser.parse(source, options)` vrací `{:ok, ast}` nebo `{:error, %Flow.ParseError{}}`. Chyba obsahuje stabilní `code`, `message`, `file`, `offset`, `line`, `column` a případně `opened_at` pro nedokončenou strukturu. API nepohlcuje jiné programátorské chyby.

Kódy: `byte_limit`, `depth_limit`, `node_limit`, `expected_list`, `unexpected_close`, `unclosed_list`, `missing_separator`, `unexpected_character`, `invalid_utf8`, `invalid_number`, `unclosed_string`, `invalid_string`, `invalid_escape`.

Výchozí limity jsou 4 MiB vstupu, 256 úrovní seznamů a 100 000 uzlů. Kořenový dokument se do limitu uzlů nepočítá. Každý otevřený seznam a scalar se počítá jednou. Limity jsou nastavitelné kladnými integers. Omezují práci a alokace i při nevalidním nebo nedokončeném zdroji.

Parser čte vstup dopředu, používá explicitní zásobník seznamů a akumuluje děti obráceně s jedním otočením při uzavření. Nemá zpětné prohledávání gramatiky ani opakované připojování k rostoucím seznamům. Základní algoritmus je přenosný do jiných jazyků: scanner hodnot, zásobník otevřených seznamů a stejný AST.
