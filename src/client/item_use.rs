//! Shared rules for the common item-use operations (use, release, use on a block).
use super::Hand;
use crate::{Error, ErrorKind, Result};

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

/// Item-use state blocks motion prediction; the slowdown is not modeled.
pub(crate) const USING_ITEM_STOP: &str = "item in use (its slowdown is not modeled)";

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
    fn cursor_must_lie_inside_the_cell() {
        assert!(validate_cursor([0.0, 0.5, 1.0]).is_ok());
        assert!(validate_cursor([1.01, 0.5, 0.5]).is_err());
        assert!(validate_cursor([f32::NAN, 0.5, 0.5]).is_err());
    }
}
