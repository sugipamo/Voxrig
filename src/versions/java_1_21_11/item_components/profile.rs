//! Constructor normalization for the pinned original profile stream root.
use super::values::Value;
use crate::client::{
    nbt::NbtString,
    profile::{self, Profile, Property, Skin, Texture},
    text::Identifier,
};
use anyhow::{Context, Result, bail};

fn sequence(value: &Value, length: usize) -> Result<&[Value]> {
    let Value::Sequence(values) = value else {
        bail!("native profile sequence required");
    };
    if values.len() != length {
        bail!("native profile sequence length mismatch");
    }
    Ok(values)
}
fn optional(value: &Value) -> Result<Option<&Value>> {
    let Value::Optional(value) = value else {
        bail!("native profile optional required");
    };
    Ok(value.as_deref())
}
fn string(value: &Value) -> Result<NbtString> {
    let Value::String(value) = value else {
        bail!("native profile wire string required");
    };
    Ok(NbtString::from_text(value))
}
fn uuid(value: &Value) -> Result<[i32; 4]> {
    let Value::Uuid { most, least } = value else {
        bail!("native profile UUID required");
    };
    Ok([
        (*most >> 32) as i32,
        *most as i32,
        (*least >> 32) as i32,
        *least as i32,
    ])
}
fn texture(value: &Value) -> Result<Texture> {
    let Value::Forward { codec: 594, value } = value else {
        bail!("native resource texture forward required");
    };
    let Value::Identifier { namespace, path } = &**value else {
        bail!("native texture identifier required");
    };
    Ok(Texture::new(Identifier {
        namespace: namespace.clone(),
        path: path.clone(),
    }))
}
#[inline(never)]
pub(super) fn from_fields(value: &Value) -> Result<Box<Profile>> {
    let parts = sequence(value, 2)?;
    let Value::Either { left: full, value } = &parts[0] else {
        bail!("native profile either required");
    };
    let identity = sequence(value, 3)?;
    let (name, id) = if *full {
        (Some(string(&identity[1])?), Some(uuid(&identity[0])?))
    } else {
        (
            optional(&identity[0])?.map(string).transpose()?,
            optional(&identity[1])?.map(uuid).transpose()?,
        )
    };
    let Value::ProfileProperties(properties) = &identity[2] else {
        bail!("native profile properties required");
    };
    let properties = profile::groups(properties.iter().map(|p| Property {
        name: NbtString::from_text(&p.name),
        value: NbtString::from_text(&p.value),
        signature: p.signature.as_deref().map(NbtString::from_text),
    }));
    let patch = sequence(&parts[1], 4)?;
    let model = optional(&patch[3])?
        .map(|value| {
            let Value::Forward { codec: 598, value } = value else {
                bail!("native skin model forward required");
            };
            let Value::Boolean(bit) = &**value else {
                bail!("native skin model bit required");
            };
            Ok::<_, anyhow::Error>(profile::model(*bit))
        })
        .transpose()?;
    let skin = Skin {
        texture: optional(&patch[0])?.map(texture).transpose()?,
        cape: optional(&patch[1])?.map(texture).transpose()?,
        elytra: optional(&patch[2])?.map(texture).transpose()?,
        model,
    };
    Ok(Box::new(
        profile::create(*full, name, id, properties, skin).context("native profile constructor")?,
    ))
}
