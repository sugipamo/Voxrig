//! Inventory helpers composed from `Survival::click_inventory`: store the cursor,
//! merge partial stacks, fill a crafting grid by hand and craft once. Each click waits
//! for its received outcome before the next. See docs/common-inventory-helpers.md.
use super::crafting::CraftingTakeRecord;
use super::inventory::{InventoryClickButton, InventoryClickRecord, InventoryClickSource};
use super::{ItemStack, SlotKnowledge};
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

impl super::Survival {
    async fn storage_view(&self) -> Result<View> {
        let screen = self.client.screen_state().await?;
        let cursor = known(screen.cursor.as_ref());
        if let Some(container) = screen.screen {
            let layout = container.layout.as_ref().ok_or_else(|| {
                error(ErrorKind::Unsupported, "open screen has no verified layout")
            })?;
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
            return Ok(View {
                source: InventoryClickSource::Container {
                    screen: container.id,
                },
                storage,
                cursor,
            });
        }
        let player = self.client.player_state().await?;
        let storage = (9u16..45)
            .map(|slot| {
                (
                    slot,
                    known(
                        player
                            .inventory
                            .slots
                            .get(usize::from(slot))
                            .and_then(Option::as_ref),
                    ),
                )
            })
            .collect();
        Ok(View {
            source: InventoryClickSource::Player,
            storage,
            cursor,
        })
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

    /// `fill_crafting_grid`, then shift-take the result into the inventory once
    /// (`transfer_crafting_result`) and wait for its received outcome. No result
    /// is an error; the ingredients then stay in the grid.
    pub async fn craft_once(&self, ingredients: &[Option<&str>]) -> Result<CraftOnceRecord> {
        use super::crafting::CraftingTakeStage;
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
        let sent = self.transfer_crafting_result(&grid).await?;
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
        Ok(CraftOnceRecord { fill, take })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn item_names_default_to_the_minecraft_namespace() {
        assert_eq!(super::item_name("oak_planks"), "minecraft:oak_planks");
        assert_eq!(super::item_name("minecraft:stick"), "minecraft:stick");
    }
}
