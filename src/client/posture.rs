//! Received posture inputs and explicitly computed viewpoints.
use super::{EntityPose, ObservedValue, PlayerObservation, SessionStamp, ValueSource};
use crate::MinecraftVersion;

/// A calculated local-player eye height with the receipt inputs kept separately.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlayerEyeHeight {
    /// Native float height above feet. Always Predicted, never a packet field.
    pub height: ObservedValue<f32>,
    /// Explicit original pose metadata used by this calculation.
    pub pose: ObservedValue<i32>,
    /// Received effective scale for modern scalable poses; absent for legacy
    /// and fixed sleeping/dying dimensions. No missing modern default is invented.
    pub scale: Option<ObservedValue<super::PlayerAttribute>>,
}

/// Own-player posture at one coherent adapter boundary.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlayerPostureObservation {
    /// Owning connection/world.
    pub session: SessionStamp,
    /// Capture receive boundary, independent of individual field receipt ordinals.
    pub receive_sequence: u64,
    /// Exact native pose metadata. Absent until actually received in this world.
    pub pose: Option<ObservedValue<i32>>,
    /// Shared entity flag 0x10. Independent of pose: crawling can use the swimming
    /// pose while this flag is false. This is the last receipt, not server-current state.
    pub swimming: Option<ObservedValue<bool>>,
    /// Unscaled pose constant, calculated from the explicit received pose.
    /// Available even before a modern scale attribute receipt; always Predicted.
    pub unscaled_eye_height: Option<ObservedValue<f32>>,
    /// Version-correct calculated height, absent for missing/unknown pose or scale.
    pub eye_height: Option<PlayerEyeHeight>,
}
impl PlayerPostureObservation {
    /// Common name of the explicitly received native pose; unknown IDs remain raw.
    pub fn entity_pose(&self) -> Option<EntityPose> {
        EntityPose::from_native(self.session.version, self.pose.as_ref()?.value)
    }
}
impl EntityPose {
    /// Unscaled original player eye height for this version's pose. Sleeping is
    /// 0.2; fall flying, swimming/crawling and spin attack are 0.4; crouching 1.27;
    /// other valid player poses use 1.62. An enum absent in the version returns None.
    pub fn player_eye_height(self, version: MinecraftVersion) -> Option<f32> {
        use EntityPose::*;
        match self {
            Sleeping => Some(0.2),
            FallFlying | Swimming | SpinAttack => Some(0.4),
            Crouching => Some(1.27),
            Standing | Dying => Some(1.62),
            _ if version == MinecraftVersion::Java1_21_11 => Some(1.62),
            _ => None,
        }
    }
}
pub(crate) fn capture(
    player: &PlayerObservation,
    pose: Option<ObservedValue<i32>>,
    swimming: Option<ObservedValue<bool>>,
) -> PlayerPostureObservation {
    let version = player.session.version;
    let unscaled_eye_height = pose
        .as_ref()
        .and_then(|pose| EntityPose::from_native(version, pose.value))
        .and_then(|pose| pose.player_eye_height(version))
        .map(|value| ObservedValue {
            value,
            source: ValueSource::Predicted,
        });
    let eye_height = (|| {
        let pose = pose.as_ref()?;
        let common = EntityPose::from_native(version, pose.value)?;
        let mut height = common.player_eye_height(version)?;
        let fixed = matches!(common, EntityPose::Sleeping | EntityPose::Dying);
        let scale = if version == MinecraftVersion::Java1_21_11 && !fixed {
            let scale = player.attributes.get("minecraft:scale")?;
            if !scale.value.value.is_finite()
                || !matches!(scale.source, ValueSource::Received { .. })
            {
                return None;
            }
            height *= scale.value.value.clamp(0.0625, 16.) as f32;
            Some(scale.clone())
        } else {
            None
        };
        Some(PlayerEyeHeight {
            height: ObservedValue {
                value: height,
                source: ValueSource::Predicted,
            },
            pose: pose.clone(),
            scale,
        })
    })();
    PlayerPostureObservation {
        session: player.session,
        receive_sequence: player.receive_sequence,
        pose,
        swimming,
        unscaled_eye_height,
        eye_height,
    }
}
impl crate::Client {
    /// Received own pose/swimming flag and derived eye height. Neither supported
    /// protocol sends an own `in_water` boolean: use received block properties to
    /// inspect fluid, and keep the control model's water intersection separate.
    pub async fn player_posture(&self) -> crate::Result<PlayerPostureObservation> {
        super::dispatch!(&self.adapter, a => super::adapter::CoreOps::player_posture(a).await)
    }
}

impl super::control::ControlFrame {
    /// Numerical water intersection, explicitly labeled as predicted.
    pub fn in_water_observation(&self) -> crate::client::ObservedValue<bool> {
        crate::client::ObservedValue {
            value: self.in_water,
            source: crate::client::ValueSource::Predicted,
        }
    }
    /// Numerical swimming state, independent of the received shared entity flag
    /// available through `Client::player_posture`.
    pub fn swimming_observation(&self) -> crate::client::ObservedValue<bool> {
        crate::client::ObservedValue {
            value: self.swimming,
            source: crate::client::ValueSource::Predicted,
        }
    }
    /// Numerical eye-water intersection for this model's pose.
    pub fn eye_in_water_observation(&self) -> crate::client::ObservedValue<bool> {
        crate::client::ObservedValue {
            value: self.eye_in_water,
            source: crate::client::ValueSource::Predicted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{InventoryObservation, PlayerAttribute, received};
    fn player(version: MinecraftVersion) -> PlayerObservation {
        PlayerObservation {
            session: SessionStamp {
                version,
                connection_id: 1,
                world_generation: 0,
            },
            receive_sequence: 20,
            pending_dispatch: false,
            dimension: None,
            position: None,
            received_pose: None,
            rotation: [0.; 2],
            rotation_source: None,
            on_ground: None,
            game_mode: None,
            may_fly: None,
            health: None,
            selected_hotbar: None,
            inventory: InventoryObservation::default(),
            using_item: None,
            fall_flying: None,
            entity_id: None,
            attributes: Default::default(),
            effects: Default::default(),
            air_supply: None,
            world_time: None,
        }
    }
    #[test]
    fn posture_retains_exact_receipts_and_never_synthesizes_pose_or_modern_scale() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut player = player(version);
            let absent = capture(&player, None, None);
            assert!(
                absent.pose.is_none() && absent.swimming.is_none() && absent.eye_height.is_none()
            );
            let unknown = capture(&player, Some(received(99, 3)), Some(received(false, 4)));
            assert_eq!(unknown.pose.unwrap().value, 99);
            assert!(unknown.eye_height.is_none());
            if version == MinecraftVersion::Java1_21_11 {
                let without_scale = capture(&player, Some(received(0, 3)), None);
                assert!(without_scale.eye_height.is_none());
                assert_eq!(
                    without_scale.unscaled_eye_height.unwrap(),
                    ObservedValue {
                        value: 1.62,
                        source: ValueSource::Predicted
                    }
                );
                player.attributes.insert(
                    "minecraft:scale".into(),
                    received(
                        PlayerAttribute {
                            base: 2.,
                            modifiers: vec![],
                            value: 2.,
                        },
                        5,
                    ),
                );
            }
            for (id, eye) in [
                (0, 1.62_f32),
                (1, 0.4),
                (2, 0.2),
                (3, 0.4),
                (4, 0.4),
                (5, 1.27),
            ] {
                let observed = capture(&player, Some(received(id, 6)), Some(received(false, 7)));
                assert_eq!(
                    observed.pose.as_ref().unwrap().source,
                    ValueSource::Received { sequence: 6 }
                );
                assert_eq!(observed.swimming, Some(received(false, 7)));
                assert_eq!(
                    observed.unscaled_eye_height.unwrap(),
                    ObservedValue {
                        value: eye,
                        source: ValueSource::Predicted
                    }
                );
                let height = observed.eye_height.unwrap();
                let scale = if version == MinecraftVersion::Java1_21_11 && id != 2 {
                    2.
                } else {
                    1.
                };
                assert_eq!(height.height.value, eye * scale);
                assert_eq!(height.height.source, ValueSource::Predicted);
                assert_eq!(height.pose, received(id, 6));
                assert_eq!(height.scale.is_some(), scale == 2.);
            }
            // Protocol world reset must return absence again, independent of old numeric values.
            player.session.world_generation += 1;
            assert!(capture(&player, None, None).eye_height.is_none());
        }
        assert_eq!(
            EntityPose::LongJumping.player_eye_height(MinecraftVersion::Java1_16_1),
            None
        );
    }
}
