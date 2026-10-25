//! Body admission and evidence, independent of the strict dig estimator.
use super::{
    BodySlot, BodySnapshot,
    evidence::{EvidenceBatch, Update},
};
use crate::{
    MinecraftVersion, Result,
    client::{GameMode, PlayerObservation, SlotKnowledge},
};

pub(crate) fn hardness(version: MinecraftVersion, state: Option<i32>) -> Result<()> {
    if state
        .and_then(|id| crate::client::dig::known_hardness(version, id))
        .is_some_and(|h| h < 0.)
    {
        return Err(crate::error::invalid_input(
            "the block cannot be broken in survival",
        ));
    }
    Ok(())
}

/// Even with unknown eye height, horizontal distance is a lower bound on
/// the official eye-to-block-box distance. Use only an observed range attribute.
pub(crate) fn horizontal_range(player: &PlayerObservation, target: [i32; 3]) -> Result<()> {
    if let Some((feet, range)) = player
        .position
        .as_ref()
        .zip(player.attributes.get("minecraft:block_interaction_range"))
    {
        if outside_horizontal_range(feet.value, range.value.value, target) {
            return Err(crate::error::invalid_input(
                "body dig exceeds official server range",
            ));
        }
    }
    Ok(())
}
fn outside_horizontal_range(feet: [f64; 3], range: f64, target: [i32; 3]) -> bool {
    [0, 2]
        .into_iter()
        .map(|i| {
            let min = f64::from(target[i]);
            (feet[i] - feet[i].clamp(min, min + 1.)).powi(2)
        })
        .sum::<f64>()
        >= (range.clamp(0., 64.) + 1.).powi(2)
}

/// Retain known identity; lack of initial knowledge is not a timing veto.
/// Enchantments/damage/count/effects are timing inputs, not a different item kind.
pub(crate) fn hand(initial: &BodySnapshot, now: &PlayerObservation) -> Result<()> {
    if now.game_mode != initial.player.game_mode {
        return Err(crate::error::state_error("body dig mode changed"));
    }
    if now.game_mode != Some(GameMode::Survival) {
        return Ok(());
    }
    let selected = initial.player.selected_hotbar.as_ref().map(|s| s.value);
    let current = now.selected_hotbar.as_ref().map(|s| s.value);
    if selected.is_some_and(|s| Some(s) != current) {
        return Err(crate::error::state_error(
            "body dig held item identity changed (selected slot)",
        ));
    }
    if let Some(slot) = selected.map(|s| 36 + usize::from(s)) {
        let name = initial
            .player
            .inventory
            .get(slot)
            .and_then(Option::as_ref)
            .and_then(|s| match &s.value {
                BodySlot::Empty => Some(""),
                BodySlot::Item(item) => Some(item.name.as_str()),
                _ => None,
            });
        let now = now
            .inventory
            .slots
            .get(slot)
            .and_then(Option::as_ref)
            .and_then(|s| match &s.value {
                SlotKnowledge::Empty => Some(""),
                SlotKnowledge::Item { item } => Some(item.name.as_str()),
                _ => None,
            });
        if name.is_some() && name != now {
            return Err(crate::error::state_error(
                "body dig held item identity changed",
            ));
        }
    }
    Ok(())
}

pub(crate) fn started(batch: &EvidenceBatch, target: [i32; 3], sequence: Option<i32>) -> bool {
    sequence.is_some_and(|s| batch.ack.is_some_and(|ack| ack >= s))
        || batch.events.iter().any(|e| {
            matches!(e.update,
        Update::DigAck { pos, status: 0, successful: true, .. } if pos == target)
        })
}
pub(crate) fn removed(batch: &EvidenceBatch, target: [i32; 3]) -> bool {
    batch.events.iter().any(|e| {
        let state = match e.update {
            Update::Block(pos, state) if pos == target => state,
            Update::DigAck {
                pos,
                state,
                status: 0 | 2,
                successful: true,
            } if pos == target => state,
            _ => return false,
        };
        (batch.snapshot.version.table().registry.native_state)(state).is_ok_and(|s| {
            matches!(
                s.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_horizontal_range_cannot_be_bypassed_by_unknown_eye_height() {
        assert!(!outside_horizontal_range([0., 65., 0.], 4.5, [5, 65, 0]));
        assert!(outside_horizontal_range([0., 65., 0.], 4.5, [6, 65, 0]));
        assert!(outside_horizontal_range([0., 65., 0.], 4.5, [30, 65, 0]));
        assert!(!outside_horizontal_range([0., 65., 0.], 32., [30, 65, 0]));
        assert!(outside_horizontal_range([0., 65., 0.], 1000., [65, 65, 0]));
    }
    #[test]
    fn hardness_guard_distinguishes_unknown_from_known_unbreakable_in_both_versions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for state in [None, Some(-1), Some(i32::MAX)] {
                hardness(version, state).unwrap();
            }
            let state = |name: &str| {
                (version.table().registry.state_id)(&crate::NativeBlockState {
                    name: format!("minecraft:{name}"),
                    properties: Default::default(),
                })
                .unwrap()
            };
            assert!(hardness(version, Some(state("bedrock"))).is_err());
            assert!(hardness(version, Some(state("dirt"))).is_ok());
        }
    }
}
