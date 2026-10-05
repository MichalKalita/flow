# Flow: jednotný hranatý zápis

Program je posloupnost forem `[název argumenty ...]`. Stejnou strukturu mají deklarace, výrazy, seznamy a záznamy. Řádky a odsazení nemají význam. `#` mimo řetězec zahajuje komentář do konce řádku.

```flow
[type UserID [id User]]
[type Price [decimal [range 0.01 [pow 10 9]] [scale 2]]]
[permissions User
  [Order [READ [when [eq actor target.user]]]]]
[query Orders
  [input userId UserID]
  [output [list OrderSummary [max 10]]]
  [http GET "/users/{userId}/orders"]
  [user [entity $userId]]
  [result [order [last user.orders 10] Order.createdAt DESC]]]
```

Výrazy jsou prefixové: `[add a b]`, `[eq a b]`, `[and podmínka podmínka]`. `actor`, `$userId` a `user.orders` jsou symboly, jejich význam řeší až program a evaluator. JSON řetězce podporují Unicode a escapes včetně surrogate pairs. Čísla se zapisují podle JSON syntaxe, uchovávají se jako původní text a vyhodnocují přesně. Mocniny používají `[pow 10 9]`.

```ebnf
document = { list }, EOF ;
list     = "[", { value }, "]" ;
value    = list | symbol | number | string ;
```

Tokeny odděluje whitespace, závorky nebo komentář; sousední řetězec a symbol vyžadují oddělovač. Parser přijímá prázdné seznamy a neznámé keywords, jejich významová platnost se kontroluje později. Neprovádí SQL ani nevolá externí kód.

Rust API `syntax::parse(&str)` vrací `Result<Vec<Node>>`. AST obsahuje pouze `Node::List`, `Node::Symbol`, `Node::String` a `Node::Number`. Zachovává pořadí a duplicity. Neobsahuje closures ani Rust AST; runtime nezavádí vlastní kód přes `eval`. Chyby používají `Error { code, message }`; některé syntaktické chyby uvádějí byte offset, úplné source spans zatím nejsou implementované.

Parser používá explicitní zásobník. Limity jsou 4 MiB zdroje, 256 úrovní a 100 000 průchodů scanneru. Soubor se musí načíst jako UTF-8. Runtime má samostatný rozpočet 100 000 vyhodnocovacích kroků v požadavku. Přesná čísla mají omezenou délku a exponent; dělení s nekonečným desetinným rozvojem se odmítne, nezaokrouhluje se potichu.

Podporu doménových konstrukcí a omezení aktuální HTTP verze popisuje [README.md](README.md). Plný původní návrh je v [examples/application.flow](../examples/application.flow). Zvýraznění syntaxe pro VS Code je v [editors/vscode](../editors/vscode).

## Numeric identity values

`[type UserID [id User]]` declares a branded numeric identity. Seeds use `[id 1]`,
references use integer values such as `[owner 1]`, and JSON clients send
`{"userId":1}`. Quoted, fractional, zero, negative and unsafe integer IDs are
rejected. `[new UserID]` reserves the next transactional ID for that entity.
Entity brands are part of runtime values, not encoded into prefixes in the ID.
