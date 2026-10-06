//! Evaluates one operation inside a transaction the caller already opened.
//! Applies grants, field visibility, and staged writes. Does not commit or roll back.

use crate::{
    Error, Result,
    program::{Program, Type, literal},
    syntax::Node,
    value::{Number, Value, count, equal, integer, number},
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};

use super::store::{err, exists, insert, next_id, q, storage, stored_type};

#[derive(Clone, Debug)]
pub(in crate::engine) struct Change {
    pub(in crate::engine) reference: Value,
    pub(in crate::engine) action: String,
    pub(in crate::engine) after: BTreeMap<String, Value>,
    pub(in crate::engine) changed: BTreeSet<String>,
}
#[derive(Clone, Default)]
pub(in crate::engine) struct Scope {
    pub(in crate::engine) defs: BTreeMap<String, Node>,
    pub(in crate::engine) values: BTreeMap<String, Value>,
    pub(in crate::engine) visiting: BTreeSet<String>,
}
pub(in crate::engine) struct Session<'a> {
    pub(in crate::engine) observer: &'a crate::observability::Observability,
    pub(in crate::engine) base_path: &'a str,
    pub(in crate::engine) p: &'a Program,
    pub(in crate::engine) db: &'a Connection,
    pub(in crate::engine) actor: Value,
    pub(in crate::engine) actor_type: String,
    pub(in crate::engine) changes: BTreeMap<(String, i64), Change>,
    pub(in crate::engine) cache: BTreeMap<(String, i64, String), Value>,
    pub(in crate::engine) permission_stack: BTreeSet<(String, String, String)>,
    pub(in crate::engine) steps: usize,
    pub(in crate::engine) now: String,
    pub(in crate::engine) sql: Vec<String>,
    pub(in crate::engine) invocations: Vec<(String, Value)>,
    pub(in crate::engine) blobs: BTreeMap<i64, Vec<u8>>,
}

impl Session<'_> {
    pub(in crate::engine) fn allocate_id(&self, entity: &str) -> Result<i64> {
        let floor = self
            .changes
            .keys()
            .filter(|(kind, _)| kind == entity)
            .map(|(_, id)| *id)
            .max()
            .unwrap_or(0);
        next_id(self.db, entity, floor)
    }
    pub(in crate::engine) fn step(&mut self) -> Result<()> {
        self.steps += 1;
        if self.steps > 100000 {
            return Err(err("limit"));
        };
        Ok(())
    }
    pub(in crate::engine) fn binding(
        &mut self,
        name: &str,
        scope: &mut Scope,
        policy: bool,
    ) -> Result<Value> {
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
    pub(in crate::engine) fn raw(&mut self, target: &Value, field: &str) -> Result<Value> {
        self.step()?;
        match target {
            Value::Image(image) => match field {
                "width" => Ok(integer(image.width as usize)),
                "height" => Ok(integer(image.height as usize)),
                _ => Err(err("invalid_input")),
            },
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
                if let Some(change) = self.changes.get(&(entity.clone(), *id))
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
                    let mut span = self.observer.span("io", "sqlite.read_relation");
                    let mut statement = self.db.prepare(&sql)?;
                    let ids = statement
                        .query_map([id], |r| r.get::<_, i64>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    span.success();
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
                                    ids.push(*id);
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
                    let key = (entity.clone(), *id, field.into());
                    if let Some(v) = self.cache.get(&key) {
                        return Ok(if *before { v.pin() } else { v.clone() });
                    };
                    let sql = format!("SELECT {} FROM {} WHERE id=?1", q(field), q(entity));
                    self.sql.push(sql.clone());
                    let mut span = self.observer.span("io", "sqlite.read_field");
                    let stored: Option<Option<String>> = self
                        .db
                        .query_row(&sql, [id], |r| {
                            use rusqlite::types::ValueRef;
                            Ok(match r.get_ref(0)? {
                                ValueRef::Null => None,
                                ValueRef::Integer(n) => Some(n.to_string()),
                                ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
                                _ => None,
                            })
                        })
                        .optional()?;
                    span.success();
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
                                Value::Num(number(&s)?)
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
    pub(in crate::engine) fn target_kind(target: &Value) -> Option<String> {
        match target {
            Value::Ref { entity, .. } => Some(entity.clone()),
            Value::Record { kind, .. } => kind.clone(),
            _ => None,
        }
    }
    pub(in crate::engine) fn allowed(
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
    pub(in crate::engine) fn granted(
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
    pub(in crate::engine) fn permitted_field(
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
    pub(in crate::engine) fn field(
        &mut self,
        target: &Value,
        field: &str,
        policy: bool,
    ) -> Result<Value> {
        if field == "id"
            && equal(target, &self.actor)
            && let Value::Ref { entity, id, .. } = target
        {
            return Ok(Value::Id(entity.clone(), *id));
        }
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
    pub(in crate::engine) fn visible(&mut self, values: Vec<Value>) -> Result<Vec<Value>> {
        let mut out = vec![];
        for v in values {
            if self.allowed("READ", &v, None)? {
                out.push(v)
            }
        }
        Ok(out)
    }
    pub(in crate::engine) fn eval(
        &mut self,
        n: &Node,
        scope: &mut Scope,
        policy: bool,
    ) -> Result<Value> {
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
                if matches!((&a,&b),(Value::BrandedNumber(a,_),Value::BrandedNumber(b,_)) if a!=b) {
                    return Err(err("invalid_input"));
                };
                Ok(Value::Bool(equal(&a, &b) == (n.head() == "eq")))
            }
            "gt" | "ge" | "lt" | "le" => {
                let a = self.eval(arg(0)?, scope, policy)?;
                let b = self.eval(arg(1)?, scope, policy)?;
                let ordering = match (&a, &b) {
                    (Value::Num(a), Value::Num(b))
                    | (Value::BrandedNumber(_, a), Value::Num(b))
                    | (Value::Num(a), Value::BrandedNumber(_, b)) => a.cmp(b),
                    (Value::BrandedNumber(a, x), Value::BrandedNumber(b, y)) if a == b => x.cmp(y),
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
                if !self.changes.contains_key(&(entity.clone(), id))
                    && !exists(self.db, &entity, &id)?
                {
                    return Err(err("not_found"));
                };
                Ok(Value::reference(&entity, &id))
            }
            "page" => {
                let entity = arg(0)?.text()?;
                if !self.p.entities.contains_key(entity) {
                    return Err(err("invalid_program"));
                }
                let cursor = self.eval(arg(1)?, scope, policy)?;
                let after = match cursor {
                    Value::Null => 0,
                    Value::Id(ref brand, id) if brand == entity => id,
                    _ => return Err(err("invalid_input")),
                };
                let limit = count(&self.eval(arg(2)?, scope, policy)?)?;
                if limit == 0 || limit > 1000 {
                    return Err(err("limit"));
                }
                let mut output = Vec::new();
                let mut cursor = after;
                loop {
                    let sql = format!(
                        "SELECT id FROM {} WHERE id>?1 ORDER BY id LIMIT 256",
                        q(entity)
                    );
                    self.sql.push(sql.clone());
                    let ids = self
                        .db
                        .prepare(&sql)?
                        .query_map([cursor], |row| row.get::<_, i64>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    if ids.is_empty() {
                        break;
                    }
                    for id in &ids {
                        self.step()?;
                        cursor = *id;
                        let reference = Value::reference(entity, id);
                        if policy || self.allowed("READ", &reference, None)? {
                            output.push(reference);
                        }
                        if output.len() == limit {
                            break;
                        }
                    }
                    if output.len() == limit || ids.len() < 256 {
                        break;
                    }
                }
                for ((kind, id), change) in &self.changes.clone() {
                    if kind == entity
                        && *id > after
                        && change.action == "CREATE"
                        && (policy || self.allowed("READ", &change.reference, None)?)
                    {
                        output.push(change.reference.clone());
                    }
                }
                output.sort_by_key(|value| value.id().unwrap_or(0));
                output.truncate(limit);
                Ok(Value::List(output))
            }
            "entities" => {
                let entity = arg(0)?.text()?;
                if !self.p.entities.contains_key(entity) {
                    return Err(err("invalid_program"));
                };
                let sql = format!("SELECT id FROM {} ORDER BY id LIMIT 100001", q(entity));
                self.sql.push(sql.clone());
                let mut span = self.observer.span("io", "sqlite.scan_entities");
                let mut statement = self.db.prepare(&sql)?;
                let ids = statement
                    .query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if ids.len() > 100000 {
                    return Err(err("limit"));
                }
                span.success();
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
                Ok(Value::Id(entity.clone(), self.allocate_id(entity)?))
            }
            "as" => {
                let value = self.eval(arg(1)?, scope, policy)?;
                let value = if let Value::BrandedNumber(_, number) = value {
                    Value::Num(number)
                } else {
                    value
                };
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
            "invoke" => {
                let name = arg(0)?.text()?;
                let mut span = self.observer.span("plugin", name);
                let method = self
                    .p
                    .plugins
                    .get(name)
                    .ok_or_else(|| err("invalid_program"))?
                    .clone();
                let input = self.eval(arg(1)?, scope, policy)?;
                let input = self.materialize(&Type::Record(method.inputs), input)?;
                let output = match name {
                    "Payment.createUrl" => Value::record(BTreeMap::from([(
                        "url".into(),
                        Value::Str(format!(
                            "/payments/{}",
                            input
                                .fields()?
                                .get("orderId")
                                .ok_or_else(|| err("invalid_input"))?
                                .text()?
                        )),
                    )])),
                    "Image.resize" => {
                        let fields = input.fields()?;
                        let Value::Image(image) =
                            fields.get("image").ok_or_else(|| err("invalid_input"))?
                        else {
                            return Err(err("invalid_input"));
                        };
                        let width =
                            count(fields.get("width").ok_or_else(|| err("invalid_input"))?)?;
                        let height =
                            count(fields.get("height").ok_or_else(|| err("invalid_input"))?)?;
                        Value::Image(image.resize(width as u32, height as u32)?)
                    }
                    "Files.put" => {
                        let Value::Image(image) = input
                            .fields()?
                            .get("file")
                            .ok_or_else(|| err("invalid_input"))?
                        else {
                            return Err(err("invalid_input"));
                        };
                        let id = self.allocate_id("File")?;
                        let reference = self.create(
                            "File",
                            BTreeMap::from([
                                ("id".into(), Value::Id("File".into(), id)),
                                (
                                    "url".into(),
                                    Value::Str(format!("{}/api/files/{id}", self.base_path)),
                                ),
                            ]),
                        )?;
                        self.blobs.insert(id, image.bytes.clone());
                        reference
                    }
                    _ => return Err(err("invalid_program")),
                };
                self.invocations.push((name.into(), input));
                let result = self.p.validate(&method.output, output, None);
                if result.is_ok() {
                    span.success();
                }
                result
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
                        (Value::Num(a), Value::Num(b))
                        | (Value::BrandedNumber(_, a), Value::Num(b))
                        | (Value::Num(a), Value::BrandedNumber(_, b)) => a.cmp(b),
                        (Value::BrandedNumber(a, x), Value::BrandedNumber(b, y)) if a == b => {
                            x.cmp(y)
                        }
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
            "expectVersion" => {
                let target = self.eval(arg(0)?, scope, policy)?;
                let expected = self.eval(arg(1)?, scope, policy)?;
                let original = self.field(&target.pin(), "version", false)?;
                if !equal(&original, &expected) {
                    return Err(Error::new(
                        "conflict",
                        "This record changed; reload before saving",
                    ));
                }
                Ok(target)
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
                let key = (entity.clone(), *id);
                if self.changes.contains_key(&key) {
                    return Err(err("invalid_input"));
                };
                self.changes.insert(
                    key,
                    Change {
                        reference: target.clone(),
                        action: "DELETE".into(),
                        after: BTreeMap::new(),
                        changed: self.p.entities[entity]
                            .fields
                            .iter()
                            .filter(|(_, f)| f.relation.is_none())
                            .map(|(name, _)| name.clone())
                            .collect(),
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
    pub(in crate::engine) fn create(
        &mut self,
        entity: &str,
        mut fields: BTreeMap<String, Value>,
    ) -> Result<Value> {
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
                    Value::Id(entity.into(), self.allocate_id(entity)?)
                } else {
                    Value::Str(self.now.clone())
                };
                fields.insert(name.clone(), v);
            }
        }
        let id = fields.get("id").ok_or_else(|| err("invalid_input"))?.id()?;
        let reference = Value::reference(entity, &id);
        let row = self.p.validate(
            &stored_type(self.p, entity)?,
            Value::record(fields),
            Some(reference.clone()),
        )?;
        let key = (entity.into(), id);
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
    pub(in crate::engine) fn set(
        &mut self,
        target: &Value,
        field: &str,
        value: Value,
    ) -> Result<()> {
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
        let key = (entity.clone(), *id);
        if !self.changes.get(&key).is_some_and(|c| c.action == "CREATE") {
            self.raw(&target.pin(), field)?;
        }
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
    pub(in crate::engine) fn materialize(&mut self, t: &Type, value: Value) -> Result<Value> {
        let resolved = self.p.resolve(t)?.clone();
        let value = match resolved {
            Type::Record(fields) => {
                if let Value::Record {
                    fields: values,
                    kind: None,
                    parent: None,
                } = &value
                    && values.keys().any(|key| !fields.contains_key(key))
                {
                    return Err(err("invalid_input"));
                };
                let mut output = BTreeMap::new();
                for (name, field) in fields {
                    let missing = matches!(&value,Value::Record{fields,kind:None,parent:None} if !fields.contains_key(&name));
                    let field_value = if missing {
                        Value::Null
                    } else {
                        self.field(&value, &name, false)?
                    };
                    output.insert(name, self.materialize(&field.ty, field_value)?);
                }
                Value::record(output)
            }
            Type::List(inner, _, _) => {
                let values = self.visible(value.list()?.to_vec())?;
                Value::List(
                    values
                        .into_iter()
                        .map(|value| self.materialize(&inner, value))
                        .collect::<Result<_>>()?,
                )
            }
            Type::Optional(inner) if value != Value::Null => self.materialize(&inner, value)?,
            _ => value,
        };
        self.p.validate(t, value, None)
    }
    pub(in crate::engine) fn authorize(&mut self) -> Result<()> {
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
        for (name, args) in self.invocations.clone() {
            let facts = BTreeMap::from([
                ("args".into(), args),
                ("transaction".into(), Value::Str("transaction".into())),
            ]);
            if !self.granted("INVOKE", &name, &Value::Null, Some(facts))? {
                return Err(err("forbidden"));
            };
        }
        Ok(())
    }
    pub(in crate::engine) fn references(&self, v: &Value) -> Result<()> {
        match v {
            Value::Ref { entity, id, .. } => {
                if let Some(c) = self.changes.get(&(entity.clone(), *id)) {
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
    pub(in crate::engine) fn project(&mut self, t: &Type, value: Value) -> Result<Value> {
        if matches!(t, Type::Named(_)) && matches!(self.p.resolve(t)?, Type::Number { .. }) {
            return self
                .p
                .validate(t, value, None)
                .map_err(|_| err("invalid_output"));
        };
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
    pub(in crate::engine) fn apply(&self) -> Result<()> {
        if self.changes.is_empty() && self.blobs.is_empty() {
            return Ok(());
        }
        let mut span = self.observer.span("io", "sqlite.apply");
        let result = self.apply_inner();
        if result.is_ok() {
            span.success();
        }
        result
    }
    pub(in crate::engine) fn apply_inner(&self) -> Result<()> {
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
                    values.push(Some(id.to_string()));
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
        for (id, bytes) in &self.blobs {
            self.db.execute(
                "INSERT INTO _flow_blobs(file_id,bytes) VALUES(?1,?2)",
                rusqlite::params![id, bytes],
            )?;
        }
        for ((entity, _), change) in &self.changes {
            let Some(stream) = self.p.streams.get(entity) else {
                continue;
            };
            if change.action != "CREATE" {
                continue;
            };
            let now = chrono::DateTime::parse_from_rfc3339(&self.now)
                .map_err(|_| err("invalid_input"))?;
            let cutoff = (now - chrono::Duration::seconds(stream.duration)).to_rfc3339();
            self.db.execute(
                &format!(
                    "DELETE FROM {} WHERE json_extract(receivedAt,'$') < ?1",
                    q(entity)
                ),
                [cutoff],
            )?;
            let mut predicates = vec![];
            let mut values = vec![];
            for field in stream
                .topic
                .split('/')
                .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
            {
                predicates.push(format!("{}=?{}", q(field), values.len() + 1));
                values.push(storage(
                    change
                        .after
                        .get(field)
                        .ok_or_else(|| err("invalid_program"))?,
                )?);
            }
            let clause = if predicates.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", predicates.join(" AND "))
            };
            self.db.execute(&format!("DELETE FROM {} WHERE id IN (SELECT id FROM {} {} ORDER BY rowid DESC LIMIT -1 OFFSET {})",q(entity),q(entity),clause,stream.max_messages),rusqlite::params_from_iter(values))?;
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
