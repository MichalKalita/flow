# Flow — deklarativní aplikace

Elixir parser a rozpracovaný validátor/runtime. Parser zůstává nezávislý na doméně. Nové moduly doplňují datový model, přesné hodnoty, autentizaci, permissions a líné čtení nad SQLite. Kompletní provádění operací, transakční autorizace zápisů, pluginy a HTTP/MQTT/WebSocket server jsou ještě rozpracované; aplikaci zatím nelze spustit jako hotový server.

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

Celá deklarace aplikace je v [application.flow](priv/workflows/application.flow); další permission scénáře jsou v [permissions.flow](../examples/permissions.flow). Oba soubory procházejí parserem v testech. Datový model celé aplikace také prochází `Flow.Schema.compile/1`. Toto API zatím nekontroluje typy výrazů operací a není zárukou spustitelnosti programu.

## Implementované části

- `Flow.Schema` kontroluje deklarace typů, entity, brandy ID, číselné rozsahy a vazby. Uložené a vstupní/výstupní seznamy vyžadují `max`; relační seznam se omezuje až ve výrazu operace.
- `Flow.Value` validuje konkrétní vstupy, včetně rozsahů, délky seznamů, neznámých polí a přesné desetinné aritmetiky. JSON se dekóduje pomocí `Jason.decode!(json, floats: :decimals)`.
- `Flow.Auth` ověřuje JWT (konfigurované HS256/RS256), API klíče a certifikáty získané důvěryhodným TLS adaptérem. Ověřený údaj váže na unikátní pole identity. Transport určuje povolené adaptéry; neplatné credentials nezískají anonymní identitu.
- `Flow.Permissions` kompiluje pozitivní granty a explicitní dědičnost akcí. Právo na pole vyžaduje také právo na entitu. Cyklická dědičnost a zděděné čtení závislé na navrhované změně jsou odmítnuty.
- `Flow.Expression` vyhodnocuje čisté podmínky s rozpočtem kroků sdíleným i při odkazu na další permissions. Neobsahuje spouštění Elixir kódu.
- `Flow.Store` serializuje SQLite transakce, zapíná cizí klíče a ukládá čísla bez převodu na floating point. Jeho nízkoúrovňové API je určeno důvěryhodnému hostiteli, samo neautorizuje zápisy.
- `Flow.Access` a `Flow.Projection` vynucují práva při čtení a vybírají pouze výstupní pole a závislosti permissions. Každý požadavek má vlastní cache; nová session zohlední aktuální práva. Seznamy se filtrují před limitem.

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
