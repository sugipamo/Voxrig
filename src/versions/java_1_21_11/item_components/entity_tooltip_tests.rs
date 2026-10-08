use super::{text, text_tests};
use crate::client::text::{Argument, Contents, Hover, Text};
use serde_json::{Value as Json, json};
use std::io::Read;
fn entities(value: &Text, route: &str, out: &mut Vec<Json>) {
    if let Some(Hover::Entity(v)) = &value.style.hover {
        let fields = json!({"id":format!("{}:{}",v.entity_type.namespace,v.entity_type.path), "uuid":v.uuid, "name": v.name.is_some()});
        out.push(json!({"route":route,"fields":fields}));
        if let Some(name) = &v.name {
            entities(name, &format!("{route}/name"), out);
        }
    }
    match &value.contents {
        Contents::Translate { arguments, .. } => {
            for (i, arg) in arguments.iter().enumerate() {
                if let Argument::Text(child) = arg {
                    entities(child, &format!("{route}/arg{i}"), out);
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
        } => entities(child, &format!("{route}/separator"), out),
        _ => {}
    }
    if let Some(Hover::Text(child)) = &value.style.hover {
        entities(child, &format!("{route}/hover"), out);
    }
    for (i, child) in value.siblings.iter().enumerate() {
        entities(child, &format!("{route}/sibling{i}"), out);
    }
}
fn check_names(value: &Text, route: &str, native: &[Json]) {
    if let Some(Hover::Entity(v)) = &value.style.hover {
        let row = native.iter().find(|r| r["route"] == route).unwrap();
        if let Some(name) = &v.name {
            text_tests::describe(name, &row["fields"]["name"]);
            check_names(name, &format!("{route}/name"), native);
        }
    }
    match &value.contents {
        Contents::Translate { arguments, .. } => {
            for (i, arg) in arguments.iter().enumerate() {
                if let Argument::Text(child) = arg {
                    check_names(child, &format!("{route}/arg{i}"), native);
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
        } => check_names(child, &format!("{route}/separator"), native),
        _ => {}
    }
    if let Some(Hover::Text(child)) = &value.style.hover {
        check_names(child, &format!("{route}/hover"), native);
    }
    for (i, child) in value.siblings.iter().enumerate() {
        check_names(child, &format!("{route}/sibling{i}"), native);
    }
}
#[test]
fn original_entity_tooltip_constructor_fields_nested_names_and_pairs_match() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/entity_tooltip_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let facts: Json = serde_json::from_slice(&bytes).unwrap();
    let mut values = Vec::new();
    let mut accepted = 0;
    let mut mismatches = Vec::new();
    for row in facts["cases"].as_array().unwrap() {
        let root = crate::client::nbt::decode_unnamed_tag(
            &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap()
        .unwrap();
        let result = text::project(&root);
        if result.is_ok() != row["accepted"].as_bool().unwrap() {
            mismatches.push(format!(
                "{} Rust={}, native={}, {}",
                row["case"],
                result.is_ok(),
                row["accepted"],
                result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            ));
        }
        let key = result.ok().map(|v| {
            if row["accepted"] == true {
                text_tests::describe(&v, &row["fields"]);
                let mut found = Vec::new();
                entities(&v, "root", &mut found);
                let native = row["entities"].as_array().unwrap();
                assert_eq!(found.len(), native.len(), "{}", row["case"]);
                for (a, b) in found.iter().zip(native) {
                    assert_eq!(a["route"], b["route"]);
                    assert_eq!(a["fields"]["id"], b["fields"]["id"]);
                    assert_eq!(a["fields"]["uuid"], b["fields"]["uuid"]);
                    assert_eq!(a["fields"]["name"], !b["fields"]["name"].is_null());
                }
                check_names(&v, "root", native);
            }
            v.modern_field_key()
                .expect("native entity-tooltip-only corpus field key")
        });
        if key.is_some() {
            accepted += 1;
        }
        values.push(key);
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
    assert_eq!((values.len(), accepted), (355, 94));
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "{pair}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 4465);
}
#[test]
fn original_entity_tooltip_rules_and_sources_are_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/entity_tooltip_source.json"
    ))
    .unwrap();
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
