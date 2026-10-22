use super::UiReader;
use crate::Result;
use crate::client::GameMode;
use crate::client::recording::invalid;
use crate::client::ui::{PlayerChatSession, PlayerListing, PlayerProfile, PlayerProperty, UiText};
#[derive(Default, Clone)]
pub(crate) struct PlayerUpdate {
    pub(crate) uuid: [u8; 16],
    pub(crate) profile: Option<PlayerProfile>,
    pub(crate) game_mode: Option<Option<GameMode>>,
    pub(crate) latency: Option<i32>,
    pub(crate) display_name: Option<Option<UiText>>,
    pub(crate) listing: Option<PlayerListing>,
    pub(crate) list_order: Option<i32>,
    pub(crate) show_hat: Option<bool>,
    pub(crate) chat_session: Option<Option<PlayerChatSession>>,
}
#[derive(Clone)]
pub(crate) enum PlayerChanges {
    Updates(Vec<PlayerUpdate>),
    Remove(Vec<[u8; 16]>),
}
fn mode<'a, R: UiReader<'a>>(r: &mut R, modern: bool) -> Result<Option<GameMode>> {
    let value = r.varint()?;
    if value == -1 && !modern {
        return Ok(None);
    }
    if !(0..=3).contains(&value) {
        return Err(invalid("unsupported player-list game mode"));
    }
    Ok(Some(GameMode::decode(value as u8)?))
}
pub(super) fn decode<'a, R: UiReader<'a>>(id: i32, payload: &'a [u8]) -> Result<PlayerPacket> {
    use crate::versions::java_1_21_11::ClientboundIds as ids;
    let modern = R::MODERN;
    let mut r = R::new(payload);
    if modern && id == ids::PLAYER_REMOVE {
        let mut uuids = Vec::new();
        for _ in 0..r.count(4096)? {
            uuids.push(r.take(16)?.try_into().unwrap());
        }
        r.end()?;
        return Ok(PlayerPacket {
            action: None,
            changes: PlayerChanges::Remove(uuids),
        });
    }
    if id != if modern { ids::PLAYER_INFO } else { 0x33 } {
        return Err(invalid("unsupported player-info packet"));
    }
    let action = if modern { None } else { Some(r.varint()?) };
    let (flags, remove) = if modern {
        (r.u8()?, false)
    } else {
        match action.unwrap() {
            0 => (1 | 4 | 16 | 32, false),
            1 => (4, false),
            2 => (16, false),
            3 => (32, false),
            4 => (0, true),
            _ => return Err(invalid("unknown player-info action")),
        }
    };
    let mut updates = Vec::new();
    let mut removed = Vec::new();
    for _ in 0..r.count(4096)? {
        let uuid = r.take(16)?.try_into().unwrap();
        if remove {
            removed.push(uuid);
            continue;
        }
        let mut u = PlayerUpdate {
            uuid,
            ..Default::default()
        };
        if flags & 1 != 0 {
            let name = r.string()?;
            if name.is_empty() || name.len() > 64 {
                return Err(invalid("invalid player profile name"));
            }
            let mut properties = Vec::new();
            for _ in 0..r.count(1024)? {
                properties.push(PlayerProperty {
                    name: r.string()?,
                    value: r.string()?,
                    signature: if r.bool()? { Some(r.string()?) } else { None },
                });
            }
            u.profile = Some(PlayerProfile { name, properties });
            if !modern {
                u.listing = Some(PlayerListing::LegacyEntry);
            }
        }
        if flags & 2 != 0 {
            u.chat_session = Some(if r.bool()? {
                Some(PlayerChatSession {
                    uuid: r.take(16)?.try_into().unwrap(),
                    expires_at_epoch_millis: r.u64()? as i64,
                    public_key: r.byte_array(65536)?.to_vec(),
                    key_signature: r.byte_array(65536)?.to_vec(),
                })
            } else {
                None
            });
        }
        if flags & 4 != 0 {
            u.game_mode = Some(mode(&mut r, modern)?);
        }
        if flags & 8 != 0 {
            u.listing = Some(PlayerListing::Listed { listed: r.bool()? });
        }
        if flags & 16 != 0 {
            u.latency = Some(r.varint()?);
        }
        if flags & 32 != 0 {
            u.display_name = Some(if r.bool()? { Some(r.text()?) } else { None });
        }
        if flags & 64 != 0 {
            u.list_order = Some(r.varint()?);
        }
        if flags & 128 != 0 {
            u.show_hat = Some(r.bool()?);
        }
        updates.push(u);
    }
    r.end()?;
    Ok(PlayerPacket {
        action,
        changes: if remove {
            PlayerChanges::Remove(removed)
        } else {
            PlayerChanges::Updates(updates)
        },
    })
}

#[derive(Clone)]
pub(crate) struct PlayerPacket {
    pub(crate) action: Option<i32>,
    pub(crate) changes: PlayerChanges,
}
