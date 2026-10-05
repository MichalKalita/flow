use crate::{
    Error, Result,
    syntax::{Node, parse},
    value::{Number, Value, number},
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub enum Type {
    Image {
        max_bytes: usize,
        width: u32,
        height: u32,
    },
    String,
    Bool,
    DateTime,
    Named(String),
    Id(String),
    Number {
        min: Number,
        max: Number,
        scale: u32,
    },
    List(Box<Type>, usize, Option<usize>),
    Optional(Box<Type>),
    Record(BTreeMap<String, Field>),
    Enum(Vec<String>),
}
#[derive(Clone, Debug)]
pub struct Field {
    pub ty: Type,
    pub relation: Option<(String, String)>,
    pub unique: bool,
    pub generated: bool,
}
#[derive(Clone, Debug)]
pub struct Entity {
    pub fields: BTreeMap<String, Field>,
}
#[derive(Clone, Debug)]
pub struct Method {
    pub inputs: BTreeMap<String, Field>,
    pub output: Type,
    pub mode: String,
}
#[derive(Clone, Debug)]
pub struct Stream {
    pub topic: String,
    pub duration: i64,
    pub max_messages: usize,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub source: String,
    pub adapter: String,
    pub actor_id: i64,
    pub condition: Node,
}
#[derive(Clone, Debug)]
pub struct Operation {
    pub name: String,
    pub mutation: bool,
    pub inputs: BTreeMap<String, (Type, Option<Node>)>,
    pub output: Type,
    pub bindings: BTreeMap<String, Node>,
    pub binding_order: Vec<String>,
    pub result: Node,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub stream: Option<String>,
    pub event: Option<Event>,
}
#[derive(Clone, Debug)]
pub struct Grant {
    pub actor: String,
    pub target: String,
    pub action: String,
    pub condition: Node,
}
#[derive(Clone, Debug)]
pub struct Auth {
    pub alias: String,
    pub entity: String,
    pub mode: String,
    pub field: String,
    pub issuer: String,
    pub audience: String,
}
#[derive(Clone, Debug)]
pub struct Program {
    pub types: BTreeMap<String, Type>,
    pub plugins: BTreeMap<String, Method>,
    pub entities: BTreeMap<String, Entity>,
    pub operations: Vec<Operation>,
    pub grants: Vec<Grant>,
    pub auth: Vec<Auth>,
    pub http_auth: Vec<String>,
    pub transports: BTreeMap<String, Vec<String>>,
    pub streams: BTreeMap<String, Stream>,
    pub seeds: Vec<(String, Vec<Node>)>,
    pub source: String,
}
fn fail(s: impl Into<String>) -> Error {
    Error::new("invalid_program", s)
}
pub fn ident(s: &str) -> Result<String> {
    if !s.is_empty()
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()))
    {
        Ok(s.into())
    } else {
        Err(fail(format!("Invalid identifier {s}")))
    }
}
fn unique<T>(map: &mut BTreeMap<String, T>, key: String, value: T) -> Result<()> {
    if map.insert(key.clone(), value).is_some() {
        Err(fail(format!("Duplicate {key}")))
    } else {
        Ok(())
    }
}
fn opt<'a>(node: &'a Node, name: &str) -> Result<&'a Node> {
    node.option(name)
        .ok_or_else(|| fail(format!("Missing {name} in {}", node.head())))
}
fn constant(n: &Node) -> Result<Number> {
    if let Node::Number(s) = n {
        return number(s);
    };
    let a = constant(n.arg(0)?)?;
    let b = constant(n.arg(1)?)?;
    let value = match n.head() {
        "add" => a + b,
        "sub" => a - b,
        "mul" => a * b,
        "pow" => {
            let e = b
                .to_integer()
                .to_i32()
                .filter(|e| b.is_integer() && *e >= 0 && *e <= 100)
                .ok_or_else(|| fail("Invalid exponent"))?;
            a.pow(e)
        }
        _ => return Err(fail("Invalid numeric constant")),
    };
    crate::value::decimal(&value)?;
    Ok(value)
}
fn size(n: &Node) -> Result<usize> {
    constant(n)?
        .to_integer()
        .to_usize()
        .filter(|_| constant(n).unwrap().is_integer())
        .filter(|n| *n <= 100000)
        .ok_or_else(|| fail("Invalid collection limit"))
}
fn fields(nodes: &[Node]) -> Result<BTreeMap<String, Field>> {
    let mut fields = BTreeMap::new();
    for f in nodes {
        if f.head() != "field" {
            return Err(fail("Expected field"));
        };
        let name = ident(f.arg(0)?.text()?)?;
        let ty = ty(f.arg(1)?)?;
        let relation = f
            .option("inverse")
            .or_else(|| f.option("stream"))
            .map(|r| {
                let path = r.arg(0)?.text()?;
                let (entity, field) = path
                    .split_once('.')
                    .ok_or_else(|| fail("Relation needs Entity.field"))?;
                Ok::<_, Error>((ident(entity)?, ident(field)?))
            })
            .transpose()?;
        unique(
            &mut fields,
            name,
            Field {
                ty,
                relation,
                unique: f.option("unique").is_some(),
                generated: f.option("receivedAt").is_some(),
            },
        )?;
    }
    Ok(fields)
}
fn ty(n: &Node) -> Result<Type> {
    if let Node::Symbol(s) = n {
        return Ok(match s.as_str() {
            "String" => Type::String,
            "Bool" => Type::Bool,
            "DateTime" => Type::DateTime,
            _ => Type::Named(ident(s)?),
        });
    };
    Ok(match n.head() {
        "image" => {
            let max_bytes = constant(opt(n, "maxBytes")?.arg(0)?)?
                .to_integer()
                .to_usize()
                .filter(|v| *v > 0 && *v <= 64 * 1024 * 1024)
                .ok_or_else(|| fail("Invalid image byte limit"))?;
            let width = size(opt(n, "maxWidth")?.arg(0)?)? as u32;
            let height = size(opt(n, "maxHeight")?.arg(0)?)? as u32;
            if width == 0 || height == 0 {
                return Err(fail("Image dimensions must be positive"));
            };
            Type::Image {
                max_bytes,
                width,
                height,
            }
        }
        "id" => Type::Id(ident(n.arg(0)?.text()?)?),
        "integer" | "decimal" => {
            let range = opt(n, "range")?;
            let min = constant(range.arg(0)?)?;
            let max = constant(range.arg(1)?)?;
            let scale = if n.head() == "integer" {
                0
            } else {
                size(opt(n, "scale")?.arg(0)?)? as u32
            };
            if min > max || scale > 100 || (scale == 0 && (!min.is_integer() || !max.is_integer()))
            {
                return Err(fail("Invalid range/scale"));
            };
            Type::Number { min, max, scale }
        }
        "list" => Type::List(
            Box::new(ty(n.arg(0)?)?),
            n.option("min")
                .map(|n| size(n.arg(0)?))
                .transpose()?
                .unwrap_or(0),
            n.option("max").map(|n| size(n.arg(0)?)).transpose()?,
        ),
        "optional" => Type::Optional(Box::new(ty(n.arg(0)?)?)),
        "record" => Type::Record(fields(n.args()?)?),
        _ => return Err(fail(format!("Unsupported type {}", n.head()))),
    })
}
impl Program {
    pub fn compile(source: &str) -> Result<Self> {
        let nodes = parse(source)?;
        let mut p = Self {
            types: BTreeMap::new(),
            plugins: BTreeMap::new(),
            entities: BTreeMap::new(),
            operations: vec![],
            grants: vec![],
            auth: vec![],
            http_auth: vec![],
            transports: BTreeMap::new(),
            streams: BTreeMap::new(),
            seeds: vec![],
            source: source.into(),
        };
        let mut permission_nodes = vec![];
        let mut op_nodes = vec![];
        for n in &nodes {
            match n.head() {
                "type" => {
                    let name = ident(n.arg(0)?.text()?)?;
                    unique(&mut p.types, name, ty(n.arg(1)?)?)?;
                }
                "enum" => {
                    let name = ident(n.arg(0)?.text()?)?;
                    let values = n.args()?[1..]
                        .iter()
                        .map(|n| ident(n.text()?))
                        .collect::<Result<Vec<_>>>()?;
                    if values.is_empty()
                        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
                    {
                        return Err(fail("Empty or duplicate enum"));
                    };
                    unique(&mut p.types, name, Type::Enum(values))?;
                }
                "entity" | "stream" => {
                    let name = ident(n.arg(0)?.text()?)?;
                    let field_nodes: Vec<_> = n.args()?[1..]
                        .iter()
                        .filter(|n| n.head() == "field")
                        .cloned()
                        .collect();
                    let mut fs = fields(&field_nodes)?;
                    if n.head() == "stream" {
                        let topic = opt(n, "mqtt")?.arg(0)?.text()?.to_owned();
                        let history = opt(n, "history")?;
                        let duration =
                            crate::engine::duration(opt(history, "duration")?.arg(0)?.text()?)?;
                        let max_messages = size(opt(history, "maxMessages")?.arg(0)?)?;
                        if max_messages == 0 || duration == 0 {
                            return Err(fail("Stream history must be positive"));
                        };
                        p.streams.insert(
                            name.clone(),
                            Stream {
                                topic,
                                duration,
                                max_messages,
                            },
                        );
                        fs.entry("id".into()).or_insert(Field {
                            ty: Type::Id(name.clone()),
                            relation: None,
                            unique: false,
                            generated: true,
                        });
                        fs.entry("receivedAt".into()).or_insert(Field {
                            ty: Type::DateTime,
                            relation: None,
                            unique: false,
                            generated: true,
                        });
                        fs.get_mut("receivedAt").unwrap().generated = true;
                    }
                    unique(&mut p.entities, name, Entity { fields: fs })?;
                }
                "permissions" => permission_nodes.push(n),
                "query" | "mutate" => op_nodes.push(n),
                "seed" => p
                    .seeds
                    .push((ident(n.arg(0)?.text()?)?, opt(n, "rows")?.args()?.to_vec())),
                "transport" => {
                    let name = ident(n.arg(0)?.text()?)?;
                    let aliases = opt(n, "auth")?
                        .args()?
                        .iter()
                        .map(|n| Ok(n.text()?.to_owned()))
                        .collect::<Result<Vec<_>>>()?;
                    if name == "HTTP" {
                        p.http_auth = aliases.clone()
                    };
                    unique(&mut p.transports, name, aliases)?;
                }
                "auth" => {
                    for a in n.args()? {
                        let alias = ident(a.head())?;
                        let entity = ident(a.arg(0)?.text()?)?;
                        if a.head() == "anonymous" {
                            p.auth.push(Auth {
                                alias,
                                entity,
                                mode: "anonymous".into(),
                                field: String::new(),
                                issuer: String::new(),
                                audience: String::new(),
                            });
                            continue;
                        }
                        let mode = if a.option("jwt").is_some() {
                            "jwt"
                        } else if a.option("apiKey").is_some() {
                            "apiKey"
                        } else if a.option("certificate").is_some() {
                            "certificate"
                        } else {
                            return Err(fail("Unknown authentication adapter"));
                        };
                        let eq = opt(a, "entity")?.arg(0)?;
                        if eq.head() != "eq" {
                            return Err(fail("Authentication requires unique equality lookup"));
                        };
                        let path = eq.arg(0)?.text()?;
                        let (target, field) = path
                            .split_once('.')
                            .ok_or_else(|| fail("Invalid auth lookup"))?;
                        if target != entity {
                            return Err(fail("Auth lookup brand mismatch"));
                        };
                        let claim = eq.arg(1)?.text()?;
                        if claim
                            != match mode {
                                "jwt" => "claims.sub",
                                "apiKey" => "credential.hash",
                                _ => "certificate.fingerprint",
                            }
                        {
                            return Err(fail("Invalid auth claim"));
                        };
                        let (issuer, audience) = if mode == "jwt" {
                            let jwt = opt(a, "jwt")?;
                            (
                                opt(jwt, "issuer")?.arg(0)?.text()?.into(),
                                opt(jwt, "audience")?.arg(0)?.text()?.into(),
                            )
                        } else {
                            (String::new(), String::new())
                        };
                        p.auth.push(Auth {
                            alias,
                            entity,
                            mode: mode.into(),
                            field: ident(field)?,
                            issuer,
                            audience,
                        });
                    }
                }
                "plugin" => {
                    let plugin = ident(n.arg(0)?.text()?)?;
                    for method in &n.args()?[1..] {
                        let name = format!("{plugin}.{}", ident(method.head())?);
                        let mut inputs = BTreeMap::new();
                        for input in method.args()?.iter().filter(|n| n.head() == "input") {
                            unique(
                                &mut inputs,
                                ident(input.arg(0)?.text()?)?,
                                Field {
                                    ty: ty(input.arg(1)?)?,
                                    relation: None,
                                    unique: false,
                                    generated: false,
                                },
                            )?;
                        }
                        let output = ty(opt(method, "output")?.arg(0)?)?;
                        let mode = if method.option("pure").is_some() {
                            "pure"
                        } else if method.option("transactional").is_some() {
                            "transactional"
                        } else {
                            "external"
                        };
                        unique(
                            &mut p.plugins,
                            name,
                            Method {
                                inputs,
                                output,
                                mode: mode.into(),
                            },
                        )?;
                    }
                }
                _ => return Err(fail(format!("Unknown declaration {}", n.head()))),
            }
        }
        for method in p.plugins.values() {
            for field in method.inputs.values() {
                p.check_type(&field.ty, false, &mut BTreeSet::new())?;
            }
            p.check_type(&method.output, false, &mut BTreeSet::new())?;
        }
        for name in p.types.keys() {
            if p.entities.contains_key(name) {
                return Err(fail("Duplicate type/entity name"));
            };
            p.check_type(&Type::Named(name.clone()), false, &mut BTreeSet::new())?;
        }
        for (name, e) in &p.entities {
            let id = e
                .fields
                .get("id")
                .ok_or_else(|| fail(format!("{name} needs branded id")))?;
            if !matches!(p.resolve(&id.ty)?,Type::Id(target) if target==name) {
                return Err(fail("Entity id brand mismatch"));
            };
            for f in e.fields.values() {
                p.check_type(&f.ty, f.relation.is_some(), &mut BTreeSet::new())?;
                if let Some((entity, field)) = &f.relation {
                    let related = p
                        .entities
                        .get(entity)
                        .and_then(|e| e.fields.get(field))
                        .ok_or_else(|| fail("Unknown relation"))?;
                    if !matches!(p.resolve(&related.ty)?,Type::Named(target) if target==name) {
                        return Err(fail("Inverse relation brand mismatch"));
                    };
                    match p.resolve(&f.ty)? {
                        Type::List(inner, _, _) if matches!(p.resolve(inner)?,Type::Named(t) if t==entity) =>
                            {}
                        _ => return Err(fail("Inverse needs matching list")),
                    };
                }
            }
        }
        let aliases = p.auth.iter().map(|a| &a.alias).collect::<BTreeSet<_>>();
        if aliases.len() != p.auth.len() {
            return Err(fail("Duplicate auth alias"));
        };
        for alias in p.transports.values().flatten() {
            if !aliases.contains(alias) {
                return Err(fail("Unknown HTTP auth alias"));
            };
        }
        for a in &p.auth {
            if a.mode != "anonymous" {
                let f = p
                    .entities
                    .get(&a.entity)
                    .and_then(|e| e.fields.get(&a.field))
                    .ok_or_else(|| fail("Unknown identity field"))?;
                if !f.unique || !matches!(p.resolve(&f.ty)?, Type::String | Type::Optional(_)) {
                    return Err(fail("Auth requires unique text field"));
                };
            }
        }
        for n in permission_nodes {
            let actor = n.arg(0)?.text()?.to_owned();
            if actor != "*" && actor != "Anonymous" && !p.entities.contains_key(&actor) {
                return Err(fail("Unknown permission actor"));
            };
            for target in &n.args()?[1..] {
                let target_name = target.head().to_owned();
                p.permission_target(&target_name)?;
                let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
                let mut rules = vec![];
                for rule in target.args()? {
                    let action = rule.head().to_owned();
                    if !["READ", "USE", "CREATE", "UPDATE", "DELETE", "INVOKE"]
                        .contains(&action.as_str())
                    {
                        return Err(fail("Unknown action"));
                    };
                    let condition = opt(rule, "when")?.arg(0)?.clone();
                    check_expr(&condition, true)?;
                    if ["READ", "USE"].contains(&action.as_str()) && uses_state(&condition) {
                        return Err(fail("Read grant requires unavailable write state"));
                    };
                    let includes = rule
                        .option("includes")
                        .map(|n| {
                            n.args()?
                                .iter()
                                .map(|n| Ok(n.text()?.to_owned()))
                                .collect::<Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default();
                    for included in &includes {
                        if !["READ", "USE", "CREATE", "UPDATE", "DELETE", "INVOKE"]
                            .contains(&included.as_str())
                        {
                            return Err(fail("Unknown inherited action"));
                        };
                    }
                    graph.entry(action.clone()).or_default().extend(includes);
                    rules.push((action, condition));
                }
                for (action, condition) in rules {
                    let mut actions = BTreeSet::new();
                    expand(&action, &graph, &mut BTreeSet::new(), &mut actions)?;
                    for action in actions {
                        if ["READ", "USE"].contains(&action.as_str()) && uses_state(&condition) {
                            return Err(fail("Inherited read cannot use write state"));
                        };
                        p.grants.push(Grant {
                            actor: actor.clone(),
                            target: target_name.clone(),
                            action,
                            condition: condition.clone(),
                        });
                    }
                }
            }
        }
        for n in op_nodes {
            let name = ident(n.arg(0)?.text()?)?;
            if p.operations.iter().any(|o| o.name == name) {
                return Err(fail("Duplicate operation"));
            };
            let mutation = n.head() == "mutate";
            if mutation != n.option("atomic").is_some() {
                return Err(fail("Mutations require atomic; queries cannot be atomic"));
            };
            let event =
                if let Some(on) = n.option("on") {
                    if !mutation {
                        return Err(fail("Event operations must be atomic mutations"));
                    };
                    let source = ident(on.arg(0)?.text()?)?;
                    if !p.streams.contains_key(&source) {
                        return Err(fail("Unknown event source"));
                    };
                    let actor = opt(on, "actor")?.arg(0)?;
                    let adapter = ident(actor.head())?;
                    let actor_id = valid_id(literal(actor.arg(0)?)?)?;
                    if !p.auth.iter().any(|a| {
                        a.alias == adapter && a.mode != "anonymous" && a.mode != "certificate"
                    }) {
                        return Err(fail(
                            "Event actor requires a verifiable configured auth adapter",
                        ));
                    };
                    let condition = n
                        .option("when")
                        .map(|w| w.arg(0).cloned())
                        .transpose()?
                        .unwrap_or(Node::Symbol("true".into()));
                    check_expr(&condition, false)?;
                    Some(Event {
                        source,
                        adapter,
                        actor_id,
                        condition,
                    })
                } else {
                    None
                };
            if event.is_none() && n.option("when").is_some() {
                return Err(fail("Operation when requires an event source"));
            };
            if event.is_some() && n.option("websocket").is_some() {
                return Err(fail("Operation has conflicting transports"));
            };
            let websocket = n.option("websocket");
            let (method, path, status, stream) = if let Some(ws) = websocket {
                if mutation {
                    return Err(fail("WebSocket subscriptions must be queries"));
                };
                let stream = opt(ws, "source")?.arg(0)?.text()?.to_owned();
                if !p.streams.contains_key(&stream) {
                    return Err(fail("Unknown WebSocket stream"));
                };
                (
                    "WS".to_owned(),
                    ws.arg(0)?.text()?.to_owned(),
                    200,
                    Some(stream),
                )
            } else if event.is_some() {
                ("EVENT".into(), "/".into(), 200, None)
            } else {
                let http = opt(n, "http")?;
                let method = http.arg(0)?.text()?.to_owned();
                if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str())
                    || (!mutation && method != "GET")
                {
                    return Err(fail("Invalid HTTP method"));
                };
                let status = http
                    .option("status")
                    .map(|n| size(n.arg(0)?))
                    .transpose()?
                    .unwrap_or(200) as u16;
                if !(200..300).contains(&status) {
                    return Err(fail("Invalid success status"));
                };
                (method, http.arg(1)?.text()?.to_owned(), status, None)
            };
            if !path.starts_with('/') || path.contains(['?', '#']) {
                return Err(fail("Invalid endpoint path"));
            };
            let mut output = ty(opt(n, "output")?.arg(0)?)?;
            p.check_type(&output, false, &mut BTreeSet::new())?;
            if let Some(stream) = &stream {
                output = Type::List(Box::new(output), 0, Some(p.streams[stream].max_messages));
            }
            let mut inputs = BTreeMap::new();
            let mut bindings = BTreeMap::new();
            let mut binding_order = Vec::new();
            for part in &n.args()?[1..] {
                match part.head() {
                    "input" => {
                        let key = ident(part.arg(0)?.text()?)?;
                        let t = ty(part.arg(1)?)?;
                        p.check_type(&t, false, &mut BTreeSet::new())?;
                        let default = part
                            .option("default")
                            .map(|d| d.arg(0).cloned())
                            .transpose()?;
                        if let Some(d) = &default {
                            p.validate(&t, literal(d)?, None)?;
                        }
                        unique(&mut inputs, key, (t, default))?;
                    }
                    "output" | "http" | "atomic" | "result" | "websocket" | "on" | "when" => {}
                    _ => {
                        let key = ident(part.head())?;
                        if ["actor", "target", "before", "after", "request", "context"]
                            .contains(&key.as_str())
                        {
                            return Err(fail("Reserved binding"));
                        };
                        if part.args()?.len() != 1 {
                            return Err(fail("Binding requires one expression"));
                        };
                        check_expr(part.arg(0)?, false)?;
                        if !mutation && has_effect(part.arg(0)?) {
                            return Err(fail("Query cannot write"));
                        };
                        unique(&mut bindings, key.clone(), part.arg(0)?.clone())?;
                        binding_order.push(key);
                    }
                }
            }
            if event.is_some() && !inputs.is_empty() {
                return Err(fail("Event handlers cannot take request inputs"));
            };
            for binding in bindings.values() {
                p.check_plugins(binding, mutation)?;
            }
            let result = opt(n, "result")?.arg(0)?.clone();
            p.check_plugins(&result, mutation)?;
            check_expr(&result, false)?;
            if stream.is_some() && result.head() != "live" {
                return Err(fail("WebSocket result requires live collection"));
            };
            if !mutation && has_effect(&result) {
                return Err(fail("Query cannot write"));
            };
            for segment in path.split('/').skip(1) {
                if segment.contains(['{', '}']) {
                    let key = segment
                        .strip_prefix('{')
                        .and_then(|s| s.strip_suffix('}'))
                        .ok_or_else(|| fail("Invalid path parameter"))?;
                    ident(key)?;
                    if !inputs.contains_key(key) {
                        return Err(fail("Path parameter needs typed input"));
                    };
                }
            }
            for key in bindings.keys() {
                check_dependencies(
                    key,
                    &bindings,
                    &inputs,
                    &mut BTreeSet::new(),
                    &mut BTreeSet::new(),
                )?;
            }
            let mut symbols = vec![];
            collect_symbols(&result, &mut symbols);
            check_symbols(&symbols, &bindings, &inputs)?;
            let pattern = path
                .split('/')
                .map(|s| if s.starts_with('{') { "{}" } else { s })
                .collect::<Vec<_>>()
                .join("/");
            if method != "WS"
                && method != "EVENT"
                && p.operations.iter().any(|o| {
                    o.method == method
                        && o.path
                            .split('/')
                            .map(|s| if s.starts_with('{') { "{}" } else { s })
                            .collect::<Vec<_>>()
                            .join("/")
                            == pattern
                })
            {
                return Err(fail("Duplicate route"));
            };
            p.operations.push(Operation {
                name,
                mutation,
                inputs,
                output,
                bindings,
                binding_order,
                result,
                method,
                path,
                status,
                stream,
                event,
            });
        }
        for (entity, rows) in &p.seeds {
            let e = p
                .entities
                .get(entity)
                .ok_or_else(|| fail("Unknown seed entity"))?;
            for row in rows {
                p.validate(
                    &Type::Record(
                        e.fields
                            .iter()
                            .filter(|(_, f)| f.relation.is_none())
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    ),
                    literal(row)?,
                    None,
                )?;
            }
        }
        Ok(p)
    }
    fn check_plugins(&self, node: &Node, mutation: bool) -> Result<()> {
        if node.head() == "invoke" {
            let name = node.arg(0)?.text()?;
            let method = self
                .plugins
                .get(name)
                .ok_or_else(|| fail("Unknown plugin method"))?;
            if method.mode == "external" || (!mutation && method.mode != "pure") {
                return Err(fail("Invocation mode does not match operation"));
            };
        }
        if node.head() == "record" {
            for field in node.args()? {
                self.check_plugins(field.arg(0)?, mutation)?;
            }
            return Ok(());
        };
        if node.head() == "map" {
            self.check_plugins(node.arg(0)?, mutation)?;
            for binding in &node.args()?[2..] {
                self.check_plugins(binding.arg(0)?, mutation)?;
            }
            return Ok(());
        };
        if let Node::List(nodes) = node {
            for node in nodes.iter().skip(1) {
                self.check_plugins(node, mutation)?;
            }
        }
        Ok(())
    }
    pub fn resolve<'a>(&'a self, t: &'a Type) -> Result<&'a Type> {
        if let Type::Named(n) = t
            && let Some(t) = self.types.get(n)
        {
            return self.resolve(t);
        }
        Ok(t)
    }
    fn check_type(&self, t: &Type, relation: bool, seen: &mut BTreeSet<String>) -> Result<()> {
        if seen.len() > 256 {
            return Err(fail("Type chain exceeds 256"));
        }
        match t {
            Type::Named(n) => {
                if self.entities.contains_key(n) {
                    return Ok(());
                };
                let t = self
                    .types
                    .get(n)
                    .ok_or_else(|| fail(format!("Unknown type {n}")))?;
                if !seen.insert(n.clone()) {
                    return Err(fail("Recursive type"));
                };
                self.check_type(t, relation, seen)?;
                seen.remove(n);
            }
            Type::List(inner, min, max) => {
                if (!relation && max.is_none()) || max.is_some_and(|max| max < *min) {
                    return Err(fail("List needs finite max >= min"));
                };
                self.check_type(inner, false, seen)?;
            }
            Type::Optional(inner) => self.check_type(inner, relation, seen)?,
            Type::Record(fs) => {
                for f in fs.values() {
                    self.check_type(&f.ty, false, seen)?;
                }
            }
            Type::Id(n) if !self.entities.contains_key(n) && n != "Request" => {
                return Err(fail("Unknown ID brand"));
            }
            _ => {}
        }
        Ok(())
    }
    fn permission_target(&self, target: &str) -> Result<()> {
        if self.plugins.contains_key(target) {
            return Ok(());
        };
        let (entity, field) = target
            .split_once('.')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((target, None));
        let fields = if let Some(e) = self.entities.get(entity) {
            &e.fields
        } else if let Some(Type::Record(fs)) = self.types.get(entity) {
            fs
        } else {
            return Err(fail(format!("Unknown permission target {entity}")));
        };
        if field.is_some_and(|f| !fields.contains_key(f)) {
            return Err(fail("Unknown permission field"));
        };
        Ok(())
    }
    pub fn validate(&self, t: &Type, value: Value, parent: Option<Value>) -> Result<Value> {
        let invalid = || Error::new("invalid_input", format!("Value does not match {t:?}"));
        Ok(match t {
            Type::Named(name) => {
                if let Type::Number { .. } = self.resolve(t)? {
                    if matches!(&value,Value::BrandedNumber(brand,_) if brand!=name) {
                        return Err(invalid());
                    };
                    let number = value.number()?.clone();
                    self.validate(self.resolve(t)?, Value::Num(number.clone()), None)?;
                    return Ok(Value::BrandedNumber(name.clone(), number));
                }

                if self.entities.contains_key(name) {
                    let id = match value {
                        Value::Ref { entity, id, .. }
                            if entity == *name && (1..=9_007_199_254_740_991).contains(&id) =>
                        {
                            id
                        }
                        Value::Id(entity, id)
                            if entity == *name && (1..=9_007_199_254_740_991).contains(&id) =>
                        {
                            id
                        }
                        Value::Num(id) => valid_id(Value::Num(id))?,
                        _ => return Err(invalid()),
                    };
                    Value::reference(name, &id)
                } else {
                    let inner = self.types.get(name).ok_or_else(invalid)?;
                    let mut v = self.validate(inner, value, parent.clone())?;
                    if let Value::Record {
                        kind, parent: vp, ..
                    } = &mut v
                    {
                        *kind = parent.as_ref().map(|_| name.clone());
                        *vp = parent.map(Box::new);
                    }
                    v
                }
            }
            Type::Id(entity) => {
                let id = match value {
                    Value::Id(brand, id)
                        if brand == *entity && (1..=9_007_199_254_740_991).contains(&id) =>
                    {
                        id
                    }
                    Value::Num(id) => valid_id(Value::Num(id))?,
                    _ => return Err(invalid()),
                };
                Value::Id(entity.clone(), id)
            }
            Type::Image {
                max_bytes,
                width,
                height,
            } => {
                let bytes = match value {
                    Value::Image(image) => image.bytes,
                    Value::Str(encoded) => {
                        use base64::Engine as _;
                        if encoded.len() > max_bytes.saturating_mul(4) / 3 + 8 {
                            return Err(invalid());
                        };
                        base64::engine::general_purpose::STANDARD
                            .decode(encoded)
                            .map_err(|_| invalid())?
                    }
                    _ => return Err(invalid()),
                };
                Value::Image(crate::media::Image::decode(
                    bytes, *max_bytes, *width, *height,
                )?)
            }
            Type::String => match value {
                Value::Str(s) => Value::Str(s),
                _ => return Err(invalid()),
            },
            Type::Bool => match value {
                Value::Bool(b) => Value::Bool(b),
                _ => return Err(invalid()),
            },
            Type::DateTime => {
                let s = value.text()?;
                let d = chrono::DateTime::parse_from_rfc3339(&s).map_err(|_| invalid())?;
                Value::Str(d.with_timezone(&chrono::Utc).to_rfc3339())
            }
            Type::Number { min, max, scale } => {
                let n = value.number()?.clone();
                let scaled = &n * Number::from_integer(BigInt::from(10).pow(*scale));
                if &n < min || &n > max || !scaled.is_integer() {
                    return Err(invalid());
                };
                Value::Num(n)
            }
            Type::Optional(inner) => {
                if value == Value::Null {
                    Value::Null
                } else {
                    self.validate(inner, value, parent)?
                }
            }
            Type::List(inner, min, max) => {
                let Value::List(values) = value else {
                    return Err(invalid());
                };
                if values.len() < *min || max.is_some_and(|m| values.len() > m) {
                    return Err(invalid());
                };
                Value::List(
                    values
                        .into_iter()
                        .map(|v| self.validate(inner, v, parent.clone()))
                        .collect::<Result<_>>()?,
                )
            }
            Type::Enum(values) => {
                let s = value.text()?;
                if !values.contains(&s) {
                    return Err(invalid());
                };
                Value::Str(s)
            }
            Type::Record(fs) => {
                let values = value.fields()?;
                if values.keys().any(|k| !fs.contains_key(k)) {
                    return Err(invalid());
                };
                let fields = fs
                    .iter()
                    .map(|(name, f)| {
                        let v = values.get(name).cloned().unwrap_or(Value::Null);
                        Ok((name.clone(), self.validate(&f.ty, v, parent.clone())?))
                    })
                    .collect::<Result<_>>()?;
                Value::Record {
                    fields,
                    kind: None,
                    parent: parent.map(Box::new),
                }
            }
        })
    }
}
pub fn valid_id(value: Value) -> Result<i64> {
    use num_traits::ToPrimitive;
    match value {
        Value::Num(n) if n.is_integer() => n
            .to_integer()
            .to_i64()
            .filter(|id| (1..=9_007_199_254_740_991).contains(id))
            .ok_or_else(|| Error::new("invalid_input", "ID must be a positive safe integer")),
        _ => Err(Error::new(
            "invalid_input",
            "ID must be a positive safe integer",
        )),
    }
}

pub fn literal(n: &Node) -> Result<Value> {
    Ok(match n {
        Node::Number(s) => Value::Num(number(s)?),
        Node::String(s) => Value::Str(s.clone()),
        Node::Symbol(s) => match s.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" => Value::Null,
            _ => Value::Str(s.clone()),
        },
        Node::List(_) => match n.head() {
            "list" => Value::List(n.args()?.iter().map(literal).collect::<Result<_>>()?),
            "record" => {
                let mut values = BTreeMap::new();
                for f in n.args()? {
                    unique(&mut values, ident(f.head())?, literal(f.arg(0)?)?)?;
                }
                Value::record(values)
            }
            _ => return Err(fail("Expected literal")),
        },
    })
}
fn expand(
    action: &str,
    graph: &BTreeMap<String, Vec<String>>,
    visiting: &mut BTreeSet<String>,
    out: &mut BTreeSet<String>,
) -> Result<()> {
    if !visiting.insert(action.into()) {
        return Err(fail("Cyclic action inheritance"));
    };
    out.insert(action.into());
    if let Some(next) = graph.get(action) {
        for n in next {
            expand(n, graph, visiting, out)?;
        }
    }
    visiting.remove(action);
    Ok(())
}
fn uses_state(n: &Node) -> bool {
    match n {
        Node::Symbol(s) => ["before", "after", "changed", "transaction"]
            .iter()
            .any(|p| s.split('.').next() == Some(p)),
        Node::List(ns) => ns.iter().any(uses_state),
        _ => false,
    }
}
fn has_effect(n: &Node) -> bool {
    matches!(n.head(), "create" | "set" | "delete" | "publish")
        || n.list().is_ok_and(|ns| ns.iter().any(has_effect))
}
fn check_expr(n: &Node, policy: bool) -> Result<()> {
    if let Node::List(ns) = n {
        let h = n.head();
        let (min, max) = match h {
            "record" => {
                let mut seen = BTreeSet::new();
                for f in n.args()? {
                    if !seen.insert(ident(f.head())?) {
                        return Err(fail("Duplicate record field"));
                    };
                    if f.args()?.len() != 1 {
                        return Err(fail("Record field needs one value"));
                    };
                    check_expr(f.arg(0)?, policy)?;
                }
                return Ok(());
            }
            "list" | "and" | "or" | "concat" => (0, 100000),
            "not" | "entity" | "new" | "entities" | "single" | "count" | "ago" | "delete"
            | "live" => (1, 1),
            "eq" | "ne" | "gt" | "ge" | "lt" | "le" | "add" | "sub" | "mul" | "div"
            | "contains" | "can" | "as" | "set" | "create" | "publish" | "last" | "first"
            | "since" | "creates" | "invoke" => (2, 2),
            "only" => (1, 1000),
            "any" | "all" | "where" | "flatMap" | "sum" | "order" => (3, 3),
            "map" => (3, 1000),
            "groupSum" => (4, 4),
            _ => return Err(fail(format!("Unsupported expression {h}"))),
        };
        let argc = ns.len().saturating_sub(1);
        if argc < min || argc > max {
            return Err(fail(format!("Wrong arity for {h}")));
        };
        if policy && matches!(h, "create" | "set" | "delete" | "publish" | "new" | "map") {
            return Err(fail("Permission must be pure"));
        };
        // Permission recursion under negation is deliberately rejected rather than assigned unsafe semantics.
        if h == "not" && contains_can(n.arg(0)?) {
            return Err(fail("Negative permission recursion unsupported"));
        };
        if h == "map" {
            check_expr(n.arg(0)?, policy)?;
            ident(n.arg(1)?.text()?)?;
            for f in &n.args()?[2..] {
                if f.args()?.len() != 1 {
                    return Err(fail("Map binding needs one expression"));
                };
                check_expr(f.arg(0)?, policy)?;
            }
            return Ok(());
        }
        for child in ns.iter().skip(1) {
            check_expr(child, policy)?;
        }
    }
    Ok(())
}
fn contains_can(n: &Node) -> bool {
    n.head() == "can" || n.list().is_ok_and(|ns| ns.iter().any(contains_can))
}
fn collect_symbols<'a>(n: &'a Node, out: &mut Vec<&'a str>) {
    fn walk<'a>(n: &'a Node, out: &mut Vec<&'a str>, locals: &BTreeSet<&'a str>) {
        if let Node::Symbol(s) = n {
            if !locals.contains(s.split('.').next().unwrap()) {
                out.push(s)
            }
        } else if let Node::List(ns) = n {
            if n.head() == "record" {
                for f in n.args().unwrap() {
                    walk(f.arg(0).unwrap(), out, locals)
                }
                return;
            }
            if ["any", "all", "where", "flatMap", "sum", "map"].contains(&n.head()) {
                walk(n.arg(0).unwrap(), out, locals);
                let mut locals = locals.clone();
                locals.insert(n.arg(1).unwrap().text().unwrap());
                if n.head() == "map" {
                    for f in &n.args().unwrap()[2..] {
                        locals.insert(f.head());
                    }
                    for f in &n.args().unwrap()[2..] {
                        walk(f.arg(0).unwrap(), out, &locals)
                    }
                } else {
                    walk(n.arg(2).unwrap(), out, &locals)
                };
                return;
            }
            let skip = match n.head() {
                "new" | "entities" | "creates" | "as" | "create" | "publish" | "can" | "invoke" => {
                    2
                }
                _ => 1,
            };
            for child in ns.iter().skip(skip) {
                walk(child, out, locals);
            }
        }
    }
    walk(n, out, &BTreeSet::new());
}
fn check_symbols(
    symbols: &[&str],
    bindings: &BTreeMap<String, Node>,
    inputs: &BTreeMap<String, (Type, Option<Node>)>,
) -> Result<()> {
    for symbol in symbols {
        if let Some(input) = symbol.strip_prefix('$') {
            if !inputs.contains_key(input) {
                return Err(fail(format!("Unknown input {symbol}")));
            }
        } else if symbol.contains('.') {
            let root = symbol.split('.').next().unwrap();
            if !bindings.contains_key(root)
                && root != "request"
                && root != "event"
                && root.chars().next().is_some_and(char::is_lowercase)
            {
                return Err(fail(format!("Unknown reference {symbol}")));
            }
        }
    }
    Ok(())
}
fn check_dependencies(
    key: &str,
    bindings: &BTreeMap<String, Node>,
    inputs: &BTreeMap<String, (Type, Option<Node>)>,
    visiting: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> Result<()> {
    if done.contains(key) {
        return Ok(());
    };
    if !visiting.insert(key.into()) {
        return Err(fail("Cyclic bindings"));
    };
    let mut symbols = vec![];
    let expr = &bindings[key];
    collect_symbols(expr, &mut symbols);
    // Locals in map/quantifiers are checked by the evaluator; top-level cycles still include nested references.
    if expr.head() != "map" {
        check_symbols(&symbols, bindings, inputs)?;
    }
    for s in symbols {
        let root = s.split('.').next().unwrap();
        if bindings.contains_key(root) {
            check_dependencies(root, bindings, inputs, visiting, done)?;
        }
    }
    visiting.remove(key);
    done.insert(key.into());
    Ok(())
}
