//! Packet targets and samples belonging to an original received spawn.
use super::*;

/// Latest received spatial fields, without render interpolation or entity physics.
/// Each field retains its own packet ordinal. A velocity sample is not proof
/// of current motion or a stop. Missing fields are never filled with defaults.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntityMotionObservation {
    /// Original spawn receipt, whose coordinates are never advanced.
    pub entity: EntitySpawn,
    /// Coherent capture boundary, separate from each field's receipt.
    pub receive_sequence: u64,
    /// Latest decoded position target; absent for an unresolved relative correction.
    pub position: Option<ObservedValue<EntityPosition>>,
    /// Body yaw/pitch in degrees, separate from head yaw.
    pub rotation: Option<ObservedValue<[f32; 2]>>,
    /// Explicitly supplied head yaw in degrees.
    pub head_yaw: Option<ObservedValue<f32>>,
    /// Last supplied velocity sample, in blocks per tick.
    pub velocity: Option<ObservedValue<[f64; 3]>>,
    /// Explicit ground flag; not independently validated collision/contact.
    pub on_ground: Option<ObservedValue<bool>>,
    /// Latest modern teleport fields, retained even when its baseline is unknown.
    pub correction: Option<ObservedValue<EntityPositionCorrection>>,
}
/// A native packet target, not the entity's server-current or interpolated pose.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct EntityPosition {
    /// Coordinates in blocks. Legacy painting anchors are not used as feet.
    pub position: [f64; 3],
    /// Conservative per-axis relative codec quantization bound in blocks.
    pub quantization_error: [f64; 3],
}
/// Original modern teleport data. Relative components need the actual native
/// interpolated baseline, which a historical packet sample does not establish.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct EntityPositionCorrection {
    /// Original position components.
    pub position: [f64; 3],
    /// Original movement components.
    pub delta: [f64; 3],
    /// Original yaw/pitch in degrees.
    pub rotation: [f32; 2],
    /// Native PositionFlag bit mask, scoped to the owning version.
    pub flags: u32,
}
#[derive(Clone, Default)]
pub(super) struct Motion {
    pub position: Option<ObservedValue<EntityPosition>>,
    pub rotation: Option<ObservedValue<[f32; 2]>>,
    pub head_yaw: Option<ObservedValue<f32>>,
    pub velocity: Option<ObservedValue<[f64; 3]>>,
    pub on_ground: Option<ObservedValue<bool>>,
    pub correction: Option<ObservedValue<EntityPositionCorrection>>,
    delta_base: Option<[f64; 3]>,
}
#[derive(Default)]
pub(crate) struct NativeSpawnMotion {
    pub position: Option<[f64; 3]>,
    pub rotation: Option<[f32; 2]>,
    pub head_yaw: Option<f32>,
    pub velocity: Option<[f64; 3]>,
}
pub(crate) enum NativeMotion {
    Relative {
        delta: [i16; 3],
        rotation: Option<[f32; 2]>,
        ground: bool,
    },
    Rotation {
        rotation: [f32; 2],
        ground: bool,
    },
    Absolute {
        position: [f64; 3],
        rotation: [f32; 2],
        velocity: Option<[f64; 3]>,
        ground: Option<bool>,
        reset_base: bool,
    },
    Correction {
        change: EntityPositionCorrection,
        ground: bool,
    },
    Velocity([f64; 3]),
    HeadYaw(f32),
}
impl Motion {
    fn position(&mut self, position: [f64; 3], error: [f64; 3], sequence: u64) {
        self.position = Some(super::super::received(
            EntityPosition {
                position,
                quantization_error: error,
            },
            sequence,
        ));
    }
    pub(super) fn initialize(&mut self, native: NativeSpawnMotion, sequence: u64) {
        self.delta_base = native.position;
        if let Some(position) = native.position {
            self.position(position, [0.0; 3], sequence);
        }
        self.rotation = native.rotation.map(|v| super::super::received(v, sequence));
        self.head_yaw = native.head_yaw.map(|v| super::super::received(v, sequence));
        self.velocity = native.velocity.map(|v| super::super::received(v, sequence));
    }
    pub(super) fn receive(
        &mut self,
        version: MinecraftVersion,
        update: NativeMotion,
        sequence: u64,
    ) {
        use super::super::received;
        match update {
            NativeMotion::Relative {
                delta,
                rotation,
                ground,
            } => {
                if let Some(mut base) = self.delta_base {
                    for axis in 0..3 {
                        match version {
                            MinecraftVersion::Java1_16_1 => {
                                base[axis] = ((base[axis] * 4096.0).floor()
                                    + f64::from(delta[axis]))
                                    / 4096.0
                            }
                            MinecraftVersion::Java1_21_11 if delta[axis] != 0 => {
                                base[axis] = ((base[axis] * 4096.0 + 0.5).floor()
                                    + f64::from(delta[axis]))
                                    / 4096.0
                            }
                            MinecraftVersion::Java1_21_11 => {}
                        }
                    }
                    self.delta_base = Some(base);
                    self.position(base, [1.0 / 4096.0; 3], sequence);
                }
                if let Some(rotation) = rotation {
                    self.rotation = Some(received(rotation, sequence));
                }
                self.on_ground = Some(received(ground, sequence));
            }
            NativeMotion::Rotation { rotation, ground } => {
                self.rotation = Some(received(rotation, sequence));
                self.on_ground = Some(received(ground, sequence));
            }
            NativeMotion::Absolute {
                position,
                rotation,
                velocity,
                ground,
                reset_base,
            } => {
                self.position(position, [0.0; 3], sequence);
                self.rotation = Some(received(rotation, sequence));
                if reset_base {
                    self.delta_base = Some(position);
                }
                if let Some(velocity) = velocity {
                    self.velocity = Some(received(velocity, sequence));
                }
                if let Some(ground) = ground {
                    self.on_ground = Some(received(ground, sequence));
                }
            }
            NativeMotion::Correction { change, ground } => {
                self.correction = Some(received(change, sequence));
                // Do not substitute historical packet targets for native interpolation.
                self.position = None;
                if change.flags & 7 == 0 {
                    self.position(change.position, [0.0; 3], sequence);
                }
                self.rotation = if change.flags & 24 == 0 {
                    Some(received(
                        [change.rotation[0], change.rotation[1].clamp(-90.0, 90.0)],
                        sequence,
                    ))
                } else {
                    None
                };
                self.velocity = if change.flags & 224 == 0 {
                    Some(received(change.delta, sequence))
                } else {
                    None
                };
                self.on_ground = Some(received(ground, sequence));
                // Teleports leave the separate native delta codec base intact.
            }
            NativeMotion::Velocity(v) => self.velocity = Some(received(v, sequence)),
            NativeMotion::HeadYaw(v) => self.head_yaw = Some(received(v, sequence)),
        }
    }
}
impl SpawnLedger {
    pub(crate) fn initialize_motion(&mut self, id: i32, native: NativeSpawnMotion, sequence: u64) {
        if let Some(spawn) = self.0.get_mut(&id).filter(|s| s.sequence == sequence) {
            spawn.motion.initialize(native, sequence);
        }
    }
    pub(crate) fn receive_motion(
        &mut self,
        version: MinecraftVersion,
        id: i32,
        update: NativeMotion,
        sequence: u64,
    ) {
        if let Some(spawn) = self.0.get_mut(&id) {
            spawn.motion.receive(version, update, sequence);
        }
    }
    pub(crate) fn capture_motion(
        &self,
        session: SessionStamp,
        target: EntityId,
        receive_sequence: u64,
    ) -> Result<EntityMotionObservation> {
        self.validate(session, target)?;
        let spawn = &self.0[&target.native_id];
        let motion = &spawn.motion;
        Ok(EntityMotionObservation {
            entity: spawn.capture(target),
            receive_sequence,
            position: motion.position.clone(),
            rotation: motion.rotation.clone(),
            head_yaw: motion.head_yaw.clone(),
            velocity: motion.velocity.clone(),
            on_ground: motion.on_ground.clone(),
            correction: motion.correction.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(version: MinecraftVersion) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 7,
            world_generation: 9,
        }
    }
    fn spawn(
        ledger: &mut SpawnLedger,
        version: MinecraftVersion,
        position: [f64; 3],
        sequence: u64,
    ) -> EntityId {
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 42,
                    uuid: Some([7; 16]),
                    type_id: None,
                    dedicated_type_name: Some("minecart"),
                    position,
                },
                sequence,
                2,
            )
            .unwrap();
        ledger.initialize_motion(
            42,
            NativeSpawnMotion {
                position: Some(position),
                ..Default::default()
            },
            sequence,
        );
        ledger.identity(session(version), 42).unwrap()
    }
    #[test]
    fn original_entity_motion_relative_codecs_match_all_native_samples() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../../data/client_api/entity_motion_packets.json"
        ))
        .unwrap();
        for data in corpus["versions"].as_array().unwrap() {
            let version = if data["version"] == "1.16.1" {
                MinecraftVersion::Java1_16_1
            } else {
                MinecraftVersion::Java1_21_11
            };
            if version == MinecraftVersion::Java1_16_1 {
                assert_eq!(data["legacy_base_payload_hex"], "2a");
            }
            let rows = data["relative_positions"].as_array().unwrap();
            assert_eq!(rows.len(), 64);
            for row in rows {
                let base: [f64; 3] = serde_json::from_value(row["base"].clone()).unwrap();
                let delta: [i16; 3] = serde_json::from_value(row["delta"].clone()).unwrap();
                let expected: [f64; 3] = serde_json::from_value(row["position"].clone()).unwrap();
                let mut ledger = SpawnLedger::default();
                let target = spawn(&mut ledger, version, base, 10);
                ledger.receive_motion(
                    version,
                    42,
                    NativeMotion::Relative {
                        delta,
                        rotation: None,
                        ground: false,
                    },
                    20,
                );
                let state = ledger.capture_motion(session(version), target, 25).unwrap();
                let position = state.position.unwrap();
                for (actual, expected) in position.value.position.into_iter().zip(expected) {
                    // JSON readers can differ by one ULP on large exact binary fractions.
                    assert!(
                        (actual - expected).abs() <= 2.0 * f64::EPSILON * expected.abs().max(1.0),
                        "{version:?} {row}: {actual} vs {expected}"
                    );
                }
                assert_eq!(
                    position.source,
                    super::super::super::ValueSource::Received { sequence: 20 }
                );
                assert_eq!(position.value.quantization_error, [1.0 / 4096.0; 3]);
                assert_eq!(state.entity.spawn_position.value, base);
                assert!(state.velocity.is_none());
                assert!(state.rotation.is_none());
            }
        }
    }
    #[test]
    fn entity_motion_field_receipts_are_independent_and_retired_with_the_spawn() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut ledger = SpawnLedger::default();
            ledger.receive_motion(version, 42, NativeMotion::Velocity([99.0; 3]), 8);
            let target = spawn(&mut ledger, version, [1.5, 65.0, 2.5], 10);
            ledger.receive_motion(version, 42, NativeMotion::Velocity([0.25, 0.0, 0.0]), 12);
            ledger.receive_motion(
                version,
                42,
                NativeMotion::Rotation {
                    rotation: [90.0, 0.0],
                    ground: true,
                },
                13,
            );
            ledger.receive_motion(version, 42, NativeMotion::HeadYaw(45.0), 14);
            let state = ledger.capture_motion(session(version), target, 18).unwrap();
            assert_eq!(
                state.position.unwrap().source,
                super::super::super::ValueSource::Received { sequence: 10 }
            );
            assert_eq!(
                state.velocity.unwrap(),
                super::super::super::received([0.25, 0.0, 0.0], 12)
            );
            assert_eq!(
                state.head_yaw.unwrap(),
                super::super::super::received(45.0, 14)
            );
            ledger.remove(42);
            assert!(ledger.capture_motion(session(version), target, 20).is_err());
            let fresh = spawn(&mut ledger, version, [3.5, 65.0, 2.5], 21);
            assert!(ledger.capture_motion(session(version), target, 22).is_err());
            assert!(
                ledger
                    .capture_motion(session(version), fresh, 22)
                    .unwrap()
                    .velocity
                    .is_none()
            );
            let mut other = session(version);
            other.world_generation += 1;
            assert!(ledger.capture_motion(other, fresh, 22).is_err());
        }
    }
    #[test]
    fn entity_motion_relative_correction_does_not_use_historical_pose_or_reset_codec() {
        let version = MinecraftVersion::Java1_21_11;
        let mut ledger = SpawnLedger::default();
        let target = spawn(&mut ledger, version, [1.5, 65.0, 2.5], 10);
        ledger.receive_motion(version, 42, NativeMotion::Velocity([9.0; 3]), 12);
        let change = EntityPositionCorrection {
            position: [7.0; 3],
            delta: [0.5; 3],
            rotation: [45.0, 10.0],
            flags: 511,
        };
        ledger.receive_motion(
            version,
            42,
            NativeMotion::Correction {
                change,
                ground: false,
            },
            14,
        );
        let state = ledger.capture_motion(session(version), target, 15).unwrap();
        assert!(state.position.is_none());
        assert!(state.rotation.is_none());
        assert!(state.velocity.is_none());
        assert_eq!(state.correction.unwrap().value, change);
        ledger.receive_motion(
            version,
            42,
            NativeMotion::Relative {
                delta: [4096, 0, 0],
                rotation: None,
                ground: true,
            },
            16,
        );
        assert_eq!(
            ledger
                .capture_motion(session(version), target, 17)
                .unwrap()
                .position
                .unwrap()
                .value
                .position,
            [2.5, 65.0, 2.5]
        );
        ledger.receive_motion(
            version,
            42,
            NativeMotion::Absolute {
                position: [10.0; 3],
                rotation: [0.0; 2],
                velocity: Some([0.0; 3]),
                ground: Some(false),
                reset_base: true,
            },
            18,
        );
        ledger.receive_motion(
            version,
            42,
            NativeMotion::Relative {
                delta: [4096, 0, 0],
                rotation: None,
                ground: true,
            },
            19,
        );
        assert_eq!(
            ledger
                .capture_motion(session(version), target, 20)
                .unwrap()
                .position
                .unwrap()
                .value
                .position,
            [11.0, 10.0, 10.0]
        );
    }
    #[test]
    fn entity_motion_rotate_delta_without_relative_velocity_retains_absolute_sample() {
        // Original Correction::resolve needs a velocity baseline only when a
        // DELTA_X/Y/Z flag is set, even with ROTATE_DELTA present.
        let version = MinecraftVersion::Java1_21_11;
        let mut ledger = SpawnLedger::default();
        let target = spawn(&mut ledger, version, [1.5, 65.0, 2.5], 10);
        let change = EntityPositionCorrection {
            position: [3.0, 65.0, 2.5],
            delta: [0.25, 0.0, 0.0],
            rotation: [90.0, 0.0],
            flags: 256,
        };
        ledger.receive_motion(
            version,
            42,
            NativeMotion::Correction {
                change,
                ground: false,
            },
            12,
        );
        let state = ledger.capture_motion(session(version), target, 13).unwrap();
        assert_eq!(
            state.velocity.unwrap(),
            super::super::super::received(change.delta, 12)
        );
        assert_eq!(state.position.unwrap().value.position, change.position);
    }
    #[test]
    fn entity_motion_painting_anchor_does_not_create_an_unreceived_feet_baseline() {
        let version = MinecraftVersion::Java1_16_1;
        let mut ledger = SpawnLedger::default();
        ledger
            .insert(
                version,
                NativeSpawn {
                    id: 42,
                    uuid: Some([7; 16]),
                    type_id: None,
                    dedicated_type_name: Some("painting"),
                    position: [1.0, 65.0, 2.0],
                },
                10,
                2,
            )
            .unwrap();
        ledger.initialize_motion(42, NativeSpawnMotion::default(), 10);
        let target = ledger.identity(session(version), 42).unwrap();
        ledger.receive_motion(
            version,
            42,
            NativeMotion::Relative {
                delta: [1, 2, 3],
                rotation: None,
                ground: false,
            },
            12,
        );
        let state = ledger.capture_motion(session(version), target, 13).unwrap();
        assert!(state.position.is_none());
        assert!(state.velocity.is_none());
        assert_eq!(state.entity.spawn_position.value, [1.0, 65.0, 2.0]);
    }
}
