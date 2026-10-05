# Ověření implementace Flow

Tento přehled ověřuje požadovaný Elixir prototyp na jednom stroji: skutečný jazyk pro aplikační scénáře, objednávku, pluginy, MQTT, autorizovaný WebSocket, admin a dokumentaci. Celý BLACKBOX návrh platformy obsahuje také další požadavky; tento prototyp není jeho hotovou implementací. Replikace a Lean byly pro tuto implementaci výslovně odloženy.

| Požadavek | Implementace | Ověření |
|---|---|---|
| Obecný jazyk místo předem napsané objednávkové funkce | Lexer, parser, compiler, checker, evaluator a runtime v `lib/order_lab/language/`; AST interpretuje obecné příkazy | E2E rozšíří jediný zdroj o Notes, Reviews, Contacts, filtry, JOIN, iterace a CRUD bez změny Elixiru |
| Jediný aplikační soubor | `priv/workflows/application.flow`; `Workflow.compile!/0` načítá jediný `FLOW_PATH` | E2E načítají jednu rozšířenou aplikaci a samostatnou prázdnou aplikaci bez demo dat |
| Deklarace kontraktů, bez vlastních funkcí | TYPE, FILTER, TABLE, SEED, HTTP, MQTT, ON MQTT, WEBSOCKET; registrované operace | Kontrola zdroje přes HTTP odmítá neznámé operace, pole, neplatné typy a účinky |
| Rozsahy a strukturované vstupy | Refinement typy, záznamy, seznamy, optional, omezení hodnot a vypočtených vazeb | Rating 1–5, množství, rozměry obrázků, MQTT souřadnice, konstanty a vypočtené neplatné hodnoty |
| Dotazy a podmíněné scénáře | FROM, WHERE, ORDER BY, LIMIT, SELECT, ONE/LAST, INNER/LEFT/WHEN JOIN, EXISTS/ALL/SUM, FOR EACH, WHEN/ELSE, IF výraz | Obecné CRUD, podmíněné filtry, korelované JOIN, agregace, ALL/NOT EXISTS nad prázdnou i neprázdnou kolekcí a výsledky iterací |
| Objednávka | Cenový snapshot, agregace košíku, kontrola a odečet skladu, platební URL, transakční emailová fronta, odpověď | Chrome odešle objednávku; HTTP odpověď souhlasí s uloženými položkami, cenou a platebním odkazem; souběh nepřeprodá sklad |
| Elixir pluginy pro platbu a falešný email | `Plugins.Payment`, `Plugins.Email`, typované kontrakty v `Native.operations/0` | Vstupy a výsledky jsou v adminu; odmítnutá země/metoda vrátí objednávku i sklad; email skutečně prochází workerem |
| Rollback a zachycení chyb | TRANSACTION/COMMIT, TRY/CATCH se savepointem; typované chyby SQL | Konflikty klíčů, reference, CHECK, rollback celého bloku i savepointu a pokračování po zachycení chyby |
| Obnova a externí účinky | Trvalé checkpointy, stabilní UUID/časy, deník CALL i QUEUE, kontrola zdrojových dat, kompenzační záměry a potvrzovací markery | SIGKILL před/po commitu i externím výsledku, ztracená odpověď, jedna logická platba/zrušení, konfigurace poskytovatele, ruční obnova v Chrome |
| Typované MQTT zdroje a historie | Pojmenované zdroje s PARAM/PAYLOAD/HISTORY; příjem, ON MQTT a transakční PUBLISH | Skutečná TCP spojení, QoS 1, DUP, retained, historie pěti minut, HTTP výběr posledního stavu, rollback a restart outboxu |
| WebSocket kopie a autorizace zařízení | WEBSOCKET INPUT/AUTHORIZE a SOURCE WHERE; kontrola před odběrem i každým doručením | Více skutečných klientů a Chrome, izolace zařízení, odmítnuté tokeny, odebrání jednoho i všech oprávnění, unsubscribe |
| Soubor jako omezený typ | File/Image, ověření bajtů, Image.decode/resize, transakční Files.put/read/delete | Skutečné PNG/JPEG a Chrome dekódování; PDF, falešné rozměry a vadné bajty odmítnuty; rollback obsahu i záznamu |
| Admin a dohledatelnost | Požadavky, vstupy/výstupy pluginů, fronty, MQTT, externí deník, kompenzace a původní rodič | E2E procházejí Chrome panel, detail požadavku, retry emailu, obnovu požadavku, úlohy i zrušení |
| Pouze E2E testy | `e2e/order-lab.spec.js`, skutečný server, SQLite, síť a Chrome; žádné unit testy | Kompletní sada `npm test`; kompilace `mix compile --warnings-as-errors` |
| Dokumentace skutečné syntaxe | [LANGUAGE.md](LANGUAGE.md), [README](../README.md), samotný `application.flow` | Dokumentované konstrukce odpovídají parseru, kontraktům a používaným E2E scénářům |

## Výsledek ověření

Na této verzi proběhla kompletní sada `cd prototype/e2e && npm test`: 69 E2E prošlo. `cd prototype && mix compile --warnings-as-errors` skončilo úspěšně bez varování. Žádné unit testy nebyly přidány. Testy obsahují i přímé porovnání odpovědi objednávky s uloženým stavem, znovunačtení jediného Flow souboru a skutečné pády/restarty Elixiru.

## Hranice garancí

Jazyk je implementovaný prototyp, nikoliv Lean ani důkaz bezchybnosti libovolného programu. Typová kontrola a validace chrání uvedené kontrakty; správnost zvolených obchodních pravidel zůstává odpovědností aplikace.

Externí právě-jednou účinek a zrušení závisejí na idempotentním kontraktu poskytovatele. Databázový rollback je okamžitý, kompenzace následná. Simulovaný email nepředstírá doručení skutečnou službou. Nejasné nekompenzovatelné účinky se nepovažují za vyřešené jen kvůli retry.

Prototyp nemá replikaci, hot updates, migraci rozpracovaných scénářů, správu životního cyklu deníku ani celou HTTPS/secrets/backup/KV platformu z BLACKBOX návrhu. Obecné tabulky mají unikátní neměnné id; libovolné deklarativní FOREIGN KEY/UNIQUE nejsou součástí aktuální syntaxe. MQTT a admin jsou lokální služby podle README. Podrobné kontrakty, omezení a příklady uvádí LANGUAGE.md.
