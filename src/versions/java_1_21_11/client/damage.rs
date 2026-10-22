//! Complete damage/hurt/combat payloads, retained with the transient effect history.
use super::{super::ids::play_clientbound as input, super::wire::Reader, State};
use crate::client::effects::{DamageObservation, MAX_EFFECT_PACKET_BYTES, PlayEffect};
use anyhow::{Result, ensure};

#[derive(Clone, Copy)]
enum Layout {
    Damage,
    Hurt,
    Enter,
    End,
    Kill,
}
// Java 1.21.11 packet layouts; the protocol's split combat packets carry no
// attacker field in END and no fields at all in ENTER.
const LAYOUTS: &[(i32, Layout)] = &[
    (input::DAMAGE_EVENT, Layout::Damage),
    (input::HURT_ANIMATION, Layout::Hurt),
    (input::ENTER_COMBAT_EVENT, Layout::Enter),
    (input::END_COMBAT_EVENT, Layout::End),
    (input::DEATH_COMBAT_EVENT, Layout::Kill),
];

pub(super) fn decode(state: &State, id: i32, payload: &[u8]) -> Result<Option<PlayEffect>> {
    let Some((_, layout)) = LAYOUTS.iter().find(|(packet, _)| *packet == id) else {
        return Ok(None);
    };
    ensure!(
        payload.len() <= MAX_EFFECT_PACKET_BYTES,
        "effect packet exceeds byte limit"
    );
    let mut r = Reader::new(payload);
    let effect = match layout {
        Layout::Damage => {
            let entity_id = r.varint()?;
            let native_type = r.varint()?;
            let cause_entity_id = optional_entity(&mut r)?;
            let direct_entity_id = optional_entity(&mut r)?;
            let source_position = if r.bool()? {
                Some([r.f64()?, r.f64()?, r.f64()?])
            } else {
                None
            };
            let registries = state
                .registries
                .capture(state.snapshot_session(), state.sequence);
            let source_type = registries.bind("minecraft:damage_type", native_type)?;
            let source_type_name = registries.resolve(&source_type)?.name.clone();
            PlayEffect::Damage(DamageObservation {
                entity_id,
                source_type,
                source_type_name,
                cause_entity_id,
                direct_entity_id,
                source_position,
            })
        }
        Layout::Hurt => PlayEffect::HurtAnimation {
            entity_id: r.varint()?,
            yaw: r.f32()?,
        },
        Layout::Enter => PlayEffect::CombatEntered,
        Layout::End => PlayEffect::CombatEnded {
            duration_ticks: r.varint()?,
        },
        Layout::Kill => PlayEffect::CombatKill {
            player_id: r.varint()?,
            message: crate::client::ui::UiText::NativeNbt {
                bytes: r.encoded_nbt()?,
            },
        },
    };
    r.end()?;
    Ok(Some(effect))
}
fn optional_entity(r: &mut Reader<'_>) -> Result<Option<i32>> {
    // readOptionalEntityId subtracts one with Java's wrapping int arithmetic.
    // Retain all signed entity IDs, including the i32::MAX wraparound encoding.
    let value = r.varint()?.wrapping_sub(1);
    Ok((value != -1).then_some(value))
}

#[cfg(test)]
mod tests;
