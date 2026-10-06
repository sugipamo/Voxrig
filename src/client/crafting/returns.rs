//! Read-only inventory return simulation for the entire received crafting grid.
use super::{CraftingSource, ReceivedCraftingContext};
use crate::{
    Result,
    client::{
        ItemStack, ObservedValue, SessionStamp, SlotKnowledge, ValueSource,
        inventory::{ItemContext, unavailable},
        registry::ServerRegistryStamp,
    },
};

/// One hypothetical input-to-player transfer. It is not a dispatched click.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftingGridReturnStep {
    input: [usize; 2],
    player_slot: usize,
    amount: u32,
}
impl CraftingGridReturnStep {
    /// Original input x/y in the captured grid.
    pub fn input(&self) -> [usize; 2] {
        self.input
    }
    /// Canonical player-screen destination, including offhand 45 for merging.
    pub fn player_slot(&self) -> usize {
        self.player_slot
    }
    /// Predicted transferred count, not an observed inventory change.
    pub fn amount(&self) -> u32 {
        self.amount
    }
}
/// A hypothetical native source split that cannot fully insert at its chosen
/// destination. The caller does not reattach that copy. This is an unsafe return
/// diagnostic, never a received loss or permission to clear an input.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftingGridUnreturnedSplit {
    input: [usize; 2],
    player_slot: usize,
    amount: u32,
}
impl CraftingGridUnreturnedSplit {
    /// Original input x/y.
    pub fn input(&self) -> [usize; 2] {
        self.input
    }
    /// Fixed canonical player destination chosen by the native return path.
    pub fn player_slot(&self) -> usize {
        self.player_slot
    }
    /// Copy count that cannot safely return, not an actual loss receipt.
    pub fn amount(&self) -> u32 {
        self.amount
    }
}
/// Hypothetical inventory capacity after returning all current crafting inputs.
/// Uses native selected/offhand/main merge order and first-free main/hotbar order,
/// with resolved native item data and effective capacity. Cursor/result/armor do
/// not contribute. This does not send, authorize clearing, or predict drops.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CraftingGridReturnPlan {
    session: SessionStamp,
    receive_sequence: u64,
    registry_owner: ServerRegistryStamp,
    source: CraftingSource,
    selected_hotbar: ObservedValue<u8>,
    steps: Vec<CraftingGridReturnStep>,
    predictions: Vec<(usize, ObservedValue<SlotKnowledge>)>,
    remaining: Vec<([usize; 2], ItemStack)>,
    unreturned_splits: Vec<CraftingGridUnreturnedSplit>,
}
impl CraftingGridReturnPlan {
    /// Original coherent world/transport.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Original packet boundary; predictions are never actual receipts.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Actual registry configuration owning all data used in the simulation.
    pub fn registry_owner(&self) -> ServerRegistryStamp {
        self.registry_owner
    }
    /// Original input UI/opening.
    pub fn source(&self) -> CraftingSource {
        self.source
    }
    /// Selected-slot basis used in the hypothetical native search. A completed
    /// local submission stays Submitted, never a received server acknowledgement.
    pub fn selected_hotbar(&self) -> &ObservedValue<u8> {
        &self.selected_hotbar
    }
    /// Whether every input can return under this selection basis without
    /// requiring a drop or leaving an unreturned split.
    pub fn fits(&self) -> bool {
        self.remaining.is_empty()
    }
    /// Hypothetical ordered transfers, potentially partial when fits() is false.
    pub fn steps(&self) -> &[CraftingGridReturnStep] {
        &self.steps
    }
    /// Canonical main/hotbar/offhand inventory predictions, all explicitly Predicted.
    pub fn predictions(&self) -> &[(usize, ObservedValue<SlotKnowledge>)] {
        &self.predictions
    }
    /// Native fixed-destination copies that cannot completely insert. Other
    /// free slots do not repair this caller path; fits() is false even then.
    pub fn unreturned_splits(&self) -> &[CraftingGridUnreturnedSplit] {
        &self.unreturned_splits
    }
    /// Inputs left without safe capacity, including unreturned native split
    /// copies. No actual loss or drop is claimed.
    pub fn remaining(&self) -> &[([usize; 2], ItemStack)] {
        &self.remaining
    }
}
fn valid(item: &ItemStack) -> Result<u32> {
    let properties = item.properties()?;
    let capacity = u32::try_from(properties.max_stack_size)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| unavailable("crafting return capacity must be positive"))?;
    if item.count == 0 || item.count > i32::MAX as u32 {
        return Err(unavailable(
            "crafting return requires valid native stack counts",
        ));
    }
    Ok(capacity)
}
pub(super) fn capture(context: &ReceivedCraftingContext) -> Result<CraftingGridReturnPlan> {
    let inventory = context.inventory();
    let grid = context.grid();
    let version = context.session().version;
    let semantic = ItemContext::new(inventory.registry_state().clone(), context.player())?;
    let selected_hotbar = context
        .player()
        .selected_hotbar
        .as_ref()
        .filter(|v| {
            v.value < 9
                && match v.source {
                    ValueSource::Received { sequence } => {
                        sequence <= context.receive_sequence()
                            && sequence
                                >= inventory.registry_state().stamp().configuration_generation
                    }
                    ValueSource::Submitted => !context.player().pending_dispatch,
                    ValueSource::Predicted | ValueSource::LocalCache => false,
                }
        })
        .cloned()
        .ok_or_else(|| {
            unavailable("crafting return requires a received or resolved submitted selected hotbar")
        })?;
    let selected = selected_hotbar.value;
    // Native Inventory indices: hotbar 0..8, main 9..35, offhand 40. Keep its
    // ordering while retaining public canonical player-screen indices throughout.
    let order = (36..45).chain(9..36).collect::<Vec<_>>();
    let mut slots = std::collections::BTreeMap::new();
    for index in 9..46 {
        let slot = inventory
            .slot(index)?
            .ok_or_else(|| unavailable("crafting return destination has not been received"))?;
        match slot.value() {
            SlotKnowledge::Empty => {
                slots.insert(index, None);
            }
            SlotKnowledge::Item { item } => {
                valid(item)?;
                slots.insert(index, Some(item.clone()));
            }
            SlotKnowledge::Unavailable => {
                return Err(unavailable("crafting return destination is unavailable"));
            }
        }
    }
    let mut steps = Vec::new();
    let mut remaining = Vec::new();
    let mut unreturned_splits = Vec::new();
    let inventory_limit = if version == crate::MinecraftVersion::Java1_16_1 {
        64
    } else {
        99
    };
    let [width, height] = grid.dimensions();
    for y in 0..height {
        for x in 0..width {
            let input = [x, y];
            let receipt = grid
                .input(x, y)?
                .ok_or_else(|| unavailable("crafting return input has not been received"))?;
            let mut item = match receipt.value() {
                SlotKnowledge::Empty => continue,
                SlotKnowledge::Item { item } => {
                    valid(item)?;
                    item.clone()
                }
                SlotKnowledge::Unavailable => {
                    return Err(unavailable("crafting return input is unavailable"));
                }
            };
            let mut unreturned = 0;
            while item.count > 0 {
                let mut destination = None;
                // Native merge search starts at selected, then offhand, then main list.
                for index in std::iter::once(36 + usize::from(selected))
                    .chain(std::iter::once(45))
                    .chain(order.iter().copied())
                {
                    let Some(existing) = slots[&index].as_ref() else {
                        continue;
                    };
                    if existing.id != item.id {
                        continue;
                    }
                    let properties = existing.properties()?;
                    if properties.stackable
                        && existing.count < valid(existing)?.min(inventory_limit)
                        && semantic.same_data(existing, &item)?
                    {
                        destination = Some(index);
                        break;
                    }
                }
                if destination.is_none() {
                    destination = order.iter().copied().find(|index| slots[index].is_none());
                }
                let Some(index) = destination else {
                    break;
                };
                let destination_count = slots[&index].as_ref().map_or(0, |v| v.count);
                let source_capacity = valid(&item)?;
                let source_damaged = item.properties()?.damaged;
                // Legacy recipe-book clearing offers one copied unit at a time.
                // Modern placeItemBack splits by raw item capacity, then inserts
                // repeatedly at this fixed destination. It does not retry a copy
                // at another slot when the inventory limit leaves a remainder.
                let offered = if version == crate::MinecraftVersion::Java1_16_1 {
                    1
                } else {
                    item.count
                        .min(source_capacity.saturating_sub(destination_count))
                };
                if offered == 0 {
                    return Err(unavailable("crafting return native split made no progress"));
                }
                item.count -= offered;
                let mut left = offered;
                while left > 0 {
                    let amount = if source_damaged {
                        // Original successful Inventory.add uses copyAndClear for
                        // damaged stacks, preserving the entire offered split.
                        let mut placed = item.clone();
                        placed.count = left;
                        slots.insert(index, Some(placed));
                        left
                    } else if let Some(existing) = slots.get_mut(&index).expect("known destination")
                    {
                        let amount = left.min(
                            valid(existing)?
                                .min(inventory_limit)
                                .saturating_sub(existing.count),
                        );
                        existing.count += amount;
                        amount
                    } else {
                        // Modern addResource's initial empty getter capacity is
                        // one; its next fixed-destination pass merges the rest.
                        let amount = if version == crate::MinecraftVersion::Java1_21_11 {
                            1.min(left)
                        } else {
                            left.min(source_capacity.min(inventory_limit))
                        };
                        let mut placed = item.clone();
                        placed.count = amount;
                        slots.insert(index, Some(placed));
                        amount
                    };
                    if amount == 0 {
                        break;
                    }
                    left -= amount;
                    steps.push(CraftingGridReturnStep {
                        input,
                        player_slot: index,
                        amount,
                    });
                }
                if left > 0 {
                    unreturned += left;
                    unreturned_splits.push(CraftingGridUnreturnedSplit {
                        input,
                        player_slot: index,
                        amount: left,
                    });
                }
            }
            item.count += unreturned;
            if item.count > 0 {
                remaining.push((input, item));
            }
        }
    }
    let predictions = slots
        .into_iter()
        .map(|(index, item)| {
            (
                index,
                ObservedValue {
                    source: ValueSource::Predicted,
                    value: item.map_or(SlotKnowledge::Empty, |item| SlotKnowledge::Item { item }),
                },
            )
        })
        .collect();
    Ok(CraftingGridReturnPlan {
        session: context.session(),
        receive_sequence: context.receive_sequence(),
        registry_owner: inventory.registry_state().stamp(),
        source: grid.source(),
        selected_hotbar,
        steps,
        predictions,
        remaining,
        unreturned_splits,
    })
}
#[cfg(test)]
mod tests;
