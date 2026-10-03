//! Common bounded preview with legacy-native rules and coherent legacy admission.
use super::*;
use crate::client::{
    GameMode,
    survival::{MotionPreview, SurvivalControl, TerminalClearance, model},
};

impl Bot {
    pub(crate) async fn common_preview_path(
        &self,
        controls: &[SurvivalControl],
    ) -> Result<MotionPreview> {
        model::validate_controls(controls)?;
        let _gate = self.coherent_state_gate.lock().await;
        if self.connection_state() != ConnectionState::Ready {
            return Err(motion_state("connection not ready"));
        }
        let initial = self.common_player_unlocked().await?;
        if initial.game_mode != Some(GameMode::Survival)
            || !initial
                .health
                .as_ref()
                .is_some_and(|health| health.value.health > 0.0)
            || *self.local_pose.lock().await != Some(0)
        {
            return Err(motion_state(
                "dry preview requires healthy received survival mode and native standing pose",
            ));
        }
        let survival = self.survival.read().await;
        if survival.flying
            || !survival.effects.is_empty()
            || survival.attributes.values().any(|attribute| {
                matches!(
                    attribute.key.as_str(),
                    "minecraft:generic.movement_speed" | "generic.movement_speed"
                ) && attribute.value() as f32 != 0.1f32
            })
        {
            return Err(motion_state(
                "dry preview requires native default motion and no received flight/effects",
            ));
        }
        drop(survival);
        if self.control().await != ControlState::default() {
            return Err(motion_state(
                "persistent legacy controls must be released before a bounded preview",
            ));
        }
        let player = self.player.lock().await.clone();
        let motion = **self.motion.lock().await;
        if !player.on_ground
            || motion.velocity.x != 0.0
            || motion.velocity.z != 0.0
            || (motion.velocity.y != 0.0
                && (motion.velocity.y + 0.08 * f64::from(0.98f32)).abs() > 1e-8)
        {
            return Err(motion_state(
                "dry preview requires locally stationary grounded context",
            ));
        }
        let position = initial
            .position
            .as_ref()
            .ok_or_else(|| motion_state("position unavailable"))?
            .value;
        let mut model = model::Model::new(crate::MinecraftVersion::Java1_16_1, position);
        model.frame.velocity = [motion.velocity.x, motion.velocity.y, motion.velocity.z];
        let initial_frame = model.initial_frame();
        let world = self.world.lock().await;
        let block_at = |position| legacy_motion_block(&world, position);
        // Check the whole standing body and support even for released inputs.
        legacy_clearance(&block_at, position, 0.0)?;
        let frames = model::predict(&block_at, &mut model, controls)?;
        let terminal = frames.last().expect("bounded nonempty controls");
        let terminal_clearance = if terminal.resting {
            match legacy_clearance(&block_at, terminal.position, 1.0 / 16.0) {
                Ok(()) => TerminalClearance::Admitted {
                    horizontal_margin: 1.0 / 16.0,
                },
                Err(error) => TerminalClearance::RequiresReplan {
                    reason: error.to_string(),
                },
            }
        } else {
            TerminalClearance::RequiresReplan {
                reason: "terminal motion must be released and resting".into(),
            }
        };
        Ok(MotionPreview {
            initial,
            world_revision: world.revision(),
            initial_frame,
            controls: controls.to_vec(),
            frames,
            terminal_clearance,
        })
    }
}
fn legacy_motion_block(
    world: &super::World,
    position: [i32; 3],
) -> Result<crate::NativeBlockState> {
    if !(0..=255).contains(&position[1])
        || position[0].abs_diff(0) > 30_000_000
        || position[2].abs_diff(0) > 30_000_000
    {
        return Err(motion_state("motion geometry crosses native world bounds"));
    }
    let state = world
        .block(position[0], position[1], position[2])
        .ok_or_else(|| motion_state("motion geometry is not loaded"))?;
    crate::versions::java_1_16_1::native_state(state)
}
fn legacy_clearance(
    block_at: &impl Fn([i32; 3]) -> Result<crate::NativeBlockState>,
    position: [f64; 3],
    margin: f64,
) -> Result<()> {
    let mut bounds = model::body(position);
    bounds[0] -= margin;
    bounds[2] -= margin;
    bounds[3] += margin;
    bounds[5] += margin;
    let cubes = model::geometry(block_at, position, [0.0; 3])?;
    if cubes.iter().any(|cube| {
        (0..3).all(|axis| {
            bounds[axis] + 1e-7 < cube[axis + 3] && bounds[axis + 3] - 1e-7 > cube[axis]
        })
    }) {
        return Err(motion_state(
            "standing body or reserve intersects a dry cube",
        ));
    }
    let supported_area: f64 = cubes
        .iter()
        .filter(|cube| (cube[4] - position[1]).abs() < 1e-7)
        .map(|cube| {
            (bounds[3].min(cube[3]) - bounds[0].max(cube[0])).max(0.0)
                * (bounds[5].min(cube[5]) - bounds[2].max(cube[2])).max(0.0)
        })
        .sum();
    if supported_area + 1e-7 < (bounds[3] - bounds[0]) * (bounds[5] - bounds[2]) {
        return Err(motion_state(
            "standing body and reserve need full known dry support",
        ));
    }
    Ok(())
}
fn motion_state(message: &str) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn common_preview_uses_legacy_rules_without_mutating_player_or_receipts() {
        let (bot, _, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        bot.survival.write().await.game_mode = Some(0);
        *bot.local_pose.lock().await = Some(0);
        let mut health = 20.0f32.to_be_bytes().to_vec();
        put_varint(&mut health, 20);
        health.extend(5.0f32.to_be_bytes());
        bot.apply_packet(0x49, health).await.unwrap();
        {
            let mut player = bot.player.lock().await;
            player.x = 8.5;
            player.y = 65.0;
            player.z = 8.5;
            player.on_ground = true;
        }
        bot.common_receipts.lock().await.pose = Some(crate::client::ReceivedPose {
            position: [8.5, 65.0, 8.5],
            rotation: [0.0; 2],
            receive_sequence: 1,
        });
        bot.world.lock().await.apply_chunk(&[0; 14], 256).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                bot.world
                    .lock()
                    .await
                    .set_block_for_test(BlockPos { x, y: 64, z }, 1);
            }
        }
        let client = crate::Client::from_java_1_16_1(bot.clone());
        crate::client::tests::common_motion_preview_scenario(&client).await;
        *bot.local_pose.lock().await = Some(5);
        assert!(
            client
                .survival()
                .preview_path(&[SurvivalControl {
                    yaw: 0.0,
                    input: Default::default()
                }])
                .await
                .is_err()
        );
        *bot.local_pose.lock().await = Some(0);
        bot.survival.write().await.game_mode = Some(1);
        assert!(
            client
                .survival()
                .preview_path(&[SurvivalControl {
                    yaw: 0.0,
                    input: Default::default()
                }])
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
}
