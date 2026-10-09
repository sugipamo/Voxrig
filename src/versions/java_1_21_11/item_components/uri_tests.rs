use super::{text, text_tests};
use crate::client::{nbt, uri};
use serde_json::Value as Json;
use std::io::Read;
fn string(value: &Json) -> nbt::NbtString {
    match value.as_str() {
        Some(v) => nbt::NbtString::from_text(v),
        None => nbt::NbtString::from_units(
            value["utf16"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u.as_u64().unwrap() as u16)
                .collect(),
        ),
    }
}
fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/uri_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[derive(Eq, PartialEq)]
enum Key {
    Uri(Box<uri::Uri>),
    Text(Box<crate::client::text::FieldKey>),
}
fn clicks(value: &crate::client::text::Text, route: &str, out: &mut Vec<Json>) {
    use crate::client::text::{Argument, Click, Contents, Hover};
    if let Some(Click::OpenUrl(v)) = &value.style.click {
        out.push(serde_json::json!({"route":route,"fields":serde_json::to_value(v).unwrap()}));
    }
    match &value.contents {
        Contents::Translate { arguments, .. } => {
            for (i, arg) in arguments.iter().enumerate() {
                if let Argument::Text(child) = arg {
                    clicks(child, &format!("{route}/arg{i}"), out);
                }
            }
        }
        Contents::Selector {
            separator: Some(child),
            ..
        }
        | Contents::Nbt {
            separator: Some(child),
            ..
        } => clicks(child, &format!("{route}/separator"), out),
        _ => {}
    }
    if let Some(Hover::Text(child)) = &value.style.hover {
        clicks(child, &format!("{route}/hover"), out);
    }
    for (i, child) in value.siblings.iter().enumerate() {
        clicks(child, &format!("{route}/sibling{i}"), out);
    }
}
#[test]
fn original_uri_constructors_getters_nested_text_and_pairs_match() {
    let facts = facts();
    let mut values = Vec::new();
    let mut accepted = 0;
    let mut mismatches = Vec::new();
    for row in facts["cases"].as_array().unwrap() {
        let result: anyhow::Result<Key> = match row["kind"].as_str().unwrap() {
            "uri" => uri::parse(&string(&row["value"]), &mut 65536).map(|v| {
                if row["accepted"] == true {
                    assert_eq!(
                        serde_json::to_value(&v).unwrap(),
                        row["fields"],
                        "{} {:?}",
                        row["case"],
                        row["value"]
                    );
                }
                Key::Uri(Box::new(v))
            }),
            "text" => {
                let root = nbt::decode_unnamed_tag(
                    &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
                )
                .unwrap()
                .unwrap();
                text::project(&root).map(|v| {
                    if row["accepted"] == true {
                        text_tests::describe(&v, &row["fields"]);
                        let mut found = Vec::new();
                        clicks(&v, "root", &mut found);
                        assert_eq!(
                            serde_json::to_value(found).unwrap(),
                            row["uri_clicks"],
                            "{}",
                            row["case"]
                        );
                    }
                    Key::Text(Box::new(
                        v.modern_field_key().expect("URI-only native corpus key"),
                    ))
                })
            }
            _ => panic!("unreviewed native URI row kind"),
        };
        if result.is_ok() != row["accepted"].as_bool().unwrap() {
            mismatches.push(format!(
                "{} {:?}: Rust={}, native={}, {}",
                row["case"],
                row.get("value"),
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
            .take(40)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!((values.len(), accepted), (1810, 1089));
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "{pair}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 593505);
}
#[test]
fn original_uri_rules_and_sources_are_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Json =
        serde_json::from_str(include_str!("../../../../data/client_api/uri_source.json")).unwrap();
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
