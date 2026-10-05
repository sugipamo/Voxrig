//! Server-supplied registries and tags, scoped to one configuration of one connection.
use super::{Registry, RegistryEntryId, catalog, invalid};
use crate::client::{ObservedValue, SessionStamp, received};
use crate::{MinecraftVersion, Result};
use anyhow::{Context, bail};
use std::{collections::BTreeMap, sync::Arc};

/// Ownership of server-assigned IDs. Respawning alone does not replace registries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ServerRegistryStamp {
    /// Selected native version.
    pub version: MinecraftVersion,
    /// Process-local transport identity.
    pub connection_id: u64,
    /// Receive ordinal starting this registry configuration; zero for initial modern setup.
    pub configuration_generation: u64,
}

/// An ID validated against received entries, distinct from bundled static registry IDs.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ServerRegistryId {
    stamp: ServerRegistryStamp,
    registry: String,
    value: i32,
}
impl ServerRegistryId {
    /// Owning connection and configuration.
    pub fn stamp(&self) -> ServerRegistryStamp {
        self.stamp
    }
    /// Namespaced registry key, not an item or block-state kind.
    pub fn registry(&self) -> &str {
        &self.registry
    }
    /// Explicit native representation.
    pub fn value(&self) -> i32 {
        self.value
    }
}

/// One complete entry in a modern server-supplied registry. Position is its native ID.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ServerRegistryEntry {
    /// Namespaced key received from this server.
    pub name: String,
    /// Exact unnamed compound NBT, including the root byte. Legacy dimension entries
    /// retain their inline name field. No rendering or prototype defaults.
    pub data: Vec<u8>,
}

/// Raw tag members are native IDs in the named registry, never block-state IDs.
pub type ServerRegistryTags = BTreeMap<String, BTreeMap<String, Vec<i32>>>;

/// Immutable received registry state at one adapter capture boundary.
/// Bundled vanilla fixture IDs are never inserted into this observation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ServerRegistryObservation {
    session: SessionStamp,
    stamp: ServerRegistryStamp,
    receive_sequence: u64,
    complete: bool,
    registries: BTreeMap<String, ObservedValue<Arc<Vec<ServerRegistryEntry>>>>,
    tags: Option<ObservedValue<Arc<ServerRegistryTags>>>,
    tag_packet: Option<ObservedValue<Arc<Vec<u8>>>>,
    legacy_codec: Option<ObservedValue<Arc<Vec<u8>>>>,
}
impl ServerRegistryObservation {
    /// World at capture, separate from registry configuration lifetime.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Owner of server-assigned registry IDs.
    pub fn stamp(&self) -> ServerRegistryStamp {
        self.stamp
    }
    /// Last applied packet ordinal at capture.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Initial legacy join or modern FINISH_CONFIGURATION has been received.
    /// Does not assert item-data semantics, world readiness or action permissions.
    pub fn complete(&self) -> bool {
        self.complete
    }
    /// Received modern lists or the legacy codec's dimension list, with packet ordinals.
    pub fn registries(&self) -> &BTreeMap<String, ObservedValue<Arc<Vec<ServerRegistryEntry>>>> {
        &self.registries
    }
    /// Most recent full tag declaration, including empty declared registries/tags.
    pub fn tags(&self) -> Option<&ObservedValue<Arc<ServerRegistryTags>>> {
        self.tags.as_ref()
    }
    /// Exact complete native tag packet payload, in this version's outer format.
    /// Retained alongside parsed members for diagnostics, not a semantic item value.
    pub fn tag_packet(&self) -> Option<&ObservedValue<Arc<Vec<u8>>>> {
        self.tag_packet.as_ref()
    }
    /// Original named NBT registry codec from a legacy join. Not modern registry-data packets.
    /// Individual legacy dimension entries are also available through the common resolver.
    pub fn legacy_codec(&self) -> Option<&ObservedValue<Arc<Vec<u8>>>> {
        self.legacy_codec.as_ref()
    }
    /// Validate an explicitly native ID against this connection's received entries.
    /// Missing registries remain unavailable, including static registries sent only as tags.
    pub fn bind(&self, registry: &str, value: i32) -> Result<ServerRegistryId> {
        self.entry(registry, value)?;
        Ok(ServerRegistryId {
            stamp: self.stamp,
            registry: registry.to_owned(),
            value,
        })
    }
    /// Resolve a namespaced entry key without assuming vanilla numeric ordering.
    pub fn find(&self, registry: &str, name: &str) -> Result<ServerRegistryId> {
        if !self.complete {
            return Err(invalid("registry configuration is not complete"));
        }
        let entries = self
            .registries
            .get(registry)
            .ok_or_else(|| invalid("registry has no received entry list"))?;
        let value = entries
            .value
            .iter()
            .position(|entry| entry.name == name)
            .ok_or_else(|| invalid("entry name was not received in this registry"))?;
        self.bind(registry, value as i32)
    }
    /// Resolve only an ID belonging to this exact connection and registry configuration.
    pub fn resolve(&self, id: &ServerRegistryId) -> Result<&ServerRegistryEntry> {
        if id.stamp != self.stamp {
            return Err(invalid(
                "server registry ID belongs to another connection or configuration",
            ));
        }
        self.entry(&id.registry, id.value)
    }
    /// Bind a numeric registry entry using its builtin or actually received owner.
    /// Missing dynamic registries are never filled from vanilla defaults.
    /// This explicitly binds an ID; it does not prove provenance of arbitrary item bytes.
    pub fn bind_entry(&self, registry: &str, value: i32) -> Result<RegistryEntryId> {
        if !self.complete {
            return Err(invalid("registry configuration is not complete"));
        }
        if catalog::is_builtin(self.stamp.version, registry)? {
            self.check_builtin(registry)?;
            Registry::for_version(self.stamp.version)
                .builtin_id_by_native_id(registry, value)
                .map(RegistryEntryId::Builtin)
        } else {
            self.bind(registry, value).map(RegistryEntryId::Server)
        }
    }
    /// Find an entry by name through the same API on both selected versions.
    pub fn find_entry(&self, registry: &str, name: &str) -> Result<RegistryEntryId> {
        if !self.complete {
            return Err(invalid("registry configuration is not complete"));
        }
        if catalog::is_builtin(self.stamp.version, registry)? {
            self.check_builtin(registry)?;
            Registry::for_version(self.stamp.version)
                .builtin_id(registry, name)
                .map(RegistryEntryId::Builtin)
        } else {
            self.find(registry, name).map(RegistryEntryId::Server)
        }
    }
    /// Resolve a builtin version or the exact received connection/configuration owner.
    pub fn entry_name(&self, id: &RegistryEntryId) -> Result<&str> {
        if !self.complete || id.version() != self.stamp.version {
            return Err(invalid(
                "registry entry belongs to another version or incomplete configuration",
            ));
        }
        match id {
            RegistryEntryId::Builtin(id) => {
                self.check_builtin(id.registry())?;
                Registry::for_version(self.stamp.version).builtin_name(id)
            }
            RegistryEntryId::Server(id) => {
                if catalog::is_builtin(self.stamp.version, id.registry())? {
                    return Err(invalid(
                        "builtin registry entry cannot use a server-assigned identity",
                    ));
                }
                Ok(&self.resolve(id)?.name)
            }
        }
    }
    fn check_builtin(&self, registry: &str) -> Result<()> {
        if self.registries.contains_key(registry) {
            return Err(invalid(
                "server supplied entries for a builtin registry; update Voxrig for this protocol",
            ));
        }
        Ok(())
    }
    fn entry(&self, registry: &str, value: i32) -> Result<&ServerRegistryEntry> {
        if !self.complete {
            return Err(invalid("registry configuration is not complete"));
        }
        let index = usize::try_from(value).map_err(|_| invalid("negative server registry ID"))?;
        self.registries
            .get(registry)
            .and_then(|entries| entries.value.get(index))
            .ok_or_else(|| invalid("server registry entry was not received"))
    }
}

// Finite aggregate bounds across a configuration, not just within each packet.
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 262_144;
const MAX_REGISTRIES: usize = 256;
const MAX_TAG_VALUES: usize = 1_048_576;

#[derive(Clone, Debug, Default)]
pub(crate) struct ReceivedRegistries {
    generation: u64,
    complete: bool,
    entries: usize,
    bytes: usize,
    tag_bytes: usize,
    registries: BTreeMap<String, ObservedValue<Arc<Vec<ServerRegistryEntry>>>>,
    tags: Option<ObservedValue<Arc<ServerRegistryTags>>>,
    tag_packet: Option<ObservedValue<Arc<Vec<u8>>>>,
    legacy_codec: Option<ObservedValue<Arc<Vec<u8>>>>,
}
impl ReceivedRegistries {
    pub fn reset(&mut self, generation: u64) {
        *self = Self {
            generation,
            ..Self::default()
        };
    }
    pub fn finish(&mut self) {
        self.complete = true;
    }
    pub fn modern_registry(
        &mut self,
        name: String,
        entries: Vec<ServerRegistryEntry>,
        sequence: u64,
        bytes: usize,
    ) -> anyhow::Result<()> {
        identifier(&name)?;
        if self.complete || self.registries.contains_key(&name) {
            bail!("duplicate registry or registry received after configuration finished");
        }
        if self.registries.len() >= MAX_REGISTRIES
            || self.entries.saturating_add(entries.len()) > MAX_ENTRIES
            || self
                .bytes
                .saturating_add(self.tag_bytes)
                .saturating_add(bytes)
                > MAX_BYTES
        {
            bail!("received registry aggregate limit exceeded");
        }
        let mut names = std::collections::BTreeSet::new();
        for entry in &entries {
            identifier(&entry.name)?;
            if !names.insert(&entry.name) {
                bail!("duplicate registry entry name");
            }
        }
        self.entries += entries.len();
        self.bytes += bytes;
        self.registries
            .insert(name, received(Arc::new(entries), sequence));
        Ok(())
    }
    pub fn legacy_join(&mut self, codec: Vec<u8>, sequence: u64) -> anyhow::Result<()> {
        if codec.len() > MAX_BYTES {
            bail!("legacy registry codec limit exceeded");
        }
        let entries = crate::client::nbt::legacy_dimension_entries(&codec)?;
        // Parse/validate into a new state so a malformed join cannot destroy the
        // preceding immutable configuration or leave partially applied entries.
        let mut replacement = Self::default();
        replacement.reset(sequence);
        if let Some(entries) = entries {
            replacement.modern_registry(
                "minecraft:dimension_type".into(),
                entries
                    .into_iter()
                    .map(|entry| ServerRegistryEntry {
                        name: entry.name,
                        data: entry.data,
                    })
                    .collect(),
                sequence,
                codec.len(),
            )?;
        } else {
            replacement.bytes = codec.len();
        }
        replacement.legacy_codec = Some(received(Arc::new(codec), sequence));
        replacement.finish();
        *self = replacement;
        Ok(())
    }
    pub fn receive_tags(
        &mut self,
        payload: &[u8],
        sequence: u64,
        version: MinecraftVersion,
    ) -> anyhow::Result<()> {
        if self.bytes.saturating_add(payload.len()) > MAX_BYTES {
            bail!("received registry/tag aggregate limit exceeded");
        }
        let tags = parse_tags(payload, version)?;
        self.tag_bytes = payload.len();
        self.tags = Some(received(Arc::new(tags), sequence));
        self.tag_packet = Some(received(Arc::new(payload.to_vec()), sequence));
        Ok(())
    }
    pub fn capture(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> ServerRegistryObservation {
        ServerRegistryObservation {
            session,
            stamp: ServerRegistryStamp {
                version: session.version,
                connection_id: session.connection_id,
                configuration_generation: self.generation,
            },
            receive_sequence,
            complete: self.complete,
            registries: self.registries.clone(),
            tags: self.tags.clone(),
            tag_packet: self.tag_packet.clone(),
            legacy_codec: self.legacy_codec.clone(),
        }
    }
}

// The two versions have different outer tag-list formats, sharing VarInt/string primitives.
fn parse_tags(payload: &[u8], version: MinecraftVersion) -> anyhow::Result<ServerRegistryTags> {
    let mut rest = payload;
    let modern = version == MinecraftVersion::Java1_21_11;
    let count = if modern {
        count(&mut rest, MAX_REGISTRIES)?
    } else {
        4
    };
    let legacy_keys = [
        "minecraft:block",
        "minecraft:item",
        "minecraft:fluid",
        "minecraft:entity_type",
    ];
    let mut tags = BTreeMap::new();
    let (mut tag_count, mut values) = (0usize, 0usize);
    for index in 0..count {
        let key = if modern {
            crate::protocol::get_string(&mut rest)?
        } else {
            legacy_keys
                .get(index)
                .context("unknown legacy tag registry")?
                .to_string()
        };
        identifier(&key)?;
        let mut group = BTreeMap::new();
        let n = self::count(&mut rest, 65_536)?;
        tag_count = tag_count.saturating_add(n);
        if tag_count > 65_536 {
            bail!("aggregate registry tag count limit exceeded");
        }
        for _ in 0..n {
            let name = crate::protocol::get_string(&mut rest)?;
            identifier(&name)?;
            let n = self::count(&mut rest, 65_536)?;
            values = values.saturating_add(n);
            if values > MAX_TAG_VALUES {
                bail!("aggregate tag member count limit exceeded");
            }
            let mut members = Vec::with_capacity(n);
            for _ in 0..n {
                let value = crate::protocol::get_varint(&mut rest)?;
                if value < 0 {
                    bail!("negative registry tag member ID");
                }
                members.push(value);
            }
            if group.insert(name, members).is_some() {
                bail!("duplicate registry tag key");
            }
        }
        if tags.insert(key, group).is_some() {
            bail!("duplicate registry tag group");
        }
    }
    if !rest.is_empty() {
        bail!("trailing registry tag packet bytes");
    }
    Ok(tags)
}
fn count(rest: &mut &[u8], maximum: usize) -> anyhow::Result<usize> {
    let count =
        usize::try_from(crate::protocol::get_varint(rest)?).context("negative registry count")?;
    if count > maximum {
        bail!("registry count limit exceeded");
    }
    Ok(count)
}
pub(crate) fn identifier(name: &str) -> anyhow::Result<()> {
    let Some((namespace, path)) = name.split_once(':') else {
        bail!("namespaced registry key required");
    };
    if namespace.is_empty()
        || path.is_empty()
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
        || !path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-/".contains(&c))
    {
        bail!("invalid namespaced registry key");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ValueSource;
    use crate::protocol::{put_string, put_varint};
    fn session(connection_id: u64, world_generation: u64) -> SessionStamp {
        SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id,
            world_generation,
        }
    }
    fn entries() -> Vec<ServerRegistryEntry> {
        ["example:second", "example:first"]
            .into_iter()
            .map(|name| ServerRegistryEntry {
                name: name.into(),
                data: vec![10, 0],
            })
            .collect()
    }
    #[test]
    fn received_registry_ids_resolve_actual_order_and_cannot_cross_owners() {
        let mut state = ReceivedRegistries::default();
        state
            .modern_registry("example:test".into(), entries(), 4, 40)
            .unwrap();
        let pending = state.capture(session(11, 0), 4);
        assert!(!pending.complete());
        assert!(pending.bind("example:test", 0).is_err());
        state.finish();
        let captured = state.capture(session(11, 9), 10);
        let first = captured.find("example:test", "example:first").unwrap();
        assert_eq!(first.value(), 1);
        assert_eq!(first.registry(), "example:test");
        assert_eq!(captured.resolve(&first).unwrap().name, "example:first");
        assert_eq!(
            captured.registries()["example:test"].source,
            ValueSource::Received { sequence: 4 }
        );
        assert!(captured.bind("example:test", -1).is_err());
        assert!(captured.bind("example:test", 2).is_err());
        assert!(captured.bind("minecraft:enchantment", 0).is_err());
        assert!(captured.find("example:test", "example:missing").is_err());
        assert!(state.capture(session(12, 9), 10).resolve(&first).is_err());
        let mut other_version = session(11, 9);
        other_version.version = MinecraftVersion::Java1_16_1;
        assert!(state.capture(other_version, 10).resolve(&first).is_err());
        assert!(state.capture(session(11, 20), 21).resolve(&first).is_ok());
        state.reset(30);
        state
            .modern_registry(
                "example:test".into(),
                entries().into_iter().rev().collect(),
                31,
                40,
            )
            .unwrap();
        state.finish();
        let replacement = state.capture(session(11, 33), 34);
        assert!(replacement.resolve(&first).is_err());
        assert_eq!(
            replacement
                .find("example:test", "example:first")
                .unwrap()
                .value(),
            0
        );
        assert_eq!(captured.resolve(&first).unwrap().name, "example:first");
        assert_eq!(replacement.stamp().configuration_generation, 30);
    }
    fn tags(version: MinecraftVersion, value: i32) -> Vec<u8> {
        let mut payload = Vec::new();
        if version == MinecraftVersion::Java1_21_11 {
            put_varint(&mut payload, 1);
            put_string(&mut payload, "minecraft:item");
        }
        payload.push(1);
        put_string(&mut payload, "example:empty_or_members");
        payload.push(1);
        put_varint(&mut payload, value);
        if version == MinecraftVersion::Java1_16_1 {
            payload.extend([0, 0, 0]);
        }
        payload
    }
    #[test]
    fn received_tags_are_atomic_bounded_and_never_promote_members_into_static_ids() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut state = ReceivedRegistries::default();
            let payload = tags(version, 3);
            state.receive_tags(&payload, 7, version).unwrap();
            let before = state.tags.clone().unwrap();
            let key = if version == MinecraftVersion::Java1_16_1 {
                "minecraft:block"
            } else {
                "minecraft:item"
            };
            assert_eq!(before.value[key]["example:empty_or_members"], [3]);
            for prefix in 0..payload.len() {
                assert!(state.receive_tags(&payload[..prefix], 8, version).is_err());
                assert_eq!(state.tags.as_ref().unwrap(), &before);
            }
            let mut trailing = payload.clone();
            trailing.push(0);
            assert!(state.receive_tags(&trailing, 8, version).is_err());
            assert!(state.receive_tags(&tags(version, -1), 8, version).is_err());
            assert_eq!(state.tags.as_ref().unwrap(), &before);
            let empty = if version == MinecraftVersion::Java1_16_1 {
                vec![0; 4]
            } else {
                vec![0]
            };
            state.receive_tags(&empty, 9, version).unwrap();
            assert!(
                state
                    .tags
                    .as_ref()
                    .unwrap()
                    .value
                    .values()
                    .all(BTreeMap::is_empty)
            );
            assert_eq!(
                state.tags.as_ref().unwrap().source,
                ValueSource::Received { sequence: 9 }
            );
            state.reset(10);
            assert!(state.tags.is_none());
        }
    }
    #[test]
    fn registry_duplicate_and_aggregate_limits_leave_previous_receipts_unchanged() {
        let mut state = ReceivedRegistries::default();
        state
            .modern_registry("example:test".into(), entries(), 1, 40)
            .unwrap();
        assert!(
            state
                .modern_registry("example:test".into(), entries(), 2, 40)
                .is_err()
        );
        let mut duplicates = entries();
        duplicates.push(duplicates[0].clone());
        assert!(
            state
                .modern_registry("example:other".into(), duplicates, 2, 40)
                .is_err()
        );
        assert!(
            state
                .modern_registry("invalid".into(), entries(), 2, 40)
                .is_err()
        );
        state.bytes = MAX_BYTES;
        assert!(
            state
                .modern_registry("example:other".into(), entries(), 2, 1)
                .is_err()
        );
        assert!(
            state
                .receive_tags(&[0], 2, MinecraftVersion::Java1_21_11)
                .is_err()
        );
        state.bytes = 40;
        state.entries = MAX_ENTRIES;
        assert!(
            state
                .modern_registry("example:other".into(), entries(), 2, 40)
                .is_err()
        );
        assert_eq!(state.registries.len(), 1);
        assert_eq!(
            state.registries["example:test"].source,
            ValueSource::Received { sequence: 1 }
        );
        let mut duplicate_groups = vec![2];
        for _ in 0..2 {
            put_string(&mut duplicate_groups, "minecraft:item");
            duplicate_groups.push(0);
        }
        assert!(parse_tags(&duplicate_groups, MinecraftVersion::Java1_21_11).is_err());
        let mut duplicate_tags = vec![1];
        put_string(&mut duplicate_tags, "minecraft:item");
        duplicate_tags.push(2);
        for _ in 0..2 {
            put_string(&mut duplicate_tags, "example:tag");
            duplicate_tags.push(0);
        }
        assert!(parse_tags(&duplicate_tags, MinecraftVersion::Java1_21_11).is_err());
        for bad in [
            "minecraft:",
            ":item",
            "minecraft:UPPER",
            "minecraft:a:b",
            "item",
        ] {
            assert!(identifier(bad).is_err());
        }
    }
}
