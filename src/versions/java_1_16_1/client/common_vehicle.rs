//! Actor-owned mounted-input dispatch, separate from passenger receipts.
use super::*;
use crate::client::{
    self as api,
    inventory::unavailable,
    vehicle::{
        MountId,
        dismount::{self as contract, DismountId, DismountRecord},
    },
};
impl Bot {
    async fn vehicle_capture_unlocked(
        &self,
    ) -> Result<(api::PlayerObservation, api::VehicleObservation)> {
        let player = self.common_player_unlocked().await?;
        let own = self.player.lock().await.entity_id;
        let receipts = self.common_receipts.lock().await;
        let vehicle = receipts.vehicles.capture(
            player.session,
            player.receive_sequence,
            own,
            &receipts.entities,
        );
        Ok((player, vehicle))
    }
    fn dismount_snapshot(&self, id: DismountId) -> Result<DismountRecord> {
        self.dismount_history
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
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if self.connection_state() != ConnectionState::Ready
            || self.control().await != ControlState::default()
        {
            return Err(unavailable(
                "dismount requires ready connection and released legacy controls",
            ));
        }
        let (player, vehicle) = self.vehicle_capture_unlocked().await?;
        let record = {
            let mut history = self.dismount_history.lock().expect("dismount history");
            let record = contract::prepare(&player, vehicle, mode, mount, history.as_ref())?;
            *history = Some(record.clone());
            record
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        let id = record.id;
        tokio::spawn(async move {
            let _ = reply.send(bot.dismount_send_owned(id, false).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| unavailable("dismount owner result unavailable"))?
    }
    pub(crate) async fn common_complete_dismount(
        &self,
        mode: api::GameMode,
        id: DismountId,
    ) -> Result<DismountRecord> {
        let gate = self.coherent_state_gate.lock().await;
        let record = self.dismount_snapshot(id)?;
        if record.mode != mode || record.release_claimed {
            return Err(unavailable(
                "dismount release already claimed or wrong handle mode",
            ));
        }
        let (player, vehicle) = self.vehicle_capture_unlocked().await?;
        contract::validate_before(&record, &player, &vehicle, true)?;
        self.dismount_history
            .lock()
            .expect("dismount history")
            .as_mut()
            .expect("retained")
            .release_claimed = true;
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.dismount_send_owned(id, true).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| unavailable("dismount release owner result unavailable"))?
    }
    async fn dismount_send_owned(&self, id: DismountId, release: bool) -> Result<DismountRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let record = self.dismount_snapshot(id)?;
            let (player, vehicle) = self.vehicle_capture_unlocked().await?;
            if self.connection_state() != ConnectionState::Ready
                || self.control().await != ControlState::default()
            {
                return Err(unavailable(
                    "dismount native connection/control ownership changed",
                ));
            }
            contract::validate_before(&record, &player, &vehicle, release)?;
            if !release {
                self.dismount_history
                    .lock()
                    .expect("dismount history")
                    .as_mut()
                    .expect("retained")
                    .after_sequence = player.receive_sequence;
            }
            let (packet, payload) = contract::payload(id.mount().session().version, release);
            self.send(packet, &payload).await?;
            self.dismount_history
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
    pub(super) async fn common_dismount_context_received(&self) {
        let active = self
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .is_some_and(|r| r.unresolved());
        if !active {
            return;
        }
        let capture = self.vehicle_capture_unlocked().await;
        let mut history = self.dismount_history.lock().expect("dismount history");
        if let Some(record) = history.as_mut() {
            match capture {
                Ok((player, vehicle)) => contract::receive(record, &player, &vehicle),
                Err(e) => record.inspection(e),
            }
        }
    }
    pub(crate) async fn common_dismount_record(&self) -> Result<Option<DismountRecord>> {
        if let Ok(_gate) = self.coherent_state_gate.try_lock() {
            self.common_dismount_context_received().await;
        }
        let mut history = self.dismount_history.lock().expect("dismount history");
        if self.is_stopped() {
            if let Some(record) = history.as_mut() {
                record.inspection("dismount connection closed");
            }
        }
        Ok(history.clone())
    }
}
