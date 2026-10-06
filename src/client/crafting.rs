//! Received crafting grids using the selected original menu's topology.
//! Displayed results are server receipts, not evidence of recipe consumption.
use super::container::{PlayerScreenAccess, ScreenId, ScreenObservation};
use super::inventory::{InventorySource, slot_policy::SlotPolicy};
use super::received_items::ReceiptLocation;
use super::registry::ServerRegistryObservation;
use super::{PlayerObservation, ReceivedSlot, SessionStamp};
use crate::{MinecraftVersion, Result};
use std::sync::{Arc, OnceLock};

/// UI supplying the captured grid. Access basis does not grant click authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CraftingSource {
    /// Player grid, with actual received or submitted-close admission explicit.
    Player {
        /// Received player UI or the exact completely submitted local close.
        access: PlayerScreenAccess,
    },
    /// Crafting table bound to one actual received opening.
    Table {
        /// Session/world/opening identity of the table screen.
        screen: ScreenId,
    },
}

/// Immutable input and result receipts from one adapter capture boundary.
/// Only Client creates this value. Missing inputs remain distinct from empty.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedCrafting {
    session: SessionStamp,
    receive_sequence: u64,
    source: CraftingSource,
    width: usize,
    height: usize,
    input_slots: Vec<usize>,
    inputs: Vec<Option<ReceivedSlot>>,
    result: Option<ReceivedSlot>,
    registries: Arc<ServerRegistryObservation>,
}
impl ReceivedCrafting {
    /// Transport and world at the capture boundary.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Last applied packet at capture, not a per-input freshness guarantee.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Actual UI and its admission basis.
    pub fn source(&self) -> CraftingSource {
        self.source
    }
    /// Native grid dimensions, using zero-based x/y coordinates.
    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }
    /// Owning registries from the same capture boundary.
    pub fn registry_state(&self) -> &ServerRegistryObservation {
        &self.registries
    }
    fn index(&self, x: usize, y: usize) -> Result<usize> {
        if x >= self.width || y >= self.height {
            return Err(super::registry::invalid(
                "coordinate outside native crafting grid",
            ));
        }
        Ok(y * self.width + x)
    }
    /// One actual input receipt; absence does not mean an empty input.
    pub fn input(&self, x: usize, y: usize) -> Result<Option<&ReceivedSlot>> {
        Ok(self.inputs[self.index(x, y)?].as_ref())
    }
    /// Constructor-derived input location for `click_inventory`.
    /// The operation rechecks the live opening, mode and predecessor receipts.
    pub fn input_source(&self, x: usize, y: usize) -> Result<(InventorySource, u16)> {
        let slot = self.input_slots[self.index(x, y)?];
        let source = match self.source {
            CraftingSource::Player { .. } => InventorySource::Player,
            CraftingSource::Table { screen } => InventorySource::Container { screen },
        };
        Ok((
            source,
            u16::try_from(slot).map_err(|_| {
                super::registry::invalid("native input slot exceeds protocol index")
            })?,
        ))
    }
    /// Actual displayed result. This does not prove permission to take it or
    /// predict recipe ingredients, consumption, remainders or completion.
    pub fn result(&self) -> Option<&ReceivedSlot> {
        self.result.as_ref()
    }

    pub(crate) fn capture(
        player: &PlayerObservation,
        screen: &ScreenObservation,
        registries: ServerRegistryObservation,
    ) -> Result<Option<Self>> {
        if player.session != screen.session
            || player.session != registries.session()
            || player.receive_sequence != screen.receive_sequence
            || player.receive_sequence != registries.receive_sequence()
            || player.inventory.player_screen != screen.player_screen
            || player.inventory.window_id != screen.active_window
        {
            return Err(super::registry::invalid(
                "crafting capture boundaries differ",
            ));
        }
        let (source, name, slots) = if let Some(access) = screen.player_screen {
            match access {
                PlayerScreenAccess::Received if screen.active_window == Some(0) => {}
                PlayerScreenAccess::SubmittedClose { close }
                    if close.screen().session() == player.session => {}
                _ => {
                    return Err(super::registry::invalid(
                        "crafting player access belongs to another boundary",
                    ));
                }
            }
            (
                CraftingSource::Player { access },
                "minecraft:player",
                &player.inventory.slots,
            )
        } else if let Some(table) = screen
            .screen
            .as_ref()
            .filter(|s| s.menu_name.as_deref() == Some("minecraft:crafting"))
        {
            if table.id.session() != player.session
                || screen.active_window != Some(table.id.window_id())
                || table.id.opened_sequence() > player.receive_sequence
            {
                return Err(super::registry::invalid(
                    "crafting table opening belongs to another boundary",
                ));
            }
            let menu = native_menu(player.session.version, "minecraft:crafting")
                .expect("pinned table topology");
            if table.native_menu_id != menu.native_id
                || table.layout.as_ref().map(|l| l.total_slots) != Some(menu.total_slots)
            {
                return Err(super::registry::invalid(
                    "crafting table native layout unavailable",
                ));
            }
            (
                CraftingSource::Table { screen: table.id },
                "minecraft:crafting",
                &table.slots,
            )
        } else {
            return Ok(None);
        };
        let menu = native_menu(player.session.version, name).expect("pinned crafting topology");
        let registries = Arc::new(registries);
        let capture = |slot: usize| -> Result<Option<ReceivedSlot>> {
            let location = match source {
                CraftingSource::Player { .. } => ReceiptLocation::PlayerSlot(slot),
                CraftingSource::Table { screen } => ReceiptLocation::ContainerSlot { screen, slot },
            };
            slots
                .get(slot)
                .cloned()
                .flatten()
                .map(|v| {
                    if let CraftingSource::Table { screen } = source {
                        if !matches!(v.source, super::ValueSource::Received { sequence } if sequence >= screen.opened_sequence()) {
                            return Err(super::registry::invalid("crafting slot predates its opening"));
                        }
                    }
                    Ok(v)
                })
                .transpose()?
                .map(|v| ReceivedSlot::capture(&registries, v, location))
                .transpose()
        };
        let mut input_slots = vec![0; menu.grid_width * menu.grid_height];
        for input in &menu.input_slots {
            input_slots[input.grid_index] = input.screen_slot;
        }
        let inputs = input_slots
            .iter()
            .map(|&slot| capture(slot))
            .collect::<Result<_>>()?;
        let result = capture(menu.result_slot)?;
        Ok(Some(Self {
            session: player.session,
            receive_sequence: player.receive_sequence,
            source,
            width: menu.grid_width,
            height: menu.grid_height,
            input_slots,
            inputs,
            result,
            registries,
        }))
    }
}

#[derive(serde::Deserialize)]
struct Menus {
    menus: Vec<NativeCraftingMenu>,
}
#[derive(serde::Deserialize)]
pub(crate) struct NativeCraftingMenu {
    pub(crate) name: String,
    pub(crate) native_id: Option<i32>,
    pub(crate) total_slots: usize,
    pub(crate) grid_width: usize,
    pub(crate) grid_height: usize,
    pub(crate) result_slot: usize,
    input_slots: Vec<InputSlot>,
    pub(crate) player_slots: Vec<PlayerSlot>,
    slot_policies: Vec<NativePolicy>,
}
#[derive(serde::Deserialize)]
struct InputSlot {
    grid_index: usize,
    screen_slot: usize,
}
#[derive(serde::Deserialize)]
pub(crate) struct PlayerSlot {
    pub(crate) screen_slot: usize,
    pub(crate) raw_player_slot: usize,
}
#[derive(serde::Deserialize)]
struct NativePolicy {
    slot: usize,
    native_class: String,
    #[serde(flatten)]
    policy: SlotPolicy,
}
pub(crate) fn native_menu(
    version: MinecraftVersion,
    name: &str,
) -> Option<&'static NativeCraftingMenu> {
    static LEGACY: OnceLock<Menus> = OnceLock::new();
    static MODERN: OnceLock<Menus> = OnceLock::new();
    let parse = |bytes: &[u8]| {
        serde_json::from_reader(flate2::read::GzDecoder::new(bytes))
            .expect("pinned original crafting topology")
    };
    let menus = match version {
        MinecraftVersion::Java1_16_1 => LEGACY.get_or_init(|| {
            parse(include_bytes!(
                "../../data/client_api/crafting_menus-1.16.1.json.gz"
            ))
        }),
        MinecraftVersion::Java1_21_11 => MODERN.get_or_init(|| {
            parse(include_bytes!(
                "../../data/client_api/crafting_menus-1.21.11.json.gz"
            ))
        }),
    };
    menus.menus.iter().find(|m| m.name == name)
}
pub(crate) fn regular_slot(
    version: MinecraftVersion,
    menu: &str,
    index: usize,
) -> Option<&'static SlotPolicy> {
    let menu = native_menu(version, menu)?;
    if index == menu.result_slot {
        return None;
    }
    let policy = menu.slot_policies.iter().find(|s| s.slot == index)?;
    let generic_class = match version {
        MinecraftVersion::Java1_16_1 => "bhw",
        MinecraftVersion::Java1_21_11 => "dji",
    };
    // Empty-result mayPickup never establishes a result-take policy. Ordinary
    // input/player slots use the original generic Slot implementation only.
    (policy.native_class == generic_class).then_some(&policy.policy)
}

#[cfg(test)]
mod tests;
