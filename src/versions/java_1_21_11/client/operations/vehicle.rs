//! Owned native Input dispatch; history remains readable while writer waits.
use super::*;
use crate::client::{
    self as api,
    inventory::unavailable,
    vehicle::{
        MountId,
        dismount::{self as contract, DismountId, DismountRecord},
    },
};
fn capture(state: &State, player: &api::PlayerObservation) -> api::VehicleObservation {
    state.vehicles.capture(
        player.session,
        state.sequence,
        state.operations.local_player.entity_id,
        &state.entities,
    )
}
pub(in crate::versions::java_1_21_11::client) fn context_received(state: &mut State) {
    let previous = state
        .dismount_history
        .lock()
        .expect("dismount history")
        .clone();
    let Some(previous) = previous.filter(|r| r.unresolved()) else {
        return;
    };
    let player = common_player_in_state(state, previous.id.mount().session().connection_id, false);
    let vehicle = player.as_ref().ok().map(|p| capture(state, p));
    let mut history = state.dismount_history.lock().expect("dismount history");
    if let Some(record) = history.as_mut() {
        if !state.ready || state.failure.is_some() || !matches!(state.phase, Phase::Play) {
            record.inspection("dismount native session unavailable");
        } else {
            match (player, vehicle) {
                (Ok(player), Some(vehicle)) => contract::receive(record, &player, &vehicle),
                (Err(e), _) => record.inspection(e),
                _ => record.inspection("dismount capture unavailable"),
            }
        }
    }
}
impl Operations {
    fn dismount_snapshot(&self, id: DismountId) -> Result<DismountRecord> {
        self.bot
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .filter(|r| r.id == id)
            .cloned()
            .ok_or_else(|| unavailable("dismount intent superseded or belongs to another Client"))
    }
    pub(crate) async fn common_dismount(
        &self,
        mode: api::GameMode,
        mount: MountId,
    ) -> Result<DismountRecord> {
        let state = self.bot.session.state.lock().await;
        self.mutable_for_dismount(&state)?;
        let player = self.common_player_unlocked(&state)?;
        let vehicle = capture(&state, &player);
        let record = {
            let mut history = self.bot.dismount_history.lock().expect("dismount history");
            let record = contract::prepare(&player, vehicle, mode, mount, history.as_ref())?;
            *history = Some(record.clone());
            record
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.bot.clone();
        let id = record.id;
        tokio::spawn(async move {
            let _ = reply.send(bot.operations().dismount_send_owned(id, false).await);
        });
        drop(state);
        result
            .await
            .map_err(|_| unavailable("dismount owner result unavailable"))?
    }
    pub(crate) async fn common_complete_dismount(
        &self,
        mode: api::GameMode,
        id: DismountId,
    ) -> Result<DismountRecord> {
        let state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        let record = self.dismount_snapshot(id)?;
        if record.mode != mode || record.release_claimed {
            return Err(unavailable(
                "dismount release already claimed or wrong handle mode",
            ));
        }
        let player = self.common_player_unlocked(&state)?;
        let vehicle = capture(&state, &player);
        contract::validate_before(&record, &player, &vehicle, true)?;
        self.bot
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_mut()
            .expect("retained")
            .release_claimed = true;
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.bot.clone();
        tokio::spawn(async move {
            let _ = reply.send(bot.operations().dismount_send_owned(id, true).await);
        });
        drop(state);
        result
            .await
            .map_err(|_| unavailable("dismount release owner result unavailable"))?
    }
    async fn dismount_send_owned(&self, id: DismountId, release: bool) -> Result<DismountRecord> {
        let state = self.bot.session.state.lock().await;
        let result = async {
            self.ready(&state)?;
            let record = self.dismount_snapshot(id)?;
            let player = self.common_player_unlocked(&state)?;
            let vehicle = capture(&state, &player);
            contract::validate_before(&record, &player, &vehicle, release)?;
            if !release {
                self.bot
                    .dismount_history
                    .lock()
                    .expect("dismount history")
                    .as_mut()
                    .expect("retained")
                    .after_sequence = state.sequence;
            }
            let (packet, payload) = contract::payload(id.mount().session().version, release);
            self.bot.session.send(packet, &payload).await?;
            self.bot
                .dismount_history
                .lock()
                .expect("dismount history")
                .as_mut()
                .expect("retained")
                .sent(release);
            self.dismount_snapshot(id)
        }
        .await;
        if let Err(e) = &result {
            if let Some(record) = self
                .bot
                .dismount_history
                .lock()
                .expect("dismount history")
                .as_mut()
            {
                record.inspection(format!("dismount input uncertain: {e}"));
            }
        }
        result
    }
    pub(crate) async fn common_dismount_record(&self) -> Result<Option<DismountRecord>> {
        if let Ok(mut state) = self.bot.session.state.try_lock() {
            context_received(&mut state);
        }
        let mut history = self.bot.dismount_history.lock().expect("dismount history");
        if self.bot.session.stopped.load(Ordering::Acquire) {
            if let Some(record) = history.as_mut() {
                record.inspection("dismount connection closed or uncertain");
            }
        }
        Ok(history.clone())
    }
}
