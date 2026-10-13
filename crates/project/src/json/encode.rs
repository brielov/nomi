//! Upstream's JSON output (`Json/Encode.hs`): `write` indents by four
//! spaces, one entry per line, and ends with a newline.

/// A value to write. Strings hold the bytes between the quotes exactly as
/// they will be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Array(Vec<Value>),
    /// Keys are written as given, unescaped, as upstream does.
    Object(Vec<(Vec<u8>, Value)>),
    String(Vec<u8>),
    Bool(bool),
    Int(i64),
    Null,
}

/// Dropped with a loop, as parsed values are.
impl Drop for Value {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_children(self, &mut pending);
        while let Some(mut value) = pending.pop() {
            take_children(&mut value, &mut pending);
        }
    }
}

fn take_children(value: &mut Value, pending: &mut Vec<Value>) {
    match value {
        Value::Array(entries) => pending.append(entries),
        Value::Object(fields) => pending.extend(fields.drain(..).map(|(_, v)| v)),
        _ => {}
    }
}

impl Value {
    /// `string` and `jsonString`: bytes written as they are, which is how
    /// upstream writes back a string it read without decoding escapes.
    pub fn string(bytes: impl Into<Vec<u8>>) -> Value {
        Value::String(bytes.into())
    }

    /// `chars`: text with `\r`, `\n`, `"` and `\` escaped, and nothing else.
    pub fn chars(text: &[u8]) -> Value {
        let mut out = Vec::with_capacity(text.len());
        for &byte in text {
            match byte {
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'"' => out.extend_from_slice(b"\\\""),
                b'\\' => out.extend_from_slice(b"\\\\"),
                _ => out.push(byte),
            }
        }
        Value::String(out)
    }
}

/// `write`: the file upstream writes for `value`.
pub fn write(value: &Value) -> Vec<u8> {
    let mut out = encode(value);
    out.push(b'\n');
    out
}

/// `encode`: `value`, indented, with no newline after it.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let mut indent = Vec::new();
    encode_help(&mut out, &mut indent, value);
    out
}

fn encode_help(out: &mut Vec<u8>, indent: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Array(entries) if entries.is_empty() => out.extend_from_slice(b"[]"),
        Value::Array(entries) => sequence(out, indent, b'[', b']', entries, |out, indent, v| {
            encode_help(out, indent, v);
        }),
        Value::Object(fields) if fields.is_empty() => out.extend_from_slice(b"{}"),
        Value::Object(fields) => {
            sequence(out, indent, b'{', b'}', fields, |out, indent, (key, v)| {
                out.push(b'"');
                out.extend_from_slice(key);
                out.extend_from_slice(b"\": ");
                encode_help(out, indent, v);
            });
        }
        Value::String(bytes) => {
            out.push(b'"');
            out.extend_from_slice(bytes);
            out.push(b'"');
        }
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Null => out.extend_from_slice(b"null"),
    }
}

/// `encodeSequence`: the opener, each entry on its own line one level in,
/// and the closer back at `indent`.
fn sequence<T>(
    out: &mut Vec<u8>,
    indent: &mut Vec<u8>,
    open: u8,
    close: u8,
    entries: &[T],
    mut entry: impl FnMut(&mut Vec<u8>, &mut Vec<u8>, &T),
) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        out.push(open);
        indent.extend_from_slice(b"    ");
        for (i, e) in entries.iter().enumerate() {
            out.extend_from_slice(if i == 0 { b"\n" } else { b",\n" });
            out.extend_from_slice(indent);
            entry(out, indent, e);
        }
        indent.truncate(indent.len() - 4);
        out.push(b'\n');
        out.extend_from_slice(indent);
        out.push(close);
    });
}

/// A parsed value as upstream would write it back: strings and keys keep
/// their bytes, escapes included.
impl From<&super::parse::Value<'_>> for Value {
    fn from(value: &super::parse::Value<'_>) -> Value {
        use super::parse::Kind;
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || match &value.kind {
            Kind::Array(entries) => Value::Array(entries.iter().map(Value::from).collect()),
            Kind::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, v)| (key.bytes.to_vec(), Value::from(v)))
                    .collect(),
            ),
            Kind::String(snippet) => Value::string(snippet.bytes),
            Kind::Int(n) => Value::Int(*n),
            Kind::True => Value::Bool(true),
            Kind::False => Value::Bool(false),
            Kind::Null => Value::Null,
        })
    }
}
