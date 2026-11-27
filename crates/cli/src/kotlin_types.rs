//! The Kotlin versions of the types shared with the Android app (`android/`), written by `launchpad gen-types`
//! beside the TypeScript ones. They are derived from the ts-rs output (one `export type` per file), so both clients
//! read the same definitions:
//!
//! - an object type becomes a `@Serializable data class` whose fields all have defaults (a Home that is newer or
//!   older than the app still decodes);
//! - a union of string literals becomes `typealias X = String` (an unknown value is not an error);
//! - anything else (tagged unions, intersections, generics) becomes `typealias X = JsonElement`.
//!
//! Numbers are `Double`: TypeScript has one number type, and timestamps in milliseconds fit exactly.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Named(String, Vec<Ty>),
    StrLit,
    OtherLit,
    Null,
    Object(Vec<Field>),
    Map(Box<Ty>),
    Array(Box<Ty>),
    Union(Vec<Ty>),
    /// Intersections and tuples: kept as raw JSON.
    Opaque,
}

#[derive(Clone, Debug, PartialEq)]
struct Field {
    name: String,
    optional: bool,
    ty: Ty,
}

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.at < self.src.len() && self.src[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.src.get(self.at).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            return true;
        }
        false
    }
    fn expect(&mut self, c: u8) -> Result<()> {
        if !self.eat(c) {
            bail!("expected '{}' at {}: {:?}", c as char, self.at, String::from_utf8_lossy(&self.src[self.at..(self.at + 40).min(self.src.len())]));
        }
        Ok(())
    }
    fn ident(&mut self) -> Option<String> {
        self.ws();
        let start = self.at;
        while self.at < self.src.len() && (self.src[self.at].is_ascii_alphanumeric() || matches!(self.src[self.at], b'_' | b'$')) {
            self.at += 1;
        }
        (self.at > start).then(|| String::from_utf8_lossy(&self.src[start..self.at]).into_owned())
    }
    fn string(&mut self) -> Result<String> {
        let quote = self.src[self.at];
        self.at += 1;
        let mut out = Vec::new();
        while self.at < self.src.len() && self.src[self.at] != quote {
            if self.src[self.at] == b'\\' {
                self.at += 1;
            }
            out.push(self.src[self.at]);
            self.at += 1;
        }
        self.at += 1;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn union(&mut self) -> Result<Ty> {
        self.eat(b'|');
        let mut parts = vec![self.intersection()?];
        while self.eat(b'|') {
            parts.push(self.intersection()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Ty::Union(parts) })
    }
    fn intersection(&mut self) -> Result<Ty> {
        let first = self.postfix()?;
        let mut opaque = false;
        while self.eat(b'&') {
            self.postfix()?;
            opaque = true;
        }
        Ok(if opaque { Ty::Opaque } else { first })
    }
    fn postfix(&mut self) -> Result<Ty> {
        let mut ty = self.primary()?;
        while self.peek() == Some(b'[') && self.src.get(self.at + 1) == Some(&b']') {
            self.at += 2;
            ty = Ty::Array(Box::new(ty));
        }
        Ok(ty)
    }
    fn primary(&mut self) -> Result<Ty> {
        match self.peek().context("unexpected end of type")? {
            b'"' | b'\'' => {
                self.string()?;
                Ok(Ty::StrLit)
            }
            b'(' => {
                self.at += 1;
                let ty = self.union()?;
                self.expect(b')')?;
                Ok(ty)
            }
            b'[' => {
                self.at += 1;
                while !self.eat(b']') {
                    self.union()?;
                    self.eat(b',');
                }
                Ok(Ty::Opaque)
            }
            b'{' => {
                self.at += 1;
                self.object()
            }
            c if c.is_ascii_digit() || c == b'-' => {
                self.at += 1;
                while self.at < self.src.len() && (self.src[self.at].is_ascii_digit() || self.src[self.at] == b'.') {
                    self.at += 1;
                }
                Ok(Ty::OtherLit)
            }
            _ => {
                let name = self.ident().with_context(|| format!("unexpected character at {}", self.at))?;
                match name.as_str() {
                    "null" | "undefined" => return Ok(Ty::Null),
                    "true" | "false" => return Ok(Ty::OtherLit),
                    _ => {}
                }
                let mut args = vec![];
                if self.eat(b'<') {
                    while !self.eat(b'>') {
                        args.push(self.union()?);
                        self.eat(b',');
                    }
                }
                Ok(Ty::Named(name, args))
            }
        }
    }
    /// After `{`: fields, or one index signature (`[key in string]?: T`).
    fn object(&mut self) -> Result<Ty> {
        let mut fields = vec![];
        loop {
            if self.eat(b'}') {
                return Ok(Ty::Object(fields));
            }
            if self.eat(b'[') {
                while self.at < self.src.len() && self.src[self.at] != b']' {
                    self.at += 1;
                }
                self.expect(b']')?;
                self.eat(b'?');
                self.expect(b':')?;
                let value = self.union()?;
                let _ = self.eat(b',') || self.eat(b';');
                self.expect(b'}')?;
                return Ok(Ty::Map(Box::new(value)));
            }
            let name = match self.peek() {
                Some(b'"') | Some(b'\'') => self.string()?,
                _ => self.ident().context("expected a field name")?,
            };
            let optional = self.eat(b'?');
            self.expect(b':')?;
            let ty = self.union()?;
            let _ = self.eat(b',') || self.eat(b';');
            fields.push(Field { name, optional, ty });
        }
    }
}

/// The source without comments (`/** … */`, `// …`), outside string literals.
fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                let q = b[i];
                out.push(q);
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        out.push(b[i]);
                        i += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                out.push(q);
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `export type Name<…> = body;` of one ts-rs file: the name, whether it is generic, and its body.
fn declaration(src: &str) -> Option<(String, bool, String)> {
    let clean = strip_comments(src);
    let rest = &clean[clean.find("export type ")? + "export type ".len()..];
    let eq = rest.find('=')?;
    let head = rest[..eq].trim();
    let (name, generic) = match head.split_once('<') {
        Some((name, _)) => (name.trim(), true),
        None => (head, false),
    };
    Some((name.to_string(), generic, rest[eq + 1..].trim().trim_end_matches(';').to_string()))
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Class,
    Text,
    Json,
    Alias,
}

const KEYWORDS: &[&str] = &["as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is", "null", "object", "package", "return", "super", "this", "throw", "true", "try", "typealias", "typeof", "val", "var", "when", "while"];

fn field_name(name: &str) -> (String, bool) {
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit());
    if !plain {
        let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        return (format!("f_{safe}"), true);
    }
    if KEYWORDS.contains(&name) { (format!("`{name}`"), false) } else { (name.to_string(), false) }
}

struct Emitter {
    shapes: BTreeMap<String, Shape>,
}

impl Emitter {
    /// The Kotlin type of a TypeScript one, and its default value.
    fn kotlin(&self, ty: &Ty) -> (String, String) {
        const JSON: (&str, &str) = ("JsonElement", "JsonNull");
        let json = || (JSON.0.to_string(), JSON.1.to_string());
        match ty {
            Ty::StrLit => ("String".into(), "\"\"".into()),
            Ty::Null => ("JsonElement?".into(), "null".into()),
            Ty::OtherLit | Ty::Opaque | Ty::Object(_) => json(),
            Ty::Array(inner) => (format!("List<{}>", self.kotlin(inner).0), "emptyList()".into()),
            Ty::Map(inner) => (format!("Map<String, {}>", self.kotlin(inner).0), "emptyMap()".into()),
            Ty::Union(parts) => {
                let rest: Vec<&Ty> = parts.iter().filter(|p| **p != Ty::Null).collect();
                let nullable = rest.len() < parts.len();
                let inner = if rest.iter().all(|p| matches!(p, Ty::StrLit)) && !rest.is_empty() {
                    ("String".to_string(), "\"\"".to_string())
                } else if rest.len() == 1 {
                    self.kotlin(rest[0])
                } else {
                    json()
                };
                if nullable && !inner.0.ends_with('?') { (format!("{}?", inner.0), "null".into()) } else { inner }
            }
            Ty::Named(name, args) => match (name.as_str(), args.as_slice()) {
                ("string", _) => ("String".into(), "\"\"".into()),
                ("number" | "bigint", _) => ("Double".into(), "0.0".into()),
                ("boolean", _) => ("Boolean".into(), "false".into()),
                ("Array", [inner]) => (format!("List<{}>", self.kotlin(inner).0), "emptyList()".into()),
                ("Record", [_, value]) => (format!("Map<String, {}>", self.kotlin(value).0), "emptyMap()".into()),
                (name, []) => match self.shapes.get(name) {
                    Some(Shape::Class) => (name.to_string(), format!("{name}()")),
                    Some(Shape::Text) => (name.to_string(), "\"\"".into()),
                    // An alias of something else: nullable, so it needs no default of its own.
                    Some(Shape::Alias) => (format!("{name}?"), "null".into()),
                    _ => json(),
                },
                _ => json(),
            },
        }
    }

    fn declare(&self, name: &str, ty: &Ty, shape: Shape, out: &mut String) {
        match (shape, ty) {
            (Shape::Class, Ty::Object(fields)) if fields.is_empty() => out.push_str(&format!("@Serializable\nclass {name}\n\n")),
            (Shape::Class, Ty::Object(fields)) => {
                out.push_str(&format!("@Serializable\ndata class {name}(\n"));
                for f in fields {
                    let (kotlin, default) = self.kotlin(&f.ty);
                    // A type that names itself (a tree) cannot build its own default.
                    let own = matches!(&f.ty, Ty::Named(n, _) if n == name);
                    let (kotlin, default) = if (f.optional || own) && !kotlin.ends_with('?') { (format!("{kotlin}?"), "null".to_string()) } else { (kotlin, default) };
                    let (field, renamed) = field_name(&f.name);
                    if renamed {
                        out.push_str(&format!("    @SerialName({:?}) val {field}: {kotlin} = {default},\n", f.name));
                    } else {
                        out.push_str(&format!("    val {field}: {kotlin} = {default},\n"));
                    }
                }
                out.push_str(")\n\n");
            }
            (Shape::Text, _) => out.push_str(&format!("typealias {name} = String\n\n")),
            (Shape::Alias, ty) => out.push_str(&format!("typealias {name} = {}\n\n", self.kotlin(ty).0.trim_end_matches('?'))),
            _ => out.push_str(&format!("typealias {name} = JsonElement\n\n")),
        }
    }
}

fn shape_of(generic: bool, ty: &Ty) -> Shape {
    if generic {
        return Shape::Json;
    }
    match ty {
        Ty::Object(_) => Shape::Class,
        Ty::StrLit => Shape::Text,
        Ty::Union(parts) if parts.iter().all(|p| matches!(p, Ty::StrLit)) => Shape::Text,
        Ty::Named(n, _) if matches!(n.as_str(), "string" | "number" | "boolean" | "Array" | "Record") => Shape::Alias,
        Ty::Array(_) | Ty::Map(_) => Shape::Alias,
        _ => Shape::Json,
    }
}

const HEADER: &str = "// Generated by `launchpad gen-types` from Home's Rust types (through web/src/gen). Do not edit.\n@file:Suppress(\"unused\", \"PropertyName\", \"ConstructorParameterNaming\", \"ClassName\")\n\npackage dev.launchpad.android.gen\n\n";

/// Reads the ts-rs files of `ts_dir` and writes `Types.kt` and `Actions.kt` into `out_dir`.
pub fn write(ts_dir: &Path, out_dir: &Path) -> Result<()> {
    let mut types = BTreeMap::new();
    for entry in std::fs::read_dir(ts_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        // Hand-written lists (the action list, the folio block catalog), not ts-rs types.
        if stem.starts_with(|c: char| c.is_ascii_lowercase()) {
            continue;
        }
        let Some((name, generic, body)) = declaration(&std::fs::read_to_string(&path)?) else { continue };
        let ty = Parser { src: body.as_bytes(), at: 0 }.union().with_context(|| format!("TypeScript type {name}"))?;
        types.insert(name, (generic, ty));
    }
    let emitter = Emitter { shapes: types.iter().map(|(name, (generic, ty))| (name.clone(), shape_of(*generic, ty))).collect() };
    let mut out = String::from(HEADER);
    out.push_str("import kotlinx.serialization.SerialName\nimport kotlinx.serialization.Serializable\nimport kotlinx.serialization.json.JsonElement\nimport kotlinx.serialization.json.JsonNull\n\n");
    for (name, (_, ty)) in &types {
        emitter.declare(name, ty, emitter.shapes[name], &mut out);
    }
    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("Types.kt"), out)?;
    std::fs::write(out_dir.join("Actions.kt"), actions())?;
    Ok(())
}

/// The action list: each action as its JSON (decoded on first use), keyed by name.
fn actions() -> String {
    let mut out = String::from(HEADER);
    out.push_str("import kotlinx.serialization.json.Json\n\n/** The action list (crates/core/src/actions.rs): every button of the app runs one of these. */\nobject Actions {\n    private val json = Json { ignoreUnknownKeys = true }\n    private val raw: Map<String, String> = mapOf(\n");
    for a in launchpad_core::actions::ACTIONS {
        let text = serde_json::to_string(a).expect("serializable").replace('$', "${'$'}");
        out.push_str(&format!("        {:?} to \"\"\"{text}\"\"\",\n", a.name));
    }
    out.push_str("    )\n    private val decoded = HashMap<String, Action>()\n\n    val names: Set<String> get() = raw.keys\n\n    @Synchronized\n    fun get(name: String): Action? = decoded[name] ?: raw[name]?.let { text -> json.decodeFromString(Action.serializer(), text).also { decoded[name] = it } }\n}\n");
    out
}
