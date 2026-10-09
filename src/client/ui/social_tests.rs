//! Original native packet facts plus lifecycle/atomicity boundaries.
use super::*;
use crate::MinecraftVersion;
use crate::client::{GameMode, SessionStamp, ValueSource};
use serde_json::{Value, json};
fn cases(version: MinecraftVersion) -> Value {
    let data: Value =
        serde_json::from_str(include_str!("../../../data/client_api/social_packets.json")).unwrap();
    data["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == version.to_string())
        .unwrap()
        .clone()
}
fn bytes(row: &Value) -> Vec<u8> {
    row["payload_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn stamp(version: MinecraftVersion) -> SessionStamp {
    SessionStamp {
        version,
        connection_id: 77,
        world_generation: 2,
    }
}
fn text(value: &UiText, expected: &Value) {
    match value {
        UiText::LegacyJson { json } => assert_eq!(
            serde_json::from_str::<Value>(json).unwrap()["text"],
            *expected
        ),
        UiText::NativeNbt { bytes } => {
            assert_eq!(bytes[0], 8);
            let length = u16::from_be_bytes(bytes[1..3].try_into().unwrap()) as usize;
            assert_eq!(
                std::str::from_utf8(&bytes[3..3 + length]).unwrap(),
                expected.as_str().unwrap()
            );
            assert_eq!(length + 3, bytes.len());
        }
        UiText::Unavailable => panic!("received text unavailable"),
    }
}
fn params(params: &TeamParameters, expected: &Value) {
    text(&params.display, &expected["display"]);
    text(&params.prefix, &expected["prefix"]);
    text(&params.suffix, &expected["suffix"]);
    assert_eq!(
        params.friendly_flags,
        expected["friendly_flags"].as_u64().unwrap() as u8
    );
    assert_eq!(
        serde_json::to_value(params.color).unwrap(),
        expected["color"].as_str().unwrap().to_ascii_lowercase()
    );
    let visibility = expected["visibility"].as_str().unwrap();
    assert!(
        visibility == params.visibility.legacy_name()
            || visibility.to_ascii_lowercase() == serde_json::to_value(params.visibility).unwrap()
    );
    let collision = expected["collision"].as_str().unwrap();
    assert!(
        collision == params.collision.legacy_name()
            || collision.to_ascii_lowercase() == serde_json::to_value(params.collision).unwrap()
    );
}
fn team_seed(cases: &Value) -> &Value {
    cases["packets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["group"] == "team" && r["operation"] == 0)
        .unwrap()
}
fn roster_seed(cases: &Value, modern: bool) -> &Value {
    cases["packets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["group"] == "roster"
                && if modern {
                    r["flags"] == 255
                } else {
                    r["operation"] == 0
                }
        })
        .unwrap()
}
fn received(sequence: u64) -> ValueSource {
    ValueSource::Received { sequence }
}
#[test]
fn social_original_team_packets_match_complete_parameters_and_membership_origins() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let c = cases(version);
        for row in c["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["group"] == "team")
        {
            let mut ledger = teams::TeamLedger::default();
            let op = row["operation"].as_u64().unwrap();
            if op != 0 {
                ledger.receive(version, &bytes(team_seed(&c)), 1).unwrap();
            }
            ledger.receive(version, &bytes(row), 9).unwrap();
            let view = ledger.capture(stamp(version), 12);
            assert_eq!(view.last_update_sequence, Some(9));
            if op == 1 {
                assert!(view.teams.is_empty());
                continue;
            }
            let team = &view.teams[0];
            assert_eq!(team.name, row["name"]);
            assert_eq!(team.created_sequence, if op == 0 { 9 } else { 1 });
            if op == 0 || op == 2 {
                params(&team.parameters.value, &row["parameters"]);
                assert_eq!(team.parameters.source, received(9));
            } else {
                assert_eq!(team.parameters.source, received(1));
            }
            if op == 4 {
                assert!(team.members.is_empty());
                assert_eq!(team.last_members_update_sequence, Some(9));
            } else {
                let mut names = row["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap())
                    .collect::<Vec<_>>();
                if op == 2 {
                    names = vec!["OfflineHolder", "SocialProbe"];
                }
                names.sort();
                assert_eq!(
                    team.members
                        .iter()
                        .map(|r| r.value.as_str())
                        .collect::<Vec<_>>(),
                    names
                );
                assert!(
                    team.members
                        .iter()
                        .all(|r| r.source == received(if op == 2 { 1 } else { 9 }))
                );
                assert_eq!(
                    team.last_members_update_sequence,
                    Some(if op == 2 { 1 } else { 9 })
                );
            }
        }
    }
}
#[test]
fn social_original_roster_actions_preserve_optional_fields_signed_values_and_sources() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let modern = version == MinecraftVersion::Java1_21_11;
        let c = cases(version);
        let seed = roster_seed(&c, modern);
        for row in c["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["group"] == "roster")
        {
            let mut ledger = player_list::PlayerListLedger::default();
            ledger
                .receive(
                    version,
                    seed["packet_id"].as_i64().unwrap() as i32,
                    &bytes(seed),
                    1,
                )
                .unwrap();
            let before = ledger.capture(stamp(version), 1).entries.remove(0);
            ledger
                .receive(
                    version,
                    row["packet_id"].as_i64().unwrap() as i32,
                    &bytes(row),
                    9,
                )
                .unwrap();
            let view = ledger.capture(stamp(version), 12);
            assert_eq!(view.last_update_sequence, Some(9));
            if row["remove"] == true || row["operation"] == 4 {
                assert!(view.entries.is_empty());
                continue;
            }
            let expected = &row["entries"][0];
            let entry = &view.entries[0];
            let add = expected.get("profile").is_some();
            assert_eq!(serde_json::to_value(entry.uuid).unwrap(), expected["uuid"]);
            if add {
                assert_eq!(
                    serde_json::to_value(&entry.profile.value).unwrap(),
                    expected["profile"]
                );
                assert_eq!(entry.profile.source, received(9));
            } else {
                assert_eq!(entry.profile, before.profile);
            }
            if let Some(mode) = expected.get("game_mode") {
                let r = entry.game_mode.as_ref().unwrap();
                assert_eq!(r.source, received(9));
                assert_eq!(
                    r.value,
                    match mode.as_i64().unwrap() {
                        -1 => None,
                        0 => Some(GameMode::Survival),
                        1 => Some(GameMode::Creative),
                        2 => Some(GameMode::Adventure),
                        3 => Some(GameMode::Spectator),
                        _ => panic!("native mode differs"),
                    }
                );
            } else {
                assert_eq!(entry.game_mode, if add { None } else { before.game_mode });
            }
            if let Some(latency) = expected.get("latency") {
                let r = entry.latency.as_ref().unwrap();
                assert_eq!(r.source, received(9));
                assert_eq!(serde_json::to_value(r.value).unwrap(), *latency);
            } else {
                assert_eq!(entry.latency, if add { None } else { before.latency });
            }
            if let Some(display) = expected.get("display_name") {
                let r = entry.display_name.as_ref().unwrap();
                assert_eq!(r.source, received(9));
                if display.is_null() {
                    assert!(r.value.is_none());
                } else {
                    text(r.value.as_ref().unwrap(), display);
                }
            } else {
                assert_eq!(
                    entry.display_name,
                    if add { None } else { before.display_name }
                );
            }
            if let Some(listed) = expected.get("listed") {
                let r = entry.listing.as_ref().unwrap();
                assert_eq!(
                    r.value,
                    PlayerListing::Listed {
                        listed: listed.as_bool().unwrap()
                    }
                );
                assert_eq!(r.source, received(9));
            } else if !modern && add {
                let r = entry.listing.as_ref().unwrap();
                assert_eq!(r.value, PlayerListing::LegacyEntry);
                assert_eq!(r.source, received(9));
            } else {
                assert_eq!(entry.listing, if add { None } else { before.listing });
            }
            if let Some(order) = expected.get("list_order") {
                let r = entry.list_order.as_ref().unwrap();
                assert_eq!(r.source, received(9));
                assert_eq!(serde_json::to_value(r.value).unwrap(), *order);
            } else {
                assert_eq!(entry.list_order, if add { None } else { before.list_order });
            }
            if let Some(hat) = expected.get("show_hat") {
                let r = entry.show_hat.as_ref().unwrap();
                assert_eq!(r.source, received(9));
                assert_eq!(serde_json::to_value(r.value).unwrap(), *hat);
            } else {
                assert_eq!(entry.show_hat, if add { None } else { before.show_hat });
            }
            if let Some(chat) = expected.get("chat_session") {
                let r = entry.chat_session.as_ref().unwrap();
                assert_eq!(r.source, received(9));
                assert_eq!(serde_json::to_value(&r.value).unwrap(), *chat);
            } else {
                assert_eq!(
                    entry.chat_session,
                    if add { None } else { before.chat_session }
                );
            }
        }
        // No profile/permission is made from UUID-only updates.
        let mut empty = player_list::PlayerListLedger::default();
        for row in c["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["group"] == "roster" && r["entries"][0].get("profile").is_none())
        {
            empty
                .receive(
                    version,
                    row["packet_id"].as_i64().unwrap() as i32,
                    &bytes(row),
                    9,
                )
                .unwrap();
            assert!(empty.capture(stamp(version), 9).entries.is_empty());
        }
    }
}
#[test]
fn social_native_membership_rules_move_holders_and_fence_duplicate_and_wrong_leave() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let c = cases(version);
        let seed = bytes(team_seed(&c));
        let mut ledger = teams::TeamLedger::default();
        ledger.receive(version, &seed, 1).unwrap();
        let before = serde_json::to_value(ledger.capture(stamp(version), 1)).unwrap();
        let duplicate = ledger.receive(version, &seed, 3);
        if c["native_membership_rules"]["duplicate_create_policy"] == "refused" {
            assert!(duplicate.is_err());
            assert_eq!(
                serde_json::to_value(ledger.capture(stamp(version), 1)).unwrap(),
                before
            );
        } else {
            duplicate.unwrap();
            assert_eq!(ledger.capture(stamp(version), 3).teams[0].members.len(), 2);
        }
        let mut other = seed.clone();
        other[1..7].copy_from_slice(b"second");
        ledger.receive(version, &other, 5).unwrap();
        let view = ledger.capture(stamp(version), 5);
        let first = view.teams.iter().find(|t| t.name == "social").unwrap();
        let second = view.teams.iter().find(|t| t.name == "second").unwrap();
        assert!(first.members.is_empty());
        assert_eq!(first.last_members_update_sequence, Some(5));
        assert_eq!(second.members.len(), 2);
        let leave = c["packets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["group"] == "team" && r["operation"] == 4)
            .unwrap();
        let before = serde_json::to_value(ledger.capture(stamp(version), 5)).unwrap();
        assert!(ledger.receive(version, &bytes(leave), 7).is_err());
        assert_eq!(
            serde_json::to_value(ledger.capture(stamp(version), 5)).unwrap(),
            before
        );
        let mut correct = bytes(leave);
        correct[1..7].copy_from_slice(b"second");
        ledger.receive(version, &correct, 8).unwrap();
        assert!(
            ledger
                .capture(stamp(version), 8)
                .teams
                .iter()
                .all(|t| t.members.is_empty())
        );
        assert_eq!(
            c["native_membership_rules"]["wrong_team_leave_refused"],
            true
        );
    }
}
#[test]
fn social_original_truncations_and_trailing_fields_never_partially_update_either_ledger() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let c = cases(version);
        let modern = version == MinecraftVersion::Java1_21_11;
        for row in c["packets"].as_array().unwrap() {
            let mut teams = teams::TeamLedger::default();
            teams.receive(version, &bytes(team_seed(&c)), 1).unwrap();
            let mut roster = player_list::PlayerListLedger::default();
            let seed = roster_seed(&c, modern);
            roster
                .receive(
                    version,
                    seed["packet_id"].as_i64().unwrap() as i32,
                    &bytes(seed),
                    1,
                )
                .unwrap();
            let initial = json!({"teams":teams.capture(stamp(version),1),"roster":roster.capture(stamp(version),1)});
            let raw = bytes(row);
            for cut in 0..raw.len() + 1 {
                let mut payload = if cut == raw.len() {
                    raw.clone()
                } else {
                    raw[..cut].to_vec()
                };
                if cut == raw.len() {
                    payload.push(0);
                }
                if row["group"] == "team" {
                    assert!(teams.receive(version, &payload, 9).is_err());
                } else {
                    assert!(
                        roster
                            .receive(
                                version,
                                row["packet_id"].as_i64().unwrap() as i32,
                                &payload,
                                9
                            )
                            .is_err()
                    );
                }
                assert_eq!(
                    json!({"teams":teams.capture(stamp(version),1),"roster":roster.capture(stamp(version),1)}),
                    initial
                );
            }
        }
    }
}

pub(crate) fn bridge_cases(version: MinecraftVersion) -> Vec<Value> {
    let data = cases(version);
    let rows = data["packets"].as_array().unwrap();
    let mut result = Vec::new();
    for group in ["team", "roster"] {
        let remove = |r: &Value| {
            if group == "team" {
                r["operation"] == 1
            } else {
                r["remove"] == true || r["operation"] == 4
            }
        };
        result.extend(
            rows.iter()
                .filter(|r| r["group"] == group && !remove(r))
                .cloned(),
        );
        result.extend(
            rows.iter()
                .filter(|r| r["group"] == group && remove(r))
                .cloned(),
        );
    }
    result
}
pub(crate) fn bridge_bytes(row: &Value) -> Vec<u8> {
    bytes(row)
}
pub(crate) async fn assert_bridge(client: &crate::Client, row: &Value) {
    if row["group"] == "team" {
        let view = client.teams().await.unwrap();
        assert!(view.last_update_sequence.is_some());
        if row["operation"] == 1 {
            assert!(view.teams.is_empty());
            return;
        }
        let team = &view.teams[0];
        if let Some(expected) = row.get("parameters") {
            params(&team.parameters.value, expected);
        }
        if row["operation"] == 4 {
            assert!(team.members.is_empty());
        }
    } else {
        let view = client.player_list().await.unwrap();
        assert!(view.last_update_sequence.is_some());
        if row["remove"] == true || row["operation"] == 4 {
            assert!(view.entries.is_empty());
            return;
        }
        let expected = &row["entries"][0];
        let entry = &view.entries[0];
        if let Some(profile) = expected.get("profile") {
            assert_eq!(
                serde_json::to_value(&entry.profile.value).unwrap(),
                *profile
            );
        }
        if let Some(mode) = expected.get("game_mode") {
            assert_eq!(
                entry.game_mode.as_ref().unwrap().value,
                match mode.as_i64().unwrap() {
                    -1 => None,
                    0 => Some(GameMode::Survival),
                    1 => Some(GameMode::Creative),
                    2 => Some(GameMode::Adventure),
                    3 => Some(GameMode::Spectator),
                    _ => panic!(),
                }
            );
        }
        if let Some(latency) = expected.get("latency") {
            assert_eq!(
                entry.latency.as_ref().unwrap().value,
                latency.as_i64().unwrap() as i32
            );
        }
        if let Some(chat) = expected.get("chat_session") {
            assert_eq!(
                serde_json::to_value(&entry.chat_session.as_ref().unwrap().value).unwrap(),
                *chat
            );
        }
        if let Some(display) = expected.get("display_name") {
            let value = &entry.display_name.as_ref().unwrap().value;
            if display.is_null() {
                assert!(value.is_none());
            } else {
                text(value.as_ref().unwrap(), display);
            }
        }
        if let Some(order) = expected.get("list_order") {
            assert_eq!(
                entry.list_order.as_ref().unwrap().value,
                order.as_i64().unwrap() as i32
            );
        }
        if let Some(hat) = expected.get("show_hat") {
            assert_eq!(
                entry.show_hat.as_ref().unwrap().value,
                hat.as_bool().unwrap()
            );
        }
        if let Some(listed) = expected.get("listed") {
            assert_eq!(
                entry.listing.as_ref().unwrap().value.is_listed(),
                listed.as_bool().unwrap()
            );
        }
    }
}
