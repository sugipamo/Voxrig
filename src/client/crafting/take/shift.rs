//! One native result QUICK_MOVE. Its repeated crafts are received, not predicted.
use super::*;
use crate::client::{ItemStack, ReceivedInventory, registry::Registry};

fn output(record: &CraftingTakeRecord) -> &ItemStack {
    match record.before.result.as_ref().expect("complete").value() {
        SlotKnowledge::Item { item } => item,
        _ => unreachable!("validated result"),
    }
}

fn count(
    player: &PlayerObservation,
    result: &ItemStack,
    context: &ItemContext,
    registries: &ServerRegistryObservation,
) -> Result<u64> {
    let mut total = 0u64;
    for index in 9..=45 {
        let value = player.inventory.slots[index].as_ref().ok_or_else(|| {
            unavailable("complete actual crafting destination inventory required")
        })?;
        if !matches!(value.source, ValueSource::Received {sequence}
            if sequence <= player.receive_sequence && sequence >= registries.stamp().configuration_generation)
        {
            return Err(unavailable(
                "actual crafting inventory receipt outside registry boundary",
            ));
        }
        // Validate actual constructors and named-holder lifetimes, including the
        // offhand that can receive native crafting remainders. Output counting
        // itself covers only the original main/hotbar destination range.
        context.classify(value, value, &value.value, registries)?;
        if index < 45 {
            if let SlotKnowledge::Item { item } = &value.value {
                if context.same_data(result, item)? {
                    total = total
                        .checked_add(u64::from(item.count))
                        .ok_or_else(|| unavailable("crafting output stock count overflow"))?;
                }
            }
        }
    }
    Ok(total)
}

pub(super) fn prepare_inventory(
    player: &PlayerObservation,
    grid: &ReceivedCrafting,
    context: &ItemContext,
    result: &ItemStack,
) -> Result<u64> {
    for index in 9..=45 {
        let before = player.inventory.slots[index]
            .as_ref()
            .ok_or_else(|| unavailable("known crafting destination inventory required"))?;
        context.predecessor(before)?;
    }
    let name = match grid.source() {
        CraftingSource::Player { .. } => "minecraft:player",
        CraftingSource::Table { .. } => "minecraft:crafting",
    };
    let menu = native_menu(player.session.version, name)
        .ok_or_else(|| unavailable("native crafting destination layout absent"))?;
    let mut destinations: Vec<_> = menu
        .player_slots
        .iter()
        .filter(|slot| slot.raw_player_slot < 36)
        .collect();
    destinations.sort_by_key(|slot| std::cmp::Reverse(slot.screen_slot));
    if destinations.len() != 36 {
        return Err(unavailable(
            "native crafting main/hotbar destination range absent",
        ));
    }
    let maximum = u32::try_from(result.properties()?.max_stack_size)
        .map_err(|_| unavailable("crafting result capacity invalid"))?;
    let mut remaining = result.count;
    // Original moveItemStackTo merges compatible stacks, then stops at its first
    // eligible empty slot. Do not sum all empty slots for an oversized result.
    if result.properties()?.stackable {
        for slot in &destinations {
            let index = if slot.raw_player_slot < 9 {
                36 + slot.raw_player_slot
            } else {
                slot.raw_player_slot
            };
            if let SlotKnowledge::Item { item } =
                &player.inventory.slots[index].as_ref().expect("known").value
            {
                if context.same_data(result, item)? {
                    let capacity = crate::client::inventory::slot_policy::regular_slot(
                        player.session.version,
                        name,
                        slot.screen_slot,
                    )?
                    .base_capacity
                    .min(maximum);
                    remaining = remaining.saturating_sub(capacity.saturating_sub(item.count));
                }
            }
        }
    }
    if remaining > 0 {
        for slot in &destinations {
            let index = if slot.raw_player_slot < 9 {
                36 + slot.raw_player_slot
            } else {
                slot.raw_player_slot
            };
            if player.inventory.slots[index].as_ref().expect("known").value == SlotKnowledge::Empty
            {
                let policy = crate::client::inventory::slot_policy::regular_slot(
                    player.session.version,
                    name,
                    slot.screen_slot,
                )?;
                if !policy.rejected_default_items.contains(&result.name) {
                    remaining = remaining.saturating_sub(policy.base_capacity.min(maximum));
                    break;
                }
            }
        }
    }
    if remaining != 0 {
        return Err(unavailable(
            "entire first crafting result does not fit native inventory move",
        ));
    }
    count(player, result, context, grid.registry_state())
}

pub(super) fn validate_before(
    record: &CraftingTakeRecord,
    player: &PlayerObservation,
) -> Result<()> {
    for index in 9..=45 {
        if player.inventory.slots[index] != record.initial.inventory.slots[index] {
            return Err(unavailable(
                "crafting destination predecessor changed before I/O",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_retained(
    record: &CraftingTakeRecord,
    player: &PlayerObservation,
    registries: &ServerRegistryObservation,
) -> Result<()> {
    if let Some(after) = &record.inventory_after {
        for index in 9..=45 {
            let retained = after
                .slot(index)?
                .ok_or_else(|| unavailable("retained crafting inventory absent"))?;
            let actual = player.inventory.slots[index]
                .as_ref()
                .ok_or_else(|| unavailable("crafting inventory receipt disappeared"))?;
            let before =
                crate::client::received(retained.value().clone(), retained.receive_sequence());
            if !record
                .context
                .classify(actual, &before, retained.value(), registries)?
                .0
            {
                return Err(unavailable(
                    "crafting inventory changed before transfer completion",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn capture_after(
    record: &CraftingTakeRecord,
    player: &PlayerObservation,
    registries: &ServerRegistryObservation,
    full: u64,
) -> Result<Option<(ReceivedInventory, u64)>> {
    if !(9..45).all(|index| {
        player.inventory.slots[index]
            .as_ref()
            .is_some_and(|slot| slot.source == ValueSource::Received { sequence: full })
    }) {
        return Ok(None);
    }
    let total = count(player, output(record), &record.context, registries)?;
    let Some(increase) =
        total.checked_sub(record.inventory_output_before.expect("inventory baseline"))
    else {
        return Err(unavailable("crafting output inventory stock decreased"));
    };
    if increase < u64::from(output(record).count) {
        return Ok(None);
    }
    Ok(Some((
        ReceivedInventory::capture(
            player.session,
            player.receive_sequence,
            &player.inventory,
            registries.clone(),
        )?,
        increase,
    )))
}

pub(super) fn legacy_comparison(record: &CraftingTakeRecord) -> Result<SlotKnowledge> {
    // A one-batch QUICK_MOVE returns Empty; repeated native rounds can return
    // the original result item. A distinct item mismatches both possibilities.
    let name = if output(record).name == "minecraft:dirt" {
        "minecraft:stone"
    } else {
        "minecraft:dirt"
    };
    let item = Registry::for_version(record.id.session().version).item(name)?;
    Ok(SlotKnowledge::Item {
        item: ItemStack {
            id: item.id,
            name: item.name,
            count: 1,
            data: crate::client::ItemData::Default,
        },
    })
}
