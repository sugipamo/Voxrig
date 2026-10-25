//! UseItemOn server admission, independent of the client's pick ray.
//! Official bytecode and exact boundaries: docs/explicit-block-interactions.md.
use crate::client::PlayerObservation;
use crate::{MinecraftVersion, Result};

pub(crate) fn validate_server_range(
    player: &PlayerObservation,
    eye_height: Option<f64>,
    pos: [i32; 3],
) -> Result<()> {
    if !(-33_554_432..=33_554_431).contains(&pos[0])
        || !(-2048..=2047).contains(&pos[1])
        || !(-33_554_432..=33_554_431).contains(&pos[2])
    {
        return Err(crate::error::invalid_input(
            "block position exceeds protocol representation",
        ));
    }
    let Some(feet) = player.position.as_ref().map(|p| p.value) else {
        // Missing observation is not proof of a server refusal. The server still
        // applies the hard limit; never invent standing eyes/attributes here.
        return Ok(());
    };
    let outside = match player.session.version {
        MinecraftVersion::Java1_16_1 => !legacy_range(feet, pos),
        MinecraftVersion::Java1_21_11 => {
            let Some((height, range)) = eye_height.zip(
                player
                    .attributes
                    .get("minecraft:block_interaction_range")
                    .map(|a| a.value.value),
            ) else {
                return Ok(());
            };
            !modern_range([feet[0], feet[1] + height, feet[2]], range, pos)
        }
    };
    if outside {
        return Err(crate::error::invalid_input(
            "block interaction exceeds official server distance limit",
        ));
    }
    Ok(())
}

fn legacy_range(feet: [f64; 3], pos: [i32; 3]) -> bool {
    (0..3)
        .map(|i| (feet[i] - (f64::from(pos[i]) + 0.5)).powi(2))
        .sum::<f64>()
        < 64.
}
fn modern_range(eye: [f64; 3], range: f64, pos: [i32; 3]) -> bool {
    let squared = (0..3)
        .map(|i| {
            let min = f64::from(pos[i]);
            (eye[i] - eye[i].clamp(min, min + 1.)).powi(2)
        })
        .sum::<f64>();
    // PlayerAttribute.value is pre-clamp; official RangedAttribute is 0..64.
    squared < (range.clamp(0., 64.) + 1.).powi(2)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_uses_feet_to_center_and_strict_eight_block_boundary() {
        assert!(legacy_range([0.5, 0.5, 0.5], [7, 0, 0]));
        assert!(!legacy_range([0.5, 0.5, 0.5], [8, 0, 0]));
        // Beyond client pick range, regardless of face/cursor/view/ground.
        assert!(legacy_range([0.5, 0.5, 0.5], [6, 0, 0]));
    }
    #[test]
    fn modern_uses_eye_to_box_attribute_plus_one_and_strict_boundary() {
        assert!(modern_range([0., 0.5, 0.5], 4.5, [5, 0, 0]));
        assert!(!modern_range([0., 0.5, 0.5], 4.5, [6, 0, 0]));
        assert!(!modern_range([-0.5, 0.5, 0.5], 4.5, [5, 0, 0]));
        assert!(modern_range([0., 0.5, 0.5], 10., [10, 0, 0]));
        assert!(!modern_range([0., 0.5, 0.5], 2., [3, 0, 0]));
        assert!(!modern_range([0., 0.5, 0.5], 1000., [65, 0, 0]));
        assert!(!modern_range([0., 0.5, 0.5], -10., [1, 0, 0]));
    }
}
