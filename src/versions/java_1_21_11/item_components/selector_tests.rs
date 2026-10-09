use super::{framing, profile_tests, text, values};
use crate::client::{nbt, selector};
use serde_json::Value as Json;
use std::io::Read;

fn string(value: &Json) -> nbt::NbtString {
    match value.as_str() {
        Some(value) => nbt::NbtString::from_text(value),
        None => nbt::NbtString::from_units(
            value["utf16"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u16)
                .collect(),
        ),
    }
}
#[derive(PartialEq, Eq)]
enum Key {
    Pattern(selector::Pattern),
    Text(crate::client::text::FieldKey),
    Profile(crate::client::profile::Profile),
}
#[test]
fn original_selector_constructors_cursors_text_branches_profiles_and_pairs_match() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/selector_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let facts: Json = serde_json::from_slice(&bytes).unwrap();
    let mut values = Vec::new();
    let mut accepted = 0;
    let mut mismatches = Vec::new();
    for row in facts["cases"].as_array().unwrap() {
        let result: anyhow::Result<Key> = match row["kind"].as_str().unwrap() {
            "selector" => {
                selector::parse(&string(&row["pattern"]), &mut 65536).map(|(value, cursor)| {
                    if row["accepted"] == true {
                        assert_eq!(
                            serde_json::to_value(&value).unwrap(),
                            row["fields"]["pattern"],
                            "{}",
                            row["case"]
                        );
                        assert_eq!(
                            cursor as u64,
                            row["fields"]["cursor"].as_u64().unwrap(),
                            "{}",
                            row["case"]
                        );
                    }
                    Key::Pattern(value)
                })
            }
            "profile" => {
                let value = framing::decode_node(
                    583,
                    &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
                );
                value.and_then(|v| match v {
                    values::Value::Profile(p) => {
                        let mut expected = row["fields"].clone();
                        expected["properties"]
                            .as_array_mut()
                            .unwrap()
                            .sort_by_key(|v| {
                                v["key"]
                                    .as_str()
                                    .unwrap()
                                    .encode_utf16()
                                    .collect::<Vec<_>>()
                            });
                        assert_eq!(profile_tests::fields(&p), expected, "{}", row["case"]);
                        Ok(Key::Profile(*p))
                    }
                    _ => anyhow::bail!("native profile root not normalized"),
                })
            }
            "text" => {
                let root = nbt::decode_unnamed_tag(
                    &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
                )
                .unwrap()
                .unwrap();
                text::project(&root).map(|v| {
                    if mismatches.is_empty() && row["accepted"] == true {
                        super::text_tests::describe(&v, &row["fields"]);
                    }
                    Key::Text(
                        v.modern_field_key()
                            .expect("complete selector/profile-only corpus key"),
                    )
                })
            }
            _ => panic!("unreviewed native row kind"),
        };
        if result.is_ok() != row["accepted"].as_bool().unwrap() {
            mismatches.push(format!(
                "{} {:?}: Rust={}, native={}, {}",
                row["case"],
                row.get("pattern"),
                result.is_ok(),
                row["accepted"],
                result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            ));
        }
        if result.is_ok() {
            accepted += 1;
        }
        values.push(result.ok());
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches
            .into_iter()
            .take(30)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!((values.len(), accepted), (1955, 998));
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "{pair}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 498501);
}
#[test]
fn original_selector_and_character_rules_and_sources_are_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for source in [
        include_str!("../../../../data/client_api/selector_source.json"),
        include_str!("../../../../data/client_api/character_names_source.json"),
    ] {
        let source: Json = serde_json::from_str(source).unwrap();
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
    }
}
