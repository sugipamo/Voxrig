//! Received profile/list records, independent of spatial entities and authentication.
use super::{UiText, received};
use crate::client::adapter::UiOps;
use crate::client::{GameMode, ObservedValue, SessionStamp};
use crate::versions::ui_decode::{PlayerChanges, PlayerPacket};
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
        crate::client::dispatch!(&self.adapter, a => UiOps::player_list(a).await)
    }
}
#[derive(Clone, Default)]
pub(crate) struct PlayerListLedger {
    context_reset_sequence: Option<u64>,
    entries: BTreeMap<[u8; 16], PlayerListEntry>,
    sequence: Option<u64>,
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
        let packet = crate::versions::ui_decode::decode_player_list(version, id, payload)?;
        self.apply(packet, sequence)
    }
    pub(crate) fn apply(&mut self, packet: PlayerPacket, sequence: u64) -> Result<()> {
        match packet.changes {
            PlayerChanges::Remove(uuids) => {
                for uuid in uuids {
                    self.entries.remove(&uuid);
                }
            }
            PlayerChanges::Updates(updates) => {
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
