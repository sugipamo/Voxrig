//! Constructor fields shared independently of their version's wire composition.
//! Text references still require their owning context. These fields do not attest
//! legacy book encoding, cached server hashes or inventory action admission.
use super::{
    nbt::NbtString,
    text::{Dependency, FieldKey, Text},
};
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Filtered<T> {
    pub raw: T,
    pub filtered: Option<T>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Enchantability {
    pub value: i32,
}
impl Enchantability {
    pub(crate) fn new(value: i32) -> Result<Self> {
        if value <= 0 {
            bail!("native enchantability must be positive");
        }
        Ok(Self { value })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct WritableBook {
    pub pages: Vec<Filtered<NbtString>>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WrittenBook {
    pub title: Filtered<NbtString>,
    pub author: NbtString,
    pub generation: i32,
    pub pages: Vec<Filtered<Box<Text>>>,
    pub resolved: bool,
}
pub(crate) fn validate_generation(generation: i32) -> Result<()> {
    if !(0..=3).contains(&generation) {
        bail!("native written book generation must be in 0..=3");
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct WrittenFields {
    title: Filtered<NbtString>,
    author: NbtString,
    generation: i32,
    pages: Vec<Filtered<FieldKey>>,
    resolved: bool,
}
impl WrittenBook {
    pub(crate) fn dependencies(&self) -> Vec<Dependency> {
        self.pages
            .iter()
            .flat_map(|page| std::iter::once(&page.raw).chain(page.filtered.iter()))
            .flat_map(|text| text.dependencies())
            .collect()
    }

    // A represented-field comparison only. Unresolved nested items/dialogs
    // cannot become a comparable whole book simply by wrapping them in a page.
    pub(crate) fn field_comparison(&self) -> Option<WrittenFields> {
        Some(WrittenFields {
            title: self.title.clone(),
            author: self.author.clone(),
            generation: self.generation,
            pages: self
                .pages
                .iter()
                .map(|page| {
                    Some(Filtered {
                        raw: page.raw.modern_field_key()?,
                        filtered: match &page.filtered {
                            Some(text) => Some(text.modern_field_key()?),
                            None => None,
                        },
                    })
                })
                .collect::<Option<_>>()?,
            resolved: self.resolved,
        })
    }
}

impl WrittenFields {
    pub(crate) fn uses_tags(&self) -> bool {
        self.pages.iter().any(|page| {
            page.raw.uses_tags() || page.filtered.as_ref().is_some_and(FieldKey::uses_tags)
        })
    }
}
