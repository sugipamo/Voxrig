//! Bundled entry identities. Dynamic vanilla vectors are excluded from runtime facts.
use super::{Registry, ServerRegistryId, ServerRegistryStamp, invalid};
use crate::{MinecraftVersion, Result};
use std::collections::BTreeMap;

/// An entry in a builtin registry, bound to the selected version and full registry key.
/// A block entry is distinct from a block state. Server registries use a different owner.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct BuiltinRegistryId {
    version: MinecraftVersion,
    registry: String,
    value: i32,
}
impl BuiltinRegistryId {
    /// Selected version.
    pub fn version(&self) -> MinecraftVersion {
        self.version
    }
    /// Full namespaced registry key.
    pub fn registry(&self) -> &str {
        &self.registry
    }
    /// Explicit native representation within this registry.
    pub fn value(&self) -> i32 {
        self.value
    }
}

/// An entry identity with its actual builtin or received configuration owner.
/// This is not whole component equality or proof that arbitrary item bytes came from this owner.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "owner", content = "id", rename_all = "snake_case")]
pub enum RegistryEntryId {
    /// Library version's builtin root entry.
    Builtin(BuiltinRegistryId),
    /// Entry actually supplied by this connection's configuration.
    Server(ServerRegistryId),
}
impl RegistryEntryId {
    /// Selected version, regardless of entry ownership.
    pub fn version(&self) -> MinecraftVersion {
        match self {
            Self::Builtin(id) => id.version(),
            Self::Server(id) => id.stamp().version,
        }
    }
    /// Full namespaced registry key.
    pub fn registry(&self) -> &str {
        match self {
            Self::Builtin(id) => id.registry(),
            Self::Server(id) => id.registry(),
        }
    }
    /// Explicit native representation within this registry and owner.
    pub fn value(&self) -> i32 {
        match self {
            Self::Builtin(id) => id.value(),
            Self::Server(id) => id.value(),
        }
    }
    /// Connection/configuration ownership for server-assigned entries.
    pub fn server_stamp(&self) -> Option<ServerRegistryStamp> {
        match self {
            Self::Builtin(_) => None,
            Self::Server(id) => Some(id.stamp()),
        }
    }
}

#[derive(serde::Deserialize)]
struct Facts {
    registries: Vec<NativeRegistry>,
}
#[derive(serde::Deserialize)]
struct NativeRegistry {
    name: String,
    builtin: bool,
    entries: Vec<Entry>,
}
#[derive(serde::Deserialize)]
struct Entry {
    name: String,
    id: i32,
}
struct Catalog {
    registries: BTreeMap<String, NativeRegistry>,
}
fn catalog(version: MinecraftVersion) -> &'static Catalog {
    static CATALOG: crate::versions::table::PerVersion<Catalog> =
        crate::versions::table::PerVersion::new();
    CATALOG.get(version, |table| {
        let source: Facts = serde_json::from_str(table.data.registry_catalog)
            .expect("pinned original registry catalog");
        let mut registries = BTreeMap::new();
        for registry in source.registries {
            assert!(
                registry.builtin || registry.entries.is_empty(),
                "dynamic oracle entries must not enter runtime catalog"
            );
            let mut ids = std::collections::BTreeSet::new();
            let mut names = std::collections::BTreeSet::new();
            for entry in &registry.entries {
                assert!(
                    entry.id >= 0 && ids.insert(entry.id) && names.insert(entry.name.clone()),
                    "invalid pinned registry identity"
                );
            }
            assert!(
                registries.insert(registry.name.clone(), registry).is_none(),
                "duplicate pinned registry"
            );
        }
        Catalog { registries }
    })
}
pub(super) fn is_builtin(version: MinecraftVersion, registry: &str) -> Result<bool> {
    catalog(version)
        .registries
        .get(registry)
        .map(|entry| entry.builtin)
        .ok_or_else(|| invalid("unknown registry; update Voxrig for new definitions"))
}
impl Registry {
    /// Bind an explicit native ID only in this version's builtin registry root.
    /// Dynamic defaults are never a substitute for missing server entries.
    pub fn builtin_id_by_native_id(self, registry: &str, value: i32) -> Result<BuiltinRegistryId> {
        let native = catalog(self.version())
            .registries
            .get(registry)
            .filter(|registry| registry.builtin)
            .ok_or_else(|| invalid("registry is not a known builtin registry"))?;
        if !native.entries.iter().any(|entry| entry.id == value) {
            return Err(invalid("unknown builtin registry entry; update Voxrig"));
        }
        Ok(BuiltinRegistryId {
            version: self.version(),
            registry: registry.into(),
            value,
        })
    }
    /// Find a builtin entry by its full namespaced name using the selected version.
    pub fn builtin_id(self, registry: &str, name: &str) -> Result<BuiltinRegistryId> {
        let value = catalog(self.version())
            .registries
            .get(registry)
            .filter(|registry| registry.builtin)
            .and_then(|registry| registry.entries.iter().find(|entry| entry.name == name))
            .ok_or_else(|| invalid("unknown builtin registry/name; update Voxrig"))?
            .id;
        self.builtin_id_by_native_id(registry, value)
    }
    /// Resolve an entry bound to this exact builtin version and registry namespace.
    pub fn builtin_name(self, id: &BuiltinRegistryId) -> Result<&'static str> {
        if id.version != self.version() {
            return Err(invalid("builtin registry ID belongs to another version"));
        }
        catalog(self.version())
            .registries
            .get(id.registry())
            .filter(|registry| registry.builtin)
            .and_then(|registry| registry.entries.iter().find(|entry| entry.id == id.value()))
            .map(|entry| entry.name.as_str())
            .ok_or_else(|| invalid("invalid builtin registry ID"))
    }
}

#[cfg(test)]
mod tests;
