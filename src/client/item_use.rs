//! Shared rules for the common item-use operations (use, release, use on a block).
use super::Hand;
use super::physics::ItemUse;
use crate::{Error, ErrorKind, MinecraftVersion, Result};

/// Expected held identity was unknown or different at the dispatch boundary.
/// No use-item packet was sent. Count and item data are not compared.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub struct UseItemRefusal {
    /// Hand whose player-inventory slot was inspected.
    pub hand: Hand,
    /// Required selected hotbar index (0..8), including for offhand use.
    pub expected_selected_hotbar: u8,
    /// Known received or fully submitted selection; None means unknown.
    pub observed_selected_hotbar: Option<u8>,
    /// Required version-bound item definition.
    pub expected_item: super::registry::RegistryId,
    /// Received slot value; None means no received value. Empty is explicit.
    pub observed_item: Option<super::SlotKnowledge>,
}
impl std::fmt::Display for UseItemRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "use item refused for {:?}: expected selected slot {} and item {:?}, observed selected slot {:?} and item {:?}",
            self.hand,
            self.expected_selected_hotbar,
            self.expected_item,
            self.observed_selected_hotbar,
            self.observed_item
        )
    }
}
impl std::error::Error for UseItemRefusal {}

/// Caller holds the native coherent state lock until dispatch completes.
pub(crate) fn validate_expected(
    player: &super::PlayerObservation,
    hand: Hand,
    selected: u8,
    item: super::registry::RegistryId,
) -> Result<()> {
    if selected > 8 {
        return Err(crate::error::invalid_input(
            "expected hotbar slot must be 0..8",
        ));
    }
    super::registry::Registry::for_version(player.session.version).item_definition(item)?;
    let observed_selected = player
        .selected_hotbar
        .as_ref()
        .filter(|s| {
            matches!(
                s.source,
                super::ValueSource::Received { .. } | super::ValueSource::Submitted
            )
        })
        .map(|s| s.value);
    let slot = match hand {
        Hand::Main => observed_selected
            .filter(|s| *s <= 8)
            .map(|s| 36 + usize::from(s)),
        Hand::Off => Some(45),
    };
    let observed = slot
        .and_then(|s| player.inventory.slots.get(s))
        .and_then(Option::as_ref)
        .filter(|s| matches!(s.source, super::ValueSource::Received { .. }));
    if observed_selected == Some(selected)
        && matches!(observed.map(|s| &s.value), Some(super::SlotKnowledge::Item { item: actual }) if actual.id == item)
    {
        return Ok(());
    }
    Err(Error::new(
        ErrorKind::State,
        UseItemRefusal {
            hand,
            expected_selected_hotbar: selected,
            observed_selected_hotbar: observed_selected,
            expected_item: item,
            observed_item: observed.map(|s| s.value.clone()),
        },
    ))
}

/// `LivingEntity.isUsingItem` and `getUsedItemHand` on the received flags byte.
pub(crate) fn hand_from_living_flags(flags: u8) -> Option<Hand> {
    (flags & 1 != 0).then_some(if flags & 2 != 0 {
        Hand::Off
    } else {
        Hand::Main
    })
}

/// Compatibility API hit point local to the clicked cell (0..1). This is a
/// local packet-construction contract, narrower than either server handler.
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
pub(crate) async fn expect_test_contract(
    client: &crate::Client,
    hand: Hand,
    item: super::registry::RegistryId,
) {
    use super::{SlotKnowledge, ValueSource};
    let original = client.player_state().await.unwrap();
    let index = if hand == Hand::Main { 36 } else { 45 };
    let validate = |player: &super::PlayerObservation| validate_expected(player, hand, 0, item);
    let mut p = original.clone();
    p.selected_hotbar = None;
    assert!(validate(&p).unwrap_err().use_item_refusal().is_some());
    p = original.clone();
    p.selected_hotbar.as_mut().unwrap().source = ValueSource::Predicted;
    assert!(validate(&p).unwrap_err().use_item_refusal().is_some());
    for value in [
        None,
        Some(super::received(SlotKnowledge::Unavailable, 90)),
        Some(super::received(SlotKnowledge::Empty, 90)),
    ] {
        p = original.clone();
        p.inventory.slots[index] = value;
        assert!(validate(&p).unwrap_err().use_item_refusal().is_some());
    }
    p = original.clone();
    p.inventory.slots[index].as_mut().unwrap().source = ValueSource::LocalCache;
    assert!(validate(&p).unwrap_err().use_item_refusal().is_some());
    p = original.clone();
    if let SlotKnowledge::Item { item } = &mut p.inventory.slots[index].as_mut().unwrap().value {
        item.count = 2;
    }
    validate(&p).unwrap(); // Count is explicitly outside the identity contract.
    assert_eq!(
        validate_expected(&original, hand, 9, item)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let other_version = if client.version() == MinecraftVersion::Java1_16_1 {
        MinecraftVersion::Java1_21_11
    } else {
        MinecraftVersion::Java1_16_1
    };
    let wrong_version = super::registry::Registry::for_version(other_version)
        .item("minecraft:dirt")
        .unwrap()
        .id;
    assert_eq!(
        validate_expected(&original, hand, 0, wrong_version)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let wrong_item = client.registry().item("minecraft:stone").unwrap().id;
    let error = validate_expected(&original, hand, 0, wrong_item).unwrap_err();
    let refusal = error.use_item_refusal().unwrap().clone();
    let error = Error::from(anyhow::Error::new(refusal.clone()).context("context"));
    assert_eq!(error.kind(), ErrorKind::State);
    assert_eq!(error.use_item_refusal(), Some(&refusal));
    // Body retains typed refusals after its asynchronous action terminates.
    use crate::body::{ActionStatus, Body, BodyConfig, HeadlessBody};
    let body = Body::new(
        HeadlessBody::new(
            client.clone(),
            BodyConfig {
                settle_window: std::time::Duration::from_millis(10),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    for (slot, accepted) in [(1, false), (0, true)] {
        let id = body
            .backend()
            .use_item_expect(hand, slot, item)
            .await
            .unwrap();
        let status = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let status = body.action_status(id).await.unwrap();
                if status.is_terminal() {
                    break status;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if accepted {
            assert!(matches!(status, ActionStatus::Accepted), "{status:?}");
            assert!(body.backend().use_item_refusal(id).unwrap().is_none());
        } else {
            assert!(
                matches!(status, ActionStatus::Rejected { .. }),
                "{status:?}"
            );
            assert_eq!(
                body.backend()
                    .use_item_refusal(id)
                    .unwrap()
                    .unwrap()
                    .observed_selected_hotbar,
                Some(0)
            );
        }
        // Wait until the worker has released its action ownership.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
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
