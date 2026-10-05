# Flow — parser deklarativní aplikace

Samostatný Elixir parser bez závislostí. Původní aplikační prototyp byl nahrazen; HTTP server, databáze, interpret, typová kontrola ani permissions runtime nejsou součástí tohoto projektu.

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

Celá deklarace aplikace je v [application.flow](priv/workflows/application.flow); další permission scénáře jsou v [permissions.flow](../examples/permissions.flow). Oba soubory procházejí skutečným parserem v testech. Jejich doménový význam je návrh pro budoucí validátor, nikoliv implementovaná funkcionalita.

## Použití

Vyžaduje Elixir 1.14+ a odpovídající Erlang/OTP. Žádné `mix deps.get` ani síťové služby:

```sh
cd prototype
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
