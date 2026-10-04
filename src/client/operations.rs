//! Mode-specific common operations. Handles do not grant or change game mode.
use super::{GameMode, PlayerObservation};
use crate::{BlockFace, Client, MinecraftVersion, Result};

/// Complete packet dispatch, separate from protocol acknowledgement or game outcome.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DispatchReceipt {
    /// Owning version.
    pub version: MinecraftVersion,
    /// Owning transport, meaningful within this process only.
    pub connection_id: u64,
    /// Interaction sequence if this native protocol provides one.
    pub interaction_sequence: Option<i32>,
}
/// Operations whose permissions are checked against received survival mode.
///
/// Survival handles cannot manufacture creative items:
/// ```compile_fail
/// async fn bypass(ops: &voxrig::client::Survival) {
///     ops.set_hotbar(0, Some(("minecraft:stone", 1))).await.unwrap();
/// }
/// ```

#[derive(Clone)]
pub struct Survival {
    pub(crate) client: Client,
}
/// Operations whose permissions are checked against received creative mode.
#[derive(Clone)]
pub struct Creative {
    pub(crate) client: Client,
}

pub(crate) enum Action<'a> {
    Look([f32; 2]),
    SelectHotbar(u8),
    SetFlying(bool),
    MoveFlying([f64; 3], [f32; 2]),
    SetHotbar(u8, Option<(&'a str, u8)>),
    Dig([i32; 3], BlockFace),
    UseOnBlock([i32; 3], BlockFace, [f32; 3]),
}
impl Survival {
    /// Activate one received first-outline storage target with empty hands/cursor.
    /// Retains intent before I/O; complete dispatch and received screen/content facts
    /// are separate. OPEN packets contain no causal target-block identity.
    pub async fn open_container(
        &self,
        target: [i32; 3],
    ) -> Result<super::container::ContainerOpenRecord> {
        self.client
            .common_open_container(GameMode::Survival, target)
            .await
    }
    /// Read the latest activation record without resending, including after cancellation/closure.
    pub async fn container_open_record(
        &self,
    ) -> Result<Option<super::container::ContainerOpenRecord>> {
        self.client.common_container_open_record().await
    }

    /// Close one actual opening once. Complete dispatch is not server closure.
    /// Requires matching received mode and empty cursor; retains intent before I/O.
    pub async fn close_container(
        &self,
        screen: super::container::ScreenId,
    ) -> Result<super::container::ContainerCloseRecord> {
        self.client
            .common_close_container(GameMode::Survival, screen)
            .await
    }
    /// Read retained close dispatch/actual response without replay, including after closure.
    pub async fn container_close_record(
        &self,
    ) -> Result<Option<super::container::ContainerCloseRecord>> {
        self.client.common_container_close_record().await
    }
    /// One ordinary left/right click of player slots 9..44 or the same storage opening.
    /// Requires received default predecessors; retains intent and separate prediction before I/O.
    pub async fn click_inventory(
        &self,
        source: super::inventory::InventoryClickSource,
        slot: u16,
        button: super::inventory::InventoryClickButton,
    ) -> Result<super::inventory::InventoryClickRecord> {
        self.client
            .common_click_inventory(GameMode::Survival, source, slot, button)
            .await
    }
    /// Inspect the retained click without replay. Both fresh source/cursor receipts are required.
    pub async fn inventory_click_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryClickRecord>> {
        self.client.common_inventory_click_record().await
    }
    /// Shift-transfer one received source using native destination order, once.
    /// Player slots 5..45 include armor/offhand; storage uses the same original opening.
    /// Default data and empty cursor are required. All changed-slot receipts are separate from predictions.
    pub async fn transfer_inventory(
        &self,
        source: super::inventory::InventorySource,
        slot: u16,
    ) -> Result<super::inventory::InventoryTransferRecord> {
        self.client
            .common_transfer_inventory(GameMode::Survival, source, slot)
            .await
    }
    /// Inspect the retained transfer without repeating it, including partial capacity/conflicts.
    pub async fn inventory_transfer_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryTransferRecord>> {
        self.client.common_inventory_transfer_record().await
    }
    /// Exchange one constructor-verified storage slot and a hotbar index once.
    /// Requires the same live opening, default received stacks and empty cursor.
    pub async fn swap_container_hotbar(
        &self,
        screen: super::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        self.client
            .common_swap_container_hotbar(GameMode::Survival, screen, slot, hotbar)
            .await
    }
    /// Exchange main screen slot 9..35 and hotbar index 0..8 once.
    /// Uses complete received default stacks/cursor; retains intent before I/O.
    pub async fn swap_hotbar(
        &self,
        main_slot: u8,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        super::inventory::validate_slots(main_slot, hotbar)?;
        self.client
            .common_swap_hotbar(GameMode::Survival, main_slot, hotbar)
            .await
    }
    /// Reconcile/read the latest common swap without sending another click.
    /// Partial results, conflicts and closure retain the original attempt.
    pub async fn inventory_swap_record(
        &self,
    ) -> Result<Option<super::inventory::InventorySwapRecord>> {
        self.client.common_inventory_swap_record().await
    }
    /// Submit one stationary default passive-cube placement into received air.
    /// Derives the native first-outline cursor and retains the attempt before I/O.
    /// No prediction, retry, mode change or edit permission is supplied.
    pub async fn place_cube(
        &self,
        support: [i32; 3],
        face: BlockFace,
    ) -> Result<super::survival::PlacementRecord> {
        self.client.place_common_cube(support, face).await
    }
    /// Reconcile/read the latest retained placement, including closure diagnostics.
    /// Target, material and native protocol processing are separate receipts.
    pub async fn placement_record(&self) -> Result<Option<super::survival::PlacementRecord>> {
        self.client.common_placement_record().await
    }

    /// Begin one stationary, received-empty-hand dirt/stone attempt.
    /// Retains coherent capture/intent before I/O. No timer or cancellation sends
    /// FINISH/ABORT automatically, and observed air never permits continuation.
    pub async fn start_mining(
        &self,
        target: [i32; 3],
        face: BlockFace,
    ) -> Result<super::survival::MiningRecord> {
        self.client.start_common_mining(target, face).await
    }
    /// Attempt FINISH once for the owning retained attempt; never replay it.
    /// Estimated duration and protocol acknowledgement are not removal authority.
    pub async fn finish_mining(
        &self,
        id: super::survival::MiningId,
    ) -> Result<super::survival::MiningRecord> {
        self.client
            .send_common_mining(id, super::survival::MiningAction::Finish)
            .await
    }
    /// Attempt ABORT once. Delayed mining remains unresolved after dispatch.
    pub async fn abort_mining(
        &self,
        id: super::survival::MiningId,
    ) -> Result<super::survival::MiningRecord> {
        self.client
            .send_common_mining(id, super::survival::MiningAction::Abort)
            .await
    }
    /// Reconcile/read the retained attempt, including diagnostics after closure.
    /// No resends or implicit recovery. Target/inventory conflicts stay latched.
    pub async fn mining_record(&self) -> Result<Option<super::survival::MiningRecord>> {
        self.client.common_mining_record().await
    }
    /// Query the first static outline from a coherent dry-standing capture.
    /// Known version shapes and native view-vector math are used. Unavailable or
    /// unsupported geometry errors; the result never grants mining permission.
    pub async fn target_block(
        &self,
        maximum_distance: f64,
    ) -> Result<super::survival::BlockTargetObservation> {
        super::survival::target::validate_reach(maximum_distance)?;
        self.client
            .common_block_target(GameMode::Survival, maximum_distance)
            .await
    }
    /// Forecast bounded walking/jump controls against one captured dry-cube world.
    /// Uses version-specific native defaults and current stationary admission.
    /// This is read-only; returned frames are predictions, never action authority.
    pub async fn preview_path(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::survival::MotionPreview> {
        super::survival::model::validate_controls(controls)?;
        self.client.preview_survival_path(controls).await
    }
    /// Retain and start a finite path under the explicit prediction contract.
    /// The connection owns dispatch after this call returns or its future drops.
    /// Requires a freshly validated, released-rest endpoint. Never auto-replays.
    /// Predicted completion is not an independent position observation.
    pub async fn start_predicted_path(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::survival::MotionRecord> {
        super::survival::model::validate_controls(controls)?;
        self.client.start_predicted_survival_path(controls).await
    }
    /// Read the latest retained common run, including failure after closure.
    /// Pure inspection; never resumes, cancels or replays input.
    pub async fn motion_record(&self) -> Result<Option<super::survival::MotionRecord>> {
        self.client.survival_motion_record().await
    }
    /// Capture player/inventory without inventing mode, item or position facts.
    pub async fn player_state(&self) -> Result<PlayerObservation> {
        self.client.player_state().await
    }
    /// Dispatch a look. Native admission applies; success is not acceptance.
    pub async fn look(&self, rotation: [f32; 2]) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::Look(rotation))
            .await
    }
    /// Select main-hand hotbar index 0..8 without inventing an item echo.
    pub async fn select_hotbar(&self, slot: u8) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::SelectHotbar(slot))
            .await
    }
    /// Select the additional audited dry-cube contract, when implemented.
    /// This remains an explicitly restricted extension, not general survival parity.
    pub fn checked(&self) -> Result<crate::checked_survival::Operations> {
        self.client.checked_survival()
    }
}
impl Creative {
    /// Activate one received first-outline storage target with empty hands/cursor.
    /// Retains intent before I/O; complete dispatch and received screen/content facts
    /// are separate. OPEN packets contain no causal target-block identity.
    pub async fn open_container(
        &self,
        target: [i32; 3],
    ) -> Result<super::container::ContainerOpenRecord> {
        self.client
            .common_open_container(GameMode::Creative, target)
            .await
    }
    /// Read the latest activation record without resending, including after cancellation/closure.
    pub async fn container_open_record(
        &self,
    ) -> Result<Option<super::container::ContainerOpenRecord>> {
        self.client.common_container_open_record().await
    }

    /// Read the first audited static outline from a coherent dry-standing capture.
    /// Checks received creative mode; no flight, mutation or open result is implied.
    /// Shares native shape selection with Survival::target_block, reach <=4.5.
    pub async fn target_block(
        &self,
        maximum_distance: f64,
    ) -> Result<super::survival::BlockTargetObservation> {
        super::survival::target::validate_reach(maximum_distance)?;
        self.client
            .common_block_target(GameMode::Creative, maximum_distance)
            .await
    }
    /// Close one received opening once. This ordinary operation does not change mode.
    /// Vanilla need not acknowledge it; inspect `dispatched` separately from actual closure.
    pub async fn close_container(
        &self,
        screen: super::container::ScreenId,
    ) -> Result<super::container::ContainerCloseRecord> {
        self.client
            .common_close_container(GameMode::Creative, screen)
            .await
    }
    /// Read the retained close intent, including after caller cancellation/disconnection.
    pub async fn container_close_record(
        &self,
    ) -> Result<Option<super::container::ContainerCloseRecord>> {
        self.client.common_container_close_record().await
    }
    /// One ordinary left/right click of player slots 9..44 or the same storage opening.
    /// Requires received default predecessors; retains intent and separate prediction before I/O.
    pub async fn click_inventory(
        &self,
        source: super::inventory::InventoryClickSource,
        slot: u16,
        button: super::inventory::InventoryClickButton,
    ) -> Result<super::inventory::InventoryClickRecord> {
        self.client
            .common_click_inventory(GameMode::Creative, source, slot, button)
            .await
    }
    /// Inspect the retained click without replay. Both fresh source/cursor receipts are required.
    pub async fn inventory_click_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryClickRecord>> {
        self.client.common_inventory_click_record().await
    }
    /// Shift-transfer one received source using native destination order, once.
    /// Player slots 5..45 include armor/offhand; storage uses the same original opening.
    /// Default data and empty cursor are required. All changed-slot receipts are separate from predictions.
    pub async fn transfer_inventory(
        &self,
        source: super::inventory::InventorySource,
        slot: u16,
    ) -> Result<super::inventory::InventoryTransferRecord> {
        self.client
            .common_transfer_inventory(GameMode::Creative, source, slot)
            .await
    }
    /// Inspect the retained transfer without repeating it, including partial capacity/conflicts.
    pub async fn inventory_transfer_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryTransferRecord>> {
        self.client.common_inventory_transfer_record().await
    }

    /// Ordinary storage exchange; this does not manufacture creative items.
    pub async fn swap_container_hotbar(
        &self,
        screen: super::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        self.client
            .common_swap_container_hotbar(GameMode::Creative, screen, slot, hotbar)
            .await
    }
    /// Exchange complete received default player stacks once in creative mode.
    /// This ordinary inventory click does not create items or change mode.
    pub async fn swap_hotbar(
        &self,
        main_slot: u8,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        super::inventory::validate_slots(main_slot, hotbar)?;
        self.client
            .common_swap_hotbar(GameMode::Creative, main_slot, hotbar)
            .await
    }
    /// Reconcile/read the retained common inventory exchange, including closure.
    pub async fn inventory_swap_record(
        &self,
    ) -> Result<Option<super::inventory::InventorySwapRecord>> {
        self.client.common_inventory_swap_record().await
    }
    /// Capture player/inventory. Holding this handle does not imply creative permission.
    pub async fn player_state(&self) -> Result<PlayerObservation> {
        self.client.player_state().await
    }
    /// Dispatch a view rotation under received creative-mode admission.
    pub async fn look(&self, rotation: [f32; 2]) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::Look(rotation))
            .await
    }
    /// Select main-hand hotbar index 0..8.
    pub async fn select_hotbar(&self, slot: u8) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SelectHotbar(slot))
            .await
    }
    /// Request flight; enabling requires a received server flight permission.
    pub async fn set_flying(&self, flying: bool) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SetFlying(flying))
            .await
    }
    /// Submit a flight step of at most four blocks. Not collision resolution or teleport.
    pub async fn move_flying(
        &self,
        position: [f64; 3],
        rotation: [f32; 2],
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::MoveFlying(position, rotation))
            .await
    }
    /// Write a default stack by namespaced name, or clear a hotbar slot.
    /// Inventory remains unknown until received; no local result prediction.
    pub async fn set_hotbar(&self, slot: u8, item: Option<(&str, u8)>) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SetHotbar(slot, item))
            .await
    }
    /// Dispatch one creative START break to a loaded reachable target.
    pub async fn break_block(&self, target: [i32; 3], face: BlockFace) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::Dig(target, face))
            .await
    }
    /// Use held item on a loaded reachable block face. Can place or activate.
    /// Cursor coordinates are relative to the target block, each within 0..1.
    pub async fn use_on_block(
        &self,
        target: [i32; 3],
        face: BlockFace,
        cursor: [f32; 3],
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::UseOnBlock(target, face, cursor))
            .await
    }
}
pub(crate) fn validate_rotation(rotation: [f32; 2]) -> Result<()> {
    if rotation.iter().any(|v| !v.is_finite()) || !(-90.0..=90.0).contains(&rotation[1]) {
        return Err(super::registry::invalid(
            "rotation must be finite, with pitch -90..90",
        ));
    }
    Ok(())
}
