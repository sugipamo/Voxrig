//! Native enchantment constructor fields. References remain IDs in the owning
//! registry context; this model does not resolve them or admit an inventory action.
use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Enchantments {
    pub levels: BTreeMap<i32, i32>,
}
impl Enchantments {
    pub(crate) fn from_entries(entries: BTreeMap<i32, i32>) -> Result<Self> {
        // The stream map overwrites duplicate keys before the native constructor
        // validates the effective map. An invalid superseded level is permitted.
        if entries.values().any(|level| !(0..=255).contains(level)) {
            bail!("native enchantment level must be in 0..=255");
        }
        Ok(Self { levels: entries })
    }
}
