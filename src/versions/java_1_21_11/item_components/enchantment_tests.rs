use super::{Reader, definitions, framing, values::Value};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, io::Read};
fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!(
            "../../../../data/client_api/enchantment_constructor_cases-1.21.11.json.gz"
        )[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn fields(native: &Json) -> BTreeMap<i32, i32> {
    native
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let bytes = hex::decode(e["key_hex"].as_str().unwrap()).unwrap();
            let mut r = Reader::new(&bytes);
            let key = r.varint().unwrap();
            r.end().unwrap();
            (key, e["level"].as_i64().unwrap() as i32)
        })
        .collect()
}
#[test]
fn original_enchantment_constructors_effective_maps_and_all_pairs_match() {
    let facts = facts();
    let mut accepted = 0;
    let mut values = Vec::new();
    for row in facts["cases"].as_array().unwrap() {
        let native = definitions()
            .iter()
            .find(|d| d.name == row["component"])
            .unwrap();
        let input = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
        let decoded = framing::decode_value(native, &input);
        assert_eq!(
            decoded.is_ok(),
            row["accepted"].as_bool().unwrap(),
            "{}: {:?}",
            row["case"],
            decoded.as_ref().err()
        );
        let mut r = Reader::new(&input);
        let mut budget = framing::Budget::new(&r);
        let framing_result = framing::value(&mut r, native, &mut budget, 0).and_then(|_| r.end());
        assert_eq!(
            framing_result.is_ok(),
            decoded.is_ok(),
            "{} normal receive",
            row["case"]
        );
        values.push(decoded.ok().map(|v| {
            let Value::Enchantments(v) = v else {
                panic!("native enchantment model missing")
            };
            assert_eq!(v.levels, fields(&row["fields"]), "{}", row["case"]);
            let encoded = hex::decode(row["encoded_hex"].as_str().unwrap()).unwrap();
            let Value::Enchantments(encoded) = framing::decode_value(native, &encoded).unwrap()
            else {
                panic!("encoded enchantment model missing")
            };
            assert_eq!(v, encoded);
            accepted += 1;
            v
        }));
    }
    assert_eq!((values.len(), accepted), (92, 54));
    for p in facts["pairs"].as_array().unwrap() {
        let a = values[p["a"].as_u64().unwrap() as usize].as_ref().unwrap();
        let b = values[p["b"].as_u64().unwrap() as usize].as_ref().unwrap();
        assert_eq!(a == b, p["equal"].as_bool().unwrap(), "{p}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 1485);
}
#[test]
fn original_enchantment_rules_sources_and_normal_receive_budget_are_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/enchantment_constructor_source.json"
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
    let rules: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/enchantment_constructor_rules-1.21.11.json"
    ))
    .unwrap();
    assert_eq!(rules["level_minimum"], 0);
    assert_eq!(rules["level_maximum"], 255);
    assert_eq!(rules["duplicate_keys"], "last_wins");
    let native = definitions()
        .iter()
        .find(|d| d.name == "minecraft:enchantments")
        .unwrap();
    let input = [1, 40, 2];
    for depth in [255, 256] {
        let mut r = Reader::new(&input);
        let mut budget = framing::Budget::new(&r);
        assert!(
            framing::value(&mut r, native, &mut budget, depth)
                .unwrap_err()
                .to_string()
                .contains("depth limit")
        );
    }
    // Many duplicate entries still consume work; overwriting cannot bypass the
    // work bound or produce an accepted truncated field.
    let mut input = vec![0x80, 0x80, 0x02];
    for _ in 0..32768 {
        input.extend_from_slice(&[40, 2]);
    }
    let mut r = Reader::new(&input);
    let mut budget = framing::Budget::new(&r);
    assert!(
        framing::value(&mut r, native, &mut budget, 0)
            .unwrap_err()
            .to_string()
            .contains("work limit")
    );
    assert_eq!(json!(source["accepted"]), 54);
}
