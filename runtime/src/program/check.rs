//! Checks that an expression is safe to evaluate under the program's rules.
//! Tracks effects, policy (`can`) expressions, and binding cycles.
//! Does not execute the expression.

use crate::{Result, syntax::Node};
use std::collections::{BTreeMap, BTreeSet};

use super::grammar::{fail, ident};
use super::model::*;

pub(in crate::program) fn expand(
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
pub(in crate::program) fn uses_state(n: &Node) -> bool {
    match n {
        Node::Symbol(s) => ["before", "after", "changed", "transaction"]
            .iter()
            .any(|p| s.split('.').next() == Some(p)),
        Node::List(ns) => ns.iter().any(uses_state),
        _ => false,
    }
}
pub(in crate::program) fn has_effect(n: &Node) -> bool {
    matches!(n.head(), "create" | "set" | "delete" | "publish")
        || n.list().is_ok_and(|ns| ns.iter().any(has_effect))
}
pub(in crate::program) fn check_expr(n: &Node, policy: bool) -> Result<()> {
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
            | "since" | "creates" | "invoke" | "expectVersion" => (2, 2),
            "only" => (1, 1000),
            "any" | "all" | "where" | "flatMap" | "sum" | "order" | "page" => (3, 3),
            "map" => (3, 1000),
            "groupSum" => (4, 4),
            _ => return Err(fail(format!("Unsupported expression {h}"))),
        };
        let argc = ns.len().saturating_sub(1);
        if argc < min || argc > max {
            return Err(fail(format!("Wrong arity for {h}")));
        };
        if policy
            && matches!(
                h,
                "create" | "set" | "delete" | "publish" | "new" | "map" | "expectVersion"
            )
        {
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
pub(in crate::program) fn contains_can(n: &Node) -> bool {
    n.head() == "can" || n.list().is_ok_and(|ns| ns.iter().any(contains_can))
}
pub(in crate::program) fn collect_symbols<'a>(n: &'a Node, out: &mut Vec<&'a str>) {
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
pub(in crate::program) fn check_symbols(
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
pub(in crate::program) fn check_dependencies(
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
