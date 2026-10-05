//! Semantic predecessors retained with their actual registry capture.
use super::*;
use crate::client::{ItemStack, item_semantics, registry::ServerRegistryObservation};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) enum Stack {
    Empty,
    Legacy(item_semantics::LegacyItem),
    Modern(item_semantics::Item),
}
impl Stack {
    pub(super) fn read(value: &SlotKnowledge, owner: &ServerRegistryObservation) -> Result<Self> {
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
    pub(super) fn matches(&self, other: &Self, shared: bool) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Legacy(a), Self::Legacy(b)) => a.matches(b, shared),
            (Self::Modern(a), Self::Modern(b)) => a == b,
            _ => false,
        }
    }
    fn same_data(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Legacy(a), Self::Legacy(b)) => a.same_data(b, false),
            (Self::Modern(a), Self::Modern(b)) => {
                a.native_id == b.native_id && a.components == b.components
            }
            _ => false,
        }
    }
    pub(super) fn uses_tags(&self) -> bool {
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

/// Immutable semantic context shared by predictions and later actual receipts.
#[derive(Clone, Debug)]
pub(crate) struct ItemContext {
    owner: Arc<ServerRegistryObservation>,
}
impl ItemContext {
    pub(crate) fn new(
        owner: ServerRegistryObservation,
        initial: &PlayerObservation,
    ) -> Result<Self> {
        if owner.session() != initial.session
            || owner.receive_sequence() != initial.receive_sequence
        {
            return Err(unavailable(
                "item registries and inventory capture disagree",
            ));
        }
        Ok(Self {
            owner: Arc::new(owner),
        })
    }
    pub(crate) fn predecessor(&self, value: &ObservedValue<SlotKnowledge>) -> Result<()> {
        if !matches!(value.source, ValueSource::Received { sequence } if sequence <= self.owner.receive_sequence() && sequence >= self.owner.stamp().configuration_generation)
        {
            return Err(unavailable(
                "item predecessor outside registry configuration",
            ));
        }
        self.key(&value.value)?;
        Ok(())
    }
    pub(super) fn key(&self, value: &SlotKnowledge) -> Result<Stack> {
        Stack::read(value, &self.owner)
    }
    pub(crate) fn same_data(&self, a: &ItemStack, b: &ItemStack) -> Result<bool> {
        Ok(self
            .key(&SlotKnowledge::Item { item: a.clone() })?
            .same_data(&self.key(&SlotKnowledge::Item { item: b.clone() })?))
    }
    pub(crate) fn equivalent_values(
        &self,
        a: &SlotKnowledge,
        b: &SlotKnowledge,
        shared: bool,
    ) -> Result<bool> {
        Ok(self.key(a)?.matches(&self.key(b)?, shared))
    }
    /// Original armor slots allow creative pickup, otherwise use the owning
    /// version's actual enchantment predicate. Stored enchantments do not apply.
    pub(super) fn armor_may_pickup(&self, item: &ItemStack, mode: GameMode) -> Result<bool> {
        use item_semantics::Component as C;
        if mode == GameMode::Creative {
            return Ok(true);
        }
        match self.key(&SlotKnowledge::Item { item: item.clone() })? {
            Stack::Empty => Ok(true),
            Stack::Legacy(stack) => {
                let entries = stack
                    .tag
                    .as_deref()
                    .and_then(crate::client::nbt::NbtValue::as_compound)
                    .and_then(|tag| tag.get("Enchantments"))
                    .and_then(crate::client::nbt::NbtValue::as_list);
                for entry in entries.into_iter().flatten() {
                    let Some(entry) = entry.as_compound() else {
                        continue;
                    };
                    let Some(id) = entry
                        .get("id")
                        .and_then(crate::client::nbt::NbtValue::as_string)
                    else {
                        continue;
                    };
                    if ["minecraft:binding_curse", "binding_curse", ":binding_curse"]
                        .into_iter()
                        .any(|name| id.utf16().iter().copied().eq(name.encode_utf16()))
                    {
                        // Native helper returns the first matching ID, including
                        // zero/negative/non-numeric levels, rather than searching later entries.
                        let level = entry.get("lvl").map_or(0, crate::client::item::legacy_int);
                        return Ok(level <= 0);
                    }
                }
                Ok(true)
            }
            Stack::Modern(stack) => {
                let id = crate::client::registry::Registry::for_version(item.id.version())
                    .item_component("minecraft:enchantments")?
                    .id
                    .value();
                let Some(C::Enchantments(entries)) = stack.components.get(&id) else {
                    return if stack.components.contains_key(&id) {
                        Err(unavailable("armor enchantment constructor unavailable"))
                    } else {
                        Ok(true)
                    };
                };
                for entry in entries.keys() {
                    if entry.registry != "minecraft:enchantment"
                        || entry.owner
                            != Some((
                                self.owner.stamp().connection_id,
                                self.owner.stamp().configuration_generation,
                            ))
                    {
                        return Err(unavailable(
                            "armor enchantment registry ownership disagrees",
                        ));
                    }
                    let id = self.owner.bind("minecraft:enchantment", entry.native_id)?;
                    let definition = self.owner.resolve(&id)?;
                    let root = crate::client::nbt::decode(&definition.data, item.id.version())?;
                    if let Some(effects) = root.root().get("effects") {
                        let effects = effects.as_compound().ok_or_else(|| {
                            unavailable("armor enchantment effects are not a compound")
                        })?;
                        // Native has() checks effect presence for every map entry,
                        // independently of the enchantment's level on this item.
                        if effects.get("minecraft:prevent_armor_change").is_some() {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
        }
    }
    /// Native player-menu routing uses the effective component's slot, separately
    /// from ArmorSlot's allowed-entity predicate. BODY/SADDLE do not route into UI armor.
    pub(super) fn transfer_equipment_slot(&self, item: &ItemStack) -> Result<Option<usize>> {
        Ok(self.equipment(item)?.and_then(|(slot, _)| match slot {
            1..=4 => Some(9 - slot as usize),
            5 => Some(45),
            _ => None,
        }))
    }
    pub(super) fn equipment_may_place(&self, item: &ItemStack, slot: usize) -> Result<bool> {
        if slot == 45 {
            return Ok(true); // Native offhand slot accepts any valid item.
        }
        Ok(self
            .equipment(item)?
            .is_some_and(|(native_slot, allowed)| (9 - slot) as i32 == native_slot && allowed))
    }
    fn equipment(&self, item: &ItemStack) -> Result<Option<(i32, bool)>> {
        use item_semantics::Component as C;
        let Stack::Modern(actual) = self.key(&SlotKnowledge::Item { item: item.clone() })? else {
            return Err(unavailable(
                "effective equipment query requires modern item",
            ));
        };
        let id = crate::client::registry::Registry::for_version(item.id.version())
            .item_component("minecraft:equippable")?
            .id
            .value();
        let Some(C::Sequence(fields)) = actual.components.get(&id) else {
            return if actual.components.contains_key(&id) {
                Err(unavailable("equippable constructor fields unavailable"))
            } else {
                Ok(None)
            };
        };
        let [
            C::Enumeration(470, slot),
            _,
            _,
            _,
            C::Optional(entities),
            _,
            _,
            _,
            _,
            _,
            _,
        ] = fields.as_slice()
        else {
            return Err(unavailable("equippable constructor shape unavailable"));
        };
        let player: item_semantics::Entry = self
            .owner
            .find_entry("minecraft:entity_type", "minecraft:player")?
            .into();
        let allowed = match entities.as_deref() {
            None => true,
            Some(C::HolderList(registry, entries)) if registry == "minecraft:entity_type" => {
                entries.iter().any(
                    |entry| matches!(entry, C::Registry(id) | C::Reference(id) if id == &player),
                )
            }
            Some(C::Tag {
                registry,
                name,
                owner,
            }) if registry == "minecraft:entity_type"
                && *owner
                    == (
                        self.owner.stamp().connection_id,
                        self.owner.stamp().configuration_generation,
                    ) =>
            {
                self.owner
                    .tags()
                    .and_then(|tags| tags.value.get(registry))
                    .and_then(|tags| tags.get(name))
                    .ok_or_else(|| unavailable("equipment entity tag was not received"))?
                    .contains(&player.native_id)
            }
            _ => return Err(unavailable("equipment entity set constructor unavailable")),
        };
        Ok(Some((*slot, allowed)))
    }
    pub(crate) fn classify(
        &self,
        actual: &ObservedValue<SlotKnowledge>,
        before: &ObservedValue<SlotKnowledge>,
        after: &SlotKnowledge,
        current: &ServerRegistryObservation,
    ) -> Result<(bool, bool)> {
        if current.stamp() != self.owner.stamp() {
            return Err(unavailable("item registry configuration changed"));
        }
        let actual_key = Stack::read(&actual.value, current)?;
        let before_key = self.key(&before.value)?;
        let after_key = self.key(after)?;
        if (actual_key.uses_tags() || before_key.uses_tags() || after_key.uses_tags())
            && self.owner.tags().map(|v| &v.source) != current.tags().map(|v| &v.source)
        {
            return Err(unavailable(
                "item named-holder lifetime across tag reload is unresolved",
            ));
        }
        Ok((
            actual_key.matches(&before_key, actual.source == before.source),
            actual_key.matches(&after_key, false),
        ))
    }
}
