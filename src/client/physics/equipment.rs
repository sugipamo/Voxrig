//! Received legacy boots into client-model inputs, without modifying receipts.
use super::Environment;
use crate::client::registry::{Registry, received::ReceivedRegistries};
use crate::client::{ItemStack, ObservedValue, SlotKnowledge, ValueSource};

fn level(item: &ItemStack, name: &str) -> crate::Result<u8> {
    let tag = crate::client::item::legacy_constructor_tag(item)?;
    let entries = tag
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
        let Ok(id) = id.text() else { continue };
        let Ok((namespace, path)) = crate::client::identifier::parts(&id) else {
            continue;
        };
        if namespace == "minecraft" && path == name {
            // Original EnchantmentHelper: first matching key, getInt, clamp 0..255.
            // StoredEnchantments do not apply to equipment.
            return Ok(entry
                .get("lvl")
                .map_or(0, crate::client::item::legacy_int)
                .clamp(0, 255) as u8);
        }
    }
    Ok(0)
}

pub(crate) fn legacy_boots(
    boots: Option<&ObservedValue<SlotKnowledge>>,
    registries: &ReceivedRegistries,
    env: &mut Environment,
) {
    let apply = || -> crate::Result<(u8, Option<Vec<String>>)> {
        let Some(boots) = boots else {
            return Ok((0, None));
        };
        if !matches!(boots.source, ValueSource::Received { .. }) {
            return Err(crate::client::inventory::unavailable(
                "movement equipment is not received",
            ));
        }
        let item = match &boots.value {
            SlotKnowledge::Empty => return Ok((0, None)),
            SlotKnowledge::Item { item } => item,
            SlotKnowledge::Unavailable => {
                return Err(crate::client::inventory::unavailable(
                    "movement equipment unavailable",
                ));
            }
        };
        let depth = level(item, "depth_strider")?;
        if level(item, "soul_speed")? == 0 {
            return Ok((depth, None));
        }
        let ids = registries
            .tag_members("minecraft:block", "minecraft:soul_speed_blocks")
            .ok_or_else(|| {
                crate::client::inventory::unavailable("received Soul Speed block tag unavailable")
            })?;
        if ids.len() > 4096 {
            return Err(crate::client::inventory::unavailable(
                "movement Soul Speed tag exceeds model budget",
            ));
        }
        let registry = Registry::for_version(crate::MinecraftVersion::Java1_16_1);
        let names = ids
            .iter()
            .map(|&id| {
                let id = registry.builtin_id_by_native_id("minecraft:block", id)?;
                Ok(registry.builtin_name(&id)?.to_owned())
            })
            .collect::<crate::Result<std::collections::BTreeSet<_>>>()?
            .into_iter()
            .collect();
        Ok((depth, Some(names)))
    };
    match apply() {
        Ok((depth, blocks)) => {
            env.depth_strider = depth;
            env.legacy_soul_speed_blocks = blocks;
        }
        Err(_) => env.equipment_unavailable = true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ItemData, received};
    fn boots(tag_name: &str, entries: &[(&str, i32)]) -> ItemStack {
        fn text(bytes: &mut Vec<u8>, v: &str) {
            bytes.extend((v.len() as u16).to_be_bytes());
            bytes.extend(v.as_bytes());
        }
        let mut bytes = vec![10, 0, 0, 9];
        text(&mut bytes, tag_name);
        bytes.push(10);
        bytes.extend((entries.len() as i32).to_be_bytes());
        for (id, lvl) in entries {
            bytes.push(8);
            text(&mut bytes, "id");
            text(&mut bytes, id);
            bytes.push(3);
            text(&mut bytes, "lvl");
            bytes.extend(lvl.to_be_bytes());
            bytes.push(0);
        }
        bytes.push(0);
        let definition = Registry::for_version(crate::MinecraftVersion::Java1_16_1)
            .item("minecraft:diamond_boots")
            .unwrap();
        ItemStack {
            id: definition.id,
            name: definition.name.to_owned(),
            count: 1,
            data: ItemData::LegacyNbt { bytes },
        }
    }
    #[test]
    fn native_first_match_and_numeric_clamp_do_not_use_stored_enchantments() {
        for (name, entries, want) in [
            ("Enchantments", vec![("minecraft:depth_strider", 3)], 3),
            ("Enchantments", vec![("depth_strider", 1)], 1),
            ("Enchantments", vec![(":depth_strider", 300)], 255),
            (
                "Enchantments",
                vec![
                    ("minecraft:depth_strider", -1),
                    ("minecraft:depth_strider", 3),
                ],
                0,
            ),
            (
                "StoredEnchantments",
                vec![("minecraft:depth_strider", 3)],
                0,
            ),
            ("Enchantments", vec![("custom:depth_strider", 3)], 0),
        ] {
            assert_eq!(
                level(&boots(name, &entries), "depth_strider").unwrap(),
                want
            );
        }
    }
    #[test]
    fn soul_speed_requires_actual_tag_and_unavailable_inputs_never_become_empty() {
        let version = crate::MinecraftVersion::Java1_16_1;
        let registry = Registry::for_version(version);
        let mut registries = ReceivedRegistries::default();
        let item = received(
            SlotKnowledge::Item {
                item: boots(
                    "Enchantments",
                    &[("minecraft:depth_strider", 3), ("minecraft:soul_speed", 1)],
                ),
            },
            2,
        );
        let mut env = Environment::defaults(version);
        legacy_boots(Some(&item), &registries, &mut env);
        assert!(env.equipment_unavailable);
        let mut body = super::super::Body::new([0.5, 1.0, 0.5]);
        let before = serde_json::to_value(&body).unwrap();
        let mut looked_up = false;
        assert!(
            super::super::tick(version, &mut body, &env, Default::default(), &mut |_| {
                looked_up = true;
                unreachable!("missing equipment must refuse before terrain lookup")
            })
            .is_err()
        );
        assert!(!looked_up);
        assert_eq!(serde_json::to_value(&body).unwrap(), before);
        let id = registry
            .builtin_id("minecraft:block", "minecraft:stone")
            .unwrap()
            .value();
        let mut tag = vec![1];
        crate::protocol::put_string(&mut tag, "minecraft:soul_speed_blocks");
        tag.push(1);
        crate::protocol::put_varint(&mut tag, id);
        tag.extend([0, 0, 0]);
        registries.receive_tags(&tag, 3, version).unwrap();
        let mut env = Environment::defaults(version);
        legacy_boots(Some(&item), &registries, &mut env);
        assert_eq!(env.depth_strider, 3);
        assert_eq!(
            env.legacy_soul_speed_blocks,
            Some(vec!["minecraft:stone".into()])
        );
        assert!(!env.equipment_unavailable);
        let bad = received(SlotKnowledge::Unavailable, 4);
        let mut env = Environment::defaults(version);
        legacy_boots(Some(&bad), &registries, &mut env);
        assert!(env.equipment_unavailable);
    }
}
