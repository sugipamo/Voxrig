//! Internal UUID constructor fields. Strict profile words and JDK lenient
//! strings remain distinct entry points; neither performs entity/profile lookup.
use super::nbt::NbtValue;
use anyhow::{Result, bail};
use serde::Deserialize;
use std::sync::OnceLock;
fn integer(value: &NbtValue) -> Result<i32> {
    Ok(match value {
        NbtValue::Byte(value) => i32::from(*value),
        NbtValue::Short(value) => i32::from(*value),
        NbtValue::Int(value) => *value,
        NbtValue::Long(value) => *value as i32,
        NbtValue::Float { bits } => f32::from_bits(*bits) as i32,
        NbtValue::Double { bits } => f64::from_bits(*bits) as i32,
        _ => bail!("native UUID words must be numeric"),
    })
}
pub(crate) fn from_nbt(value: &NbtValue) -> Result<[i32; 4]> {
    let words: Vec<_> = match value {
        NbtValue::IntArray(values) => values.clone(),
        NbtValue::ByteArray(values) => values.iter().map(|n| i32::from(*n)).collect(),
        NbtValue::LongArray(values) => values.iter().map(|n| *n as i32).collect(),
        NbtValue::List(values) => values
            .iter()
            .map(|value| integer(value))
            .collect::<Result<_>>()?,
        _ => bail!("native UUID requires numeric array/list"),
    };
    words
        .try_into()
        .map_err(|_| anyhow::anyhow!("native UUID requires 4 words"))
}
pub(crate) fn from_string(units: &[u16]) -> Option<[i32; 4]> {
    if units.len() > 36 {
        return None;
    }
    #[derive(Deserialize)]
    struct Digits {
        hex_utf16_digits: std::collections::BTreeMap<String, u8>,
    }
    static DIGITS: OnceLock<std::collections::BTreeMap<u16, u8>> = OnceLock::new();
    let digits = DIGITS.get_or_init(|| {
        let facts: Digits = serde_json::from_str(include_str!(
            "../../data/client_api/text_color_rules-1.21.11.json"
        ))
        .expect("pinned JDK hex digit grammar");
        facts
            .hex_utf16_digits
            .into_iter()
            .map(|(u, n)| (u.parse().unwrap(), n))
            .collect()
    });
    let parts: Vec<_> = units.split(|u| *u == 45).collect();
    if parts.len() != 5 {
        return None;
    }
    let mut groups = [0u64; 5];
    for (out, part) in groups.iter_mut().zip(parts) {
        let part = part.strip_prefix(&[43]).unwrap_or(part);
        if part.is_empty() {
            return None;
        }
        let mut number = 0i64;
        for unit in part {
            number = number
                .checked_mul(16)?
                .checked_add(i64::from(*digits.get(unit)?))?;
        }
        *out = number as u64;
    }
    let most =
        ((groups[0] & 0xffffffff) << 32) | ((groups[1] & 0xffff) << 16) | (groups[2] & 0xffff);
    let least = ((groups[3] & 0xffff) << 48) | (groups[4] & 0xffffffffffff);
    Some([
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ])
}
