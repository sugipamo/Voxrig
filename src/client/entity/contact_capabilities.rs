//! Private, per-version owner policy. Ancestry alone never qualifies dispatch.
use super::{
    EntityHistoryValue, EntityId, LivingInterpolationPose, LivingPacketGuards, LivingPacketModel,
};
use crate::{MinecraftVersion, Result, client::Aabb};
#[path = "contact_capabilities_generated.rs"]
mod generated;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Capability {
    Inherited,
    RemotePlayer,
    Irrelevant,
    BoundedUnknown,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WalkingPush {
    None,
    Inherited,
    RemotePlayer,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IncomingPush {
    None,
    Living,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SolidCollision {
    None,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Geometry {
    Living,
    ConservativeLiving,
    Unknown,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Capabilities {
    pub walking: WalkingPush,
    pub incoming: IncomingPush,
    pub solid: SolidCollision,
    pub geometry: Geometry,
}
pub(super) fn capabilities(version: MinecraftVersion, name: &str) -> Capabilities {
    match generated::lookup(version, name) {
        Capability::Inherited | Capability::RemotePlayer => Capabilities {
            walking: if name == "minecraft:player" {
                WalkingPush::RemotePlayer
            } else {
                WalkingPush::Inherited
            },
            incoming: IncomingPush::Living,
            solid: SolidCollision::None,
            geometry: Geometry::Living,
        },
        Capability::Irrelevant => Capabilities {
            walking: WalkingPush::None,
            incoming: IncomingPush::None,
            solid: SolidCollision::None,
            geometry: Geometry::Unknown,
        },
        capability => Capabilities {
            walking: WalkingPush::Unknown,
            incoming: IncomingPush::Unknown,
            solid: if capability == Capability::BoundedUnknown {
                SolidCollision::None
            } else {
                SolidCollision::Unknown
            },
            geometry: if capability == Capability::BoundedUnknown {
                Geometry::ConservativeLiving
            } else {
                Geometry::Unknown
            },
        },
    }
}

/// Unsupported hooks never get a public numerical sample. The hull includes
/// every possible inherited interpolation anchor since admission, not merely
/// the latest received target. It deliberately does not shrink with elapsed
/// ticks. Unknown movement/metadata/mounting permanently removes the proof.
#[derive(Clone, Debug)]
pub(super) struct DeferredContact {
    pub entity: EntityId,
    pub capabilities: Capabilities,
    motion: Option<LivingPacketModel>,
    envelope: Option<Aabb>,
    dimensions: [f64; 2],
    name: String,
    version: MinecraftVersion,
}
impl DeferredContact {
    pub fn new(
        version: MinecraftVersion,
        entity: EntityId,
        name: &str,
        pose: Option<LivingInterpolationPose>,
        ordinal: u64,
    ) -> Self {
        let capabilities = capabilities(version, name);
        let dimensions = version
            .table()
            .entity_dimensions(name)
            .map(|d| [f64::from(d.width as f32), f64::from(d.height as f32)]);
        let motion = (capabilities.geometry == Geometry::ConservativeLiving)
            .then(|| pose.and_then(|p| LivingPacketModel::new(version, p, ordinal).ok()))
            .flatten();
        let mut result = Self {
            entity,
            capabilities,
            motion,
            envelope: None,
            dimensions: dimensions.unwrap_or([0.; 2]),
            name: name.to_owned(),
            version,
        };
        if dimensions.is_none() {
            result.motion = None;
        }
        result.extend();
        result
    }
    fn extend(&mut self) {
        let Some(motion) = &self.motion else {
            return;
        };
        let frame = motion.frame().interpolation;
        for position in std::iter::once(frame.pose.position).chain(frame.target.map(|p| p.position))
        {
            // Full width covers legacy refreshDimensions' temporarily anchored
            // growing box as well as both standing/baby centered shapes.
            let [width, height] = self.dimensions;
            let b = Aabb {
                min_x: position[0] - width,
                min_y: position[1],
                min_z: position[2] - width,
                max_x: position[0] + width,
                max_y: position[1] + height.max(f64::from(0.2_f32)),
                max_z: position[2] + width,
            };
            self.envelope = Some(self.envelope.map_or(b, |a| hull(a, b)));
        }
    }
    pub fn invalidate(&mut self) {
        self.motion = None;
        self.envelope = None;
    }
    pub fn apply(&mut self, ordinal: u64, packet: Option<super::LivingMotionPacket>) {
        let Some(motion) = &mut self.motion else {
            return;
        };
        if packet.is_none_or(|packet| {
            motion
                .apply(
                    ordinal,
                    packet,
                    LivingPacketGuards {
                        local_authoritative: false,
                        ticking: true,
                    },
                )
                .is_err()
        }) {
            self.invalidate();
        } else {
            self.extend();
        }
    }
    pub fn metadata(
        &mut self,
        entries: &EntityHistoryValue<Vec<super::EntityMetadataEntry>>,
        complete: bool,
    ) {
        if !complete {
            self.invalidate();
            return;
        }
        let EntityHistoryValue::Retained { value } = entries else {
            self.invalidate();
            return;
        };
        for entry in value {
            let row = self
                .version
                .table()
                .entity_data(&self.name)
                .iter()
                .find(|r| r.index == entry.index);
            let safe = row.is_some_and(|r| {
                let field = |f: super::EntityDataField| f.official_names().contains(&(r.owner, r.field));
                if entry.index == 6 {
                    matches!(entry.value, super::EntityDataValue::Pose(id) if super::EntityPose::from_native(self.version, id).is_some())
                } else if field(super::EntityDataField::Baby) {
                    matches!(entry.value, super::EntityDataValue::Bool(_))
                } else if field(super::EntityDataField::Health) {
                    matches!(entry.value, super::EntityDataValue::Float(v) if v.is_finite())
                } else { false }
            });
            if !safe {
                self.invalidate();
                break;
            }
        }
    }

    pub fn relevant(&self, boat: bool) -> bool {
        self.capabilities.solid != SolidCollision::None
            || if boat {
                self.capabilities.incoming != IncomingPush::None
            } else {
                self.capabilities.walking != WalkingPush::None
            }
    }
    pub fn check(&self, query: Aabb, boat: bool) -> Result<()> {
        if self.relevant(boat) && self.envelope.is_none_or(|b| intersects(query, b)) {
            return Err(super::contact_tick::failure(format!(
                "unqualified relevant contact: {:?}",
                self.entity
            )));
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
fn intersects(a: Aabb, b: Aabb) -> bool {
    a.min_x < b.max_x
        && a.max_x > b.min_x
        && a.min_y < b.max_y
        && a.max_y > b.min_y
        && a.min_z < b.max_z
        && a.max_z > b.min_z
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
        assert_eq!(
            evidence["generator_sha256"],
            format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../scripts/audit_contact_capabilities.py"
                ))
            )
        );
        assert_eq!(
            evidence["generated_sha256"],
            format!(
                "{:x}",
                Sha256::digest(include_bytes!("contact_capabilities_generated.rs"))
            )
        );
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
            for (name, row) in evidence["versions"][key]["types"].as_object().unwrap() {
                assert_eq!(
                    format!(
                        "{:?}",
                        generated::lookup(version, &format!("minecraft:{name}"))
                    ),
                    row["capability"]
                );
                if row["capability"] == "Inherited" {
                    for method in ["pushEntities", "doPush", "push", "isPushable"] {
                        assert_eq!(
                            row[method], "net.minecraft.world.entity.LivingEntity",
                            "{key} {name} {method}"
                        );
                    }
                }
            }
            assert_eq!(
                capabilities(version, "minecraft:bat").walking,
                WalkingPush::None
            );
            assert_eq!(
                capabilities(version, "minecraft:armor_stand").incoming,
                IncomingPush::None
            );
            assert_eq!(
                capabilities(version, "minecraft:slime").geometry,
                Geometry::Unknown
            );
            assert_eq!(
                capabilities(version, "minecraft:shulker").solid,
                SolidCollision::Unknown
            );
            assert_eq!(
                capabilities(version, "minecraft:firework_rocket").walking,
                WalkingPush::Unknown
            );
        }
    }
}
