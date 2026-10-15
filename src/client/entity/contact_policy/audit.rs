//! Test-only export for the Python evidence auditor; never a public/runtime API.
use super::{key::*, table};
use crate::MinecraftVersion;
use serde_json::{Value, json};

fn capability_export() -> Value {
    let mut versions = serde_json::Map::new();
    for version in VERSIONS {
        let name = match version {
            MinecraftVersion::Java1_16_1 => "1.16.1",
            MinecraftVersion::Java1_21_11 => "1.21.11",
        };
        let mut types = serde_json::Map::new();
        for &entity in EntityType::ALL {
            if entity.native_id(version).is_none() {
                continue;
            }
            let effects = table::lookup(version, entity, OwnerLane::OwnedWalking).effects;
            types.insert(
                entity.name().strip_prefix("minecraft:").unwrap().into(),
                json!({
                    "walking": format!("{:?}", effects.walking),
                    "incoming": format!("{:?}", effects.incoming),
                    "solid": format!("{:?}", effects.solid),
                    "geometry": format!("{:?}", effects.geometry),
                    "metadata": format!("{:?}", effects.metadata),
                    "status": format!("{:?}", effects.status),
                    "animation": format!("{:?}", effects.animation),
                }),
            );
        }
        versions.insert(name.into(), Value::Object(types));
    }
    Value::Object(versions)
}

#[test]
fn capability_audit_matches_const_table() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../docs/evidence/contact-capabilities-20261015.json"
    ))
    .unwrap();
    let observed: serde_json::Map<String, Value> = evidence["versions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(version, evidence)| {
            let types: serde_json::Map<String, Value> = evidence["types"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, row)| (name.clone(), row["classifications"].clone()))
                .collect();
            (version.clone(), Value::Object(types))
        })
        .collect();
    // Object equality checks both directions, including all Unknown entries.
    assert_eq!(capability_export(), Value::Object(observed));
}

#[test]
fn export_contact_capability_policy() {
    println!("CONTACT_CAPABILITY_POLICY={}", capability_export());
}
