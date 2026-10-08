//! Modern SNBT argument validation used by selector predicates; no NBT hash/encoder.
use super::{
    constructor::Limit,
    selector::{Reader, uuid_name},
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug)]
enum Kind {
    Number,
    Boolean,
    String(Vec<u16>),
    Other,
}
#[derive(Deserialize)]
struct NameFacts {
    names: Vec<(u32, String)>,
    ascii_upper: Vec<(u32, String)>,
}
struct Names {
    names: HashMap<String, u32>,
    folds: HashMap<u32, String>,
}
fn names() -> &'static Names {
    static NAMES: OnceLock<Names> = OnceLock::new();
    NAMES.get_or_init(|| {
        let facts: NameFacts = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../data/client_api/character_names-21.0.12.1.json.gz")[..],
        ))
        .expect("pinned JDK name grammar");
        Names {
            names: facts
                .names
                .into_iter()
                .map(|(cp, name)| (name, cp))
                .collect(),
            folds: facts.ascii_upper.into_iter().collect(),
        }
    })
}
fn name_codepoint(value: &[u16]) -> Result<u32> {
    let mut trimmed = value;
    while trimmed.first().is_some_and(|u| *u <= 32) {
        trimmed = &trimmed[1..];
    }
    while trimmed.last().is_some_and(|u| *u <= 32) {
        trimmed = &trimmed[..trimmed.len() - 1];
    }
    if !trimmed
        .iter()
        .all(|u| matches!(*u,32|45|48..=57|65..=90|97..=122))
    {
        bail!("invalid SNBT character name spelling");
    }
    let mut key = String::new();
    for c in char::decode_utf16(trimmed.iter().copied()) {
        let c = c.context("invalid native character name")?;
        if c.is_ascii() {
            key.push(c.to_ascii_uppercase());
        } else {
            key.push_str(
                names()
                    .folds
                    .get(&(c as u32))
                    .context("unknown native named escape")?,
            );
        }
    }
    names()
        .names
        .get(&key)
        .copied()
        .context("unknown native named escape")
}
fn hex(r: &mut Reader<'_>, length: usize) -> Result<u32> {
    let mut result = 0u32;
    for _ in 0..length {
        let c = r.take()?;
        let digit = match c {
            48..=57 => u32::from(c - 48),
            65..=70 => u32::from(c - 65 + 10),
            97..=102 => u32::from(c - 97 + 10),
            _ => bail!("invalid SNBT hex escape"),
        };
        result = (result << 4) | digit;
    }
    Ok(result)
}
fn quoted(r: &mut Reader<'_>) -> Result<Vec<u16>> {
    let quote = r.take()?;
    let mut value = Vec::new();
    loop {
        let c = r.take()?;
        if c == quote {
            return Ok(value);
        }
        if c != 92 {
            value.push(c);
            continue;
        }
        let cp = match r.take()? {
            34 => 34,
            39 => 39,
            92 => 92,
            98 => 8,
            102 => 12,
            110 => 10,
            114 => 13,
            115 => 32,
            116 => 9,
            120 => hex(r, 2)?,
            117 => hex(r, 4)?,
            85 => hex(r, 8)?,
            78 => {
                r.expect(b'{')?;
                let mut name = Vec::new();
                while r.peek() != Some(125) {
                    name.push(r.take()?);
                }
                r.expect(b'}')?;
                name_codepoint(&name)?
            }
            _ => bail!("invalid SNBT escape"),
        };
        if cp > 0x10ffff {
            bail!("invalid SNBT escaped code point");
        }
        if cp <= 65535 {
            value.push(cp as u16);
        } else {
            let cp = cp - 65536;
            value.extend([0xd800 + (cp >> 10) as u16, 0xdc00 + (cp & 1023) as u16]);
        }
    }
}
fn string(r: &mut Reader<'_>) -> Result<Vec<u16>> {
    if matches!(r.peek(), Some(34 | 39)) {
        quoted(r)
    } else {
        r.word()
    }
}
fn text(value: &[u16]) -> Result<String> {
    Ok(String::from_utf16(value)?)
}
fn numeric(token: &str, target: Option<u8>, separated_suffix: bool) -> Result<()> {
    let negative = token.starts_with('-');
    let value = token.strip_prefix(['-', '+']).unwrap_or(token);
    let (radix, value) = if let Some(value) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        (16, value)
    } else if let Some(value) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        (2, value)
    } else {
        (10, value)
    };
    let lower = value.to_ascii_lowercase();
    let float =
        radix == 10 && (lower.contains('.') || lower.contains('e') || lower.ends_with(['f', 'd']));
    if float {
        if target.is_some() {
            bail!("floating typed SNBT array element");
        }
        let body = token.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(token);
        if !valid_underscores(body, 10) {
            bail!("invalid SNBT numeric underscore");
        }
        let number = body.replace('_', "").parse::<f64>()?;
        if !number.is_finite() || token.ends_with(['f', 'F']) && !(number as f32).is_finite() {
            bail!("infinite SNBT literal");
        }
        return Ok(());
    }
    let mut body = value;
    let mut width = target.unwrap_or(2);
    let mut explicit_unsigned = None;
    // Hex numerals greedily include b/d/f digits. An explicit u/s prefix
    // disambiguates suffixes; decimal/binary suffixes need no such prefix.
    let suffix = body
        .as_bytes()
        .last()
        .copied()
        .map(|u| u.to_ascii_lowercase());
    if let Some(kind @ (b'b' | b's' | b'i' | b'l')) = suffix {
        let prefix = body
            .as_bytes()
            .get(body.len().saturating_sub(2))
            .copied()
            .map(|u| u.to_ascii_lowercase());
        if radix != 16 || kind != b'b' || separated_suffix || matches!(prefix, Some(b'u' | b's')) {
            width = match kind {
                b'b' => 0,
                b's' => 1,
                b'i' => 2,
                _ => 3,
            };
            body = &body[..body.len() - 1];
            if matches!(prefix, Some(b'u' | b's')) {
                explicit_unsigned = Some(prefix == Some(b'u'));
                body = &body[..body.len() - 1];
            }
        }
    }
    if target.is_some_and(|limit| width > limit) {
        bail!("invalid typed SNBT integer width");
    }
    if !valid_underscores(body, radix) {
        bail!("invalid SNBT integer numeral");
    }
    let digits = body.replace('_', "");
    if radix == 10 && digits.len() > 1 && digits.starts_with('0') {
        bail!("leading SNBT decimal zero");
    }
    let magnitude = u128::from_str_radix(&digits, radix)?;
    let bits = [8, 16, 32, 64][width as usize];
    let unsigned = explicit_unsigned.unwrap_or(radix != 10);
    if unsigned {
        if negative || magnitude > (1u128 << bits) - 1 {
            bail!("out of range unsigned SNBT integer");
        }
    } else {
        let maximum = (1u128 << (bits - 1)) - 1;
        if magnitude > maximum + u128::from(negative) {
            bail!("out of range signed SNBT integer");
        }
    }
    Ok(())
}
fn valid_underscores(value: &str, radix: u32) -> bool {
    let mut parts = value.split('_');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty() || value.ends_with('_') {
        return false;
    }
    value.chars().all(|c| {
        c == '_' || c.is_digit(radix) || radix == 10 && matches!(c, '.' | 'e' | 'E' | '+' | '-')
    }) && value
        .split(['.', 'e', 'E', '+', '-'])
        .all(|run| run.is_empty() || !run.starts_with('_') && !run.ends_with('_'))
}
fn numeric_token(r: &mut Reader<'_>, mut token: String) -> Result<(String, bool)> {
    let mut separated_suffix = false;
    loop {
        let saved = r.cursor;
        r.space()?;
        let next = text(&r.word()?)?;
        let lower = next.to_ascii_lowercase();
        let base = token.trim_start_matches(['+', '-']);
        let base_marker = base == "0" && matches!(lower.as_str(), "b" | "x");
        let base_digits = matches!(base, "0b" | "0B" | "0x" | "0X");
        let hex_base = base.starts_with("0x") || base.starts_with("0X");
        let exponent_end = !hex_base && token.ends_with(['e', 'E']);
        let suffix = matches!(
            lower.as_str(),
            "b" | "s"
                | "i"
                | "l"
                | "f"
                | "d"
                | "u"
                | "ub"
                | "us"
                | "ui"
                | "ul"
                | "sb"
                | "ss"
                | "si"
                | "sl"
        );
        let continuation = if base_digits {
            next.as_bytes().first().is_some_and(|c| {
                if hex_base {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_digit()
                }
            })
        } else if exponent_end || token.ends_with(['.', '+', '-', 'u', 'U']) {
            next.as_bytes().first().is_some_and(|c| c.is_ascii_digit())
                || exponent_end && next.starts_with(['+', '-'])
                || token.ends_with(['u', 'U']) && suffix
        } else {
            base_marker || suffix || matches!(lower.as_str(), "e") || next.starts_with('.')
        };
        if !continuation {
            r.cursor = saved;
            break;
        }
        separated_suffix |= suffix && !base_marker;
        token.push_str(&next);
    }
    Ok((token, separated_suffix))
}
fn value(r: &mut Reader<'_>, depth: usize) -> Result<Kind> {
    if depth > 64 {
        return Err(Limit("selector SNBT depth limit").into());
    }
    r.space()?;
    match r.peek() {
        Some(123) => {
            compound(r, depth)?;
            Ok(Kind::Other)
        }
        Some(91) => {
            list(r, depth)?;
            Ok(Kind::Other)
        }
        Some(34 | 39) => Ok(Kind::String(quoted(r)?)),
        _ => {
            let raw = r.word()?;
            if raw.is_empty() {
                bail!("SNBT value required");
            }
            let token = text(&raw)?;
            if token
                .as_bytes()
                .first()
                .is_some_and(|u| u.is_ascii_digit() || b"+-.".contains(u))
            {
                let (token, separated_suffix) = numeric_token(r, token)?;
                numeric(&token, None, separated_suffix)?;
                return Ok(Kind::Number);
            }
            r.space()?;
            if r.consume(b'(')? {
                let arg = value(r, depth + 1)?;
                r.space()?;
                if r.consume(b',')? {
                    r.space()?;
                }
                r.expect(b')')?;
                match (token.as_str(), arg) {
                    ("bool", Kind::Number | Kind::Boolean) => Ok(Kind::Boolean),
                    ("uuid", Kind::String(s)) if uuid_name(&s) => Ok(Kind::Other),
                    _ => bail!("unknown or invalid SNBT operation"),
                }
            } else if matches!(token.as_str(), "true" | "false") {
                Ok(Kind::Boolean)
            } else {
                Ok(Kind::String(raw))
            }
        }
    }
}
fn list(r: &mut Reader<'_>, depth: usize) -> Result<()> {
    r.expect(b'[')?;
    r.space()?;
    let target = if matches!(r.peek(), Some(66 | 73 | 76)) && r.units.get(r.cursor + 1) == Some(&59)
    {
        let prefix = r.take()?;
        r.expect(b';')?;
        r.space()?;
        Some(match prefix {
            66 => 0,
            73 => 2,
            _ => 3,
        })
    } else {
        None
    };
    while r.peek() != Some(93) {
        if let Some(target) = target {
            let raw = r.word()?;
            let (token, separated_suffix) = numeric_token(r, text(&raw)?)?;
            numeric(&token, Some(target), separated_suffix)?;
        } else {
            value(r, depth + 1)?;
        }
        r.space()?;
        if !r.consume(b',')? {
            break;
        }
        r.space()?;
    }
    r.expect(b']')?;
    Ok(())
}
pub(super) fn compound(r: &mut Reader<'_>, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(Limit("selector SNBT depth limit").into());
    }
    r.space()?;
    r.expect(b'{')?;
    r.space()?;
    while r.peek() != Some(125) {
        if string(r)?.is_empty() {
            bail!("empty SNBT compound key");
        }
        r.space()?;
        r.expect(b':')?;
        value(r, depth + 1)?;
        r.space()?;
        if !r.consume(b',')? {
            break;
        }
        r.space()?;
    }
    r.expect(b'}')?;
    Ok(())
}
