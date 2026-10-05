use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    List(Vec<Node>),
    Symbol(String),
    String(String),
    Number(String),
}
impl Node {
    pub fn text(&self) -> Result<&str> {
        match self {
            Self::Symbol(s) | Self::String(s) | Self::Number(s) => Ok(s),
            _ => Err(Error::new("syntax", "Expected scalar")),
        }
    }
    pub fn list(&self) -> Result<&[Node]> {
        match self {
            Self::List(v) => Ok(v),
            _ => Err(Error::new("syntax", "Expected list")),
        }
    }
    pub fn head(&self) -> &str {
        self.list()
            .ok()
            .and_then(|v| v.first())
            .and_then(|n| n.text().ok())
            .unwrap_or("")
    }
    pub fn args(&self) -> Result<&[Node]> {
        self.list()?
            .get(1..)
            .ok_or_else(|| Error::new("syntax", "Empty form"))
    }
    pub fn option(&self, name: &str) -> Option<&Node> {
        self.list().ok()?.iter().skip(1).find(|n| n.head() == name)
    }
    pub fn arg(&self, i: usize) -> Result<&Node> {
        self.args()?
            .get(i)
            .ok_or_else(|| Error::new("syntax", format!("Missing argument in {}", self.head())))
    }
}

pub fn parse(source: &str) -> Result<Vec<Node>> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(Error::new("syntax", "Source exceeds 4 MiB"));
    }
    let mut stack: Vec<Vec<Node>> = vec![vec![]];
    let mut i = 0;
    let bytes = source.as_bytes();
    let mut nodes = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        let start = i;
        match bytes[i] {
            b'[' => {
                if stack.len() > 256 {
                    return Err(Error::new("syntax", "Nesting exceeds 256"));
                }
                stack.push(vec![]);
                i += 1;
            }
            b']' => {
                if stack.len() == 1 {
                    return Err(Error::new("syntax", format!("Unexpected ] at byte {i}")));
                }
                let value = Node::List(stack.pop().unwrap());
                stack.last_mut().unwrap().push(value);
                i += 1;
            }
            b'"' => {
                i += 1;
                let mut escaped = false;
                let mut closed = false;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if !escaped && b == b'"' {
                        closed = true;
                        break;
                    }
                    escaped = !escaped && b == b'\\';
                }
                if !closed {
                    return Err(Error::new(
                        "syntax",
                        format!("Unclosed string at byte {start}"),
                    ));
                }
                let value: String = serde_json::from_str(&source[start..i])?;
                stack.last_mut().unwrap().push(Node::String(value));
                if i < bytes.len() && !bytes[i].is_ascii_whitespace() && !b"[]#".contains(&bytes[i])
                {
                    return Err(Error::new("syntax", "Missing separator"));
                }
            }
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !b"[]#\"".contains(&bytes[i])
                {
                    i += 1;
                }
                if start == i {
                    return Err(Error::new("syntax", "Unexpected character"));
                }
                let s = &source[start..i];
                if s.chars().any(char::is_control) {
                    return Err(Error::new("syntax", "Control character in symbol"));
                }
                let numeric = bytes[start].is_ascii_digit()
                    || ((bytes[start] == b'-' || bytes[start] == b'+')
                        && bytes.get(start + 1).is_some_and(u8::is_ascii_digit));
                let value = if numeric {
                    let _: serde_json::Number = serde_json::from_str(s).map_err(|_| {
                        Error::new("syntax", format!("Invalid number at byte {start}"))
                    })?;
                    Node::Number(s.into())
                } else {
                    Node::Symbol(s.into())
                };
                stack.last_mut().unwrap().push(value);
                if bytes.get(i) == Some(&b'"') {
                    return Err(Error::new("syntax", "Missing separator"));
                }
            }
        }
        nodes += 1;
        if nodes > 100000 {
            return Err(Error::new("syntax", "Node limit"));
        }
    }
    if stack.len() != 1 {
        return Err(Error::new("syntax", "Unclosed list"));
    }
    let root = stack.pop().unwrap();
    if root.iter().any(|n| !matches!(n, Node::List(_))) {
        return Err(Error::new("syntax", "Expected top-level list"));
    }
    Ok(root)
}
