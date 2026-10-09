//! Rigid collision boxes derived from owned received targets, never contacts.
use super::control::VehicleControlRecord;
use crate::Result;
use crate::client::{
    EntityId, EntityPosition, ObservedValue, ValueSource, entity::SpawnLedger,
    inventory::unavailable,
};

/// Maximum rigid bodies retained in each of at most 120 local boat ticks.
pub const MAX_BOAT_COLLISION_BODIES: usize = 32;

/// One fixed-shape vehicle used by the boat's local collision model.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BoatCollisionBody {
    /// Original connection/world/spawn lifetime, independent of reusable native ID.
    pub entity: EntityId,
    /// Original known type, restricted to fixed-shape boats/rafts and minecarts.
    pub type_name: String,
    /// Latest received packet target and quantization bound, without interpolation.
    pub received_position: ObservedValue<EntityPosition>,
    /// Derived model AABB at that target; not a received or server-current box.
    pub model_box: [f64; 6],
}

/// Coherent inputs selected before an attempted local boat tick.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BoatCollisionSample {
    /// Upcoming one-based tick; sampling does not prove complete dispatch.
    pub sampled_before_tick: u16,
    /// Coherent receive boundary, separate from each body's original source.
    pub receive_sequence: u64,
    /// Latest actual ordered own-boat passengers excluded from collision input.
    pub passengers: ObservedValue<Vec<i32>>,
    /// Rigid-body inputs in native ID order. An empty list is not a current-world fact.
    pub bodies: Vec<BoatCollisionBody>,
}

fn rigid(name: &str) -> bool {
    name == "minecraft:boat"
        || name.ends_with("_boat")
        || name.ends_with("_raft")
        || name == "minecraft:minecart"
        || name.ends_with("_minecart")
}
fn noncollidable(name: &str) -> bool {
    matches!(
        name,
        "minecraft:item"
            | "minecraft:experience_orb"
            | "minecraft:area_effect_cloud"
            | "minecraft:arrow"
            | "minecraft:spectral_arrow"
            | "minecraft:trident"
            | "minecraft:egg"
            | "minecraft:snowball"
            | "minecraft:ender_pearl"
            | "minecraft:potion"
            | "minecraft:experience_bottle"
            | "minecraft:firework_rocket"
    )
}

pub(crate) fn sample(
    record: &VehicleControlRecord,
    vehicle: &super::VehicleObservation,
    ledger: &SpawnLedger,
    relations: &super::PassengerLedger,
    sequence: u64,
) -> Result<Option<BoatCollisionSample>> {
    let Some(boat) = &record.boat_motion else {
        return Ok(None);
    };
    let session = record.id.mount().session();
    relations.collision_safe(record.id.mount(), ledger)?;
    let passengers = vehicle
        .passengers
        .clone()
        .ok_or_else(|| unavailable("boat collision passengers unavailable"))?;
    if vehicle.session != session
        || vehicle.receive_sequence != sequence
        || !matches!(passengers.source,ValueSource::Received{sequence:s} if s>=record.id.mount().receive_sequence() && s<=sequence)
    {
        return Err(unavailable(
            "boat collision passengers outside coherent owned boundary",
        ));
    }
    let mut seed = boat.frames.last().unwrap_or(&boat.initial_frame).clone();
    if let Some(v) = boat.pending_velocity {
        seed.velocity = v;
    }
    let bodies = bodies(
        session,
        sequence,
        boat.received.entity.id,
        &passengers.value,
        &seed,
        ledger,
    )?;
    Ok(Some(BoatCollisionSample {
        sampled_before_tick: record.dispatched_ticks + 1,
        receive_sequence: sequence,
        passengers,
        bodies,
    }))
}

fn bodies(
    session: crate::client::SessionStamp,
    sequence: u64,
    own: EntityId,
    passengers: &[i32],
    seed: &super::BoatFrame,
    ledger: &SpawnLedger,
) -> Result<Vec<BoatCollisionBody>> {
    let mut bodies = Vec::new();
    for spawn in ledger.capture(session, sequence).entities {
        if spawn.id == own || passengers.contains(&spawn.id.native_id()) {
            continue;
        }
        let name = spawn
            .type_name
            .as_deref()
            .ok_or_else(|| unavailable("boat collision type unavailable"))?;
        if noncollidable(name) {
            continue;
        }
        let motion = ledger.capture_motion(session, spawn.id, sequence)?;
        let position = motion
            .position
            .ok_or_else(|| unavailable("boat collision target has unresolved position"))?;
        if !matches!(position.source,ValueSource::Received{sequence:s} if s>=spawn.id.spawn_sequence() && s<=sequence)
            || position.value.position.iter().any(|v| !v.is_finite())
        {
            return Err(unavailable(
                "boat collision target lacks owned finite original receipt",
            ));
        }
        if !rigid(name) {
            // Other entities need their own pose/scale/pushability audit. Refuse
            // nearby unsupported targets rather than using default adult boxes.
            if (0..3).all(|i| {
                (position.value.position[i] - seed.position[i]).abs()
                    <= 3.0
                        + seed.velocity[i].abs()
                        + if i == 1 {
                            seed.last_vertical_movement.abs()
                        } else {
                            0.0
                        }
            }) {
                return Err(unavailable(
                    "nearby boat entity collision type/pose not audited",
                ));
            }
            continue;
        }
        if bodies.len() == MAX_BOAT_COLLISION_BODIES {
            return Err(unavailable("boat collision body budget exceeded"));
        }
        let dimensions = session
            .version
            .table()
            .entity_dimensions(name)
            .ok_or_else(|| unavailable("rigid boat collision dimensions unavailable"))?;
        // EntityDimensions stores native floats. Decimal table observations are
        // converted through f32 before constructing the original AABB.
        let width = f64::from(dimensions.width as f32);
        let height = f64::from(dimensions.height as f32);
        let [x, y, z] = position.value.position;
        bodies.push(BoatCollisionBody {
            entity: spawn.id,
            type_name: name.to_owned(),
            received_position: position,
            model_box: [
                x - width / 2.0,
                y,
                z - width / 2.0,
                x + width / 2.0,
                y + height,
                z + width / 2.0,
            ],
        });
    }
    Ok(bodies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MinecraftVersion,
        client::{
            SessionStamp,
            entity::{NativeMotion, NativeSpawn, NativeSpawnMotion},
        },
    };
    fn spawn(
        ledger: &mut SpawnLedger,
        version: MinecraftVersion,
        id: i32,
        name: &'static str,
        position: [f64; 3],
        sequence: u64,
    ) {
        ledger
            .insert(
                version,
                NativeSpawn {
                    id,
                    uuid: None,
                    type_id: None,
                    dedicated_type_name: Some(name.strip_prefix("minecraft:").unwrap()),
                    position,
                    living: None,
                },
                sequence,
                4096,
            )
            .unwrap();
        ledger.initialize_motion(
            id,
            NativeSpawnMotion {
                position: Some(position),
                ..Default::default()
            },
            sequence,
        );
    }
    #[test]
    fn received_rigid_shapes_match_original_native_boxes_and_id_reuse_does_not_rebind_history() {
        let oracle: serde_json::Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/boat_collision_oracle.json.gz")[..],
        ))
        .unwrap();
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let key = if version == MinecraftVersion::Java1_16_1 {
                "1.16.1"
            } else {
                "1.21.11"
            };
            let boat = if version == MinecraftVersion::Java1_16_1 {
                "minecraft:boat"
            } else {
                "minecraft:oak_boat"
            };
            let session = SessionStamp {
                version,
                connection_id: 1,
                world_generation: 2,
            };
            for (scene, name) in [
                ("rigid_front_boat", boat),
                ("rigid_front_cart", "minecraft:minecart"),
            ] {
                let result = oracle["results"][key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["name"] == scene)
                    .unwrap();
                let mut ledger = SpawnLedger::default();
                let origin = [1024., 100., 1024.];
                let start = [1024.5, 101., 1024.5];
                let other = [1024.5, 101., 1027.];
                spawn(&mut ledger, version, 10, boat, start, 1);
                spawn(&mut ledger, version, 11, name, other, 2);
                let own = ledger.identity(session, 10).unwrap();
                let seed = super::super::BoatFrame::new(start, [0.; 2], [0.; 3]);
                let old = bodies(session, 3, own, &[42], &seed, &ledger).unwrap();
                let expected: [f64; 6] = std::array::from_fn(|i| {
                    result["collision_boxes"][0][i]
                        .as_str()
                        .unwrap()
                        .parse::<f64>()
                        .unwrap()
                        + origin[i % 3]
                });
                assert_eq!(old[0].model_box, expected, "{key}/{scene}");
                assert_eq!(
                    old[0].received_position.source,
                    ValueSource::Received { sequence: 2 }
                );
                ledger.remove(11);
                assert!(
                    bodies(session, 4, own, &[42], &seed, &ledger)
                        .unwrap()
                        .is_empty()
                );
                spawn(&mut ledger, version, 11, name, [1024.5, 101., 1029.], 5);
                let fresh = bodies(session, 6, own, &[42], &seed, &ledger).unwrap();
                assert_ne!(old[0].entity, fresh[0].entity);
                assert_eq!(old[0].model_box, expected);
                assert_eq!(
                    old[0].received_position.source,
                    ValueSource::Received { sequence: 2 }
                );
                assert_eq!(
                    fresh[0].received_position.source,
                    ValueSource::Received { sequence: 5 }
                );
                assert!(
                    bodies(session, 6, own, &[42, 11], &seed, &ledger)
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }
    #[test]
    fn unresolved_targets_unsupported_nearby_poses_and_body_budget_refuse() {
        let version = MinecraftVersion::Java1_21_11;
        let session = SessionStamp {
            version,
            connection_id: 1,
            world_generation: 2,
        };
        let mut ledger = SpawnLedger::default();
        spawn(
            &mut ledger,
            version,
            10,
            "minecraft:oak_boat",
            [0.5, 1., 0.5],
            1,
        );
        spawn(
            &mut ledger,
            version,
            11,
            "minecraft:minecart",
            [0.5, 1., 3.],
            2,
        );
        let own = ledger.identity(session, 10).unwrap();
        let seed = super::super::BoatFrame::new([0.5, 1., 0.5], [0.; 2], [0.; 3]);
        ledger.receive_motion(
            version,
            11,
            NativeMotion::Correction {
                change: crate::client::EntityPositionCorrection {
                    position: [1.; 3],
                    delta: [0.; 3],
                    rotation: [0.; 2],
                    flags: 1,
                },
                ground: false,
            },
            3,
        );
        assert!(bodies(session, 3, own, &[42], &seed, &ledger).is_err());
        ledger.receive_motion(
            version,
            11,
            NativeMotion::Absolute {
                position: [0.5, 1., 3.],
                rotation: [0.; 2],
                velocity: None,
                ground: None,
                reset_base: true,
            },
            4,
        );
        assert!(bodies(session, 4, own, &[42], &seed, &ledger).is_ok());
        spawn(
            &mut ledger,
            version,
            12,
            "minecraft:sheep",
            [0.5, 1., 2.],
            5,
        );
        assert!(bodies(session, 5, own, &[42], &seed, &ledger).is_err());
        ledger.remove(12);
        for id in 100..132 {
            spawn(
                &mut ledger,
                version,
                id,
                "minecraft:minecart",
                [0.5, 1., f64::from(id)],
                6,
            );
        }
        assert!(bodies(session, 6, own, &[42], &seed, &ledger).is_err());
        ledger.remove(131);
        assert_eq!(
            bodies(session, 7, own, &[42], &seed, &ledger)
                .unwrap()
                .len(),
            MAX_BOAT_COLLISION_BODIES
        );
    }
}
