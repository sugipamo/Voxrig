//! Coherent recipe/inventory/grid observations for read-only crafting planning.
use super::{ReceivedCrafting, ReceivedRecipes, RecipeDisplay, RecipeId, RecipeSlotDisplay};
use crate::{
    Result,
    client::{
        PlayerObservation, ReceivedInventory, SessionStamp, container::ScreenObservation,
        registry::ServerRegistryObservation,
    },
};

/// Recipes, main inventory and current crafting grid from one adapter boundary.
/// Inventory/grid values exclude local predictions. The player view retains
/// explicitly tagged submitted/model values; separately captured components
/// cannot construct this context.
/// Planning from it neither sends packets nor authorizes an operation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedCraftingContext {
    player: PlayerObservation,
    inventory: ReceivedInventory,
    grid: ReceivedCrafting,
    recipes: ReceivedRecipes,
}
impl ReceivedCraftingContext {
    /// Transport/world shared by every captured component.
    pub fn session(&self) -> SessionStamp {
        self.player.session
    }
    /// Applied packet boundary; slots retain individual receipt ordinals.
    pub fn receive_sequence(&self) -> u64 {
        self.player.receive_sequence
    }
    /// Coherent player/mode/pose facts, separate from operation permission.
    pub fn player(&self) -> &PlayerObservation {
        &self.player
    }
    /// Actual received player inventory, including offhand and cursor.
    pub fn inventory(&self) -> &ReceivedInventory {
        &self.inventory
    }
    /// Actual player/table inputs/result and owning opening identity.
    pub fn grid(&self) -> &ReceivedCrafting {
        &self.grid
    }
    /// Actual received recipe catalogue and registry/tag owner.
    pub fn recipes(&self) -> &ReceivedRecipes {
        &self.recipes
    }
    /// Validated display arrangement using the original recipe-book geometry.
    /// It is a display layout, not ingredient membership or a placement prediction.
    pub fn recipe_layout(&self, recipe: &RecipeId) -> Result<super::RecipeCraftingLayout> {
        super::layout::capture(self, recipe)
    }
    /// Simulate returning every received input without treating unknown slots as
    /// free space. Partial predictions do not authorize clearing when fits() is false.
    pub fn grid_return_plan(&self) -> Result<super::CraftingGridReturnPlan> {
        super::returns::capture(self)
    }
    /// Coherent recipe-book planning, including the existing grid, safe returns,
    /// native material accounting and data-compatible post-return sources.
    /// Plans remain read-only and never convert predictions into receipts.
    pub fn recipe_placement_plan(
        &self,
        recipe: &RecipeId,
        amount: super::RecipePlacementAmount,
    ) -> Result<super::RecipePlacementPlan> {
        super::placement::capture(self, recipe, amount)
    }
    pub(crate) fn capture(
        player: PlayerObservation,
        screen: ScreenObservation,
        registries: ServerRegistryObservation,
        recipes: ReceivedRecipes,
    ) -> Result<Option<Self>> {
        if player.session != recipes.session()
            || player.receive_sequence != recipes.receive_sequence()
            || registries.stamp() != recipes.registry_state().stamp()
        {
            return Err(crate::client::inventory::unavailable(
                "crafting context capture boundaries differ",
            ));
        }
        let Some(grid) = ReceivedCrafting::capture(&player, &screen, registries.clone())? else {
            return Ok(None);
        };
        let inventory = ReceivedInventory::capture(
            player.session,
            player.receive_sequence,
            &player.inventory,
            registries,
        )?;
        Ok(Some(Self {
            player,
            inventory,
            grid,
            recipes,
        }))
    }
}

pub(super) fn display_cells(
    display: &RecipeDisplay,
    grid: [usize; 2],
) -> Result<([usize; 2], &[RecipeSlotDisplay])> {
    let fail = || {
        crate::client::inventory::unavailable(
            "recipe display does not fit the received crafting grid",
        )
    };
    match display {
        RecipeDisplay::Shaped {
            width,
            height,
            ingredients,
            ..
        } => {
            let w = usize::try_from(*width).map_err(|_| fail())?;
            let h = usize::try_from(*height).map_err(|_| fail())?;
            if w == 0 || h == 0 || w > grid[0] || h > grid[1] || ingredients.len() != w * h {
                return Err(fail());
            }
            Ok(([w, h], ingredients))
        }
        RecipeDisplay::Shapeless { ingredients, .. } => {
            if ingredients.is_empty() || ingredients.len() > grid[0] * grid[1] {
                return Err(fail());
            }
            Ok((grid, ingredients))
        }
        _ => Err(fail()),
    }
}

#[cfg(test)]
mod tests;
