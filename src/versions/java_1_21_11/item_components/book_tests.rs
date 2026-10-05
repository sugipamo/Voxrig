use super::{Reader, definitions, framing, text_tests, values::Value};
use crate::client::books::{Enchantability, Filtered, WritableBook, WrittenFields};
use serde_json::Value as Json;
use std::io::Read;

fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/book_constructor_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[derive(Debug, PartialEq, Eq)]
enum Key {
    Enchantability(Enchantability),
    Writable(WritableBook),
    Written(Box<WrittenFields>),
}
fn describe(value: &Value, expected: &Json) -> Key {
    match value {
        Value::Enchantability(value) => {
            assert_eq!(value.value, expected["value"].as_i64().unwrap() as i32);
            Key::Enchantability(value.clone())
        }
        Value::WritableBook(value) => {
            assert_eq!(serde_json::to_value(value).unwrap(), *expected);
            Key::Writable(value.clone())
        }
        Value::WrittenBook {
            fields,
            dependencies,
            field_key,
        } => {
            assert_eq!(
                serde_json::to_value(&fields.title).unwrap(),
                expected["title"]
            );
            assert_eq!(
                serde_json::to_value(&fields.author).unwrap(),
                expected["author"]
            );
            assert_eq!(
                fields.generation,
                expected["generation"].as_i64().unwrap() as i32
            );
            assert_eq!(fields.resolved, expected["resolved"].as_bool().unwrap());
            assert_eq!(
                fields.pages.len(),
                expected["pages"].as_array().unwrap().len()
            );
            for (page, row) in fields
                .pages
                .iter()
                .zip(expected["pages"].as_array().unwrap())
            {
                text_tests::describe(&page.raw, &row["raw"]);
                match &page.filtered {
                    Some(text) => text_tests::describe(text, &row["filtered"]),
                    None => assert!(row["filtered"].is_null()),
                }
            }
            assert!(dependencies.is_empty());
            Key::Written(
                field_key
                    .clone()
                    .expect("observed book has fully represented text fields"),
            )
        }
        _ => panic!("missing common book constructor fields: {value:?}"),
    }
}
#[test]
fn original_book_constructors_fields_and_all_observed_pairs_match() {
    let facts = facts();
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
        let mut reader = Reader::new(&input);
        let mut budget = framing::Budget::new(&reader);
        let receipt =
            framing::value(&mut reader, native, &mut budget, 0).and_then(|_| reader.end());
        assert_eq!(
            receipt.is_ok(),
            decoded.is_ok(),
            "{} ordinary receipt",
            row["case"]
        );
        values.push(decoded.ok().map(|value| {
            let key = describe(&value, &row["fields"]);
            let encoded = hex::decode(row["encoded_hex"].as_str().unwrap()).unwrap();
            let encoded = framing::decode_value(native, &encoded).unwrap();
            assert_eq!(
                key,
                describe(&encoded, &row["fields"]),
                "{} original re-encoded fields",
                row["case"]
            );
            key
        }));
    }
    assert_eq!(
        (values.len(), values.iter().filter(|v| v.is_some()).count()),
        (118, 71)
    );
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "{pair}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 2556);
}
#[test]
fn original_book_sources_and_filtered_text_reference_dependencies_are_retained() {
    use crate::client::{books::WrittenBook, nbt::NbtString, text::Dependency};
    use sha2::{Digest, Sha256};
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/book_constructor_source.json"
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
    // Use previously observed original text constructors. A filtered page must
    // not drop references absent from its raw counterpart or grant comparison.
    let mut text_bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/text_constructor_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut text_bytes)
    .unwrap();
    let text_facts: Json = serde_json::from_slice(&text_bytes).unwrap();
    let mut checked = 0;
    for row in text_facts["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["accepted"] == true)
    {
        let bytes = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
        let native = definitions()
            .iter()
            .find(|d| d.name == "minecraft:custom_name")
            .unwrap();
        let Value::Text {
            fields: text,
            dependencies,
            ..
        } = framing::decode_value(native, &bytes).unwrap()
        else {
            unreachable!()
        };
        if !dependencies.contains(&Dependency::Item) && !dependencies.contains(&Dependency::Dialog)
        {
            continue;
        }
        let mut book = WrittenBook {
            title: Filtered {
                raw: NbtString::from_text(""),
                filtered: None,
            },
            author: NbtString::from_text(""),
            generation: 0,
            pages: vec![Filtered {
                raw: text.clone(),
                filtered: Some(text),
            }],
            resolved: false,
        };
        let expected = dependencies
            .iter()
            .chain(&dependencies)
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(book.dependencies(), expected);
        assert!(book.field_comparison().is_none());
        // Reference only in the filtered version remains unresolved too.
        let literal = std::sync::Arc::new(crate::client::nbt::NbtValue::String(
            NbtString::from_text("raw"),
        ));
        *book.pages[0].raw = super::text::project(&literal).unwrap();
        assert_eq!(book.dependencies(), dependencies);
        assert!(book.field_comparison().is_none());
        checked += 1;
    }
    assert!(checked > 0);
}
