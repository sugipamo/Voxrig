//! Mode-specific common operations. Handles do not grant or change game mode.
use super::{GameMode, PlayerObservation};
use crate::client::adapter::{
    ContainerOps, ControlOps, CraftingTakeOps, FlightOps, InventoryClickOps, InventorySwapOps,
    InventoryTransferOps, MiningOps, PathMotionOps, PlacementOps, RecipePlacementOps,
    StandingQueryOps,
};
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
    Entity(super::EntityId, super::entity::EntityAction),
    Look([f32; 2]),
    SelectHotbar(u8),
    SetFlying(bool),
    MoveFlying([f64; 3], [f32; 2]),
    SetHotbar(u8, Option<(&'a str, u8)>),
    Dig([i32; 3], BlockFace),
    UseOnBlock([i32; 3], BlockFace, [f32; 3], super::Hand),
    UseItem(super::Hand),
    ReleaseUseItem,
    Swing(super::Hand),
    /// Survival START_DESTROY_BLOCK.
    DigStart([i32; 3], BlockFace),
    /// Survival STOP_DESTROY_BLOCK.
    DigFinish([i32; 3], BlockFace),
}
impl Survival {
    /// Dispatch one interaction with an original received entity lifetime.
    /// Rechecks connection/world/spawn and received mode before I/O. No target
    /// selection, aim/reach/visibility proof, automatic retry or outcome ACK is supplied.
    /// Cancellation never retries; native interrupted dispatch requires inspection/reconnect.
    pub async fn interact_entity(
        &self,
        target: super::EntityId,
        hand: super::Hand,
        sneaking: bool,
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(
                GameMode::Survival,
                Action::Entity(
                    target,
                    super::entity::EntityAction::Interact { hand, sneaking },
                ),
            )
            .await
    }
    /// Dispatch exactly one attack, with the same lifetime and mode checks.
    /// This does not wait for a cooldown, choose a weapon, swing an arm or confirm damage.
    pub async fn attack_entity(
        &self,
        target: super::EntityId,
        sneaking: bool,
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(
                GameMode::Survival,
                Action::Entity(target, super::entity::EntityAction::Attack { sneaking }),
            )
            .await
    }

    /// Activate one received first-outline storage or crafting-table target with empty hands/cursor.
    /// Retains intent before I/O; complete dispatch and received screen/content facts
    /// are separate. OPEN packets contain no causal target-block identity.
    pub async fn open_container(
        &self,
        target: [i32; 3],
    ) -> Result<super::container::ContainerOpenRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::open_container(a, GameMode::Survival, target).await)
    }
    /// Read the latest activation record without resending, including after cancellation/closure.
    pub async fn container_open_record(
        &self,
    ) -> Result<Option<super::container::ContainerOpenRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::container_open_record(a).await)
    }

    /// Close one actual opening once. Complete dispatch is not server closure.
    /// Requires matching received mode; returns a known cursor through actual
    /// player-slot receipts before dispatch. Native crafting inputs are disposed
    /// by the server on close; dispatch does not prove their return.
    pub async fn close_container(
        &self,
        screen: super::container::ScreenId,
    ) -> Result<super::container::ContainerCloseRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::close_container(a, GameMode::Survival, screen).await)
    }
    /// Read retained close dispatch/actual response without replay, including after closure.
    pub async fn container_close_record(
        &self,
    ) -> Result<Option<super::container::ContainerCloseRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::container_close_record(a).await)
    }
    /// One ordinary left/right click of player input slots 1..4, inventory slots
    /// 9..44, or ordinary slots in the same audited storage/crafting opening.
    /// Requires received predecessors with resolved native data; retains separate prediction before I/O.
    pub async fn click_inventory(
        &self,
        source: super::inventory::InventoryClickSource,
        slot: u16,
        button: super::inventory::InventoryClickButton,
    ) -> Result<super::inventory::InventoryClickRecord> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryClickOps::click_inventory(a, GameMode::Survival, source, slot, button).await)
    }
    /// Submit this sealed Next/Maximum recipe plan once. Live predecessors,
    /// mode and opening are rechecked. Inputs and inventory remain actual receipts.
    /// Caller cancellation retains the owned attempt; inspection never replays it.
    pub async fn place_recipe(
        &self,
        plan: &super::crafting::RecipePlacementPlan,
    ) -> Result<super::crafting::RecipePlacementRecord> {
        crate::client::dispatch!(&self.client.adapter, a => RecipePlacementOps::place_recipe(a, GameMode::Survival, plan).await)
    }
    /// Inspect the retained placement, including during a stalled write.
    pub async fn recipe_placement_record(
        &self,
    ) -> Result<Option<super::crafting::RecipePlacementRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => RecipePlacementOps::recipe_placement_record(a).await)
    }
    /// Take one displayed result into an actual empty or compatible cursor.
    /// A held cursor must fit the entire result; no partial crafting is submitted.
    /// Rechecks this sealed snapshot before I/O. Ingredient consumption and
    /// remainders are observed from the server, never predicted or replayed.
    pub async fn take_crafting_result(
        &self,
        grid: &super::crafting::ReceivedCrafting,
    ) -> Result<super::crafting::CraftingTakeRecord> {
        crate::client::dispatch!(&self.client.adapter, a => CraftingTakeOps::take_crafting_result(a, GameMode::Survival, grid, super::crafting::CraftingResultDestination::Cursor).await)
    }
    /// Shift the actual result into main/hotbar with one native QUICK_MOVE.
    /// The first whole result must fit. Native internal crafts/remainders/drops
    /// are not predicted; inspect actual full inventory/grid and retained history.
    pub async fn transfer_crafting_result(
        &self,
        grid: &super::crafting::ReceivedCrafting,
    ) -> Result<super::crafting::CraftingTakeRecord> {
        crate::client::dispatch!(&self.client.adapter, a => CraftingTakeOps::take_crafting_result(a, GameMode::Survival, grid, super::crafting::CraftingResultDestination::Inventory).await)
    }
    /// Inspect the latest cursor take or inventory result transfer with actual receipts.
    /// Accessible while the writer is stalled; cancellation never resends it.
    pub async fn crafting_take_record(
        &self,
    ) -> Result<Option<super::crafting::CraftingTakeRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => CraftingTakeOps::crafting_take_record(a).await)
    }
    /// Inspect the retained click without replay. Both fresh source/cursor receipts are required.
    pub async fn inventory_click_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryClickRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryClickOps::inventory_click_record(a).await)
    }
    /// Shift-transfer one received source using native destination order, once.
    /// Player slots 5..45 include armor/offhand; storage uses the same original opening.
    /// Resolved native data and a known cursor are required; QUICK_MOVE preserves the cursor; changed-slot receipts remain separate from predictions.
    pub async fn transfer_inventory(
        &self,
        source: super::inventory::InventorySource,
        slot: u16,
    ) -> Result<super::inventory::InventoryTransferRecord> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryTransferOps::transfer_inventory(a, GameMode::Survival, source, slot).await)
    }
    /// Inspect the retained transfer without repeating it, including partial capacity/conflicts.
    pub async fn inventory_transfer_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryTransferRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryTransferOps::inventory_transfer_record(a).await)
    }
    /// Exchange one constructor-verified storage slot and a hotbar index once.
    /// Requires the same live opening, resolved received stacks and empty cursor.
    pub async fn swap_container_hotbar(
        &self,
        screen: super::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        crate::client::dispatch!(&self.client.adapter, a => InventorySwapOps::swap_container_hotbar(a, GameMode::Survival, screen, slot, hotbar).await)
    }
    /// Exchange main screen slot 9..35 and hotbar index 0..8 once.
    /// Uses complete received stacks and empty cursor; retains registry ownership before I/O.
    pub async fn swap_hotbar(
        &self,
        main_slot: u8,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        super::inventory::validate_slots(main_slot, hotbar)?;
        crate::client::dispatch!(&self.client.adapter, a => InventorySwapOps::swap_hotbar(a, GameMode::Survival, main_slot, hotbar).await)
    }
    /// Reconcile/read the latest common swap without sending another click.
    /// Partial results, conflicts and closure retain the original attempt.
    pub async fn inventory_swap_record(
        &self,
    ) -> Result<Option<super::inventory::InventorySwapRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => InventorySwapOps::inventory_swap_record(a).await)
    }
    /// Submit one stationary default passive-cube placement into received air.
    /// Derives the native first-outline cursor and retains the attempt before I/O.
    /// No prediction, retry, mode change or edit permission is supplied.
    pub async fn place_cube(
        &self,
        support: [i32; 3],
        face: BlockFace,
    ) -> Result<super::survival::PlacementRecord> {
        crate::client::dispatch!(&self.client.adapter, a => PlacementOps::place_cube(a, support, face).await)
    }
    /// Reconcile/read the latest retained placement, including closure diagnostics.
    /// Target, material and native protocol processing are separate receipts.
    pub async fn placement_record(&self) -> Result<Option<super::survival::PlacementRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => PlacementOps::placement_record(a).await)
    }

    /// Begin one stationary, received-empty-hand dirt/stone attempt.
    /// Retains coherent capture/intent before I/O. No timer or cancellation sends
    /// FINISH/ABORT automatically, and observed air never permits continuation.
    pub async fn start_mining(
        &self,
        target: [i32; 3],
        face: BlockFace,
    ) -> Result<super::survival::MiningRecord> {
        crate::client::dispatch!(&self.client.adapter, a => MiningOps::start_mining(a, target, face).await)
    }
    /// Attempt FINISH once for the owning retained attempt; never replay it.
    /// Estimated duration and protocol acknowledgement are not removal authority.
    pub async fn finish_mining(
        &self,
        id: super::survival::MiningId,
    ) -> Result<super::survival::MiningRecord> {
        crate::client::dispatch!(&self.client.adapter, a => MiningOps::mining_send(a, id, super::survival::MiningAction::Finish).await)
    }
    /// Attempt ABORT once. Delayed mining remains unresolved after dispatch.
    pub async fn abort_mining(
        &self,
        id: super::survival::MiningId,
    ) -> Result<super::survival::MiningRecord> {
        crate::client::dispatch!(&self.client.adapter, a => MiningOps::mining_send(a, id, super::survival::MiningAction::Abort).await)
    }
    /// Reconcile/read the retained attempt, including diagnostics after closure.
    /// No resends or implicit recovery. Target/inventory conflicts stay latched.
    pub async fn mining_record(&self) -> Result<Option<super::survival::MiningRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => MiningOps::mining_record(a).await)
    }
    /// Query the first static outline from a coherent dry-standing capture.
    /// Known version shapes and native view-vector math are used. Unavailable or
    /// unsupported geometry errors; the result never grants mining permission.
    pub async fn target_block(
        &self,
        maximum_distance: f64,
    ) -> Result<super::survival::BlockTargetObservation> {
        super::survival::target::validate_reach(maximum_distance)?;
        crate::client::dispatch!(&self.client.adapter, a => StandingQueryOps::target_block(a, GameMode::Survival, maximum_distance).await)
    }
    /// Forecast bounded walking/jump controls against one captured dry-cube world.
    /// Uses version-specific native defaults and current stationary admission.
    /// Fresh owned respawn may admit released-only settling onto known dry support.
    /// This is read-only; returned frames are predictions, never action authority.
    pub async fn preview_path(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::survival::MotionPreview> {
        super::survival::model::validate_controls(controls)?;
        crate::client::dispatch!(&self.client.adapter, a => StandingQueryOps::preview_path(a, GameMode::Survival, controls).await)
    }
    /// Retain and start a finite path under the explicit prediction contract.
    /// The connection owns dispatch after this call returns or its future drops.
    /// Requires a freshly validated, released-rest endpoint. Never auto-replays.
    /// Fresh owned respawn permits released-only settling with received zero velocity.
    /// Predicted completion is not an independent position observation.
    pub async fn start_predicted_path(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::survival::MotionRecord> {
        super::survival::model::validate_controls(controls)?;
        crate::client::dispatch!(&self.client.adapter, a => PathMotionOps::start_predicted_path(a, GameMode::Survival, controls).await)
    }
    /// Read the latest retained common run, including failure after closure.
    /// Pure inspection; never resumes, cancels or replays input.
    pub async fn motion_record(&self) -> Result<Option<super::survival::MotionRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => PathMotionOps::motion_record(a).await)
    }
    /// Capture player/inventory without inventing mode, item or position facts.
    pub async fn player_state(&self) -> Result<PlayerObservation> {
        self.client.player_state().await
    }
    /// Start continuous control from the latest received pose with released keys.
    /// The client then runs the shared physics every 50 ms tick and sends each
    /// tick's movement; sent positions are submissions, not server acceptance.
    pub async fn start_control(&self) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::start_control(a, GameMode::Survival).await)
    }
    /// Replace the held keys; they apply from the next tick until replaced.
    pub async fn set_controls(
        &self,
        controls: super::control::Controls,
    ) -> Result<super::control::ControlRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::set_controls(a, GameMode::Survival, controls).await)
    }
    /// Stop the session, releasing sprint and sneak. Returns the final record.
    pub async fn stop_control(&self) -> Result<Option<super::control::ControlRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::stop_control(a).await)
    }
    /// Latest control record, readable after the session or connection ended.
    pub async fn control_record(&self) -> Result<Option<super::control::ControlRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ControlOps::control_record(a).await)
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
}
impl Creative {
    /// Dispatch one interaction with an original received entity lifetime.
    /// Rechecks connection/world/spawn and received mode before I/O. No target
    /// selection, aim/reach/visibility proof, automatic retry or outcome ACK is supplied.
    /// Cancellation never retries; native interrupted dispatch requires inspection/reconnect.
    pub async fn interact_entity(
        &self,
        target: super::EntityId,
        hand: super::Hand,
        sneaking: bool,
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(
                GameMode::Creative,
                Action::Entity(
                    target,
                    super::entity::EntityAction::Interact { hand, sneaking },
                ),
            )
            .await
    }
    /// Dispatch exactly one attack, with the same lifetime and mode checks.
    /// This does not wait for a cooldown, choose a weapon, swing an arm or confirm damage.
    pub async fn attack_entity(
        &self,
        target: super::EntityId,
        sneaking: bool,
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(
                GameMode::Creative,
                Action::Entity(target, super::entity::EntityAction::Attack { sneaking }),
            )
            .await
    }

    /// Read-only finite ground walking/jump preview while flight is inactive.
    /// Uses the selected adapter's native dry defaults and received Creative mode.
    /// Fresh owned respawn may admit released-only settling onto known dry support.
    pub async fn preview_path(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::survival::MotionPreview> {
        super::survival::model::validate_controls(controls)?;
        crate::client::dispatch!(&self.client.adapter, a => StandingQueryOps::preview_path(a, GameMode::Creative, controls).await)
    }
    /// Start a finite ground path under the explicit prediction contract.
    /// Flight must be inactive. Complete dispatch is not a received position.
    /// Fresh owned respawn permits released-only settling with received zero velocity.
    pub async fn start_predicted_path(
        &self,
        controls: &[super::survival::SurvivalControl],
    ) -> Result<super::survival::MotionRecord> {
        super::survival::model::validate_controls(controls)?;
        crate::client::dispatch!(&self.client.adapter, a => PathMotionOps::start_predicted_path(a, GameMode::Creative, controls).await)
    }
    /// Read the retained common ground run without replaying any input.
    pub async fn motion_record(&self) -> Result<Option<super::survival::MotionRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => PathMotionOps::motion_record(a).await)
    }
    /// Activate one received first-outline storage or crafting-table target with empty hands/cursor.
    /// Retains intent before I/O; complete dispatch and received screen/content facts
    /// are separate. OPEN packets contain no causal target-block identity.
    pub async fn open_container(
        &self,
        target: [i32; 3],
    ) -> Result<super::container::ContainerOpenRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::open_container(a, GameMode::Creative, target).await)
    }
    /// Read the latest activation record without resending, including after cancellation/closure.
    pub async fn container_open_record(
        &self,
    ) -> Result<Option<super::container::ContainerOpenRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::container_open_record(a).await)
    }

    /// Read the first audited static outline from a coherent dry-standing capture.
    /// Checks received creative mode; no flight, mutation or open result is implied.
    /// Shares native shape selection with Survival::target_block, reach <=4.5.
    pub async fn target_block(
        &self,
        maximum_distance: f64,
    ) -> Result<super::survival::BlockTargetObservation> {
        super::survival::target::validate_reach(maximum_distance)?;
        crate::client::dispatch!(&self.client.adapter, a => StandingQueryOps::target_block(a, GameMode::Creative, maximum_distance).await)
    }
    /// Close one received opening once. This ordinary operation does not change mode.
    /// Vanilla need not acknowledge it; inspect `dispatched` separately from actual closure.
    /// Cursor return uses actual player-slot receipts. Native crafting inputs
    /// are disposed by the server on close; dispatch does not prove their return.
    pub async fn close_container(
        &self,
        screen: super::container::ScreenId,
    ) -> Result<super::container::ContainerCloseRecord> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::close_container(a, GameMode::Creative, screen).await)
    }
    /// Read the retained close intent, including after caller cancellation/disconnection.
    pub async fn container_close_record(
        &self,
    ) -> Result<Option<super::container::ContainerCloseRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => ContainerOps::container_close_record(a).await)
    }
    /// One ordinary left/right click of player input slots 1..4, inventory slots
    /// 9..44, or ordinary slots in the same audited storage/crafting opening.
    /// Requires received predecessors with resolved native data; retains separate prediction before I/O.
    pub async fn click_inventory(
        &self,
        source: super::inventory::InventoryClickSource,
        slot: u16,
        button: super::inventory::InventoryClickButton,
    ) -> Result<super::inventory::InventoryClickRecord> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryClickOps::click_inventory(a, GameMode::Creative, source, slot, button).await)
    }
    /// Submit this sealed Next/Maximum recipe plan once. Live predecessors,
    /// mode and opening are rechecked. Inputs and inventory remain actual receipts.
    /// Caller cancellation retains the owned attempt; inspection never replays it.
    pub async fn place_recipe(
        &self,
        plan: &super::crafting::RecipePlacementPlan,
    ) -> Result<super::crafting::RecipePlacementRecord> {
        crate::client::dispatch!(&self.client.adapter, a => RecipePlacementOps::place_recipe(a, GameMode::Creative, plan).await)
    }
    /// Inspect the retained placement, including during a stalled write.
    pub async fn recipe_placement_record(
        &self,
    ) -> Result<Option<super::crafting::RecipePlacementRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => RecipePlacementOps::recipe_placement_record(a).await)
    }
    /// Take one displayed result into an actual empty or compatible cursor.
    /// A held cursor must fit the entire result; no partial crafting is submitted.
    /// Rechecks this sealed snapshot before I/O. Ingredient consumption and
    /// remainders are observed from the server, never predicted or replayed.
    pub async fn take_crafting_result(
        &self,
        grid: &super::crafting::ReceivedCrafting,
    ) -> Result<super::crafting::CraftingTakeRecord> {
        crate::client::dispatch!(&self.client.adapter, a => CraftingTakeOps::take_crafting_result(a, GameMode::Creative, grid, super::crafting::CraftingResultDestination::Cursor).await)
    }
    /// Shift the actual result into main/hotbar with one native QUICK_MOVE.
    /// The first whole result must fit. Native internal crafts/remainders/drops
    /// are not predicted; inspect actual full inventory/grid and retained history.
    pub async fn transfer_crafting_result(
        &self,
        grid: &super::crafting::ReceivedCrafting,
    ) -> Result<super::crafting::CraftingTakeRecord> {
        crate::client::dispatch!(&self.client.adapter, a => CraftingTakeOps::take_crafting_result(a, GameMode::Creative, grid, super::crafting::CraftingResultDestination::Inventory).await)
    }
    /// Inspect the latest cursor take or inventory result transfer with actual receipts.
    /// Accessible while the writer is stalled; cancellation never resends it.
    pub async fn crafting_take_record(
        &self,
    ) -> Result<Option<super::crafting::CraftingTakeRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => CraftingTakeOps::crafting_take_record(a).await)
    }
    /// Inspect the retained click without replay. Both fresh source/cursor receipts are required.
    pub async fn inventory_click_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryClickRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryClickOps::inventory_click_record(a).await)
    }
    /// Shift-transfer one received source using native destination order, once.
    /// Player slots 5..45 include armor/offhand; storage uses the same original opening.
    /// Resolved native data and a known cursor are required; QUICK_MOVE preserves the cursor; changed-slot receipts remain separate from predictions.
    pub async fn transfer_inventory(
        &self,
        source: super::inventory::InventorySource,
        slot: u16,
    ) -> Result<super::inventory::InventoryTransferRecord> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryTransferOps::transfer_inventory(a, GameMode::Creative, source, slot).await)
    }
    /// Inspect the retained transfer without repeating it, including partial capacity/conflicts.
    pub async fn inventory_transfer_record(
        &self,
    ) -> Result<Option<super::inventory::InventoryTransferRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => InventoryTransferOps::inventory_transfer_record(a).await)
    }

    /// Ordinary storage exchange; this does not manufacture creative items.
    pub async fn swap_container_hotbar(
        &self,
        screen: super::container::ScreenId,
        slot: u16,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        crate::client::dispatch!(&self.client.adapter, a => InventorySwapOps::swap_container_hotbar(a, GameMode::Creative, screen, slot, hotbar).await)
    }
    /// Exchange complete received player stacks once in creative mode.
    /// This ordinary inventory click does not create items or change mode.
    pub async fn swap_hotbar(
        &self,
        main_slot: u8,
        hotbar: u8,
    ) -> Result<super::inventory::InventorySwapRecord> {
        super::inventory::validate_slots(main_slot, hotbar)?;
        crate::client::dispatch!(&self.client.adapter, a => InventorySwapOps::swap_hotbar(a, GameMode::Creative, main_slot, hotbar).await)
    }
    /// Reconcile/read the retained common inventory exchange, including closure.
    pub async fn inventory_swap_record(
        &self,
    ) -> Result<Option<super::inventory::InventorySwapRecord>> {
        crate::client::dispatch!(&self.client.adapter, a => InventorySwapOps::inventory_swap_record(a).await)
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
    /// The Client owns this one-shot write after admission, even if the caller stops waiting.
    /// Inspect `Client::flight_record` after cancellation; do not replay the request.
    /// Enabling retires the previous ground endpoint. Disabling does not establish grounding.
    pub async fn set_flying(&self, flying: bool) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SetFlying(flying))
            .await
    }
    /// Submit a flight step of at most four blocks. Not collision resolution or teleport.
    /// The Client retains and owns the write; `Client::flight_record` is readable during I/O.
    /// This updates submitted position only, preserving the last actual received pose.
    pub async fn move_flying(
        &self,
        position: [f64; 3],
        rotation: [f32; 2],
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::MoveFlying(position, rotation))
            .await
    }
    /// Explicitly stop owned flight at its current fully submitted position.
    /// First use `move_flying` to return to known dry support. This checks standing
    /// clearance, disables flight, releases input and owns two released ground ticks.
    /// The zero controller seed is declared model state, never a received velocity
    /// or landing ACK. Inspect `Client::flight_record()` after cancelling the wait.
    pub async fn land(&self) -> Result<super::flight::FlightRecord> {
        crate::client::dispatch!(&self.client.adapter, a => FlightOps::flight(a, super::flight::FlightCommand::Land).await)
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
}
impl Creative {
    /// Use the item in `hand` on a loaded block face within 4.5 blocks of the eye
    /// (place, open, press, light...). `cursor` is the hit point inside the target cell,
    /// each axis within 0..1. Dispatch only: the server's result arrives as received block,
    /// inventory and screen updates. See `docs/common-item-use.md`.
    pub async fn use_on_block(
        &self,
        target: [i32; 3],
        face: BlockFace,
        cursor: [f32; 3],
        hand: super::Hand,
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(
                GameMode::Creative,
                Action::UseOnBlock(target, face, cursor, hand),
            )
            .await
    }
    /// Use the item in `hand` without a target (eat, drink, raise a shield, draw a bow).
    /// Dispatch only: whether use started is the received `PlayerObservation::using_item`.
    /// A running control session applies the item-use slowdown once that is received.
    pub async fn use_item(&self, hand: super::Hand) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::UseItem(hand))
            .await
    }
    /// Release the item in use (shoot a bow, lower a shield, stop eating). Dispatch only.
    pub async fn release_use_item(&self) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::ReleaseUseItem)
            .await
    }
    /// Swing an arm (ServerboundSwingPacket). The official client swings the main hand
    /// after each attack and after a successful use; `attack_entity` does not.
    pub async fn swing_arm(&self, hand: super::Hand) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::Swing(hand))
            .await
    }
}
impl Survival {
    /// Use the item in `hand` on a loaded block face within 4.5 blocks of the eye
    /// (place, open, press, light...). `cursor` is the hit point inside the target cell,
    /// each axis within 0..1. Dispatch only: the server's result arrives as received block,
    /// inventory and screen updates. See `docs/common-item-use.md`.
    pub async fn use_on_block(
        &self,
        target: [i32; 3],
        face: BlockFace,
        cursor: [f32; 3],
        hand: super::Hand,
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(
                GameMode::Survival,
                Action::UseOnBlock(target, face, cursor, hand),
            )
            .await
    }
    /// Use the item in `hand` without a target (eat, drink, raise a shield, draw a bow).
    /// Dispatch only: whether use started is the received `PlayerObservation::using_item`.
    /// A running control session applies the item-use slowdown once that is received.
    pub async fn use_item(&self, hand: super::Hand) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::UseItem(hand))
            .await
    }
    /// Release the item in use (shoot a bow, lower a shield, stop eating). Dispatch only.
    pub async fn release_use_item(&self) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::ReleaseUseItem)
            .await
    }
    /// Swing an arm (ServerboundSwingPacket). The official client swings the main hand
    /// after each attack and after a successful use; `attack_entity` does not.
    pub async fn swing_arm(&self, hand: super::Hand) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::Swing(hand))
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
