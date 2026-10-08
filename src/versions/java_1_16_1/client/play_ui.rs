//! Protocol 736 ui packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

impl Bot {
    pub(super) async fn receive_boss_bar(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        self.common_boss_bars.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            p,
            packet_sequence,
        )?;
        self.ui.write().await.apply_boss_bar(p)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::BossBar));
        Ok(())
    }

    pub(super) async fn receive_player_info(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        self.common_player_list.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
        )?;
        let mut players = self.players.write().await;
        let (action, uuids) = apply_player_info(&mut players, p)?;
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
        self.common_display.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
            generation,
        )?;
        self.ui.write().await.apply_border(p)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::WorldBorder));
        Ok(())
    }

    pub(super) async fn receive_display_objective(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        self.common_scoreboard.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
        )?;
        self.ui.write().await.apply_display(p)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::DisplayObjective));
        Ok(())
    }

    pub(super) async fn receive_scoreboard_objective(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        self.common_scoreboard.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
        )?;
        self.ui.write().await.apply_objective(p)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::Objective));
        Ok(())
    }

    pub(super) async fn receive_team(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        self.common_teams.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            p,
            packet_sequence,
        )?;
        self.ui.write().await.apply_team(p)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::Team));
        Ok(())
    }

    pub(super) async fn receive_score(
        &self,
        p: &[u8],
        id: i32,
        packet_sequence: u64,
    ) -> Result<()> {
        self.common_scoreboard.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
        )?;
        self.ui.write().await.apply_score(p)?;
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
        self.common_display.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
            generation,
        )?;
        self.ui.write().await.apply_title(p)?;
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
        self.common_display.lock().await.receive(
            crate::MinecraftVersion::Java1_16_1,
            id,
            p,
            packet_sequence,
            generation,
        )?;
        self.ui.write().await.apply_tab(p)?;
        self.emit(Event::UiStateUpdated(UiUpdateKind::TabList));
        Ok(())
    }
}
