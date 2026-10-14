//! Elytra request, receipt and equipment inputs, separate from held-flight permission.
use super::{ItemData, ItemProperties, ItemStack, ObservedValue, SlotKnowledge, ValueSource};
use crate::{MinecraftVersion, Result};

/// Actual usable chest Elytra sample converted for an owned glide model.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct GliderEquipment {
    /// Original slot receipt; this is not a locally submitted equip action.
    pub sequence: u64,
    /// Exact received native item and data.
    pub item: ItemStack,
    /// Native effective durability predicates, not predicted wear.
    pub properties: ItemProperties,
}

/// An owned start intent and actual metadata, never a start-packet acknowledgement.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct GlideRecord {
    /// Original receipt fence before this owner requested START_FALL_FLYING.
    pub after_sequence: u64,
    /// Complete start write; uncertainty or cancellation never triggers replay.
    pub dispatched: bool,
    /// First actual set fall-flying flag after the start fence.
    pub accepted_sequence: Option<u64>,
    /// First actual cleared flag after acceptance. Restoration does not resume.
    pub ended_sequence: Option<u64>,
    /// Latest actual own-player flag, distinct from numerical model flight.
    pub actual: Option<ObservedValue<bool>>,
    /// Actual usable Elytra at admission; received wear is not predicted.
    pub equipment: GliderEquipment,
    /// First packet making equipment unavailable or unusable; restoration cannot resume.
    pub equipment_invalidated_sequence: Option<u64>,
    /// Explicit ticks spent awaiting an actual flag (maximum forty).
    pub waiting_ticks: u8,
}

pub(crate) fn equipment(slot: Option<&ObservedValue<SlotKnowledge>>) -> Result<GliderEquipment> {
    let invalid = super::registry::invalid;
    let slot = slot.ok_or_else(|| invalid("glide requires received chest equipment"))?;
    let ValueSource::Received { sequence } = slot.source else {
        return Err(invalid("glide equipment is not an actual slot receipt"));
    };
    let SlotKnowledge::Item { item } = &slot.value else {
        return Err(invalid("glide requires usable received chest Elytra"));
    };
    if item.name != "minecraft:elytra" || item.count != 1 {
        return Err(invalid("only vanilla chest Elytra is qualified for glide"));
    }
    let properties = item.properties()?;
    if item.id.version() == MinecraftVersion::Java1_16_1 {
        // Original ElytraItem.isFlyEnabled compares native damage/max even if
        // unbreakable; modern's nextDamageWillBreak is a different predicate.
        if properties.damage >= properties.max_damage - 1 {
            return Err(invalid("received Elytra is not fly-enabled"));
        }
    } else {
        let prototype: Vec<_> = super::item::modern_prototype_components(item.id.value())
            .map_err(|e| invalid(&e.to_string()))?
            .collect();
        let patch = match &item.data {
            ItemData::Default => None,
            ItemData::ModernComponents { patch } => Some(patch),
            _ => return Err(invalid("glider component representation unavailable")),
        };
        let fields = super::item_components::ComponentFields::apply(
            item.id.version(),
            prototype.iter().copied(),
            patch,
        )
        .map_err(|e| invalid(&e.to_string()))?;
        for name in ["minecraft:glider", "minecraft:equippable"] {
            let actual = fields
                .get(name)
                .ok_or_else(|| invalid("required glider component absent"))?;
            let original = prototype
                .iter()
                .find(|p| p.definition.name == name)
                .ok_or_else(|| invalid("original Elytra component unavailable"))?;
            if actual.bytes != original.bytes {
                return Err(invalid(
                    "custom glider/equippable component outside qualification",
                ));
            }
        }
        if properties.damageable && properties.damage >= properties.max_damage - 1 {
            return Err(invalid("received Elytra's next damage will break"));
        }
    }
    Ok(GliderEquipment {
        sequence,
        item: item.clone(),
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::registry::Registry;
    fn slot(version: MinecraftVersion, damage: i32) -> ObservedValue<SlotKnowledge> {
        let registry = Registry::for_version(version);
        let item = registry.item("minecraft:elytra").unwrap();
        let data = if version == MinecraftVersion::Java1_16_1 {
            let mut bytes = vec![10, 0, 0, 3, 0, 6];
            bytes.extend_from_slice(b"Damage");
            bytes.extend(damage.to_be_bytes());
            bytes.push(0);
            ItemData::LegacyNbt { bytes }
        } else {
            let definition = registry.item_component("minecraft:damage").unwrap();
            let mut bytes = Vec::new();
            crate::protocol::put_varint(&mut bytes, damage);
            ItemData::ModernComponents {
                patch: crate::client::ItemComponentPatch {
                    added: vec![crate::client::ItemComponent { definition, bytes }],
                    removed: vec![],
                },
            }
        };
        crate::client::received(
            SlotKnowledge::Item {
                item: ItemStack {
                    id: item.id,
                    name: item.name.clone(),
                    count: 1,
                    data,
                },
            },
            10,
        )
    }
    #[test]
    fn received_elytra_uses_the_versions_actual_durability_predicate() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            assert_eq!(
                equipment(Some(&slot(version, 0)))
                    .unwrap()
                    .properties
                    .damage,
                0
            );
            assert!(equipment(Some(&slot(version, 430))).is_ok());
            assert!(equipment(Some(&slot(version, 431))).is_err());
            assert!(equipment(Some(&slot(version, 432))).is_err());
        }
    }
    #[test]
    fn missing_nonreceived_empty_and_removed_glider_components_refuse() {
        assert!(equipment(None).is_err());
        assert!(equipment(Some(&crate::client::received(SlotKnowledge::Empty, 10))).is_err());
        let mut sample = slot(MinecraftVersion::Java1_21_11, 0);
        sample.source = ValueSource::Predicted;
        assert!(equipment(Some(&sample)).is_err());
        sample.source = ValueSource::Received { sequence: 10 };
        let SlotKnowledge::Item { item } = &mut sample.value else {
            unreachable!()
        };
        let ItemData::ModernComponents { patch } = &mut item.data else {
            unreachable!()
        };
        patch.removed.push(
            Registry::for_version(MinecraftVersion::Java1_21_11)
                .item_component("minecraft:glider")
                .unwrap(),
        );
        assert!(equipment(Some(&sample)).is_err());
    }
}
