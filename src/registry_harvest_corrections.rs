//! Version-pinned corrections to the mining tool gate, never loot guarantees.
//! Original upstream files remain intact; every edit requires its exact preimage.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Manifest {
    minecraft_version: String,
    correction_version: String,
    source_sha256: BTreeMap<String, String>,
    rows: Vec<Row>,
}
#[derive(Deserialize)]
struct Row {
    block: String,
    expected_tools: Option<Vec<String>>,
    corrected_gate_tools: Option<Vec<String>>,
}

fn apply(
    blocks: &str,
    items: &str,
    materials: &str,
    manifest: &str,
) -> Result<Vec<super::BlockData>, String> {
    let manifest: Manifest = serde_json::from_str(manifest).map_err(|e| e.to_string())?;
    if manifest.minecraft_version != "1.16.1"
        || manifest.correction_version != "java-1.16.1/harvest-gate-v1"
    {
        return Err("harvest correction version mismatch".into());
    }
    for (name, data) in [
        ("blocks.json", blocks),
        ("items.json", items),
        ("materials.json", materials),
    ] {
        let hash = format!("{:x}", Sha256::digest(data.as_bytes()));
        if manifest.source_sha256.get(name) != Some(&hash) {
            return Err(format!("harvest correction source mismatch: {name}"));
        }
    }
    let items: Vec<serde_json::Value> = serde_json::from_str(items).map_err(|e| e.to_string())?;
    let mut by_name = BTreeMap::new();
    let mut by_id = BTreeMap::new();
    for item in items {
        let name = item["name"].as_str().ok_or("missing item name")?.to_owned();
        let id = item["id"].as_i64().ok_or("missing item id")?.to_string();
        if by_name.insert(name.clone(), id.clone()).is_some() || by_id.insert(id, name).is_some() {
            return Err("duplicate item identity".into());
        }
    }
    let mut blocks: Vec<serde_json::Value> =
        serde_json::from_str(blocks).map_err(|e| e.to_string())?;
    let mut positions = BTreeMap::new();
    for (i, block) in blocks.iter().enumerate() {
        let name = block["name"]
            .as_str()
            .ok_or("missing block name")?
            .to_owned();
        if positions.insert(name, i).is_some() {
            return Err("duplicate block name".into());
        }
    }
    let mut seen = BTreeSet::new();
    for row in manifest.rows {
        if !seen.insert(row.block.clone()) {
            return Err("duplicate correction block".into());
        }
        let block = &mut blocks[*positions
            .get(&row.block)
            .ok_or("unknown correction block")?];
        let actual = match block.get("harvestTools") {
            None => None,
            Some(value) => {
                let mut names = Vec::new();
                for (id, allowed) in value.as_object().ok_or("invalid harvest tools")? {
                    if allowed.as_bool() != Some(true) {
                        return Err("unexpected harvest tool flag".into());
                    }
                    names.push(by_id.get(id).ok_or("unknown harvest tool id")?.clone());
                }
                names.sort();
                Some(names)
            }
        };
        if actual != row.expected_tools {
            return Err(format!(
                "harvest correction preimage mismatch: {}",
                row.block
            ));
        }
        let object = block.as_object_mut().ok_or("invalid block")?;
        match row.corrected_gate_tools {
            None => {
                object.remove("harvestTools");
            }
            Some(names) => {
                let mut tools = serde_json::Map::new();
                for name in names {
                    let id = by_name.get(&name).ok_or("unknown correction tool")?;
                    if tools.insert(id.clone(), true.into()).is_some() {
                        return Err("duplicate correction tool".into());
                    }
                }
                object.insert("harvestTools".into(), tools.into());
            }
        }
    }
    serde_json::from_value(blocks.into()).map_err(|e| e.to_string())
}

pub(super) fn blocks() -> Result<Vec<super::BlockData>, String> {
    apply(
        include_str!("../data/blocks.json"),
        include_str!("../data/items.json"),
        include_str!("../data/materials.json"),
        include_str!("../data/harvest_gate_corrections.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn changed(
        mut change: impl FnMut(&mut serde_json::Value),
    ) -> Result<Vec<crate::registry::BlockData>, String> {
        let mut m: serde_json::Value =
            serde_json::from_str(include_str!("../data/harvest_gate_corrections.json")).unwrap();
        change(&mut m);
        apply(
            include_str!("../data/blocks.json"),
            include_str!("../data/items.json"),
            include_str!("../data/materials.json"),
            &m.to_string(),
        )
    }
    #[test]
    fn source_preimage_and_named_identity_are_required() {
        assert!(blocks().is_ok());
        assert!(changed(|m| m["source_sha256"]["items.json"] = "wrong".into()).is_err());
        assert!(changed(|m| m["rows"][0]["expected_tools"] = serde_json::json!([])).is_err());
        assert!(
            changed(|m| m["rows"][0]["corrected_gate_tools"] = serde_json::json!(["unknown_tool"]))
                .is_err()
        );
        assert!(
            changed(|m| {
                let row = m["rows"][0].clone();
                m["rows"].as_array_mut().unwrap().push(row);
            })
            .is_err()
        );
        assert!(changed(|m| m["minecraft_version"] = "1.16.2".into()).is_err());
    }
    #[test]
    fn corrected_gate_preserves_source_and_separates_loot() {
        let corrected = blocks().unwrap();
        let tools = |name: &str| {
            corrected
                .iter()
                .find(|b| b.name == name)
                .unwrap()
                .harvest_tools
                .as_ref()
        };
        assert!(tools("stone").unwrap().contains_key("608"));
        assert!(!tools("stone").unwrap().contains_key("609"));
        assert!(tools("diamond_ore").unwrap().contains_key("608"));
        assert!(tools("snow").unwrap().contains_key("607"));
        assert!(tools("peony").is_none());
        assert!(tools("dead_tube_coral").is_some());
        assert!(tools("command_block").unwrap().contains_key("608"));
        assert!(
            !corrected
                .iter()
                .find(|b| b.name == "command_block")
                .unwrap()
                .diggable
        );
    }
}
