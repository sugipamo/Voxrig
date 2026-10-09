//! Typed item/component comparison fields. No JSON, wire-byte or CRC equality.
use super::{books, fraction::Fraction, nbt, profile, registry::RegistryId, text};
use crate::MinecraftVersion;
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct LegacyItem {
    pub count: i32,
    pub native_id: Option<RegistryId>,
    pub tag: Option<Arc<nbt::NbtValue>>,
}
impl LegacyItem {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            count: 0,
            native_id: None,
            tag: None,
        }
    }
    pub(crate) fn stack(item: &super::ItemStack) -> crate::Result<Self> {
        Ok(Self {
            count: i32::try_from(item.count)
                .map_err(|e| crate::Error::new(crate::ErrorKind::InvalidInput, e))?,
            native_id: Some(item.id),
            tag: super::item::legacy_constructor_tag(item)?,
        })
    }
    pub(crate) fn matches(&self, other: &Self, shared_receipt: bool) -> bool {
        self.count == other.count && self.same_data(other, shared_receipt)
    }
    pub(crate) fn same_data(&self, other: &Self, shared_receipt: bool) -> bool {
        self.native_id == other.native_id
            && match (&self.tag, &other.tag) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    nbt::equivalent_receipt(a, b, MinecraftVersion::Java1_16_1, shared_receipt)
                }
                _ => false,
            }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub(crate) struct ModernNbt(pub Arc<nbt::NbtValue>);
impl PartialEq for ModernNbt {
    fn eq(&self, other: &Self) -> bool {
        nbt::equivalent(&self.0, &other.0, MinecraftVersion::Java1_21_11)
    }
}
impl Eq for ModernNbt {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Entry {
    pub version: u8,
    pub registry: String,
    pub owner: Option<(u64, u64)>,
    pub native_id: i32,
}
impl From<super::registry::RegistryEntryId> for Entry {
    fn from(value: super::registry::RegistryEntryId) -> Self {
        Self {
            version: match value.version() {
                MinecraftVersion::Java1_16_1 => 0,
                MinecraftVersion::Java1_21_11 => 1,
            },
            registry: value.registry().into(),
            owner: value
                .server_stamp()
                .map(|s| (s.connection_id, s.configuration_generation)),
            native_id: value.value(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) enum MapKey {
    Integer(i32),
    String(String),
    Identifier(String, String),
    Entry(Entry),
    Forward(usize, Box<MapKey>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Item {
    pub count: i32,
    pub native_id: Option<RegistryId>,
    pub components: BTreeMap<i32, Component>,
}
impl Item {
    pub(crate) fn empty() -> Self {
        Self {
            count: 0,
            native_id: None,
            components: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum Component {
    Unit,
    Boolean(bool),
    Integer(i32),
    Float(u32),
    Double(u64),
    BlockPosition(i64),
    Uuid(i64, i64),
    String(String),
    Identifier(String, String),
    Forward(usize, Box<Component>),
    Enumeration(usize, i32),
    Nbt(Option<ModernNbt>),
    Text(Box<text::FieldKey>),
    Profile(Box<profile::Profile>),
    Enchantments(BTreeMap<Entry, i32>),
    Enchantability(books::Enchantability),
    WritableBook(books::WritableBook),
    WrittenBook(Box<books::WrittenFields>),
    Sequence(Vec<Component>),
    List(Vec<Component>),
    Map(BTreeMap<MapKey, Component>),
    Optional(Option<Box<Component>>),
    Either(bool, Box<Component>),
    Registry(Entry),
    Reference(Entry),
    Inline(String, Box<Component>),
    // Original stream decoding requires a declared named set and returns that
    // lookup's cached object. Independent emptyNamed factories are not decoded tags.
    Tag {
        registry: String,
        name: String,
        owner: (u64, u64),
    },
    HolderList(String, Vec<Component>),
    ProfileProperties(BTreeMap<String, Vec<(String, Option<String>)>>),
    Typed(i32, Box<Component>),
    Item(Box<Item>),
    Bundle(Vec<Item>, Fraction),
    Patch(BTreeMap<i32, Option<Component>>),
    Dispatch(Option<bool>, i32, Box<Component>),
}
impl Component {
    pub(crate) fn uses_tags(&self) -> bool {
        match self {
            Self::Tag { .. } => true,
            Self::Forward(_, v)
            | Self::Either(_, v)
            | Self::Inline(_, v)
            | Self::Typed(_, v)
            | Self::Dispatch(_, _, v) => v.uses_tags(),
            Self::Sequence(v) | Self::List(v) | Self::HolderList(_, v) => {
                v.iter().any(Self::uses_tags)
            }
            Self::Map(v) => v.values().any(Self::uses_tags),
            Self::Optional(v) => v.as_deref().is_some_and(Self::uses_tags),
            Self::Item(v) => v.components.values().any(Self::uses_tags),
            Self::Bundle(v, _) => v.iter().any(|v| v.components.values().any(Self::uses_tags)),
            Self::Patch(v) => v.values().flatten().any(Self::uses_tags),
            Self::Text(v) => v.uses_tags(),
            Self::WrittenBook(v) => v.uses_tags(),
            Self::Unit
            | Self::Boolean(_)
            | Self::Integer(_)
            | Self::Float(_)
            | Self::Double(_)
            | Self::BlockPosition(_)
            | Self::Uuid(_, _)
            | Self::String(_)
            | Self::Identifier(_, _)
            | Self::Enumeration(_, _)
            | Self::Nbt(_)
            | Self::Profile(_)
            | Self::Enchantments(_)
            | Self::Enchantability(_)
            | Self::WritableBook(_)
            | Self::Registry(_)
            | Self::Reference(_)
            | Self::ProfileProperties(_) => false,
        }
    }
    pub(crate) fn map_key(&self) -> anyhow::Result<MapKey> {
        Ok(match self {
            Self::Integer(v) => MapKey::Integer(*v),
            Self::String(v) => MapKey::String(v.clone()),
            Self::Identifier(namespace, path) => {
                MapKey::Identifier(namespace.clone(), path.clone())
            }
            Self::Registry(v) | Self::Reference(v) => MapKey::Entry(v.clone()),
            Self::Forward(codec, v) => MapKey::Forward(*codec, Box::new(v.map_key()?)),
            _ => anyhow::bail!("unverified native component map key constructor"),
        })
    }
}

#[cfg(test)]
mod tests;
