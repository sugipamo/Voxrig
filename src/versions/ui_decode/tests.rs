//! Regressions for the adapter boundary and decode-once native projections.
use super::*;
use crate::client::ui::{self, UiText};
use crate::client::{SessionStamp, ValueSource};
use crate::protocol::{put_string, put_varint};
use crate::versions::java_1_16_1::{chat, ui::UiState};
use serde_json::Value;

fn fixtures(name: &str) -> Value {
    serde_json::from_str(match name {
        "display" => include_str!("../../../data/client_api/display_packets.json"),
        "boss" => include_str!("../../../data/client_api/boss_bar_packets.json"),
        "social" => include_str!("../../../data/client_api/social_packets.json"),
        _ => unreachable!(),
    })
    .unwrap()
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
        connection_id: 91,
        world_generation: 3,
    }
}
fn legacy_rows(data: &Value) -> &Vec<Value> {
    data["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == "1.16.1")
        .unwrap()["packets"]
        .as_array()
        .unwrap()
}

#[test]
fn legacy_display_and_boss_projections_match_original_native_parsers() {
    let version = MinecraftVersion::Java1_16_1;
    let mut original = UiState::default();
    let mut projected = UiState::default();
    let mut common = ui::display::DisplayLedger::default();
    for row in legacy_rows(&fixtures("display")) {
        let payload = bytes(row);
        let id = row["packet_id"].as_i64().unwrap() as i32;
        let update = decode_display(version, id, &payload).unwrap();
        common.apply(update.clone(), 13, 3).unwrap();
        projected.project_display(update);
        match row["group"].as_str().unwrap() {
            "title" => original.apply_title(&payload).unwrap(),
            "tab" => original.apply_tab(&payload).unwrap(),
            "border" => original.apply_border(&payload).unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(projected, original, "{row}");
    }
    let data = fixtures("boss");
    let rows = legacy_rows(&data);
    for known in [false, true] {
        for row in rows.iter().filter(|r| known || r["operation"] != 0) {
            let mut original = UiState::default();
            let mut projected = UiState::default();
            let mut common = ui::boss_bar::BossBarLedger::default();
            if known {
                original.apply_boss_bar(&bytes(&rows[0])).unwrap();
                let (uuid, update) = decode_boss_bar(version, &bytes(&rows[0])).unwrap();
                projected.project_boss_bar(uuid, update.clone());
                common.apply(uuid, update, 2).unwrap();
            }
            let payload = bytes(row);
            let (uuid, update) = decode_boss_bar(version, &payload).unwrap();
            common.apply(uuid, update.clone(), 17).unwrap();
            projected.project_boss_bar(uuid, update);
            original.apply_boss_bar(&payload).unwrap();
            assert_eq!(projected, original, "{row}");
            assert_eq!(
                common.capture(stamp(version), 20).last_update_sequence,
                Some(17)
            );
        }
    }
}

#[test]
fn legacy_player_projections_preserve_native_fields_event_order_and_empty_actions() {
    let version = MinecraftVersion::Java1_16_1;
    let data = fixtures("social");
    let rows = legacy_rows(&data);
    let seed = rows
        .iter()
        .find(|r| r["group"] == "roster" && r["operation"] == 0)
        .unwrap();
    for known in [false, true] {
        for row in rows.iter().filter(|r| r["group"] == "roster") {
            let mut original = chat::PlayerList::default();
            if known {
                chat::apply_player_info(&mut original, &bytes(seed)).unwrap();
            }
            let mut projected = original.clone();
            let mut common = ui::player_list::PlayerListLedger::default();
            if known {
                common.receive(version, 0x33, &bytes(seed), 2).unwrap();
            }
            let payload = bytes(row);
            let packet = decode_player_list(version, 0x33, &payload).unwrap();
            common.apply(packet.clone(), 19).unwrap();
            let event = chat::project_player_info(&mut projected, packet);
            let old_event = chat::apply_player_info(&mut original, &payload).unwrap();
            assert_eq!(event, old_event);
            assert_eq!(projected, original, "{row}");
        }
    }
    for action in 0..=4 {
        let payload = [action, 0];
        let packet = decode_player_list(version, 0x33, &payload).unwrap();
        let mut list = chat::PlayerList::default();
        assert_eq!(
            chat::project_player_info(&mut list, packet),
            (i32::from(action), vec![])
        );
        assert!(list.entries.is_empty());
    }
}

#[test]
fn legacy_scoreboard_and_team_projections_preserve_native_removals_and_membership() {
    let version = MinecraftVersion::Java1_16_1;
    let mut original = UiState::default();
    let mut projected = UiState::default();
    let mut common = ui::ScoreboardLedger::default();
    let mut objective = vec![];
    put_string(&mut objective, "kills");
    objective.push(0);
    put_string(&mut objective, " {\"text\":\"Kills\"} ");
    put_varint(&mut objective, 1);
    let mut display = vec![18];
    put_string(&mut display, "kills");
    let mut score = vec![];
    put_string(&mut score, "Alice");
    score.push(0);
    put_string(&mut score, "kills");
    put_varint(&mut score, -7);
    let mut reset = vec![];
    put_string(&mut reset, "Alice");
    reset.push(1);
    put_string(&mut reset, "");
    let mut remove = vec![];
    put_string(&mut remove, "kills");
    remove.push(1);
    for (id, payload) in [
        (0x4a, objective),
        (0x43, display),
        (0x4d, score.clone()),
        (0x4d, reset),
        (0x4d, score),
        (0x4a, remove),
    ] {
        let update = decode_scoreboard(version, id, &payload).unwrap();
        common.apply(update.clone(), 23).unwrap();
        projected.project_scoreboard(update);
        match id {
            0x4a => original.apply_objective(&payload).unwrap(),
            0x43 => original.apply_display(&payload).unwrap(),
            _ => original.apply_score(&payload).unwrap(),
        }
        assert_eq!(projected, original);
    }
    let data = fixtures("social");
    let rows = legacy_rows(&data);
    let seed = rows
        .iter()
        .find(|r| r["group"] == "team" && r["operation"] == 0)
        .unwrap();
    for row in rows.iter().filter(|r| r["group"] == "team") {
        let mut original = UiState::default();
        let mut common = ui::teams::TeamLedger::default();
        if row["operation"] != 0 {
            original.apply_team(&bytes(seed)).unwrap();
            common.receive(version, &bytes(seed), 1).unwrap();
        }
        let mut projected = original.clone();
        let payload = bytes(row);
        let packet = decode_team(version, &payload).unwrap();
        common.apply_packet(version, packet.clone(), 29).unwrap();
        projected.project_team(packet).unwrap();
        original.apply_team(&payload).unwrap();
        assert_eq!(projected, original);
    }
}

fn error(result: Result<impl Sized>) -> (crate::ErrorKind, String, String) {
    let e = result.err().expect("expected decode failure");
    (e.kind(), e.to_string(), format!("{:#}", e.diagnostic()))
}
#[test]
fn both_layouts_keep_error_kinds_text_and_validation_order() {
    use crate::ErrorKind::{InvalidInput, Protocol};
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let modern = version == MinecraftVersion::Java1_21_11;
        let title = if modern {
            crate::versions::java_1_21_11::ClientboundIds::SET_TITLE_TEXT
        } else {
            0x4f
        };
        let mut invalid_boss = vec![0; 16];
        invalid_boss.push(6);
        invalid_boss.push(0);
        assert_eq!(
            error(decode_boss_bar(version, &invalid_boss)),
            (
                InvalidInput,
                "invalid boss bar operation".into(),
                "invalid boss bar operation".into()
            )
        );
        let mut remove_boss = vec![0; 16];
        remove_boss.extend([1, 0]);
        assert_eq!(
            error(decode_boss_bar(version, &remove_boss)),
            (
                Protocol,
                "unexpected trailing packet data: 1 bytes".into(),
                "unexpected trailing packet data: 1 bytes".into()
            )
        );
        assert_eq!(
            error(decode_boss_bar(version, &[0; 15])),
            (
                Protocol,
                "truncated packet field".into(),
                "truncated packet field".into()
            )
        );
        // An unknown operation is diagnosed before its trailing field.
        assert_eq!(
            error(decode_display(version, -1, &[0])),
            (
                InvalidInput,
                "unsupported display packet".into(),
                "unsupported display packet".into()
            )
        );
        let mut team = vec![];
        put_string(&mut team, "a");
        team.extend([5, 0]);
        assert_eq!(
            error(decode_team(version, &team)),
            (
                InvalidInput,
                "unknown team operation".into(),
                "unknown team operation".into()
            )
        );
        let id = if modern {
            crate::versions::java_1_21_11::ClientboundIds::PLAYER_INFO
        } else {
            0x33
        };
        let roster = if modern { vec![0, 0, 0] } else { vec![4, 0, 0] };
        assert_eq!(
            error(decode_player_list(version, id, &roster)),
            (
                Protocol,
                "unexpected trailing packet data: 1 bytes".into(),
                "unexpected trailing packet data: 1 bytes".into()
            )
        );
        // Malformed text still fails before applying any title update.
        assert!(decode_display(version, title, &[]).is_err());
    }
}

#[test]
fn both_text_codecs_preserve_unrendered_wire_bytes_and_field_origins() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let mut payload = vec![9; 16];
        payload.push(0);
        let text = " {\"text\":\"\\u0041\"} ";
        let expected = if version == MinecraftVersion::Java1_16_1 {
            put_string(&mut payload, text);
            UiText::LegacyJson { json: text.into() }
        } else {
            // A compound text component with a noncanonical field order is retained verbatim.
            let nbt = b"\x0a\x01\x00\x04bold\x01\x08\x00\x04text\x00\x01A\x00";
            payload.extend(nbt);
            UiText::NativeNbt {
                bytes: nbt.to_vec(),
            }
        };
        payload.extend(0.25f32.to_be_bytes());
        payload.extend([6, 4, 255]);
        let (uuid, update) = decode_boss_bar(version, &payload).unwrap();
        let mut ledger = ui::boss_bar::BossBarLedger::default();
        ledger.apply(uuid, update, 31).unwrap();
        let bar = ledger.capture(stamp(version), 33).bars.remove(0);
        assert_eq!(bar.title.value, expected);
        assert_eq!(bar.title.source, ValueSource::Received { sequence: 31 });
        assert_eq!(bar.flags.value.raw, 255);
    }
}

fn encoded_text(payload: &mut Vec<u8>, version: MinecraftVersion, text: &str) {
    if version == MinecraftVersion::Java1_16_1 {
        put_string(payload, text);
    } else {
        payload.push(8);
        payload.extend((text.len() as u16).to_be_bytes());
        payload.extend(text.as_bytes());
    }
}
fn player_batch(version: MinecraftVersion, start: u128, count: i32) -> Vec<u8> {
    let legacy = version == MinecraftVersion::Java1_16_1;
    let mut payload = vec![if legacy { 0 } else { 1 }];
    put_varint(&mut payload, count);
    for n in start..start + count as u128 {
        payload.extend(n.to_be_bytes());
        put_string(&mut payload, "Player");
        payload.push(0); // properties
        if legacy {
            payload.extend([0, 0, 0]);
        } // mode, latency, optional name
    }
    payload
}

#[test]
fn both_versions_keep_ledger_capacity_refusals_atomic_without_eviction() {
    use crate::ErrorKind::ResourceLimit;
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let modern = version == MinecraftVersion::Java1_21_11;
        let player_id = if modern {
            crate::versions::java_1_21_11::ClientboundIds::PLAYER_INFO
        } else {
            0x33
        };
        let mut roster = ui::player_list::PlayerListLedger::default();
        roster
            .receive(version, player_id, &player_batch(version, 0, 4096), 2)
            .unwrap();
        let before = serde_json::to_value(roster.capture(stamp(version), 2)).unwrap();
        let e = error(roster.receive(version, player_id, &player_batch(version, 4096, 1), 9));
        assert_eq!(
            e,
            (
                ResourceLimit,
                "player-list observation capacity exceeded".into(),
                "player-list observation capacity exceeded".into()
            )
        );
        assert_eq!(
            serde_json::to_value(roster.capture(stamp(version), 2)).unwrap(),
            before
        );
        let e = error(decode_player_list(
            version,
            player_id,
            &[if modern { 1 } else { 0 }, 0x81, 0x20],
        ));
        assert_eq!(
            e,
            (
                ResourceLimit,
                "packet count exceeds limit 4096".into(),
                "packet count exceeds limit 4096".into()
            )
        );

        let mut bars = ui::boss_bar::BossBarLedger::default();
        let mut boss = vec![0; 16];
        boss.push(0);
        encoded_text(&mut boss, version, "raw");
        boss.extend(0.5f32.to_be_bytes());
        boss.extend([0, 0, 0]);
        for n in 0u128..4096 {
            boss[..16].copy_from_slice(&n.to_be_bytes());
            bars.receive(version, &boss, 3).unwrap();
        }
        boss[..16].copy_from_slice(&4096u128.to_be_bytes());
        assert_eq!(
            error(bars.receive(version, &boss, 9)),
            (
                ResourceLimit,
                "boss bars exceed 4096 received entries".into(),
                "boss bars exceed 4096 received entries".into()
            )
        );
        let view = bars.capture(stamp(version), 9);
        assert_eq!(view.bars.len(), 4096);
        assert_eq!(view.last_update_sequence, Some(3));
        assert_eq!(view.bars.first().unwrap().uuid, [0; 16]);
        // A replacement consumes no new capacity and keeps the original receipt boundary.
        boss[..16].fill(0);
        bars.receive(version, &boss, 11).unwrap();
        assert_eq!(
            bars.capture(stamp(version), 12).bars[0].title.source,
            ValueSource::Received { sequence: 11 }
        );

        let mut teams = ui::teams::TeamLedger::default();
        let data = fixtures("social");
        let rows = data["versions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["version"] == version.to_string())
            .unwrap()["packets"]
            .as_array()
            .unwrap();
        let seed = rows
            .iter()
            .find(|r| r["group"] == "team" && r["operation"] == 0)
            .unwrap();
        let mut packet = decode_team(version, &bytes(seed)).unwrap();
        packet.members = (0..4095).map(|n| format!("holder{n}")).collect();
        teams.apply_packet(version, packet, 4).unwrap();
        let before = serde_json::to_value(teams.capture(stamp(version), 4)).unwrap();
        let mut join = vec![];
        put_string(&mut join, seed["name"].as_str().unwrap());
        join.push(3);
        join.push(1);
        put_string(&mut join, "overflow");
        assert_eq!(
            error(teams.receive(version, &join, 9)),
            (
                ResourceLimit,
                "team observation capacity exceeded".into(),
                "team observation capacity exceeded".into()
            )
        );
        assert_eq!(
            serde_json::to_value(teams.capture(stamp(version), 4)).unwrap(),
            before
        );

        let mut scores = ui::ScoreboardLedger::default();
        let score_id = if modern { 0x6c } else { 0x4d };
        let score = |owner: &str| {
            let mut p = vec![];
            put_string(&mut p, owner);
            if !modern {
                p.push(0);
            }
            put_string(&mut p, "objective");
            p.push(1);
            if modern {
                p.extend([0, 0]);
            }
            p
        };
        for n in 0..4096 {
            scores
                .receive(version, score_id, &score(&format!("holder{n}")), 5)
                .unwrap();
        }
        assert_eq!(
            error(scores.receive(version, score_id, &score("overflow"), 9)),
            (
                ResourceLimit,
                "scoreboard exceeds 4096 received entries".into(),
                "scoreboard exceeds 4096 received entries".into()
            )
        );
        let view = scores.capture(stamp(version), 9);
        assert_eq!(view.scores.len(), 4096);
        assert_eq!(view.last_update_sequence, Some(5));
    }
}
