//! Effective native component fields. Values keep their original encoding;
//! presence and patch application do not imply semantic equality or live ID resolution.
use super::{ItemComponent, ItemComponentPatch, registry::Registry};
use crate::MinecraftVersion;
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ComponentFields<'a> {
    fields: BTreeMap<i32, &'a ItemComponent>,
}

impl<'a> ComponentFields<'a> {
    pub(crate) fn apply<'p: 'a>(
        version: MinecraftVersion,
        prototype: impl IntoIterator<Item = &'p ItemComponent>,
        patch: Option<&'a ItemComponentPatch>,
    ) -> Result<Self> {
        let registry = Registry::for_version(version);
        let check = |field: &crate::client::registry::ItemComponentDefinition| -> Result<()> {
            if registry.item_component_definition(field.id)? != *field {
                bail!("item-component name/identity mismatch");
            }
            Ok(())
        };
        let mut fields: BTreeMap<i32, &'a ItemComponent> = BTreeMap::new();
        for field in prototype {
            check(&field.definition)?;
            if fields.insert(field.definition.id.value(), field).is_some() {
                bail!("duplicate prototype component");
            }
        }
        if let Some(patch) = patch {
            let mut seen = BTreeSet::new();
            for field in &patch.added {
                check(&field.definition)?;
                if !seen.insert(field.definition.id.value()) {
                    bail!("duplicate component patch type");
                }
                fields.insert(field.definition.id.value(), field);
            }
            for field in &patch.removed {
                check(field)?;
                if !seen.insert(field.id.value()) {
                    bail!("duplicate component patch type");
                }
                fields.remove(&field.id.value());
            }
        }
        Ok(Self { fields })
    }
    pub(crate) fn get(&self, name: &str) -> Option<&'a ItemComponent> {
        self.fields
            .values()
            .find(|field| field.definition.name == name)
            .copied()
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = &'a ItemComponent> + '_ {
        self.fields.values().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effective_fields_refuse_foreign_namespaces_names_and_overlapping_changes() {
        let modern = Registry::for_version(MinecraftVersion::Java1_21_11);
        let prototype = ItemComponent {
            definition: modern.item_component_by_native_id(1).unwrap(),
            bytes: vec![64],
        };
        let mut patch = ItemComponentPatch {
            added: vec![prototype.clone()],
            removed: vec![prototype.definition.clone()],
        };
        assert!(ComponentFields::apply(modern.version(), [&prototype], Some(&patch)).is_err());
        patch.removed.clear();
        patch.added[0].definition.name = "minecraft:damage".into();
        assert!(ComponentFields::apply(modern.version(), [&prototype], Some(&patch)).is_err());
        patch.added[0].definition.id = modern.item("minecraft:stone").unwrap().id;
        assert!(ComponentFields::apply(modern.version(), [&prototype], Some(&patch)).is_err());
        patch.added[0].definition.id = Registry::for_version(MinecraftVersion::Java1_16_1)
            .item("minecraft:stone")
            .unwrap()
            .id;
        assert!(ComponentFields::apply(modern.version(), [&prototype], Some(&patch)).is_err());
        assert!(ComponentFields::apply(modern.version(), [&prototype, &prototype], None).is_err());
    }
}
