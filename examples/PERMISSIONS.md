# Permissions a auth v jednotném hranatém formátu

Aktuální parser převádí pouze syntaxi do AST. Níže jsou dohodnuté doménové konvence pro budoucí validátor a runtime. Kompletní aplikace je v [application.flow](../prototype/priv/workflows/application.flow), další scénáře v [permissions.flow](permissions.flow). Gramatiku a API popisuje [LANGUAGE.md](../prototype/docs/LANGUAGE.md).

## Permissions ze strany aktéra

```flow
[permissions User
  [Post
    [READ [when target.public]]
    [UPDATE [includes READ] [when [eq target.author actor]]]
    [DELETE [includes UPDATE] [when [eq target.author actor]]]]]

[permissions Service
  [Post [READ [when [eq target.organization actor.organization]]]]]

[permissions *
  [Post [READ [when target.public]]]]
```

Vše používá stejnou syntaxi `[název argumenty...]`, včetně podmínek a reference na permissions jiného zdroje: `[can READ target.device]`. Infixové operátory ani speciální anotace nejsou potřeba.

`actor` je ověřená doménová identita typu `User`, `Service`, `Device` nebo jiného definovaného typu. Nemáme povinnou centrální tabulku uživatelů ani jednu roli. Role, skupiny, tenanty, vlastnictví a sdílení jsou doménová data a vztahy. `*` zahrnuje všechny aktéry i anonymní.

`target` je chráněný zdroj, `before` a `after` stavy změny, `changed` množina měněných polí, `transaction` navrhované související změny. Pro plugin je dostupné `args`, pro vloženou hodnotu `parent`. `context` obsahuje důvěryhodné fakty, čas, MFA, účel a delegaci. Tyto údaje nelze bez ověření převzít z klientského vstupu.

## Pozitivní granty a explicitní dědičnost

Bez odpovídajícího grantu není přístup povolený. Více grantů se skládá přes OR. Neexistuje samostatné negativní právo ani priorita `DENY`; negace je běžná část podmínky:

```flow
[permissions User
  [Document [READ [when [and
    [not [contains target.blocked actor]]
    [or [eq target.owner actor] [contains target.team.members actor]]]]]]]
```

Omezení musí platit v každé povolující větvi, včetně děděných grantů. Jiný grant jinak může přístup udělit. Případné nezávislé organizační hranice by vyžadovaly další explicitně definované skládání, ne implicitní zákaz.

`[UPDATE [includes READ] [when podmínka]]` uděluje při splnění podmínky obě akce nad stejným cílem. Deklarovaný graf `includes` v bloku cílového typu umožňuje tranzitivní dědičnost: `DELETE → UPDATE → READ`. Podmínka původního grantu stále platí; nepřidává se podmínka jiné větve `UPDATE` či `READ`. Bez explicitně deklarované vazby `UPDATE` nezahrnuje čtení.

Podmínka akce s dědičností musí být vyhodnotitelná i pro zděděné akce. Pravidlo používající `after` nebo `changed` nelze bez dalšího použít pro běžné `READ`; musí se rozdělit. Dědičnost typů aktérů nebo pojmenovaných sad není automatickou součástí `includes`.

Základní akce: `READ`, `USE`, `CREATE`, `UPDATE`, `DELETE`, `INVOKE`. Doménové akce jako `CONTROL` musí mít explicitní kontrakt účinku. `USE` může povolit interní použití pole, ale nikoliv jeho přímé nebo libovolné odvozené zveřejnění.

Pole a externí účinek používají stejný blok:

```flow
[permissions User
  [Employee.salary [READ [when [eq target.user actor]]]]
  [Bank.transfer [INVOKE [when [and
    [eq args.account.owner actor] [eq context.mfa STRONG]]]]]]
```

Pro pole `Employee.salary` je `target` vlastnící zaměstnanec. Oprávnění pole zpřísňuje oprávnění entity; pokud není samostatně definované, dědí jej.

## Authentizace oddělená od autorizace

```flow
[auth
  [user User
    [jwt [issuer "https://identity.example.com"] [audience "application"]]
    [entity [eq User.identitySubject claims.sub]]]
  [service Service [apiKey] [entity [eq Service.keyHash credential.hash]]]
  [device Device [certificate]
    [entity [eq Device.certificateFingerprint certificate.fingerprint]]]
  [anonymous Anonymous]]

[transport HTTP [auth user service anonymous]]
[transport MQTT [auth device service]]
[transport WebSocket [auth user service]]
```

Auth adaptér nejprve ověří credential a potom jej jednoznačně naváže na identitu. Ověřuje například JWT podpis, issuer, audience a expiraci. Vyhledávání identity probíhá v důvěryhodné části runtime a neodhaluje ji klientovi. V běžném scénáři `[entity $id]` vyhledává podle brandovaného ID.

Neplatný credential se nesmí změnit na anonymní přístup. Více různých identit se nespojuje do jednoho aktéra se sjednocenými právy. Interní služby a zařízení mají explicitně ověřené identity; transport nemůže předstírat administrátora.

## Automatické vynucování runtime

Permissions nepatří endpointům. Stejně se uplatňují na seznamy, přímé entity, reference, MQTT historii, každou WebSocket zprávu, fronty, pluginy a interní volání.

- Seznam se filtruje před limitem, řazením, stránkováním a agregací. Bez oprávnění je prázdný.
- Přímé čtení neprozradí rozdíl mezi zakázanou a neexistující entitou.
- Výstupní typ určuje projekci. Čtou se pouze potřebné sloupce a závislosti permissions, filtrů a výpočtů.
- Zápisy autorizují všechna měněná data, i mimo výstup, na konzistentním původním a navrhovaném stavu před commitem.
- Odvozené efekty a pluginy nemají automatický bypass. Služba jedná s vlastní nebo explicitně delegovanou identitou.
- Každé doručení znovu ověřuje aktuální práva. Fronta zachová aktéra a delegaci a zkontroluje je před provedením.
- Vyhodnocovač pravidel může interně číst nezbytné závislosti, ale aplikaci tím neuděluje přístup k těmto datům.
- Selhání ověření, nedostupné nezbytné fakty nebo překročený rozpočet neudělují právo. Cache musí respektovat změny práv i identity.

Parser tyto významové vlastnosti nevynucuje. Zachovává pouze úplný ordered AST pro následnou validaci a implementaci.
