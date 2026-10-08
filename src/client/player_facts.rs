//! Received own-player facts beyond position and health: attributes, effects and air.
use super::control::Modifier;
use crate::MinecraftVersion;

/// One received attribute of the own player.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PlayerAttribute {
    /// Received base value.
    pub base: f64,
    /// Received modifiers in arrival order.
    pub modifiers: Vec<Modifier>,
    /// `AttributeInstance.calculateValue` in the version's modifier order,
    /// before the attribute's range clamp.
    pub value: f64,
}

/// One received status effect of the own player. No local countdown is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PlayerEffect {
    /// Native amplifier (level − 1).
    pub amplifier: i32,
    /// Remaining ticks when received; -1 is infinite (1.21.11).
    pub duration_at_receipt: i32,
    /// Ambient (beacon) effect.
    pub ambient: bool,
    /// Particles shown.
    pub visible: bool,
    /// Icon shown.
    pub show_icon: bool,
}

pub(crate) fn attribute(
    version: MinecraftVersion,
    base: f64,
    modifiers: Vec<Modifier>,
) -> PlayerAttribute {
    let refs: Vec<&Modifier> = modifiers.iter().collect();
    let value = super::physics::attribute_value(version, base, &refs);
    PlayerAttribute {
        base,
        modifiers,
        value,
    }
}

/// 1.16.1 keys (`generic.attack_speed`, `horse.jump_strength`, …) in 1.21.11 names
/// (`minecraft:attack_speed`, `minecraft:jump_strength`), so both versions share keys.
pub(crate) fn legacy_attribute_name(key: &str) -> String {
    let key = key.strip_prefix("minecraft:").unwrap_or(key);
    let key = ["generic.", "horse.", "zombie."]
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .unwrap_or(key);
    format!("minecraft:{key}")
}

/// Namespaced name of a native attribute ID (1.21.11).
pub(crate) fn modern_attribute_name(id: i32) -> Option<String> {
    name_of(
        &super::physics::blocks::table(MinecraftVersion::Java1_21_11).attribute_ids,
        id,
    )
}

/// Namespaced name of a native effect ID.
pub(crate) fn effect_name(version: MinecraftVersion, id: i32) -> Option<String> {
    name_of(&super::physics::blocks::table(version).effect_ids, id)
}

fn name_of(ids: &std::collections::BTreeMap<String, i32>, id: i32) -> Option<String> {
    ids.iter().find(|(_, v)| **v == id).map(|(k, _)| k.clone())
}

/// Native effect flags: ambient 1, visible 2, icon 4.
pub(crate) fn effect(amplifier: i32, duration_at_receipt: i32, flags: u8) -> PlayerEffect {
    PlayerEffect {
        amplifier,
        duration_at_receipt,
        ambient: flags & 1 != 0,
        visible: flags & 2 != 0,
        show_icon: flags & 4 != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names_match_modern_names() {
        assert_eq!(
            legacy_attribute_name("generic.attack_speed"),
            "minecraft:attack_speed"
        );
        assert_eq!(
            legacy_attribute_name("minecraft:generic.movement_speed"),
            "minecraft:movement_speed"
        );
        assert_eq!(
            legacy_attribute_name("horse.jump_strength"),
            "minecraft:jump_strength"
        );
        assert_eq!(
            modern_attribute_name(4).as_deref(),
            Some("minecraft:attack_speed")
        );
        assert_eq!(
            effect_name(MinecraftVersion::Java1_16_1, 1).as_deref(),
            Some("minecraft:speed")
        );
        assert_eq!(
            effect_name(MinecraftVersion::Java1_21_11, 0).as_deref(),
            Some("minecraft:speed")
        );
    }

    #[test]
    fn value_adds_then_multiplies_the_added_base() {
        use super::super::control::ModifierOperation::*;
        let m = |operation, amount| Modifier {
            id: "minecraft:test".into(),
            operation,
            amount,
        };
        let a = attribute(
            MinecraftVersion::Java1_21_11,
            4.0,
            vec![
                m(Addition, -2.4),
                m(MultiplyBase, 0.5),
                m(MultiplyTotal, 0.1),
            ],
        );
        assert_eq!(a.value, (1.6 + 1.6 * 0.5) * 1.1);
    }
}
