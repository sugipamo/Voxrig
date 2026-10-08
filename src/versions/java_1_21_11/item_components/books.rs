//! Project original stream constructor fields into the common internal model.
use super::values::Value;
use crate::client::{
    books::{Filtered, WritableBook, WrittenBook, validate_generation},
    nbt::NbtString,
    text::Text,
};
use anyhow::{Result, bail};

fn sequence(value: Value, length: usize) -> Result<Vec<Value>> {
    let Value::Sequence(values) = value else {
        bail!("native book sequence required");
    };
    if values.len() != length {
        bail!("native book sequence length mismatch");
    }
    Ok(values)
}
fn string(value: Value) -> Result<NbtString> {
    let Value::String(value) = value else {
        bail!("native book string required");
    };
    Ok(NbtString::from_text(&value))
}
fn text(value: Value) -> Result<Box<Text>> {
    let Value::Text { fields, .. } = value else {
        bail!("native book text required");
    };
    Ok(fields)
}
fn filtered<T>(value: Value, field: fn(Value) -> Result<T>) -> Result<Filtered<T>> {
    let mut values = sequence(value, 2)?.into_iter();
    let raw = field(values.next().unwrap())?;
    let Value::Optional(value) = values.next().unwrap() else {
        bail!("native book filtered optional required");
    };
    let filtered = value.map(|value| field(*value)).transpose()?;
    Ok(Filtered { raw, filtered })
}
fn pages<T>(value: Value, field: fn(Value) -> Result<T>) -> Result<Vec<Filtered<T>>> {
    let Value::List(values) = value else {
        bail!("native book pages required");
    };
    values
        .into_iter()
        .map(|value| filtered(value, field))
        .collect()
}
pub(super) fn writable(value: Value) -> Result<WritableBook> {
    Ok(WritableBook {
        pages: pages(value, string)?,
    })
}
#[inline(never)]
pub(super) fn written(value: Value) -> Result<Box<WrittenBook>> {
    let mut values = sequence(value, 5)?.into_iter();
    let title = filtered(values.next().unwrap(), string)?;
    let author = string(values.next().unwrap())?;
    let Value::Integer(generation) = values.next().unwrap() else {
        bail!("native book generation required");
    };
    validate_generation(generation)?;
    let pages = pages(values.next().unwrap(), text)?;
    let Value::Boolean(resolved) = values.next().unwrap() else {
        bail!("native book resolved flag required");
    };
    Ok(Box::new(WrittenBook {
        title,
        author,
        generation,
        pages,
        resolved,
    }))
}
