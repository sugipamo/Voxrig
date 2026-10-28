use super::*;
use crate::client::{
    adapter::DigOps,
    operational_dig::{self as dig, Action, Dispatch, Inputs},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
impl Operations {
    fn dig_snapshot_unlocked(&self, state: &mut State, target: [i32; 3]) -> Result<Inputs> {
        let player = self.common_player_unlocked(state)?;
        let (eye, _) = dig::eye_cell(&player)?;
        let (on_ground, vertical_velocity) = if let Some(frame) = state
            .control
            .session
            .as_ref()
            .filter(|s| s.running())
            .and_then(|s| s.frame.as_ref())
        {
            (frame.on_ground, Some(frame.velocity[1]))
        } else {
            let tick = self.bot.session.started.elapsed().as_millis() as u64 / 50;
            (
                survival::context(state, self.bot.session.id, tick)?.on_ground,
                state.operations.local_player.velocity.map(|v| v.value[1]),
            )
        };
        let block = |at: [i32; 3]| -> Result<crate::NativeBlockState> {
            crate::versions::java_1_21_11::native_state(
                state
                    .world
                    .block(at)
                    .ok_or_else(|| dig::unavailable("dig cells are not loaded"))?,
            )
        };
        dig::inputs(
            player,
            block(target)?,
            on_ground,
            vertical_velocity,
            block(eye)?,
            block([eye[0], eye[1] + 1, eye[2]])?,
        )
    }
}
impl DigOps for Operations {
    fn dig_work(&self) -> Result<crate::client::ledger::WorkGuard> {
        self.bot.session.ledger.try_work()
    }
    fn dig_coordinator(&self) -> Arc<dig::Coordinator> {
        self.bot.session.operational_dig.clone()
    }
    async fn dig_snapshot(&self, target: [i32; 3]) -> Result<Inputs> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        self.dig_snapshot_unlocked(&mut state, target)
    }
    async fn dig_dispatch(
        &self,
        initial: &Inputs,
        target: [i32; 3],
        face: crate::client::BlockFace,
        action: Action,
        cancelled: &AtomicBool,
    ) -> Result<Dispatch> {
        let mut state = self.bot.session.state.lock().await;
        let player = self.common_player_unlocked(&state)?;
        let same = player.session == initial.player.session
            && player.game_mode == Some(GameMode::Survival);
        if matches!(action, Action::Abort) {
            if !same {
                return Ok(Dispatch::ContextChanged);
            }
            self.ready(&state)?;
            if self.bot.session.interrupted_packet.load(Ordering::Acquire) >= 0 {
                return Err(dig::unavailable("dig cleanup dispatch remains unresolved"));
            }
        } else {
            if cancelled.load(Ordering::Acquire) {
                return Err(dig::unavailable("dig caller cancelled before dispatch"));
            }
            if !same {
                return Err(dig::unavailable(
                    "dig original world or mode changed before dispatch",
                ));
            }
            self.mutable(&state)?;
            let target_now = state
                .world
                .block(target)
                .map(crate::versions::java_1_21_11::native_state)
                .transpose()?;
            if target_now.as_ref() != Some(&initial.state) {
                return Ok(Dispatch::Changed(target_now));
            }
            let current = self.dig_snapshot_unlocked(&mut state, target)?;
            dig::validate(initial, &current, action)?;
            check_reach(&state, target)?;
        }
        let sequence = self.next_sequence()?;
        let after = state.sequence;
        crate::client::dig::DigWatch::sent_into(
            &mut state.dig_watch,
            action,
            target,
            &initial.state,
            after,
            Some(sequence),
        );
        let mut payload = Vec::new();
        put_varint(&mut payload, action.status());
        payload.extend(pack_position(target).to_be_bytes());
        payload.push(face as u8);
        put_varint(&mut payload, sequence);
        self.bot
            .session
            .send(ids::play_serverbound::BLOCK_DIG, &payload)
            .await?;
        Ok(Dispatch::Sent(Some(sequence)))
    }
    async fn dig_dispatch_tracked(
        &self,
        initial: &Inputs,
        target: [i32; 3],
        face: crate::client::BlockFace,
        action: Action,
        operation: &Arc<crate::client::dig_operation::OperationState>,
    ) -> Result<Dispatch> {
        // Match the existing state -> writer lock order. The state remains held
        // while queued, and every final check is repeated under the writer lock.
        let mut state = self.bot.session.state.lock().await;
        let work = crate::client::ledger::ScopedWork::new(self.bot.session.ledger.try_work()?);
        let mut writer = self.bot.session.writer.lock().await;
        self.bot.session.check_outbound()?;
        let player = self.common_player_unlocked(&state)?;
        if player.session != initial.player.session || player.game_mode != Some(GameMode::Survival)
        {
            operation.context_changed();
            if matches!(action, Action::Abort) {
                return Ok(Dispatch::ContextChanged);
            }
            return Err(dig::unavailable(
                "dig original world or mode changed before dispatch",
            ));
        }
        if matches!(action, Action::Abort) {
            self.ready(&state)?;
            if self.bot.session.interrupted_packet.load(Ordering::Acquire) >= 0 {
                return Err(dig::unavailable("dig cleanup dispatch remains unresolved"));
            }
        } else {
            self.mutable(&state)?;
            let target_now = state
                .world
                .block(target)
                .map(crate::versions::java_1_21_11::native_state)
                .transpose()?;
            if target_now.as_ref() != Some(&initial.state) {
                if operation.cancelled.load(Ordering::Acquire) {
                    operation.cooperative();
                    return Ok(Dispatch::Cancelled);
                }
                return Ok(Dispatch::Changed(target_now));
            }
            let current = self.dig_snapshot_unlocked(&mut state, target)?;
            dig::validate(initial, &current, action)?;
            check_reach(&state, target)?;
        }
        let sequence = self.next_sequence()?;
        let after = state.sequence;
        crate::client::dig::DigWatch::sent_into(
            &mut state.dig_watch,
            action,
            target,
            &initial.state,
            after,
            Some(sequence),
        );
        let mut payload = Vec::new();
        put_varint(&mut payload, action.status());
        payload.extend(pack_position(target).to_be_bytes());
        payload.push(face as u8);
        put_varint(&mut payload, sequence);
        let compression = writer.compression;
        self.bot
            .session
            .write_dig_frame(
                &mut writer.stream,
                compression,
                &payload,
                work.guard(),
                operation,
                action,
                sequence,
            )
            .await?;
        Ok(Dispatch::Sent(Some(sequence)))
    }
    async fn dig_rejection(&self) -> Result<Option<crate::client::DigRejection>> {
        let state = self.bot.session.state.lock().await;
        let ack = state
            .operations
            .ack
            .zip(state.operations.ack_receive_sequence);
        Ok(state.dig_watch.as_ref().and_then(|w| w.rejection(ack)))
    }
}
