//! Runs one compiled program against its own SQLite database.
//! Owns the transaction and permission boundary for that project.
//! Does not host HTTP, load other projects, or keep server secrets.

use crate::{
    Error, Result,
    program::{Program, Type, literal},
    value::Value,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use auth::{authenticate, authenticate_candidates};
pub(crate) use session::duration;
use session::{Scope, Session};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use store::{
    err, exists, insert, lookup, q, request_id, stored_type, validate_program_configuration,
    validate_stored_data, wire_type,
};
use wire::wire;

mod admin;
mod auth;
mod session;
mod store;
mod wire;

#[derive(Default)]
pub struct RuntimeGate {
    pub stopped: AtomicBool,
    pub lock: RwLock<()>,
}

#[derive(Clone, Default)]
pub struct Config {
    pub base_path: String,
    pub test_seeds: bool,
    pub manifest: Option<serde_json::Value>,
    pub jwt_keys: BTreeMap<String, Vec<u8>>,
    pub event_credentials: BTreeMap<String, String>,
}
pub struct Runtime {
    pub program: Program,
    pub(crate) db: Connection,
    config: Config,
    pub observability: crate::observability::Observability,
    pub(crate) frontend: Option<serde_json::Value>,
    pub(crate) suspended: bool,
    pub(crate) gate: Option<Arc<RuntimeGate>>,
}
impl Runtime {
    pub fn open(source: &str, path: &str, config: Config) -> Result<Self> {
        Self::open_inner(source, path, config, false)
    }
    pub fn reload(&mut self, source: &str, config: Config) -> Result<()> {
        self.require_available()?;
        let program = Program::compile(source)?;
        let migration_chain = crate::migrations::chain(&program, self.program.schema_version)?;
        for (name, entity) in &self.program.entities {
            let next = program.entities.get(name).ok_or_else(|| {
                Error::new("configuration", "Removing an entity requires migration")
            })?;
            for (field, definition) in &entity.fields {
                let renamed = crate::migrations::renamed(&migration_chain, name, field);
                let other = next.fields.get(&renamed).ok_or_else(|| {
                    Error::new("configuration", "Removing a field requires migration")
                })?;
                let describe = |p: &Program, f: &crate::program::Field| -> Result<String> {
                    Ok(format!(
                        "{:?}/{:?}/{}/{}",
                        wire_type(p, &f.ty)?,
                        f.relation,
                        f.unique,
                        f.generated
                    ))
                };
                let mut previous = definition.clone();
                if let Some((entity, field)) = &mut previous.relation {
                    *field = crate::migrations::renamed(&migration_chain, entity, field);
                }
                if renamed != *field && (definition.relation.is_some() || other.relation.is_some())
                {
                    return Err(Error::new(
                        "configuration",
                        "Migration renames must refer to stored fields",
                    ));
                }
                if describe(&self.program, &previous)? != describe(&program, other)? {
                    return Err(Error::new(
                        "configuration",
                        "Changing stored field types or constraints requires migration",
                    ));
                }
            }
            for (field, definition) in &next.fields {
                if !entity
                    .fields
                    .keys()
                    .any(|old| crate::migrations::renamed(&migration_chain, name, old) == *field)
                    && definition.relation.is_none()
                    && !matches!(program.resolve(&definition.ty)?, Type::Optional(_))
                {
                    return Err(Error::new(
                        "configuration",
                        "New fields on existing entities must be optional",
                    ));
                }
            }
        }
        if self.program.plugins.contains_key("Files.put")
            && !program.plugins.contains_key("Files.put")
        {
            return Err(Error::new(
                "configuration",
                "Removing blob storage requires migration",
            ));
        }
        let path = self
            .db
            .path()
            .ok_or_else(|| err("configuration"))?
            .to_string();
        if path.is_empty() {
            return Err(Error::new(
                "configuration",
                "Reload requires a disk database",
            ));
        }
        let mut candidate = Self::open_inner(source, &path, config, true)?;
        candidate.observability = self.observability.clone();
        candidate.gate = self.gate.clone();
        *self = candidate;
        Ok(())
    }
    fn open_inner(source: &str, path: &str, config: Config, reload: bool) -> Result<Self> {
        let program = Program::compile(source)?;
        validate_program_configuration(&program, &config)?;
        for (name, method) in &program.plugins {
            let expected = match name.as_str() {
                "Payment.createUrl" | "Image.resize" => "pure",
                "Files.put" => "transactional",
                _ => {
                    return Err(Error::new(
                        "configuration",
                        format!("No native implementation for {name}"),
                    ));
                }
            };
            if method.mode != expected {
                return Err(Error::new("configuration", "Native plugin mode mismatch"));
            };
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000; BEGIN IMMEDIATE;")?;
        let pause_limit = reload.then(|| crate::migrations::PauseLimit::new(&db));
        let result = (|| -> Result<()> {
            db.execute_batch("CREATE TABLE IF NOT EXISTS _flow_id_sequences(entity TEXT PRIMARY KEY, value INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS _flow_schema (id INTEGER PRIMARY KEY CHECK(id=1), hash TEXT NOT NULL);")?;
            let hash = format!("numeric-v1:{:x}", Sha256::digest(source));
            let stored: Option<String> = db
                .query_row("SELECT hash FROM _flow_schema WHERE id=1", [], |r| r.get(0))
                .optional()?;
            if stored
                .as_ref()
                .is_some_and(|s| !s.starts_with("numeric-v1:"))
            {
                return Err(Error::new(
                    "configuration",
                    "Database uses legacy text IDs; start with a new database",
                ));
            }
            let initialized = stored.is_some();
            if !reload && stored.as_ref().is_some_and(|s| s != &hash) {
                return Err(Error::new(
                    "configuration",
                    "Database belongs to a different Flow program; explicit migration required",
                ));
            };
            let migrated = crate::migrations::apply(&db, &program, initialized)?;
            for (name, entity) in &program.entities {
                let existing = db
                    .prepare(&format!("PRAGMA table_info({})", q(name)))?
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<std::result::Result<BTreeSet<_>, _>>()?;
                let mut definitions = vec![];
                for (field, f) in &entity.fields {
                    if f.relation.is_some() {
                        continue;
                    };
                    let t = program.resolve(&f.ty)?;
                    let base = if let Type::Optional(inner) = t {
                        program.resolve(inner)?
                    } else {
                        t
                    };
                    let numeric = matches!(base, Type::Id(_))
                        || matches!(base, Type::Named(target) if program.entities.contains_key(target));
                    let mut def =
                        format!("{} {}", q(field), if numeric { "INTEGER" } else { "TEXT" });
                    if field == "id" {
                        def.push_str(" PRIMARY KEY NOT NULL")
                    } else if !matches!(t, Type::Optional(_)) {
                        def.push_str(" NOT NULL")
                    };
                    if numeric {
                        def.push_str(&format!(" CHECK({0} IS NULL OR (typeof({0})='integer' AND {0} BETWEEN 1 AND 9007199254740991))", q(field)));
                    }
                    if f.unique {
                        def.push_str(" UNIQUE")
                    };
                    let base = if let Type::Optional(t) = t {
                        program.resolve(t)?
                    } else {
                        t
                    };
                    if let Type::Named(target) = base
                        && program.entities.contains_key(target)
                    {
                        def.push_str(&format!(
                            " REFERENCES {}(id) DEFERRABLE INITIALLY DEFERRED",
                            q(target)
                        ))
                    }
                    if reload && !existing.is_empty() && !existing.contains(field) {
                        db.execute_batch(&format!(
                            "ALTER TABLE {} ADD COLUMN {};",
                            q(name),
                            def.replace(" UNIQUE", "")
                        ))?;
                    }
                    definitions.push(def);
                }
                db.execute_batch(&format!(
                    "CREATE TABLE IF NOT EXISTS {} ({});",
                    q(name),
                    definitions.join(",")
                ))?;
                for (field, f) in &entity.fields {
                    if f.unique && f.relation.is_none() {
                        db.execute_batch(&format!(
                            "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {}({});",
                            q(&format!("unique_{name}_{field}")),
                            q(name),
                            q(field)
                        ))?;
                    }
                    if matches!(program.resolve(&f.ty)?,Type::Named(t) if program.entities.contains_key(t))
                    {
                        db.execute_batch(&format!(
                            "CREATE INDEX IF NOT EXISTS {} ON {}({});",
                            q(&format!("idx_{name}_{field}")),
                            q(name),
                            q(field)
                        ))?;
                    }
                }
            }
            if program.plugins.contains_key("Files.put") {
                db.execute_batch("CREATE TABLE IF NOT EXISTS _flow_blobs(file_id INTEGER PRIMARY KEY REFERENCES File(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,bytes BLOB NOT NULL);")?;
            }
            crate::audit::install(&db, &program)?;
            db.execute_batch("CREATE TABLE IF NOT EXISTS _flow_seed_rows(entity TEXT NOT NULL,id INTEGER NOT NULL,PRIMARY KEY(entity,id));")?;
            for seed in &program.seeds {
                if seed.group == crate::program::SeedGroup::Test && !config.test_seeds {
                    continue;
                }
                let entity = &seed.entity;
                for node in &seed.rows {
                    let fields = literal(node)?.fields()?.clone();
                    let id = crate::program::valid_id(
                        fields
                            .get("id")
                            .ok_or_else(|| err("invalid_input"))?
                            .clone(),
                    )?;
                    let first = db.execute(
                        "INSERT OR IGNORE INTO _flow_seed_rows(entity,id) VALUES(?1,?2)",
                        rusqlite::params![entity, id],
                    )? > 0;
                    if !first {
                        continue;
                    }
                    if exists(&db, entity, &id)? {
                        return Err(Error::new(
                            "configuration",
                            "New seed conflicts with an existing entity ID",
                        ));
                    }
                    let reference = Value::reference(entity, &id);
                    let t = stored_type(&program, entity)?;
                    let row = program.validate(&t, Value::record(fields), Some(reference))?;
                    insert(&db, entity, row.fields()?)?;
                }
            }
            for operation in &program.operations {
                if let Some(event) = &operation.event {
                    let credential =
                        &config.event_credentials[&format!("{}:{}", event.adapter, event.actor_id)];
                    let (_, actor) = authenticate_candidates(
                        &program,
                        &db,
                        &config,
                        Some(credential),
                        std::slice::from_ref(&event.adapter),
                    )?;
                    if actor.id()? != event.actor_id {
                        return Err(Error::new(
                            "configuration",
                            "Event credential does not match declared actor",
                        ));
                    };
                }
            }
            if migrated {
                validate_stored_data(&db, &program, pause_limit.as_ref())?;
            }
            if let Some(limit) = &pause_limit {
                limit.check()?;
            }
            db.execute(
                "INSERT INTO _flow_schema(id,hash) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET hash=excluded.hash",
                [hash],
            )?;
            crate::generations::install(
                &db,
                source,
                config.manifest.as_ref().unwrap_or(&serde_json::json!({})),
            )?;
            Ok(())
        })();
        let result = if pause_limit
            .as_ref()
            .is_some_and(|limit| limit.check().is_err())
        {
            Err(Error::new(
                "limit",
                "Schema activation exceeded its five-second pause budget",
            ))
        } else {
            result
        };
        drop(pause_limit);
        match result {
            Ok(()) => db.execute_batch("COMMIT;")?,
            Err(e) => {
                let _ = db.execute_batch("ROLLBACK;");
                return Err(e);
            }
        };
        Ok(Self {
            program,
            db,
            config,
            observability: Default::default(),
            frontend: None,
            suspended: false,
            gate: None,
        })
    }
    pub(crate) fn validate_configuration(&self, config: &Config) -> Result<()> {
        self.require_available()?;
        validate_program_configuration(&self.program, config)?;
        for operation in &self.program.operations {
            if let Some(event) = &operation.event {
                let credential =
                    &config.event_credentials[&format!("{}:{}", event.adapter, event.actor_id)];
                let (_, actor) = authenticate_candidates(
                    &self.program,
                    &self.db,
                    config,
                    Some(credential),
                    std::slice::from_ref(&event.adapter),
                )
                .map_err(|_| {
                    Error::new(
                        "configuration",
                        "New secret does not authorize the configured event actor",
                    )
                })?;
                if actor.id()? != event.actor_id {
                    return Err(Error::new(
                        "configuration",
                        "Event credential does not match declared actor",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn activate_configuration(&mut self, config: Config) {
        self.config = config;
    }
    pub(crate) fn suspend(&mut self) -> Result<()> {
        self.suspended = true;
        self.db = Connection::open_in_memory()?;
        Ok(())
    }
    pub(crate) fn require_available(&self) -> Result<()> {
        if self.suspended
            || self
                .gate
                .as_ref()
                .is_some_and(|gate| gate.stopped.load(Ordering::Acquire))
        {
            Err(Error::new("unavailable", "This project is restarting"))
        } else {
            Ok(())
        }
    }
    pub fn snapshot_to(&self, path: &std::path::Path) -> Result<()> {
        self.db.execute(
            "VACUUM INTO ?1",
            [path
                .to_str()
                .ok_or_else(|| Error::new("invalid_input", "Invalid snapshot path"))?],
        )?;
        Ok(())
    }
    pub(crate) fn generation(&self) -> Result<serde_json::Value> {
        crate::generations::read(&self.db)?
            .ok_or_else(|| Error::new("configuration", "Active program metadata is unavailable"))
    }
    pub(crate) fn configuration(&self) -> Config {
        self.config.clone()
    }
    pub fn validate_storage(&self) -> Result<()> {
        validate_stored_data(&self.db, &self.program, None)
    }
    pub fn migration_status(&self) -> Result<serde_json::Value> {
        self.require_available()?;
        let mut statement=self.db.prepare("SELECT target_version,migration_id,checksum,applied,time FROM _flow_migrations ORDER BY target_version DESC LIMIT 256")?;
        let rows=statement.query_map([],|row|Ok(serde_json::json!({"to":row.get::<_,u32>(0)?,"id":row.get::<_,String>(1)?,"checksum":row.get::<_,String>(2)?,"applied":row.get::<_,bool>(3)?,"time":row.get::<_,String>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(
            serde_json::json!({"schema_version":self.program.schema_version,"migrations":rows,"activation_pause_limit_seconds":5}),
        )
    }
    pub fn jwt_adapters(&self) -> Result<serde_json::Value> {
        let mut adapters = vec![];
        for auth in self.program.auth.iter().filter(|a| a.mode == "jwt") {
            let mut statement = self.db.prepare(&format!(
                "SELECT id,{} FROM {} ORDER BY id LIMIT 100",
                q(&auth.field),
                q(&auth.entity)
            ))?;
            let subjects=statement.query_map([],|row| {
                let subject:String=row.get(1)?;
                Ok(serde_json::json!({"actor_id":row.get::<_,i64>(0)?,"subject":serde_json::from_str::<serde_json::Value>(&subject).unwrap_or(serde_json::Value::Null)}))
            })?.collect::<std::result::Result<Vec<_>,_>>()?;
            adapters.push(serde_json::json!({"alias":auth.alias,"issuer":auth.issuer,"audience":auth.audience,"configured":self.config.jwt_keys.contains_key(&auth.alias),"subjects":subjects}));
        }
        Ok(serde_json::json!(adapters))
    }
    pub fn issue_admin_jwt(
        &self,
        alias: &str,
        subject: &str,
        ttl: u64,
    ) -> Result<serde_json::Value> {
        if !(60..=86400).contains(&ttl) || subject.is_empty() || subject.len() > 512 {
            return Err(err("invalid_input"));
        }
        let auth = self
            .program
            .auth
            .iter()
            .find(|a| a.alias == alias && a.mode == "jwt")
            .ok_or_else(|| err("invalid_input"))?;
        let key = self
            .config
            .jwt_keys
            .get(alias)
            .ok_or_else(|| err("configuration"))?;
        if lookup(&self.db, auth, subject)?.is_none() {
            return Err(err("not_found"));
        }
        let now = Utc::now().timestamp();
        let claims = serde_json::json!({"iss":auth.issuer,"aud":auth.audience,"sub":subject,"iat":now,"nbf":now,"exp":now+ttl as i64,"jti":uuid::Uuid::new_v4().to_string()});
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        let unsigned = format!("{header}.{payload}");
        let signature = ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key),
            unsigned.as_bytes(),
        );
        let token = format!("{unsigned}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()));
        self.observability.log(serde_json::json!({"kind":"admin","name":"jwt.issued","adapter":alias,"subject":subject,"ttl_seconds":ttl}));
        Ok(
            serde_json::json!({"token":token,"authorization":format!("Bearer {token}"),"claims":claims}),
        )
    }
    fn run_events(&self, events: Vec<Value>) -> Result<(u64, u64)> {
        let mut handlers = 0;
        let mut queue = std::collections::VecDeque::from(events);
        let mut count = 0;
        while let Some(reference) = queue.pop_front() {
            count += 1;
            if count > 256 {
                return Err(err("limit"));
            };
            let Value::Ref { entity, .. } = &reference else {
                return Err(err("invalid_program"));
            };
            for operation in &self.program.operations {
                let Some(event) = &operation.event else {
                    continue;
                };
                if event.source != *entity {
                    continue;
                };
                let credential = self
                    .config
                    .event_credentials
                    .get(&format!("{}:{}", event.adapter, event.actor_id))
                    .ok_or_else(|| err("unauthenticated"))?;
                let (actor_type, actor) = authenticate_candidates(
                    &self.program,
                    &self.db,
                    &self.config,
                    Some(credential),
                    std::slice::from_ref(&event.adapter),
                )?;
                if actor.id()? != event.actor_id {
                    return Err(err("forbidden"));
                };
                let now = Utc::now().to_rfc3339();
                let mut session = Session {
                    observer: &self.observability,
                    base_path: &self.config.base_path,
                    p: &self.program,
                    db: &self.db,
                    actor,
                    actor_type,
                    changes: BTreeMap::new(),
                    cache: BTreeMap::new(),
                    permission_stack: BTreeSet::new(),
                    steps: 0,
                    now: now.clone(),
                    sql: vec![],
                    invocations: vec![],
                    blobs: BTreeMap::new(),
                };
                let mut scope = Scope {
                    defs: operation.bindings.clone(),
                    values: BTreeMap::from([
                        ("event".into(), reference.clone()),
                        (
                            "request".into(),
                            Value::record(BTreeMap::from([
                                ("id".into(), Value::Id("Request".into(), request_id())),
                                ("time".into(), Value::Str(now)),
                                ("actor".into(), session.actor.clone()),
                            ])),
                        ),
                    ]),
                    ..Default::default()
                };
                if !session.eval(&event.condition, &mut scope, false)?.truth()? {
                    continue;
                };
                for binding in &operation.binding_order {
                    session.binding(binding, &mut scope, false)?;
                }
                let value = session.eval(&operation.result, &mut scope, false)?;
                session.authorize()?;
                session.project(&operation.output, value)?;
                if !session.changes.is_empty() {
                    self.db.execute(
                        "UPDATE _flow_audit_context SET operation=?1, transport='event', actor=?2 WHERE id=1",
                        rusqlite::params![
                            operation.name,
                            serde_json::json!({"type": session.actor_type, "id": session.actor.id().ok()}).to_string()
                        ],
                    )?;
                }
                session.apply()?;
                handlers += 1;
                queue.extend(
                    session
                        .changes
                        .iter()
                        .filter(|((entity, _), change)| {
                            change.action == "CREATE" && self.program.streams.contains_key(entity)
                        })
                        .map(|(_, change)| change.reference.clone()),
                );
            }
        }
        Ok((count as u64, handlers))
    }
    pub fn file_bytes(&self, id: &i64, credential: Option<&str>) -> Result<Vec<u8>> {
        let mut span = self.observability.span("io", "sqlite.file_bytes");
        let result = self.file_bytes_inner(id, credential);
        if result.is_ok() {
            span.success();
        }
        result
    }
    fn file_bytes_inner(&self, id: &i64, credential: Option<&str>) -> Result<Vec<u8>> {
        self.require_available()?;
        if !self.program.plugins.contains_key("Files.put") {
            return Err(err("not_found"));
        };
        let (actor_type, actor) =
            authenticate(&self.program, &self.db, &self.config, credential, "HTTP")?;
        let mut session = Session {
            observer: &self.observability,
            base_path: &self.config.base_path,
            p: &self.program,
            db: &self.db,
            actor,
            actor_type,
            changes: BTreeMap::new(),
            cache: BTreeMap::new(),
            permission_stack: BTreeSet::new(),
            steps: 0,
            now: Utc::now().to_rfc3339(),
            sql: vec![],
            invocations: vec![],
            blobs: BTreeMap::new(),
        };
        let target = Value::reference("File", id);
        if !session.allowed("READ", &target, None)? {
            return Err(err("not_found"));
        };
        self.db
            .query_row(
                "SELECT bytes FROM _flow_blobs WHERE file_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| err("not_found"))
    }
    pub fn authenticate_transport(&self, credential: Option<&str>, transport: &str) -> Result<()> {
        self.require_available()?;
        authenticate(&self.program, &self.db, &self.config, credential, transport).map(|_| ())
    }
    pub fn mqtt_credential(&self, alias: &str, secret: &str) -> Result<String> {
        let auth = self
            .program
            .auth
            .iter()
            .find(|a| a.alias == alias)
            .ok_or_else(|| err("unauthenticated"))?;
        if !self
            .program
            .transports
            .get("MQTT")
            .is_some_and(|a| a.contains(&auth.alias))
        {
            return Err(err("unauthenticated"));
        };
        match auth.mode.as_str() {
            "apiKey" => Ok(format!("ApiKey {secret}")),
            "jwt" => Ok(format!("Bearer {secret}")),
            _ => Err(err("unauthenticated")),
        }
    }
    pub fn publish(
        &mut self,
        stream: &str,
        input: serde_json::Value,
        credential: Option<&str>,
        transport: &str,
    ) -> Result<String> {
        let mut span = self.observability.span("io", "sqlite.publish");
        let result = self.publish_inner(stream, input, credential, transport);
        if result.is_ok() {
            span.success();
        }
        result
    }
    fn publish_inner(
        &mut self,
        stream: &str,
        input: serde_json::Value,
        credential: Option<&str>,
        transport: &str,
    ) -> Result<String> {
        self.require_available()?;
        let gate = self.gate.clone();
        let _activity = gate
            .as_ref()
            .map(|gate| {
                gate.lock
                    .read()
                    .map_err(|_| Error::new("internal", "Runtime activity lock failed"))
            })
            .transpose()?;
        self.require_available()?;
        if !self.program.streams.contains_key(stream) {
            return Err(err("not_found"));
        };
        self.db.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| -> Result<_> {
            let (actor_type, actor) =
                authenticate(&self.program, &self.db, &self.config, credential, transport)?;
            let mut session = Session {
                observer: &self.observability,
                base_path: &self.config.base_path,
                p: &self.program,
                db: &self.db,
                actor,
                actor_type,
                changes: BTreeMap::new(),
                cache: BTreeMap::new(),
                permission_stack: BTreeSet::new(),
                steps: 0,
                now: Utc::now().to_rfc3339(),
                sql: vec![],
                invocations: vec![],
                blobs: BTreeMap::new(),
            };
            crate::audit::context(
                &self.db,
                stream,
                transport,
                &serde_json::json!({"type":session.actor_type,"id":session.actor.id().ok()}),
            )?;
            let fields = Value::from_json(&input)?.fields()?.clone();
            let reference = session.create(stream, fields)?;
            session.authorize()?;
            session.apply()?;
            let processed = self.run_events(vec![reference.clone()])?;
            Ok((reference.text()?, processed))
        })();
        match result {
            Ok((id, (events, handlers))) => {
                if let Err(e) = self.db.execute_batch("COMMIT;") {
                    let _ = self.db.execute_batch("ROLLBACK;");
                    return Err(e.into());
                };
                self.observability.event("stream_events", events);
                self.observability.event("automation_runs", handlers);
                Ok(id)
            }
            Err(e) => {
                let _ = self.db.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }
    pub fn publish_topic(
        &mut self,
        topic: &str,
        input: serde_json::Value,
        credential: Option<&str>,
    ) -> Result<String> {
        let mut matches = vec![];
        for (name, stream) in &self.program.streams {
            if let Some(fields) = crate::mqtt::topic_inputs(&stream.topic, topic)? {
                matches.push((name.clone(), fields));
            }
        }
        if matches.len() != 1 {
            return Err(err("not_found"));
        };
        let (name, fields) = matches.pop().unwrap();
        let mut input = input
            .as_object()
            .cloned()
            .ok_or_else(|| err("invalid_input"))?;
        for (field, id) in fields {
            let id = id.parse::<i64>().map_err(|_| err("invalid_input"))?;
            if input
                .get(&field)
                .is_some_and(|v| v != &serde_json::json!(id))
            {
                return Err(err("invalid_input"));
            };
            input.insert(field, serde_json::json!(id));
        }
        self.publish(&name, serde_json::Value::Object(input), credential, "MQTT")
    }
    pub fn mqtt_messages(
        &mut self,
        filter: &str,
        credential: Option<&str>,
    ) -> Result<Vec<(String, String, serde_json::Value)>> {
        self.db.execute_batch("BEGIN;")?;
        let result = (|| -> Result<_> {
            let (actor_type, actor) =
                authenticate(&self.program, &self.db, &self.config, credential, "MQTT")?;
            let mut session = Session {
                observer: &self.observability,
                base_path: &self.config.base_path,
                p: &self.program,
                db: &self.db,
                actor,
                actor_type,
                changes: BTreeMap::new(),
                cache: BTreeMap::new(),
                permission_stack: BTreeSet::new(),
                steps: 0,
                now: Utc::now().to_rfc3339(),
                sql: vec![],
                invocations: vec![],
                blobs: BTreeMap::new(),
            };
            let mut messages = vec![];
            for (name, stream) in &self.program.streams {
                let sql = format!("SELECT id FROM {} ORDER BY rowid", q(name));
                let mut span = self.observability.span("io", "sqlite.scan_entities");
                let mut statement = self.db.prepare(&sql)?;
                let ids = statement
                    .query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                span.success();
                for id in ids {
                    let reference = Value::reference(name, &id);
                    if !session.allowed("READ", &reference, None)? {
                        continue;
                    };
                    let mut segments = vec![];
                    for segment in stream.topic.split('/') {
                        if let Some(field) =
                            segment.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
                        {
                            segments.push(session.field(&reference, field, false)?.text()?)
                        } else {
                            segments.push(segment.into())
                        }
                    }
                    let topic = segments.join("/");
                    if !crate::mqtt::matches(filter, &topic) {
                        continue;
                    };
                    let payload = session
                        .project(
                            &wire_type(&self.program, &stored_type(&self.program, name)?)?,
                            reference,
                        )?
                        .json()?;
                    messages.push((format!("{name}:{id}"), topic, payload));
                }
            }
            Ok(messages)
        })();
        let _ = self.db.execute_batch("ROLLBACK;");
        result
    }
    pub fn execute(
        &mut self,
        name: &str,
        input: serde_json::Value,
        authorization: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.execute_traced(name, input, authorization).map(|r| r.0)
    }
    pub fn execute_traced(
        &mut self,
        name: &str,
        input: serde_json::Value,
        authorization: Option<&str>,
    ) -> Result<(serde_json::Value, Vec<String>)> {
        self.execute_transport(name, input, authorization, "HTTP")
            .map(|(out, sql, _)| (out, sql))
    }
    pub fn execute_transport(
        &mut self,
        name: &str,
        input: serde_json::Value,
        authorization: Option<&str>,
        transport: &str,
    ) -> Result<(serde_json::Value, Vec<String>, Vec<String>)> {
        let mut span = self.observability.span("io", "sqlite.operation");
        let result = self.execute_transport_inner(name, input, authorization, transport);
        if result.is_ok() {
            span.success();
        }
        result
    }
    fn execute_transport_inner(
        &mut self,
        name: &str,
        input: serde_json::Value,
        authorization: Option<&str>,
        transport: &str,
    ) -> Result<(serde_json::Value, Vec<String>, Vec<String>)> {
        self.require_available()?;
        let gate = self.gate.clone();
        let _activity = gate
            .as_ref()
            .map(|gate| {
                gate.lock
                    .read()
                    .map_err(|_| Error::new("internal", "Runtime activity lock failed"))
            })
            .transpose()?;
        self.require_available()?;
        let op = self
            .program
            .operations
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| err("not_found"))?
            .clone();
        if op.event.is_some() {
            return Err(err("not_found"));
        };
        self.db.execute_batch("BEGIN IMMEDIATE;")?;
        let mut processed = (0, 0);
        let result = (|| -> Result<_> {
            let (actor_type, actor) = authenticate(
                &self.program,
                &self.db,
                &self.config,
                authorization,
                transport,
            )?;
            let now = Utc::now().to_rfc3339();
            let mut session = Session {
                observer: &self.observability,
                base_path: &self.config.base_path,
                p: &self.program,
                db: &self.db,
                actor,
                actor_type,
                changes: BTreeMap::new(),
                cache: BTreeMap::new(),
                permission_stack: BTreeSet::new(),
                steps: 0,
                now: now.clone(),
                sql: vec![],
                invocations: vec![],
                blobs: BTreeMap::new(),
            };
            let input = Value::from_json(&input)?;
            let inputs = input.fields()?;
            if inputs.keys().any(|k| !op.inputs.contains_key(k)) {
                return Err(err("invalid_input"));
            };
            let mut scope = Scope {
                defs: op.bindings.clone(),
                ..Default::default()
            };
            for (key, (ty, default)) in &op.inputs {
                let value = inputs
                    .get(key)
                    .cloned()
                    .or_else(|| default.as_ref().and_then(|n| literal(n).ok()))
                    .unwrap_or(Value::Null);
                let value = wire(&self.program, ty, value)?;
                scope
                    .values
                    .insert(format!("${key}"), self.program.validate(ty, value, None)?);
            }
            scope.values.insert(
                "request".into(),
                Value::record(BTreeMap::from([
                    ("id".into(), Value::Id("Request".into(), request_id())),
                    ("time".into(), Value::Str(now)),
                    ("actor".into(), session.actor.clone()),
                ])),
            );
            // Source order: a later [set] must not hide fields a previous binding reads.
            for binding in &op.binding_order {
                session.binding(binding, &mut scope, false)?;
            }
            let value = session.eval(&op.result, &mut scope, false)?;
            session.authorize()?;
            let keys = if op.stream.is_some() {
                value
                    .list()?
                    .iter()
                    .map(Value::text)
                    .collect::<Result<Vec<_>>>()?
            } else {
                vec![]
            };
            let output = session.project(&op.output, value)?.json()?;
            if !session.changes.is_empty() {
                crate::audit::context(
                    &self.db,
                    name,
                    transport,
                    &serde_json::json!({"type": session.actor_type, "id": session.actor.id().ok()}),
                )?;
            }
            session.apply()?;
            let events = session
                .changes
                .iter()
                .filter(|((entity, _), change)| {
                    change.action == "CREATE" && self.program.streams.contains_key(entity)
                })
                .map(|(_, change)| change.reference.clone())
                .collect();
            processed = self.run_events(events)?;
            Ok((output, session.sql, keys))
        })();
        match result {
            Ok(v) => {
                if let Err(e) = self.db.execute_batch("COMMIT;") {
                    let _ = self.db.execute_batch("ROLLBACK;");
                    return Err(e.into());
                }
                self.observability.event("stream_events", processed.0);
                self.observability.event("automation_runs", processed.1);
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }
}
