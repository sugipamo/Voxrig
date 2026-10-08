//! Shared rules for the common item-use operations (use, release, use on a block).
use super::Hand;
use super::physics::ItemUse;
use crate::{Error, ErrorKind, MinecraftVersion, Result};

/// `LivingEntity.isUsingItem` and `getUsedItemHand` on the received flags byte.
pub(crate) fn hand_from_living_flags(flags: u8) -> Option<Hand> {
    (flags & 1 != 0).then_some(if flags & 2 != 0 {
        Hand::Off
    } else {
        Hand::Main
    })
}

/// Hit point local to the clicked cell. Both servers reject points outside the cell.
pub(crate) fn validate_cursor(cursor: [f32; 3]) -> Result<()> {
    if cursor
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            anyhow::anyhow!("block hit cursor must be finite and within 0..1"),
        ));
    }
    Ok(())
}

/// Movement effects of using `stack` (1.21.11 `minecraft:use_effects`, resolved like
/// `getOrDefault`: the stack's patch, then the item prototype, then the default).
/// 1.16.1 has one fixed behavior.
pub(crate) fn use_effects(stack: &super::ItemStack) -> Result<ItemUse> {
    if stack.id.version() == MinecraftVersion::Java1_16_1 {
        return Ok(ItemUse::DEFAULT);
    }
    if let super::ItemData::ModernComponents { patch } = &stack.data {
        if let Some(added) = patch.added.iter().find(|c| c.definition.name == NAME) {
            return decode(&added.bytes);
        }
        if patch.removed.iter().any(|d| d.name == NAME) {
            return Ok(ItemUse::DEFAULT);
        }
    }
    prototype_use_effects(stack.id.value())
}

/// Use effects of an unpatched item of this 1.21.11 native ID.
pub(crate) fn prototype_use_effects(native_id: i32) -> Result<ItemUse> {
    let prototype = super::item::modern_prototype_components(native_id)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?
        .find(|c| c.definition.name == NAME);
    prototype.map_or(Ok(ItemUse::DEFAULT), |c| decode(&c.bytes))
}

const NAME: &str = "minecraft:use_effects";

/// Received item use for a control session: the flags' receive sequence and the use
/// effects of the stack in the used hand, or why that stack cannot be resolved.
pub(crate) type ReceivedUse = (u64, std::result::Result<Option<ItemUse>, String>);

pub(crate) fn received_use(
    using: Option<&super::ObservedValue<Option<Hand>>>,
    selected_hotbar: Option<u8>,
    slots: &[Option<super::ObservedValue<super::SlotKnowledge>>],
) -> Option<ReceivedUse> {
    let using = using?;
    let super::ValueSource::Received { sequence } = using.source else {
        return None;
    };
    let Some(hand) = using.value else {
        return Some((sequence, Ok(None)));
    };
    let slot = match hand {
        Hand::Main => selected_hotbar.map(|s| 36 + usize::from(s)),
        Hand::Off => Some(45),
    };
    let effects = match slot.and_then(|i| slots.get(i)).and_then(Option::as_ref) {
        Some(super::ObservedValue {
            value: super::SlotKnowledge::Item { item },
            ..
        }) => use_effects(item).map(Some).map_err(|e| e.to_string()),
        _ => Err("the item in use is not known from received inventory".to_owned()),
    };
    Some((sequence, effects))
}

/// UseEffects.STREAM_CODEC: can_sprint, interact_vibrations, speed_multiplier (0..1).
fn decode(bytes: &[u8]) -> Result<ItemUse> {
    let invalid = || {
        Error::new(
            ErrorKind::InvalidInput,
            anyhow::anyhow!("malformed use_effects component"),
        )
    };
    let [can_sprint, vibrations, a, b, c, d] = bytes else {
        return Err(invalid());
    };
    let speed_multiplier = f32::from_be_bytes([*a, *b, *c, *d]);
    if *can_sprint > 1 || *vibrations > 1 || !(0.0..=1.0).contains(&speed_multiplier) {
        return Err(invalid());
    }
    Ok(ItemUse {
        speed_multiplier,
        can_sprint: *can_sprint == 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn living_flags_map_to_the_used_hand() {
        assert_eq!(hand_from_living_flags(0), None);
        assert_eq!(hand_from_living_flags(2), None);
        assert_eq!(hand_from_living_flags(4), None);
        assert_eq!(hand_from_living_flags(1), Some(Hand::Main));
        assert_eq!(hand_from_living_flags(3), Some(Hand::Off));
        assert_eq!(hand_from_living_flags(5), Some(Hand::Main));
    }

    #[test]
    fn modern_use_effects_follow_the_prototype() {
        let registry =
            crate::client::registry::Registry::for_version(MinecraftVersion::Java1_21_11);
        let id = |name| registry.item(name).unwrap().id.value();
        assert_eq!(
            prototype_use_effects(id("minecraft:shield")).unwrap(),
            ItemUse::DEFAULT
        );
        assert_eq!(
            prototype_use_effects(id("minecraft:iron_spear")).unwrap(),
            ItemUse {
                speed_multiplier: 1.0,
                can_sprint: true
            }
        );
        assert!(decode(&[2, 0, 0, 0, 0, 0]).is_err());
        assert!(decode(&[0, 0, 0x3f, 0xc0, 0, 0]).is_err()); // 1.5 is out of range.
    }

    #[test]
    fn cursor_must_lie_inside_the_cell() {
        assert!(validate_cursor([0.0, 0.5, 1.0]).is_ok());
        assert!(validate_cursor([1.01, 0.5, 0.5]).is_err());
        assert!(validate_cursor([f32::NAN, 0.5, 0.5]).is_err());
    }
}
