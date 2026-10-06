use super::*;
use crate::{
    MinecraftVersion,
    client::{
        ItemData,
        registry::{ServerRegistryEntry, received::ReceivedRegistries},
    },
};
fn session(version: MinecraftVersion) -> SessionStamp {
    SessionStamp {
        version,
        connection_id: 7,
        world_generation: 2,
    }
}
fn capture(receipts: &RecipeReceipts, version: MinecraftVersion, sequence: u64) -> ReceivedRecipes {
    receipts
        .capture(
            session(version),
            sequence,
            ReceivedRegistries::default().capture(session(version), sequence),
        )
        .unwrap()
}
fn entry(version: MinecraftVersion) -> RecipeEntry {
    let result = RecipeSlotDisplay::Item {
        item: Registry::for_version(version)
            .item("minecraft:stick")
            .unwrap(),
    };
    RecipeEntry {
        display: RecipeDisplay::Shapeless {
            ingredients: vec![],
            result,
            crafting_station: None,
        },
        requirements: None,
        group: None,
        category: None,
        highlighted: None,
        notification: None,
    }
}
#[test]
fn recipe_catalogue_legacy_declaration_book_availability_and_owner_are_distinct() {
    let version = MinecraftVersion::Java1_16_1;
    let mut receipts = RecipeReceipts::default();
    assert!(!capture(&receipts, version, 10).book_initialized());
    receipts.declare_legacy(
        [(
            NativeRecipeId::Legacy("minecraft:stick".into()),
            entry(version),
        )]
        .into(),
        11,
    );
    let declared = capture(&receipts, version, 12);
    let id = declared.entries()[0].id().clone();
    assert_eq!(declared.entries()[0].unlocked(), None);
    receipts.legacy_book(
        ["minecraft:stick".into()].into(),
        BTreeSet::new(),
        false,
        13,
    );
    let added = capture(&receipts, version, 13);
    assert!(!added.book_initialized());
    assert_eq!(added.entry(&id).unwrap().unlocked(), Some(true));
    assert_eq!(added.entry(&id).unwrap().highlighted(), None);
    receipts.legacy_book(BTreeSet::new(), BTreeSet::new(), true, 14);
    let removed = capture(&receipts, version, 14);
    assert_eq!(removed.entry(&id).unwrap().unlocked(), Some(false));
    assert_eq!(removed.entry(&id).unwrap().highlighted(), Some(false));
    receipts.legacy_book(
        ["minecraft:stick".into()].into(),
        BTreeSet::new(),
        false,
        15,
    );
    let readded = capture(&receipts, version, 15);
    assert_eq!(readded.entry(&id).unwrap().unlocked(), Some(true));
    assert_eq!(readded.entry(&id).unwrap().highlighted(), None);
    let mut respawn = session(version);
    respawn.world_generation += 1;
    let capture = receipts
        .capture(
            respawn,
            15,
            ReceivedRegistries::default().capture(respawn, 15),
        )
        .unwrap();
    assert!(capture.entry(&id).is_ok());
    let mut other = session(version);
    other.connection_id += 1;
    let foreign = receipts
        .capture(other, 15, ReceivedRegistries::default().capture(other, 15))
        .unwrap();
    assert!(foreign.entry(&id).is_err());
    assert!(declared.entries()[0].output_items(&foreign).is_err());
    receipts.declare_legacy(
        [(
            NativeRecipeId::Legacy("minecraft:stick".into()),
            entry(version),
        )]
        .into(),
        16,
    );
    assert!(
        super::tests::capture(&receipts, version, 16)
            .entry(&id)
            .is_err()
    );
    assert!(declared.entry(&id).is_ok());
}
#[test]
fn recipe_catalogue_tags_absent_received_empty_and_version_ids_are_not_defaults() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let mut receipts = RecipeReceipts::default();
        let mut e = entry(version);
        e.display = RecipeDisplay::Shapeless {
            ingredients: vec![],
            result: RecipeSlotDisplay::Tag {
                name: "minecraft:planks".into(),
            },
            crafting_station: None,
        };
        receipts.add_modern(vec![(1, e)], true, 11);
        let absent = capture(&receipts, version, 12);
        assert!(absent.entries()[0].output_items(&absent).is_err());
        let mut received = ReceivedRegistries::default();
        // Actual empty native tag packet, with no invented vanilla defaults.
        let packet = if version == MinecraftVersion::Java1_16_1 {
            vec![0, 0, 0, 0]
        } else {
            vec![0]
        };
        received.receive_tags(&packet, 12, version).unwrap();
        let empty = receipts
            .capture(session(version), 12, received.capture(session(version), 12))
            .unwrap();
        assert!(empty.entries()[0].output_items(&empty).unwrap().is_empty());
        let wrong = if version == MinecraftVersion::Java1_16_1 {
            MinecraftVersion::Java1_21_11
        } else {
            MinecraftVersion::Java1_16_1
        };
        assert!(
            RecipeIngredient::Items {
                items: vec![
                    Registry::for_version(wrong)
                        .item("minecraft:stone")
                        .unwrap()
                        .id
                ]
            }
            .items(&empty)
            .is_err()
        );
    }
}
#[test]
fn recipe_catalogue_dynamic_trim_references_bind_actual_received_entries() {
    let version = MinecraftVersion::Java1_21_11;
    let mut receipts = RecipeReceipts::default();
    let mut e = entry(version);
    e.display = RecipeDisplay::Shapeless {
        ingredients: vec![],
        result: RecipeSlotDisplay::TrimPreview {
            base: Box::new(RecipeSlotDisplay::Empty),
            material: Box::new(RecipeSlotDisplay::Empty),
            pattern: RecipeTrimPattern::raw_reference(0),
        },
        crafting_station: None,
    };
    receipts.add_modern(vec![(3, e)], true, 12);
    assert!(
        receipts
            .capture(
                session(version),
                13,
                ReceivedRegistries::default().capture(session(version), 13)
            )
            .is_err()
    );
    let mut received = ReceivedRegistries::default();
    received
        .modern_registry(
            "minecraft:trim_pattern".into(),
            vec![ServerRegistryEntry {
                name: "custom:actual".into(),
                data: vec![10, 0],
            }],
            10,
            2,
        )
        .unwrap();
    received.finish();
    let catalogue = receipts
        .capture(session(version), 13, received.capture(session(version), 13))
        .unwrap();
    let RecipeSlotDisplay::TrimPreview { pattern, .. } =
        catalogue.entries()[0].display().result().unwrap()
    else {
        panic!("trim");
    };
    let bound = pattern.reference().unwrap();
    assert_eq!(bound.value(), 0);
    assert_eq!(
        catalogue.registry_state().resolve(bound).unwrap().name,
        "custom:actual"
    );
    let mut reset = ReceivedRegistries::default();
    reset.reset(14);
    assert!(
        receipts
            .capture(session(version), 15, reset.capture(session(version), 15))
            .is_err()
    );
}
#[test]
fn recipe_catalogue_legacy_import_preserves_topology_serializer_and_native_display_stack() {
    use crate::versions::java_1_16_1::{
        inventory::ItemStack as Native,
        server_registry::{ServerRecipe, ServerRecipeData, ServerRecipes},
    };
    let version = MinecraftVersion::Java1_16_1;
    let reg = Registry::for_version(version);
    let planks = Native {
        item_id: reg.item("minecraft:oak_planks").unwrap().id.value(),
        count: 1,
        nbt: None,
    };
    let result = Native {
        item_id: reg.item("minecraft:stick").unwrap().id.value(),
        count: 4,
        nbt: None,
    };
    let native = ServerRecipes {
        recipes: [
            (
                "minecraft:stick".into(),
                ServerRecipe {
                    id: "minecraft:stick".into(),
                    kind: "minecraft:crafting_shaped".into(),
                    data: ServerRecipeData::Shaped {
                        width: 1,
                        height: 2,
                        group: "sticks".into(),
                        ingredients: vec![vec![Some(planks.clone())], vec![Some(planks)]],
                        result,
                    },
                },
            ),
            (
                "custom:special".into(),
                ServerRecipe {
                    id: "custom:special".into(),
                    kind: "minecraft:crafting_special_armordye".into(),
                    data: ServerRecipeData::Special,
                },
            ),
        ]
        .into(),
    };
    let mut receipts = RecipeReceipts::default();
    receipts.declare_legacy(legacy_entries(&native).unwrap(), 11);
    let catalogue = capture(&receipts, version, 12);
    let stick = catalogue
        .entries()
        .iter()
        .find(|r| r.group() == Some("sticks"))
        .unwrap();
    assert_eq!(stick.requirements().unwrap().len(), 2);
    let RecipeDisplay::Shaped {
        width,
        height,
        ingredients,
        result,
        crafting_station,
    } = stick.display()
    else {
        panic!("shaped");
    };
    assert_eq!((*width, *height), (1, 2));
    assert_eq!(ingredients.len(), 2);
    assert!(crafting_station.is_none());
    let RecipeSlotDisplay::Stack { item } = result else {
        panic!("stack");
    };
    assert_eq!(item.name, "minecraft:stick");
    assert_eq!(item.count, 4);
    assert!(matches!(
        item.data,
        ItemData::Default | ItemData::LegacyNbt { .. }
    ));
    let special = catalogue
        .entries()
        .iter()
        .find(|r| matches!(r.display(), RecipeDisplay::Special { .. }))
        .unwrap();
    assert!(special.output_items(&catalogue).is_err());
}
