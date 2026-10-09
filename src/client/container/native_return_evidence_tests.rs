//! Frozen network facts from original vanilla servers, separate from primitive predictions.
use serde_json::Value;
#[test]
fn cursor_return_native_cases_have_fresh_actual_steps_one_close_and_independent_counts() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../data/client_api/cursor_return_native_evidence.json"
    ))
    .unwrap();
    assert_eq!(evidence["status"], "passed");
    let runs = evidence["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0]["runtime_inputs"]["consumer_binary_sha256"],
        runs[1]["runtime_inputs"]["consumer_binary_sha256"]
    );
    for run in runs {
        let legacy = run["version"] == "1.16.1";
        assert_eq!(run["result"], "passed");
        assert_eq!(run["server_exit_code"], 0);
        assert_eq!(run["runtime_removed"], true);
        assert!(run["packet_trace"]["errors"].as_array().unwrap().is_empty());
        let cases = run["cursor_return_close"].as_object().unwrap();
        assert_eq!(cases.len(), if legacy { 4 } else { 6 });
        for mode in ["survival", "creative"] {
            for item in if legacy {
                vec!["stone", "diamond_helmet"]
            } else {
                vec!["stone", "diamond_helmet", "bundle"]
            } {
                let key = format!("{mode}/minecraft:{item}");
                let case = &cases[&key];
                assert_eq!(case["position_before"], case["position_after"]);
                assert_eq!(case["rotation_before"], case["rotation_after"]);
                assert_eq!(case["no_drop"], "Test passed");
                assert!(
                    case["barrel_empty_closed"]
                        .as_str()
                        .unwrap()
                        .ends_with("[]")
                );
                let inventory = if legacy {
                    &case["inventory_after"]
                } else {
                    &case["inventory_after"]["inventory"]
                };
                let inventory = inventory.as_str().unwrap();
                assert!(inventory.contains(&format!("minecraft:{item}")));
                assert!(inventory.contains("Slot: 9b"));
                if item == "stone" {
                    assert!(inventory.contains("Slot: 10b"));
                    assert!(inventory.contains(if legacy { "Count: 64b" } else { "count: 64" }));
                    assert!(inventory.contains(if legacy { "Count: 4b" } else { "count: 4" }));
                }
                let close = &case["close"];
                assert_eq!(close["mode"], mode);
                assert_eq!(close["stage"], "dispatched");
                assert_eq!(close["dispatched"], true);
                assert!(close["requires_inspection"].is_null());
                assert!(close["server_close_sequence"].is_null());
                assert_eq!(
                    close["initial"]["inventory"]["cursor"],
                    case["held"]["cursor_receipt"]
                );
                let steps = close["return_steps"].as_array().unwrap();
                assert_eq!(steps.len(), if item == "stone" { 2 } else { 1 });
                assert_eq!(steps.len(), close["return_plan"].as_array().unwrap().len());
                for step in steps {
                    assert_eq!(step["id"]["close"], close["id"]);
                    assert_eq!(step["stage"], "observed_clicked");
                    assert_eq!(step["send"]["dispatched"], true);
                    assert!(step["requires_inspection"].is_null());
                    let boundary = step["send"]["after_sequence"].as_u64().unwrap();
                    for (before, receipt, predicted) in [
                        ("source_before", "source_receipt", "source"),
                        ("cursor_before", "cursor_receipt", "cursor"),
                    ] {
                        assert_eq!(step[before]["source"]["kind"], "received");
                        assert_eq!(step[receipt]["source"]["kind"], "received");
                        assert!(step[receipt]["source"]["sequence"].as_u64().unwrap() > boundary);
                        assert_eq!(
                            step[receipt]["value"],
                            step["prediction"][predicted]["value"]
                        );
                        assert_eq!(step["prediction"][predicted]["source"]["kind"], "predicted");
                    }
                    if legacy {
                        assert_eq!(step["legacy_reply"]["accepted"], false);
                        assert_eq!(
                            step["legacy_reply"]["action"],
                            step["send"]["legacy_action"]
                        );
                        assert!(
                            step["legacy_reply"]["receive_sequence"].as_u64().unwrap() > boundary
                        );
                    } else {
                        assert!(step["legacy_reply"].is_null());
                    }
                }
                assert_eq!(
                    steps.last().unwrap()["cursor_receipt"]["value"]["kind"],
                    "empty"
                );
                let frames = case["frames"].as_array().unwrap();
                assert!(
                    frames
                        .iter()
                        .all(|f| f["direction"] == "serverbound" && f["phase"] == "play")
                );
                let clicks: Vec<_> = frames
                    .iter()
                    .filter(|f| f["packet_id"] == if legacy { 0x09 } else { 0x11 })
                    .collect();
                assert_eq!(clicks.len(), steps.len());
                let closes: Vec<_> = frames
                    .iter()
                    .filter(|f| f["packet_id"] == if legacy { 0x0a } else { 0x12 })
                    .collect();
                assert_eq!(closes.len(), 1);
                let window = close["id"]["screen"]["window"].as_u64().unwrap();
                assert_eq!(closes[0]["body_hex"], format!("{window:02x}"));
                assert!(
                    closes[0]["ordinal"].as_u64().unwrap()
                        > clicks.last().unwrap()["ordinal"].as_u64().unwrap()
                );
            }
        }
    }
    assert!(
        evidence["earlier_attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["result"] == "failed")
    );
    assert!(
        evidence["earlier_attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["server_exit_code"] == 0 && a["runtime_removed"] == true)
    );
}
