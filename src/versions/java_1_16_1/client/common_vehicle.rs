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
        self.vehicle_control_context_received().await;
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

impl Bot {
    fn vehicle_control_snapshot(
        &self,
        id: api::VehicleControlId,
    ) -> Result<api::VehicleControlRecord> {
        self.vehicle_control_history
            .lock()
            .expect("vehicle control history")
            .as_ref()
            .filter(|r| r.id == id)
            .cloned()
            .ok_or_else(|| unavailable("vehicle control superseded or wrong Client"))
    }
    pub(crate) async fn common_vehicle_control(
        &self,
        mode: api::GameMode,
        mount: MountId,
        inputs: &[api::VehicleInput],
    ) -> Result<api::VehicleControlRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        if self.connection_state() != ConnectionState::Ready
            || self.control().await != ControlState::default()
        {
            return Err(unavailable(
                "vehicle control requires ready released native controls",
            ));
        }
        let (player, vehicle) = self.vehicle_capture_unlocked().await?;
        let record = {
            let mut history = self
                .vehicle_control_history
                .lock()
                .expect("vehicle control history");
            let record = api::vehicle::control::prepare(
                player,
                vehicle,
                mode,
                mount,
                inputs,
                history.as_ref(),
            )?;
            *history = Some(record.clone());
            record
        };
        let id = record.id;
        let bot = self.clone_internal();
        let (reply, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = reply.send(bot.vehicle_control_send_owned(id).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| unavailable("vehicle control owner result unavailable"))?
    }
    async fn vehicle_control_send_owned(
        &self,
        id: api::VehicleControlId,
    ) -> Result<api::VehicleControlRecord> {
        let inputs = self.vehicle_control_snapshot(id)?.inputs;
        let result = async {
            for (index, input) in inputs.into_iter().enumerate() {
                if index > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                let _gate = self.coherent_state_gate.lock().await;
                if self.connection_state() != ConnectionState::Ready
                    || self.control().await != ControlState::default()
                {
                    return Err(unavailable(
                        "vehicle control connection/native control changed",
                    ));
                }
                let record = self.vehicle_control_snapshot(id)?;
                let (player, vehicle) = self.vehicle_capture_unlocked().await?;
                api::vehicle::control::validate(&record, &player, &vehicle)?;
                if usize::from(record.dispatched_ticks) != index
                    || usize::from(record.attempted_tick) != index
                {
                    return Err(unavailable(
                        "vehicle control frame already claimed or uncertain",
                    ));
                }
                self.vehicle_control_history
                    .lock()
                    .expect("vehicle control history")
                    .as_mut()
                    .unwrap()
                    .attempted_tick = (index + 1) as u16;
                let (packet, payload) =
                    api::vehicle::control::payload(id.mount().session().version, input);
                self.send(packet, &payload).await?;
                let mut history = self
                    .vehicle_control_history
                    .lock()
                    .expect("vehicle control history");
                let record = history
                    .as_mut()
                    .filter(|r| r.id == id)
                    .ok_or_else(|| unavailable("vehicle control owner superseded"))?;
                record.dispatched_ticks = (index + 1) as u16;
                if self.is_stopped() {
                    record.inspection("vehicle control connection closed during write");
                }
                if record.stage != api::VehicleControlStage::Running
                    || record.requires_inspection.is_some()
                {
                    return Err(unavailable("vehicle control interrupted during write"));
                }
                if index + 1 == record.inputs.len() {
                    record.stage = api::VehicleControlStage::Submitted;
                    // Return this owner's result before a subsequent run can replace history.
                    return Ok(record.clone());
                }
            }
            Err(unavailable("vehicle control missing final neutral"))
        }
        .await;
        if let Err(e) = &result {
            if let Some(record) = self
                .vehicle_control_history
                .lock()
                .expect("vehicle control history")
                .as_mut()
                .filter(|r| r.id == id)
            {
                record.inspection(e);
            }
        }
        result
    }
    async fn vehicle_control_context_received(&self) {
        if !api::vehicle::control::unresolved(&self.vehicle_control_history) {
            return;
        }
        let context = self.vehicle_capture_unlocked().await;
        let mut history = self
            .vehicle_control_history
            .lock()
            .expect("vehicle control history");
        if let Some(record) = history.as_mut() {
            match context {
                Ok((player, vehicle)) => api::vehicle::control::receive(record, &player, &vehicle),
                Err(e) => record.inspection(e),
            }
        }
    }
    pub(crate) async fn common_vehicle_control_record(
        &self,
    ) -> Result<Option<api::VehicleControlRecord>> {
        if let Ok(_gate) = self.coherent_state_gate.try_lock() {
            self.vehicle_control_context_received().await;
        }
        let mut history = self
            .vehicle_control_history
            .lock()
            .expect("vehicle control history");
        if self.is_stopped() {
            if let Some(record) = history.as_mut() {
                record.inspection("vehicle control connection closed");
            }
        }
        Ok(history.clone())
    }
}
