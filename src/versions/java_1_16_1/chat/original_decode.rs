//! Original player-info parser, retained only as a differential regression oracle.
use super::*;
use crate::versions::java_1_16_1::protocol::get_string;
use crate::versions::java_1_16_1::protocol::get_varint;
use anyhow::{Context, Result, bail};

fn optional_string(rest: &mut &[u8]) -> Result<Option<String>> {
    let present = *rest.first().context("missing optional string presence")? != 0;
    *rest = &rest[1..];
    Ok(if present {
        Some(get_string(rest)?)
    } else {
        None
    })
}

pub(crate) fn apply_player_info(
    list: &mut PlayerList,
    payload: &[u8],
) -> Result<(i32, Vec<[u8; 16]>)> {
    let mut rest = payload;
    let action = get_varint(&mut rest)?;
    let count = get_varint(&mut rest)?;
    if !(0..=4096).contains(&count) {
        bail!("invalid player info count {count}");
    }
    let mut changed = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let uuid = read_uuid(&mut rest)?;
        changed.push(uuid);
        match action {
            0 => {
                let name = get_string(&mut rest)?;
                let property_count = get_varint(&mut rest)?;
                if !(0..=1024).contains(&property_count) {
                    bail!("invalid player property count {property_count}");
                }
                let mut properties = Vec::with_capacity(property_count as usize);
                for _ in 0..property_count {
                    properties.push(PlayerProperty {
                        name: get_string(&mut rest)?,
                        value: get_string(&mut rest)?,
                        signature: optional_string(&mut rest)?,
                    });
                }
                let game_mode = get_varint(&mut rest)?;
                let latency = get_varint(&mut rest)?;
                let display_name_json = optional_string(&mut rest)?;
                list.entries.insert(
                    uuid,
                    PlayerListEntry {
                        uuid,
                        name,
                        properties,
                        game_mode,
                        latency,
                        display_name_json,
                    },
                );
            }
            1 => {
                let game_mode = get_varint(&mut rest)?;
                if let Some(entry) = list.entries.get_mut(&uuid) {
                    entry.game_mode = game_mode;
                }
            }
            2 => {
                let latency = get_varint(&mut rest)?;
                if let Some(entry) = list.entries.get_mut(&uuid) {
                    entry.latency = latency;
                }
            }
            3 => {
                let display_name_json = optional_string(&mut rest)?;
                if let Some(entry) = list.entries.get_mut(&uuid) {
                    entry.display_name_json = display_name_json;
                }
            }
            4 => {
                list.entries.remove(&uuid);
            }
            _ => bail!("unknown player info action {action}"),
        }
    }
    Ok((action, changed))
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
