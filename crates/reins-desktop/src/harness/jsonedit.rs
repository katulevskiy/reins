//! Editing a harness's JSON settings in place: one member or array item is inserted or removed as text, so the rest of
//! the file (the user's formatting, key order, comments) stays byte for byte as it was, and removing what was inserted
//! gives back the file exactly. Comments (`//`, `/* */`) and trailing commas are read as whitespace (JSONC).

use serde_json::Value;

/// A JSON value and where it is in the text.
#[derive(Clone, Debug)]
pub enum Node {
    Object {
        open: usize,
        close: usize,
        /// Key, where the key starts, the value.
        members: Vec<(String, usize, Node)>,
    },
    Array {
        open: usize,
        close: usize,
        items: Vec<Node>,
    },
    Scalar {
        start: usize,
        end: usize,
    },
}

impl Node {
    #[must_use]
    pub fn start(&self) -> usize {
        match self {
            Self::Object {
                open,
                ..
            }
            | Self::Array {
                open,
                ..
            } => *open,
            Self::Scalar {
                start,
                ..
            } => *start,
        }
    }

    /// One past the last byte.
    #[must_use]
    pub fn end(&self) -> usize {
        match self {
            Self::Object {
                close,
                ..
            }
            | Self::Array {
                close,
                ..
            } => close + 1,
            Self::Scalar {
                end,
                ..
            } => *end,
        }
    }

    #[must_use]
    pub fn member(&self, key: &str) -> Option<&Node> {
        match self {
            Self::Object {
                members,
                ..
            } => members.iter().find(|(k, _, _)| k == key).map(|(_, _, v)| v),
            _ => None,
        }
    }

    /// The node at `path` (object keys), if every step exists.
    #[must_use]
    pub fn at(&self, path: &[String]) -> Option<&Node> {
        path.iter().try_fold(self, |node, key| node.member(key))
    }

    /// Number of members or items (0 for a scalar).
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Object {
                members,
                ..
            } => members.len(),
            Self::Array {
                items,
                ..
            } => items.len(),
            Self::Scalar {
                ..
            } => 0,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `(start, end)` of each entry: a member from its key to the end of its value, or an item.
    fn entries(&self) -> Vec<(usize, usize)> {
        match self {
            Self::Object {
                members,
                ..
            } => members.iter().map(|(_, k, v)| (*k, v.end())).collect(),
            Self::Array {
                items,
                ..
            } => items.iter().map(|n| (n.start(), n.end())).collect(),
            Self::Scalar {
                ..
            } => Vec::new(),
        }
    }

    /// The value (comments dropped).
    #[must_use]
    pub fn value(&self, text: &str) -> Value {
        match self {
            Self::Object {
                members,
                ..
            } => Value::Object(members.iter().map(|(k, _, v)| (k.clone(), v.value(text))).collect()),
            Self::Array {
                items,
                ..
            } => Value::Array(items.iter().map(|n| n.value(text)).collect()),
            Self::Scalar {
                start,
                end,
            } => serde_json::from_str(&text[*start..*end]).unwrap_or(Value::Null),
        }
    }
}

/// Parses `text` (one JSON value, JSONC comments and trailing commas allowed).
pub fn parse(text: &str) -> Result<Node, String> {
    let mut p = Parser {
        b: text.as_bytes(),
        text,
        i: 0,
    };
    p.ws()?;
    let node = p.value(0)?;
    p.ws()?;
    if p.i != p.b.len() {
        return Err(p.error("text after the end of the JSON value"));
    }
    Ok(node)
}

struct Parser<'a> {
    b: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        let line = self.text[..self.i.min(self.text.len())].matches('\n').count() + 1;
        format!("line {line}: {what}")
    }

    fn ws(&mut self) -> Result<(), String> {
        loop {
            match self.b.get(self.i) {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.i += 1,
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'/') => {
                    while self.b.get(self.i).is_some_and(|&c| c != b'\n') {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'*') => {
                    let rest = &self.text[self.i + 2..];
                    let end = rest.find("*/").ok_or_else(|| self.error("unclosed comment"))?;
                    self.i += 2 + end + 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<Node, String> {
        if depth > 200 {
            return Err(self.error("nested too deeply"));
        }
        match self.b.get(self.i) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => {
                let start = self.i;
                self.string()?;
                Ok(Node::Scalar {
                    start,
                    end: self.i,
                })
            }
            Some(_) => {
                let start = self.i;
                while self.b.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'.')) {
                    self.i += 1;
                }
                let raw = &self.text[start..self.i];
                if raw.is_empty() || serde_json::from_str::<Value>(raw).is_err() {
                    return Err(self.error("not a JSON value"));
                }
                Ok(Node::Scalar {
                    start,
                    end: self.i,
                })
            }
            None => Err(self.error("unexpected end of the file")),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.i;
        self.i += 1;
        loop {
            match self.b.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => self.i += 2,
                Some(_) => self.i += 1,
                None => return Err(self.error("unclosed string")),
            }
        }
        serde_json::from_str(&self.text[start..self.i]).map_err(|_| self.error("bad string"))
    }

    fn object(&mut self, depth: usize) -> Result<Node, String> {
        let open = self.i;
        self.i += 1;
        let mut members = Vec::new();
        loop {
            self.ws()?;
            match self.b.get(self.i) {
                Some(b'}') => break,
                Some(b'"') => {
                    let key_at = self.i;
                    let key = self.string()?;
                    self.ws()?;
                    if self.b.get(self.i) != Some(&b':') {
                        return Err(self.error("expected `:`"));
                    }
                    self.i += 1;
                    self.ws()?;
                    let v = self.value(depth + 1)?;
                    members.push((key, key_at, v));
                    self.ws()?;
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => break,
                        _ => return Err(self.error("expected `,` or `}`")),
                    }
                }
                _ => return Err(self.error("expected a key")),
            }
        }
        let node = Node::Object {
            open,
            close: self.i,
            members,
        };
        self.i += 1;
        Ok(node)
    }

    fn array(&mut self, depth: usize) -> Result<Node, String> {
        let open = self.i;
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.ws()?;
            if self.b.get(self.i) == Some(&b']') {
                break;
            }
            items.push(self.value(depth + 1)?);
            self.ws()?;
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => break,
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
        let node = Node::Array {
            open,
            close: self.i,
            items,
        };
        self.i += 1;
        Ok(node)
    }
}

/// `v` as JSON, objects and arrays over several lines indented by `unit` below `indent`.
#[must_use]
pub fn pretty(v: &Value, unit: &str, indent: &str) -> String {
    let inner = format!("{indent}{unit}");
    match v {
        Value::Object(m) if !m.is_empty() => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{inner}{}: {}", Value::String(k.clone()), pretty(v, unit, &inner)))
                .collect();
            format!("{{\n{}\n{indent}}}", parts.join(",\n"))
        }
        Value::Array(a) if !a.is_empty() => {
            let parts: Vec<String> = a.iter().map(|v| format!("{inner}{}", pretty(v, unit, &inner))).collect();
            format!("[\n{}\n{indent}]", parts.join(",\n"))
        }
        other => other.to_string(),
    }
}

/// The leading whitespace of the line `pos` is on.
fn line_indent(text: &str, pos: usize) -> &str {
    let line_start = text[..pos].rfind('\n').map_or(0, |n| n + 1);
    let line = &text[line_start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// The file's indentation step: what the root's first entry is indented by, else two spaces.
#[must_use]
pub fn indent_unit(text: &str, root: &Node) -> String {
    let first = root.entries().first().map(|e| e.0);
    match first {
        Some(at) if text[root.start() + 1..at].contains('\n') => {
            let unit = line_indent(text, at);
            if unit.is_empty() {
                "  ".to_owned()
            } else {
                unit.to_owned()
            }
        }
        _ => "  ".to_owned(),
    }
}

/// What to put into a container: a member (`key`) or an array item (`None`).
pub struct Entry<'a> {
    pub key: Option<&'a str>,
    pub value: &'a Value,
}

/// Inserts `entry` as the last entry of `container` (a node of `text`). Returns the new text and, when the container
/// was empty, its old inside (what [`remove`] puts back when the container is empty again).
#[must_use]
pub fn insert(text: &str, root: &Node, container: &Node, entry: &Entry<'_>) -> (String, Option<String>) {
    let unit = indent_unit(text, root);
    let render = |indent: &str, multiline: bool| {
        let value = if multiline {
            pretty(entry.value, &unit, indent)
        } else {
            entry.value.to_string()
        };
        match entry.key {
            Some(k) => format!("{}: {value}", Value::String(k.to_owned())),
            None => value,
        }
    };
    let entries = container.entries();
    let open = container.start();
    let close = container.end() - 1;
    if let (Some(first), Some(last)) = (entries.first(), entries.last()) {
        let before_first = &text[open + 1..first.0];
        let (sep, rendered) = if before_first.contains('\n') {
            let indent = line_indent(text, first.0);
            (format!(",\n{indent}"), render(indent, true))
        } else {
            (", ".to_owned(), render("", false))
        };
        let mut out = String::with_capacity(text.len() + sep.len() + rendered.len());
        out.push_str(&text[..last.1]);
        out.push_str(&sep);
        out.push_str(&rendered);
        out.push_str(&text[last.1..]);
        (out, None)
    } else {
        let interior = text[open + 1..close].to_owned();
        let outer = line_indent(text, open);
        let inner = format!("{outer}{unit}");
        let rendered = format!("\n{inner}{}\n{outer}", render(&inner, true));
        let mut out = String::with_capacity(text.len() + rendered.len());
        out.push_str(&text[..=open]);
        out.push_str(&rendered);
        out.push_str(&text[close..]);
        (out, Some(interior))
    }
}

/// Removes entry `index` of `container`; when it was the only one, the container's inside becomes `interior` (or
/// nothing). The exact inverse of [`insert`] for the last entry.
#[must_use]
pub fn remove(text: &str, container: &Node, index: usize, interior: Option<&str>) -> String {
    let entries = container.entries();
    let (from, to, with) = if entries.len() == 1 {
        (container.start() + 1, container.end() - 1, interior.unwrap_or(""))
    } else if index > 0 {
        (entries[index - 1].1, entries[index].1, "")
    } else {
        (entries[0].0, entries[1].0, "")
    };
    format!("{}{with}{}", &text[..from], &text[to..])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn member_index(node: &Node, key: &str) -> usize {
        let Node::Object {
            members,
            ..
        } = node
        else {
            panic!()
        };
        members.iter().position(|(k, _, _)| k == key).unwrap()
    }

    #[test]
    fn jsonc_parses_with_spans_and_values() {
        let text = "// settings\n{\n  \"a\": [1, 2,], /* c */ \"b\": {\"x\": \"y\\\"z\"},\n}\n";
        let root = parse(text).unwrap();
        assert_eq!(root.value(text), json!({"a": [1, 2], "b": {"x": "y\"z"}}));
        let b = root.member("b").unwrap();
        assert_eq!(&text[b.start()..b.end()], "{\"x\": \"y\\\"z\"}");
        assert_eq!(root.at(&["b".into(), "x".into()]).unwrap().value(text), json!("y\"z"));
        for bad in ["{", "{\"a\" 1}", "[1 2]", "{} x", "nope", "{\"a\": tru}", "/* x"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn inserting_then_removing_gives_back_the_same_bytes() {
        let v = json!({"command": "/bin/reins", "args": ["mcp", "--via", "X"]});
        for text in [
            "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"o\"}\n  }\n}\n",
            "{\n\t\"mcpServers\": {}\n}",
            "{\"mcpServers\": {\"a\": 1}}",
            "{\n    \"mcpServers\": { }\n}\n",
            "{\n  \"mcpServers\": {\n    \"a\": 1, // keep\n  }\n}\n",
        ] {
            let root = parse(text).unwrap();
            let servers = root.member("mcpServers").unwrap();
            let (added, interior) = insert(
                text,
                &root,
                servers,
                &Entry {
                    key: Some("reins"),
                    value: &v,
                },
            );
            let new_root = parse(&added).unwrap_or_else(|e| panic!("{e}\n{added}"));
            let servers = new_root.member("mcpServers").unwrap();
            assert_eq!(servers.member("reins").unwrap().value(&added), v, "{added}");
            let back = remove(&added, servers, member_index(servers, "reins"), interior.as_deref());
            assert_eq!(back, text, "{added}");
        }
    }

    #[test]
    fn new_entries_follow_the_file_style() {
        let text = "{\n    \"hooks\": {\n        \"PreToolUse\": []\n    }\n}\n";
        let root = parse(text).unwrap();
        let arr = root.at(&["hooks".into(), "PreToolUse".into()]).unwrap();
        let (added, interior) = insert(
            text,
            &root,
            arr,
            &Entry {
                key: None,
                value: &json!({"matcher": "Bash"}),
            },
        );
        assert_eq!(interior.as_deref(), Some(""));
        assert_eq!(
            added,
            "{\n    \"hooks\": {\n        \"PreToolUse\": [\n            {\n                \"matcher\": \"Bash\"\n            }\n        ]\n    }\n}\n"
        );
        let compact = "{\"a\": [1]}";
        let root = parse(compact).unwrap();
        let (added, _) = insert(
            compact,
            &root,
            root.member("a").unwrap(),
            &Entry {
                key: None,
                value: &json!({"b": 2}),
            },
        );
        assert_eq!(added, "{\"a\": [1, {\"b\":2}]}");
    }

    #[test]
    fn removing_a_first_or_middle_entry_keeps_the_others() {
        let text = "[\n  1,\n  2,\n  3\n]";
        let root = parse(text).unwrap();
        assert_eq!(remove(text, &root, 0, None), "[\n  2,\n  3\n]");
        assert_eq!(remove(text, &root, 1, None), "[\n  1,\n  3\n]");
        assert_eq!(remove(text, &root, 2, None), "[\n  1,\n  2\n]");
    }
}
