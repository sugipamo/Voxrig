use super::*;
use crate::client::{EventKind, ValueSource};

#[test]
fn captured_conduct_matches_official_server_and_empty_accept_packet() {
    // Official 1.21.11 frames in docs/evidence/code-of-conduct-20261022.json.
    let payload =
        hex::decode("204265206b696e642e0ae5bbbae7af89e38292e5a4a7e58887e381ab20f09f9880").unwrap();
    let mut state = State::default();
    assert_eq!(
        state.receive(0x13, &payload, 64).unwrap(),
        vec![(0x09, vec![])]
    );
    assert_eq!(
        state.code_of_conduct.unwrap().value,
        "Be kind.\n建築を大切に 😀"
    );
    let log = state.events.after(0, state.sequence).unwrap();
    assert_eq!(log.events.len(), 1);
    assert_eq!(log.events[0].kind, EventKind::CodeOfConductReceived);
    assert_eq!(log.events[0].receive_sequence, 1);
}

#[test]
fn conduct_accepts_each_complete_string_and_records_received_sequence() {
    const TEXTS: [&str; 4] = [
        "",
        "Be kind.\nRespect builds.",
        "建築を大切に 😀",
        "建築を大切に 😀",
    ];
    let mut state = State::default();
    assert!(state.code_of_conduct.is_none());
    for (index, text) in TEXTS.into_iter().enumerate() {
        let mut payload = Vec::new();
        put_string(&mut payload, text);
        // Independent wire IDs: clientbound conduct=0x13, serverbound accept=0x09.
        assert_eq!(
            state.receive(0x13, &payload, 64).unwrap(),
            vec![(0x09, vec![])]
        );
        assert_eq!(state.phase, Phase::Configuration);
        let receipt = state.code_of_conduct.as_ref().unwrap();
        assert_eq!(receipt.value, text);
        assert_eq!(
            receipt.source,
            ValueSource::Received {
                sequence: state.sequence
            }
        );
        let events = state.events.after(0, state.sequence).unwrap().events;
        assert_eq!(events.len(), index + 1);
        assert_eq!(events[index].kind, EventKind::CodeOfConductReceived);
        assert_eq!(events[index].receive_sequence, state.sequence);
    }
    state.dimensions.push(Dimension::new(-64, 384).unwrap());
    let receipt = state.code_of_conduct.clone();
    assert_eq!(state.receive(0x03, &[], 64).unwrap(), vec![(0x03, vec![])]);
    assert_eq!(state.phase, Phase::Play);
    // Reconfiguration retains the last fact and accepts a new text.
    state
        .receive(ids::play_clientbound::START_CONFIGURATION, &[], 64)
        .unwrap();
    assert_eq!(state.code_of_conduct, receipt);
    let mut payload = Vec::new();
    put_string(&mut payload, "Updated rules");
    assert_eq!(
        state.receive(0x13, &payload, 64).unwrap(),
        vec![(0x09, vec![])]
    );
    assert_eq!(state.code_of_conduct.unwrap().value, "Updated rules");
}

#[test]
fn malformed_conduct_never_commits_receipt_event_or_acceptance() {
    let mut valid = Vec::new();
    put_string(&mut valid, "Be kind.\n建築を大切に 😀");
    let mut malformed: Vec<Vec<u8>> = (0..valid.len()).map(|n| valid[..n].to_vec()).collect();
    let mut trailing = valid.clone();
    trailing.push(0);
    malformed.push(trailing);
    const INVALID_FIELDS: [&[u8]; 5] = [
        &[1, 0xff],                         // Invalid UTF-8.
        &[3, 0xed, 0xa0, 0x80],             // Unpaired surrogate encoded as UTF-8.
        &[0xff, 0xff, 0xff, 0xff, 0x0f],    // Negative length.
        &[0x80],                            // Truncated VarInt.
        &[0x80, 0x80, 0x80, 0x80, 0x80, 0], // Oversized VarInt.
    ];
    malformed.extend(INVALID_FIELDS.map(<[u8]>::to_vec));
    for payload in malformed {
        let mut state = State::default();
        state.receive(0x13, &valid, 64).unwrap();
        let before = state.code_of_conduct.clone();
        assert!(state.receive(0x13, &payload, 64).is_err());
        assert_eq!(state.code_of_conduct, before);
        let events = state.events.after(0, state.sequence).unwrap().events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::CodeOfConductReceived);
    }
}

#[test]
fn conduct_string_limit_counts_utf16_units() {
    // ByteBufCodecs.STRING_UTF8 uses Java String.length(), hence UTF-16 units.
    let texts = [
        ("a".repeat(32767), true),
        ("界".repeat(32767), true),
        (format!("{}x", "😀".repeat(16383)), true),
        ("a".repeat(32768), false),
        ("😀".repeat(16384), false),
    ];
    for (text, accepted) in texts {
        let mut state = State::default();
        let mut payload = Vec::new();
        put_string(&mut payload, &text);
        let result = state.receive(0x13, &payload, 64);
        assert_eq!(result.is_ok(), accepted);
        if accepted {
            assert_eq!(result.unwrap(), vec![(0x09, vec![])]);
            assert_eq!(state.code_of_conduct.unwrap().value, text);
        } else {
            assert!(state.code_of_conduct.is_none());
            assert!(
                state
                    .events
                    .after(0, state.sequence)
                    .unwrap()
                    .events
                    .is_empty()
            );
        }
    }
}
