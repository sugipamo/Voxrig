use super::{Reader, definitions, framing, values::Value};
use crate::{
    MinecraftVersion,
    client::{item_constructor::Item, registry::RegistryKind},
};
use serde_json::Value as Json;
use std::io::Read;
fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!(
            "../../../../data/client_api/nested_item_constructor_cases-1.21.11.json.gz"
        )[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn items(value: &Value) -> Vec<&Item<Box<Value>>> {
    match value {
        Value::Bundle(value) => value.items.iter().collect(),
        Value::Forward {
            codec: 608 | 511,
            value,
        } => {
            let Value::List(values) = &**value else {
                panic!("native item list missing");
            };
            values
                .iter()
                .map(|value| {
                    let Value::Item(item) = value else {
                        panic!("common item missing");
                    };
                    item
                })
                .collect()
        }
        Value::Forward { codec: 453, value } => {
            let Value::Item(item) = &**value else {
                panic!("common remainder item missing");
            };
            vec![item]
        }
        _ => panic!("unexpected native nested item root: {value:?}"),
    }
}
fn describe(value: &Value, native: &Json) {
    let actual = items(value);
    let expected = native["items"].as_array().unwrap();
    assert_eq!(actual.len(), expected.len());
    for (item, row) in actual.iter().zip(expected) {
        assert_eq!(item.count, row["count"].as_i64().unwrap() as i32);
        assert_eq!(item.native_id.is_none(), row["empty"].as_bool().unwrap());
        let bytes = hex::decode(row["encoded_hex"].as_str().unwrap()).unwrap();
        let mut r = Reader::new(&bytes);
        let count = r.varint().unwrap();
        assert_eq!(count, item.count);
        if let Some(id) = item.native_id {
            assert_eq!(id.version(), MinecraftVersion::Java1_21_11);
            assert_eq!(id.kind(), RegistryKind::Item);
            assert_eq!(id.value(), r.varint().unwrap());
            assert!(item.patch.is_some());
        } else {
            assert!(item.patch.is_none());
            r.end().unwrap();
        }
    }
    if let Value::Bundle(fields) = value {
        assert_eq!(
            fields.weight.numerator,
            native["numerator"].as_i64().unwrap() as i32
        );
        assert_eq!(
            fields.weight.denominator,
            native["denominator"].as_i64().unwrap() as i32
        );
        assert_eq!(native["selected"], -1);
    }
}
#[test]
fn original_nested_item_constructors_receive_capture_and_weight_routes_match() {
    let data = facts();
    let mut accepted = 0;
    for row in data["cases"].as_array().unwrap() {
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
        let receipt = framing::value(&mut r, native, &mut budget, 0).and_then(|_| r.end());
        assert_eq!(
            receipt.is_ok(),
            decoded.is_ok(),
            "{} receipt {:?}",
            row["case"],
            receipt.err()
        );
        if let Ok(value) = decoded {
            describe(&value, &row["fields"]);
            let encoded = hex::decode(row["encoded_hex"].as_str().unwrap()).unwrap();
            describe(
                &framing::decode_value(native, &encoded).unwrap(),
                &row["fields"],
            );
            accepted += 1;
        }
    }
    assert_eq!(
        (data["cases"].as_array().unwrap().len(), accepted),
        (425, 364)
    );
    // Original whole-component equals pairs are retained as evidence for the
    // next prototype/context comparison stage, not claimed as implemented here.
    assert_eq!(data["pairs"].as_array().unwrap().len(), 66430);
}
#[test]
fn original_fraction_and_nested_item_facts_are_source_bound() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for encoded in [
        include_str!("../../../../data/client_api/fraction_constructor_source.json"),
        include_str!("../../../../data/client_api/nested_item_constructor_source.json"),
    ] {
        let source: Json = serde_json::from_str(encoded).unwrap();
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

#[test]
fn nested_constructor_normal_receive_keeps_depth_and_work_limits() {
    let native = definitions()
        .iter()
        .find(|d| d.name == "minecraft:bundle_contents")
        .unwrap();
    // The empty list still visits its Forward and List nodes.
    for depth in [256, 257] {
        let mut reader = Reader::new(&[0]);
        let mut budget = framing::Budget::new(&reader);
        assert!(
            framing::value(&mut reader, native, &mut budget, depth)
                .unwrap_err()
                .to_string()
                .contains("depth limit")
        );
    }
    let mut reader = Reader::new(&[0]);
    let mut budget = framing::Budget::new(&reader);
    framing::value(&mut reader, native, &mut budget, 255).unwrap();
    reader.end().unwrap();
    // Arithmetic/prototype extraction cannot turn a large valid item list into
    // a bypass of the grammar budget. Normal receipt retains no item Value tree.
    let mut bytes = vec![0x80, 0x80, 0x04];
    for _ in 0..65536 {
        bytes.extend_from_slice(&[1, 1, 0, 0]);
    }
    let mut reader = Reader::new(&bytes);
    let mut budget = framing::Budget::new(&reader);
    assert!(
        framing::value(&mut reader, native, &mut budget, 0)
            .unwrap_err()
            .to_string()
            .contains("work limit")
    );
    assert!(reader.remaining().len() >= 8);
}
