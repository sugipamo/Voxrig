//! Coherent recipe-book preflight; native choice tie ordering remains server-owned.
use super::{CraftingGridReturnPlan, ReceivedCraftingContext, RecipeCraftingLayout, RecipeId};
use crate::{
    MinecraftVersion, Result,
    client::{
        GameMode, ItemStack, SessionStamp, SlotKnowledge, ValueSource,
        inventory::{ItemContext, unavailable},
        registry::Registry,
    },
};
use std::collections::BTreeMap;

/// Native recipe-book request intent. Next increases an already matched grid;
/// it does not request an arbitrary exact number of crafted outputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipePlacementAmount {
    /// One batch on an unmatched grid, or one more than its minimum input count.
    Next,
    /// Native material maximum, subject to original matching and item/UI caps.
    Maximum,
}
/// Sealed historical recipe-book preflight, distinct from dispatch or actual
/// placement. Materials include the received grid. Post-return source checks
/// retain data equality and exclude offhand/cursor/armor from ingredient lookup.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipePlacementPlan {
    session: SessionStamp,
    receive_sequence: u64,
    recipe: RecipeId,
    amount: RecipePlacementAmount,
    mode: GameMode,
    layout: RecipeCraftingLayout,
    grid_return: CraftingGridReturnPlan,
    grid_matches: bool,
    material_maximum: u32,
    requested_crafts: u32,
    matched_capacity: bool,
    source_data_safe: bool,
    unlocked: bool,
    #[serde(skip)]
    pub(super) targets: std::collections::BTreeSet<u32>,
    #[serde(skip)]
    context: ReceivedCraftingContext,
}
impl RecipePlacementPlan {
    /// Original transport/world.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Applied packet boundary, not renewed individual slot freshness.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Original entry identity; native display IDs can later be reused.
    pub fn recipe(&self) -> &RecipeId {
        &self.recipe
    }
    /// Requested native Next/Maximum intent.
    pub fn amount(&self) -> RecipePlacementAmount {
        self.amount
    }
    /// Captured received survival/creative mode.
    pub fn mode(&self) -> GameMode {
        self.mode
    }
    /// Validated native target geometry and exact original UI opening.
    pub fn layout(&self) -> &RecipeCraftingLayout {
        &self.layout
    }
    /// Hypothetical return of the entire actual current grid, excluding cursor.
    pub fn grid_return(&self) -> &CraftingGridReturnPlan {
        &self.grid_return
    }
    /// Original ingredient-based grid match, including offsets/mirrors/empty cells.
    pub fn grid_matches(&self) -> bool {
        self.grid_matches
    }
    /// Native type-level maximum before default UI caps and source data lookup.
    /// It is neither output count nor an exact server placement prediction.
    pub fn material_maximum(&self) -> u32 {
        self.material_maximum
    }
    /// Native request count before selected types' default caps. Zero denotes
    /// an insufficient-material/no-increase case. Native type ties remain unknown.
    pub fn requested_crafts(&self) -> u32 {
        self.requested_crafts
    }
    /// Every nonempty matched input can increase under native material/effective caps.
    /// Original handlers apply this guard even to Maximum requests.
    pub fn matched_capacity(&self) -> bool {
        self.matched_capacity
    }
    /// Conservative source safety across native choices/cap reductions. Main
    /// sources must fill cells without mixing incompatible data within a cell.
    /// False is a diagnostic requiring a different arrangement, never permission.
    pub fn source_data_safe(&self) -> bool {
        self.source_data_safe
    }
    /// Actual captured book membership, separately from material availability.
    pub fn unlocked(&self) -> bool {
        self.unlocked
    }
    /// Captured facts pass recipe-book preflight. This read-only result does not
    /// reserve or authorize a future operation; the live owner must recheck it.
    pub fn can_place(&self) -> bool {
        self.unlocked
            && self.requested_crafts > 0
            && self.matched_capacity
            && self.grid_return.fits()
            && self.source_data_safe
    }
    /// Coherent actual facts retained behind the historical plan. Returned-slot
    /// predictions remain separate in grid_return(), never received inventory.
    pub fn source_context(&self) -> &ReceivedCraftingContext {
        &self.context
    }
}
fn add(amounts: &mut BTreeMap<i32, i32>, id: i32, count: i32) -> Result<()> {
    if count < 0 {
        return Err(unavailable("negative native crafting contribution"));
    }
    let total = amounts.entry(id).or_default();
    *total = total
        .checked_add(count)
        .ok_or_else(|| unavailable("native crafting stock overflow"))?;
    Ok(())
}
pub(super) fn capture(
    context: &ReceivedCraftingContext,
    recipe: &RecipeId,
    amount: RecipePlacementAmount,
) -> Result<RecipePlacementPlan> {
    let player = context.player();
    let mode = player
        .game_mode
        .filter(|m| matches!(m, GameMode::Survival | GameMode::Creative))
        .ok_or_else(|| {
            unavailable("recipe placement requires a received survival/creative mode")
        })?;
    if player.pending_dispatch {
        return Err(unavailable("recipe placement has an unresolved mutation"));
    }
    let entry = context.recipes().entry(recipe)?;
    let unlocked = entry
        .unlocked()
        .ok_or_else(|| unavailable("recipe book membership has not been received"))?;
    let cursor = context
        .inventory()
        .cursor()
        .ok_or_else(|| unavailable("actual crafting cursor required"))?;
    if *cursor.value() != SlotKnowledge::Empty {
        return Err(unavailable(
            "return the actual cursor before recipe placement",
        ));
    }
    let pattern = super::matching::resolve(context, recipe)?;
    let grid = super::matching::grid(context)?;
    let grid_matches = super::matching::matches(
        &pattern,
        context.grid().dimensions(),
        &grid,
        context.session().version,
    );
    let mut amounts = BTreeMap::new();
    for index in 9..45 {
        let slot = context
            .inventory()
            .slot(index)?
            .ok_or_else(|| unavailable("recipe inventory slot has not been received"))?;
        match slot.value() {
            SlotKnowledge::Empty => {}
            SlotKnowledge::Item { item } => add(
                &mut amounts,
                item.id.value(),
                slot.item().expect("item").recipe_book_stock()?.count(),
            )?,
            _ => return Err(unavailable("recipe inventory item is unavailable")),
        }
    }
    let [w, h] = context.grid().dimensions();
    let mut inputs = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if let SlotKnowledge::Item { item } = context
                .grid()
                .input(x, y)?
                .expect("matched grid knowledge")
                .value()
            {
                add(
                    &mut amounts,
                    item.id.value(),
                    super::stock::grid_count(item)?,
                )?;
                inputs.push(item);
            }
        }
    }
    // Bound matching CPU by stocked types, including when a native tag has many members.
    let choices = pattern
        .ingredients
        .iter()
        .map(|ids| {
            ids.iter()
                .copied()
                .filter(|id| amounts.get(id).is_some_and(|v| *v > 0))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let maximum = super::materials::maximum(&choices, &amounts, i32::MAX as u32);
    let matched_capacity = !grid_matches
        || inputs.iter().all(|item| {
            item.properties().is_ok_and(|p| {
                p.max_stack_size > 0 && maximum.min(p.max_stack_size as u32) > item.count
            })
        });
    let requested_crafts = if maximum == 0 || !matched_capacity {
        0
    } else {
        match amount {
            RecipePlacementAmount::Maximum => maximum,
            RecipePlacementAmount::Next if !grid_matches => 1,
            RecipePlacementAmount::Next => {
                let minimum = inputs
                    .iter()
                    .map(|item| item.count)
                    .min()
                    .expect("matched nonempty pattern");
                if context.session().version == MinecraftVersion::Java1_16_1 {
                    minimum.min(64).saturating_add(u32::from(minimum < 64))
                } else {
                    minimum
                        .checked_add(1)
                        .ok_or_else(|| unavailable("native matched grid count overflow"))?
                }
            }
        }
    };
    let layout = context.recipe_layout(recipe)?;
    let grid_return = context.grid_return_plan()?;
    let targets = target_counts(context, &choices, &amounts, requested_crafts)?;
    let source_data_safe = grid_return.fits()
        && requested_crafts > 0
        && source_safety(context, &grid_return, &choices, &amounts, &targets)?;
    Ok(RecipePlacementPlan {
        session: context.session(),
        receive_sequence: context.receive_sequence(),
        recipe: entry.id().clone(),
        amount,
        mode,
        layout,
        grid_return,
        grid_matches,
        material_maximum: maximum,
        requested_crafts,
        matched_capacity,
        source_data_safe,
        unlocked,
        targets,
        context: context.clone(),
    })
}
// Native first eligible inventory stack fixes a cell's data, and later lookup
// requires exact item/data equality. Keep first-seen groups in raw hotbar/main
// inventory order. Type-only availability must never imply data-compatible fill.
fn source_safety(
    context: &ReceivedCraftingContext,
    returned: &CraftingGridReturnPlan,
    choices: &[Vec<i32>],
    amounts: &BTreeMap<i32, i32>,
    targets: &std::collections::BTreeSet<u32>,
) -> Result<bool> {
    let semantic = ItemContext::new(
        context.inventory().registry_state().clone(),
        context.player(),
    )?;
    let mut groups = BTreeMap::<i32, Vec<(ItemStack, u64)>>::new();
    for index in (36..45).chain(9..36) {
        let slot = returned
            .predictions()
            .iter()
            .find(|(slot, _)| *slot == index)
            .expect("complete return destinations");
        debug_assert_eq!(slot.1.source, ValueSource::Predicted);
        let SlotKnowledge::Item { item } = &slot.1.value else {
            continue;
        };
        let (damaged, enchanted, named, _) = super::stock::native_facts(item)?;
        if damaged || enchanted || named {
            continue;
        }
        let variants = groups.entry(item.id.value()).or_default();
        let mut found = None;
        for (i, (other, _)) in variants.iter().enumerate() {
            if semantic.same_data(item, other)? {
                found = Some(i);
                break;
            }
        }
        if let Some(i) = found {
            variants[i].1 += u64::from(item.count);
        } else {
            variants.push((item.clone(), u64::from(item.count)));
        }
    }
    // Native selection can clamp and repick at a smaller count, with different
    // choices. Only selected default caps/requested count can become the final
    // count; intermediate numbers are not native clamp outcomes. This is a sufficient safety check;
    // it does not assert the deterministic local assignment is the native tie.
    for &crafts in targets {
        if crafts == 0 {
            return Err(unavailable(
                "native default crafting capacity must be positive",
            ));
        }
        if super::materials::assign(choices, amounts, crafts).is_none() {
            continue;
        }
        for (&id, &stock) in amounts {
            let ingredients = choices.iter().filter(|ids| ids.contains(&id)).count();
            let cells = ingredients.min(stock.max(0) as usize / crafts as usize);
            if cells == 0 {
                continue;
            }
            let mut groups = groups
                .get(&id)
                .map(|g| g.iter().map(|(_, count)| *count).collect::<Vec<_>>())
                .unwrap_or_default();
            for _ in 0..cells {
                let Some(group) = groups.iter_mut().find(|count| **count > 0) else {
                    return Ok(false);
                };
                if *group < u64::from(crafts) {
                    return Ok(false);
                }
                *group -= u64::from(crafts);
            }
        }
    }
    Ok(true)
}

fn target_counts(
    context: &ReceivedCraftingContext,
    choices: &[Vec<i32>],
    amounts: &BTreeMap<i32, i32>,
    requested: u32,
) -> Result<std::collections::BTreeSet<u32>> {
    if requested == 0 {
        return Ok(Default::default());
    }
    let registry = Registry::for_version(context.session().version);
    let targets = choices
        .iter()
        .flatten()
        .filter(|id| {
            amounts
                .get(id)
                .is_some_and(|count| *count >= requested as i32)
        })
        .map(|id| {
            registry
                .item_by_native_id(*id)
                .map(|item| item.max_stack_size.min(requested))
        })
        .collect::<Result<std::collections::BTreeSet<_>>>()?;
    Ok(targets
        .into_iter()
        .filter(|&n| n > 0 && super::materials::assign(choices, amounts, n).is_some())
        .collect())
}

#[cfg(test)]
mod tests;
