//! Regular expressions shared by the interpreter, VM, and string methods.
//!
//! Patterns use a bounded `fancy-regex` backend. See
//! `docs/js/regular-expressions.md` for the supported ECMAScript subset.
//! Matching uses Unicode scalar values (even without `u`), while result
//! positions use UTF-16 code units. Lone surrogates, full Unicode grammar,
//! legacy escape/backreference corner cases, and symbol protocols are not
//! implemented. Pattern compilation and backtracking have independent limits.
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use fancy_regex::{Regex, RegexBuilder};

use super::conversions::{to_number, to_uint32};
use super::{Function, Object, host_this};
use crate::{JsError, JsErrorCategory, JsResult, Value};

/// Immutable compiled pattern; mutable `lastIndex` belongs to its JS object.
pub(crate) struct RegExpData {
    regex: Regex,
    pattern: String,
    flags: String,
}

impl RegExpData {
    fn has(&self, flag: char) -> bool {
        self.flags.contains(flag)
    }

    pub(crate) fn display(&self) -> String {
        format!("/{}/{}", escape_source(&self.pattern), self.flags)
    }

    fn find(&self, text: &str, from: usize, sticky: bool) -> JsResult<Option<Match>> {
        let captures = self.regex.captures_from_pos(text, from).map_err(|error| {
            JsError::with_context(
                JsErrorCategory::Limit,
                error.to_string(),
                "regular expression matching",
            )
        })?;
        let Some(captures) = captures else {
            return Ok(None);
        };
        let whole = captures.get(0).expect("whole match");
        if sticky && whole.start() != from {
            return Ok(None);
        }
        let ranges = captures
            .iter()
            .map(|value| value.map(|value| value.range()))
            .collect();
        let names = self
            .regex
            .capture_names()
            .enumerate()
            .filter_map(|(index, name)| name.map(|name| (name.to_owned(), index)))
            .collect();
        Ok(Some(Match { ranges, names }))
    }
}

struct Match {
    ranges: Vec<Option<Range<usize>>>,
    names: Vec<(String, usize)>,
}
impl Match {
    fn range(&self) -> Range<usize> {
        self.ranges[0].clone().expect("whole match")
    }
    fn value(&self, text: &str, index: usize) -> Value {
        self.ranges
            .get(index)
            .and_then(Option::as_ref)
            .map(|range| Value::String(text[range.clone()].into()))
            .unwrap_or(Value::Undefined)
    }
    fn groups(&self, text: &str) -> Value {
        if self.names.is_empty() {
            return Value::Undefined;
        }
        let object = Object::new();
        for (name, index) in &self.names {
            object.set(name, self.value(text, *index));
        }
        Value::Object(object)
    }
    fn array(&self, text: &str, indices: bool) -> Value {
        let array = values_array(
            self.ranges
                .iter()
                .enumerate()
                .map(|(index, _)| self.value(text, index))
                .collect(),
        );
        array.set(
            "index",
            Value::Number(utf16_index(text, self.range().start) as f64),
        );
        array.set("input", Value::String(text.into()));
        array.set("groups", self.groups(text));
        if indices {
            let pairs: Vec<Value> = self
                .ranges
                .iter()
                .map(|range| match range {
                    Some(range) => Value::Array(values_array(vec![
                        Value::Number(utf16_index(text, range.start) as f64),
                        Value::Number(utf16_index(text, range.end) as f64),
                    ])),
                    None => Value::Undefined,
                })
                .collect();
            let groups = if self.names.is_empty() {
                Value::Undefined
            } else {
                let groups = Object::new();
                for (name, index) in &self.names {
                    groups.set(name, pairs[*index].clone());
                }
                Value::Object(groups)
            };
            let indices = values_array(pairs);
            indices.set("groups", groups);
            array.set("indices", Value::Array(indices));
        }
        Value::Array(array)
    }
}

fn values_array(values: Vec<Value>) -> Object {
    let object = Object::new();
    object.set("length", Value::Number(values.len() as f64));
    for (index, value) in values.into_iter().enumerate() {
        object.set(index.to_string(), value);
    }
    object
}

fn compile(pattern: &str, flags: &str) -> Result<RegExpData, String> {
    let mut seen = String::new();
    for flag in flags.chars() {
        if !"dgimsuy".contains(flag) || seen.contains(flag) {
            return Err(format!(
                "Invalid or unsupported regular expression flag: {flag}"
            ));
        }
        seen.push(flag);
    }
    let flags: String = "dgimsuy"
        .chars()
        .filter(|flag| seen.contains(*flag))
        .collect();
    let translated = translate(pattern, &flags)?;
    let regex = RegexBuilder::new(&translated)
        .case_insensitive(flags.contains('i'))
        .backtrack_limit(100_000)
        .delegate_size_limit(2 * 1024 * 1024)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(RegExpData {
        regex,
        pattern: pattern.into(),
        flags,
    })
}

/// Validates literal syntax without creating script-visible mutable state.
pub(crate) fn validate(pattern: &str, flags: &str) -> Result<(), String> {
    compile(pattern, flags).map(|_| ())
}

// Map the common JS character classes and line terminators explicitly;
// Rust's default \w, \d, \s, dot, ^, and $ have different Unicode semantics.
fn translate(pattern: &str, flags: &str) -> Result<String, String> {
    if pattern.len() > 64 * 1024 {
        return Err("Regular expression pattern exceeds 64 KiB".into());
    }
    let mut output = String::new();
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let escaped = chars
                .next()
                .ok_or("Trailing backslash in regular expression")?;
            let mapped = match escaped {
                '/' => "/",
                'd' => "[0-9]",
                'D' => "[^0-9]",
                'w' => "[A-Za-z0-9_]",
                'W' => "[^A-Za-z0-9_]",
                's' => {
                    "[\\x09-\\x0D \\u{00a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}]"
                }
                'S' => {
                    "[^\\x09-\\x0D \\u{00a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}]"
                }
                'b' if in_class => "\\x08",
                'B' if in_class => {
                    if flags.contains('u') {
                        return Err("Invalid class escape: \\B".into());
                    }
                    "B"
                }
                'b' => "(?:(?<=[A-Za-z0-9_])(?![A-Za-z0-9_])|(?<![A-Za-z0-9_])(?=[A-Za-z0-9_]))",
                'B' => "(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_]))",
                'v' => "\\x0b",
                '0' if !chars.peek().is_some_and(char::is_ascii_digit) => "\\x00",
                'c' => {
                    let letter = chars
                        .next()
                        .filter(char::is_ascii_alphabetic)
                        .ok_or("Invalid control escape")?;
                    output.push_str(&format!("\\x{:02x}", (letter as u8) % 32));
                    continue;
                }
                // Reject backend-only constructs rather than interpreting them
                // as Rust/PCRE semantics that JS does not have.
                value if value.is_ascii_alphabetic() && !"fnrtuxkpP".contains(value) => {
                    if flags.contains('u') {
                        return Err(format!("Invalid escape: \\{escaped}"));
                    }
                    output.push(escaped);
                    continue;
                }
                _ => {
                    output.push('\\');
                    output.push(escaped);
                    continue;
                }
            };
            output.push_str(mapped);
        } else {
            match ch {
                '[' if !in_class => {
                    if chars.peek() == Some(&']') {
                        chars.next();
                        output.push_str("(?!)");
                    } else if chars.clone().take(2).collect::<String>() == "^]" {
                        chars.next();
                        chars.next();
                        output.push_str("[\\s\\S]");
                    } else {
                        in_class = true;
                        output.push(ch);
                    }
                }
                '[' if in_class => output.push_str("\\["),
                // These operators are ordinary repeated characters in legacy
                // JS classes, whereas Rust treats them as set operations.
                '&' | '~' if in_class => {
                    output.push('\\');
                    output.push(ch);
                }
                '(' if !in_class && chars.peek() == Some(&'?') => {
                    let next = chars.clone().nth(1);
                    if !matches!(next, Some(':' | '=' | '!' | '<')) {
                        return Err("Unsupported regular expression group extension".into());
                    }
                    output.push(ch);
                }
                '*' | '+' | '?' | '}' if !in_class && chars.peek() == Some(&'+') => {
                    return Err("Possessive quantifiers are not JavaScript syntax".into());
                }
                ']' => {
                    in_class = false;
                    output.push(ch);
                }
                '.' if !in_class => output.push_str(if flags.contains('s') {
                    "[\\s\\S]"
                } else {
                    "[^\\n\\r\\u{2028}\\u{2029}]"
                }),
                '^' if !in_class => output.push_str(if flags.contains('m') {
                    "(?:\\A|(?<=[\\n\\r\\u{2028}\\u{2029}]))"
                } else {
                    "\\A"
                }),
                '$' if !in_class => output.push_str(if flags.contains('m') {
                    "(?:\\z|(?=[\\n\\r\\u{2028}\\u{2029}]))"
                } else {
                    "\\z"
                }),
                _ => output.push(ch),
            }
        }
    }
    Ok(output)
}

fn escape_source(pattern: &str) -> String {
    if pattern.is_empty() {
        return "(?:)".into();
    }
    let mut output = String::new();
    let mut escaped = false;
    for ch in pattern.chars() {
        match ch {
            '/' if !escaped => output.push_str("\\/"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\u{2028}' => output.push_str("\\u2028"),
            '\u{2029}' => output.push_str("\\u2029"),
            _ => output.push(ch),
        }
        escaped = ch == '\\' && !escaped;
    }
    output
}

fn syntax_error(message: String) -> JsError {
    JsError::thrown(super::errors::new_error("SyntaxError", &message))
}
fn type_error() -> JsError {
    JsError::thrown(super::errors::new_error(
        "TypeError",
        "RegExp method requires a regular expression receiver",
    ))
}

/// Creates a fresh branded regular expression with its own writable lastIndex.
pub(crate) fn create(pattern: &str, flags: &str) -> JsResult<Object> {
    let data = Arc::new(compile(pattern, flags).map_err(syntax_error)?);
    let object = Object::with_prototype(Some(prototype()));
    object.set("lastIndex", Value::Number(0.0));
    object.set_read_only("source", Value::String(escape_source(pattern)));
    object.set_read_only("flags", Value::String(data.flags.clone()));
    for (name, flag) in [
        ("global", 'g'),
        ("ignoreCase", 'i'),
        ("multiline", 'm'),
        ("dotAll", 's'),
        ("unicode", 'u'),
        ("sticky", 'y'),
        ("hasIndices", 'd'),
    ] {
        object.set_read_only(name, Value::Boolean(data.has(flag)));
    }
    object.set_regexp_data(data);
    Ok(object)
}

/// Creates the callable and constructible RegExp global.
pub(crate) fn constructor() -> Function {
    let function = Function::from_host_constructor(
        "RegExp",
        Arc::new(|args| construct(args, false)),
        Arc::new(|args| construct(args, true)),
    );
    function.set_own("prototype", Value::Object(prototype()));
    prototype().set_constructor(&function);
    function
}
fn construct(args: &[Value], fresh: bool) -> JsResult<Value> {
    let flags = args.get(1).filter(|value| **value != Value::Undefined);
    if let Some(Value::Object(object)) = args.first()
        && let Some(data) = object.regexp_data()
    {
        if !fresh && flags.is_none() {
            return Ok(Value::Object(object.clone()));
        }
        return create(
            &data.pattern,
            &flags
                .map(ToString::to_string)
                .unwrap_or_else(|| data.flags.clone()),
        )
        .map(Value::Object);
    }
    let pattern = args
        .first()
        .filter(|value| **value != Value::Undefined)
        .map(ToString::to_string)
        .unwrap_or_default();
    create(
        &pattern,
        &flags.map(ToString::to_string).unwrap_or_default(),
    )
    .map(Value::Object)
}
fn prototype() -> Object {
    static PROTOTYPE: OnceLock<Object> = OnceLock::new();
    PROTOTYPE
        .get_or_init(|| {
            let object = Object::new();
            object.set_method(
                "exec",
                Arc::new(|args| {
                    let object = receiver()?;
                    let text = argument(args.first());
                    Ok(exec_match(&object, &text)?
                        .map(|matched| {
                            matched.array(&text, object.regexp_data().expect("brand").has('d'))
                        })
                        .unwrap_or(Value::Null))
                }),
            );
            object.set_method(
                "test",
                Arc::new(|args| {
                    let object = receiver()?;
                    Ok(Value::Boolean(
                        exec_match(&object, &argument(args.first()))?.is_some(),
                    ))
                }),
            );
            object.set_method(
                "toString",
                Arc::new(|_| {
                    let object = receiver()?;
                    Ok(Value::String(
                        object.regexp_data().expect("brand").display(),
                    ))
                }),
            );
            object
        })
        .clone()
}
fn receiver() -> JsResult<Object> {
    match host_this() {
        Value::Object(object) if object.regexp_data().is_some() => Ok(object),
        _ => Err(type_error()),
    }
}
fn argument(value: Option<&Value>) -> String {
    value.unwrap_or(&Value::Undefined).to_string()
}
fn utf16_index(text: &str, byte: usize) -> usize {
    text[..byte].encode_utf16().count()
}
fn byte_index(text: &str, units: usize) -> Option<usize> {
    let mut count = 0;
    for (byte, ch) in text.char_indices() {
        if count >= units {
            return Some(byte);
        }
        count += ch.len_utf16();
    }
    (count == units).then_some(text.len())
}
fn next_index(text: &str, byte: usize) -> usize {
    text[byte..]
        .chars()
        .next()
        .map(|ch| byte + ch.len_utf8())
        .unwrap_or(text.len() + 1)
}
fn set_index(object: &Object, index: usize) -> JsResult<()> {
    object.set_property("lastIndex", Value::Number(index as f64))
}
fn exec_match(object: &Object, text: &str) -> JsResult<Option<Match>> {
    let data = object.regexp_data().ok_or_else(type_error)?;
    let stateful = data.has('g') || data.has('y');
    let from = if stateful {
        let number = to_number(
            &object
                .get_property("lastIndex")?
                .unwrap_or(Value::Undefined),
        );
        let units = if number.is_nan() || number < 0.0 {
            0
        } else {
            number as usize
        };
        let Some(byte) = byte_index(text, units) else {
            set_index(object, 0)?;
            return Ok(None);
        };
        byte
    } else {
        0
    };
    let matched = data.find(text, from, data.has('y'))?;
    if stateful {
        set_index(
            object,
            matched
                .as_ref()
                .map(|value| utf16_index(text, value.range().end))
                .unwrap_or(0),
        )?;
    }
    Ok(matched)
}

/// Handles a regex argument to String.match/search/replace/split. Other
/// methods or non-regex replace/split arguments are left to the string host.
pub(super) fn string_method(name: &str, text: &str, args: &[Value]) -> JsResult<Option<Value>> {
    let object = match args.first() {
        Some(Value::Object(object)) if object.regexp_data().is_some() => object.clone(),
        _ if name == "match" || name == "search" => {
            let pattern = args
                .first()
                .filter(|value| **value != Value::Undefined)
                .map(ToString::to_string)
                .unwrap_or_default();
            create(&pattern, "")?
        }
        _ => return Ok(None),
    };
    let data = object.regexp_data().expect("brand");
    Ok(Some(match name {
        "search" => Value::Number(
            data.find(text, 0, data.has('y'))?
                .map(|matched| utf16_index(text, matched.range().start) as f64)
                .unwrap_or(-1.0),
        ),
        "match" if !data.has('g') => exec_match(&object, text)?
            .map(|matched| matched.array(text, data.has('d')))
            .unwrap_or(Value::Null),
        "match" => {
            set_index(&object, 0)?;
            let mut matches = Vec::new();
            while let Some(matched) = exec_match(&object, text)? {
                let range = matched.range();
                matches.push(Value::String(text[range.clone()].into()));
                if range.is_empty() {
                    set_index(
                        &object,
                        if range.end == text.len() {
                            text.encode_utf16().count() + 1
                        } else {
                            utf16_index(text, next_index(text, range.end))
                        },
                    )?;
                }
            }
            if matches.is_empty() {
                Value::Null
            } else {
                Value::Array(values_array(matches))
            }
        }
        "replace" => replace(&object, text, args.get(1).unwrap_or(&Value::Undefined))?,
        "split" => split(&data, text, args.get(1))?,
        _ => return Ok(None),
    }))
}

fn replace(object: &Object, text: &str, replacement: &Value) -> JsResult<Value> {
    let data = object.regexp_data().expect("brand");
    if data.has('g') {
        set_index(object, 0)?;
    }
    let mut matches = Vec::new();
    while let Some(matched) = exec_match(object, text)? {
        let range = matched.range();
        matches.push(matched);
        if !data.has('g') {
            break;
        }
        if range.is_empty() {
            set_index(
                object,
                if range.end == text.len() {
                    text.encode_utf16().count() + 1
                } else {
                    utf16_index(text, next_index(text, range.end))
                },
            )?;
        }
    }
    let mut output = String::new();
    let mut cursor = 0;
    for matched in matches {
        let range = matched.range();
        output.push_str(&text[cursor..range.start]);
        if let Value::Function(function) = replacement {
            let mut args: Vec<Value> = matched
                .ranges
                .iter()
                .enumerate()
                .map(|(index, _)| matched.value(text, index))
                .collect();
            args.push(Value::Number(utf16_index(text, range.start) as f64));
            args.push(Value::String(text.into()));
            if !matched.names.is_empty() {
                args.push(matched.groups(text));
            }
            output.push_str(
                &function
                    .call_with_this(Value::Undefined, &args)?
                    .to_string(),
            );
        } else {
            output.push_str(&substitute(&replacement.to_string(), text, &matched));
        }
        cursor = range.end;
    }
    output.push_str(&text[cursor..]);
    Ok(Value::String(output))
}
fn substitute(replacement: &str, text: &str, matched: &Match) -> String {
    let mut output = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                output.push('$');
            }
            Some('&') => {
                chars.next();
                output.push_str(&text[matched.range()]);
            }
            Some('`') => {
                chars.next();
                output.push_str(&text[..matched.range().start]);
            }
            Some('\'') => {
                chars.next();
                output.push_str(&text[matched.range().end..]);
            }
            Some(first @ '0'..='9') => {
                let mut index = first.to_digit(10).expect("digit") as usize;
                let mut lookahead = chars.clone();
                lookahead.next();
                let second = lookahead.peek().and_then(|ch| ch.to_digit(10));
                let double = second.map(|digit| index * 10 + digit as usize);
                if let Some(double) =
                    double.filter(|index| *index > 0 && *index < matched.ranges.len())
                {
                    chars.next();
                    chars.next();
                    index = double;
                } else if index > 0 && index < matched.ranges.len() {
                    chars.next();
                } else {
                    output.push('$');
                    continue;
                }
                if let Value::String(value) = matched.value(text, index) {
                    output.push_str(&value);
                }
            }
            Some('<') if !matched.names.is_empty() => {
                let mut probe = chars.clone();
                probe.next();
                let mut name = String::new();
                let mut closed = false;
                for ch in probe.by_ref() {
                    if ch == '>' {
                        closed = true;
                        break;
                    }
                    name.push(ch);
                }
                if closed {
                    chars = probe;
                    if let Some((_, index)) = matched.names.iter().find(|(key, _)| *key == name)
                        && let Value::String(value) = matched.value(text, *index)
                    {
                        output.push_str(&value);
                    }
                } else {
                    output.push('$');
                }
            }
            _ => output.push('$'),
        }
    }
    output
}
fn split(data: &RegExpData, text: &str, limit: Option<&Value>) -> JsResult<Value> {
    let limit = limit
        .filter(|value| **value != Value::Undefined)
        .map(to_uint32)
        .unwrap_or(u32::MAX) as usize;
    let mut values = Vec::new();
    if limit == 0 {
        return Ok(Value::Array(values_array(values)));
    }
    if text.is_empty() {
        if data.find(text, 0, false)?.is_none() {
            values.push(Value::String(String::new()));
        }
        return Ok(Value::Array(values_array(values)));
    }
    let mut cursor = 0;
    let mut from = 0;
    while from < text.len() {
        let Some(matched) = data.find(text, from, false)? else {
            break;
        };
        let range = matched.range();
        if range.end == cursor {
            from = next_index(text, from);
            continue;
        }
        if range.is_empty() && range.start == text.len() {
            break;
        }
        values.push(Value::String(text[cursor..range.start].into()));
        for index in 1..matched.ranges.len() {
            values.push(matched.value(text, index));
        }
        cursor = range.end;
        from = if range.is_empty() {
            next_index(text, range.end)
        } else {
            range.end
        };
        if values.len() >= limit {
            values.truncate(limit);
            return Ok(Value::Array(values_array(values)));
        }
    }
    values.push(Value::String(text[cursor..].into()));
    values.truncate(limit);
    Ok(Value::Array(values_array(values)))
}
