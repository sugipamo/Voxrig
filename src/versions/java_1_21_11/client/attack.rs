//! Local whole-tick attack charge; independent of packet and movement tick rates.
use super::*;

const ATTACK_SPEED_ATTRIBUTE: i32 = 4;
const DEFAULT_ATTACK_SPEED: f64 = 4.0;
const MAX_ATTACK_SPEED: f64 = 1024.0;
const CLIENT_TICK_MILLIS: u128 = 50;

pub(super) struct AttackStrength {
    epoch: tokio::time::Instant,
    reset_tick: u128,
    held_slot: u8,
    held_item: Option<i32>,
}
impl Default for AttackStrength {
    fn default() -> Self {
        Self {
            epoch: tokio::time::Instant::now(),
            reset_tick: 0,
            held_slot: 0,
            held_item: None,
        }
    }
}
impl AttackStrength {
    fn tick(&self) -> u128 {
        self.epoch.elapsed().as_millis() / CLIENT_TICK_MILLIS
    }
    pub(super) fn reset(&mut self) {
        self.reset_tick = self.tick();
    }
    pub(super) fn reset_world(&mut self) {
        self.reset();
        self.held_slot = 0;
        self.held_item = None;
    }
    fn value(&self, player: &operations::LocalPlayerState) -> f32 {
        let speed = player
            .received_attributes
            .get(&ATTACK_SPEED_ATTRIBUTE)
            .map(|a| {
                let modifiers = a.modifiers.iter().collect::<Vec<_>>();
                crate::client::physics::attribute_value(
                    MinecraftVersion::Java1_21_11,
                    a.base,
                    &modifiers,
                )
            })
            .unwrap_or(DEFAULT_ATTACK_SPEED);
        // RangedAttribute sanitizes NaN to its minimum as well as clamping.
        let speed = if speed.is_nan() {
            0.0
        } else {
            speed.clamp(0.0, MAX_ATTACK_SPEED)
        };
        // Preserve Player.getCurrentItemAttackStrengthDelay's double reciprocal
        // and multiply before the float cast; 20.0 / speed can round differently
        // at a full-charge boundary.
        let delay = ((1.0 / speed) * 20.0) as f32;
        (self.tick().saturating_sub(self.reset_tick) as f32 / delay).clamp(0.0, 1.0)
    }
}

// Selection attempts are recorded before sending. Only a complete send or an
// actual selection receipt changes the hand used by this ticker.
pub(super) fn observe(state: &mut State, selected_slot: Option<u8>) {
    let charge = &mut state.attack_strength;
    if let Some(slot) = selected_slot {
        if charge.held_slot != slot {
            charge.reset();
            charge.held_slot = slot;
            charge.held_item = None;
        }
    }
    if let Some(item) = held_item(&state.operations, charge.held_slot) {
        if charge.held_item != Some(item) {
            charge.reset();
            charge.held_item = Some(item);
        }
    }
}

// Vanilla's ticker compares item identity, not stack count or component data.
// Missing inventory knowledge (e.g. opening a screen) is not an item change.
// A received empty stack is a known identity and must still reset the ticker.
fn held_item(operations: &operations::OperationState, slot: u8) -> Option<i32> {
    use operations::InventorySlot;
    match operations.inventory.slots.get(36 + usize::from(slot))? {
        InventorySlot::Unavailable => None,
        InventorySlot::Empty => Some(0),
        InventorySlot::Item { item } | InventorySlot::ItemWithComponents { item, .. } => {
            Some(item.item_id)
        }
    }
}

impl operations::Operations {
    /// Local whole-tick attack charge; see [`crate::Client::attack_strength`].
    pub async fn attack_strength(&self) -> Result<f32> {
        let state = self.bot.session.state.lock().await;
        self.bot.session.check(&state)?;
        if !state.ready {
            return Err(crate::error::invalid_input("play state is not ready"));
        }
        Ok(state.attack_strength.value(&state.operations.local_player))
    }
}

#[cfg(test)]
mod tests;
