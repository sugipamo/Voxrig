//! Original ClientboundExplodePacket codec (official Java 1.21.11).
use super::{super::wire::Reader, effects};
use crate::client::explosions::*;
use anyhow::{Result, ensure};

pub(super) fn decode(payload: &[u8]) -> Result<ExplosionObservation> {
    ensure!(
        payload.len() <= MAX_EXPLOSION_PACKET_BYTES,
        "explosion packet exceeds byte limit"
    );
    let mut r = Reader::new(payload);
    let center = vector(&mut r)?;
    let radius = r.f32()?;
    // INT, not VAR_INT, in the original 1.21.11 codec.
    let block_count = r.i32()?;
    let player_knockback = if r.bool()? {
        Some(vector(&mut r)?)
    } else {
        None
    };
    let original = r.remaining();
    let presentation = match presentation(&mut r)? {
        Some(presentation) => {
            r.end()?;
            presentation
        }
        None => ExplosionPresentation::Undecoded {
            bytes: original.to_vec(),
        },
    };
    Ok(ExplosionObservation {
        center,
        radius,
        block_count,
        player_knockback,
        presentation,
    })
}
fn vector(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    Ok([r.f64()?, r.f64()?, r.f64()?])
}
fn particle(r: &mut Reader<'_>) -> Result<Option<ExplosionParticle>> {
    Ok(effects::embedded_particle(r)?
        .map(|(native_id, data)| ExplosionParticle { native_id, data }))
}
fn presentation(r: &mut Reader<'_>) -> Result<Option<ExplosionPresentation>> {
    let Some(particle) = particle(r)? else {
        return Ok(None);
    };
    let sound = effects::sound_reference(r)?;
    // Each entry needs at least one particle ID, two floats and one weight.
    let count = r.count(MAX_EXPLOSION_PACKET_BYTES / 10)?;
    let mut block_particles = Vec::new();
    let mut total_weight = 0i32;
    for _ in 0..count {
        let Some(particle) = self::particle(r)? else {
            return Ok(None);
        };
        let scaling = r.f32()?;
        let speed = r.f32()?;
        let weight = r.count(i32::MAX as usize)? as i32;
        total_weight = total_weight
            .checked_add(weight)
            .ok_or_else(|| anyhow::anyhow!("explosion particle total weight overflow"))?;
        block_particles.push(ExplosionBlockParticle {
            particle,
            scaling,
            speed,
            weight,
        });
    }
    Ok(Some(ExplosionPresentation::Decoded {
        particle,
        sound,
        block_particles,
    }))
}

#[cfg(test)]
mod tests;
