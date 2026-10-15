//! Independent pre-step-4b static locality/usage source (4892564).
//! Original expressions from DeferredContact and the owned Spawn boundary.
//! Never derive expectations from table assignments or production facades.
use super::{frozen_capabilities, key::PacketKey};
use crate::MinecraftVersion;

pub(super) fn envelope(version: MinecraftVersion, name: &str) -> bool {
    matches!(
        (version, name),
        (
            MinecraftVersion::Java1_16_1 | MinecraftVersion::Java1_21_11,
            "minecraft:spider" | "minecraft:cave_spider"
        )
    )
}
pub(super) fn modeled(version: MinecraftVersion, name: &str) -> bool {
    frozen_capabilities::lookup(version, name).modeled()
}
pub(super) fn relevant(version: MinecraftVersion, name: &str, boat: bool) -> bool {
    use crate::client::entity::contact_capabilities::Classification;
    let capabilities = frozen_capabilities::lookup(version, name);
    capabilities.solid != Classification::ProvenIrrelevant
        || if boat {
            capabilities.incoming != Classification::ProvenIrrelevant
        } else {
            capabilities.walking != Classification::ProvenIrrelevant
        }
}
pub(super) fn revokes(packet: PacketKey) -> bool {
    // Original reducer families; lifetime/passenger membership stays dynamic.
    matches!(
        packet,
        PacketKey::Metadata
            | PacketKey::Equipment
            | PacketKey::Scale
            | PacketKey::Status(_)
            | PacketKey::Animation(_)
            | PacketKey::Passengers
    )
}
