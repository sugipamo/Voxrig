//! Structured chat messages and the server-maintained player list.

use crate::versions::java_1_16_1::protocol::get_string;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `ChatMessage`.
pub struct ChatMessage {
    /// Raw JSON chat component; rendering and natural-language conversion are caller concerns.
    pub json: String,
    /// The `position` value.
    pub position: i8,
    /// The `sender` value.
    pub sender: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `PlayerProperty`.
pub struct PlayerProperty {
    /// The `name` value.
    pub name: String,
    /// The `value` value.
    pub value: String,
    /// The `signature` value.
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `PlayerListEntry`.
pub struct PlayerListEntry {
    /// The `uuid` value.
    pub uuid: [u8; 16],
    /// The `name` value.
    pub name: String,
    /// The `properties` value.
    pub properties: Vec<PlayerProperty>,
    /// The `game_mode` value.
    pub game_mode: i32,
    /// The `latency` value.
    pub latency: i32,
    /// The `display_name_json` value.
    pub display_name_json: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// State and protocol data represented by `PlayerList`.
pub struct PlayerList {
    /// The `entries` value.
    pub entries: HashMap<[u8; 16], PlayerListEntry>,
}

fn read_uuid(rest: &mut &[u8]) -> Result<[u8; 16]> {
    if rest.len() < 16 {
        bail!("truncated player UUID");
    }
    let mut uuid = [0; 16];
    uuid.copy_from_slice(&rest[..16]);
    *rest = &rest[16..];
    Ok(uuid)
}

pub(crate) fn parse_chat(payload: &[u8]) -> Result<ChatMessage> {
    let mut rest = payload;
    let json = get_string(&mut rest)?;
    let position = *rest.first().context("missing chat position")? as i8;
    rest = &rest[1..];
    Ok(ChatMessage {
        json,
        position,
        sender: read_uuid(&mut rest)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::java_1_16_1::protocol::put_string;

    #[test]
    fn chat_keeps_raw_json_and_sender() {
        let mut payload = Vec::new();
        put_string(&mut payload, r#"{\"text\":\"hello\"}"#);
        payload.push(0);
        payload.extend([7; 16]);
        let message = parse_chat(&payload).unwrap();
        assert_eq!(message.json, r#"{\"text\":\"hello\"}"#);
        assert_eq!(message.sender, [7; 16]);
    }
}

pub(crate) fn project_player_info(
    list: &mut PlayerList,
    packet: crate::versions::ui_decode::PlayerPacket,
) -> (i32, Vec<[u8; 16]>) {
    use crate::versions::ui_decode::PlayerChanges;
    let action = packet.action.expect("legacy player-info action");
    let mut changed = Vec::new();
    let json = |text| match text {
        crate::client::ui::UiText::LegacyJson { json } => json,
        _ => unreachable!("legacy player text"),
    };
    let mode = |value: Option<crate::client::GameMode>| value.map_or(-1, |value| value as i32);
    match packet.changes {
        PlayerChanges::Remove(uuids) => {
            for uuid in uuids {
                changed.push(uuid);
                list.entries.remove(&uuid);
            }
        }
        PlayerChanges::Updates(updates) => {
            for u in updates {
                changed.push(u.uuid);
                if let Some(profile) = u.profile {
                    list.entries.insert(
                        u.uuid,
                        PlayerListEntry {
                            uuid: u.uuid,
                            name: profile.name,
                            properties: profile
                                .properties
                                .into_iter()
                                .map(|p| PlayerProperty {
                                    name: p.name,
                                    value: p.value,
                                    signature: p.signature,
                                })
                                .collect(),
                            game_mode: mode(u.game_mode.unwrap()),
                            latency: u.latency.unwrap(),
                            display_name_json: u.display_name.unwrap().map(json),
                        },
                    );
                } else if let Some(entry) = list.entries.get_mut(&u.uuid) {
                    if let Some(value) = u.game_mode {
                        entry.game_mode = mode(value);
                    }
                    if let Some(value) = u.latency {
                        entry.latency = value;
                    }
                    if let Some(value) = u.display_name {
                        entry.display_name_json = value.map(json);
                    }
                }
            }
        }
    }
    (action, changed)
}

#[cfg(test)]
mod original_decode;
#[cfg(test)]
pub(crate) use original_decode::apply_player_info;
