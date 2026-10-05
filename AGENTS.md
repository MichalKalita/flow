# Pravidla práce v projektu Flow

- Runtime je v `runtime/`, implementovaný v Rustu. Zachovej centrální permissions a atomické transakce pro všechny transporty.
- Před změnou prohlédni relevantní implementaci a testy. Změny chování doprovázej odpovídajícími integračními testy a dokumentací.
- Audit každé potvrzené mutace patří do SQLite a do stejné transakce jako data. Rollback nesmí zanechat audit potvrzené změny.
- Provozní logy a metriky ukládej mimo aplikační databázi. Neloguj credentials, tokeny, request body ani pluginové argumenty.
- Admin rozhraní odděluj od veřejného listeneru a chraň přístup. Konzole musí používat běžnou autentizaci a permissions aplikace.
- Před dokončením spusť v `runtime/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Průběžně commituj ucelené části práce s konkrétním popisem změny. Nezahrnuj cizí změny ani generované databáze, logy, secrets a build výstupy.
- Uživateli průběžně česky sděluj podstatné výsledky a omezení.
