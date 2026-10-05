//! Immutable inventory receipts and their registry owner, captured together.
use super::registry::ServerRegistryObservation;
use super::{InventoryObservation, ItemStack, SessionStamp, SlotKnowledge, ValueSource};
use crate::{Error, ErrorKind, Result};
use std::sync::Arc;

/// Player inventory receipts and registries from one adapter boundary.
/// Only the Client can create this value. Local click predictions and caches are
/// excluded. Serialized receipts are diagnostics, not a way to create authority.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedInventory {
    session: SessionStamp,
    receive_sequence: u64,
    registries: Arc<ServerRegistryObservation>,
    slots: Vec<Option<ReceivedSlot>>,
    cursor: Option<ReceivedSlot>,
}

impl ReceivedInventory {
    /// Transport and world at capture; distinct from registry configuration lifetime.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Last applied packet at capture, not a freshness guarantee for each slot.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Registry snapshot from the same capture boundary.
    pub fn registry_state(&self) -> &ServerRegistryObservation {
        &self.registries
    }
    /// Player-screen slots 0..45. Absence means no receipt, not an empty slot.
    pub fn slots(&self) -> &[Option<ReceivedSlot>] {
        &self.slots
    }
    /// A player-screen slot, rejecting indices outside this captured layout.
    pub fn slot(&self, index: usize) -> Result<Option<&ReceivedSlot>> {
        self.slots
            .get(index)
            .map(Option::as_ref)
            .ok_or_else(|| invalid("slot index is outside the received player inventory"))
    }
    /// Actual received cursor, absent until established by a packet.
    pub fn cursor(&self) -> Option<&ReceivedSlot> {
        self.cursor.as_ref()
    }
    pub(crate) fn capture(
        session: SessionStamp,
        receive_sequence: u64,
        inventory: &InventoryObservation,
        registries: ServerRegistryObservation,
    ) -> Result<Self> {
        if session != registries.session() || receive_sequence != registries.receive_sequence() {
            return Err(invalid(
                "inventory and registries have different capture boundaries",
            ));
        }
        let registries = Arc::new(registries);
        let receipt = |observed: super::ObservedValue<SlotKnowledge>,
                       location|
         -> Result<ReceivedSlot> {
            let ValueSource::Received { sequence } = observed.source else {
                return Err(invalid("inventory value is not a received slot"));
            };
            if sequence > receive_sequence || sequence < registries.stamp().configuration_generation
            {
                return Err(invalid(
                    "slot receipt is outside its registry configuration",
                ));
            }
            if let SlotKnowledge::Item { item } = &observed.value {
                let registry = super::registry::Registry::for_version(session.version);
                let definition = registry.item_by_native_id(item.id.value())?;
                if item.id.version() != session.version
                    || definition.id != item.id
                    || definition.name != item.name
                {
                    return Err(invalid(
                        "received item identity belongs to another registry",
                    ));
                }
            }
            Ok(ReceivedSlot {
                value: observed.value,
                receive_sequence: sequence,
                registries: Arc::clone(&registries),
                location,
            })
        };
        let slots = inventory
            .slots
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, value)| {
                value
                    .map(|v| receipt(v, ReceiptLocation::PlayerSlot(slot)))
                    .transpose()
            })
            .collect::<Result<_>>()?;
        let cursor = inventory
            .cursor
            .clone()
            .map(|v| receipt(v, ReceiptLocation::Cursor))
            .transpose()?;
        Ok(Self {
            session,
            receive_sequence,
            registries,
            slots,
            cursor,
        })
    }
}

/// One actual slot or cursor receipt with immutable registry ownership.
#[derive(Clone)]
pub struct ReceivedSlot {
    value: SlotKnowledge,
    receive_sequence: u64,
    registries: Arc<ServerRegistryObservation>,
    location: ReceiptLocation,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReceiptLocation {
    PlayerSlot(usize),
    Cursor,
}
impl std::fmt::Debug for ReceivedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedSlot")
            .field("value", &self.value)
            .field("receive_sequence", &self.receive_sequence)
            .field("session", &self.registries.session())
            .field("registry_stamp", &self.registries.stamp())
            .finish()
    }
}
impl serde::Serialize for ReceivedSlot {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        // Include the large registry payload once at the inventory boundary,
        // retaining just its owner alongside each independently printable slot.
        #[derive(serde::Serialize)]
        struct Receipt<'a> {
            value: &'a SlotKnowledge,
            receive_sequence: u64,
            session: SessionStamp,
            registry_stamp: super::registry::ServerRegistryStamp,
        }
        Receipt {
            value: &self.value,
            receive_sequence: self.receive_sequence,
            session: self.registries.session(),
            registry_stamp: self.registries.stamp(),
        }
        .serialize(serializer)
    }
}
impl ReceivedSlot {
    /// Exact received knowledge; unavailable and empty remain distinct.
    pub fn value(&self) -> &SlotKnowledge {
        &self.value
    }
    /// Packet establishing this value, which may precede the capture boundary.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Registry owner retained by the receipt, including after Client reconnects.
    pub fn registry_state(&self) -> &ServerRegistryObservation {
        &self.registries
    }
    /// A borrowed item receipt, only when this slot contains a known stack.
    pub fn item(&self) -> Option<ReceivedItem<'_>> {
        match &self.value {
            SlotKnowledge::Item { item } => Some(ReceivedItem {
                item,
                receipt: self,
            }),
            _ => None,
        }
    }
}

/// A received stack paired with its owning registry snapshot.
/// Copying public ItemStack bytes alone cannot create a ReceivedItem.
/// This type does not imply native item equality or permission to mutate a slot.
///
/// ```compile_fail
/// use voxrig::client::{ItemStack, ReceivedItem, ReceivedSlot};
/// fn forge<'a>(item: &'a ItemStack, receipt: &'a ReceivedSlot) -> ReceivedItem<'a> {
///     ReceivedItem { item, receipt }
/// }
/// ```
#[derive(Clone, Copy)]
pub struct ReceivedItem<'a> {
    item: &'a ItemStack,
    receipt: &'a ReceivedSlot,
}
impl std::fmt::Debug for ReceivedItem<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedItem")
            .field("stack", &self.item)
            .field("receive_sequence", &self.receive_sequence())
            .field("session", &self.registry_state().session())
            .field("registry_stamp", &self.registry_state().stamp())
            .finish()
    }
}
impl<'a> ReceivedItem<'a> {
    /// Compare count, item type and effective native component values within
    /// the same registry configuration. Raw bytes and persistent hashes are not
    /// item equality. Unverified constructors return an explicit error.
    /// Legacy NBT constructor coercions and modern prototype/patch values use
    /// their own native rules. Matching decoded source/location preserves legacy
    /// NaN identity across captures; distinct receipts stay independently decoded.
    /// Items containing named tags also require the same received tag source;
    /// named-holder identity across tag reloads is not yet verified.
    ///
    /// ```no_run
    /// use voxrig::client::prelude::*;
    /// async fn compare(client: &Client) -> Result<()> {
    ///     let inventory = client.received_inventory().await?;
    ///     if let (Some(a), Some(b)) = (
    ///         inventory.slot(9)?.and_then(ReceivedSlot::item),
    ///         inventory.slot(10)?.and_then(ReceivedSlot::item),
    ///     ) {
    ///         println!("same native stack: {}", a.native_equivalent(&b)?);
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn native_equivalent(&self, other: &ReceivedItem<'_>) -> Result<bool> {
        self.compare(other, true)
    }
    /// Compare native item type and data while ignoring only the outer stack count.
    /// Nested item counts remain part of their data. This does not prove slot
    /// acceptance, stackability or permission to merge/mutate an inventory.
    /// Registry and named-tag lifetime requirements are the same as native_equivalent.
    pub fn native_data_equivalent(&self, other: &ReceivedItem<'_>) -> Result<bool> {
        self.compare(other, false)
    }
    fn compare(&self, other: &ReceivedItem<'_>, include_count: bool) -> Result<bool> {
        if self.registry_state().stamp() != other.registry_state().stamp() {
            return Err(invalid(
                "item comparison requires the same registry configuration",
            ));
        }
        match self.item.id.version() {
            crate::MinecraftVersion::Java1_21_11 => {
                let compare = || -> anyhow::Result<bool> {
                    let left = crate::versions::java_1_21_11::item_components::comparison::stack(
                        self.item,
                        self.registry_state(),
                    )?;
                    let right = crate::versions::java_1_21_11::item_components::comparison::stack(
                        other.item,
                        other.registry_state(),
                    )?;
                    if (left
                        .components
                        .values()
                        .any(super::item_semantics::Component::uses_tags)
                        || right
                            .components
                            .values()
                            .any(super::item_semantics::Component::uses_tags))
                        && self.registry_state().tags().map(|v| &v.source)
                            != other.registry_state().tags().map(|v| &v.source)
                    {
                        anyhow::bail!(
                            "native named-holder lifetime across tag reloads is not yet verified"
                        );
                    }
                    Ok(if include_count {
                        left == right
                    } else {
                        left.native_id == right.native_id && left.components == right.components
                    })
                };
                compare().map_err(|error| Error::new(ErrorKind::Unsupported, error))
            }
            crate::MinecraftVersion::Java1_16_1 => {
                let left = super::item_semantics::LegacyItem::stack(self.item)?;
                let right = super::item_semantics::LegacyItem::stack(other.item)?;
                let shared = self.receipt.location == other.receipt.location
                    && self.receive_sequence() == other.receive_sequence()
                    && self.registry_state().session() == other.registry_state().session();
                Ok(if include_count {
                    left.matches(&right, shared)
                } else {
                    left.same_data(&right, shared)
                })
            }
        }
    }
    /// Exact version-bound stack and original data bytes.
    pub fn stack(&self) -> &'a ItemStack {
        self.item
    }
    /// Packet establishing this stack.
    pub fn receive_sequence(&self) -> u64 {
        self.receipt.receive_sequence()
    }
    /// Registry snapshot retained with the received stack.
    pub fn registry_state(&self) -> &'a ServerRegistryObservation {
        self.receipt.registry_state()
    }
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}

#[cfg(test)]
mod tests;
