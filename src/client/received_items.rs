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
        let receipt = |observed: super::ObservedValue<SlotKnowledge>| -> Result<ReceivedSlot> {
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
            })
        };
        let slots = inventory
            .slots
            .iter()
            .cloned()
            .map(|value| value.map(receipt).transpose())
            .collect::<Result<_>>()?;
        let cursor = inventory.cursor.clone().map(receipt).transpose()?;
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
