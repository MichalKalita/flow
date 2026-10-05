use crate::{
    Error, Result,
    program::{Program, Type, literal},
    syntax::Node,
    value::{Number, Value, count, equal, integer, number},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn err(code: &'static str) -> Error {
    Error::new(code, code)
}
fn q(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
#[derive(Clone, Default)]
pub struct Config {
    pub jwt_keys: BTreeMap<String, Vec<u8>>,
}
pub struct Runtime {
    pub program: Program,
    db: Connection,
    config: Config,
}
#[derive(Clone, Debug)]
struct Change {
    reference: Value,
    action: String,
    after: BTreeMap<String, Value>,
    changed: BTreeSet<String>,
}
#[derive(Clone, Default)]
struct Scope {
    defs: BTreeMap<String, Node>,
    values: BTreeMap<String, Value>,
    visiting: BTreeSet<String>,
}
struct Session<'a> {
    p: &'a Program,
    db: &'a Connection,
    actor: Value,
    actor_type: String,
    changes: BTreeMap<(String, String), Change>,
    cache: BTreeMap<(String, String, String), Value>,
    permission_stack: BTreeSet<(String, String, String)>,
    steps: usize,
    now: String,
    sql: Vec<String>,
}
impl Runtime {
    pub fn open(source: &str, path: &str, config: Config) -> Result<Self> {
        let program = Program::compile(source)?;
        for (alias, key) in &config.jwt_keys {
            if key.len() < 32
                || !program
                    .auth
                    .iter()
                    .any(|a| a.alias == *alias && a.mode == "jwt")
            {
                return Err(Error::new(
                    "configuration",
                    "JWT key must have at least 32 bytes and match a declared adapter",
                ));
            };
        }
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; BEGIN IMMEDIATE;")?;
        let result = (|| -> Result<()> {
            db.execute_batch("CREATE TABLE IF NOT EXISTS _flow_schema (id INTEGER PRIMARY KEY CHECK(id=1), hash TEXT NOT NULL);")?;
            let hash = format!("{:x}", Sha256::digest(source));
            let stored: Option<String> = db
                .query_row("SELECT hash FROM _flow_schema WHERE id=1", [], |r| r.get(0))
                .optional()?;
            if stored.is_some_and(|s| s != hash) {
                return Err(Error::new(
                    "configuration",
                    "Database belongs to a different Flow program; explicit migration required",
                ));
            };
            for (name, entity) in &program.entities {
                let mut definitions = vec![];
                for (field, f) in &entity.fields {
                    if f.relation.is_some() {
                        continue;
                    };
                    let t = program.resolve(&f.ty)?;
                    let mut def = format!("{} TEXT", q(field));
                    if field == "id" {
                        def.push_str(" PRIMARY KEY NOT NULL")
                    } else if !matches!(t, Type::Optional(_)) {
                        def.push_str(" NOT NULL")
                    };
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
                    definitions.push(def);
                }
                db.execute_batch(&format!(
                    "CREATE TABLE IF NOT EXISTS {} ({});",
                    q(name),
                    definitions.join(",")
                ))?;
                for (field, f) in &entity.fields {
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
            for (entity, rows) in &program.seeds {
                for node in rows {
                    let fields = literal(node)?.fields()?.clone();
                    let id = fields
                        .get("id")
                        .ok_or_else(|| err("invalid_input"))?
                        .text()?;
                    if exists(&db, entity, &id)? {
                        continue;
                    };
                    let reference = Value::reference(entity, &id);
                    let t = stored_type(&program, entity)?;
                    let row = program.validate(&t, Value::record(fields), Some(reference))?;
                    insert(&db, entity, row.fields()?)?;
                }
            }
            db.execute(
                "INSERT OR IGNORE INTO _flow_schema(id,hash) VALUES(1,?1)",
                [hash],
            )?;
            Ok(())
        })();
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
        })
    }
    pub fn authenticate_transport(&self, credential: Option<&str>, transport: &str) -> Result<()> {
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
        let definition = self
            .program
            .streams
            .get(stream)
            .ok_or_else(|| err("not_found"))?
            .clone();
        self.db.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| -> Result<_> {
            let (actor_type, actor) =
                authenticate(&self.program, &self.db, &self.config, credential, transport)?;
            let mut session = Session {
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
            };
            let fields = Value::from_json(&input)?.fields()?.clone();
            let reference = session.create(stream, fields)?;
            session.authorize()?;
            session.apply()?;
            let cutoff = (Utc::now() - chrono::Duration::seconds(definition.duration)).to_rfc3339();
            self.db.execute(
                &format!(
                    "DELETE FROM {} WHERE json_extract(receivedAt,'$') < ?1",
                    q(stream)
                ),
                [cutoff],
            )?;
            let groups = definition
                .topic
                .split('/')
                .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
                .collect::<Vec<_>>();
            let mut predicates = vec![];
            let mut group_values = vec![];
            for field in groups {
                predicates.push(format!("{}=?{}", q(field), group_values.len() + 1));
                group_values.push(storage(&session.raw(&reference, field)?)?);
            }
            let where_clause = if predicates.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", predicates.join(" AND "))
            };
            let sql = format!(
                "DELETE FROM {} WHERE id IN (SELECT id FROM {} {} ORDER BY rowid DESC LIMIT -1 OFFSET {})",
                q(stream),
                q(stream),
                where_clause,
                definition.max_messages
            );
            self.db
                .execute(&sql, rusqlite::params_from_iter(group_values))?;
            reference.text()
        })();
        match result {
            Ok(id) => {
                if let Err(e) = self.db.execute_batch("COMMIT;") {
                    let _ = self.db.execute_batch("ROLLBACK;");
                    return Err(e.into());
                };
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
            if input
                .get(&field)
                .is_some_and(|v| v != &serde_json::Value::String(id.clone()))
            {
                return Err(err("invalid_input"));
            };
            input.insert(field, serde_json::Value::String(id));
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
            };
            let mut messages = vec![];
            for (name, stream) in &self.program.streams {
                let sql = format!("SELECT id FROM {} ORDER BY rowid", q(name));
                let mut statement = self.db.prepare(&sql)?;
                let ids = statement
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
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
        let op = self
            .program
            .operations
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| err("not_found"))?
            .clone();
        self.db.execute_batch("BEGIN IMMEDIATE;")?;
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
                    (
                        "id".into(),
                        Value::Id("Request".into(), uuid::Uuid::new_v4().to_string()),
                    ),
                    ("time".into(), Value::Str(now)),
                ])),
            );
            for binding in op.bindings.keys() {
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
            session.apply()?;
            Ok((output, session.sql, keys))
        })();
        match result {
            Ok(v) => {
                if let Err(e) = self.db.execute_batch("COMMIT;") {
                    let _ = self.db.execute_batch("ROLLBACK;");
                    return Err(e.into());
                }
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }
}
fn stored_type(p: &Program, entity: &str) -> Result<Type> {
    let fields = &p
        .entities
        .get(entity)
        .ok_or_else(|| err("invalid_program"))?
        .fields;
    Ok(Type::Record(
        fields
            .iter()
            .filter(|(_, f)| f.relation.is_none())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}
fn wire_type(p: &Program, t: &Type) -> Result<Type> {
    Ok(match p.resolve(t)? {
        Type::Named(entity) if p.entities.contains_key(entity) => Type::Id(entity.clone()),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(name, field)| {
                    let mut field = field.clone();
                    field.ty = wire_type(p, &field.ty)?;
                    Ok((name.clone(), field))
                })
                .collect::<Result<_>>()?,
        ),
        Type::List(inner, min, max) => Type::List(Box::new(wire_type(p, inner)?), *min, *max),
        Type::Optional(inner) => Type::Optional(Box::new(wire_type(p, inner)?)),
        t => t.clone(),
    })
}
fn exists(db: &Connection, entity: &str, id: &str) -> Result<bool> {
    Ok(db
        .query_row(
            &format!("SELECT id FROM {} WHERE id=?1", q(entity)),
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
fn storage(v: &Value) -> Result<Option<String>> {
    Ok(match v {
        Value::Null => None,
        Value::Ref { id, .. } | Value::Id(_, id) => Some(id.clone()),
        _ => Some(v.json()?.to_string()),
    })
}
fn insert(db: &Connection, entity: &str, values: &BTreeMap<String, Value>) -> Result<()> {
    let columns = values.keys().map(|s| q(s)).collect::<Vec<_>>().join(",");
    let placeholders = (1..=values.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let params = values.values().map(storage).collect::<Result<Vec<_>>>()?;
    db.execute(
        &format!(
            "INSERT INTO {} ({columns}) VALUES ({placeholders})",
            q(entity)
        ),
        rusqlite::params_from_iter(params),
    )?;
    Ok(())
}
fn lookup(db: &Connection, auth: &crate::program::Auth, value: &str) -> Result<Option<Value>> {
    let encoded = serde_json::to_string(value)?;
    let id: Option<String> = db
        .query_row(
            &format!(
                "SELECT id FROM {} WHERE {}=?1",
                q(&auth.entity),
                q(&auth.field)
            ),
            [encoded],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id.map(|id| Value::reference(&auth.entity, &id)))
}
fn authenticate(
    p: &Program,
    db: &Connection,
    config: &Config,
    credential: Option<&str>,
    transport: &str,
) -> Result<(String, Value)> {
    if let Some(credential) = credential {
        if credential.len() > 65536 {
            return Err(err("unauthenticated"));
        };
        let (scheme, secret) = credential
            .split_once(' ')
            .ok_or_else(|| err("unauthenticated"))?;
        let mode = match scheme {
            "Bearer" => "jwt",
            "ApiKey" => "apiKey",
            _ => return Err(err("unauthenticated")),
        };
        for auth in p.auth.iter().filter(|a| {
            a.mode == mode
                && p.transports
                    .get(transport)
                    .is_some_and(|aliases| aliases.contains(&a.alias))
        }) {
            let subject = if mode == "apiKey" {
                if secret.len() < 32 || secret.len() > 4096 {
                    return Err(err("unauthenticated"));
                };
                format!("{:x}", Sha256::digest(secret))
            } else {
                let Some(key) = config.jwt_keys.get(&auth.alias) else {
                    continue;
                };
                match jwt(secret, key, &auth.issuer, &auth.audience) {
                    Ok(s) => s,
                    Err(_) => continue,
                }
            };
            if let Some(actor) = lookup(db, auth, &subject)? {
                return Ok((auth.entity.clone(), actor.pin()));
            }
        }
        return Err(err("unauthenticated"));
    }
    if p.auth.iter().any(|a| {
        a.mode == "anonymous"
            && p.transports
                .get(transport)
                .is_some_and(|aliases| aliases.contains(&a.alias))
    }) {
        Ok(("Anonymous".into(), Value::Null))
    } else {
        Err(err("unauthenticated"))
    }
}
fn jwt(token: &str, key: &[u8], issuer: &str, audience: &str) -> Result<String> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(err("unauthenticated"));
    };
    let header: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| err("unauthenticated"))?,
    )?;
    if header["alg"] != "HS256" || header.get("crit").is_some() {
        return Err(err("unauthenticated"));
    };
    let signature = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| err("unauthenticated"))?;
    let hmac = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::verify(
        &hmac,
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &signature,
    )
    .map_err(|_| err("unauthenticated"))?;
    let claims: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| err("unauthenticated"))?,
    )?;
    let now = Utc::now().timestamp();
    if claims["iss"] != issuer
        || !(claims["aud"] == audience
            || claims["aud"]
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v == audience)))
        || !claims["exp"].as_i64().is_some_and(|exp| exp > now)
        || claims
            .get("nbf")
            .is_some_and(|n| !n.as_i64().is_some_and(|n| n <= now))
    {
        return Err(err("unauthenticated"));
    };
    let subject = claims["sub"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(|| err("unauthenticated"))?;
    Ok(subject.into())
}
fn wire(p: &Program, t: &Type, v: Value) -> Result<Value> {
    let t = p.resolve(t)?;
    Ok(match (t, v) {
        (Type::Number { .. }, Value::Str(s)) => Value::Num(number(&s)?),
        (Type::Optional(inner), v) if v != Value::Null => wire(p, inner, v)?,
        (Type::List(inner, _, _), Value::List(values)) => Value::List(
            values
                .into_iter()
                .map(|v| wire(p, inner, v))
                .collect::<Result<_>>()?,
        ),
        (Type::Record(fs), Value::Record { fields, .. }) => Value::record(
            fields
                .into_iter()
                .map(|(k, v)| {
                    Ok((
                        k.clone(),
                        match fs.get(&k) {
                            Some(f) => wire(p, &f.ty, v)?,
                            None => v,
                        },
                    ))
                })
                .collect::<Result<_>>()?,
        ),
        (_, v) => v,
    })
}
impl Session<'_> {
    fn step(&mut self) -> Result<()> {
        self.steps += 1;
        if self.steps > 100000 {
            return Err(err("limit"));
        };
        Ok(())
    }
    fn binding(&mut self, name: &str, scope: &mut Scope, policy: bool) -> Result<Value> {
        if let Some(v) = scope.values.get(name) {
            return Ok(v.clone());
        };
        if let Some(node) = scope.defs.get(name).cloned() {
            if !scope.visiting.insert(name.into()) {
                return Err(err("invalid_program"));
            };
            let v = self.eval(&node, scope, policy)?;
            scope.visiting.remove(name);
            scope.values.insert(name.into(), v.clone());
            return Ok(v);
        };
        match name {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            "null" => Ok(Value::Null),
            _ => Ok(Value::Str(name.into())),
        }
    }
    fn raw(&mut self, target: &Value, field: &str) -> Result<Value> {
        self.step()?;
        match target {
            Value::List(v) => Ok(Value::List(
                v.iter()
                    .map(|v| self.raw(v, field))
                    .collect::<Result<_>>()?,
            )),
            Value::Record { fields, parent, .. } => {
                if field == "parent" {
                    return parent
                        .as_ref()
                        .map(|p| *p.clone())
                        .ok_or_else(|| err("invalid_input"));
                };
                fields
                    .get(field)
                    .cloned()
                    .ok_or_else(|| err("invalid_input"))
            }
            Value::Ref { entity, id, before } => {
                if let Some(change) = self.changes.get(&(entity.clone(), id.clone()))
                    && !before
                {
                    if change.action == "DELETE" {
                        return Err(err("not_found"));
                    };
                    if let Some(v) = change.after.get(field) {
                        return Ok(v.clone());
                    }
                }
                let f = self
                    .p
                    .entities
                    .get(entity)
                    .and_then(|e| e.fields.get(field))
                    .ok_or_else(|| err("invalid_program"))?
                    .clone();
                if let Some((related, foreign)) = &f.relation {
                    let sql = format!(
                        "SELECT id FROM {} WHERE {}=?1 ORDER BY rowid",
                        q(related),
                        q(foreign)
                    );
                    self.sql.push(sql.clone());
                    let mut statement = self.db.prepare(&sql)?;
                    let ids = statement
                        .query_map([id], |r| r.get::<_, String>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    let mut ids = ids;
                    if !before {
                        for ((e, id), change) in &self.changes {
                            if e == related {
                                if change.action == "DELETE"
                                    || change.after.get(foreign).is_some_and(|v| !equal(v, target))
                                {
                                    ids.retain(|existing| existing != id);
                                } else if change
                                    .after
                                    .get(foreign)
                                    .is_some_and(|v| equal(v, target))
                                    && !ids.contains(id)
                                {
                                    ids.push(id.clone());
                                }
                            }
                        }
                    }
                    Ok(Value::List(
                        ids.into_iter()
                            .map(|id| {
                                let r = Value::reference(related, &id);
                                if *before { r.pin() } else { r }
                            })
                            .collect(),
                    ))
                } else {
                    let key = (entity.clone(), id.clone(), field.into());
                    if let Some(v) = self.cache.get(&key) {
                        return Ok(if *before { v.pin() } else { v.clone() });
                    };
                    let sql = format!("SELECT {} FROM {} WHERE id=?1", q(field), q(entity));
                    self.sql.push(sql.clone());
                    let stored: Option<Option<String>> =
                        self.db.query_row(&sql, [id], |r| r.get(0)).optional()?;
                    let stored = stored.ok_or_else(|| err("not_found"))?;
                    let t = self.p.resolve(&f.ty)?;
                    let base = if let Type::Optional(inner) = t {
                        self.p.resolve(inner)?
                    } else {
                        t
                    };
                    let v = match stored {
                        None => Value::Null,
                        Some(s) => {
                            if matches!(base, Type::Id(_) | Type::Named(_)) {
                                Value::Str(s)
                            } else {
                                Value::from_json(&serde_json::from_str(&s)?)?
                            }
                        }
                    };
                    let v = self
                        .p
                        .validate(&f.ty, v, Some(Value::reference(entity, id)))?;
                    self.cache.insert(key, v.clone());
                    Ok(if *before { v.pin() } else { v })
                }
            }
            _ => Err(err("invalid_input")),
        }
    }
    fn target_kind(target: &Value) -> Option<String> {
        match target {
            Value::Ref { entity, .. } => Some(entity.clone()),
            Value::Record { kind, .. } => kind.clone(),
            _ => None,
        }
    }
    fn allowed(
        &mut self,
        action: &str,
        target: &Value,
        extra: Option<BTreeMap<String, Value>>,
    ) -> Result<bool> {
        self.step()?;
        let Some(kind) = Self::target_kind(target) else {
            return Ok(true);
        };
        let identity = target.text().unwrap_or_else(|_| format!("{target:?}"));
        let key = (kind.clone(), identity, action.into());
        if !self.permission_stack.insert(key.clone()) {
            return Ok(false);
        };
        let result = self.granted(action, &kind, target, extra);
        self.permission_stack.remove(&key);
        result
    }
    fn granted(
        &mut self,
        action: &str,
        kind: &str,
        target: &Value,
        extra: Option<BTreeMap<String, Value>>,
    ) -> Result<bool> {
        let grants = self
            .p
            .grants
            .iter()
            .filter(|g| {
                (g.actor == self.actor_type || g.actor == "*")
                    && g.target == kind
                    && g.action == action
            })
            .cloned()
            .collect::<Vec<_>>();
        for grant in grants {
            let parent = if let Value::Record { parent, .. } = target {
                parent.as_ref().map(|v| *v.clone()).unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            let mut values = BTreeMap::from([
                ("actor".into(), self.actor.clone()),
                ("target".into(), target.clone()),
                ("parent".into(), parent),
                (
                    "context".into(),
                    Value::record(BTreeMap::from([(
                        "now".into(),
                        Value::Str(self.now.clone()),
                    )])),
                ),
            ]);
            if let Some(extra) = &extra {
                values.extend(extra.clone())
            };
            let mut scope = Scope {
                values,
                ..Default::default()
            };
            match self.eval(&grant.condition, &mut scope, true) {
                Ok(Value::Bool(true)) => return Ok(true),
                Err(e) if e.code == "limit" => return Err(e),
                _ => {}
            }
        }
        Ok(false)
    }
    fn permitted_field(
        &mut self,
        action: &str,
        target: &Value,
        field: &str,
        extra: Option<BTreeMap<String, Value>>,
    ) -> Result<bool> {
        if let Some(kind) = Self::target_kind(target) {
            let name = format!("{kind}.{field}");
            if self
                .p
                .grants
                .iter()
                .any(|g| g.target == name && g.action == action)
            {
                return self.granted(action, &name, target, extra);
            }
        }
        Ok(true)
    }
    fn field(&mut self, target: &Value, field: &str, policy: bool) -> Result<Value> {
        if policy {
            return self.raw(target, field);
        };
        if let Value::List(values) = target {
            let values = self.visible(values.clone())?;
            return Ok(Value::List(
                values
                    .iter()
                    .map(|v| self.field(v, field, false))
                    .collect::<Result<_>>()?,
            ));
        }
        if !self.allowed("READ", target, None)?
            || !self.permitted_field("READ", target, field, None)?
        {
            return Err(err("not_found"));
        };
        let v = self.raw(target, field)?;
        if let Value::List(values) = v {
            return Ok(Value::List(self.visible(values)?));
        };
        Ok(v)
    }
    fn visible(&mut self, values: Vec<Value>) -> Result<Vec<Value>> {
        let mut out = vec![];
        for v in values {
            if self.allowed("READ", &v, None)? {
                out.push(v)
            }
        }
        Ok(out)
    }
    fn eval(&mut self, n: &Node, scope: &mut Scope, policy: bool) -> Result<Value> {
        self.step()?;
        if let Node::Symbol(s) = n {
            let mut path = s.split('.');
            let first = path.next().unwrap();
            let mut value = self.binding(first, scope, policy)?;
            for field in path {
                value = self.field(&value, field, policy)?
            }
            return Ok(value);
        }
        if !matches!(n, Node::List(_)) {
            return literal(n);
        }
        let args = n.args()?;
        let arg = |i: usize| args.get(i).ok_or_else(|| err("invalid_program"));
        match n.head() {
            "record" => {
                let mut values = BTreeMap::new();
                for f in args {
                    values.insert(f.head().into(), self.eval(f.arg(0)?, scope, policy)?);
                }
                Ok(Value::record(values))
            }
            "list" => Ok(Value::List(
                args.iter()
                    .map(|n| self.eval(n, scope, policy))
                    .collect::<Result<_>>()?,
            )),
            "and" | "or" => {
                let and = n.head() == "and";
                for arg in args {
                    let b = self.eval(arg, scope, policy)?.truth()?;
                    if b != and {
                        return Ok(Value::Bool(!and));
                    }
                }
                Ok(Value::Bool(and))
            }
            "not" => Ok(Value::Bool(!self.eval(arg(0)?, scope, policy)?.truth()?)),
            "eq" | "ne" => {
                let a = self.eval(arg(0)?, scope, policy)?;
                let b = self.eval(arg(1)?, scope, policy)?;
                Ok(Value::Bool(equal(&a, &b) == (n.head() == "eq")))
            }
            "gt" | "ge" | "lt" | "le" => {
                let a = self.eval(arg(0)?, scope, policy)?;
                let b = self.eval(arg(1)?, scope, policy)?;
                let ordering = match (&a, &b) {
                    (Value::Num(a), Value::Num(b)) => a.cmp(b),
                    (Value::Str(a), Value::Str(b)) => a.cmp(b),
                    _ => return Err(err("invalid_input")),
                };
                Ok(Value::Bool(match n.head() {
                    "gt" => ordering.is_gt(),
                    "ge" => !ordering.is_lt(),
                    "lt" => ordering.is_lt(),
                    _ => !ordering.is_gt(),
                }))
            }
            "add" | "sub" | "mul" | "div" => {
                let a = self.eval(arg(0)?, scope, policy)?.number()?.clone();
                let b = self.eval(arg(1)?, scope, policy)?.number()?.clone();
                let result = match n.head() {
                    "add" => a + b,
                    "sub" => a - b,
                    "mul" => a * b,
                    _ => {
                        if b == Number::from_integer(0.into()) {
                            return Err(err("invalid_input"));
                        };
                        a / b
                    }
                };
                crate::value::decimal(&result)?;
                Ok(Value::Num(result))
            }
            "contains" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let item = self.eval(arg(1)?, scope, policy)?;
                Ok(Value::Bool(values.list()?.iter().any(|v| equal(v, &item))))
            }
            "only" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let names = args[1..]
                    .iter()
                    .map(|n| n.text())
                    .collect::<Result<BTreeSet<_>>>()?;
                Ok(Value::Bool(values.list()?.iter().all(|v| {
                    v.text().is_ok_and(|s| names.contains(s.as_str()))
                })))
            }
            "count" => Ok(integer(self.eval(arg(0)?, scope, policy)?.list()?.len())),
            "concat" => {
                let values = args
                    .iter()
                    .map(|n| self.eval(n, scope, policy)?.text())
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Str(values.concat()))
            }
            "entity" => {
                let id = self.eval(arg(0)?, scope, policy)?;
                let Value::Id(entity, id) = id else {
                    return Err(err("invalid_input"));
                };
                if !self.changes.contains_key(&(entity.clone(), id.clone()))
                    && !exists(self.db, &entity, &id)?
                {
                    return Err(err("not_found"));
                };
                Ok(Value::reference(&entity, &id))
            }
            "entities" => {
                let entity = arg(0)?.text()?;
                if !self.p.entities.contains_key(entity) {
                    return Err(err("invalid_program"));
                };
                let sql = format!("SELECT id FROM {} ORDER BY id", q(entity));
                self.sql.push(sql.clone());
                let mut statement = self.db.prepare(&sql)?;
                let ids = statement
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let mut values = ids
                    .iter()
                    .map(|id| Value::reference(entity, id))
                    .collect::<Vec<_>>();
                for ((e, _), c) in &self.changes {
                    if e == entity && c.action == "CREATE" {
                        values.push(c.reference.clone())
                    }
                }
                if !policy {
                    values = self.visible(values)?
                };
                Ok(Value::List(values))
            }
            "new" => {
                let t = Type::Named(arg(0)?.text()?.into());
                let Type::Id(entity) = self.p.resolve(&t)? else {
                    return Err(err("invalid_program"));
                };
                Ok(Value::Id(entity.clone(), uuid::Uuid::new_v4().to_string()))
            }
            "as" => {
                let value = self.eval(arg(1)?, scope, policy)?;
                self.p
                    .validate(&Type::Named(arg(0)?.text()?.into()), value, None)
            }
            "first" | "last" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let limit = count(&self.eval(arg(1)?, scope, policy)?)?;
                if limit > 100000 {
                    return Err(err("limit"));
                };
                let values = values.list()?;
                let range = if n.head() == "last" {
                    values.len().saturating_sub(limit)..values.len()
                } else {
                    0..limit.min(values.len())
                };
                Ok(Value::List(values[range].to_vec()))
            }
            "live" => self.eval(arg(0)?, scope, policy),
            "single" => {
                let value = self.eval(arg(0)?, scope, policy)?;
                let values = value.list()?;
                if values.len() > 1 {
                    return Err(err("invalid_input"));
                };
                Ok(values.first().cloned().unwrap_or(Value::Null))
            }
            "order" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let field = arg(1)?
                    .text()?
                    .split_once('.')
                    .ok_or_else(|| err("invalid_program"))?
                    .1;
                let direction = arg(2)?.text()?;
                if !["ASC", "DESC"].contains(&direction) {
                    return Err(err("invalid_program"));
                };
                let mut pairs = values
                    .list()?
                    .iter()
                    .map(|v| Ok((self.field(v, field, policy)?, v.clone())))
                    .collect::<Result<Vec<_>>>()?;
                pairs.sort_by(|(a, _), (b, _)| {
                    let ordering = match (a, b) {
                        (Value::Num(a), Value::Num(b)) => a.cmp(b),
                        _ => a
                            .text()
                            .unwrap_or_default()
                            .cmp(&b.text().unwrap_or_default()),
                    };
                    if direction == "DESC" {
                        ordering.reverse()
                    } else {
                        ordering
                    }
                });
                Ok(Value::List(pairs.into_iter().map(|(_, v)| v).collect()))
            }
            "any" | "all" | "where" | "flatMap" | "sum" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let name = arg(1)?.text()?;
                let mut output = vec![];
                let mut sum = Number::from_integer(0.into());
                for value in values.list()? {
                    let mut local = scope.clone();
                    local.values.insert(name.into(), value.clone());
                    let result = self.eval(arg(2)?, &mut local, policy)?;
                    match n.head() {
                        "any" => {
                            if result.truth()? {
                                return Ok(Value::Bool(true));
                            }
                        }
                        "all" => {
                            if !result.truth()? {
                                return Ok(Value::Bool(false));
                            }
                        }
                        "where" => {
                            if result.truth()? {
                                output.push(value.clone())
                            }
                        }
                        "flatMap" => output.extend(result.list()?.to_vec()),
                        _ => sum += result.number()?,
                    };
                    if output.len() > 100000 {
                        return Err(err("limit"));
                    }
                }
                Ok(match n.head() {
                    "any" => Value::Bool(false),
                    "all" => Value::Bool(true),
                    "sum" => Value::Num(sum),
                    _ => Value::List(output),
                })
            }
            "map" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let name = arg(1)?.text()?;
                let mut out = vec![];
                for value in values.list()? {
                    let mut local = scope.clone();
                    local.values.insert(name.into(), value.clone());
                    for binding in &args[2..] {
                        local.values.remove(binding.head());
                        local
                            .defs
                            .insert(binding.head().into(), binding.arg(0)?.clone());
                    }
                    let mut fields = BTreeMap::new();
                    for binding in &args[2..] {
                        fields.insert(
                            binding.head().into(),
                            self.binding(binding.head(), &mut local, policy)?,
                        );
                    }
                    out.push(Value::record(fields));
                }
                Ok(Value::List(out))
            }
            "groupSum" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let key = arg(1)?.text()?;
                let quantity = arg(2)?.text()?;
                let brand = Type::Named(arg(3)?.text()?.into());
                let mut groups: Vec<(Value, Number)> = vec![];
                for item in values.list()? {
                    let id = self.field(item, key, policy)?;
                    let amount = self.field(item, quantity, policy)?.number()?.clone();
                    if let Some((_, n)) = groups.iter_mut().find(|(i, _)| equal(i, &id)) {
                        *n += amount
                    } else {
                        groups.push((id, amount))
                    }
                }
                Ok(Value::List(
                    groups
                        .into_iter()
                        .map(|(id, n)| {
                            Ok(Value::record(BTreeMap::from([
                                (key.into(), id),
                                (
                                    quantity.into(),
                                    self.p.validate(&brand, Value::Num(n), None)?,
                                ),
                            ])))
                        })
                        .collect::<Result<_>>()?,
                ))
            }
            "can" => {
                let action = arg(0)?.text()?;
                let target = self.eval(arg(1)?, scope, policy)?;
                Ok(Value::Bool(
                    Self::target_kind(&target).is_some() && self.allowed(action, &target, None)?,
                ))
            }
            "creates" => {
                if arg(0)?.text()? != "transaction" {
                    return Err(err("invalid_program"));
                };
                let entity = arg(1)?.text()?;
                Ok(Value::List(
                    self.changes
                        .iter()
                        .filter(|((e, _), c)| e == entity && c.action == "CREATE")
                        .map(|(_, c)| c.reference.clone())
                        .collect(),
                ))
            }
            "create" | "publish" => {
                let entity = arg(0)?.text()?.to_owned();
                let values = self.eval(arg(1)?, scope, policy)?.fields()?.clone();
                self.create(&entity, values)
            }
            "set" => {
                let path = arg(0)?.text()?;
                let (target, field) = path
                    .rsplit_once('.')
                    .ok_or_else(|| err("invalid_program"))?;
                let target = self.eval(&Node::Symbol(target.into()), scope, policy)?;
                let value = self.eval(arg(1)?, scope, policy)?;
                self.set(&target, field, value)?;
                Ok(target)
            }
            "delete" => {
                let target = self.eval(arg(0)?, scope, policy)?;
                let Value::Ref { entity, id, .. } = &target else {
                    return Err(err("invalid_input"));
                };
                if !exists(self.db, entity, id)? {
                    return Err(err("not_found"));
                };
                let key = (entity.clone(), id.clone());
                if self.changes.contains_key(&key) {
                    return Err(err("invalid_input"));
                };
                self.changes.insert(
                    key,
                    Change {
                        reference: target.clone(),
                        action: "DELETE".into(),
                        after: BTreeMap::new(),
                        changed: BTreeSet::new(),
                    },
                );
                Ok(target)
            }
            "ago" => {
                let duration = duration(arg(0)?.text()?)?;
                let now = chrono::DateTime::parse_from_rfc3339(&self.now)
                    .map_err(|_| err("invalid_input"))?;
                Ok(Value::Str(
                    (now - chrono::Duration::seconds(duration)).to_rfc3339(),
                ))
            }
            "since" => {
                let values = self.eval(arg(0)?, scope, policy)?;
                let time = self.eval(arg(1)?, scope, policy)?.text()?;
                let mut out = vec![];
                for value in values.list()? {
                    if self.field(value, "receivedAt", policy)?.text()? >= time {
                        out.push(value.clone())
                    }
                }
                Ok(Value::List(out))
            }
            _ => Err(err("invalid_program")),
        }
    }
    fn create(&mut self, entity: &str, mut fields: BTreeMap<String, Value>) -> Result<Value> {
        let schema = self
            .p
            .entities
            .get(entity)
            .ok_or_else(|| err("invalid_program"))?;
        for (name, f) in &schema.fields {
            if f.generated {
                if fields.contains_key(name) {
                    return Err(err("invalid_input"));
                };
                let v = if name == "id" {
                    Value::Id(entity.into(), uuid::Uuid::new_v4().to_string())
                } else {
                    Value::Str(self.now.clone())
                };
                fields.insert(name.clone(), v);
            }
        }
        let id = fields
            .get("id")
            .ok_or_else(|| err("invalid_input"))?
            .text()?;
        let reference = Value::reference(entity, &id);
        let row = self.p.validate(
            &stored_type(self.p, entity)?,
            Value::record(fields),
            Some(reference.clone()),
        )?;
        let key = (entity.into(), id.clone());
        if self.changes.contains_key(&key) || exists(self.db, entity, &id)? {
            return Err(err("conflict"));
        };
        let after = row.fields()?.clone();
        let changed = after.keys().cloned().collect();
        self.changes.insert(
            key,
            Change {
                reference: reference.clone(),
                action: "CREATE".into(),
                after,
                changed,
            },
        );
        Ok(reference)
    }
    fn set(&mut self, target: &Value, field: &str, value: Value) -> Result<()> {
        let Value::Ref { entity, id, .. } = target else {
            return Err(err("invalid_input"));
        };
        if field == "id" {
            return Err(err("invalid_input"));
        };
        let f = self
            .p
            .entities
            .get(entity)
            .and_then(|e| e.fields.get(field))
            .ok_or_else(|| err("invalid_program"))?;
        if f.relation.is_some() || f.generated {
            return Err(err("invalid_input"));
        };
        let value = self.p.validate(&f.ty, value, Some(target.clone()))?;
        let key = (entity.clone(), id.clone());
        self.raw(&target.pin(), field)?;
        let change = self.changes.entry(key).or_insert_with(|| Change {
            reference: target.clone(),
            action: "UPDATE".into(),
            after: BTreeMap::new(),
            changed: BTreeSet::new(),
        });
        if change.action == "DELETE" {
            return Err(err("invalid_input"));
        };
        change.after.insert(field.into(), value);
        change.changed.insert(field.into());
        Ok(())
    }
    fn authorize(&mut self) -> Result<()> {
        for change in self.changes.values().cloned().collect::<Vec<_>>() {
            let before = if change.action == "CREATE" {
                Value::Null
            } else {
                change.reference.pin()
            };
            let after = if change.action == "DELETE" {
                Value::Null
            } else {
                change.reference.clone()
            };
            let target = if change.action == "CREATE" {
                after.clone()
            } else {
                before.clone()
            };
            let facts = BTreeMap::from([
                ("before".into(), before),
                ("after".into(), after),
                (
                    "changed".into(),
                    Value::List(
                        change
                            .changed
                            .iter()
                            .map(|s| Value::Str(s.clone()))
                            .collect(),
                    ),
                ),
                ("transaction".into(), Value::Str("transaction".into())),
            ]);
            if !self.allowed(&change.action, &target, Some(facts.clone()))? {
                return Err(err("forbidden"));
            };
            for field in &change.changed {
                if !self.permitted_field(&change.action, &target, field, Some(facts.clone()))? {
                    return Err(err("forbidden"));
                };
            }
            for value in change.after.values() {
                self.references(value)?;
            }
        }
        Ok(())
    }
    fn references(&self, v: &Value) -> Result<()> {
        match v {
            Value::Ref { entity, id, .. } => {
                if let Some(c) = self.changes.get(&(entity.clone(), id.clone())) {
                    if c.action == "DELETE" {
                        return Err(err("invalid_input"));
                    }
                } else if !exists(self.db, entity, id)? {
                    return Err(err("invalid_input"));
                }
            }
            Value::List(v) => {
                for v in v {
                    self.references(v)?
                }
            }
            Value::Record { fields, .. } => {
                for v in fields.values() {
                    self.references(v)?
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn project(&mut self, t: &Type, value: Value) -> Result<Value> {
        let t = self.p.resolve(t)?.clone();
        match t {
            Type::Optional(inner) => {
                if value == Value::Null {
                    Ok(Value::Null)
                } else {
                    self.project(&inner, value)
                }
            }
            Type::List(inner, min, max) => {
                let values = self.visible(value.list()?.to_vec())?;
                if values.len() < min || max.is_some_and(|m| values.len() > m) {
                    return Err(err("invalid_output"));
                };
                Ok(Value::List(
                    values
                        .into_iter()
                        .map(|v| self.project(&inner, v))
                        .collect::<Result<_>>()?,
                ))
            }
            Type::Record(fs) => {
                let mut fields = BTreeMap::new();
                for (name, f) in fs {
                    let v = self.field(&value, &name, false)?;
                    fields.insert(name, self.project(&f.ty, v)?);
                }
                Ok(Value::record(fields))
            }
            Type::Named(entity) if self.p.entities.contains_key(&entity) => Err(Error::new(
                "invalid_program",
                "Entity outputs require an explicit record projection",
            )),
            Type::Id(ref brand) => {
                let value = if let Value::Ref { entity, id, .. } = value {
                    if entity != *brand {
                        return Err(err("invalid_output"));
                    };
                    Value::Id(entity, id)
                } else {
                    value
                };
                self.p
                    .validate(&t, value, None)
                    .map_err(|_| err("invalid_output"))
            }
            _ => self
                .p
                .validate(&t, value, None)
                .map_err(|_| err("invalid_output")),
        }
    }
    fn apply(&self) -> Result<()> {
        for ((entity, id), change) in &self.changes {
            match change.action.as_str() {
                "CREATE" => insert(self.db, entity, &change.after)?,
                "DELETE" => {
                    self.db
                        .execute(&format!("DELETE FROM {} WHERE id=?1", q(entity)), [id])?;
                }
                "UPDATE" => {
                    let setters = change
                        .after
                        .keys()
                        .enumerate()
                        .map(|(i, k)| format!("{}=?{}", q(k), i + 1))
                        .collect::<Vec<_>>()
                        .join(",");
                    let mut values = change
                        .after
                        .values()
                        .map(storage)
                        .collect::<Result<Vec<_>>>()?;
                    values.push(Some(id.clone()));
                    self.db.execute(
                        &format!(
                            "UPDATE {} SET {setters} WHERE id=?{}",
                            q(entity),
                            values.len()
                        ),
                        rusqlite::params_from_iter(values),
                    )?;
                }
                _ => return Err(err("invalid_program")),
            }
        }
        Ok(())
    }
}
pub(crate) fn duration(s: &str) -> Result<i64> {
    if !s.is_ascii() {
        return Err(err("invalid_input"));
    }
    let (n, unit) = s.split_at(s.len().checked_sub(1).ok_or_else(|| err("invalid_input"))?);
    let amount = n.parse::<i64>().map_err(|_| err("invalid_input"))?;
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return Err(err("invalid_input")),
    };
    amount
        .checked_mul(multiplier)
        .filter(|n| *n >= 0 && *n <= 315360000)
        .ok_or_else(|| err("invalid_input"))
}
