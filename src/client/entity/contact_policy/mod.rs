//! Private contact policy. Capabilities and harmless packets read the const table.
//!
//! The dense index includes absent version/entity pairs; presence is a separate
//! axis constraint. Baseline rows for those pairs are explicitly `absent`.
//! `Effect` names the seven existing capability fields, not the four evidence
//! classifications. Owner lanes remain separate from effects.
//!
//! Review snapshot changes before updating deliberately with:
//! `VOXRIG_UPDATE_CONTACT_BASELINE=1 cargo test --locked -j1 frozen_contact_policy_baseline`.
//! The test enumerates the full present product; run-length encoding only
//! compresses identical consecutive packet outcomes in the checked-in text.
// Locality/semantics declarations remain staged until their migration steps.
#![allow(dead_code)]
pub(super) mod key;

#[cfg(test)]
mod baseline;

mod table;
mod validate;

#[cfg(test)]
use super::contact_capabilities::{Capabilities, Classification};
#[cfg(test)]
mod audit;
#[cfg(test)]
mod frozen_capabilities;

/// Compatibility facade for the shared predicate, not owner-lane admission.
pub(super) fn capabilities(
    version: crate::MinecraftVersion,
    name: &str,
) -> super::contact_capabilities::Capabilities {
    use super::contact_capabilities::{Capabilities, Classification};
    use key::{EntityType, OwnerLane};
    let Some(entity) = EntityType::from_name(version, name) else {
        return Capabilities::UNKNOWN;
    };
    let effects = table::lookup(version, entity, OwnerLane::OwnedWalking).effects;
    fn class(value: table::Class) -> Classification {
        match value {
            table::Class::Qualified => Classification::Qualified,
            table::Class::ProvenIrrelevant => Classification::ProvenIrrelevant,
            table::Class::BoundedUnknown => Classification::BoundedUnknown,
            table::Class::Unknown => Classification::Unknown,
        }
    }
    Capabilities {
        walking: class(effects.walking),
        incoming: class(effects.incoming),
        solid: class(effects.solid),
        geometry: class(effects.geometry),
        metadata: class(effects.metadata),
        status: class(effects.status),
        animation: class(effects.animation),
    }
}

pub(super) fn harmless_packet(
    version: crate::MinecraftVersion,
    name: &str,
    kind: &super::EntityHistoryKind,
) -> bool {
    use key::{Effect, EntityType, OwnerLane, PacketKey};
    let Some(entity) = EntityType::from_name(version, name) else {
        return false;
    };
    // The original predicate observes only Status/Animation. Keep that boundary
    // explicit; a packet decision never implies discarding an entire receipt.
    let effect = match kind {
        super::EntityHistoryKind::Status { .. } => Effect::Status,
        super::EntityHistoryKind::Animation { .. } => Effect::Animation,
        _ => return false,
    };
    let profile = table::lookup(version, entity, OwnerLane::OwnedWalking);
    let policy = table::PACKET_POLICIES[profile.packets as usize];
    let packet = PacketKey::from_history(kind);
    let decision = policy
        .exact
        .iter()
        .find(|rule| rule.packet == packet && rule.effects.contains(&effect))
        .map_or(policy.default, |rule| rule.decision);
    matches!(decision, table::Decision::Irrelevant(_))
}
