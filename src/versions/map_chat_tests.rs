//! Adapter regressions against frozen pre-refactor parsers, including error chains.
use super::{java_1_16_1 as legacy, java_1_21_11 as modern};
use crate::client::{SessionStamp, ValueSource, chat::*, maps::*, ui::UiText};
use crate::protocol::{put_string, put_varint};
use crate::{Error, MinecraftVersion};

mod original_chat;

fn stamp(modern: bool) -> SessionStamp {
    SessionStamp {
        version: if modern {
            MinecraftVersion::Java1_21_11
        } else {
            MinecraftVersion::Java1_16_1
        },
        connection_id: 42,
        world_generation: 7,
    }
}
fn error(error: Error) -> (crate::ErrorKind, String) {
    (error.kind(), format!("{:#}", error.diagnostic()))
}
fn mutations(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut cases = (0..bytes.len())
        .map(|n| bytes[..n].to_vec())
        .collect::<Vec<_>>();
    cases.push(bytes.to_vec());
    cases.push([bytes, &[0, 255]].concat());
    for i in 0..bytes.len() {
        for value in [0, 1, 2, 127, 128, 255] {
            let mut changed = bytes.to_vec();
            changed[i] = value;
            cases.push(changed);
        }
    }
    cases
}
fn map_packet(modern: bool) -> Vec<u8> {
    // Noncanonical true bytes, signed coordinates, full rotation byte, raw text.
    let mut p = vec![7, 255, 2];
    if !modern {
        p.push(255);
    }
    if modern {
        p.push(2);
    }
    p.extend([1, 26, 255, 128, 255, 2]);
    if modern {
        p.extend([8, 0, 3, b'm', b'a', b'p']);
    } else {
        put_string(&mut p, " {\"text\":\"map\"} ");
    }
    p.extend([2, 1, 3, 4, 2, 0, 19]);
    p
}
fn map_decode(modern: bool, p: &[u8]) -> anyhow::Result<super::map_decode::MapUpdate> {
    if modern {
        modern::map_decode::decode_map(p)
    } else {
        legacy::map_decode::decode_map(p)
    }
}
fn map_snapshot(update: super::map_decode::MapUpdate, modern: bool) -> serde_json::Value {
    let id = update.id;
    let mut ledger = MapLedger::default();
    ledger.apply(update, 13).unwrap();
    let observation = ledger.capture(id, stamp(modern), 17).unwrap();
    serde_json::json!({
        "fields": observation,
        "pixels": (0..128).flat_map(|y| (0..128).map(move |x| (x,y)))
            .filter_map(|(x,y)| observation.pixel(x,y).map(|p| (x,y,p)))
            .collect::<Vec<_>>()
    })
}

#[test]
fn maps_match_original_observations_errors_and_native_projection() {
    for modern in [false, true] {
        for p in mutations(&map_packet(modern)) {
            let actual = map_decode(modern, &p).map_err(Error::from);
            let original =
                super::map_decode::original_decode::decode(&p, modern).map_err(Error::from);
            match (actual, original) {
                (Ok(actual), Ok(original)) => {
                    if !modern {
                        assert_eq!(
                            legacy::map::project_map_update(actual.clone()),
                            legacy::map::parse_map_update(&p).unwrap()
                        );
                    }
                    assert_eq!(
                        map_snapshot(actual, modern),
                        map_snapshot(original, modern),
                        "{modern} {p:?}"
                    );
                }
                (Err(actual), Err(original)) => {
                    assert_eq!(error(actual), error(original), "{modern} {p:?}")
                }
                _ => panic!("map acceptance changed: {modern} {p:?}"),
            }
        }
    }
}

#[test]
fn map_projection_preserves_coverage_raw_names_sources_and_capacity() {
    for modern in [false, true] {
        let mut common = MapLedger::default();
        let update = map_decode(modern, &map_packet(modern)).unwrap();
        common.apply_with_limit(update.clone(), 11, 1).unwrap();
        let first = common.capture(7, stamp(modern), 12).unwrap();
        assert_eq!(first.map.first_sequence, 11);
        assert_eq!(first.pixel(3, 4).unwrap().value, 0);
        assert!(first.pixel(0, 0).is_none());
        let icon = &first.icons.as_ref().unwrap().value[0];
        assert_eq!(
            (icon.x, icon.y, icon.encoded_rotation, icon.rotation()),
            (-1, -128, 255, 15)
        );
        assert_eq!(
            icon.name,
            Some(if modern {
                UiText::NativeNbt {
                    bytes: vec![8, 0, 3, b'm', b'a', b'p'],
                }
            } else {
                UiText::LegacyJson {
                    json: " {\"text\":\"map\"} ".into(),
                }
            })
        );
        let mut other = update.clone();
        other.id = 8;
        assert_eq!(
            common
                .apply_with_limit(other, 13, 1)
                .unwrap_err()
                .to_string(),
            "common map cache limit exceeded"
        );
        assert!(common.capture(8, stamp(modern), 14).is_none());
        assert_eq!(
            common.capture(7, stamp(modern), 14).unwrap().scale.source,
            ValueSource::Received { sequence: 11 }
        );
        assert_eq!(
            common.apply(update.clone(), 0).unwrap_err().to_string(),
            "map update has no original receipt sequence"
        );
        if !modern {
            let mut native = legacy::map::MapStore::default();
            native
                .apply(&legacy::map::project_map_update(update), 1)
                .unwrap();
            assert_eq!(native.maps[&7].colors[4 * 128 + 3], 0);
            assert_eq!(native.maps[&7].colors[4 * 128 + 4], 19);
            // Native zeros and common missing coverage remain distinct.
            assert_eq!(native.maps[&7].colors[0], 0);
        }
    }
}
#[test]
fn map_wire_budgets_and_validation_order_match_original() {
    for modern in [false, true] {
        let header = if modern {
            vec![7, 0, 0, 1]
        } else {
            vec![7, 0, 0, 0]
        };
        let mut cases = vec![vec![0; (1 << 20) + 128 * 128 + 65_537]];
        for count in [-1, 4097] {
            let mut p = header.clone();
            put_varint(&mut p, count);
            cases.push(p);
        }
        let mut names = header;
        put_varint(&mut names, 33);
        for _ in 0..33 {
            names.extend([0, 0, 0, 0, 1]);
            let text = "x".repeat(32767);
            if modern {
                names.extend(nbt(&text));
            } else {
                put_string(&mut names, &text);
            }
        }
        names.push(0);
        cases.push(names);
        for p in cases {
            let actual = map_decode(modern, &p).err().unwrap();
            let original = super::map_decode::original_decode::decode(&p, modern)
                .err()
                .unwrap();
            assert_eq!(error(actual.into()), error(original.into()));
        }
        let mut bad = map_packet(modern);
        bad.push(0);
        let mut ledger = MapLedger::default();
        // Wire errors still precede receipt/capacity validation.
        let result = map_decode(modern, &bad).and_then(|u| ledger.apply_with_limit(u, 0, 0));
        assert_eq!(
            result.unwrap_err().to_string(),
            "unexpected trailing packet data: 1 bytes"
        );
        assert!(ledger.capture(7, stamp(modern), 1).is_none());
    }
}

fn legacy_chat(json: &str, position: u8, sender: [u8; 16]) -> Vec<u8> {
    let mut p = vec![];
    put_string(&mut p, json);
    p.push(position);
    p.extend(sender);
    p
}
fn nbt(text: &str) -> Vec<u8> {
    let mut p = vec![8];
    p.extend((text.len() as u16).to_be_bytes());
    p.extend(text.as_bytes());
    p
}
fn player_chat(unsigned: bool) -> Vec<u8> {
    let mut p = vec![0];
    p.extend([7; 16]);
    p.extend([0, 1]);
    p.extend([3; 256]);
    put_string(&mut p, "plain body");
    p.extend([0; 16]); // timestamp and salt
    p.extend([1, 0]);
    p.extend([4; 256]); // one previous signature
    p.push(u8::from(unsigned));
    if unsigned {
        p.extend(nbt("decorated"));
    }
    p.extend([2, 1]);
    p.extend([0; 8]); // filter bitset
    p.push(1);
    p.extend(nbt("sender"));
    p.push(1);
    p.extend(nbt("target"));
    p
}

#[test]
fn chat_matches_original_raw_text_ordinals_and_malformed_policy() {
    for position in [0, 1, 2, 9, 255] {
        for p in mutations(&legacy_chat(
            " {\"text\":\"raw\\ntext\"} ",
            position,
            [7; 16],
        )) {
            let actual = legacy::chat_decode::decode_chat(&p, 23);
            let original = original_chat::legacy(&p, 23);
            match (actual, original) {
                (Ok((message, native)), Ok(original)) => {
                    assert_eq!(message, original);
                    assert_eq!(native, legacy::chat::parse_chat(&p).unwrap());
                }
                (Err(actual), Err(original)) => assert_eq!(error(actual), error(original), "{p:?}"),
                _ => panic!("legacy chat acceptance changed: {p:?}"),
            }
        }
    }
    use modern::ClientboundIds as ids;
    let system = [nbt("raw system"), vec![1]].concat();
    let profileless = [nbt("emote"), vec![1], nbt("name"), vec![0]].concat();
    for (id, packet) in [
        (ids::SYSTEM_CHAT, system),
        (ids::PLAYER_CHAT, player_chat(false)),
        (ids::PLAYER_CHAT, player_chat(true)),
        (ids::PROFILELESS_CHAT, profileless),
        (-1, vec![]),
    ] {
        for p in mutations(&packet) {
            let actual = modern::chat_decode::decode_chat(id, &p, 29);
            let original = original_chat::modern(id, &p, 29);
            match (actual, original) {
                (Ok(actual), Ok(original)) => assert_eq!(actual, original, "{id} {p:?}"),
                (Err(actual), Err(original)) => {
                    assert_eq!(error(actual), error(original), "{id} {p:?}")
                }
                _ => panic!("modern chat acceptance changed: {id} {p:?}"),
            }
        }
    }
}

#[test]
fn chat_retention_preserves_count_byte_limits_and_gap_boundaries_for_both_versions() {
    for modern in [false, true] {
        for text in ["x".to_string(), "x".repeat(32767)] {
            let packet = if modern {
                [nbt(&text), vec![0]].concat()
            } else {
                legacy_chat(&text, 1, [0; 16])
            };
            let mut ledger = ChatLedger::default();
            let mut original = ChatLedger::default();
            for sequence in 1..=270 {
                let message = if modern {
                    modern::chat_decode::decode_chat(
                        modern::ClientboundIds::SYSTEM_CHAT,
                        &packet,
                        sequence,
                    )
                    .unwrap()
                } else {
                    legacy::chat_decode::decode_chat(&packet, sequence)
                        .unwrap()
                        .0
                };
                let expected = if modern {
                    original_chat::modern(modern::ClientboundIds::SYSTEM_CHAT, &packet, sequence)
                        .unwrap()
                } else {
                    original_chat::legacy(&packet, sequence).unwrap()
                };
                ledger.apply(message);
                original.apply(expected);
            }
            let retained = if text.len() == 1 {
                MAX_RETAINED_CHAT
            } else {
                (4 << 20) / (64 + if modern { text.len() + 3 } else { text.len() })
            };
            let dropped = 270 - retained as u64;
            for cursor in [0, dropped - 1, dropped, 270, 271] {
                let actual = ledger.after(cursor, stamp(modern), 270);
                let expected = original.after(cursor, stamp(modern), 270);
                match (actual, expected) {
                    (Ok(actual), Ok(expected)) => {
                        assert_eq!(actual.messages, expected.messages);
                        assert_eq!(actual.receive_sequence, 270);
                        assert_eq!(actual.session, stamp(modern));
                        if cursor == dropped {
                            assert_eq!(actual.messages.len(), retained);
                            assert_eq!(actual.messages[0].receive_sequence, dropped + 1);
                        }
                    }
                    (Err(actual), Err(expected)) => assert_eq!(error(actual), error(expected)),
                    _ => panic!("chat retention changed"),
                }
            }
            assert_eq!(
                ledger
                    .after(dropped - 1, stamp(modern), 270)
                    .unwrap_err()
                    .to_string(),
                "chat history before the cursor was truncated"
            );
            assert_eq!(
                ledger
                    .after(271, stamp(modern), 270)
                    .unwrap_err()
                    .to_string(),
                "chat cursor is ahead of the receive state"
            );
        }
    }
}
