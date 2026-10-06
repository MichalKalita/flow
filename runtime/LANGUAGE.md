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

Podporu doménových konstrukcí a omezení aktuální HTTP verze popisuje [README.md](README.md). Ukázkové aplikace jsou hostované projekty v [projects/](../projects/). Zvýraznění syntaxe pro VS Code je v [editors/vscode](../editors/vscode).

## Numeric identity values

`[type UserID [id User]]` declares a branded numeric identity. Seeds use `[id 1]`,
references use integer values such as `[owner 1]`, and JSON clients send
`{"userId":1}`. Quoted, fractional, zero, negative and unsafe integer IDs are
rejected. `[new UserID]` reserves the next transactional ID for that entity.
Entity brands are part of runtime values, not encoded into prefixes in the ID.

## Bounded text, pagination, and edit conflicts

`[string [minBytes 1] [maxBytes 200]]` constrains UTF-8 byte length. Bounds are checked for inputs and stored fields. The maximum supported string bound is 1 MiB.

`[page Contact $after 50]` reads records ordered by positive numeric entity ID. The cursor is a matching branded ID or null; the page size is between 1 and 1000. Central READ permissions filter records before they count toward the page. Work remains bounded; extremely sparse permitted results can reach the execution limit. Staged creates participate in ID order.

`[expectVersion [entity $contactId] $version]` requires normal READ permission and compares the stored numeric `version` field before a mutation. A mismatch returns `conflict` (HTTP 409) and rolls back the transaction, including audit. Applications must increment the version under their normal UPDATE rules; the expression does not grant mutation permission.

## Production and test seeds

A seed defaults to production/bootstrap data. Use `[seed Item [group production] [rows ...]]` explicitly, or `[seed Item [group test] [rows ...]]` for optional test/demo data. Both groups are compiled and type-checked; duplicate entity IDs across declarations/groups are rejected.

Test seeds are disabled by default. A hosted project's `project.json` can explicitly enable them with `"test_seeds": true`; the value must be a boolean and affects only that project. Runtime callers use `Config.test_seeds`. Existing ungrouped seeds retain production behavior. Do not put demo credentials in an ungrouped production seed.

Selected seeds insert once with data and audit in the same transaction. Deleted seeded rows are not recreated on reload/restart. A newly declared seed colliding with an existing untracked entity ID rejects the candidate instead of overwriting user data. Changing an already applied seed does not update its row; intentional updates require a migration. Disabling test seeds does not delete previously inserted test data or reset committed numeric sequence boundaries. Seeds do not dispatch business automations.

## Versioned field rename migrations

Programs default to schema version 1. Declare a positive integer `[schema 2]` to advance the schema with an explicit contiguous migration chain:

```flow
[schema 2]
[migration RenamePhone
  [from 1] [to 2]
  [rename Contact phone telephone]]
```

The target program declares and uses `telephone`. This initial implementation supports stored-field renames with unchanged logical types/constraints and explicit no-op version steps. It does not support renaming entity IDs, dropping entities/fields, arbitrary SQL, or type/data transformations. Migration identifiers are bounded ASCII names. Definitions have a stable checksum; recorded definitions cannot be edited or removed. Each step advances one version, with at most 256 declarations/steps in a chain and 128 renames per declaration.

Existing installations run the selected renames, complete schema/seed changes, logical data/reference validation, migration history, and active program publication in one SQLite transaction. Failure rolls back the generation. Already applied steps do not run again. A fresh installation builds the final schema directly and records historical definitions as `applied: false`. Downgrading requires an explicit recovery procedure. Ordinary permissions and transactional audit remain in force for subsequent application writes. Historical audit field names are retained and their values decode normally.

Activation uses the project lock and a five-second pause budget with SQLite interruption and rollback. Long preparation/reconciliation for transformations that exceed this budget remains planned. Keep a verified whole-server backup before changing deployed programs; this initial rename slice does not automatically create a migration recovery backup.
