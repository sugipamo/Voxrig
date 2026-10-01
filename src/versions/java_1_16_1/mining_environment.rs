//! Protocol-736 local pose and eye-water observation. Unknown facts stay unknown.

/// Facts captured in the same actor turn as position and world geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiningEnvironment {
    /// Entity pose metadata (protocol default is standing at login).
    pub pose: Option<i32>,
    /// Eye-level water intersection, unavailable for unknown pose/cells.
    pub eyes_in_water: Option<bool>,
}

fn eye_height(pose: i32) -> Option<f64> {
    match pose {
        0 => Some(f64::from(1.62_f32)),
        1 | 3 | 4 => Some(f64::from(0.4_f32)),
        5 => Some(f64::from(1.27_f32)),
        _ => None,
    }
}

fn water_amount(state: i32) -> Option<u8> {
    let name = crate::block_name_from_state(state)?;
    let properties = crate::block_state_properties(state)?;
    if name == "water" {
        let level = properties
            .iter()
            .find(|(k, _)| k == "level")?
            .1
            .parse::<u8>()
            .ok()?;
        return (level <= 15).then_some(if level >= 8 { 8 } else { 8 - level });
    }
    if properties
        .iter()
        .any(|(k, v)| k == "waterlogged" && v == "true")
        || matches!(
            name,
            "bubble_column" | "kelp" | "kelp_plant" | "seagrass" | "tall_seagrass"
        )
    {
        return Some(8);
    }
    Some(0)
}

pub(crate) fn observe(
    pose: Option<i32>,
    player: &crate::Player,
    mut block: impl FnMut(i32, i32, i32) -> Option<i32>,
) -> MiningEnvironment {
    let eyes_in_water = (|| {
        let eye = player.y + eye_height(pose?)? - 0.111_111_111_938_953_4;
        let (x, y, z) = (
            player.x.floor() as i32,
            eye.floor() as i32,
            player.z.floor() as i32,
        );
        let amount = water_amount(block(x, y, z)?)?;
        if amount == 0 {
            return Some(false);
        }
        let above = water_amount(block(x, y.checked_add(1)?, z)?)?;
        let height = if above > 0 {
            1.0
        } else {
            f64::from(f32::from(amount) / 9.0)
        };
        Some(f64::from(y) + height > eye)
    })();
    MiningEnvironment {
        pose,
        eyes_in_water,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pose_heights_are_not_a_fixed_standing_default() {
        assert!(eye_height(0).unwrap() > eye_height(5).unwrap());
        assert!(eye_height(5).unwrap() > eye_height(3).unwrap());
        assert_eq!(eye_height(99), None);
    }
    #[test]
    fn eye_water_uses_pose_fluid_surface_and_loaded_evidence() {
        let player = crate::Player {
            username: "test".into(),
            entity_id: Some(1),
            x: 0.5,
            y: 0.0,
            z: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            spawned: true,
        };
        let water = (0..20000)
            .find(|s| crate::block_name_from_state(*s) == Some("water"))
            .unwrap();
        let blocks = |_, y, _| Some(if y == 0 { water } else { 0 });
        assert_eq!(observe(Some(0), &player, blocks).eyes_in_water, Some(false));
        assert_eq!(observe(Some(3), &player, blocks).eyes_in_water, Some(true));
        assert_eq!(observe(None, &player, blocks).eyes_in_water, None);
        assert_eq!(
            observe(Some(0), &player, |_, _, _| None).eyes_in_water,
            None
        );
        let waterlogged = (0..20000)
            .find(|s| {
                crate::block_state_properties(*s)
                    .is_some_and(|ps| ps.iter().any(|(k, v)| k == "waterlogged" && v == "true"))
            })
            .unwrap();
        assert_eq!(water_amount(waterlogged), Some(8));
    }

    #[test]
    fn water_registry_preserves_flowing_and_waterlogged_states() {
        let states: Vec<_> = (0..20000)
            .filter(|s| crate::block_name_from_state(*s) == Some("water"))
            .map(|s| water_amount(s).unwrap())
            .collect();
        assert!(states.contains(&1));
        assert!(states.contains(&8));
        assert_eq!(water_amount(0), Some(0));
        assert_eq!(water_amount(i32::MAX), None);
    }
}
