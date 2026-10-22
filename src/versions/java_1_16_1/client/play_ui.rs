//! Protocol 736 ui packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;
use crate::MinecraftVersion::Java1_16_1;
use crate::versions::ui_decode;

impl Bot {
    pub(super) async fn receive_boss_bar(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let (uuid, update) = ui_decode::decode_boss_bar(Java1_16_1, p)?;
        self.common_boss_bars
            .lock()
            .await
            .apply(uuid, update.clone(), packet_sequence)?;
        self.ui.write().await.project_boss_bar(uuid, update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::BossBar));
        Ok(())
    }

    pub(super) async fn receive_player_info(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let packet = ui_decode::decode_player_list(Java1_16_1, id, p)?;
        self.common_player_list
            .lock()
            .await
            .apply(packet.clone(), packet_sequence)?;
        let mut players = self.players.write().await;
        let (action, uuids) = project_player_info(&mut players, packet);
        drop(players);
        self.emit(Event::PlayerListUpdated { action, uuids });
        Ok(())
    }

    pub(super) async fn receive_world_border(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let generation = self.common_receipts.lock().await.generation;
        let update = ui_decode::decode_display(Java1_16_1, id, p)?;
        self.common_display
            .lock()
            .await
            .apply(update.clone(), packet_sequence, generation)?;
        self.ui.write().await.project_display(update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::WorldBorder));
        Ok(())
    }

    pub(super) async fn receive_display_objective(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let update = ui_decode::decode_scoreboard(Java1_16_1, id, p)?;
        self.common_scoreboard
            .lock()
            .await
            .apply(update.clone(), packet_sequence)?;
        self.ui.write().await.project_scoreboard(update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::DisplayObjective));
        Ok(())
    }

    pub(super) async fn receive_scoreboard_objective(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let update = ui_decode::decode_scoreboard(Java1_16_1, id, p)?;
        self.common_scoreboard
            .lock()
            .await
            .apply(update.clone(), packet_sequence)?;
        self.ui.write().await.project_scoreboard(update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::Objective));
        Ok(())
    }

    pub(super) async fn receive_team(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let packet = ui_decode::decode_team(Java1_16_1, p)?;
        self.common_teams
            .lock()
            .await
            .apply_packet(Java1_16_1, packet.clone(), packet_sequence)?;
        self.ui.write().await.project_team(packet)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::Team));
        Ok(())
    }

    pub(super) async fn receive_score(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let update = ui_decode::decode_scoreboard(Java1_16_1, id, p)?;
        self.common_scoreboard
            .lock()
            .await
            .apply(update.clone(), packet_sequence)?;
        self.ui.write().await.project_scoreboard(update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::Score));
        Ok(())
    }

    pub(super) async fn receive_title(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let generation = self.common_receipts.lock().await.generation;
        let update = ui_decode::decode_display(Java1_16_1, id, p)?;
        self.common_display
            .lock()
            .await
            .apply(update.clone(), packet_sequence, generation)?;
        self.ui.write().await.project_display(update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::Title));
        Ok(())
    }

    pub(super) async fn receive_tab_list(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        let generation = self.common_receipts.lock().await.generation;
        let update = ui_decode::decode_display(Java1_16_1, id, p)?;
        self.common_display
            .lock()
            .await
            .apply(update.clone(), packet_sequence, generation)?;
        self.ui.write().await.project_display(update);
        self.emit(Event::UiStateUpdated(UiUpdateKind::TabList));
        Ok(())
    }
}
