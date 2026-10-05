//! Semantic predecessors retained with their actual registry capture.
use super::*;
use crate::client::{item_semantics, registry::ServerRegistryObservation};
use std::sync::Arc;

#[derive(Clone, Debug)]
enum Stack {
    Empty,
    Legacy(item_semantics::LegacyItem),
    Modern(item_semantics::Item),
}
impl Stack {
    fn read(value: &SlotKnowledge, owner: &ServerRegistryObservation) -> Result<Self> {
        Ok(match value {
            SlotKnowledge::Empty => Self::Empty,
            SlotKnowledge::Item { item } => match owner.session().version {
                crate::MinecraftVersion::Java1_16_1 => {
                    Self::Legacy(item_semantics::LegacyItem::stack(item)?)
                }
                crate::MinecraftVersion::Java1_21_11 => Self::Modern(
                    crate::versions::java_1_21_11::item_components::comparison::stack(item, owner)
                        .map_err(|e| crate::Error::new(crate::ErrorKind::Unsupported, e))?,
                ),
            },
            SlotKnowledge::Unavailable => return Err(unavailable("swap item unavailable")),
        })
    }
    fn matches(&self, other: &Self, shared: bool) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Legacy(a), Self::Legacy(b)) => a.matches(b, shared),
            (Self::Modern(a), Self::Modern(b)) => a == b,
            _ => false,
        }
    }
    fn uses_tags(&self) -> bool {
        matches!(self, Self::Modern(v) if v.components.values().any(item_semantics::Component::uses_tags))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SwapData {
    owner: Arc<ServerRegistryObservation>,
    source: Stack,
    hotbar: Stack,
}
impl SwapData {
    pub(super) fn new(
        owner: ServerRegistryObservation,
        initial: &PlayerObservation,
        source: &ObservedValue<SlotKnowledge>,
        hotbar: &ObservedValue<SlotKnowledge>,
    ) -> Result<Self> {
        if owner.session() != initial.session
            || owner.receive_sequence() != initial.receive_sequence
        {
            return Err(unavailable(
                "swap registries and inventory capture disagree",
            ));
        }
        for value in [source, hotbar] {
            if !matches!(value.source, ValueSource::Received { sequence } if sequence <= initial.receive_sequence && sequence >= owner.stamp().configuration_generation)
            {
                return Err(unavailable(
                    "swap predecessor outside registry configuration",
                ));
            }
        }
        Ok(Self {
            source: Stack::read(&source.value, &owner)?,
            hotbar: Stack::read(&hotbar.value, &owner)?,
            owner: Arc::new(owner),
        })
    }
    pub(super) fn identical(&self) -> bool {
        self.source.matches(&self.hotbar, false)
    }
    pub(super) fn classify(
        &self,
        source_slot: bool,
        actual: &ObservedValue<SlotKnowledge>,
        before: &ObservedValue<SlotKnowledge>,
        current: &ServerRegistryObservation,
    ) -> Result<(bool, bool)> {
        if self.owner.stamp() != current.stamp() {
            return Err(unavailable("swap registry configuration changed"));
        }
        let actual_key = Stack::read(&actual.value, current)?;
        if (self.source.uses_tags() || self.hotbar.uses_tags() || actual_key.uses_tags())
            && self.owner.tags().map(|v| &v.source) != current.tags().map(|v| &v.source)
        {
            return Err(unavailable(
                "swap named-holder lifetime across tag reload is unresolved",
            ));
        }
        let (before_key, after_key) = if source_slot {
            (&self.source, &self.hotbar)
        } else {
            (&self.hotbar, &self.source)
        };
        // Only the same original slot/packet can preserve legacy NaN identity.
        // A moved value and a fresh packet are independently decoded receipts.
        Ok((
            actual_key.matches(before_key, actual.source == before.source),
            actual_key.matches(after_key, false),
        ))
    }
}
