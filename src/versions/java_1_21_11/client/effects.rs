//! Java 1.21.11 transient packet codecs; commit only complete bounded payloads.
use super::super::{ids::play_clientbound as input, wire::Reader};
use crate::client::effects::{
    MAX_EFFECT_PACKET_BYTES, ParticleData, ParticleItem, ParticleObservation, PlayEffect,
    SoundObservation, SoundOrigin, SoundReference, VibrationDestination,
};
use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Copy)]
enum ParticleLayout {
    Simple,
    Block,
    Color,
    Dust,
    DustTransition,
    Power,
    Spell,
    SculkCharge,
    Item,
    Vibration,
    Trail,
    Shriek,
}
use ParticleLayout as L;

// Original 1.21.11 built-in particle registry, indexed by native ID. Registry and
// option layouts checked against the unchanged official server codecs; see
// data/client_api/effect_packets-1.21.11.json and scripts/ExportEffectPackets.java.
const PARTICLE_LAYOUTS: [L; 115] = [
    L::Simple,         // 0: angry_villager
    L::Block,          // 1: block
    L::Block,          // 2: block_marker
    L::Simple,         // 3: bubble
    L::Simple,         // 4: cloud
    L::Simple,         // 5: copper_fire_flame
    L::Simple,         // 6: crit
    L::Simple,         // 7: damage_indicator
    L::Power,          // 8: dragon_breath
    L::Simple,         // 9: dripping_lava
    L::Simple,         // 10: falling_lava
    L::Simple,         // 11: landing_lava
    L::Simple,         // 12: dripping_water
    L::Simple,         // 13: falling_water
    L::Dust,           // 14: dust
    L::DustTransition, // 15: dust_color_transition
    L::Spell,          // 16: effect
    L::Simple,         // 17: elder_guardian
    L::Simple,         // 18: enchanted_hit
    L::Simple,         // 19: enchant
    L::Simple,         // 20: end_rod
    L::Color,          // 21: entity_effect
    L::Simple,         // 22: explosion_emitter
    L::Simple,         // 23: explosion
    L::Simple,         // 24: gust
    L::Simple,         // 25: small_gust
    L::Simple,         // 26: gust_emitter_large
    L::Simple,         // 27: gust_emitter_small
    L::Simple,         // 28: sonic_boom
    L::Block,          // 29: falling_dust
    L::Simple,         // 30: firework
    L::Simple,         // 31: fishing
    L::Simple,         // 32: flame
    L::Simple,         // 33: infested
    L::Simple,         // 34: cherry_leaves
    L::Simple,         // 35: pale_oak_leaves
    L::Color,          // 36: tinted_leaves
    L::Simple,         // 37: sculk_soul
    L::SculkCharge,    // 38: sculk_charge
    L::Simple,         // 39: sculk_charge_pop
    L::Simple,         // 40: soul_fire_flame
    L::Simple,         // 41: soul
    L::Color,          // 42: flash
    L::Simple,         // 43: happy_villager
    L::Simple,         // 44: composter
    L::Simple,         // 45: heart
    L::Spell,          // 46: instant_effect
    L::Item,           // 47: item
    L::Vibration,      // 48: vibration
    L::Trail,          // 49: trail
    L::Simple,         // 50: item_slime
    L::Simple,         // 51: item_cobweb
    L::Simple,         // 52: item_snowball
    L::Simple,         // 53: large_smoke
    L::Simple,         // 54: lava
    L::Simple,         // 55: mycelium
    L::Simple,         // 56: note
    L::Simple,         // 57: poof
    L::Simple,         // 58: portal
    L::Simple,         // 59: rain
    L::Simple,         // 60: smoke
    L::Simple,         // 61: white_smoke
    L::Simple,         // 62: sneeze
    L::Simple,         // 63: spit
    L::Simple,         // 64: squid_ink
    L::Simple,         // 65: sweep_attack
    L::Simple,         // 66: totem_of_undying
    L::Simple,         // 67: underwater
    L::Simple,         // 68: splash
    L::Simple,         // 69: witch
    L::Simple,         // 70: bubble_pop
    L::Simple,         // 71: current_down
    L::Simple,         // 72: bubble_column_up
    L::Simple,         // 73: nautilus
    L::Simple,         // 74: dolphin
    L::Simple,         // 75: campfire_cosy_smoke
    L::Simple,         // 76: campfire_signal_smoke
    L::Simple,         // 77: dripping_honey
    L::Simple,         // 78: falling_honey
    L::Simple,         // 79: landing_honey
    L::Simple,         // 80: falling_nectar
    L::Simple,         // 81: falling_spore_blossom
    L::Simple,         // 82: ash
    L::Simple,         // 83: crimson_spore
    L::Simple,         // 84: warped_spore
    L::Simple,         // 85: spore_blossom_air
    L::Simple,         // 86: dripping_obsidian_tear
    L::Simple,         // 87: falling_obsidian_tear
    L::Simple,         // 88: landing_obsidian_tear
    L::Simple,         // 89: reverse_portal
    L::Simple,         // 90: white_ash
    L::Simple,         // 91: small_flame
    L::Simple,         // 92: snowflake
    L::Simple,         // 93: dripping_dripstone_lava
    L::Simple,         // 94: falling_dripstone_lava
    L::Simple,         // 95: dripping_dripstone_water
    L::Simple,         // 96: falling_dripstone_water
    L::Simple,         // 97: glow_squid_ink
    L::Simple,         // 98: glow
    L::Simple,         // 99: wax_on
    L::Simple,         // 100: wax_off
    L::Simple,         // 101: electric_spark
    L::Simple,         // 102: scrape
    L::Shriek,         // 103: shriek
    L::Simple,         // 104: egg_crack
    L::Simple,         // 105: dust_plume
    L::Simple,         // 106: trial_spawner_detection
    L::Simple,         // 107: trial_spawner_detection_ominous
    L::Simple,         // 108: vault_connection
    L::Block,          // 109: dust_pillar
    L::Simple,         // 110: ominous_spawning
    L::Simple,         // 111: raid_omen
    L::Simple,         // 112: trial_omen
    L::Block,          // 113: block_crumble
    L::Simple,         // 114: firefly
];

pub(super) fn decode(id: i32, payload: &[u8]) -> Result<Option<PlayEffect>> {
    if !super::event_kinds::EFFECT_KINDS
        .iter()
        .any(|(packet, _)| *packet == id)
    {
        return Ok(None);
    }
    ensure!(
        payload.len() <= MAX_EFFECT_PACKET_BYTES,
        "effect packet exceeds byte limit"
    );
    let mut r = Reader::new(payload);
    let effect = match id {
        input::SOUND_EFFECT | input::ENTITY_SOUND_EFFECT => {
            let holder = r.count(i32::MAX as usize)? as i32;
            let sound = if holder == 0 {
                SoundReference::Inline {
                    name: identifier(&mut r)?,
                    fixed_range: if r.bool()? { Some(r.f32()?) } else { None },
                }
            } else {
                SoundReference::RegistryId(holder - 1)
            };
            let source = source(&mut r)?;
            let origin = if id == input::SOUND_EFFECT {
                SoundOrigin::PositionEighths([r.i32()?, r.i32()?, r.i32()?])
            } else {
                SoundOrigin::Entity(r.varint()?)
            };
            PlayEffect::Sound(SoundObservation {
                sound,
                source,
                origin,
                volume: r.f32()?,
                pitch: r.f32()?,
                seed: r.u64()? as i64,
            })
        }
        input::STOP_SOUND => {
            let flags = r.u8()?;
            ensure!(flags & !3 == 0, "invalid stop-sound flags");
            PlayEffect::SoundStopped {
                source: if flags & 1 != 0 {
                    Some(source(&mut r)?)
                } else {
                    None
                },
                name: if flags & 2 != 0 {
                    Some(identifier(&mut r)?)
                } else {
                    None
                },
            }
        }
        input::WORLD_PARTICLES => {
            let override_limiter = r.bool()?;
            let always_show = r.bool()?;
            let position = [r.f64()?, r.f64()?, r.f64()?];
            let offset = [r.f32()?, r.f32()?, r.f32()?];
            let max_speed = r.f32()?;
            let count = r.i32()?;
            let native_id = r.count(i32::MAX as usize)? as i32;
            let data = particle(&mut r, native_id)?;
            PlayEffect::Particles(ParticleObservation {
                override_limiter,
                always_show,
                position,
                offset,
                max_speed,
                count,
                native_id,
                data,
            })
        }
        input::WORLD_EVENT => PlayEffect::LevelEvent {
            event_id: r.i32()?,
            position: super::super::wire::unpack_position(r.u64()?),
            data: r.i32()?,
            global: r.bool()?,
        },
        input::BLOCK_BREAK_ANIMATION => PlayEffect::BlockDestruction {
            breaker_id: r.varint()?,
            position: super::super::wire::unpack_position(r.u64()?),
            progress: r.u8()?,
        },
        input::COLLECT => PlayEffect::ItemPickup {
            collected_id: r.varint()?,
            collector_id: r.varint()?,
            amount: r.varint()?,
        },
        _ => unreachable!("effect packet table and decoder disagree"),
    };
    r.end()?;
    Ok(Some(effect))
}
fn source(r: &mut Reader<'_>) -> Result<i32> {
    Ok(r.count(9)? as i32)
}
fn identifier(r: &mut Reader<'_>) -> Result<String> {
    let name = r.string()?;
    ensure!(
        name.len() <= 32_767,
        "effect resource identifier exceeds native limit"
    );
    crate::client::identifier::parts(&name)?;
    Ok(name)
}
fn particle(r: &mut Reader<'_>, native_id: i32) -> Result<ParticleData> {
    let original = r.remaining();
    Ok(match PARTICLE_LAYOUTS.get(native_id as usize) {
        Some(L::Simple) => ParticleData::Simple,
        Some(L::Block) => ParticleData::BlockState(r.count(i32::MAX as usize)? as i32),
        Some(L::Color) => ParticleData::Color(r.u32()?),
        Some(L::Dust) => ParticleData::Dust {
            color: r.u32()?,
            scale: r.f32()?,
        },
        Some(L::DustTransition) => ParticleData::DustTransition {
            from_color: r.u32()?,
            to_color: r.u32()?,
            scale: r.f32()?,
        },
        Some(L::Power) => ParticleData::Power(r.f32()?),
        Some(L::Spell) => ParticleData::Spell {
            color: r.u32()?,
            power: r.f32()?,
        },
        Some(L::SculkCharge) => ParticleData::SculkCharge(r.f32()?),
        Some(L::Shriek) => ParticleData::Shriek(r.varint()?),
        Some(L::Trail) => ParticleData::Trail {
            target: [r.f64()?, r.f64()?, r.f64()?],
            color: r.u32()?,
            duration_ticks: r.varint()?,
        },
        Some(L::Vibration) => {
            let destination = match r.varint()? {
                0 => VibrationDestination::Block(super::super::wire::unpack_position(r.u64()?)),
                1 => VibrationDestination::Entity {
                    native_id: r.varint()?,
                    eye_height: r.f32()?,
                },
                id if id >= 0 => {
                    r.take(r.remaining().len())?;
                    return Ok(ParticleData::Unknown {
                        bytes: original.to_vec(),
                    });
                }
                _ => bail!("negative vibration source type"),
            };
            ParticleData::Vibration {
                destination,
                arrival_ticks: r.varint()?,
            }
        }
        Some(L::Item) => {
            let count = r.varint()?;
            ensure!(count > 0, "empty or negative particle item count");
            let native_id = r.count(i32::MAX as usize)? as i32;
            let components = super::super::item_components::read_patch(r)?
                .context("unknown particle item-component value")?;
            ParticleData::Item(ParticleItem {
                count,
                native_id,
                components,
            })
        }
        None => ParticleData::Unknown {
            bytes: r.take(r.remaining().len())?.to_vec(),
        },
    })
}

#[cfg(test)]
mod tests;
