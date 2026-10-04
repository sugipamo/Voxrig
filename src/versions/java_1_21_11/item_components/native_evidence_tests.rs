//! Actual received native frames must agree with fresh common observations and RCON.
use super::{Reader, read_patch};
use serde_json::Value;

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| u8::try_from(v.as_u64().unwrap()).unwrap())
        .collect()
}
fn unhex(value: &Value) -> Vec<u8> {
    let s = value.as_str().unwrap();
    assert_eq!(s.len() % 2, 0);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn actual_complex_frames_preserve_fresh_nested_items_enchantments_and_books() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../data/client_api/item_data_complex_native_evidence.json"
    ))
    .unwrap();
    let runs = evidence["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["runtime_inputs"], runs[1]["runtime_inputs"]);
    for run in runs {
        assert_eq!(run["result"], "passed");
        assert_eq!(run["server_exit_code"], 0);
        assert_eq!(run["runtime_removed"], true);
        assert!(run["packet_trace"]["errors"].as_array().unwrap().is_empty());
        assert!(
            run["packet_trace"]["terminal_deliveries"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let legacy = run["version"] == "1.16.1";
        for mode in ["survival", "creative"] {
            let case = &run["item_data_observation"][mode];
            let received = &case["received"];
            let slot = &received["inventory"]["slots"][9];
            let item = &slot["value"]["item"];
            assert_eq!(received["game_mode"], mode);
            assert_eq!(received["session"], case["baseline"]["session"]);
            assert_eq!(slot["source"]["kind"], "received");
            assert!(
                slot["source"]["sequence"].as_u64().unwrap()
                    > case["baseline"]["receive_sequence"].as_u64().unwrap()
            );
            assert_eq!(item["name"], "minecraft:stone");
            assert_eq!(item["count"], case["count"]);
            assert_eq!(case["position_before"], case["position_after"]);
            assert_eq!(case["rotation_before"], case["rotation_after"]);
            assert!(case["frames"].as_array().unwrap().iter().all(|f| {
                f["direction"] != "serverbound"
                    || ![
                        if legacy { 0x09 } else { 0x11 },
                        if legacy { 0x0a } else { 0x12 },
                        if legacy { 0x27 } else { 0x37 },
                    ]
                    .contains(&f["packet_id"].as_u64().unwrap())
            }));
            if legacy {
                assert_eq!(item["data"]["kind"], "legacy_nbt");
                continue;
            }
            assert!(
                case["native_nested_marker"]
                    .as_str()
                    .unwrap()
                    .ends_with(": 19")
            );
            assert!(
                case["native_nested_count"]
                    .as_str()
                    .unwrap()
                    .ends_with(": 2")
            );
            assert!(
                case["native_enchantment"]
                    .as_str()
                    .unwrap()
                    .ends_with(": 2")
            );
            assert!(
                case["native_book"]
                    .as_str()
                    .unwrap()
                    .contains("ComplexProbe")
            );
            let patch = &item["data"]["patch"];
            let added = patch["added"].as_array().unwrap();
            assert_eq!(added.len(), 5);
            assert!(patch["removed"].as_array().unwrap().is_empty());
            let bundle = added
                .iter()
                .find(|c| c["definition"]["name"] == "minecraft:bundle_contents")
                .unwrap();
            let nested = bytes(&bundle["bytes"]);
            let mut marker = vec![3, 0, 17];
            marker.extend_from_slice(b"VoxrigNestedProbe");
            marker.extend_from_slice(&19_i32.to_be_bytes());
            assert!(nested.windows(marker.len()).any(|w| w == marker));
            let frame = case["frames"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| {
                    f["direction"] == "clientbound"
                        && f["packet_id"] == 0x14
                        && f["body_length"].as_u64().unwrap() > 100
                })
                .unwrap();
            let body = unhex(&frame["body_hex"]);
            let mut r = Reader::new(&body);
            assert_eq!(r.u8().unwrap(), 0);
            r.varint().unwrap();
            assert_eq!(r.u16().unwrap(), 9);
            assert_eq!(r.varint().unwrap(), item["count"].as_i64().unwrap() as i32);
            assert_eq!(r.varint().unwrap(), 1);
            assert_eq!(
                serde_json::to_value(read_patch(&mut r).unwrap().unwrap()).unwrap(),
                *patch
            );
            r.end().unwrap();
        }
    }
    let earlier = evidence["earlier_attempts"].as_array().unwrap();
    assert_eq!(earlier.len(), 2);
    assert_eq!(earlier[1]["result"], "failed");
    assert_eq!(earlier[1]["server_exit_code"], 0);
    assert_eq!(earlier[1]["runtime_removed"], true);
    assert_eq!(
        evidence["fixture_driver_failure_source_sha256"],
        earlier[1]["runtime_inputs"]["source_sha256"]["scripts/run_common_native.py"]
    );
}

#[test]
fn named_item_native_frames_match_fresh_common_receipts_and_independent_fields() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../data/client_api/item_data_native_evidence.json"
    ))
    .unwrap();
    let runs = evidence["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["runtime_inputs"], runs[1]["runtime_inputs"]);
    for run in runs {
        let legacy = run["version"] == "1.16.1";
        assert_eq!(run["result"], "passed");
        assert_eq!(run["server_exit_code"], 0);
        assert_eq!(run["runtime_removed"], true);
        assert!(run["packet_trace"]["errors"].as_array().unwrap().is_empty());
        assert!(
            run["packet_trace"]["terminal_deliveries"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for mode in ["survival", "creative"] {
            let case = &run["item_data_observation"][mode];
            let received = &case["received"];
            assert_eq!(received["game_mode"], mode);
            assert_eq!(received["session"], case["baseline"]["session"]);
            assert_eq!(case["position_before"], case["position_after"]);
            assert_eq!(case["rotation_before"], case["rotation_after"]);
            let observed = &received["inventory"]["slots"][9];
            assert_eq!(observed["source"]["kind"], "received");
            assert!(
                observed["source"]["sequence"].as_u64().unwrap()
                    > case["baseline"]["receive_sequence"].as_u64().unwrap()
            );
            let item = &observed["value"]["item"];
            assert_eq!(item["name"], "minecraft:stone");
            assert_eq!(item["count"], case["count"]);
            assert_eq!(item["id"]["kind"], "Item");
            assert_eq!(item["id"]["version"], received["session"]["version"]);
            let marker = case["marker"].as_u64().unwrap();
            assert!(
                case["native_marker"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!(": {marker}"))
            );
            let name = case["name"].as_str().unwrap();
            assert!(case["native_name"].as_str().unwrap().contains(name));
            let native_inventory = if legacy {
                &case["native_inventory"]
            } else {
                &case["native_inventory"]["inventory"]
            };
            let inventory = native_inventory.as_str().unwrap();
            assert!(inventory.contains("minecraft:stone") && inventory.contains("Slot: 9b"));
            let count = case["count"].as_u64().unwrap();
            assert!(inventory.contains(&if legacy {
                format!("Count: {count}b")
            } else {
                format!("count: {count}")
            }));
            let mut marker_bytes = vec![3, 0, 11];
            marker_bytes.extend_from_slice(b"VoxrigProbe");
            marker_bytes.extend_from_slice(&(marker as i32).to_be_bytes());
            let payload = if legacy {
                assert_eq!(item["data"]["kind"], "legacy_nbt");
                bytes(&item["data"]["bytes"])
            } else {
                assert_eq!(item["data"]["kind"], "modern_components");
                let patch = &item["data"]["patch"];
                assert!(patch["removed"].as_array().unwrap().is_empty());
                let added = patch["added"].as_array().unwrap();
                assert_eq!(added.len(), 2);
                for (component, id, label) in
                    [(&added[0], 0, "custom_data"), (&added[1], 6, "custom_name")]
                {
                    assert_eq!(component["definition"]["id"]["value"], id);
                    assert_eq!(component["definition"]["id"]["kind"], "ItemComponent");
                    assert_eq!(
                        component["definition"]["id"]["version"],
                        item["id"]["version"]
                    );
                    assert_eq!(
                        component["definition"]["name"],
                        format!("minecraft:{label}")
                    );
                }
                assert!(
                    bytes(&added[1]["bytes"])
                        .windows(name.len())
                        .any(|w| w == name.as_bytes())
                );
                bytes(&added[0]["bytes"])
            };
            assert!(
                payload
                    .windows(marker_bytes.len())
                    .any(|w| w == marker_bytes)
            );
            if legacy {
                assert!(payload.windows(name.len()).any(|w| w == name.as_bytes()));
            }
            let frames = case["frames"].as_array().unwrap();
            assert!(frames.iter().all(|f| {
                f["direction"] != "serverbound"
                    || ![
                        if legacy { 0x09 } else { 0x11 },
                        if legacy { 0x0a } else { 0x12 },
                        if legacy { 0x27 } else { 0x37 },
                    ]
                    .contains(&f["packet_id"].as_u64().unwrap())
            }));
            if !legacy {
                // Decode the actual SET_SLOT, not an item fabricated from its observation.
                let frame = frames
                    .iter()
                    .find(|f| {
                        f["direction"] == "clientbound"
                            && f["packet_id"] == 0x14
                            && f["body_length"].as_u64().unwrap() > 20
                    })
                    .unwrap();
                let body = unhex(&frame["body_hex"]);
                let mut r = Reader::new(&body);
                assert_eq!(r.u8().unwrap(), 0);
                let revision = r.varint().unwrap();
                assert_eq!(
                    revision,
                    received["inventory"]["player_screen_revision"]["value"]
                        .as_i64()
                        .unwrap() as i32
                );
                assert_eq!(r.u16().unwrap(), 9);
                assert_eq!(r.varint().unwrap(), count as i32);
                assert_eq!(
                    r.varint().unwrap(),
                    item["id"]["value"].as_i64().unwrap() as i32
                );
                assert_eq!(
                    serde_json::to_value(read_patch(&mut r).unwrap().unwrap()).unwrap(),
                    item["data"]["patch"]
                );
                r.end().unwrap();
            }
        }
    }
}

#[test]
fn native_provenance_preserves_failed_transport_attempts_and_actual_build_inputs() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../data/client_api/item_data_native_evidence.json"
    ))
    .unwrap();
    let attempts = evidence["earlier_attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 4);
    assert_eq!(
        attempts.iter().filter(|a| a["result"] == "failed").count(),
        2
    );
    assert!(
        attempts
            .iter()
            .all(|a| a["server_exit_code"] == 0 && a["runtime_removed"] == true)
    );
    let diagnostic = &attempts[3]["packet_trace"]["error_contexts"];
    assert!(
        diagnostic
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["direction"] == "serverbound"
                && c["expected_body_length"] == 6
                && c["received_body_length"] == 0)
    );
    for run in evidence["runs"].as_array().unwrap().iter().chain(attempts) {
        for sha in run["raw_output_sha256"]
            .as_object()
            .unwrap()
            .values()
            .chain(
                run["runtime_inputs"]["source_sha256"]
                    .as_object()
                    .unwrap()
                    .values(),
            )
            .chain(std::iter::once(
                &run["runtime_inputs"]["consumer_binary_sha256"],
            ))
        {
            let sha = sha.as_str().unwrap();
            assert_eq!(sha.len(), 64);
            assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert_eq!(
            run["runtime_inputs"]["source_sha256"]["examples/common_native_probe.rs"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
    }
}
