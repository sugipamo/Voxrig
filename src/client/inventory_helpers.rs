//! Inventory helpers composed from `Survival::click_inventory`: store the cursor,
//! merge partial stacks, fill a crafting grid by hand and craft once. Each click waits
//! for its received outcome before the next. See docs/common-inventory-helpers.md.
use super::container::ScreenObservation;
use super::crafting::CraftingTakeRecord;
use super::inventory::{InventoryClickButton, InventoryClickRecord, InventoryClickSource};
use super::{ItemStack, ObservedValue, SlotKnowledge};
use crate::{Error, ErrorKind, Result};
use std::time::Duration;

/// How long one click may wait for its received outcome.
const CLICK_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a filled grid may wait for the received result.
const RESULT_TIMEOUT: Duration = Duration::from_secs(2);

/// Clicks of one `fill_crafting_grid` and the result the server then showed.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftingFillRecord {
    /// Every click in order, each with its received outcome.
    pub clicks: Vec<InventoryClickRecord>,
    /// Received result slot after filling (None: no recipe matched in time).
    pub result: Option<ItemStack>,
}

/// One `craft_once`: the grid fill and the shift-take of its result.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftOnceRecord {
    /// The grid fill.
    pub fill: CraftingFillRecord,
    /// The result transfer into the inventory.
    pub take: CraftingTakeRecord,
}

/// Why a craft's retained record does not confirm completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CraftOnceUnknownCause {
    /// The take was still pending when the observation deadline was reached.
    Timeout,
    /// The take record reported a conflict or uncertain delivery.
    RequiresInspection,
}

/// Failure or unconfirmed outcome of [`super::Survival::craft_once_checked`].
///
/// Reaching the deadline does **not** mean the craft failed or was cancelled:
/// the server may still apply it. Re-observe and reconcile using received
/// inventory or operation status rather than blindly resending the craft.
#[derive(Debug)]
#[non_exhaustive]
pub enum CraftOnceError {
    /// Completion is unknown; retain the operation identity and last observed state.
    OutcomeUnknown {
        /// Grid fill and last take record, including `take.id` and `take.stage`.
        record: Box<CraftOnceRecord>,
        /// Why completion could not be confirmed.
        cause: CraftOnceUnknownCause,
    },
    /// An ordinary failure, preserving the original error and its kind.
    Failed(crate::Error),
}

impl std::fmt::Display for CraftOnceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutcomeUnknown { record, cause } => write!(
                formatter,
                "craft outcome unknown ({cause:?}); take {:?} last observed as {:?}; re-observe and reconcile before resending",
                record.take.id, record.take.stage
            ),
            Self::Failed(error) => write!(formatter, "craft failed: {error}"),
        }
    }
}

impl std::error::Error for CraftOnceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OutcomeUnknown { .. } => None,
            Self::Failed(error) => Some(error),
        }
    }
}

fn error(kind: ErrorKind, message: impl std::fmt::Display) -> Error {
    Error::new(kind, anyhow::anyhow!("{message}"))
}

/// Storage slots of the open screen in canonical player order (main, then hotbar).
struct View {
    source: InventoryClickSource,
    storage: Vec<(u16, SlotKnowledge)>,
    cursor: SlotKnowledge,
}

impl View {
    fn cursor_item(&self) -> Option<&ItemStack> {
        match &self.cursor {
            SlotKnowledge::Item { item } => Some(item),
            _ => None,
        }
    }
}

fn stacks_with(a: &ItemStack, b: &ItemStack) -> bool {
    a.id == b.id && a.data == b.data
}

fn room(stack: &ItemStack) -> Result<u32> {
    let max = u32::try_from(stack.properties()?.max_stack_size).unwrap_or(1);
    Ok(max.saturating_sub(stack.count))
}

fn known(value: Option<&super::ObservedValue<SlotKnowledge>>) -> SlotKnowledge {
    value.map_or(SlotKnowledge::Unavailable, |v| v.value.clone())
}

fn item_name(name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    }
}

fn container_storage_view(screen: &ScreenObservation) -> Result<Option<View>> {
    let cursor = known(screen.cursor.as_ref());
    if let Some(container) = screen.screen.as_ref().filter(|container| {
        screen.player_screen.is_none() && screen.active_window == Some(container.id.window_id())
    }) {
        let layout = container
            .layout
            .as_ref()
            .ok_or_else(|| error(ErrorKind::Unsupported, "open screen has no verified layout"))?;
        let mut slots = layout.player_slots.clone();
        slots.sort_by_key(|s| s.player_slot);
        let storage = slots
            .iter()
            .map(|s| {
                Ok((
                    u16::try_from(s.screen_slot)
                        .map_err(|_| error(ErrorKind::Protocol, "screen slot out of range"))?,
                    known(container.slots.get(s.screen_slot).and_then(Option::as_ref)),
                ))
            })
            .collect::<Result<_>>()?;
        return Ok(Some(View {
            source: InventoryClickSource::Container {
                screen: container.id,
            },
            storage,
            cursor,
        }));
    }
    Ok(None)
}

fn player_storage_view(
    screen: &ScreenObservation,
    player_slots: &[Option<ObservedValue<SlotKnowledge>>],
) -> View {
    let storage = (9u16..45)
        .map(|slot| {
            (
                slot,
                known(player_slots.get(usize::from(slot)).and_then(Option::as_ref)),
            )
        })
        .collect();
    View {
        source: InventoryClickSource::Player,
        storage,
        cursor: known(screen.cursor.as_ref()),
    }
}

impl super::Survival {
    async fn storage_view(&self) -> Result<View> {
        let screen = self.client.screen_state().await?;
        if let Some(view) = container_storage_view(&screen)? {
            return Ok(view);
        }
        let player = self.client.player_state().await?;
        Ok(player_storage_view(&screen, &player.inventory.slots))
    }

    /// One `click_inventory`, then wait until its record leaves `Pending`.
    /// Anything but `ObservedClicked` is an error carrying the record's reason.
    async fn click_and_wait(
        &self,
        source: InventoryClickSource,
        slot: u16,
        button: InventoryClickButton,
        clicks: &mut Vec<InventoryClickRecord>,
    ) -> Result<()> {
        use super::inventory::InventoryClickStage;
        let sent = self.click_inventory(source, slot, button).await?;
        let deadline = tokio::time::Instant::now() + CLICK_TIMEOUT;
        let mut record = sent.clone();
        while record.stage == InventoryClickStage::Pending {
            if tokio::time::Instant::now() > deadline {
                clicks.push(record);
                return Err(error(
                    ErrorKind::Timeout,
                    format!("click on slot {slot} not received"),
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Some(latest) = self.inventory_click_record().await? {
                if latest.id == sent.id {
                    record = latest;
                }
            }
        }
        let stage = record.stage;
        let reason = record.requires_inspection.clone();
        clicks.push(record);
        if stage != InventoryClickStage::ObservedClicked {
            return Err(error(
                ErrorKind::Rejected,
                format!(
                    "click on slot {slot}: {}",
                    reason.unwrap_or_else(|| format!("{stage:?}"))
                ),
            ));
        }
        Ok(())
    }

    /// Put the cursor stack into storage: compatible partial stacks first, then the
    /// first empty slot.
    async fn store_cursor(&self, clicks: &mut Vec<InventoryClickRecord>) -> Result<()> {
        for _ in 0..40 {
            let view = self.storage_view().await?;
            let Some(held) = view.cursor_item() else {
                return Ok(());
            };
            let mut target = None;
            for (slot, value) in &view.storage {
                if let SlotKnowledge::Item { item } = value {
                    if stacks_with(item, held) && room(item)? > 0 {
                        target = Some(*slot);
                        break;
                    }
                }
            }
            let target = target
                .or_else(|| {
                    view.storage
                        .iter()
                        .find(|(_, v)| matches!(v, SlotKnowledge::Empty))
                        .map(|(slot, _)| *slot)
                })
                .ok_or_else(|| error(ErrorKind::ResourceLimit, "no storage room for the cursor"))?;
            self.click_and_wait(view.source, target, InventoryClickButton::Left, clicks)
                .await?;
        }
        Err(error(ErrorKind::State, "cursor did not empty"))
    }

    /// Store the cursor, then merge partial stacks of the same item and data from the
    /// end of storage toward the front (main 9..35, then hotbar). Works on the player
    /// screen or on the player slots of an open container. Returns every click.
    pub async fn compact_inventory(&self) -> Result<Vec<InventoryClickRecord>> {
        let mut clicks = Vec::new();
        self.store_cursor(&mut clicks).await?;
        let len = self.storage_view().await?.storage.len();
        for index in (0..len).rev() {
            let view = self.storage_view().await?;
            let (slot, SlotKnowledge::Item { item }) = &view.storage[index] else {
                continue;
            };
            let mut mergeable = false;
            for (_, value) in &view.storage[..index] {
                if let SlotKnowledge::Item { item: earlier } = value {
                    if stacks_with(earlier, item) && room(earlier)? > 0 {
                        mergeable = true;
                        break;
                    }
                }
            }
            if mergeable {
                self.click_and_wait(view.source, *slot, InventoryClickButton::Left, &mut clicks)
                    .await?;
                self.store_cursor(&mut clicks).await?;
            }
        }
        Ok(clicks)
    }

    /// Fill the open crafting grid (player 2×2, or a crafting table's 3×3) by hand:
    /// return what the grid holds, then for each `Some(item)` pick a storage stack,
    /// place one with a right click and put the rest back. `ingredients` is row-major
    /// and must match the grid size. Waits up to two seconds for the received result.
    pub async fn fill_crafting_grid(
        &self,
        ingredients: &[Option<&str>],
    ) -> Result<CraftingFillRecord> {
        let grid = self
            .client
            .received_crafting()
            .await?
            .ok_or_else(|| error(ErrorKind::State, "no crafting grid is open"))?;
        let [width, height] = grid.dimensions();
        if ingredients.len() != width * height {
            return Err(error(
                ErrorKind::InvalidInput,
                format!("{width}x{height} grid needs {} ingredients", width * height),
            ));
        }
        let mut clicks = Vec::new();
        self.store_cursor(&mut clicks).await?;
        for index in 0..ingredients.len() {
            let (x, y) = (index % width, index / width);
            let grid = self
                .client
                .received_crafting()
                .await?
                .ok_or_else(|| error(ErrorKind::State, "crafting grid closed"))?;
            if grid.input(x, y)?.and_then(|s| s.item()).is_some() {
                let (source, slot) = grid.input_source(x, y)?;
                self.click_and_wait(source, slot, InventoryClickButton::Left, &mut clicks)
                    .await?;
                self.store_cursor(&mut clicks).await?;
            }
        }
        for (index, ingredient) in ingredients.iter().enumerate() {
            let Some(name) = ingredient else {
                continue;
            };
            let name = item_name(name);
            let view = self.storage_view().await?;
            let source_slot = view
                .storage
                .iter()
                .find(|(_, v)| matches!(v, SlotKnowledge::Item { item } if item.name == name))
                .map(|(slot, _)| *slot)
                .ok_or_else(|| error(ErrorKind::Rejected, format!("no {name} in storage")))?;
            let grid = self
                .client
                .received_crafting()
                .await?
                .ok_or_else(|| error(ErrorKind::State, "crafting grid closed"))?;
            let (grid_source, grid_slot) = grid.input_source(index % width, index / width)?;
            self.click_and_wait(
                view.source,
                source_slot,
                InventoryClickButton::Left,
                &mut clicks,
            )
            .await?;
            self.click_and_wait(
                grid_source,
                grid_slot,
                InventoryClickButton::Right,
                &mut clicks,
            )
            .await?;
            if self.storage_view().await?.cursor_item().is_some() {
                self.click_and_wait(
                    view.source,
                    source_slot,
                    InventoryClickButton::Left,
                    &mut clicks,
                )
                .await?;
            }
        }
        let deadline = tokio::time::Instant::now() + RESULT_TIMEOUT;
        let result = loop {
            let result = self
                .client
                .received_crafting()
                .await?
                .and_then(|g| g.result().and_then(|s| s.item()).map(|i| i.stack().clone()));
            if result.is_some() || tokio::time::Instant::now() > deadline {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        Ok(CraftingFillRecord { clicks, result })
    }

    async fn prepare_craft_once(
        &self,
        ingredients: &[Option<&str>],
    ) -> Result<(CraftingFillRecord, super::crafting::ReceivedCrafting)> {
        let fill = self.fill_crafting_grid(ingredients).await?;
        if fill.result.is_none() {
            return Err(error(
                ErrorKind::Rejected,
                "the filled grid shows no result",
            ));
        }
        let grid = self
            .client
            .received_crafting()
            .await?
            .ok_or_else(|| error(ErrorKind::State, "crafting grid closed"))?;
        Ok((fill, grid))
    }

    async fn wait_crafting_take(&self, sent: CraftingTakeRecord) -> Result<CraftingTakeRecord> {
        use super::crafting::CraftingTakeStage;
        let deadline = tokio::time::Instant::now() + CLICK_TIMEOUT;
        let mut take = sent.clone();
        while take.stage == CraftingTakeStage::Pending && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Some(latest) = self.crafting_take_record().await? {
                if latest.id == sent.id {
                    take = latest;
                }
            }
        }
        Ok(take)
    }

    /// `fill_crafting_grid`, then shift-take the result into the inventory once
    /// (`transfer_crafting_result`) and wait for its received outcome. No result
    /// is an error; the ingredients then stay in the grid.
    /// See [`Self::craft_once_checked`] for a result requiring confirmed completion.
    pub async fn craft_once(&self, ingredients: &[Option<&str>]) -> Result<CraftOnceRecord> {
        let (fill, grid) = self.prepare_craft_once(ingredients).await?;
        let sent = self.transfer_crafting_result(&grid).await?;
        let take = self.wait_crafting_take(sent).await?;
        Ok(CraftOnceRecord { fill, take })
    }

    /// Fill the grid and shift-take its result once, returning `Ok` only when the
    /// retained take record confirms completion (`ObservedTransferred`).
    /// Uses the same parameters, operations and deadlines as [`Self::craft_once`].
    ///
    /// Invalid input, fill errors, no result, and take reservation, ownership or
    /// predecessor checks rejecting before the frame write return
    /// [`CraftOnceError::Failed`] with the original error. Retention alone does
    /// not imply uncertain delivery. If an admitted take's write fails or its
    /// delivery becomes unknown, returns [`CraftOnceError::OutcomeUnknown`] with
    /// [`CraftOnceUnknownCause::RequiresInspection`], the fill record and the
    /// retained take (including `record.take.id`), without resending.
    ///
    /// A take still pending at the deadline returns
    /// [`CraftOnceError::OutcomeUnknown`] with [`CraftOnceUnknownCause::Timeout`],
    /// the operation ID (`record.take.id`) and last observed state (`record.take.stage`).
    /// Reaching the deadline does **not** mean the craft failed or was cancelled:
    /// the server may still apply it. Callers should re-observe and reconcile
    /// (for example via received inventory or [`Self::crafting_take_record`])
    /// rather than blindly resend. A returned take record requiring inspection
    /// also returns `OutcomeUnknown` with `RequiresInspection`.
    pub async fn craft_once_checked(
        &self,
        ingredients: &[Option<&str>],
    ) -> std::result::Result<CraftOnceRecord, CraftOnceError> {
        use super::adapter::CraftingTakeOps;
        use super::crafting::CraftingTakeStage;
        use super::crafting::{CraftingResultDestination, take::Submission};
        use std::sync::Arc;

        let (fill, grid) = self
            .prepare_craft_once(ingredients)
            .await
            .map_err(CraftOnceError::Failed)?;
        let submission = Arc::new(Submission::default());
        let result = crate::client::dispatch!(&self.client.adapter, a =>
            CraftingTakeOps::take_crafting_result_with_submission(
                a,
                super::GameMode::Survival,
                &grid,
                CraftingResultDestination::Inventory,
                Some(submission.clone()),
            ).await
        );
        let sent = match result {
            Ok(sent) => sent,
            Err(error) => {
                return Err(match submission.uncertain_record(&error) {
                    Some(take) => CraftOnceError::OutcomeUnknown {
                        record: Box::new(CraftOnceRecord { fill, take }),
                        cause: CraftOnceUnknownCause::RequiresInspection,
                    },
                    None => CraftOnceError::Failed(error),
                });
            }
        };
        let take = self
            .wait_crafting_take(sent)
            .await
            .map_err(CraftOnceError::Failed)?;
        let record = CraftOnceRecord { fill, take };
        if record.take.stage == CraftingTakeStage::ObservedTransferred {
            return Ok(record);
        }
        let cause = if record.take.stage == CraftingTakeStage::Pending {
            CraftOnceUnknownCause::Timeout
        } else {
            CraftOnceUnknownCause::RequiresInspection
        };
        Err(CraftOnceError::OutcomeUnknown {
            record: Box::new(record),
            cause,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinecraftVersion;
    use crate::client::{
        GameMode, InventoryObservation, PlayerObservation, SessionStamp,
        container::{
            PlayerScreenAccess, ScreenReceipts, ScreenTitle, player_screen_access, prepare_close,
        },
        received,
    };

    fn storage_fixture(version: MinecraftVersion) -> (ScreenObservation, PlayerObservation) {
        let session = SessionStamp {
            version,
            connection_id: 7,
            world_generation: 2,
        };
        let mut container = ScreenReceipts::open(version, 3, Some(2), ScreenTitle::Unavailable, 8);
        container
            .full_items(
                vec![Some(received(SlotKnowledge::Empty, 10)); 63],
                (version == MinecraftVersion::Java1_21_11).then_some(7),
                10,
            )
            .unwrap();
        let screen = ScreenObservation {
            session,
            receive_sequence: 10,
            active_window: Some(3),
            player_screen: None,
            screen: Some(container.capture(session)),
            cursor: Some(received(SlotKnowledge::Empty, 10)),
        };
        let player = PlayerObservation {
            using_item: None,
            fall_flying: None,
            entity_id: None,
            attributes: Default::default(),
            effects: Default::default(),
            air_supply: None,
            world_time: None,
            session,
            receive_sequence: 10,
            pending_dispatch: false,
            dimension: None,
            position: None,
            received_pose: None,
            rotation: [0.; 2],
            rotation_source: None,
            on_ground: None,
            game_mode: Some(GameMode::Survival),
            may_fly: None,
            health: None,
            selected_hotbar: None,
            inventory: InventoryObservation {
                slots: vec![Some(received(SlotKnowledge::Empty, 10)); 46],
                cursor: screen.cursor.clone(),
                window_id: screen.active_window,
                player_screen: None,
                ..Default::default()
            },
        };
        (screen, player)
    }

    #[test]
    fn storage_view_open_container_uses_container_player_mapping() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let (screen, _) = storage_fixture(version);
            let container = screen.screen.as_ref().unwrap();
            let view = container_storage_view(&screen).unwrap().unwrap();
            assert_eq!(
                view.source,
                InventoryClickSource::Container {
                    screen: container.id,
                }
            );
            let mut mappings = container.layout.as_ref().unwrap().player_slots.clone();
            mappings.sort_by_key(|mapping| mapping.player_slot);
            assert_eq!(
                view.storage,
                mappings
                    .iter()
                    .map(|mapping| (
                        u16::try_from(mapping.screen_slot).unwrap(),
                        known(container.slots[mapping.screen_slot].as_ref()),
                    ))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                view.storage
                    .iter()
                    .map(|(slot, _)| *slot)
                    .collect::<Vec<_>>(),
                (27..63).collect::<Vec<_>>()
            );
            assert_eq!(view.cursor, SlotKnowledge::Empty);
        }
    }

    #[test]
    fn storage_view_submitted_close_uses_player_slots() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let (mut screen, mut player) = storage_fixture(version);
            let container = screen.screen.as_ref().unwrap();
            let mut close = prepare_close(
                player.clone(),
                container.clone(),
                container.id,
                GameMode::Survival,
                None,
            )
            .unwrap();
            close.sent();
            screen.player_screen = player_screen_access(
                screen.session,
                screen.active_window,
                Some(container.id),
                Some(&close),
            );
            assert_eq!(
                screen.player_screen,
                Some(PlayerScreenAccess::SubmittedClose { close: close.id })
            );
            assert_eq!(screen.active_window, Some(container.id.window_id()));
            assert_eq!(screen.screen.as_ref().unwrap().id, close.id.screen());
            player.inventory.slots[9] = None;
            let view = container_storage_view(&screen)
                .unwrap()
                .unwrap_or_else(|| player_storage_view(&screen, &player.inventory.slots));
            assert_eq!(view.source, InventoryClickSource::Player);
            assert_eq!(
                view.storage,
                (9u16..45)
                    .map(|slot| (
                        slot,
                        known(player.inventory.slots[usize::from(slot)].as_ref()),
                    ))
                    .collect::<Vec<_>>()
            );
            assert_eq!(view.cursor, SlotKnowledge::Empty);
        }
    }

    #[test]
    fn item_names_default_to_the_minecraft_namespace() {
        assert_eq!(super::item_name("oak_planks"), "minecraft:oak_planks");
        assert_eq!(super::item_name("minecraft:stick"), "minecraft:stick");
    }
}
