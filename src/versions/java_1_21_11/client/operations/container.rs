//! Retained storage activation/close; complete sends never fabricate actual screen receipts.
use super::*;
use crate::client::{self as api, container as contract};

pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let session = api::SessionStamp {
        version: crate::MinecraftVersion::Java1_21_11,
        connection_id: state
            .common_container_close
            .as_ref()
            .map_or(0, |r| r.initial.session.connection_id),
        world_generation: state.loading.generation,
    };
    let inventory = &state.operations.inventory;
    let screen = inventory.container.as_ref().map(|s| s.capture(session).id);
    let cursor = inventory.cursor_sequence.and_then(|s| {
        super::common_slot(&inventory.cursor)
            .ok()
            .map(|v| api::received(v, s))
    });
    if let Some(r) = state.common_container_close.as_mut() {
        r.context_received(session, state.operations.game_mode, screen, cursor.as_ref());
        if !state.ready || !matches!(state.phase, Phase::Play) || state.failure.is_some() {
            r.inspection("close session closed or unavailable");
        }
    }
    open_context_received(state);
}
fn open_context_received(state: &mut State) {
    let Some(mut record) = state.common_container_open.take() else {
        return;
    };
    if record.unresolved() && record.requires_inspection.is_none() {
        let outcome = (|| -> Result<()> {
            if !state.ready || !matches!(state.phase, Phase::Play) || state.failure.is_some() {
                return Err(api::inventory::unavailable(
                    "storage activation session unavailable",
                ));
            }
            let native = survival::context(
                state,
                record.id.session().connection_id,
                state.reconstruction.tick,
            )?;
            movement::validate_initial(&native)?;
            if !native
                .player
                .health
                .as_ref()
                .is_some_and(|h| h.health > 0.0)
            {
                return Err(api::inventory::unavailable(
                    "storage activation player health unavailable",
                ));
            }
            let hit =
                super::super::raycast::stationary_outline_hit(state, native.eye_position, 4.5)?
                    .ok_or_else(|| {
                        api::inventory::unavailable("storage first outline disappeared")
                    })?;
            let cursor =
                super::super::raycast::hit_cursor_in(state.rotation, native.eye_position, &hit);
            if hit.position != record.target.position
                || !record.observe_target_state(&hit.state, state.world.revision, state.sequence)
                || hit.face.map(|f| f as u8) != Some(record.target.face as u8)
                || cursor != record.cursor
            {
                return Err(api::inventory::unavailable(
                    "storage first-outline geometry changed",
                ));
            }
            let inventory = &state.operations.inventory;
            if inventory.unsupported_components {
                return Err(api::inventory::unavailable(
                    "storage inventory components unsupported",
                ));
            }
            let session = api::SessionStamp {
                world_generation: state.loading.generation,
                ..record.initial.session
            };
            let screen = inventory.container.as_ref().map(|s| s.capture(session));
            let basis = contract::player_screen_access(
                session,
                inventory.window_id,
                screen.as_ref().map(|s| s.id),
                state.common_container_close.as_ref(),
            );
            let actual_pose = state
                .motion
                .received_pose
                .as_ref()
                .filter(|p| p.generation == session.world_generation)
                .map(|p| api::ReceivedPose {
                    position: p.position,
                    rotation: p.rotation,
                    receive_sequence: p.receive_sequence,
                });
            let selected = state.operations.selected_hotbar.as_ref().map(|s| s.slot);
            let received_slot = |slot: usize| {
                inventory
                    .slot_sequences
                    .get(slot)
                    .copied()
                    .flatten()
                    .and_then(|s| {
                        super::common_slot(&inventory.slots[slot])
                            .ok()
                            .map(|v| api::received(v, s))
                    })
            };
            let hand = selected
                .filter(|s| *s <= 8)
                .and_then(|s| received_slot(36 + usize::from(s)));
            let offhand = received_slot(45);
            let cursor = inventory.cursor_sequence.and_then(|s| {
                super::common_slot(&inventory.cursor)
                    .ok()
                    .map(|v| api::received(v, s))
            });
            if let (Some(sequence), Some(receive_sequence)) =
                (state.operations.ack, state.operations.ack_receive_sequence)
            {
                if record.send.dispatched && receive_sequence > record.send.after_sequence {
                    record.protocol_processing = Some(contract::ContainerOpenProcessing {
                        acknowledged_sequence: sequence,
                        receive_sequence,
                    });
                }
            }
            record.context_received(contract::open::OpenContext {
                session,
                mode: state.operations.game_mode,
                position: state.position,
                rotation: state.rotation,
                pose: actual_pose.as_ref(),
                selected,
                hand: hand.as_ref(),
                offhand: offhand.as_ref(),
                cursor: cursor.as_ref(),
                player_screen: basis,
                screen: screen.as_ref(),
            });
            Ok(())
        })();
        if let Err(e) = outcome {
            record.inspection(&e);
        }
    }
    state.common_container_open = Some(record);
}
pub(super) fn close_received(state: &mut State, window: i32) {
    let Some(r) = state.common_container_close.as_mut() else {
        return;
    };
    let session = api::SessionStamp {
        world_generation: state.loading.generation,
        ..r.initial.session
    };
    if let Some(s) = state
        .operations
        .inventory
        .container
        .as_ref()
        .filter(|s| s.window == window)
    {
        r.received_close(s.capture(session).id, state.sequence);
    }
}
impl Operations {
    pub(crate) async fn common_open_container(
        &self,
        mode: api::GameMode,
        target: [i32; 3],
    ) -> Result<contract::ContainerOpenRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        let inventory = &state.operations.inventory;
        if inventory.pending_swap.is_some()
            || !inventory.pending_creative.is_empty()
            || inventory.unsupported_components
        {
            return Err(api::inventory::unavailable(
                "native inventory mutation/data unresolved",
            ));
        }
        let query = self.common_target_unlocked(&mut state, mode, 4.5)?;
        let before = state
            .operations
            .inventory
            .container
            .as_ref()
            .map(|s| s.capture(query.initial.session).id);
        let sequence = self.next_sequence()?;
        let record = contract::open::prepare_open(
            query,
            target,
            mode,
            before,
            state.common_container_open.as_ref(),
            Some(sequence),
        )?;
        let mut payload = vec![0];
        payload.extend(pack_position(target).to_be_bytes());
        put_varint(&mut payload, record.target.face as i32);
        for v in record.cursor {
            payload.extend(v.to_be_bytes());
        }
        payload.extend([0, 0]);
        put_varint(&mut payload, sequence);
        state.common_container_open = Some(record);
        // Cancellation before complete send retains Pending and native writer uncertainty.
        match self
            .bot
            .session
            .send(ids::play_serverbound::BLOCK_PLACE, &payload)
            .await
        {
            Ok(()) => state
                .common_container_open
                .as_mut()
                .expect("retained")
                .sent(),
            Err(e) => {
                state
                    .common_container_open
                    .as_mut()
                    .expect("retained")
                    .inspection(&e);
                return Err(e);
            }
        }
        Ok(state
            .common_container_open
            .as_ref()
            .expect("retained")
            .clone())
    }
    pub(crate) async fn common_container_open_record(
        &self,
    ) -> Result<Option<contract::ContainerOpenRecord>> {
        let mut state = self.bot.session.state.lock().await;
        open_context_received(&mut state);
        if self.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
            if let Some(record) = state
                .common_container_open
                .as_mut()
                .filter(|r| r.unresolved())
            {
                record.inspection("storage activation connection closed or uncertain");
            }
        }
        Ok(state.common_container_open.clone())
    }
    pub(crate) async fn common_close_container(
        &self,
        mode: api::GameMode,
        screen: contract::ScreenId,
    ) -> Result<contract::ContainerCloseRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        let inventory = &state.operations.inventory;
        if inventory.pending_swap.is_some()
            || !inventory.pending_creative.is_empty()
            || inventory.unsupported_components
        {
            return Err(api::inventory::unavailable(
                "native inventory data/mutation unresolved",
            ));
        }
        let initial = self.common_player_unlocked(&state)?;
        let captured = inventory
            .container
            .as_ref()
            .map(|s| s.capture(initial.session))
            .ok_or_else(|| api::inventory::unavailable("no received container opening"))?;
        let record = contract::prepare_close(
            initial,
            captured,
            screen,
            mode,
            state.common_container_close.as_ref(),
        )?;
        let mut payload = Vec::new();
        put_varint(&mut payload, screen.window_id());
        state.common_container_close = Some(record);
        // State retains uncertainty if this future is cancelled while waiting for the writer.
        match self
            .bot
            .session
            .send(ids::play_serverbound::CLOSE_WINDOW, &payload)
            .await
        {
            Ok(()) => state
                .common_container_close
                .as_mut()
                .expect("retained")
                .sent(),
            Err(e) => {
                state
                    .common_container_close
                    .as_mut()
                    .expect("retained")
                    .inspection(&e);
                return Err(e);
            }
        }
        Ok(state
            .common_container_close
            .as_ref()
            .expect("retained")
            .clone())
    }
    pub(crate) async fn common_container_close_record(
        &self,
    ) -> Result<Option<contract::ContainerCloseRecord>> {
        let mut state = self.bot.session.state.lock().await;
        context_received(&mut state);
        if self.bot.session.stopped.load(Ordering::Acquire) || state.failure.is_some() {
            if let Some(r) = state.common_container_close.as_mut() {
                r.inspection("close connection closed or uncertain");
            }
        }
        Ok(state.common_container_close.clone())
    }
}
