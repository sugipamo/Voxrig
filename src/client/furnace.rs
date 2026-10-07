//! Received furnace slots and pinned native ordinary PICKUP rules.
use super::{
    ObservedValue, SessionStamp, SlotKnowledge,
    container::{ContainerScreen, ScreenObservation},
    inventory::{InventoryClickSource, slot_policy::SlotPolicy, unavailable},
    registry::{Registry, ServerRegistryObservation, ServerRegistryTags},
};
use crate::{MinecraftVersion, NativeBlockState, Result};
use std::collections::BTreeMap;

/// Constructor-derived role, independent of native menu IDs and offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FurnaceSlot {
    /// Material input; placement alone does not prove a matching recipe.
    Input,
    /// Fuel input, with native acceptance and bucket capacity.
    Fuel,
    /// Received smelting output; insertion is refused by the native slot.
    Output,
}
/// An actual furnace-family opening. Missing slot receipts remain missing.
/// No recipe, remaining cook time, fuel consumption or XP is predicted.
#[derive(Clone, Debug, serde::Serialize)]
pub struct FurnaceObservation {
    /// Owning transport/world.
    session: SessionStamp,
    /// Shared screen capture boundary, not a smelting tick.
    receive_sequence: u64,
    /// Actual constructor-verified opening and received slots.
    screen: ContainerScreen,
    /// Actual cursor at the same boundary.
    cursor: Option<ObservedValue<SlotKnowledge>>,
}
impl FurnaceObservation {
    /// Transport/world shared by the observed slots.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Applied packet boundary, separate from per-slot freshness.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Original received opening and contents.
    pub fn screen(&self) -> &ContainerScreen {
        &self.screen
    }
    /// Actual received cursor at the capture boundary.
    pub fn cursor(&self) -> Option<&ObservedValue<SlotKnowledge>> {
        self.cursor.as_ref()
    }
    /// Constructor-derived click location. The mode handle rechecks the live
    /// opening, predecessor receipts and native revision before dispatch.
    pub fn slot_source(&self, role: FurnaceSlot) -> (InventoryClickSource, u16) {
        let menu = native_menu(
            self.session.version,
            self.screen.menu_name.as_deref().unwrap(),
        )
        .expect("captured native furnace topology");
        let slot = menu.role(role).expect("pinned furnace role");
        (
            InventoryClickSource::Container {
                screen: self.screen.id,
            },
            slot as u16,
        )
    }
    /// One received slot; None never means empty or completed smelting.
    pub fn slot(&self, role: FurnaceSlot) -> Option<&ObservedValue<SlotKnowledge>> {
        let (_, slot) = self.slot_source(role);
        self.screen
            .slots
            .get(usize::from(slot))
            .and_then(Option::as_ref)
    }
    pub(crate) fn capture(state: ScreenObservation) -> Result<Option<Self>> {
        let Some(screen) = state.screen else {
            return Ok(None);
        };
        if state.player_screen.is_some() || state.active_window != Some(screen.id.window_id()) {
            return Ok(None);
        }
        let Some(menu) = screen
            .menu_name
            .as_deref()
            .and_then(|m| native_menu(state.session.version, m))
        else {
            return Ok(None);
        };
        if screen.id.session() != state.session
            || screen.native_menu_id != Some(menu.native_id)
            || screen.layout.as_ref().is_none_or(|l| {
                l.total_slots != menu.total_slots || l.player_slots != menu.player_mappings()
            })
        {
            return Err(unavailable(
                "received furnace constructor/opening disagrees",
            ));
        }
        Ok(Some(Self {
            session: state.session,
            receive_sequence: state.receive_sequence,
            screen,
            cursor: state.cursor,
        }))
    }
}
impl super::Client {
    /// Observe an active received furnace/blast-furnace/smoker opening through
    /// the same API on both versions. A locally closed historical screen returns None.
    pub async fn furnace_state(&self) -> Result<Option<FurnaceObservation>> {
        FurnaceObservation::capture(self.screen_state().await?)
    }
}

#[derive(serde::Deserialize)]
struct Facts {
    menus: Vec<NativeMenu>,
    states: Vec<Shape>,
    fuel_dependency_tags: BTreeMap<String, Vec<String>>,
}
#[derive(serde::Deserialize)]
pub(crate) struct NativeMenu {
    pub(crate) native_id: i32,
    pub(crate) name: String,
    pub(crate) total_slots: usize,
    pub(crate) player_slots: Vec<super::crafting::PlayerSlot>,
    slot_policies: Vec<Policy>,
}
#[derive(serde::Deserialize)]
struct Policy {
    slot: usize,
    container_index: Option<usize>,
    capacity_overrides: BTreeMap<String, u32>,
    #[serde(flatten)]
    policy: SlotPolicy,
}
impl NativeMenu {
    fn role(&self, role: FurnaceSlot) -> Option<usize> {
        let index = match role {
            FurnaceSlot::Input => 0,
            FurnaceSlot::Fuel => 1,
            FurnaceSlot::Output => 2,
        };
        self.slot_policies
            .iter()
            .find(|p| p.container_index == Some(index))
            .map(|p| p.slot)
    }
    fn player_mappings(&self) -> Vec<super::container::PlayerSlotMapping> {
        self.player_slots
            .iter()
            .map(|m| super::container::PlayerSlotMapping {
                screen_slot: m.screen_slot,
                player_slot: if m.raw_player_slot < 9 {
                    m.raw_player_slot + 36
                } else {
                    m.raw_player_slot
                },
            })
            .collect()
    }
}
#[derive(serde::Deserialize)]
struct Shape {
    state: NativeBlockState,
    outline: Vec<[f64; 6]>,
    auxiliary: Vec<[f64; 6]>,
}
fn facts(version: MinecraftVersion) -> &'static Facts {
    static FACTS: crate::versions::table::PerVersion<Facts> =
        crate::versions::table::PerVersion::new();
    FACTS.get(version, |table| {
        serde_json::from_reader(flate2::read::GzDecoder::new(table.data.furnace_menus))
            .expect("pinned native furnace facts")
    })
}
pub(crate) fn native_menu(version: MinecraftVersion, name: &str) -> Option<&'static NativeMenu> {
    facts(version).menus.iter().find(|m| m.name == name)
}
pub(crate) fn regular_slot(
    version: MinecraftVersion,
    name: &str,
    slot: usize,
) -> Option<&'static SlotPolicy> {
    native_menu(version, name)?
        .slot_policies
        .iter()
        .find(|p| p.slot == slot)
        .map(|p| &p.policy)
}
pub(crate) fn capacity(
    version: MinecraftVersion,
    name: &str,
    slot: usize,
    item: &str,
) -> Option<u32> {
    native_menu(version, name)?
        .slot_policies
        .iter()
        .find(|p| p.slot == slot)?
        .capacity_overrides
        .get(item)
        .copied()
}
type Boxes = (&'static [[f64; 6]], &'static [[f64; 6]]);
pub(crate) fn outlines(version: MinecraftVersion, state: &NativeBlockState) -> Option<Boxes> {
    facts(version)
        .states
        .iter()
        .find(|s| s.state == *state)
        .map(|s| (s.outline.as_slice(), s.auxiliary.as_slice()))
}
pub(crate) fn validate_fuel(
    version: MinecraftVersion,
    name: &str,
    slot: usize,
    cursor: &SlotKnowledge,
    registries: Option<&ServerRegistryObservation>,
) -> Result<()> {
    let Some(menu) = native_menu(version, name) else {
        return Ok(());
    };
    if menu.role(FurnaceSlot::Fuel) != Some(slot) {
        return Ok(());
    }
    let SlotKnowledge::Item { item } = cursor else {
        return Ok(());
    };
    if item.name == "minecraft:bucket" {
        return Ok(());
    } // Native unconditional bucket allowance.
    let tags = registries.and_then(|r| r.tags()).map(|t| t.value.as_ref());
    validate_memberships(version, &item.name, item.id.value(), tags)
}
fn validate_memberships(
    version: MinecraftVersion,
    name: &str,
    id: i32,
    tags: Option<&ServerRegistryTags>,
) -> Result<()> {
    if Registry::for_version(version).item(name)?.id.value() != id {
        return Err(unavailable(
            "fuel item identity disagrees with native registry",
        ));
    }
    let actual = tags
        .and_then(|t| t.get("minecraft:item"))
        .ok_or_else(|| unavailable("received native fuel tags unavailable"))?;
    for (tag, expected) in &facts(version).fuel_dependency_tags {
        let received = actual
            .get(tag)
            .ok_or_else(|| unavailable("native fuel tag declaration unavailable"))?;
        if received.contains(&id) != expected.iter().any(|n| n == name) {
            return Err(unavailable(
                "fuel tag membership differs from pinned vanilla rules; custom fuel rules require additional support",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
