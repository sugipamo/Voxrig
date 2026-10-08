use super::*;
use crate::client::crafting::test_receipts as receipts;
use crate::client::registry::Registry;

fn corpus(version: MinecraftVersion) -> serde_json::Value {
    receipts::corpus(match version {
        MinecraftVersion::Java1_16_1 => {
            include_bytes!("../../../../data/client_api/recipe_grid_matching_cases-1.16.1.json.gz")
        }
        MinecraftVersion::Java1_21_11 => {
            include_bytes!("../../../../data/client_api/recipe_grid_matching_cases-1.21.11.json.gz")
        }
    })
}
#[test]
fn actual_grid_matches_entire_original_shaped_and_shapeless_matchers() {
    let mut comparisons = 0;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let corpus = corpus(version);
        assert_eq!(corpus["cases"].as_array().unwrap().len(), 51);
        let registry = Registry::for_version(version);
        for row in corpus["cases"].as_array().unwrap() {
            let cells = row["ingredients"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    v.as_array().map(|ids| {
                        let mut ids = ids
                            .iter()
                            .map(|v| registry.item(v.as_str().unwrap()).unwrap().id.value())
                            .collect::<Vec<_>>();
                        ids.sort_unstable();
                        ids.dedup();
                        ids
                    })
                })
                .collect::<Vec<_>>();
            let pattern = Pattern {
                shaped: row["shaped"].as_bool().unwrap(),
                dimensions: [
                    row["recipe_width"].as_u64().unwrap() as usize,
                    row["recipe_height"].as_u64().unwrap() as usize,
                ],
                ingredients: cells.iter().flatten().cloned().collect(),
                cells,
            };
            let grid = row["inputs_encoded"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| match receipts::value(version, v.as_str().unwrap()) {
                    SlotKnowledge::Empty => None,
                    SlotKnowledge::Item { item } => Some(item.id.value()),
                    _ => panic!("unavailable original stack"),
                })
                .collect::<Vec<_>>();
            let dimensions = [
                row["grid_width"].as_u64().unwrap() as usize,
                row["grid_height"].as_u64().unwrap() as usize,
            ];
            assert_eq!(
                matches(&pattern, dimensions, &grid, version),
                row["matches"].as_bool().unwrap(),
                "{version:?} {}",
                row["case"]
            );
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 102);
}
#[test]
fn original_grid_match_evidence_binds_sources_requests_and_outputs() {
    use sha2::{Digest, Sha256};
    let source: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../data/client_api/recipe_grid_matching_source.json"
    ))
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for object in ["generators_sha256", "requests_sha256"]
        .into_iter()
        .map(|name| &source[name])
        .chain(
            source["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| &r["files_sha256"]),
        )
    {
        for (path, sha) in object.as_object().unwrap() {
            assert_eq!(
                hex::encode(Sha256::digest(std::fs::read(root.join(path)).unwrap())),
                sha.as_str().unwrap(),
                "{path}"
            );
        }
    }
}
