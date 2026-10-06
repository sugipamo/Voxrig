//! Names and native IDs bound to the adapter selected at setup.
use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Result};
mod catalog;
pub(crate) mod received;
pub use catalog::{BuiltinRegistryId, RegistryEntryId};
pub use received::{
    ServerRegistryEntry, ServerRegistryId, ServerRegistryObservation, ServerRegistryStamp,
    ServerRegistryTags,
};

/// Registry namespace of a numeric identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RegistryKind {
    /// Complete block state, including all properties.
    BlockState,
    /// Item definition, independent of block state IDs.
    Item,
    /// Item data component type, distinct from item and block-state IDs.
    ItemComponent,
}
crate::diagnostic_projection::diagnostic_record! {
/// A validated ID whose version and namespace cannot be discarded accidentally.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RegistryId => RecordedRegistryId {
    version: MinecraftVersion,
    kind: RegistryKind,
    value: i32,
}
    diagnostic_serde {}
}

impl RegistryId {
    /// Owning wire/registry version.
    pub fn version(self) -> MinecraftVersion {
        self.version
    }
    /// Owning registry namespace.
    pub fn kind(self) -> RegistryKind {
        self.kind
    }
    /// Numeric representation for diagnostics or explicit native integrations.
    pub fn value(self) -> i32 {
        self.value
    }
}
/// Item facts from the exact bundled registry; no unknown-item defaults.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ItemDefinition {
    /// Version-bound native identifier.
    pub id: RegistryId,
    /// Namespaced item name.
    pub name: String,
    /// Native maximum default stack count.
    pub max_stack_size: u32,
}
crate::diagnostic_projection::diagnostic_record! {
/// A native item-component type bound to its exact adapter registry.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ItemComponentDefinition => RecordedItemComponentDefinition {
    /// Version and component namespace are retained with the native ID.
    pub id: RegistryId,
    /// Namespaced component name.
    pub name: String,
}
    diagnostic_serde {}
}

/// Read-only access to one explicitly selected registry, also usable before connecting.
#[derive(Clone, Copy, Debug)]
pub struct Registry {
    version: MinecraftVersion,
}
impl Registry {
    /// Select an implemented adapter; adding versions requires updating Voxrig.
    pub const fn for_version(version: MinecraftVersion) -> Self {
        Self { version }
    }
    /// Exact owning version.
    pub const fn version(self) -> MinecraftVersion {
        self.version
    }
    /// Validate a native block-state ID and bind it to this registry.
    pub fn block_state_id(self, value: i32) -> Result<RegistryId> {
        self.decode_state(value)?;
        Ok(RegistryId {
            version: self.version,
            kind: RegistryKind::BlockState,
            value,
        })
    }
    /// Decode a validated block-state identity. Cross-version and item IDs are refused.
    pub fn block_state(self, id: RegistryId) -> Result<NativeBlockState> {
        self.check(id, RegistryKind::BlockState)?;
        self.decode_state(id.value)
    }
    /// Resolve a name and complete properties; no missing-property guesses.
    pub fn resolve_block_state(self, state: &NativeBlockState) -> Result<RegistryId> {
        let value = match self.version {
            MinecraftVersion::Java1_16_1 => crate::versions::java_1_16_1::state_id(state)?,
            MinecraftVersion::Java1_21_11 => crate::versions::java_1_21_11::state_id(state)?,
        };
        Ok(RegistryId {
            version: self.version,
            kind: RegistryKind::BlockState,
            value,
        })
    }
    /// Resolve a namespaced item definition.
    pub fn item(self, name: &str) -> Result<ItemDefinition> {
        let name = name
            .strip_prefix("minecraft:")
            .ok_or_else(|| invalid("namespaced item name required"))?;
        match self.version {
            MinecraftVersion::Java1_16_1 => {
                let value = crate::versions::java_1_16_1::item_id(name)
                    .ok_or_else(|| invalid("unknown item; update Voxrig for new definitions"))?;
                self.item_by_native_id(value)
            }
            MinecraftVersion::Java1_21_11 => {
                let item = crate::versions::java_1_21_11::operations::default_item(name, 1)?;
                self.item_by_native_id(item.item_id)
            }
        }
    }
    /// Decode an item identity, refusing identifiers from another version or registry.
    pub fn item_definition(self, id: RegistryId) -> Result<ItemDefinition> {
        self.check(id, RegistryKind::Item)?;
        self.item_by_native_id(id.value)
    }
    /// Validate and interpret an explicitly native item ID in this version only.
    pub fn item_by_native_id(self, value: i32) -> Result<ItemDefinition> {
        let (name, max_stack_size) = match self.version {
            MinecraftVersion::Java1_16_1 => {
                let name = crate::versions::java_1_16_1::item_name(value)
                    .ok_or_else(|| invalid("unknown native item ID"))?;
                let size = crate::versions::java_1_16_1::registry::item_max_stack_size(value)
                    .ok_or_else(|| invalid("invalid native item capacity"))?;
                (format!("minecraft:{name}"), u32::from(size))
            }
            MinecraftVersion::Java1_21_11 => {
                crate::versions::java_1_21_11::operations::item_definition(value)?
            }
        };
        Ok(ItemDefinition {
            id: RegistryId {
                version: self.version,
                kind: RegistryKind::Item,
                value,
            },
            name,
            max_stack_size,
        })
    }
    /// Resolve an item-component type. Legacy NBT does not have this registry.
    pub fn item_component(self, name: &str) -> Result<ItemComponentDefinition> {
        let definitions = self.component_definitions()?;
        let definition = definitions
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| invalid("unknown item component; update Voxrig for new definitions"))?;
        self.item_component_by_native_id(definition.native_id)
    }
    /// Interpret an explicitly native component ID in this version only.
    pub fn item_component_by_native_id(self, value: i32) -> Result<ItemComponentDefinition> {
        let definitions = self.component_definitions()?;
        let definition = definitions
            .iter()
            .find(|d| d.native_id == value)
            .ok_or_else(|| invalid("unknown native item-component ID"))?;
        Ok(ItemComponentDefinition {
            id: RegistryId {
                version: self.version,
                kind: RegistryKind::ItemComponent,
                value,
            },
            name: definition.name.clone(),
        })
    }
    /// Decode a component identity, rejecting item/block-state and cross-version IDs.
    pub fn item_component_definition(self, id: RegistryId) -> Result<ItemComponentDefinition> {
        self.check(id, RegistryKind::ItemComponent)?;
        self.item_component_by_native_id(id.value)
    }
    fn component_definitions(
        self,
    ) -> Result<&'static [crate::versions::java_1_21_11::item_components::Definition]> {
        match self.version {
            MinecraftVersion::Java1_21_11 => {
                Ok(crate::versions::java_1_21_11::item_components::definitions())
            }
            MinecraftVersion::Java1_16_1 => Err(Error::new(
                ErrorKind::Unsupported,
                anyhow::anyhow!("Java 1.16.1 uses legacy NBT, not an item-component registry"),
            )),
        }
    }
    fn check(self, id: RegistryId, kind: RegistryKind) -> Result<()> {
        if id.version != self.version || id.kind != kind {
            return Err(invalid("ID belongs to another version or registry"));
        }
        Ok(())
    }
    fn decode_state(self, value: i32) -> Result<NativeBlockState> {
        match self.version {
            MinecraftVersion::Java1_16_1 => crate::versions::java_1_16_1::native_state(value),
            MinecraftVersion::Java1_21_11 => crate::versions::java_1_21_11::native_state(value),
        }
    }
}
pub(crate) fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_identities_remain_version_and_namespace_bound() {
        let modern = Registry::for_version(MinecraftVersion::Java1_21_11);
        let legacy = Registry::for_version(MinecraftVersion::Java1_16_1);
        for native in crate::versions::java_1_21_11::item_components::definitions() {
            let component = modern.item_component(&native.name).unwrap();
            assert_eq!(component.id.value(), native.native_id);
            assert_eq!(component.id.kind(), RegistryKind::ItemComponent);
            assert_eq!(
                modern.item_component_definition(component.id).unwrap(),
                component
            );
            assert!(modern.item_definition(component.id).is_err());
            assert!(modern.block_state(component.id).is_err());
            assert!(legacy.item_component_definition(component.id).is_err());
        }
        assert!(modern.item_component("custom_name").is_err());
        assert!(modern.item_component("minecraft:future_component").is_err());
        assert!(modern.item_component_by_native_id(104).is_err());
        assert!(
            modern
                .item_component_definition(modern.item("minecraft:stone").unwrap().id)
                .is_err()
        );
        assert_eq!(
            legacy
                .item_component("minecraft:custom_name")
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
    }
    #[test]
    fn identities_cannot_cross_registry_or_version_and_unknowns_fail() {
        let old = Registry::for_version(MinecraftVersion::Java1_16_1);
        let new = Registry::for_version(MinecraftVersion::Java1_21_11);
        let stone = old.item("minecraft:stone").unwrap();
        assert!(new.item_definition(stone.id).is_err());
        assert!(old.block_state(stone.id).is_err());
        assert!(old.item("minecraft:future_block").is_err());
        assert!(new.block_state_id(i32::MAX).is_err());
        for registry in [old, new] {
            let state = NativeBlockState {
                name: "minecraft:stone".into(),
                properties: Default::default(),
            };
            let id = registry.resolve_block_state(&state).unwrap();
            assert_eq!(registry.block_state(id).unwrap(), state);
            assert_eq!(registry.item("minecraft:stone").unwrap().max_stack_size, 64);
        }
    }
}

crate::diagnostic_projection::identity!(RegistryKind);
