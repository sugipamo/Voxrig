use super::{text, text_tests};
use crate::client::{
    nbt::NbtValue,
    text::{Argument, Click, Contents, Hover, Text},
};
use serde_json::{Value as Json, json};
use std::io::Read;
fn payload(value: &NbtValue) -> Json {
    match value {
        NbtValue::Byte(v) => json!({"kind":1,"value":v}),
        NbtValue::Short(v) => json!({"kind":2,"value":v}),
        NbtValue::Int(v) => json!({"kind":3,"value":v}),
        NbtValue::Long(v) => json!({"kind":4,"value":v}),
        NbtValue::Float { bits } => json!({"kind":5,"bits":bits}),
        NbtValue::Double { bits } => json!({"kind":6,"bits":bits.to_string()}),
        NbtValue::ByteArray(v) => json!({"kind":7,"values":v}),
        NbtValue::String(v) => json!({"kind":8,"units":v.utf16()}),
        NbtValue::List(v) => {
            json!({"kind":9,"values":v.iter().map(|v|payload(v)).collect::<Vec<_>>()})
        }
        NbtValue::Compound(v) => {
            json!({"kind":10,"entries":v.entries().iter().map(|e|json!({"key_units":e.key().utf16(),"value":payload(e.value())})).collect::<Vec<_>>()})
        }
        NbtValue::IntArray(v) => json!({"kind":11,"values":v}),
        NbtValue::LongArray(v) => json!({"kind":12,"values":v}),
    }
}
fn clicks(value: &Text, route: &str, out: &mut Vec<Json>) {
    if let Some(click) = &value.style.click {
        let fields = match click {
            Click::RunCommand(v) => json!({"class":"yf$g","command":v}),
            Click::SuggestCommand(v) => json!({"class":"yf$i","command":v}),
            Click::Copy(v) => json!({"class":"yf$c","value":v}),
            Click::ChangePage(v) => json!({"class":"yf$b","page":v}),
            Click::Custom { id, payload: p } => {
                json!({"class":"yf$d","id":format!("{}:{}",id.namespace,id.path),"payload":p.as_ref().map(|v|payload(v))})
            }
            _ => panic!("unreviewed click in native constructor corpus"),
        };
        out.push(json!({"route":route,"fields":fields}));
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
fn original_click_constructors_fields_nested_fallback_font_sources_and_pairs_match() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/click_constructor_cases-1.21.11.json.gz")[..],
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
                clicks(&v, "root", &mut found);
                assert_eq!(
                    serde_json::to_value(found).unwrap(),
                    row["clicks"],
                    "{}",
                    row["case"]
                );
            }
            v.modern_field_key()
                .expect("native click-only corpus field key")
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
    assert_eq!((values.len(), accepted), (697, 583));
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "{pair}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 170236);
}
#[test]
fn original_click_constructor_rules_and_sources_are_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/click_constructor_source.json"
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
