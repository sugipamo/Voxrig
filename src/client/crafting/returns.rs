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
    steps: Vec<CraftingGridReturnStep>,
    predictions: Vec<(usize, ObservedValue<SlotKnowledge>)>,
    remaining: Vec<([usize; 2], ItemStack)>,
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
    /// Whether every input can return without requiring a drop or loss.
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
    /// Inputs left without capacity in this simulation. No drop is predicted.
    pub fn remaining(&self) -> &[([usize; 2], ItemStack)] {
        &self.remaining
    }
}
fn valid(item: &ItemStack, version: crate::MinecraftVersion) -> Result<u32> {
    let properties = item.properties()?;
    let capacity = u32::try_from(properties.max_stack_size)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| unavailable("crafting return capacity must be positive"))?
        .min(match version {
            crate::MinecraftVersion::Java1_16_1 => 64,
            crate::MinecraftVersion::Java1_21_11 => 99,
        });
    if item.count == 0 || item.count > capacity {
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
    let selected=context.player().selected_hotbar.as_ref().filter(|v|matches!(v.source,ValueSource::Received {sequence} if sequence <= context.receive_sequence() && sequence >= inventory.registry_state().stamp().configuration_generation))
        .filter(|v|v.value<9).ok_or_else(||unavailable("crafting return requires received selected hotbar"))?.value;
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
                valid(item, version)?;
                slots.insert(index, Some(item.clone()));
            }
            SlotKnowledge::Unavailable => {
                return Err(unavailable("crafting return destination is unavailable"));
            }
        }
    }
    let mut steps = Vec::new();
    let mut remaining = Vec::new();
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
                    valid(item, version)?;
                    item.clone()
                }
                SlotKnowledge::Unavailable => {
                    return Err(unavailable("crafting return input is unavailable"));
                }
            };
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
                        && existing.count < valid(existing, version)?
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
                let amount =
                    if let Some(existing) = slots.get_mut(&index).expect("known destination") {
                        let amount = item.count.min(valid(existing, version)? - existing.count);
                        existing.count += amount;
                        amount
                    } else {
                        // Modern addResource copies an empty stack whose capacity getter is
                        // initially 1; a later native pass merges the remaining amount.
                        // Damaged stacks take Inventory.add's first-free whole-stack path.
                        let amount = if version == crate::MinecraftVersion::Java1_21_11
                            && !item.properties()?.damaged
                        {
                            1.min(item.count)
                        } else {
                            item.count.min(valid(&item, version)?)
                        };
                        let mut placed = item.clone();
                        placed.count = amount;
                        slots.insert(index, Some(placed));
                        amount
                    };
                if amount == 0 {
                    return Err(unavailable(
                        "crafting return made no native capacity progress",
                    ));
                }
                item.count -= amount;
                steps.push(CraftingGridReturnStep {
                    input,
                    player_slot: index,
                    amount,
                });
            }
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
        steps,
        predictions,
        remaining,
    })
}
#[cfg(test)]
mod tests;
