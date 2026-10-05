# Flow — deklarativní aplikace

Elixir parser a rozpracovaný validátor/runtime. Parser zůstává nezávislý na doméně. Runtime provádí deklarativní operace nad SQLite, ověřuje credentials a autorizuje celou navrženou transakci před zápisem. Objednávky, sklad, příkazy zařízení, resize/uložení fotografií a šifrovaná fronta s opakovaným ověřováním práv mají integrační testy. Eventové automaty a HTTP/MQTT/WebSocket server jsou ještě rozpracované; aplikaci zatím nelze spustit jako hotový síťový server.

Formát používá jedinou strukturu `[ ... ]`. Parser rozpoznává seznamy, symboly, přesná číselná znění a JSON řetězce. Nezná doménové keywords a nic nevykonává. Podrobnou gramatiku a AST popisuje [LANGUAGE.md](docs/LANGUAGE.md).

```flow
[type Price [decimal [range 0.01 [pow 10 9]] [scale 2]]]

[entity Post [field id PostID] [field author User] [field public Bool]]

[permissions User
  [Post
    [READ [when target.public]]
    [UPDATE [includes READ] [when [eq target.author actor]]]]]

[query UserOrders
  [input userId UserID]
  [output [list OrderSummary [max 10]]]
  [http GET "/users/{userId}/orders"]
  [user [entity $userId]]
  [result [order [last user.orders 10] Order.createdAt DESC]]]
```

Celá deklarace aplikace je v [application.flow](priv/workflows/application.flow); další permission scénáře jsou v [permissions.flow](../examples/permissions.flow). Oba soubory procházejí parserem v testech. `Flow.Program.compile/2` kontroluje typy vstupů/výstupů, výrazy operací i permission predikátů, cykly vazeb, HTTP trasy a kontrakty pluginů. Rekurzivní permission závislosti musí být pozitivní; záporné cykly se odmítnou.

## Implementované části

- `Flow.Schema` kontroluje deklarace typů, entity, brandy ID, číselné rozsahy a vazby. Uložené a vstupní/výstupní seznamy vyžadují `max`; relační seznam se omezuje až ve výrazu operace.
- `Flow.Value` validuje konkrétní vstupy, včetně rozsahů, délky seznamů, neznámých polí a přesné desetinné aritmetiky. JSON se dekóduje pomocí `Jason.decode!(json, floats: :decimals)`.
- `Flow.Auth` ověřuje JWT (konfigurované HS256/RS256), API klíče a certifikáty získané důvěryhodným TLS adaptérem. Ověřený údaj váže na unikátní pole identity. Transport určuje povolené adaptéry; neplatné credentials nezískají anonymní identitu.
- `Flow.Permissions` kompiluje pozitivní granty a explicitní dědičnost akcí. Právo na pole vyžaduje také právo na entitu. Cyklická dědičnost a zděděné čtení závislé na navrhované změně jsou odmítnuty.
- `Flow.Expression` vyhodnocuje čisté podmínky s rozpočtem kroků sdíleným i při odkazu na další permissions. Neobsahuje spouštění Elixir kódu.
- `Flow.Store` serializuje SQLite transakce, zapíná cizí klíče a ukládá čísla bez převodu na floating point. Jeho nízkoúrovňové API je určeno důvěryhodnému hostiteli, samo neautorizuje zápisy.
- `Flow.Access` a `Flow.Projection` vynucují práva při čtení a vybírají pouze výstupní pole a závislosti permissions. Každý požadavek má vlastní cache; nová session zohlední aktuální práva. Seznamy se filtrují před limitem.
- `Flow.Transaction` autorizuje všechny vytvořené, změněné a smazané entity i všechna dotčená pole. Identita a její vztahy se při autorizaci čtou z původního stavu; `after` a `creates transaction` zpřístupňují navržený stav. Návrhy se do SQLite zapíšou teprve po autorizaci a ověření výsledné projekce.
- `Flow.Evaluator` memoizuje deklarativní vazby, včetně dopředných referencí a lokálních vazeb uvnitř `map`. Operace nemají imperativní resolver.
- `Flow.Plugins` poskytuje nativní kontrakty pro čistý výpočet platebního URL a resize obrázku, transakční uložení souboru a ukázkovou externí emailovou službu. Ukázkový email zatím žádnou zprávu neposílá. Externí účinky se musí zařadit pomocí `enqueue`; potvrzení má typ `QueueReceipt`, nikoliv budoucí výsledek pluginu.
- `Flow.Queue` atomicky ukládá šifrované úlohy spolu s transakcí. Worker znovu autentizuje identitu a ověřuje aktuální práva před každým pokusem. Předá pluginu stabilní `idempotency_key`, pokud jeho hostitelský callback přijímá dva argumenty. Chyby se opakují podle deklarovaného retry a poté zůstávají ve stavu `RETAINED`; ztráta identity či práv přejde do `BLOCKED`. Persistentní runtime vyžaduje stabilní 32bytový `queue_key`. Worker běží automaticky; pro testy lze nastavit `worker_interval: :manual` a volat `Flow.Runtime.process_jobs/1`.
- `Flow.Context` validuje externí fakta dodaná důvěryhodným hostitelem podle typu `Context`, pokud je deklarován. Klient nemůže předat context, actor, MFA ani delegaci přes parametry runtime. Bez vlastního typu obsahuje context pouze aktuální čas.

## Použití

Vyžaduje Elixir 1.18+ a odpovídající Erlang/OTP. Závislosti pro SQLite, HTTP/WebSocket a obrázky jsou uzamčené v `mix.lock`:

```sh
cd prototype
mix deps.get
mix test
mix run -e 'source = File.read!("priv/workflows/application.flow"); IO.inspect(Flow.Parser.parse(source, file: "application.flow"))'
```

```elixir
{:ok, ast} = Flow.Parser.parse("[entity User [field id UserID]]")
{:error, error} = Flow.Parser.parse("[entity User")

error.code       # :unclosed_list
error.line       # 1
error.column     # 13
error.offset     # 12, od nuly v bytech
error.opened_at  # %{offset: 0, line: 1, column: 1}
```

`parse!/2` vrací AST nebo vyvolá `Flow.ParseError`. `parse/2` vrací strukturované syntaktické chyby; neplatné argumenty API vyvolávají `ArgumentError`.

Parser má nastavitelné limity `max_bytes`, `max_depth`, `max_nodes` a volbu `file`. Nepřevádí vstupní identifikátory na Elixir atomy a čísla nezaokrouhluje. Zanoření používá explicitní zásobník, nikoliv rekurzivní sestup přes úrovně seznamů.

## Ověření

```sh
mix format --check-formatted
mix compile --warnings-as-errors
mix test --warnings-as-errors
```

Testy ověřují AST, přesné pozice, JSON escapes včetně surrogate pairs, nevalidní UTF-8, diagnostiku, hranice limitů, velké a hluboké vstupy, zachování přesnosti čísel a absenci atomů ze vstupu. Zahrnují také generované stromy, náhodné byte vstupy a kompletní aplikační příklady.
