use super::text::*;
use crate::client::{
    nbt,
    text::{Argument, Click, Contents, Hover, Number, Text},
};
use serde_json::Value as Json;
use std::io::Read;

fn facts() -> Json {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/text_core_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
pub(super) fn describe(text: &Text, expected: &Json) {
    let body = &expected["body"];
    match &text.contents {
        Contents::Literal { text } => {
            assert!(matches!(
                expected["contents_class"].as_str(),
                Some("zn$1" | "zn$a")
            ));
            assert_eq!(serde_json::to_value(text).unwrap(), body["text"]);
        }
        Contents::Keybind { keybind } => {
            assert_eq!(serde_json::to_value(keybind).unwrap(), body["keybind"])
        }
        Contents::Translate {
            key,
            fallback,
            arguments,
        } => {
            assert_eq!(serde_json::to_value(key).unwrap(), body["key"]);
            assert_eq!(serde_json::to_value(fallback).unwrap(), body["fallback"]);
            assert_eq!(arguments.len(), body["arguments"].as_array().unwrap().len());
            for (value, expected) in arguments.iter().zip(body["arguments"].as_array().unwrap()) {
                match value {
                    Argument::String(value) => {
                        assert_eq!(expected["class"], "java.lang.String");
                        assert_eq!(serde_json::to_value(value).unwrap(), expected["string"]);
                    }
                    Argument::Text(value) => {
                        assert_eq!(expected["class"], "yw");
                        describe(value, &expected["text"]);
                    }
                    Argument::Number(value) => {
                        let (class, integer) = match value {
                            Number::Byte(n) => ("java.lang.Byte", Some(i64::from(*n))),
                            Number::Short(n) => ("java.lang.Short", Some(i64::from(*n))),
                            Number::Integer(n) => ("java.lang.Integer", Some(i64::from(*n))),
                            Number::Long(n) => ("java.lang.Long", Some(*n)),
                            Number::Float { bits } => {
                                assert_eq!(expected["float_bits"], u64::from(*bits));
                                ("java.lang.Float", None)
                            }
                            Number::Double { bits } => {
                                assert_eq!(expected["double_bits"], bits.to_string());
                                ("java.lang.Double", None)
                            }
                        };
                        assert_eq!(expected["class"], class);
                        if let Some(integer) = integer {
                            assert_eq!(expected["integer_value"], integer);
                        }
                    }
                }
            }
        }
        Contents::Selector { pattern, separator } => {
            assert_eq!(serde_json::to_value(pattern).unwrap(), body["pattern"]);
            if let Some(v) = separator {
                describe(v, &body["separator"]);
            } else {
                assert!(body["separator"].is_null());
            }
        }
        Contents::Score { name, objective } => {
            let (pattern, selector) = match name {
                crate::client::text::ScoreName::Selector(v) => (&v.0, true),
                crate::client::text::ScoreName::Literal(v) => (v, false),
            };
            assert_eq!(serde_json::to_value(pattern).unwrap(), body["name"]);
            assert_eq!(body["name_is_selector"], selector);
            assert_eq!(serde_json::to_value(objective).unwrap(), body["objective"]);
        }
        Contents::Nbt {
            path,
            interpret,
            separator,
            source,
        } => {
            assert_eq!(serde_json::to_value(path).unwrap(), body["path"]);
            assert_eq!(body["interpret"], *interpret);
            if let Some(v) = separator {
                describe(v, &body["separator"]);
            } else {
                assert!(body["separator"].is_null());
            }
            let actual = match source {
                crate::client::text::NbtSource::Block(v)
                | crate::client::text::NbtSource::Entity(v) => v.text().unwrap(),
                crate::client::text::NbtSource::Storage(v) => format!("{}:{}", v.namespace, v.path),
            };
            assert_eq!(body["source"], actual);
        }
        Contents::Sprite { atlas, sprite } => {
            assert_eq!(body["object_class"], "zy");
            assert_eq!(body["atlas"], format!("{}:{}", atlas.namespace, atlas.path));
            assert_eq!(
                body["sprite"],
                format!("{}:{}", sprite.namespace, sprite.path)
            );
        }
        Contents::PlayerSprite { hat, .. } => {
            assert_eq!(body["object_class"], "aab");
            assert_eq!(body["hat"], *hat);
        }
    }
    let s = &text.style;
    let expected_style = &expected["style"];
    for (name, value) in [
        ("bold", s.bold),
        ("italic", s.italic),
        ("underlined", s.underlined),
        ("strikethrough", s.strikethrough),
        ("obfuscated", s.obfuscated),
    ] {
        assert_eq!(serde_json::to_value(value).unwrap(), expected_style[name]);
    }
    assert_eq!(
        serde_json::to_value(s.shadow_color).unwrap(),
        expected_style["shadow_color"]
    );
    assert_eq!(
        serde_json::to_value(&s.insertion).unwrap(),
        expected_style["insertion"]
    );
    if let Some(c) = &s.color {
        assert_eq!(expected_style["color"]["rgb"], c.rgb);
        assert_eq!(expected_style["color"]["serialized"], c.serialized);
    } else {
        assert!(expected_style["color"].is_null());
    }
    if let Some(font) = &s.font {
        assert_eq!(
            expected_style["font"],
            format!("{}:{}", font.namespace, font.path)
        );
    } else {
        assert!(expected_style["font"].is_null());
    }
    let click = s.click.as_ref().map(|c| match c {
        Click::OpenUrl(_) => "yf$f",
        Click::RunCommand(_) => "yf$g",
        Click::SuggestCommand(_) => "yf$i",
        Click::ChangePage(_) => "yf$b",
        Click::Copy(_) => "yf$c",
        Click::Custom { .. } => "yf$d",
        Click::Dialog(_) => "yf$h",
    });
    assert_eq!(
        serde_json::to_value(click).unwrap(),
        expected_style["click_class"]
    );
    let hover = s.hover.as_ref().map(|h| match h {
        Hover::Text(_) => "yo$e",
        Hover::Item(_) => "yo$d",
        Hover::Entity(_) => "yo$c",
    });
    assert_eq!(
        serde_json::to_value(hover).unwrap(),
        expected_style["hover_class"]
    );
    assert_eq!(
        text.siblings.len(),
        expected["siblings"].as_array().unwrap().len()
    );
    for (child, expected) in text
        .siblings
        .iter()
        .zip(expected["siblings"].as_array().unwrap())
    {
        describe(child, expected);
    }
}
#[test]
fn native_getter_fields_and_context_free_comparisons_match_original_text_values() {
    let facts = facts();
    let rows = facts["cases"].as_array().unwrap();
    let mut values = Vec::new();
    let mut accepted = 0;
    for row in rows {
        if row["accepted"] == false {
            values.push(None);
            continue;
        }
        let root =
            nbt::decode_unnamed_tag(&hex::decode(row["input_hex"].as_str().unwrap()).unwrap())
                .unwrap()
                .unwrap();
        let value = project(&root).unwrap_or_else(|e| panic!("{}: {e}", row["case"]));
        describe(&value, &row["fields"]);
        accepted += 1;
        values.push(Some(value));
    }
    assert_eq!((rows.len(), accepted), (182, 149));
    let mut comparisons = 0;
    let mut pending = 0;
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        match (a.modern_field_key(), b.modern_field_key()) {
            (Some(a), Some(b)) => {
                assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "pair {pair}");
                comparisons += 1;
            }
            _ => pending += 1,
        }
    }
    assert_eq!((comparisons, pending), (10585, 590));
}
#[test]
fn original_text_field_source_and_primitive_color_rules_are_bound() {
    use sha2::{Digest, Sha256};
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/text_core_source.json"
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
    assert_eq!(source["colors"], 16);
    assert_eq!(source["hex_utf16_digits"], 394);
}

#[test]
fn original_fuzzy_constructor_order_and_adverse_inputs_match() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/text_constructor_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let facts: Json = serde_json::from_slice(&bytes).unwrap();
    let mut values = Vec::new();
    let mut accepted = 0;
    let mut rejected = 0;
    let mut deferred_rejection = 0;
    for row in facts["cases"].as_array().unwrap() {
        let root =
            nbt::decode_unnamed_tag(&hex::decode(row["input_hex"].as_str().unwrap()).unwrap())
                .unwrap()
                .unwrap();
        let projected = project(&root);
        if row["accepted"] == false {
            match projected {
                Err(_) => rejected += 1,
                Ok(value) => {
                    assert!(
                        !value.dependencies().is_empty(),
                        "unvalidated native rejection {}",
                        row["case"]
                    );
                    assert!(value.modern_field_key().is_none());
                    deferred_rejection += 1;
                }
            }
            values.push(None);
        } else {
            let value = projected.unwrap_or_else(|e| panic!("{}: {e}", row["case"]));
            describe(&value, &row["fields"]);
            accepted += 1;
            values.push(Some(value));
        }
    }
    let mut comparisons = 0;
    let mut pending = 0;
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        match (a.modern_field_key(), b.modern_field_key()) {
            (Some(a), Some(b)) => {
                assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "pair {pair}");
                comparisons += 1;
            }
            _ => pending += 1,
        }
    }
    assert_eq!((accepted, rejected, deferred_rejection), (354, 64, 1));
    assert_eq!((comparisons, pending), (61425, 1410));
    assert_eq!(
        accepted + rejected + deferred_rejection,
        facts["cases"].as_array().unwrap().len()
    );
    assert_eq!(
        comparisons + pending,
        facts["pairs"].as_array().unwrap().len()
    );
}
#[test]
fn original_fuzzy_constructor_rules_and_sources_are_bound() {
    use sha2::{Digest, Sha256};
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/text_constructor_source.json"
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
    assert_eq!(source["cases"], 419);
}
