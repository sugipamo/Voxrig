//! Admission policy used only by acceptance/best-effort paths.
//! Strict receipt callers still inspect every retained family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Domain {
    Strict,
    CurrentInventory,
    Motion,
    Dig,
    Screen,
    ItemUse,
    Selection,
    Chat,
}
impl Domain {
    pub(crate) fn motion(self) -> bool {
        matches!(self, Self::Strict | Self::CurrentInventory | Self::Motion)
    }
    pub(crate) fn inventory(self) -> bool {
        matches!(self, Self::Strict | Self::Screen)
    }
    pub(crate) fn dig(self) -> bool {
        matches!(self, Self::Strict | Self::Dig)
    }
    pub(crate) fn held(self) -> bool {
        !matches!(self, Self::Chat | Self::Motion | Self::Screen)
    }
}

fn mapped_slot(slot: u16, screen: Option<&super::container::ContainerScreen>) -> Option<usize> {
    match screen {
        None => Some(usize::from(slot)),
        Some(screen) => screen
            .layout
            .as_ref()?
            .player_slots
            .iter()
            .find(|m| m.screen_slot == usize::from(slot))
            .map(|m| m.player_slot),
    }
}
pub(crate) fn click_changes_hand(
    record: &super::inventory::InventoryClickRecord,
    selected: Option<u8>,
) -> bool {
    selected.is_none_or(|s| {
        mapped_slot(record.source_slot, record.initial_screen.as_ref()) == Some(36 + usize::from(s))
    })
}
pub(crate) fn swap_changes_hand(
    record: &super::inventory::InventorySwapRecord,
    selected: Option<u8>,
) -> bool {
    selected.is_none_or(|s| {
        record.hotbar == s
            || mapped_slot(record.source_slot, record.initial_screen.as_ref())
                == Some(36 + usize::from(s))
    })
}
pub(crate) fn transfer_changes_hand(
    record: &super::inventory::InventoryTransferRecord,
    selected: Option<u8>,
) -> bool {
    selected.is_none_or(|s| {
        record
            .changed_slots
            .iter()
            .any(|slot| slot.player_slot == Some(36 + usize::from(s)))
    })
}
