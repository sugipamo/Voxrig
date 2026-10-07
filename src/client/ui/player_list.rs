//! Received profile/list records, independent of spatial entities and authentication.
use super::{UiText, received};
use crate::client::VersionAdapter;
use crate::client::{GameMode, ObservedValue, SessionStamp};
use crate::{MinecraftVersion, Result};
use std::collections::BTreeMap;

/// Original profile property strings; signatures are not validated by this API.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct PlayerProperty {
    /// Original property name.
    pub name: String,
    /// Original property value.
    pub value: String,
    /// Original optional signature string.
    pub signature: Option<String>,
}
/// Profile supplied in an actual ADD, with original property ordering.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct PlayerProfile {
    /// Received profile name, distinct from rendered display name.
    pub name: String,
    /// Complete original property list.
    pub properties: Vec<PlayerProperty>,
}
/// List membership evidence: a legacy entry is distinct from a modern flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlayerListing {
    /// Legacy ADD registers an entry; there is no separate listed flag.
    LegacyEntry,
    /// Actual modern UPDATE_LISTED flag, independent of profile registration.
    Listed {
        /// Original boolean flag.
        listed: bool,
    },
}
impl PlayerListing {
    /// Native list-membership interpretation, without claiming rendered pixels.
    pub fn is_listed(self) -> bool {
        match self {
            Self::LegacyEntry => true,
            Self::Listed { listed } => listed,
        }
    }
}
/// Raw received chat session/key metadata, without signature or expiry validation.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct PlayerChatSession {
    /// Original session UUID.
    pub uuid: [u8; 16],
    /// Original signed epoch milliseconds.
    pub expires_at_epoch_millis: i64,
    /// Original encoded public key bytes.
    pub public_key: Vec<u8>,
    /// Original key signature bytes.
    pub key_signature: Vec<u8>,
}
/// One actually registered profile and separately received list fields.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PlayerListEntry {
    /// Native profile UUID, not a connection-local entity ID.
    pub uuid: [u8; 16],
    /// Actual ADD profile and complete property list.
    pub profile: ObservedValue<PlayerProfile>,
    /// Received game mode. Inner None is actual legacy NOT_SET; outer None is missing.
    pub game_mode: Option<ObservedValue<Option<GameMode>>>,
    /// Original signed latency field, not a locally measured/current RTT.
    pub latency: Option<ObservedValue<i32>>,
    /// Original optional display component; inner None is an actual absent override.
    pub display_name: Option<ObservedValue<Option<UiText>>>,
    /// Legacy registration or actual modern listed flag; None means never supplied.
    pub listing: Option<ObservedValue<PlayerListing>>,
    /// Actual modern list-order integer; no legacy/default value is invented.
    pub list_order: Option<ObservedValue<i32>>,
    /// Actual modern hat-display flag; no legacy/default value is invented.
    pub show_hat: Option<ObservedValue<bool>>,
    /// Actual modern optional chat-session data; not proof of authentication.
    pub chat_session: Option<ObservedValue<Option<PlayerChatSession>>>,
}
/// Registered player-list receipts, including unlisted modern profiles.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlayerListObservation {
    /// Current capture boundary; ordinary play-world changes retain profile receipts.
    pub session: SessionStamp,
    /// Coherent receive boundary.
    pub receive_sequence: u64,
    /// Actual context-reset packet ordinal, separate from UI field receipts.
    /// None means no connection-level context reset has been received.
    pub context_reset_sequence: Option<u64>,
    /// Last actual player-info/remove packet, even when no entry survives.
    pub last_update_sequence: Option<u64>,
    /// Surviving actual ADDs sorted by UUID; not a complete online-account catalogue.
    pub entries: Vec<PlayerListEntry>,
}
impl crate::Client {
    /// Inspect received profiles/list updates; entity coordinates use entity_motion.
    pub async fn player_list(&self) -> Result<PlayerListObservation> {
        crate::client::dispatch!(&self.adapter, a => VersionAdapter::player_list(a).await)
    }
}
#[derive(Clone, Default)]
pub(crate) struct PlayerListLedger {
    context_reset_sequence: Option<u64>,
    entries: BTreeMap<[u8; 16], PlayerListEntry>,
    sequence: Option<u64>,
}
#[derive(Default)]
struct Update {
    uuid: [u8; 16],
    profile: Option<PlayerProfile>,
    game_mode: Option<Option<GameMode>>,
    latency: Option<i32>,
    display_name: Option<Option<UiText>>,
    listing: Option<PlayerListing>,
    list_order: Option<i32>,
    show_hat: Option<bool>,
    chat_session: Option<Option<PlayerChatSession>>,
}
enum Packet {
    Updates(Vec<Update>),
    Remove(Vec<[u8; 16]>),
}
fn invalid(message: &str) -> crate::Error {
    super::super::recording::invalid(message)
}
fn mode(
    r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>,
    modern: bool,
) -> Result<Option<GameMode>> {
    let value = r.varint()?;
    if value == -1 && !modern {
        return Ok(None);
    }
    if !(0..=3).contains(&value) {
        return Err(invalid("unsupported player-list game mode"));
    }
    Ok(Some(GameMode::decode(value as u8)?))
}
fn decode(version: MinecraftVersion, id: i32, payload: &[u8]) -> Result<Packet> {
    use crate::versions::java_1_21_11::ClientboundIds as ids;
    let modern = version == MinecraftVersion::Java1_21_11;
    let mut r = crate::versions::java_1_21_11::ScoreboardReader::new(payload);
    if modern && id == ids::PLAYER_REMOVE {
        let mut uuids = Vec::new();
        for _ in 0..r.count(4096)? {
            uuids.push(r.take(16)?.try_into().unwrap());
        }
        r.end()?;
        return Ok(Packet::Remove(uuids));
    }
    if id != if modern { ids::PLAYER_INFO } else { 0x33 } {
        return Err(invalid("unsupported player-info packet"));
    }
    let (flags, remove) = if modern {
        (r.u8()?, false)
    } else {
        match r.varint()? {
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
        let mut u = Update {
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
            u.display_name = Some(if r.bool()? {
                Some(super::text(&mut r, modern)?)
            } else {
                None
            });
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
    Ok(if remove {
        Packet::Remove(removed)
    } else {
        Packet::Updates(updates)
    })
}
impl PlayerListLedger {
    pub(crate) fn reset_context(&mut self, sequence: u64) {
        *self = Self {
            context_reset_sequence: Some(sequence),
            ..Self::default()
        };
    }

    pub(crate) fn receive(
        &mut self,
        version: MinecraftVersion,
        id: i32,
        payload: &[u8],
        sequence: u64,
    ) -> Result<()> {
        match decode(version, id, payload)? {
            Packet::Remove(uuids) => {
                for uuid in uuids {
                    self.entries.remove(&uuid);
                }
            }
            Packet::Updates(updates) => {
                // Stage changed records only. One malformed/resource-limited batch
                // must not expose its earlier entries or overwrite old origins.
                let mut changed = BTreeMap::new();
                for u in updates {
                    let uuid = u.uuid;
                    let mut entry = if let Some(profile) = u.profile {
                        PlayerListEntry {
                            uuid,
                            profile: received(profile, sequence),
                            game_mode: None,
                            latency: None,
                            display_name: None,
                            listing: None,
                            list_order: None,
                            show_hat: None,
                            chat_session: None,
                        }
                    } else if let Some(prior) = changed
                        .remove(&uuid)
                        .or_else(|| self.entries.get(&uuid).cloned())
                    {
                        prior
                    } else {
                        continue;
                    };
                    if let Some(value) = u.game_mode {
                        entry.game_mode = Some(received(value, sequence));
                    }
                    if let Some(value) = u.latency {
                        entry.latency = Some(received(value, sequence));
                    }
                    if let Some(value) = u.display_name {
                        entry.display_name = Some(received(value, sequence));
                    }
                    if let Some(value) = u.listing {
                        entry.listing = Some(received(value, sequence));
                    }
                    if let Some(value) = u.list_order {
                        entry.list_order = Some(received(value, sequence));
                    }
                    if let Some(value) = u.show_hat {
                        entry.show_hat = Some(received(value, sequence));
                    }
                    if let Some(value) = u.chat_session {
                        entry.chat_session = Some(received(value, sequence));
                    }
                    changed.insert(uuid, entry);
                }
                let count = self.entries.len()
                    + changed
                        .keys()
                        .filter(|u| !self.entries.contains_key(*u))
                        .count();
                let bytes = self
                    .entries
                    .iter()
                    .filter(|(uuid, _)| !changed.contains_key(*uuid))
                    .map(|(_, e)| encoded_bytes(e))
                    .sum::<usize>()
                    + changed.values().map(encoded_bytes).sum::<usize>();
                if count > 4096 || bytes > 16 * 1024 * 1024 {
                    return Err(crate::Error::new(
                        crate::ErrorKind::ResourceLimit,
                        anyhow::anyhow!("player-list observation capacity exceeded"),
                    ));
                }
                self.entries.extend(changed);
            }
        }
        self.sequence = Some(sequence);
        Ok(())
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> PlayerListObservation {
        PlayerListObservation {
            session,
            receive_sequence,
            context_reset_sequence: self.context_reset_sequence,
            last_update_sequence: self.sequence,
            entries: self.entries.values().cloned().collect(),
        }
    }
}
fn encoded_bytes(e: &PlayerListEntry) -> usize {
    e.profile.value.name.len()
        + e.profile
            .value
            .properties
            .iter()
            .map(|p| p.name.len() + p.value.len() + p.signature.as_ref().map_or(0, String::len))
            .sum::<usize>()
        + e.display_name
            .as_ref()
            .and_then(|v| v.value.as_ref())
            .map_or(0, super::teams::text_bytes)
        + e.chat_session
            .as_ref()
            .and_then(|v| v.value.as_ref())
            .map_or(0, |v| v.public_key.len() + v.key_signature.len())
}
