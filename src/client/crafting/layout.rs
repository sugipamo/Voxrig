//! Validated native recipe display placement, distinct from a mutation plan.
use super::{CraftingSource, ReceivedCraftingContext, RecipeId, RecipeSlotDisplay};
use crate::{
    Result,
    client::{SessionStamp, inventory::InventorySource},
};

/// One displayed recipe cell mapped into the captured native player/table UI.
/// Empty display cells are retained. Displays are not received inventory stacks.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipeCraftingCell {
    display_index: usize,
    coordinate: [usize; 2],
    source: InventorySource,
    slot: u16,
    display: RecipeSlotDisplay,
}
impl RecipeCraftingCell {
    /// Index in the original shaped row-major or shapeless display list.
    pub fn display_index(&self) -> usize {
        self.display_index
    }
    /// Zero-based x/y in this captured crafting grid.
    pub fn coordinate(&self) -> [usize; 2] {
        self.coordinate
    }
    /// Actual opening-bound UI/slot, not permission to click it.
    pub fn input_source(&self) -> (InventorySource, u16) {
        (self.source, self.slot)
    }
    /// Original displayed choices; not ingredient membership or an input receipt.
    pub fn display(&self) -> &RecipeSlotDisplay {
        &self.display
    }
}
/// Original native geometry applied to a received recipe display and actual grid.
/// Only a coherent received context can create it. Valid geometry does not prove
/// that displayed choices match requirements or that a recipe can be placed.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipeCraftingLayout {
    session: SessionStamp,
    receive_sequence: u64,
    recipe: RecipeId,
    source: CraftingSource,
    grid_dimensions: [usize; 2],
    recipe_dimensions: [usize; 2],
    cells: Vec<RecipeCraftingCell>,
}
impl RecipeCraftingLayout {
    /// World/transport at the original coherent context.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Capture boundary; does not refresh individual grid input receipts.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Recipe entry ordinal and registry owner.
    pub fn recipe(&self) -> &RecipeId {
        &self.recipe
    }
    /// Original received grid's UI admission/opening identity.
    pub fn source(&self) -> CraftingSource {
        self.source
    }
    /// Received player 2x2 or table 3x3 dimensions.
    pub fn grid_dimensions(&self) -> [usize; 2] {
        self.grid_dimensions
    }
    /// Shaped dimensions, or the whole target grid for shapeless displays.
    pub fn recipe_dimensions(&self) -> [usize; 2] {
        self.recipe_dimensions
    }
    /// Ordered native placements, retaining shaped empty display cells.
    pub fn cells(&self) -> &[RecipeCraftingCell] {
        &self.cells
    }
}
pub(super) fn capture(
    context: &ReceivedCraftingContext,
    recipe: &RecipeId,
) -> Result<RecipeCraftingLayout> {
    let entry = context.recipes().entry(recipe)?;
    let grid = context.grid();
    let dimensions = grid.dimensions();
    let (recipe_dimensions, displays) = super::context::display_cells(entry.display(), dimensions)?;
    let positions = positions(dimensions, recipe_dimensions, displays.len());
    let mut cells = Vec::with_capacity(displays.len());
    for (display_index, coordinate) in positions.into_iter().enumerate() {
        let (source, slot) = grid.input_source(coordinate[0], coordinate[1])?;
        cells.push(RecipeCraftingCell {
            display_index,
            coordinate,
            source,
            slot,
            display: displays[display_index].clone(),
        });
    }
    Ok(RecipeCraftingLayout {
        session: context.session(),
        receive_sequence: context.receive_sequence(),
        recipe: entry.id().clone(),
        source: grid.source(),
        grid_dimensions: dimensions,
        recipe_dimensions,
        cells,
    })
}
/// Audited input QUICK_MOVE branches. Results have their own take owner.
/// Ranges exclude armor/offhand even when the input item is equippable.
pub(crate) struct InputTransferRoute {
    menu: &'static str,
    pub(crate) inputs: std::ops::Range<usize>,
    pub(crate) destinations: std::ops::Range<usize>,
    pub(crate) result_slot: usize,
    pub(crate) total_slots: usize,
}
const INPUT_TRANSFERS: [InputTransferRoute; 2] = [
    InputTransferRoute {
        menu: "minecraft:player",
        inputs: 1..5,
        destinations: 9..45,
        result_slot: 0,
        total_slots: 46,
    },
    InputTransferRoute {
        menu: "minecraft:crafting",
        inputs: 1..10,
        destinations: 10..46,
        result_slot: 0,
        total_slots: 46,
    },
];
pub(crate) fn input_transfer(
    version: crate::MinecraftVersion,
    menu: &str,
    source: usize,
) -> Option<&'static InputTransferRoute> {
    let route = INPUT_TRANSFERS
        .iter()
        .find(|r| r.menu == menu && r.inputs.contains(&source))?;
    let native = super::native_menu(version, menu)?;
    (native.total_slots == route.total_slots
        && native.result_slot == route.result_slot
        && native.input_slots.iter().any(|s| s.screen_slot == source)
        && route.destinations.clone().all(|slot| {
            native
                .player_slots
                .iter()
                .any(|s| s.screen_slot == slot && s.raw_player_slot < 36)
        }))
    .then_some(route)
}
/// Original SWAP has a distinct hotbar return destination and never routes
/// through armor, result onTake, or item PICKUP overrides.
struct InputSwapRoute {
    menu: &'static str,
    inputs: std::ops::Range<usize>,
    hotbar_start: usize,
}
const INPUT_SWAPS: [InputSwapRoute; 2] = [
    InputSwapRoute {
        menu: "minecraft:player",
        inputs: 1..5,
        hotbar_start: 36,
    },
    InputSwapRoute {
        menu: "minecraft:crafting",
        inputs: 1..10,
        hotbar_start: 37,
    },
];
pub(crate) fn input_swap(
    version: crate::MinecraftVersion,
    menu: &str,
    input: usize,
    hotbar: u8,
) -> Option<usize> {
    let route = INPUT_SWAPS
        .iter()
        .find(|r| r.menu == menu && r.inputs.contains(&input))?;
    let native = super::native_menu(version, menu)?;
    let destination = route.hotbar_start + usize::from(hotbar);
    (hotbar < 9
        && input != native.result_slot
        && native.input_slots.iter().any(|s| s.screen_slot == input)
        && native
            .player_slots
            .iter()
            .any(|s| s.screen_slot == destination && s.raw_player_slot == usize::from(hotbar)))
    .then_some(destination)
}
// Original center rule only centers when the recipe is smaller than half the
// grid. In particular 1x1 is at (0,0) in 2x2, but (1,1) in 3x3.
fn positions(grid: [usize; 2], recipe: [usize; 2], entries: usize) -> Vec<[usize; 2]> {
    let offset = std::array::from_fn::<_, 2, _>(|axis| {
        if recipe[axis] * 2 < grid[axis] {
            (grid[axis] - recipe[axis]) / 2
        } else {
            0
        }
    });
    (0..entries)
        .map(|i| [offset[0] + i % recipe[0], offset[1] + i / recipe[0]])
        .collect()
}
#[cfg(test)]
mod tests;
