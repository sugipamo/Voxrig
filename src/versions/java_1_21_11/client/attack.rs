//! Local whole-tick attack charge; independent of packet and movement tick rates.
use super::*;

const ATTACK_SPEED_ATTRIBUTE: i32 = 4;
const DEFAULT_ATTACK_SPEED: f64 = 4.0;
const MAX_ATTACK_SPEED: f64 = 1024.0;
const CLIENT_TICK_MILLIS: u128 = 50;

pub(super) struct AttackStrength {
    epoch: tokio::time::Instant,
    reset_tick: u128,
}
impl Default for AttackStrength {
    fn default() -> Self {
        Self {
            epoch: tokio::time::Instant::now(),
            reset_tick: 0,
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
        // Player.getCurrentItemAttackStrengthDelay casts the double division to
        // float before getAttackStrengthScale divides the whole-tick counter.
        let delay = (20.0 / speed) as f32;
        (self.tick().saturating_sub(self.reset_tick) as f32 / delay).clamp(0.0, 1.0)
    }
}

pub(super) fn held_slot(operations: &operations::OperationState) -> u8 {
    operations.selected_hotbar.as_ref().map_or(0, |s| s.slot)
}

// Vanilla's ticker compares item identity, not stack count or component data.
// Unknown inventory knowledge remains distinct from a received empty stack.
pub(super) fn held_item(operations: &operations::OperationState) -> Option<i32> {
    use operations::InventorySlot;
    match operations
        .inventory
        .slots
        .get(36 + usize::from(held_slot(operations)))?
    {
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
