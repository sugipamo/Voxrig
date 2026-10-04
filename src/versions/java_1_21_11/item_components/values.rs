//! Typed stream fields for all component codec compositions.
//! These retain structure and normalize native enum IDs. They are not native
//! component/text equality, persistent encoding, resolved registry bindings or
//! inventory authority. Decode without capture keeps the receive path allocation bounded.
use crate::client::nbt::NbtValue;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(super) enum Value {
    Unit,
    Boolean(bool),
    Integer(i32),
    Float {
        bits: u32,
    },
    Double {
        bits: u64,
    },
    BlockPosition {
        packed: i64,
    },
    Uuid {
        most: i64,
        least: i64,
    },
    String(String),
    Identifier {
        namespace: String,
        path: String,
    },
    // Preserve constructor/codec identity until its native normalization is
    // implemented. Transparent fields alone lose which semantic type they form.
    Forward {
        codec: usize,
        value: Box<Value>,
    },
    Enumeration {
        codec: usize,
        native_id: i32,
    },
    Nbt(Option<Arc<NbtValue>>),
    Sequence(Vec<Value>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Optional(Option<Box<Value>>),
    Either {
        left: bool,
        value: Box<Value>,
    },
    Registry {
        registry: String,
        native_id: i32,
    },
    HolderReference {
        registry: String,
        native_id: i32,
    },
    HolderInline {
        registry: String,
        value: Box<Value>,
    },
    HolderTag {
        registry: String,
        tag: String,
    },
    HolderList {
        registry: String,
        values: Vec<Value>,
    },
    ProfileProperties(Vec<ProfileProperty>),
    TypedComponent {
        native_id: i32,
        value: Box<Value>,
    },
    Item {
        count: i32,
        native_id: Option<i32>,
        patch: Option<Box<Value>>,
    },
    Patch {
        added: Vec<(i32, Value)>,
        removed: Vec<i32>,
    },
    Dispatch {
        left: Option<bool>,
        tag: i32,
        value: Box<Value>,
    },
}
#[derive(Debug, Serialize)]
pub(super) struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}
#[derive(Deserialize)]
struct Rules {
    enums: Vec<EnumRule>,
    scalars: Vec<ScalarRule>,
}
#[derive(Deserialize)]
struct NormalizationRules {
    identifiers: Vec<IdentifierRule>,
}
#[derive(Deserialize)]
struct IdentifierRule {
    node: usize,
    child: usize,
    codec_class: String,
    value_class: String,
}
pub(super) fn identifier_forward(node: usize, child: usize, class: &str) -> Result<bool> {
    static RULES: OnceLock<NormalizationRules> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../data/client_api/component_normalization_rules-1.21.11.json"
        ))
        .expect("pinned original Identifier forward facts")
    });
    let Some(rule) = rules.identifiers.iter().find(|r| r.node == node) else {
        return Ok(false);
    };
    if rule.child != child || rule.codec_class != class || rule.value_class != "amo" {
        bail!("native Identifier composition mismatch; update Voxrig");
    }
    Ok(true)
}
#[derive(Deserialize)]
struct EnumRule {
    node: usize,
    normalization: Mode,
    values: Vec<EnumValue>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Zero,
    Wrap,
    Clamp,
}
#[derive(Deserialize)]
struct EnumValue {
    native_id: i32,
}
#[derive(Deserialize)]
struct ScalarRule {
    node: usize,
    codec_class: String,
    value_class: String,
    width: usize,
}
fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../data/client_api/component_value_rules-1.21.11.json"
        ))
        .expect("pinned original scalar/enum facts")
    })
}
pub(super) fn enumeration(node: usize, value: i32) -> Result<i32> {
    let rule = rules()
        .enums
        .iter()
        .find(|r| r.node == node)
        .context("missing native enum normalization; update Voxrig")?;
    match rule.normalization {
        Mode::Zero => Ok(if rule.values.iter().any(|v| v.native_id == value) {
            value
        } else {
            0
        }),
        Mode::Wrap => Ok(value.rem_euclid(rule.values.len() as i32)),
        Mode::Clamp => Ok(value.clamp(0, rule.values.len() as i32 - 1)),
    }
}
pub(super) fn fixed(node: usize, class: &str, bytes: &[u8]) -> Result<Value> {
    let rule = rules()
        .scalars
        .iter()
        .find(|r| r.node == node)
        .context("missing native scalar facts; update Voxrig")?;
    if rule.codec_class != class || rule.width != bytes.len() {
        bail!("native scalar composition mismatch");
    }
    Ok(match rule.value_class.as_str() {
        "java.lang.Float" => Value::Float {
            bits: u32::from_be_bytes(bytes.try_into()?),
        },
        "java.lang.Double" => Value::Double {
            bits: u64::from_be_bytes(bytes.try_into()?),
        },
        "java.lang.Integer" => Value::Integer(i32::from_be_bytes(bytes.try_into()?)),
        "is" => Value::BlockPosition {
            packed: i64::from_be_bytes(bytes.try_into()?),
        },
        "java.util.UUID" => Value::Uuid {
            most: i64::from_be_bytes(bytes[..8].try_into()?),
            least: i64::from_be_bytes(bytes[8..].try_into()?),
        },
        _ => bail!("unreviewed native fixed value kind; update Voxrig"),
    })
}
#[cfg(test)]
pub(super) fn enum_domains() -> std::collections::BTreeMap<usize, Vec<i32>> {
    rules()
        .enums
        .iter()
        .map(|r| (r.node, r.values.iter().map(|v| v.native_id).collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{definition, framing};
    use super::*;
    use serde_json::Value as Json;
    use std::io::Read;
    fn facts() -> Json {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../../../data/client_api/component_value_cases-1.21.11.json.gz")[..]).read_to_end(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    fn normalization_facts() -> Json {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!(
                "../../../../data/client_api/component_normalization_cases-1.21.11.json.gz"
            )[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    #[test]
    fn identifier_constructor_normalizes_all_native_spellings_and_rejects_bad_ids() {
        let facts = normalization_facts();
        let model = super::super::definitions()
            .iter()
            .find(|d| d.name == "minecraft:item_model")
            .unwrap();
        for row in facts["identifiers"].as_array().unwrap() {
            let input = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
            let decoded = framing::decode_node(13, &input);
            assert_eq!(
                decoded.is_ok(),
                row["accepted"].as_bool().unwrap(),
                "{:?}",
                row["input"]
            );
            let mut patch_bytes = Vec::new();
            for number in [1, 0, model.native_id] {
                crate::protocol::put_varint(&mut patch_bytes, number);
            }
            patch_bytes.extend_from_slice(&input);
            let patch = super::super::decode_patch(&patch_bytes);
            assert_eq!(
                patch.is_ok(),
                decoded.is_ok(),
                "normal receive must invoke native ID grammar"
            );
            if let Ok(patch) = patch {
                assert_eq!(
                    patch.added[0].bytes, input,
                    "receipt must keep original spelling bytes"
                );
            }
            if let Ok(decoded) = decoded {
                let Value::Identifier { namespace, path } = &decoded else {
                    panic!("native Identifier must retain normalized namespace/path")
                };
                assert_eq!(namespace.as_str(), row["namespace"].as_str().unwrap());
                assert_eq!(path.as_str(), row["path"].as_str().unwrap());
                assert_eq!(
                    serde_json::to_value(decoded).unwrap(),
                    serde_json::to_value(
                        framing::decode_node(
                            13,
                            &hex::decode(row["canonical_hex"].as_str().unwrap()).unwrap()
                        )
                        .unwrap()
                    )
                    .unwrap()
                );
            }
        }
    }
    #[test]
    fn native_text_aliases_prove_wire_fields_are_not_semantic_item_identity() {
        let facts = normalization_facts();
        let rows = facts["text"].as_array().unwrap();
        assert_eq!(rows.len(), 125);
        let native = definition(6).unwrap();
        assert_eq!(native.name, "minecraft:custom_name");
        let mut contents = std::collections::BTreeSet::new();
        for row in rows.iter().filter(|r| r["accepted"] == true) {
            let decoded = framing::decode_value(
                native,
                &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            let Value::Forward { codec, value } = decoded else {
                panic!("text constructor identity must be retained")
            };
            assert_eq!(codec, 7);
            assert!(matches!(*value, Value::Nbt(Some(_))));
            contents.insert(row["contents_class"].as_str().unwrap());
        }
        assert_eq!(contents.len(), 8); // All original contents implementations, not a runtime semantic implementation.
        let index = |name| rows.iter().position(|r| r["case"] == name).unwrap();
        for (left, right, equal) in [
            ("literal", "record", true),
            ("color-red", "color-#ff5555", true),
            ("literal", "bold-false", false),
        ] {
            let a = index(left);
            let b = index(right);
            let pair = facts["text_pairs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["a"] == a.min(b) && p["b"] == a.max(b))
                .unwrap();
            assert_eq!(pair["component_equal"], equal);
            assert_eq!(pair["same_item_data"], equal);
            assert_eq!(pair["item_matches"], equal);
            let raw = |i: usize| {
                framing::decode_value(
                    native,
                    &hex::decode(rows[i]["input_hex"].as_str().unwrap()).unwrap(),
                )
                .unwrap()
            };
            assert_ne!(
                serde_json::to_value(raw(a)).unwrap(),
                serde_json::to_value(raw(b)).unwrap()
            );
        }
    }
    #[test]
    fn normalization_facts_are_bound_to_original_inputs_and_own_generators() {
        use sha2::{Digest, Sha256};
        let source: Json = serde_json::from_str(include_str!(
            "../../../../data/client_api/component_normalization_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for group in ["generators_sha256", "files_sha256"] {
            for (path, hash) in source[group].as_object().unwrap() {
                assert_eq!(
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(root.join(path)).unwrap())
                    ),
                    hash.as_str().unwrap(),
                    "{path}"
                );
            }
        }
        assert_eq!(source["identifier_cases"], 274);
        assert_eq!(source["forward_nodes"], 308);
        assert_eq!(source["accepted_text_cases"], 83);
        assert_eq!(source["text_equality_pairs"], 3486);
    }
    #[test]
    fn every_original_enum_factory_alias_and_fixed_scalar_is_typed_exactly() {
        let facts = facts();
        let mut probes = 0;
        for entry in facts["enums"].as_array().unwrap() {
            let node = entry["node"].as_u64().unwrap() as usize;
            for probe in entry["probes"].as_array().unwrap() {
                let input = probe["input"].as_i64().unwrap() as i32;
                let mut bytes = Vec::new();
                crate::protocol::put_varint(&mut bytes, input);
                let Value::Enumeration { codec, native_id } =
                    framing::decode_node(node, &bytes).unwrap()
                else {
                    panic!("enum must retain its own codec domain")
                };
                assert_eq!(codec, node);
                assert_eq!(
                    native_id,
                    probe["native_id"].as_i64().unwrap() as i32,
                    "node {node} input {input}"
                );
                let canonical = hex::decode(probe["canonical_hex"].as_str().unwrap()).unwrap();
                assert_eq!(
                    crate::protocol::get_varint(&mut canonical.as_slice()).unwrap(),
                    native_id
                );
                probes += 1;
            }
        }
        assert_eq!(probes, 6230);
        assert!(enum_domains()[&628].contains(&1281));
        assert!(enum_domains()[&630].contains(&99));
        for entry in facts["scalars"].as_array().unwrap() {
            let node = entry["node"].as_u64().unwrap() as usize;
            for probe in entry["probes"].as_array().unwrap() {
                let decoded = framing::decode_node(
                    node,
                    &hex::decode(probe["input_hex"].as_str().unwrap()).unwrap(),
                )
                .unwrap();
                let expected = &probe["value"];
                match decoded {
                    Value::Float { bits } => {
                        assert_eq!(expected["kind"], "float");
                        assert_eq!(u64::from(bits), expected["bits"].as_u64().unwrap());
                    }
                    Value::Double { bits } => {
                        assert_eq!(expected["kind"], "double");
                        assert_eq!(bits, expected["bits"].as_u64().unwrap());
                    }
                    Value::Integer(value) => {
                        assert_eq!(expected["kind"], "integer");
                        assert_eq!(i64::from(value), expected["value"].as_i64().unwrap());
                    }
                    Value::Uuid { most, least } => {
                        assert_eq!(expected["kind"], "uuid");
                        assert_eq!(most, expected["most"].as_i64().unwrap());
                        assert_eq!(least, expected["least"].as_i64().unwrap());
                    }
                    Value::BlockPosition { packed } => {
                        assert_eq!(expected["kind"], "block_position");
                        assert_eq!(packed >> 38, expected["x"].as_i64().unwrap());
                        assert_eq!((packed << 52) >> 52, expected["y"].as_i64().unwrap());
                        assert_eq!((packed << 26) >> 38, expected["z"].as_i64().unwrap());
                    }
                    _ => panic!("fixed leaf must have the native scalar type"),
                }
            }
        }
    }
    #[test]
    fn all_original_104_component_fields_and_prototypes_decode_to_typed_structure() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/item_semantics-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let corpus: Json = serde_json::from_slice(&bytes).unwrap();
        let rows = corpus["components"].as_array().unwrap();
        let mut kinds = std::collections::BTreeSet::new();
        for row in rows {
            let native = definition(row["native_id"].as_i64().unwrap() as i32).unwrap();
            let input = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
            let canonical = hex::decode(row["canonical_hex"].as_str().unwrap()).unwrap();
            let decoded = framing::decode_value(native, &input).unwrap();
            assert_eq!(
                serde_json::to_value(&decoded).unwrap(),
                serde_json::to_value(framing::decode_value(native, &canonical).unwrap()).unwrap(),
                "{}",
                row["case"]
            );
            kinds.insert(native.name.clone());
            let mut trailing = input;
            trailing.push(0);
            assert!(
                framing::decode_value(native, &trailing).is_err(),
                "{} trailing byte",
                native.name
            );
            if let Some(node) = row["hash_node"].as_u64() {
                let expected = &corpus["hash_nodes"][node as usize];
                match (&decoded, expected["kind"].as_str().unwrap()) {
                    (Value::Integer(value), "int") => {
                        assert_eq!(i64::from(*value), expected["value"].as_i64().unwrap())
                    }
                    (Value::Float { bits }, "float") => {
                        assert_eq!(u64::from(*bits), expected["bits"].as_u64().unwrap())
                    }
                    (Value::Double { bits }, "double") => {
                        assert_eq!(bits.to_string(), expected["bits"].as_str().unwrap())
                    }
                    _ => {} // No inference from a compound/text persistent codec to wire fields.
                }
            }
        }
        assert_eq!((rows.len(), kinds.len()), (4134, 104));
    }
    #[test]
    fn original_value_rules_and_fields_are_bound_to_their_native_sources() {
        use sha2::{Digest, Sha256};
        let source: Json = serde_json::from_str(include_str!(
            "../../../../data/client_api/component_value_rules_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["generators_sha256", "files_sha256"] {
            for (path, hash) in source[name].as_object().unwrap() {
                assert_eq!(
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(root.join(path)).unwrap())
                    ),
                    hash.as_str().unwrap(),
                    "{path}"
                );
            }
        }
        assert_eq!(source["enums"], 19);
        assert_eq!(source["scalar_nodes"], 5);
        assert_eq!(source["unnamed_tags"], 20);
    }
}
