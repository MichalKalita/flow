# TODO

This is the only product TODO file. Source text is the user's original wording, not an AI rewrite. Original notes come from Git commit `bd66fe0`. Later quotations come from the conversation. Czech and spelling mistakes are deliberately preserved inside source quotations; editorial text is English.

Implementation status is separate from the source. “Partial” means a working slice exists, not that the whole requirement is complete. Uncommitted work is not listed as delivered. Read sections in dependency order. Detailed shipped APIs belong in [runtime documentation](runtime/README.md), [language documentation](runtime/LANGUAGE.md), and [project documentation](projects/README.md).

## Delivery record

| Commit | Delivered slice |
| --- | --- |
| `148fc6b` | Persistent generated keys, encrypted settings, owner setup and password login |
| `4910cb5` | Local whole-server backup/restore, one-click Contacts and an initial application frontend |
| `5569aed` | Server-owned logs, live retention/chunk limits, oldest age and filtered histograms |
| `7a285f4` | Passive hardware/disk inventory and reviewed storage suggestions |
| `02a0486` | Validated live project secret replacement |
| `1ffe2e5` | Production/bootstrap and opt-in project test seeds |
| `28d3371` | Program/source manifest committed atomically with schema activation |
| `30b2a6f` | Initial versioned field rename migrations with rollback and bounded activation |

Latest committed implementation passed Rust formatting, Clippy and the full test suite, plus nine real-runtime Playwright tests. Minimum-host production capacity, full disaster recovery and linear scaling are not verified.

Next implementation work: disk/headroom preflight and automatic recoverable capture before migration; then finish longer migration preparation/reconciliation, HTTPS and the universal durable event lifecycle. HA, remote storage and complete mail remain outstanding. This order is implementation tracking, not a quotation or a new user requirement.

## 1. Product purpose and operating defaults

Source: later user messages, verbatim.

> nechapu otazku, proste logicke poradi od nejdulezitejsich aby ten systam samotny fungoval

> myslenka je takova, ze ucel tohohle je jednoduchost a ne performance ala k8s, tohle nemuze konkurovat k8s, takze servery jsou vzdy stejne ze na nich bezi stejny software a delaji si uplnou replikaci, tzn cela db je na vsech serverech, a vsechny servery sdili vsechny projekty, to tak bude proste aby to bylo jednoduche

> ucelem v zadnem pripade neni stavet system jako k8s pro tisice serveru, tohle ma ucet udelat spolehlivy software na levnych serverech pro vsechny firmy

> primarni ucel neni vykon toho serveru, ale spolehlivost, a neprijit o zadna data, proto ta uplna replikace

> tohle vsechno nastaveni bude mit jednak nejaky default, a taky bude ten pruvodce jak to nastavit, a ten poradi jak nastavit nejake vhodne hodnoty

> automaticke prevzeti, dalsi priorita je, aby ten server mel dobry default, tzn bude tam bude pruvodce co to pomuze nastavit a zbytek hodnot by mel byt proste dobrych by default

> ve zkratce je to system pro blbecky, a to myslim presne tak jak to pisu, k8s je pro experty, a tohle je pro blbecky proste

> btw aktualizuj readme, at je to proste normalni popis, nechci high tech k8s popis, chci to based jak svina, tohle je software, ktery clovek spusti na alpine linuxu a 20 let nemusi nic resit

> ne uprav ten popis, pis to v jednoduchych vetach, zadne odstavce tam nesmi byt
> je to pro individualni lidi, male a stredni firmy
> netrpi na overprovisioning, a jde spustit na zakladnim serveru za $12 (1 shared CPU a 2GB ram)
> a stejne jde pustit na 32 cpu a 128GB ram, a skaluje se linearne

Implementation status: Partial. A standalone setup wizard, persistent server identity, host-derived worker/admission settings and passive storage guidance exist. Alpine deployment, long-term operation, minimum-host acceptance and linear scaling still need verification.

## 2. Variables, secrets and server identity

Original note: `.todo/variables-and-secrets.md` at `bd66fe0`.

```text
# Variables and secrets

It must be possible to use variables and secrets for setup projects

Secrets can be used in way, where something like [secret TOKEN [default [randomString [length 20]]]]
Where default value will be saved. 

Secrets must be saved in safe way, not just texts.
Defaults can be used for generate 

Project settings can be changed in running app, user dont need connects to ssh.
```

Source: later user messages, verbatim.

> prozatim bych to neprehanel uplne, takze by melo jit zobrazit aspon cast tech tajnych veci, jako par prvnich a poslednich znaku aspon

> a jeste k tem klicum, ten server by si mel vytvorit vlastni sifrovaci klice i na startu, a kdyz bude vice serveru pres HA, pak se bude muset u serveru nastavit jaka je adresa druheho a verejny klic druheho, a na druhem verejny klic prvniho, tim se spoji

> kazdy server by se mel nejak jmenovat, aby byl jasne oznacitelny, nevim jestli hash nejakeho klice, nebo jak, ale v logu i vsude musi byt uvedeno jaky to byl server, protoze tech serveru do te s3 bude logovat vice

Earlier answer, superseded by the later first-start key requirement above:

> Ano, hlavní klíč při nasazení, projektová tajemství přes administraci.

Implementation status: Partial. Generated persistent keys/identity, encrypted settings, project secret references, masked previews and validated live JWT/event secret replacement are delivered. General typed variables and the full rotation/deletion lifecycle remain.

## 3. Server logs

Original note: `.todo/runtime logs.md` at `bd66fe0`.

```text
# Runtime logs

It should be possible to limit size of logs for server, not projects.

Admin interface must have search for logs and histogram. Show if log is info/verbose/....

It must be possible to setup log verbose per project.

All this settings can be changed in admin settings.
```

Source: later user messages, verbatim.

> maji se prepisovat, tzn smaze se cast starych, nemusi to byt uplne atomicke, tzn na par vterin ten limit muze byt prekrocen, ale proste se ma cilovat ten limit, takze kdyz bude limit 1GB, tak dlouhodobe tam bude 1GB, a chci aby v administraci slo videt jak vlastne stare jsou ty nejstarsi logy
> a chci umoznit funkci, kdy se stare logy posilaji na s3 treba, v ramci nastaveni toho serveru jako celku, vsechny ty logy jsou per server, ne per project

> to posilani do S3 by melo mit nejaky druhy parametr, a to treba ze v nejake casti se to uz odesle do S3, asi o dost drive nez na konci, teoreticky kdyz budou treba 50MB chunky, nebo nevim, tak v momente kdy existuje ten chunk, tak se muze hned odeslat

> a jeste kazdy server i v HA loguje jenom za sebe, takze kazdy bude posilat logy za sebe

> stejne tak ty logy v tom filtrovacim nastroji by mely cist data z s3, kdyz ten log uz neni lokalne

> kdyz se v replikovanem systemu nekdo zepta na logy, tak kazdy server vrati logy za sebe a pak se jeden z nich (asi ten na ktery prisel request) podiva na s3 a sestavi to jako odpoved a vrati to jako celek

Implementation status: Partial. One server writer, project/server tags, bounded local chunks, byte-target cleanup, live limits, oldest timestamp/age and filtered search/histograms are delivered. Per-project verbosity, legacy-history migration, remote export/search and peer aggregation remain.

## 4. Capacity and setup guidance

Original note: `.todo/benchmark.md` at `bd66fe0`.

```text
# Benchmark

System itself needs benchmark, to be able detect best system setup
It needs check performance for CPU, RAM and storage. And find what is a limit.

Find best value for limit of parallell requests, find how much empty space is on disc. How much space logs can use.

It should be some guide in admin panel, to find best settings.
```

Implementation status: Partial. CPU/memory/disk inventory, current database/WAL sizes, headroom notices, first-start log defaults and reviewed suggestions are delivered. Bounded active calibration and representative small/large-host acceptance remain.

## 5. Backup and restoration

Source: later user messages, verbatim.

> taky musi jit nastavit zalohovani, a to aby to ze zalohy slo i obnovit

> to logy atd by melo jit backupovat i na disk treba, jako remote disk, smb a spol, proste klasika vsechny standardni protokoly

Implementation status: Partial. Local whole-server capture, signed manifests, password-encrypted recovery keys, isolated restore validation, offline recovery and journaled online restoration are delivered. Scheduling, standard remote destinations, continuous off-server recovery and HA restoration remain.

## 6. Migrations and seeds

Original note: `.todo/migrations.md` at `bd66fe0`.

```text
# Migrations


We need some way how to migrate data when structure is changed, it must be completed before app launches itself, but previous version should be running, maybe not using migrated content in that time, its ok


We need ability to migrate entities to other projects, entity will be copied to other project and some way labeled from where it should be migrated, and target project will copy all data.

## Seeds

Ability to mark what is test data, what is production data, and prevent write data twice.


I am not sure here: Fix ids, seeds should be possible to run event when ids are there? Maybe its bad behavior.
```

Source: later user messages, verbatim.

> Ano, krátká pauza zápisů je přijatelná.

Implementation status: Partial. Compatible reloads, production/test seed groups, once-only seed ledgers, atomic program generations and initial immutable versioned field renames are delivered. Automatic recovery capture, longer background preparation/reconciliation, general type/data transformations and cross-project transfer remain.

## 7. HTTPS and domain routing

Original note: `.todo/https.md` at `bd66fe0`.

```text
# Routing

Server must get certificates for https, from lets encrypt.

Project endpoints must have ability to mark which domain it should run.


Projects can have apis in same domains, like auth service can work on example.com/auth
And ecommerce can work on example.com
It must be allowed.
```

Implementation status: Not implemented. Canonical project paths and a separate protected admin listener exist; built-in TLS/ACME and domain/path aliases remain.

## 8. Universal events and external integrations

Original note: `.todo/external-integrations.md` at `bd66fe0`.

```text
# External integrations

This system should allow generic external integration

Core should be able to use it, plan with it.

External services should have exact behavior how its handled, like commit and rollback. In external services is not possible to rollback by definition, find some generic ways how to solve it.
I want to find a way to work how standards works.

Write list of real life needed external services, S3, REST API, MQTT, SMTP, ....

It should be possible to write a rest endpoint, where only external data will be used, validation MUST be there in some way, find way how to implement it.
```

Source: later user messages, verbatim.

> chci mit uplne kompletni email server zabudovany, ale predstavuji si to spis tak, ze to bude jeden z mnoha projektu, takze pujde nastavit co se uklada u emailu, a co se ma stat kdyz email prijde, obecne by to chtelo mit nejaky mechanismus jako kdyz neco tak neco, nevim jestli uz to je v projektu nebo ne

> tam musi byt nejake api zakladni emailove, a to bude mit prijem emailove zpravy, a nebude to jako rest endpoint, ale jako event vstup, takze se to spusti az kdyz uz se to ulozi na vstupu, z principu toho jak funguje tenhle projekt jako celek, aby se spustil event nad udalosti, tak ta udalost musi nastat a byt prijata, ale urcite musi jit nastavit treba velikost jednoho emailu atd atd

> to je jako kazdy jiny event kde selze mechanismus, ano, i tento mechanismus jako uplne vsechny v systemu muze mit nastavene co se stane pri selhani, nejaky limit opakovani a potom ulozeni nebo smazani

> chci aby prijem emailu byl uplne stejny event jako doslova vsechny jine v systemu

> jenom ten zdroj bude pres nejaky "plugin" nebo jak to pojmenovat a ten bude samozrejme delat ty emailove veci, overovani atd

Implementation status: Not implemented as requested. Native plugins and synchronous stream automations exist. Durable acceptance before dispatch, common retries/retain-or-delete, generic connectors, external effects and external-data-only endpoints remain.

## 9. Full-server HA

Source: later user messages, verbatim.

> hlavni a zalozni, a taky ty servery na sebe nemusi nutne videt, tzn jeden musi mit nejaky verejny pristup, ne oba

Implementation status: Not implemented. Each server has its own persisted identity/key material. Pairing, full replication, durability acknowledgement, safe automatic takeover and public-ingress failover remain. Logging/archive ownership stays per server.

The source for full replication, reciprocal public-key pairing, automatic takeover and reliability priority is also preserved in sections 1–3.

## 10. Storage destinations and S3

Original note: `.todo/s3.md` at `bd66fe0`.

```text
# External storage S3

Not AWS only, but it should be possible to work with S3 storage, like create url for write file and return it to user. List objects.
Sign url to read, or just return url of object.
```

Implementation status: Not implemented. Application files can be stored in local SQLite. S3 application operations, mounted/network destinations, standard protocol adapters and remote archive/backup lifecycle remain. The source for standard protocols is in section 5.

## 11. Shared accounts

Original note: `.todo/shared account.md` at `bd66fe0`.

```text
# Shared accounts

Between projects can have users shared accounts, it will be one project just for account
And other can extends it, it must be good enough to work in all possible scenarios.

```

Implementation status: Not implemented. Projects currently have isolated local actors and permissions. Explicit shared identity bindings and local extensions remain.

## 12. External authentication

Original note: `.todo/external auth.md` at `bd66fe0`.

```text
# Ext auth

project can have login with classic providers like apple/google/facebook/microsoft, or just generic standards
it can be allowed to have more providers in same time
```

Implementation status: Not implemented. Local JWT/API-key verification and administrative token issuance exist. Named providers, standards-based browser login and account linking remain.

## 13. Delegated permissions

Original note: `.todo/subpermissions.md` at `bd66fe0`.

```text
# Subpermissions

user can have allowed to make subpermission, it's for AI and integrations, where user can ask AI to find some data
it can have limited expiration by time, and can have limited permissions, like only to read some entities, it cannot have bigger permissions than original user

lifetime for this token will be limited, i am not sure how much exactly

i am not sure if this feature should be enabled by default, or just enable for some users, or disabled by default
```

Implementation status: Not implemented. Limited user-issued credentials, expiry, revocation and parent-permission intersection remain. The original notes leave defaults and lifetime undecided.

## 14. Generic application frontend

Original note: `.todo/generic frontend.md` at `bd66fe0`.

```text
# Generic frontend

i want ho have ability to generate generic frontend, it can be allowed or disabled for people
they will have login and can do things depends on permissions
```

Implementation status: Partial. An explicit string-field CRUD renderer uses normal application credentials, bounded pages and version conflicts. Broader schemas, relationships, ordinary login flows and full frontend customization remain.

## 15. One-click catalog

Source: later user messages, verbatim.

> do tech todo napadu jeste napis catalog, ze tam bude uz preddefinovany list projektu, ktere muze uzivatel pouzit doslova jednim kliknutim

Implementation status: Partial. Bundled Contacts installs isolated usable instances with generated keys and ordinary owner permissions. Additional applications, updates and HA installation remain.

## 16. Complete built-in email server

Original note: `.todo/emails.md` at `bd66fe0`.

```text
# Emails

2. lvl feature
System have email feature for complete CRUD operations with emails.

It can be enabled individually per projects, or have just email project.

External SMTP server can be used, it must be compatible to all industry standard apis.
```

Implementation status: Not implemented. The source-plugin and shared event requirements are preserved in section 8. Full mail functionality remains; a sending-only or receiving-only implementation will not complete this item.

## Essential details retained from the specifications

These are implementation rules retained from the former specifications, not verbatim user quotations or completed features.

- **Events:** commit accepted input before dispatch. Each handler attempt has its own atomic data/audit transaction. Failed attempts cannot undo accepted input. Retries and terminal retain/delete use one shared mechanism across sources.
- **HA:** replicate all projects, complete databases, audit, allocation sequences and durable work. Confirm required replica durability before acknowledging a write/event. Authenticate peers by pinned public keys. Prevent stale primaries from writing after takeover; peer connectivity alone does not provide public-ingress failover.
- **Recovery:** validate signatures/checksums, decryption, database integrity, types/references and program compatibility in staging before activation. Keep a recovery copy and recover interrupted activation. Carry forward known committed ID high-water marks. Snapshots cannot recover later lost writes or undo already completed external effects.
- **Migrations:** keep the old generation authoritative during preparation. Reconcile intervening writes before the short final pause/cutover; never activate a stale snapshot. Commit the program, schema and migration history together. Preserve seed history, IDs and audit; failed candidates retain the working generation.
- **Logs:** one owner/budget per server. Count pending exports toward bounded local storage. Closing a chunk schedules export; successful upload does not itself evict the local copy. Merge local/remote results by originating server/record identity and explicitly report incomplete scans or unavailable sources.
- **External effects/storage:** persist local changes and effect intent atomically; remote calls cannot inherit SQLite rollback. Reconcile uncertain outcomes and deduplicate where supported. Verify complete remote artifacts before publishing them. Use shared destination/retry mechanisms, not a scheduler per protocol.
- **Identity/UI:** sharing requires explicit trust and local identity mappings; matching email addresses are not proof of account ownership. Delegation intersects current parent permissions. Application frontends use ordinary credentials and server-side authorization, never the admin credential.
- **Settings/routing:** generate defaults once, preserve deliberate overrides and validate complete replacements. Keep private keys recoverable and secrets out of logs. Domain aliases retain project isolation and never expose the protected admin listener.

## Open product choices

These remain unresolved rather than assumed requirements: public-ingress failover; recovery after loss of all replicas; remote log retention and destination-outage behavior; seed-triggered events and source deletion during transfer; wildcard/domain configuration; provider/provisioning/delegation defaults; frontend registration/customization; additional catalog applications; mailbox interfaces and direct versus relay delivery. Ask only when the relevant implementation needs the answer. Preserve the answer here as another verbatim source quotation.

## Implementation rules

Keep central permissions, project isolation, positive numeric branded entity IDs and transactional data/audit. Keep operational logs outside application databases and exclude credentials/bodies/plugin arguments. Failed candidates retain the working generation. Format code, run the repository's required Rust/frontend/integration checks, and commit coherent changes. See [AGENTS.md](AGENTS.md).
