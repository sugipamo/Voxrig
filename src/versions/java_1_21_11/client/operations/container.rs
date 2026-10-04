//! Actual opening-bound close sends. No local close is fabricated as a received packet.
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
