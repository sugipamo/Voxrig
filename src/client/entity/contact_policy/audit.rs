//! Test-only export for the Python evidence auditor; never a public/runtime API.
use super::{key::*, table};
use crate::MinecraftVersion;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
    println!("CONTACT_POLICY={}", policy_export());
}

// Export resolved table values, including lane boundaries. No Rust source parser
// or second policy declaration is involved. serde_json maps use sorted keys.
fn semantics(value: table::Semantics) -> Value {
    json!({
        "tick": format!("{:?}", value.tick),
        "geometry": format!("{:?}", value.geometry),
        "metadata": format!("{:?}", value.metadata),
        "team": format!("{:?}", value.team),
        "identity": format!("{:?}", value.identity),
    })
}
fn policy_export() -> Value {
    let mut versions = serde_json::Map::new();
    let classifications = capability_export();
    let mut present_entities = 0;
    for (version, name) in VERSIONS.into_iter().zip(["1.16.1", "1.21.11"]) {
        let mut types = serde_json::Map::new();
        for &entity in EntityType::ALL {
            let Some(native_id) = entity.native_id(version) else {
                continue;
            };
            present_entities += 1;
            let mut lanes = serde_json::Map::new();
            for &lane in OwnerLane::ALL {
                let profile = table::lookup(version, entity, lane);
                let effects: serde_json::Map<String, Value> = Effect::ALL
                    .iter()
                    .map(|&effect| {
                        (
                            format!("{effect:?}"),
                            json!(format!("{:?}", profile.effects.get(effect))),
                        )
                    })
                    .collect();
                lanes.insert(
                    format!("{lane:?}"),
                    json!({
                        "profile": format!("{:?}", profile.id),
                        "effects": effects,
                        "modeled": profile.effects.modeled(),
                        "selectors": semantics(table::semantics(version, entity, lane)),
                        "packet_profile": format!("{:?}", profile.packets),
                        "locality": format!("{:?}", profile.locality),
                        "evidence": format!("{:?}", profile.evidence),
                        "scenarios": format!("{:?}", profile.scenarios),
                    }),
                );
            }
            let short = entity.name().strip_prefix("minecraft:").unwrap();
            types.insert(short.into(), json!({
                "native_id": native_id,
                "classifications": classifications[name][short],
                "lanes": lanes,
                "vehicle_selector": format!("{:?}", table::ENTITY_POLICIES[entity.ordinal()].vehicle),
            }));
        }
        versions.insert(name.into(), json!({"types": types}));
    }
    let packets: serde_json::Map<String, Value> = table::PACKET_POLICIES.iter().map(|policy| {
        (format!("{:?}", policy.id), json!({
            "default": format!("{:?}", policy.default),
            "exact": policy.exact.iter().map(|rule| json!({
                "packet": format!("{:?}", rule.packet),
                "effects": rule.effects.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
                "decision": format!("{:?}", rule.decision),
            })).collect::<Vec<_>>(),
        }))
    }).collect();
    let locality: serde_json::Map<String, Value> = table::LOCALITY_PROFILES
        .iter()
        .map(|profile| {
            (
                format!("{:?}", profile.id),
                json!({
                    "handler": format!("{:?}", profile.handler),
                    "lanes": profile.lanes.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
                    "effects": profile.effects.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
                    "requires_pose_and_dimensions": profile.requires_pose_and_dimensions,
                    "requires_motion_receipts": profile.requires_motion_receipts,
                    "requires_lifetime": profile.requires_lifetime,
                    "requires_peer_and_query_proof": profile.requires_peer_and_query_proof,
                    "revokes": profile.revokes.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
                    "evidence": format!("{:?}", profile.evidence),
                }),
            )
        })
        .collect();
    let mut output = json!({
        "schema_version": 1,
        "versions": versions,
        "packet_profiles": packets,
        "locality_profiles": locality,
        "evidence": table::EVIDENCE_REQUIREMENTS.iter().map(|e| json!({
            "id": format!("{:?}", e.id), "artifact": e.artifact,
        })).collect::<Vec<_>>(),
        "scenarios": table::SCENARIO_PROFILES.iter().map(|s| json!({
            "id": format!("{:?}", s.id), "evidence": format!("{:?}", s.evidence),
        })).collect::<Vec<_>>(),
        "expanded_present_keys": {
            "versioned_entities": present_entities,
            "lanes": OwnerLane::ALL.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
            "effects": Effect::ALL.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
            "packets": PacketKey::ALL.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
            "count": present_entities * OwnerLane::ALL.len() * Effect::ALL.len() * PacketKey::ALL.len(),
        },
    });
    // SHA-256 of compact, recursively key-sorted JSON before adding this field.
    output["policy_digest"] = json!(format!(
        "{:x}",
        Sha256::digest(output.to_string().as_bytes())
    ));
    output
}
