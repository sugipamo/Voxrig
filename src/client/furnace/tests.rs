use super::*;
use crate::client::{
    ItemData, ItemStack,
    container::{ScreenReceipts, ScreenTitle},
    inventory::{InventoryClickButton, slot_policy},
    received,
};

fn versions() -> [MinecraftVersion; 2] {
    [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11]
}
fn stack(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
    let item = Registry::for_version(version).item(name).unwrap();
    SlotKnowledge::Item {
        item: ItemStack {
            id: item.id,
            name: item.name,
            count,
            data: ItemData::Default,
        },
    }
}
#[test]
fn original_furnace_roles_preserve_real_screen_mapping_and_missing_receipts() {
    for version in versions() {
        let session = SessionStamp {
            version,
            connection_id: 7,
            world_generation: 3,
        };
        for menu in &facts(version).menus {
            assert_eq!(menu.total_slots, 39);
            let native = ScreenReceipts::open(
                version,
                4,
                Some(menu.native_id),
                ScreenTitle::Unavailable,
                10,
            );
            let state = ScreenObservation {
                session,
                receive_sequence: 12,
                active_window: Some(4),
                player_screen: None,
                screen: Some(native.capture(session)),
                cursor: Some(received(SlotKnowledge::Empty, 11)),
            };
            let observed = FurnaceObservation::capture(state.clone()).unwrap().unwrap();
            assert_eq!(
                observed.screen().layout.as_ref().unwrap().player_slots,
                menu.player_mappings()
            );
            for (role, index) in [
                (FurnaceSlot::Input, 0),
                (FurnaceSlot::Fuel, 1),
                (FurnaceSlot::Output, 2),
            ] {
                assert!(observed.slot(role).is_none());
                assert!(
                    matches!(observed.slot_source(role), (InventoryClickSource::Container{screen}, slot) if screen==observed.screen().id && slot==index)
                );
            }
            let mut changed = state.clone();
            changed.screen.as_mut().unwrap().native_menu_id = Some(-1);
            assert!(FurnaceObservation::capture(changed).is_err());
            let mut changed = state.clone();
            changed
                .screen
                .as_mut()
                .unwrap()
                .layout
                .as_mut()
                .unwrap()
                .player_slots[0]
                .player_slot += 1;
            assert!(FurnaceObservation::capture(changed).is_err());
            let mut closed = state;
            closed.player_screen = Some(super::super::container::PlayerScreenAccess::Received);
            assert!(FurnaceObservation::capture(closed).unwrap().is_none());
        }
    }
}
#[test]
fn native_fuel_capacity_and_output_refusal_apply_to_common_pickup() {
    use InventoryClickButton::Left;
    for version in versions() {
        for menu in &facts(version).menus {
            let coal = stack(version, "minecraft:coal", 3);
            let stone = stack(version, "minecraft:stone", 3);
            let bucket = stack(version, "minecraft:bucket", 3);
            let empty = SlotKnowledge::Empty;
            let fuel = menu.role(FurnaceSlot::Fuel).unwrap();
            assert_eq!(
                slot_policy::pickup(version, &menu.name, fuel, Left, &empty, &coal).unwrap(),
                (coal.clone(), empty.clone())
            );
            assert_eq!(
                slot_policy::pickup(version, &menu.name, fuel, Left, &empty, &stone).unwrap(),
                (empty.clone(), stone.clone())
            );
            assert_eq!(
                slot_policy::pickup(version, &menu.name, fuel, Left, &empty, &bucket).unwrap(),
                (
                    stack(version, "minecraft:bucket", 1),
                    stack(version, "minecraft:bucket", 2)
                )
            );
            let output = menu.role(FurnaceSlot::Output).unwrap();
            let ingot = stack(version, "minecraft:iron_ingot", 1);
            assert_eq!(
                slot_policy::pickup(version, &menu.name, output, Left, &ingot, &empty).unwrap(),
                (empty.clone(), ingot.clone())
            );
            assert_eq!(
                slot_policy::pickup(version, &menu.name, output, Left, &empty, &ingot).unwrap(),
                (empty.clone(), ingot.clone())
            );
            // Ordinary output merge extracts the output; it never deposits the cursor.
            assert_eq!(
                slot_policy::pickup(version, &menu.name, output, Left, &ingot, &ingot).unwrap(),
                (empty, stack(version, "minecraft:iron_ingot", 2))
            );
        }
    }
}
#[test]
fn fuel_requires_received_membership_for_original_native_dependencies() {
    for version in versions() {
        let expected = &facts(version).fuel_dependency_tags;
        assert!(!expected.is_empty());
        let item = "minecraft:oak_log";
        let id = Registry::for_version(version)
            .item(item)
            .unwrap()
            .id
            .value();
        let mut tags = ServerRegistryTags::from([(
            "minecraft:item".into(),
            expected
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.iter()
                            .map(|n| Registry::for_version(version).item(n).unwrap().id.value())
                            .collect(),
                    )
                })
                .collect(),
        )]);
        validate_memberships(version, item, id, Some(&tags)).unwrap();
        assert!(validate_memberships(version, item, id, None).is_err());
        let key = expected
            .iter()
            .find(|(_, v)| v.iter().any(|n| n == item))
            .unwrap()
            .0;
        tags.get_mut("minecraft:item")
            .unwrap()
            .get_mut(key)
            .unwrap()
            .retain(|v| *v != id);
        assert!(validate_memberships(version, item, id, Some(&tags)).is_err());
        assert!(validate_memberships(version, item, id + 1, Some(&tags)).is_err());
    }
}
#[test]
fn original_furnace_outlines_cover_both_lit_states_and_native_faces() {
    for version in versions() {
        let bytes: &[u8] = match version {
            MinecraftVersion::Java1_16_1 => {
                include_bytes!("../../../data/client_api/furnace_menus-1.16.1.json.gz")
            }
            MinecraftVersion::Java1_21_11 => {
                include_bytes!("../../../data/client_api/furnace_menus-1.21.11.json.gz")
            }
        };
        let native: serde_json::Value =
            serde_json::from_reader(flate2::read::GzDecoder::new(bytes)).unwrap();
        assert_eq!(facts(version).states.len(), 24);
        for row in native["states"].as_array().unwrap() {
            let state: NativeBlockState = serde_json::from_value(row["state"].clone()).unwrap();
            let id = match version {
                MinecraftVersion::Java1_16_1 => {
                    crate::versions::java_1_16_1::state_id(&state).unwrap()
                }
                MinecraftVersion::Java1_21_11 => {
                    crate::versions::java_1_21_11::state_id(&state).unwrap()
                }
            };
            assert_eq!(i64::from(id), row["native_id"].as_i64().unwrap());
            let mut incomplete = state.clone();
            incomplete.properties.remove("lit");
            assert!(outlines(version, &incomplete).is_none());
        }
        assert_eq!(native["rays"].as_array().unwrap().len(), 144);
        for ray in native["rays"].as_array().unwrap() {
            let state = &facts(version).states[ray["state_index"].as_u64().unwrap() as usize].state;
            let start = serde_json::from_value(ray["start"].clone()).unwrap();
            let end = serde_json::from_value(ray["end"].clone()).unwrap();
            let cell: [i32; 3] = serde_json::from_value(ray["cell"].clone()).unwrap();
            let air = NativeBlockState {
                name: "minecraft:air".into(),
                properties: Default::default(),
            };
            let got = super::super::survival::target::cast(
                start,
                end,
                |p| {
                    Ok(if p == cell {
                        state.clone()
                    } else {
                        air.clone()
                    })
                },
                |s| {
                    Ok(if s == &air {
                        (&[][..], &[][..])
                    } else {
                        outlines(version, s).unwrap()
                    })
                },
            )
            .unwrap()
            .unwrap();
            assert_eq!(got.position, cell);
            assert_eq!(
                format!("{:?}", got.face).to_lowercase(),
                ray["hit"]["face"].as_str().unwrap()
            );
            let expected: [f64; 3] = serde_json::from_value(ray["hit"]["point"].clone()).unwrap();
            for (a, b) in got.point.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-9);
            }
        }
    }
}
