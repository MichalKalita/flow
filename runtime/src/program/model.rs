//! Declares the compiled program: types, entities, operations, grants, and seeds.
//! Holds structure only. Parsing and permission checks live next to this module.

use crate::{syntax::Node, value::Number};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Type {
    Image {
        max_bytes: usize,
        width: u32,
        height: u32,
    },
    String,
    Text {
        min_bytes: usize,
        max_bytes: usize,
    },
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
#[derive(Clone, Debug, PartialEq)]
pub enum SeedGroup {
    Production,
    Test,
}
#[derive(Clone, Debug)]
pub struct Seed {
    pub entity: String,
    pub rows: Vec<Node>,
    pub group: SeedGroup,
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
    pub seeds: Vec<Seed>,
    pub source: String,
    pub schema_version: u32,
    pub migrations: Vec<crate::migrations::Migration>,
}
