//! Native grid matching using declared ingredient membership, never result displays.
use super::{ReceivedCraftingContext, RecipeDisplay, RecipeId, RecipeSlotDisplay};
use crate::{
    MinecraftVersion, Result,
    client::{SlotKnowledge, inventory::unavailable},
};
use std::collections::BTreeMap;

pub(super) struct Pattern {
    pub(super) shaped: bool,
    pub(super) dimensions: [usize; 2],
    pub(super) cells: Vec<Option<Vec<i32>>>,
    pub(super) ingredients: Vec<Vec<i32>>,
}
pub(super) fn resolve(context: &ReceivedCraftingContext, id: &RecipeId) -> Result<Pattern> {
    let entry = context.recipes().entry(id)?;
    let (dimensions, displays) =
        super::context::display_cells(entry.display(), context.grid().dimensions())?;
    let requirements = entry
        .requirements()
        .ok_or_else(|| unavailable("declared crafting ingredients have not been received"))?;
    let mut ingredients = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        let mut ids = requirement
            .items(context.recipes())?
            .into_iter()
            .map(|v| v.value())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        ingredients.push(ids);
    }
    let mut cells = Vec::with_capacity(displays.len());
    let mut index = 0;
    for display in displays {
        if matches!(display, RecipeSlotDisplay::Empty) {
            cells.push(None);
            continue;
        }
        let mut ids = display
            .item_ids(context.recipes().registry_state())?
            .into_iter()
            .map(|v| v.value())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        if ingredients.get(index) != Some(&ids) {
            return Err(unavailable(
                "recipe display and declared ingredients disagree",
            ));
        }
        cells.push(Some(ids));
        index += 1;
    }
    let shaped = matches!(entry.display(), RecipeDisplay::Shaped { .. });
    if index == 0
        || index != ingredients.len()
        || index > 9
        || (!shaped && cells.iter().any(Option::is_none))
    {
        return Err(unavailable(
            "recipe has no coherent native crafting pattern",
        ));
    }
    Ok(Pattern {
        shaped,
        dimensions,
        cells,
        ingredients,
    })
}
pub(super) fn grid(context: &ReceivedCraftingContext) -> Result<Vec<Option<i32>>> {
    let [w, h] = context.grid().dimensions();
    let mut ids = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let value = context
                .grid()
                .input(x, y)?
                .ok_or_else(|| unavailable("crafting input has not been received"))?;
            ids.push(match value.value() {
                SlotKnowledge::Empty => None,
                SlotKnowledge::Item { item } if item.count > 0 && item.count <= i32::MAX as u32 => {
                    item.properties()?;
                    Some(item.id.value())
                }
                _ => {
                    return Err(unavailable(
                        "crafting input is unavailable or has an invalid native count",
                    ));
                }
            });
        }
    }
    Ok(ids)
}
pub(super) fn matches(
    pattern: &Pattern,
    dimensions: [usize; 2],
    grid: &[Option<i32>],
    version: MinecraftVersion,
) -> bool {
    let [w, h] = dimensions;
    if grid.len() != w * h {
        return false;
    }
    if !pattern.shaped {
        let mut counts = BTreeMap::<i32, i32>::new();
        for id in grid.iter().flatten() {
            *counts.entry(*id).or_default() += 1;
        }
        return grid.iter().flatten().count() == pattern.ingredients.len()
            && super::materials::assign(&pattern.ingredients, &counts, 1).is_some();
    }
    let [rw, rh] = pattern.dimensions;
    if rw > w || rh > h {
        return false;
    }
    let mut offsets = Vec::new();
    if version == MinecraftVersion::Java1_21_11 {
        // Original CraftingInput.of trims the nonempty bounding box before matching.
        let positions = grid
            .iter()
            .enumerate()
            .filter(|(_, id)| id.is_some())
            .map(|(i, _)| [i % w, i / w])
            .collect::<Vec<_>>();
        if positions.is_empty() {
            return false;
        }
        let left = positions.iter().map(|p| p[0]).min().expect("nonempty");
        let right = positions.iter().map(|p| p[0]).max().expect("nonempty");
        let top = positions.iter().map(|p| p[1]).min().expect("nonempty");
        let bottom = positions.iter().map(|p| p[1]).max().expect("nonempty");
        if [right - left + 1, bottom - top + 1] != [rw, rh] {
            return false;
        }
        offsets.push([left, top]);
    } else {
        for x in 0..=w - rw {
            for y in 0..=h - rh {
                offsets.push([x, y]);
            }
        }
    }
    offsets.into_iter().any(|[left, top]| {
        [false, true].into_iter().any(|mirror| {
            (0..h).all(|y| {
                (0..w).all(|x| {
                    let ingredient = if x >= left && x < left + rw && y >= top && y < top + rh {
                        let px = if mirror {
                            rw - 1 - (x - left)
                        } else {
                            x - left
                        };
                        pattern.cells[(y - top) * rw + px].as_ref()
                    } else {
                        None
                    };
                    match (ingredient, grid[y * w + x]) {
                        (None, None) => true,
                        (Some(ids), Some(id)) => ids.binary_search(&id).is_ok(),
                        _ => false,
                    }
                })
            })
        })
    })
}

#[cfg(test)]
mod tests;
