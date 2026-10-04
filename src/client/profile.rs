//! Shared internal profile constructor fields. Modern equality keeps factory
//! kind, ordered values per property key and skin patch. No online resolution,
//! persistent encoder, item permission or legacy equality is implied.
use super::{
    nbt::{NbtCompound, NbtString, NbtValue},
    text::Identifier,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Profile {
    pub kind: Kind,
    pub skin: Skin,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Kind {
    DynamicName {
        name: NbtString,
    },
    DynamicId {
        id: [i32; 4],
    },
    Full {
        name: NbtString,
        id: [i32; 4],
        properties: Vec<Group>,
    },
    Partial {
        name: Option<NbtString>,
        id: Option<[i32; 4]>,
        properties: Vec<Group>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Group {
    pub key: NbtString,
    pub values: Vec<Property>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Property {
    pub name: NbtString,
    pub value: NbtString,
    pub signature: Option<NbtString>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Skin {
    pub texture: Option<Texture>,
    pub cape: Option<Texture>,
    pub elytra: Option<Texture>,
    pub model: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Texture {
    pub id: Identifier,
    pub path: Identifier,
}
impl Texture {
    pub(crate) fn new(id: Identifier) -> Self {
        let path = Identifier {
            namespace: id.namespace.clone(),
            path: format!("textures/{}.png", id.path),
        };
        Self { id, path }
    }
}
#[derive(Deserialize)]
struct Rules {
    name_utf16: Vec<u16>,
    empty_name: bool,
    models: BTreeMap<String, String>,
}
fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../data/client_api/profile_rules-1.21.11.json"
        ))
        .expect("pinned original profile rules")
    })
}
pub(crate) fn model(bit: bool) -> String {
    rules().models[if bit { "true" } else { "false" }].clone()
}
pub(crate) fn groups(properties: impl IntoIterator<Item = Property>) -> Vec<Group> {
    let mut map: BTreeMap<NbtString, Vec<Property>> = BTreeMap::new();
    for property in properties {
        map.entry(property.name.clone()).or_default().push(property);
    }
    map.into_iter()
        .map(|(key, values)| Group { key, values })
        .collect()
}
pub(crate) fn create(
    full: bool,
    name: Option<NbtString>,
    id: Option<[i32; 4]>,
    properties: Vec<Group>,
    skin: Skin,
) -> Result<Profile> {
    let kind = if full {
        Kind::Full {
            name: name.context("native full profile name required")?,
            id: id.context("native full profile ID required")?,
            properties,
        }
    } else if !properties.is_empty() || name.is_some() == id.is_some() {
        Kind::Partial {
            name,
            id,
            properties,
        }
    } else if let Some(name) = name {
        Kind::DynamicName { name }
    } else {
        Kind::DynamicId {
            id: id.context("native dynamic profile ID required")?,
        }
    };
    Ok(Profile { kind, skin })
}
fn valid_name(name: &NbtString) -> Result<()> {
    if name.utf16().len() > 16
        || (name.utf16().is_empty() && !rules().empty_name)
        || !name
            .utf16()
            .iter()
            .all(|unit| rules().name_utf16.binary_search(unit).is_ok())
    {
        bail!("invalid original NBT profile name");
    }
    Ok(())
}
fn string(value: &NbtValue, maximum: usize) -> Result<&NbtString> {
    let value = value
        .as_string()
        .context("native profile string required")?;
    if value.utf16().len() > maximum {
        bail!("native profile string too long");
    }
    Ok(value)
}
pub(crate) fn from_nbt(value: &NbtValue) -> Result<Profile> {
    if let Some(name) = value.as_string() {
        valid_name(name)?;
        return create(false, Some(name.clone()), None, Vec::new(), Skin::default());
    }
    let fields = value
        .as_compound()
        .context("native profile must be string or compound")?;
    let name = fields
        .get("name")
        .map(|value| {
            let name = string(value, 16)?;
            valid_name(name)?;
            Ok::<_, anyhow::Error>(name.clone())
        })
        .transpose()?;
    let id = fields.get("id").map(uuid).transpose()?;
    let properties = fields
        .get("properties")
        .map(properties)
        .transpose()?
        .unwrap_or_default();
    let skin = Skin {
        texture: texture(fields, "texture")?,
        cape: texture(fields, "cape")?,
        elytra: texture(fields, "elytra")?,
        model: fields
            .get("model")
            .map(|value| {
                let name = string(value, usize::MAX)?
                    .text()
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                if !rules().models.values().any(|known| *known == name) {
                    bail!("unknown native profile model; update Voxrig");
                }
                Ok::<_, anyhow::Error>(name)
            })
            .transpose()?,
    };
    create(name.is_some() && id.is_some(), name, id, properties, skin)
}
fn texture(fields: &NbtCompound, name: &str) -> Result<Option<Texture>> {
    fields
        .get(name)
        .map(|value| {
            let value = string(value, usize::MAX)?
                .text()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            Ok(Texture::new(Identifier::parse(&value)?))
        })
        .transpose()
}
fn properties(value: &NbtValue) -> Result<Vec<Group>> {
    let mut entries = Vec::new();
    if let Some(map) = value.as_compound() {
        if map.entries().len() > 16 {
            bail!("native compact property map exceeds 16 keys");
        }
        for entry in map.entries() {
            for value in entry
                .value()
                .as_list()
                .context("native compact property list required")?
            {
                entries.push(Property {
                    name: entry.key().clone(),
                    value: string(value, usize::MAX)?.clone(),
                    signature: None,
                });
            }
        }
    } else {
        let values = value
            .as_list()
            .context("native property list or map required")?;
        if values.len() > 16 {
            bail!("native property list exceeds 16 entries");
        }
        for value in values {
            let fields = value
                .as_compound()
                .context("native property record required")?;
            entries.push(Property {
                name: string(
                    fields
                        .get("name")
                        .context("native property name required")?,
                    64,
                )?
                .clone(),
                value: string(
                    fields
                        .get("value")
                        .context("native property value required")?,
                    32767,
                )?
                .clone(),
                signature: fields
                    .get("signature")
                    .map(|value| string(value, 1024).cloned())
                    .transpose()?,
            });
        }
    }
    Ok(groups(entries))
}
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
fn uuid(value: &NbtValue) -> Result<[i32; 4]> {
    let words: Vec<_> = match value {
        NbtValue::IntArray(values) => values.clone(),
        NbtValue::ByteArray(values) => values.iter().map(|n| i32::from(*n)).collect(),
        NbtValue::LongArray(values) => values.iter().map(|n| *n as i32).collect(),
        NbtValue::List(values) => values
            .iter()
            .map(|value| integer(value))
            .collect::<Result<_>>()?,
        _ => bail!("native profile UUID requires numeric array/list"),
    };
    words
        .try_into()
        .map_err(|_| anyhow::anyhow!("native UUID requires 4 words"))
}
