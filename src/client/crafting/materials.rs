//! Recipe-book inventory feasibility, separate from grid placement/consumption.
use super::{ReceivedRecipes, RecipeBookStock, RecipeDisplay, RecipeId};
use crate::Result;
use crate::client::{ReceivedInventory, SessionStamp, SlotKnowledge, registry::RegistryId};
use std::collections::BTreeMap;

fn invalid(message: impl std::fmt::Display) -> crate::Error {
    crate::client::inventory::unavailable(message)
}

/// Immutable recipe-book material assignment from received main/hotbar stock.
/// This excludes grid, cursor, armor and offhand. It does not authorize placement,
/// guarantee recipe unlock, account for return space, or predict consumption.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipeBookMaterials {
    session: SessionStamp,
    receive_sequence: u64,
    recipe: RecipeId,
    unlocked: Option<bool>,
    stocks: Vec<(usize, RecipeBookStock)>,
    crafts: u32,
    assignment: Option<Vec<RegistryId>>,
    maximum_bound: u32,
    maximum: u32,
}
impl RecipeBookMaterials {
    /// World/transport of the coherent catalogue/inventory capture.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Applied packet boundary; individual stacks retain their receipt ordinals.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Original entry identity, not a placement authorization.
    pub fn recipe(&self) -> &RecipeId {
        &self.recipe
    }
    /// Book membership at capture, separate from material availability.
    pub fn unlocked(&self) -> Option<bool> {
        self.unlocked
    }
    /// Player-screen indices 9..44 and their native simple-stock contributions.
    /// Actually empty slots are omitted; excluded stacks retain zero contributions.
    pub fn stocks(&self) -> &[(usize, RecipeBookStock)] {
        &self.stocks
    }
    /// Requested positive batch size.
    pub fn crafts(&self) -> u32 {
        self.crafts
    }
    /// One item type per declared ingredient, or None when stock is insufficient.
    /// All copies use that type; choices are deterministic, not native tie ordering.
    pub fn assignment(&self) -> Option<&[RegistryId]> {
        self.assignment.as_deref()
    }
    /// Caller-supplied positive bound for material-only maximum search.
    pub fn maximum_bound(&self) -> u32 {
        self.maximum_bound
    }
    /// Feasible inventory-only batches up to maximum_bound, not output item count.
    pub fn maximum(&self) -> u32 {
        self.maximum
    }

    pub(crate) fn capture(
        catalogue: ReceivedRecipes,
        inventory: ReceivedInventory,
        recipe: &RecipeId,
        crafts: u32,
        maximum_bound: u32,
    ) -> Result<Self> {
        if crafts == 0
            || maximum_bound == 0
            || crafts > i32::MAX as u32
            || maximum_bound > i32::MAX as u32
        {
            return Err(invalid(
                "recipe material batch/bound must be positive native integers",
            ));
        }
        if catalogue.session() != inventory.session()
            || catalogue.receive_sequence() != inventory.receive_sequence()
            || catalogue.registry_state().stamp() != inventory.registry_state().stamp()
        {
            return Err(invalid("recipe and inventory capture boundaries differ"));
        }
        let entry = catalogue.entry(recipe)?;
        if !matches!(
            entry.display(),
            RecipeDisplay::Shaped { .. } | RecipeDisplay::Shapeless { .. }
        ) {
            return Err(invalid("recipe has no crafting ingredient material model"));
        }
        let requirements = entry
            .requirements()
            .ok_or_else(|| invalid("recipe crafting requirements have not been received"))?;
        if requirements.is_empty() || requirements.len() > 9 {
            return Err(invalid(
                "recipe ingredient count is outside a native crafting grid",
            ));
        }
        let choices = requirements
            .iter()
            .map(|r| {
                let mut ids = r
                    .items(&catalogue)?
                    .into_iter()
                    .map(RegistryId::value)
                    .collect::<Vec<_>>();
                ids.sort_unstable();
                ids.dedup();
                Ok(ids)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut amounts = BTreeMap::<i32, i32>::new();
        let mut stocks = Vec::new();
        for index in 9..45 {
            let slot = inventory
                .slot(index)?
                .ok_or_else(|| invalid("recipe material inventory slot has not been received"))?;
            match slot.value() {
                SlotKnowledge::Empty => {}
                SlotKnowledge::Item { .. } => {
                    let stock = slot.item().expect("known item").recipe_book_stock()?;
                    if stock.count() < 0 {
                        return Err(invalid(
                            "negative native recipe stock is not available material",
                        ));
                    }
                    let amount = amounts.entry(stock.item().value()).or_default();
                    *amount = amount
                        .checked_add(stock.count())
                        .ok_or_else(|| invalid("native recipe stock accumulator overflow"))?;
                    stocks.push((index, stock));
                }
                _ => return Err(invalid("recipe material inventory item is unavailable")),
            }
        }
        let native_assignment = assign(&choices, &amounts, crafts);
        let registry = crate::client::registry::Registry::for_version(catalogue.session().version);
        let assignment = native_assignment
            .map(|ids| {
                ids.into_iter()
                    .map(|id| registry.item_by_native_id(id).map(|i| i.id))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let maximum = maximum(&choices, &amounts, maximum_bound);
        Ok(Self {
            session: catalogue.session(),
            receive_sequence: catalogue.receive_sequence(),
            recipe: entry.id().clone(),
            unlocked: entry.unlocked(),
            stocks,
            crafts,
            assignment,
            maximum_bound,
            maximum,
        })
    }
}

// Capacity matching: each ingredient consumes `crafts` of one type. Reassign
// earlier ingredients along augmenting paths instead of greedily reserving stock.
fn assign(choices: &[Vec<i32>], amounts: &BTreeMap<i32, i32>, crafts: u32) -> Option<Vec<i32>> {
    if crafts == 0 {
        return None;
    }
    let mut assigned = BTreeMap::<i32, Vec<usize>>::new();
    fn augment(
        ingredient: usize,
        choices: &[Vec<i32>],
        amounts: &BTreeMap<i32, i32>,
        crafts: u32,
        assigned: &mut BTreeMap<i32, Vec<usize>>,
        seen: &mut Vec<i32>,
    ) -> bool {
        for &item in &choices[ingredient] {
            if seen.contains(&item) {
                continue;
            }
            seen.push(item);
            let capacity = amounts.get(&item).copied().unwrap_or(0).max(0) as u32 / crafts;
            if capacity == 0 {
                continue;
            }
            if assigned.get(&item).map_or(0, Vec::len) < capacity as usize {
                assigned.entry(item).or_default().push(ingredient);
                return true;
            }
            // At most nine ingredients, so cloning this bounded adjacency is cheap.
            for (position, other) in assigned
                .get(&item)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .enumerate()
            {
                if augment(other, choices, amounts, crafts, assigned, seen) {
                    assigned.get_mut(&item).expect("assigned item")[position] = ingredient;
                    return true;
                }
            }
        }
        false
    }
    for ingredient in 0..choices.len() {
        if !augment(
            ingredient,
            choices,
            amounts,
            crafts,
            &mut assigned,
            &mut Vec::new(),
        ) {
            return None;
        }
    }
    let mut result = vec![0; choices.len()];
    for (item, ingredients) in assigned {
        for ingredient in ingredients {
            result[ingredient] = item;
        }
    }
    Some(result)
}
fn maximum(choices: &[Vec<i32>], amounts: &BTreeMap<i32, i32>, bound: u32) -> u32 {
    let (mut low, mut high) = (0, bound);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if assign(choices, amounts, middle).is_some() {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

#[cfg(test)]
mod tests;
