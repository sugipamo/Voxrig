//! Private per-version evidence policy. Unclassified effects always refuse.
use super::{
    EntityHistoryKind, EntityId, LivingInterpolationPose, LivingPacketGuards, LivingPacketModel,
};
use crate::{MinecraftVersion, MovementPacketTrigger, MovementRefusal, Result, client::Aabb};
#[path = "contact_capabilities_generated.rs"]
mod generated;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Classification {
    Qualified,
    ProvenIrrelevant,
    /// Only a bounded subset is audited; a separate envelope proof is required.
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

/// Native slots for excluded/unqualified bodies. A separate, finite proof is
/// available only for the two audited spider classes, before unproven callbacks.
/// This is a monotonically growing possibility set, never numerical geometry.
#[derive(Clone, Debug)]
pub(super) struct DeferredContact {
    pub entity: EntityId,
    pub capabilities: Capabilities,
    name: String,
    ordinal: u64,
    sequence: u64,
    trigger: MovementPacketTrigger,
    motion: Option<LivingPacketModel>,
    envelope: Option<Aabb>,
    dimensions: [f64; 2],
}
impl DeferredContact {
    pub fn new(
        version: MinecraftVersion,
        entity: EntityId,
        name: &str,
        ordinal: u64,
        pose: Option<LivingInterpolationPose>,
    ) -> Self {
        // Separate from Step C classification: Unknown never implies locality.
        let dimensions = version.table().entity_dimensions(name);
        let motion = if matches!(
            (version, name),
            (
                MinecraftVersion::Java1_16_1 | MinecraftVersion::Java1_21_11,
                "minecraft:spider" | "minecraft:cave_spider"
            )
        ) {
            pose.and_then(|p| LivingPacketModel::new(version, p, ordinal).ok())
        } else {
            None
        };
        let mut result = Self {
            entity,
            capabilities: capabilities(version, name),
            name: name.into(),
            ordinal,
            sequence: entity.spawn_sequence(),
            trigger: MovementPacketTrigger {
                packet_type: "Spawn".into(),
                opcode: 0,
            },
            motion,
            envelope: None,
            dimensions: dimensions.map_or([0.; 2], |d| {
                [f64::from(d.width as f32), f64::from(d.height as f32)]
            }),
        };
        if dimensions.is_none() {
            result.motion = None;
        }
        result.extend();
        result
    }
    fn extend(&mut self) {
        if let Some(motion) = &self.motion {
            let frame = motion.frame();
            for p in std::iter::once(frame.interpolation.pose.position)
                .chain(frame.interpolation.target.map(|p| p.position))
                .chain(std::iter::once(frame.codec_base))
            {
                self.include(p);
            }
        }
    }
    fn include(&mut self, p: [f64; 3]) {
        let [width, height] = self.dimensions;
        // Full width and a whole-block rounding margin strictly enclose the
        // default fixed shape and interpolation arithmetic in the codec domain.
        // Absolute placement can clamp X/Z while retaining the raw packet
        // codec base (legacy absMoveTo). Enclose both choices on both axes.
        let clamped = [
            p[0].clamp(-30_000_000., 30_000_000.),
            p[2].clamp(-30_000_000., 30_000_000.),
        ];
        let b = Aabb {
            min_x: p[0].min(clamped[0]) - width - 1.,
            min_y: p[1] - 1.,
            min_z: p[2].min(clamped[1]) - width - 1.,
            max_x: p[0].max(clamped[0]) + width + 1.,
            max_y: p[1] + height + 1.,
            max_z: p[2].max(clamped[1]) + width + 1.,
        };
        self.envelope = Some(self.envelope.map_or(b, |a| hull(a, b)));
    }
    pub fn invalidate(&mut self, record: &super::EntityHistoryRecord, packet: &str, opcode: i32) {
        if self.motion.take().is_some() {
            self.envelope = None;
            self.ordinal = record.ordinal;
            self.sequence = record.receive_sequence;
            self.trigger = MovementPacketTrigger {
                packet_type: packet.into(),
                opcode,
            };
        }
    }
    pub fn apply(
        &mut self,
        record: &super::EntityHistoryRecord,
        observation: &super::EntityMotionObservation,
    ) {
        if self.motion.is_none() {
            return;
        }
        let packet = observation
            .packet
            .as_ref()
            .filter(|p| {
                p.source
                    == crate::client::ValueSource::Received {
                        sequence: record.receive_sequence,
                    }
            })
            .and_then(|p| p.value.living_packet(record.session.version).ok());
        let valid = match (self.motion.as_mut(), packet) {
            (Some(motion), Some(packet)) => motion
                .apply(
                    record.ordinal,
                    packet,
                    LivingPacketGuards {
                        local_authoritative: false,
                        ticking: true,
                    },
                )
                .is_ok(),
            _ => false,
        };
        if !valid {
            self.invalidate(record, "Motion", 0);
            return;
        }
        // Include raw absolute endpoints too: legacy tiny-teleport suppression
        // depends on an unknown interpolated pose, not our unstepped codec model.
        match packet {
            Some(super::LivingMotionPacket::LegacyTeleport { position, .. })
            | Some(super::LivingMotionPacket::ModernPositionSync { position, .. }) => {
                self.include(position)
            }
            _ => {}
        }
        self.extend();
    }
    pub fn relevant(&self, boat: bool) -> bool {
        self.capabilities.solid != Classification::ProvenIrrelevant
            || if boat {
                self.capabilities.incoming != Classification::ProvenIrrelevant
            } else {
                self.capabilities.walking != Classification::ProvenIrrelevant
            }
    }
    pub fn check(&self, query: Aabb, boat: bool) -> Result<()> {
        if self.relevant(boat) && self.envelope.is_none_or(|b| intersects(query, b)) {
            let mut value = MovementRefusal::permanent(format!(
                "unqualified contact capabilities: {} {:?}",
                self.name, self.capabilities
            ));
            let context = value.context_mut();
            context.session = Some(self.entity.session());
            context.entity = Some(self.entity);
            context.native_entity_id = Some(self.entity.native_id());
            context.entity_type = Some(self.name.clone());
            context.receive_sequence = Some(self.sequence);
            context.record_ordinal = Some(self.ordinal);
            context.packet = Some(self.trigger.clone());
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
// Closed overlap intentionally refuses even touching boundaries.
fn intersects(a: Aabb, b: Aabb) -> bool {
    let valid = |b: Aabb| {
        [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
            .iter()
            .all(|v| v.is_finite())
            && b.min_x <= b.max_x
            && b.min_y <= b.max_y
            && b.min_z <= b.max_z
    };
    // Invalid geometry cannot establish a negative spatial proof.
    if !valid(a) || !valid(b) {
        return true;
    }
    a.min_x <= b.max_x
        && a.max_x >= b.min_x
        && a.min_y <= b.max_y
        && a.max_y >= b.min_y
        && a.min_z <= b.max_z
        && a.max_z >= b.min_z
}

/// Conservatively rule out indirect influence on *every* modeled remote body,
/// including both sides of its native slot and its remaining interpolation path.
/// Rejecting an unrelated remote overlap is intentional: no graph shortcut.
pub(super) fn check_peers<'a>(
    bodies: impl Iterator<Item = &'a DeferredContact> + Clone,
    world: &super::contact_tick::ContactTickWorld,
    boat: bool,
) -> Result<()> {
    for remote in world.bodies.values() {
        let frame = remote.motion.frame().interpolation;
        // A legacy metadata refresh may leave an anchored box until the next
        // setPos; enclose both that current box and its recentered placement.
        let mut query = hull(remote.contact().bounds, remote.centered_bounds());
        for t in frame
            .target
            .map(|p| p.position)
            .into_iter()
            .chain(remote.motion.collision_query_position())
        {
            let p = frame.pose.position;
            let shifted = Aabb {
                min_x: query.min_x + t[0] - p[0],
                min_y: query.min_y + t[1] - p[1],
                min_z: query.min_z + t[2] - p[2],
                max_x: query.max_x + t[0] - p[0],
                max_y: query.max_y + t[1] - p[1],
                max_z: query.max_z + t[2] - p[2],
            };
            query = hull(query, shifted);
        }
        check(bodies.clone(), query, boat)?;
    }
    Ok(())
}
/// Physics visits every cell of its swept/step-up collision region (plus a
/// border) in Level::geometry. Checking even cached-source cells on first read
/// covers clipped sweeps, not just the resulting before/after feet positions.
pub(super) fn cell(p: [i32; 3]) -> Aabb {
    Aabb {
        min_x: f64::from(p[0]),
        min_y: f64::from(p[1]),
        min_z: f64::from(p[2]),
        max_x: f64::from(p[0]) + 1.,
        max_y: f64::from(p[1]) + 1.,
        max_z: f64::from(p[2]) + 1.,
    }
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
    fn envelope_evidence_binds_original_owners_and_complete_query_sources() {
        let evidence: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/contact-envelopes-20261015.json"
        ))
        .unwrap();
        let previous: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/contact-capabilities-20261014.json"
        ))
        .unwrap();
        for version in ["1.16.1", "1.21.11"] {
            let v = &evidence["versions"][version];
            assert_eq!(
                v["original_client_jar_sha1"],
                previous["versions"][version]["original_client_jar_sha1"]
            );
            assert_eq!(
                v["client_mappings_sha1"],
                previous["versions"][version]["client_mappings_sha1"]
            );
            for (class, methods) in previous["versions"][version]["method_sha256"]
                .as_object()
                .unwrap()
            {
                if let Some(new) = v["method_sha256"].get(class) {
                    assert_eq!(new, methods, "{version} {class}");
                }
            }
            for name in ["spider", "cave_spider"] {
                for (method, owner) in previous["versions"][version]["types"][name]
                    .as_object()
                    .unwrap()
                {
                    if !matches!(method.as_str(), "audit" | "classifications") {
                        assert_eq!(v["types"][name].get(method), Some(owner));
                    }
                }
                assert_eq!(
                    v["types"][name]["pushEntities"],
                    "net.minecraft.world.entity.LivingEntity"
                );
            }
        }
        let sources: &[(&str, &[u8])] = &[
            (
                "scripts/audit_contact_envelopes.py",
                include_bytes!("../../../scripts/audit_contact_envelopes.py"),
            ),
            (
                "docs/contact-envelope-audit.md",
                include_bytes!("../../../docs/contact-envelope-audit.md"),
            ),
            (
                "docs/evidence/contact-capabilities-20261014.json",
                include_bytes!("../../../docs/evidence/contact-capabilities-20261014.json"),
            ),
            (
                "src/client/entity/contact_capabilities.rs",
                include_bytes!("../../../src/client/entity/contact_capabilities.rs"),
            ),
            (
                "src/client/entity/owned_contacts.rs",
                include_bytes!("../../../src/client/entity/owned_contacts.rs"),
            ),
            (
                "src/client/entity/interpolation.rs",
                include_bytes!("../../../src/client/entity/interpolation.rs"),
            ),
            (
                "src/client/entity/packet_model.rs",
                include_bytes!("../../../src/client/entity/packet_model.rs"),
            ),
            (
                "src/client/entity/contact_tick.rs",
                include_bytes!("../../../src/client/entity/contact_tick.rs"),
            ),
            (
                "src/client/entity/contact.rs",
                include_bytes!("../../../src/client/entity/contact.rs"),
            ),
            (
                "src/client/physics/mod.rs",
                include_bytes!("../../../src/client/physics/mod.rs"),
            ),
            (
                "src/client/physics/boat.rs",
                include_bytes!("../../../src/client/physics/boat.rs"),
            ),
            (
                "src/client/vehicle/collision.rs",
                include_bytes!("../../../src/client/vehicle/collision.rs"),
            ),
            (
                "src/client/vehicle/tests.rs",
                include_bytes!("../../../src/client/vehicle/tests.rs"),
            ),
            (
                "src/client/control.rs",
                include_bytes!("../../../src/client/control.rs"),
            ),
            (
                "docs/common-entity-owners.md",
                include_bytes!("../../../docs/common-entity-owners.md"),
            ),
            (
                "docs/common-entity-dispatch.md",
                include_bytes!("../../../docs/common-entity-dispatch.md"),
            ),
            (
                "data/entities.json",
                include_bytes!("../../../data/entities.json"),
            ),
            (
                "data/java_1_21_11/entity_dimensions.json",
                include_bytes!("../../../data/java_1_21_11/entity_dimensions.json"),
            ),
            (
                "src/versions/table.rs",
                include_bytes!("../../../src/versions/table.rs"),
            ),
            (
                "scripts/generate_version_tables.py",
                include_bytes!("../../../scripts/generate_version_tables.py"),
            ),
            (
                "data/client_api/entity_dimensions_oracle.json.gz",
                include_bytes!("../../../data/client_api/entity_dimensions_oracle.json.gz"),
            ),
        ];
        assert_eq!(
            evidence["source_sha256"].as_object().unwrap().len(),
            sources.len()
        );
        for (path, bytes) in sources {
            assert_eq!(
                evidence["source_sha256"][path],
                format!("{:x}", Sha256::digest(bytes)),
                "{path}"
            );
        }
    }
    #[test]
    fn generated_policy_is_bound_to_original_class_and_metadata_evidence() {
        let evidence: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/contact-capabilities-20261015.json"
        ))
        .unwrap();
        for (key, bytes) in [
            (
                "numerical_oracle_sha256",
                &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
            ),
            (
                "numerical_runner_sha256",
                &include_bytes!("../../../scripts/run_entity_owner_oracle.py")[..],
            ),
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
                        [
                            "cow",
                            "pig",
                            "zombie",
                            "sheep",
                            "creeper",
                            "player",
                            "wolf",
                            "chicken",
                            "rabbit",
                            "ocelot",
                            "villager",
                            "wandering_trader",
                            "mooshroom",
                            "silverfish",
                            "giant"
                        ]
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
