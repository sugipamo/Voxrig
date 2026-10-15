//! Private per-version evidence policy. Unclassified effects always refuse.
use super::{EntityHistoryKind, EntityId};
use crate::{MinecraftVersion, MovementPacketTrigger, MovementRefusal, Result, client::Aabb};
#[path = "contact_capabilities_generated.rs"]
mod generated;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Classification {
    Qualified,
    ProvenIrrelevant,
    /// Only a bounded subset is audited; this never authorizes distance deferral.
    BoundedUnknown,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Capabilities {
    pub walking: Classification,
    pub incoming: Classification,
    pub solid: Classification,
    pub geometry: Classification,
    pub metadata: Classification,
    pub status: Classification,
    pub animation: Classification,
}
impl Capabilities {
    const UNKNOWN: Self = Self {
        walking: Classification::Unknown,
        incoming: Classification::Unknown,
        solid: Classification::Unknown,
        geometry: Classification::Unknown,
        metadata: Classification::Unknown,
        status: Classification::Unknown,
        animation: Classification::Unknown,
    };
    pub fn modeled(self) -> bool {
        self.walking == Classification::Qualified
            && self.incoming == Classification::Qualified
            && self.solid == Classification::ProvenIrrelevant
            && self.geometry == Classification::Qualified
            && self.metadata == Classification::Qualified
    }
}
pub(super) fn capabilities(version: MinecraftVersion, name: &str) -> Capabilities {
    generated::lookup(version, name)
}

/// Step A's finite packet proof, used only on an audited callback lane.
pub(super) fn harmless_packet(
    version: MinecraftVersion,
    name: &str,
    kind: &EntityHistoryKind,
) -> bool {
    let policy = capabilities(version, name);
    match kind {
        EntityHistoryKind::Status { status, .. }
            if policy.status == Classification::BoundedUnknown =>
        {
            *status == 35 || (*status == 10 && name == "minecraft:sheep")
        }
        EntityHistoryKind::Animation {
            animation: 4 | 5, ..
        } => policy.animation == Classification::BoundedUnknown,
        _ => false,
    }
}

/// Native slots for excluded/unqualified bodies, without numerical geometry.
/// The WIP hull did not prove indirect effects; unknowns now always refuse.
#[derive(Clone, Debug)]
pub(super) struct DeferredContact {
    pub entity: EntityId,
    pub capabilities: Capabilities,
    name: String,
    ordinal: u64,
}
impl DeferredContact {
    pub fn new(version: MinecraftVersion, entity: EntityId, name: &str, ordinal: u64) -> Self {
        Self {
            entity,
            capabilities: capabilities(version, name),
            name: name.into(),
            ordinal,
        }
    }
    pub fn relevant(&self, boat: bool) -> bool {
        self.capabilities.solid != Classification::ProvenIrrelevant
            || if boat {
                self.capabilities.incoming != Classification::ProvenIrrelevant
            } else {
                self.capabilities.walking != Classification::ProvenIrrelevant
            }
    }
    pub fn check(&self, _query: Aabb, boat: bool) -> Result<()> {
        if self.relevant(boat) {
            let mut value = MovementRefusal::permanent(format!(
                "unqualified contact capabilities: {} {:?}",
                self.name, self.capabilities
            ));
            let context = value.context_mut();
            context.session = Some(self.entity.session());
            context.entity = Some(self.entity);
            context.native_entity_id = Some(self.entity.native_id());
            context.entity_type = Some(self.name.clone());
            context.receive_sequence = Some(self.entity.spawn_sequence());
            context.record_ordinal = Some(self.ordinal);
            context.packet = Some(MovementPacketTrigger {
                packet_type: "Spawn".into(),
                opcode: 0,
            });
            return Err(MovementRefusal::Unsupported {
                context: value.context().clone(),
            }
            .into_error());
        }
        Ok(())
    }
}
pub(super) fn check<'a>(
    bodies: impl Iterator<Item = &'a DeferredContact>,
    query: Aabb,
    boat: bool,
) -> Result<()> {
    for body in bodies {
        body.check(query, boat)?;
    }
    Ok(())
}
pub(super) fn hull(a: Aabb, b: Aabb) -> Aabb {
    Aabb {
        min_x: a.min_x.min(b.min_x),
        min_y: a.min_y.min(b.min_y),
        min_z: a.min_z.min(b.min_z),
        max_x: a.max_x.max(b.max_x),
        max_y: a.max_y.max(b.max_y),
        max_z: a.max_z.max(b.max_z),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn generated_policy_is_bound_to_original_class_and_metadata_evidence() {
        let evidence: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/contact-capabilities-20261014.json"
        ))
        .unwrap();
        for (key, bytes) in [
            (
                "generator_sha256",
                &include_bytes!("../../../scripts/audit_contact_capabilities.py")[..],
            ),
            (
                "generated_sha256",
                &include_bytes!("contact_capabilities_generated.rs")[..],
            ),
            (
                "audit_sha256",
                &include_bytes!("../../../docs/contact-capability-audit.md")[..],
            ),
            (
                "packet_audit_sha256",
                &include_bytes!("../../../data/client_api/contact_packet_audit.json")[..],
            ),
        ] {
            assert_eq!(evidence[key], format!("{:x}", Sha256::digest(bytes)));
        }
        for (version, key, metadata) in [
            (
                MinecraftVersion::Java1_16_1,
                "1.16.1",
                &include_bytes!("../../../data/client_api/entity_data-1.16.1.json")[..],
            ),
            (
                MinecraftVersion::Java1_21_11,
                "1.21.11",
                &include_bytes!("../../../data/client_api/entity_data-1.21.11.json")[..],
            ),
        ] {
            assert_eq!(
                evidence["versions"][key]["entity_data_sha256"],
                format!("{:x}", Sha256::digest(metadata))
            );
            assert!(
                !evidence["versions"][key]["method_sha256"]
                    .as_object()
                    .unwrap()
                    .is_empty()
            );
            for (name, row) in evidence["versions"][key]["types"].as_object().unwrap() {
                let c = capabilities(version, &format!("minecraft:{name}"));
                for (lane, value) in [
                    ("walking", c.walking),
                    ("incoming", c.incoming),
                    ("solid", c.solid),
                    ("geometry", c.geometry),
                    ("metadata", c.metadata),
                    ("status", c.status),
                    ("animation", c.animation),
                ] {
                    assert_eq!(
                        format!("{value:?}"),
                        row["classifications"][lane],
                        "{key} {name} {lane}"
                    );
                }
                if c.modeled() {
                    assert!(
                        ["cow", "pig", "zombie", "sheep", "creeper", "player"]
                            .contains(&name.as_str())
                    );
                }
            }
            assert_eq!(
                capabilities(version, "minecraft:bat").walking,
                Classification::ProvenIrrelevant
            );
            assert_eq!(
                capabilities(version, "minecraft:armor_stand").incoming,
                Classification::ProvenIrrelevant
            );
            assert_eq!(
                capabilities(version, "minecraft:horse").walking,
                Classification::BoundedUnknown
            );
            assert_eq!(
                capabilities(version, "minecraft:slime").geometry,
                Classification::Unknown
            );
            assert_eq!(
                capabilities(version, "minecraft:firework_rocket"),
                Capabilities::UNKNOWN
            );
            assert_eq!(
                capabilities(version, "minecraft:not_registered"),
                Capabilities::UNKNOWN
            );
        }
    }
}
