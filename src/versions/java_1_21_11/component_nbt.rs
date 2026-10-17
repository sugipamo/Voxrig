//! Bounded network-NBT projection for received text components. This preserves
//! translation keys and arguments; it does not execute or render click events.
use super::wire::Reader;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

pub(super) fn read(r: &mut Reader<'_>) -> Result<Value> {
    let kind = r.u8()?;
    value(r, kind, 0, &mut 16384)
}
fn value(r: &mut Reader<'_>, kind: u8, depth: usize, budget: &mut usize) -> Result<Value> {
    if depth > 32 || *budget == 0 {
        bail!("text component limit");
    }
    *budget -= 1;
    Ok(match kind {
        1 => Value::from(r.u8()? as i8),
        2 => Value::from(r.u16()? as i16),
        3 => Value::from(r.i32()?),
        4 => Value::from(r.u64()? as i64),
        5 => Value::from(r.f32()?),
        6 => Value::from(r.f64()?),
        8 => Value::from(r.nbt_string()?),
        7 | 9 | 11 | 12 => {
            let item = if kind == 9 {
                r.u8()?
            } else {
                match kind {
                    7 => 1,
                    11 => 3,
                    _ => 4,
                }
            };
            let count = usize::try_from(r.i32()?).context("negative component list")?;
            if count > *budget || (item == 0 && count != 0) {
                bail!("component list limit");
            }
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(value(r, item, depth + 1, budget)?);
            }
            Value::Array(values)
        }
        10 => {
            let mut values = Map::new();
            while let Some((item, key)) = r.nbt_field()? {
                values.insert(key, value(r, item, depth + 1, budget)?);
            }
            Value::Object(values)
        }
        _ => bail!("invalid text component NBT tag"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modified_utf8_text_and_keys_project_without_replacing_surrogates() {
        for (fixture, text) in [
            ("0a08000474657874000568656c6c6f00", "hello"),
            ("0a080004746578740006eda0bdedb88000", "😀"),
            ("0a08000474657874000461c0806200", "a\0b"),
        ] {
            let bytes = hex::decode(fixture).unwrap();
            let mut r = Reader::new(&bytes);
            assert_eq!(read(&mut r).unwrap(), serde_json::json!({"text":text}));
            r.end().unwrap();
            for end in 0..bytes.len() {
                assert!(read(&mut Reader::new(&bytes[..end])).is_err());
            }
        }
        let bytes = b"\x0a\x08\x00\x08\xed\xa0\xbd\xed\xb8\x80\xc0\x80\x00\x01x\x00";
        assert_eq!(
            read(&mut Reader::new(bytes)).unwrap(),
            serde_json::json!({"😀\0":"x"})
        );
        for units in [&b"\xed\xa0\x80"[..], b"\xed\xb0\x80"] {
            let mut value = vec![8, 0, 3];
            value.extend(units);
            let error = read(&mut Reader::new(&value)).unwrap_err();
            assert!(error.to_string().contains("unpaired surrogate"));
            let mut key = vec![10, 1, 0, 3];
            key.extend(units);
            key.extend([1, 0]);
            assert!(
                read(&mut Reader::new(&key))
                    .unwrap_err()
                    .to_string()
                    .contains("unpaired surrogate")
            );
        }
    }

    #[test]
    fn preserves_literal_components_and_rejects_truncation_and_depth() {
        let data = b"\x0a\x08\x00\x04text\x00\x05hello\x00";
        assert_eq!(
            read(&mut Reader::new(data)).unwrap(),
            serde_json::json!({"text":"hello"})
        );
        for end in 0..data.len() {
            assert!(read(&mut Reader::new(&data[..end])).is_err());
        }
        assert!(read(&mut Reader::new(&[9, 8, 0x7f, 0xff, 0xff, 0xff])).is_err());
        let mut deep = vec![10];
        for _ in 0..34 {
            deep.extend([10, 0, 0]);
        }
        assert!(read(&mut Reader::new(&deep)).is_err());
    }
}
