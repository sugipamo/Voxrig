//! Bounded native recipe/display decoding. Commit only complete book packets.
use super::{State, ids, operations};
use crate::MinecraftVersion;
use crate::client::{
    SlotKnowledge,
    crafting::recipes::{
        RecipeDisplay, RecipeEntry, RecipeIngredient, RecipeSlotDisplay, RecipeTrimDefinition,
        RecipeTrimPattern,
    },
    registry::Registry,
};
use crate::versions::java_1_21_11::wire::Reader;
use anyhow::{Context, bail};
use std::collections::BTreeSet;

const MAX_ENTRIES: usize = 65_536;
const MAX_DISPLAY_NODES: usize = 262_144;
const MAX_DEPTH: usize = 64;
fn registry() -> Registry {
    Registry::for_version(MinecraftVersion::Java1_21_11)
}
fn identifier(r: &mut Reader<'_>) -> anyhow::Result<String> {
    let name = r.string()?;
    // Native Identifier rejects invalid registry/tag/asset names.
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", &name));
    if namespace.is_empty()
        || path.is_empty()
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
        || !path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/_.-".contains(&c))
    {
        bail!("invalid recipe identifier");
    }
    Ok(format!("{namespace}:{path}"))
}
fn slot(r: &mut Reader<'_>, depth: usize, budget: &mut usize) -> anyhow::Result<RecipeSlotDisplay> {
    if depth >= MAX_DEPTH || *budget == 0 {
        bail!("recipe display nesting/work limit");
    }
    *budget -= 1;
    Ok(match r.varint()? {
        0 => RecipeSlotDisplay::Empty,
        1 => RecipeSlotDisplay::AnyFuel,
        2 => RecipeSlotDisplay::Item {
            item: registry().item_by_native_id(r.varint()?)?,
        },
        3 => {
            let native = operations::slot(r)?
                .context("unknown display stack component codec; update Voxrig")?;
            let SlotKnowledge::Item { item } = operations::common_slot(&native)? else {
                bail!("empty native display stack");
            };
            RecipeSlotDisplay::Stack { item }
        }
        4 => RecipeSlotDisplay::Tag {
            name: identifier(r)?,
        },
        5 => {
            let base = Box::new(slot(r, depth + 1, budget)?);
            let material = Box::new(slot(r, depth + 1, budget)?);
            let holder = r.varint()?;
            if holder < 0 {
                bail!("negative trim pattern holder");
            }
            let pattern = if holder == 0 {
                let asset = identifier(r)?;
                let encoded = r.encoded_nbt()?;
                let description = crate::client::nbt::decode_unnamed_tag(&encoded)?
                    .context("empty trim description component")?;
                let decal = r.bool()?;
                RecipeTrimPattern::inline(RecipeTrimDefinition {
                    asset,
                    description,
                    decal,
                })
            } else {
                RecipeTrimPattern::raw_reference(holder - 1)
            };
            RecipeSlotDisplay::TrimPreview {
                base,
                material,
                pattern,
            }
        }
        6 => RecipeSlotDisplay::WithRemainder {
            input: Box::new(slot(r, depth + 1, budget)?),
            remainder: Box::new(slot(r, depth + 1, budget)?),
        },
        7 => {
            let count = r.count((*budget).min(MAX_ENTRIES))?;
            let contents = (0..count)
                .map(|_| slot(r, depth + 1, budget))
                .collect::<anyhow::Result<_>>()?;
            RecipeSlotDisplay::Composite { contents }
        }
        _ => bail!("unknown native slot display type; update Voxrig"),
    })
}
fn display(r: &mut Reader<'_>, budget: &mut usize) -> anyhow::Result<RecipeDisplay> {
    Ok(match r.varint()? {
        0 => {
            let count = r.count(MAX_ENTRIES)?;
            let ingredients = (0..count)
                .map(|_| slot(r, 0, budget))
                .collect::<anyhow::Result<_>>()?;
            let result = slot(r, 0, budget)?;
            let crafting_station = Some(slot(r, 0, budget)?);
            RecipeDisplay::Shapeless {
                ingredients,
                result,
                crafting_station,
            }
        }
        1 => {
            let width = r.varint()?;
            let height = r.varint()?;
            let count = r.count(MAX_ENTRIES)?;
            if width.wrapping_mul(height) != count as i32 {
                bail!("shaped native display size mismatch");
            }
            let ingredients = (0..count)
                .map(|_| slot(r, 0, budget))
                .collect::<anyhow::Result<_>>()?;
            let result = slot(r, 0, budget)?;
            let crafting_station = Some(slot(r, 0, budget)?);
            RecipeDisplay::Shaped {
                width,
                height,
                ingredients,
                result,
                crafting_station,
            }
        }
        2 => {
            let ingredient = slot(r, 0, budget)?;
            let fuel = Some(slot(r, 0, budget)?);
            let result = slot(r, 0, budget)?;
            let crafting_station = Some(slot(r, 0, budget)?);
            let duration = r.varint()?;
            let experience_bits = r.u32()?;
            RecipeDisplay::Furnace {
                recipe_kind: "minecraft:furnace".into(),
                ingredient,
                fuel,
                result,
                crafting_station,
                duration,
                experience_bits,
            }
        }
        3 => {
            let input = slot(r, 0, budget)?;
            let result = slot(r, 0, budget)?;
            let crafting_station = Some(slot(r, 0, budget)?);
            RecipeDisplay::Stonecutter {
                input,
                result,
                crafting_station,
            }
        }
        4 => {
            let template = Some(slot(r, 0, budget)?);
            let base = slot(r, 0, budget)?;
            let addition = slot(r, 0, budget)?;
            let result = slot(r, 0, budget)?;
            let crafting_station = Some(slot(r, 0, budget)?);
            RecipeDisplay::Smithing {
                template,
                base,
                addition,
                result,
                crafting_station,
            }
        }
        _ => bail!("unknown native recipe display type; update Voxrig"),
    })
}
fn ingredient(r: &mut Reader<'_>) -> anyhow::Result<RecipeIngredient> {
    let count = r.count(MAX_ENTRIES + 1)?;
    Ok(if count == 0 {
        RecipeIngredient::Tag {
            name: identifier(r)?,
        }
    } else {
        RecipeIngredient::Items {
            items: (0..count - 1)
                .map(|_| {
                    registry()
                        .item_by_native_id(r.varint()?)
                        .map(|v| v.id)
                        .map_err(Into::into)
                })
                .collect::<anyhow::Result<_>>()?,
        }
    })
}
fn entry(r: &mut Reader<'_>, budget: &mut usize) -> anyhow::Result<(i32, RecipeEntry)> {
    let id = r.varint()?;
    if id < 0 {
        bail!("negative recipe display ID");
    }
    let display = display(r, budget)?;
    let group = r.varint()?;
    if group < 0 {
        bail!("invalid optional recipe group");
    }
    let category =
        registry().builtin_id_by_native_id("minecraft:recipe_book_category", r.varint()?)?;
    let category = Some(registry().builtin_name(&category)?.to_owned());
    let requirements = if r.bool()? {
        let count = r.count(MAX_ENTRIES)?;
        Some(
            (0..count)
                .map(|_| ingredient(r))
                .collect::<anyhow::Result<_>>()?,
        )
    } else {
        None
    };
    Ok((
        id,
        RecipeEntry {
            display,
            requirements,
            group: if group == 0 {
                None
            } else {
                Some((group - 1).to_string())
            },
            category,
            highlighted: None,
            notification: None,
        },
    ))
}
pub(super) fn receive(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<bool> {
    use ids::play_clientbound as input;
    if !matches!(
        id,
        input::RECIPE_BOOK_ADD | input::RECIPE_BOOK_REMOVE | input::RECIPE_BOOK_SETTINGS
    ) {
        return Ok(false);
    }
    let mut r = Reader::new(payload);
    match id {
        input::RECIPE_BOOK_ADD => {
            let count = r.count(MAX_ENTRIES)?;
            let mut budget = MAX_DISPLAY_NODES;
            let mut seen = BTreeSet::new();
            let mut entries = Vec::new();
            for _ in 0..count {
                let (id, mut entry) = entry(&mut r, &mut budget)?;
                if !seen.insert(id) {
                    bail!("duplicate recipe display ID in book packet");
                }
                let flags = r.u8()?;
                if flags & !3 != 0 {
                    bail!("unknown recipe book flags; update Voxrig");
                }
                entry.notification = Some(flags & 1 != 0);
                entry.highlighted = Some(flags & 2 != 0);
                entries.push((id, entry));
            }
            let replace = r.bool()?;
            r.end()?;
            state.recipes.add_modern(entries, replace, state.sequence);
        }
        input::RECIPE_BOOK_REMOVE => {
            let count = r.count(MAX_ENTRIES)?;
            let ids = (0..count)
                .map(|_| {
                    let id = r.varint()?;
                    if id < 0 {
                        bail!("negative removed recipe ID");
                    }
                    Ok(id)
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            r.end()?;
            state.recipes.remove_modern(&ids, state.sequence);
        }
        input::RECIPE_BOOK_SETTINGS => {
            for _ in 0..8 {
                r.bool()?;
            }
            r.end()?;
        }
        _ => unreachable!(),
    }
    Ok(true)
}
#[cfg(test)]
mod tests;
