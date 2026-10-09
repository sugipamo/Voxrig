//! Exact received packets and detached, read-only version decoder replay.
use crate::{Error, ErrorKind, Result};

/// Receive-side native protocol phase. Login authentication is not included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketPhase {
    /// Modern registry/configuration exchange.
    Configuration,
    /// Native gameplay packets.
    Play,
}
/// Local inputs actually used to resolve a received relative position packet.
/// These are parser inputs, not independently received position/velocity facts.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPlayerBasis {
    /// Local feet before applying the incoming packet, if available.
    pub position: Option<[f64; 3]>,
    /// Local view before the packet.
    pub rotation: [f32; 2],
    /// Received velocity baseline accepted by the native correction decoder.
    pub velocity: Option<[f64; 3]>,
}
/// One original decompressed received payload, before decoding/application.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PacketRecord {
    /// Original connection's receive ordinal, never a server tick.
    pub sequence: u64,
    /// Local reconstruction frame before application.
    pub client_tick: u64,
    /// Native protocol phase.
    pub phase: PacketPhase,
    /// Native packet identifier.
    pub packet_id: i32,
    /// Original decompressed payload bytes.
    pub payload: Vec<u8>,
    /// Local basis for a position packet; absent on other packets.
    #[serde(default)]
    pub local_player_basis: Option<LocalPlayerBasis>,
}
/// Bounded original receive evidence. Deserializing never restores a Client.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PacketTrace {
    /// Exact release name; replay rejects an unknown version before decoding.
    pub minecraft_version: String,
    /// Original process-local connection identity; diagnostic provenance only.
    pub connection_id: u64,
    /// Receive boundary at capture start.
    pub after_sequence: u64,
    /// Receive boundary at capture stop, including dropped overflow records.
    pub through_sequence: u64,
    /// Local reconstruction frame at stop.
    #[serde(default)]
    pub through_client_tick: u64,
    /// False after overflow; capture never resumes after dropping a packet.
    pub complete: bool,
    /// Original frames in receive order.
    pub records: Vec<PacketRecord>,
}

pub(crate) fn invalid(message: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}
pub(crate) fn validate_limit(limit: usize) -> Result<()> {
    if !(1..=16_777_216).contains(&limit) {
        return Err(invalid("trace limit must be 1..=16777216"));
    }
    Ok(())
}
pub(crate) struct TraceCapture {
    pub(crate) start: u64,
    bytes: usize,
    limit: usize,
    complete: bool,
    records: Vec<PacketRecord>,
}
impl TraceCapture {
    pub(crate) fn new(start: u64, limit: usize) -> Result<Self> {
        validate_limit(limit)?;
        Ok(Self {
            start,
            bytes: 0,
            limit,
            complete: true,
            records: vec![],
        })
    }
    pub(crate) fn record(
        &mut self,
        sequence: u64,
        client_tick: u64,
        phase: PacketPhase,
        packet_id: i32,
        payload: &[u8],
        local_player_basis: Option<LocalPlayerBasis>,
    ) {
        if !self.complete {
            return;
        }
        if self.bytes.saturating_add(payload.len()) > self.limit || self.records.len() >= 65_536 {
            self.complete = false;
            return;
        }
        self.bytes += payload.len();
        self.records.push(PacketRecord {
            sequence,
            client_tick,
            phase,
            packet_id,
            payload: payload.to_vec(),
            local_player_basis,
        });
    }
    pub(crate) fn finish(
        self,
        version: crate::MinecraftVersion,
        connection_id: u64,
        through_sequence: u64,
        through_client_tick: u64,
    ) -> PacketTrace {
        PacketTrace {
            minecraft_version: version.name().to_owned(),
            connection_id,
            after_sequence: self.start,
            through_sequence,
            through_client_tick,
            complete: self.complete,
            records: self.records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_overflow_never_reports_complete_or_resumes_after_a_gap() {
        let mut capture = TraceCapture::new(10, 2).unwrap();
        for (sequence, payload) in [(11, vec![1, 2]), (12, vec![3]), (13, vec![])] {
            capture.record(sequence, 0, PacketPhase::Play, 8, &payload, None);
        }
        let trace = capture.finish(crate::MinecraftVersion::Java1_16_1, 4, 13, 0);
        assert!(!trace.complete);
        assert_eq!(trace.records.len(), 1);
        assert_eq!(trace.records[0].sequence, 11);
        assert_eq!(trace.through_sequence, 13);
    }
    #[test]
    fn incomplete_histories_never_invent_a_replay_baseline() {
        let query = crate::Region {
            min: [0, 65, 0],
            max: [0, 65, 0],
        };
        let trace = PacketTrace {
            minecraft_version: "1.16.1".into(),
            connection_id: 2,
            after_sequence: 0,
            through_sequence: 1,
            through_client_tick: 0,
            complete: true,
            records: vec![PacketRecord {
                sequence: 1,
                client_tick: 0,
                phase: PacketPhase::Play,
                packet_id: 0x20,
                payload: vec![0; 8],
                local_player_basis: None,
            }],
        };
        // A structurally complete history still requires an actual initial JOIN.
        assert!(trace.replay(query, 1).is_err());
        for case in 0..8 {
            let mut bad = trace.clone();
            match case {
                0 => bad.complete = false,
                1 => bad.after_sequence = 9,
                2 => bad.records[0].sequence = 2,
                3 => bad.through_sequence = 2,
                4 => bad.minecraft_version = "1.99".into(),
                5 => bad.records[0].packet_id = -1,
                6 => bad.records[0].client_tick = 1,
                _ => {
                    bad.records[0].local_player_basis = Some(LocalPlayerBasis {
                        position: Some([f64::NAN, 0.0, 0.0]),
                        rotation: [0.0; 2],
                        velocity: None,
                    })
                }
            }
            assert!(bad.replay(query, 1).is_err(), "invalid history case {case}");
        }
        assert!(trace.replay(query, 0).is_err());
        assert!(trace.replay(query, 257).is_err());
    }
}

/// Received slot/value facts, deliberately distinct from session-owned guards.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedValue<T> {
    /// Value as decoded from the supplied packet history.
    pub value: T,
    /// Original receive ordinal, not a replay time or execution permission.
    pub source: super::ValueSource,
}
/// Read-only inventory facts. No ScreenId or PlayerScreenAccess can be restored.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayedInventory {
    /// Canonical player slots, preserving unavailable and empty separately.
    pub slots: Vec<Option<RecordedValue<RecordedSlotKnowledge>>>,
    /// Received carried stack.
    pub cursor: Option<RecordedValue<RecordedSlotKnowledge>>,
    /// Native active window number, not a usable screen handle.
    pub window_id: Option<i32>,
    /// Modern received menu revision when supplied.
    pub screen_revision: Option<i32>,
}
/// Original received state decoded off-line. It owns no Client or sender.
/// Saved inventory facts cannot become a usable stack:
/// ```compile_fail
/// use voxrig::client::{ItemStack, recording::RecordedItemStack};
/// fn restore(record: RecordedItemStack) -> ItemStack { record.into() }
/// ```
/// Replaying supplied/edited bytes never certifies their actual server origin.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayedObservation {
    /// Explicit adapter used to decode this trace.
    pub version: crate::MinecraftVersion,
    /// Original provenance only, not a restored SessionStamp.
    pub source_connection_id: u64,
    /// Original world reset boundary.
    pub world_generation: u64,
    /// Last applied original receive ordinal.
    pub receive_sequence: u64,
    /// Received dimension metadata.
    pub dimension: Option<super::Dimension>,
    /// Last received correction, separate from local replay inputs/predictions.
    pub received_pose: Option<super::ReceivedPose>,
    /// Received game mode.
    pub game_mode: Option<super::GameMode>,
    /// Received flight permission.
    pub may_fly: Option<bool>,
    /// Last received health.
    pub health: Option<RecordedValue<super::Health>>,
    /// Last received hotbar selection; local selections are not packets.
    pub selected_hotbar: Option<RecordedValue<u8>>,
    /// Received inventory values and numerical revisions.
    pub inventory: ReplayedInventory,
    /// Queried native received block-cache values; missing chunks stay unavailable.
    pub blocks: Vec<ReplayedBlock>,
    /// Native packet IDs outside the legacy selected replay surface. Their exact
    /// bytes remain in the trace; this is not a claim of full protocol replay.
    pub unhandled_packets: Vec<i32>,
}
/// One bounded replay query cell, without a live region/capture handle.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayedBlock {
    /// Absolute cell.
    pub position: [i32; 3],
    /// Decoded native state; None is unavailable, not air.
    pub state: Option<crate::NativeBlockState>,
}
/// Safe item data records contain numerical registry facts, not RegistryId.
pub use super::observation::{RecordedItemData, RecordedItemStack, RecordedSlotKnowledge};

pub(crate) fn project(
    player: &super::PlayerObservation,
    blocks: Vec<ReplayedBlock>,
    unhandled_packets: Vec<i32>,
) -> ReplayedObservation {
    use crate::diagnostic_projection::ToDiagnostic;
    let slot = |value: &super::ObservedValue<super::SlotKnowledge>| RecordedValue {
        value: value.value.diagnostic(),
        source: value.source,
    };
    ReplayedObservation {
        version: player.session.version,
        source_connection_id: player.session.connection_id,
        world_generation: player.session.world_generation,
        receive_sequence: player.receive_sequence,
        dimension: player.dimension.clone(),
        received_pose: player.received_pose.clone(),
        game_mode: player.game_mode,
        may_fly: player.may_fly,
        health: player.health.as_ref().map(|v| RecordedValue {
            value: v.value,
            source: v.source,
        }),
        selected_hotbar: player
            .selected_hotbar
            .as_ref()
            .filter(|v| matches!(v.source, super::ValueSource::Received { .. }))
            .map(|v| RecordedValue {
                value: v.value,
                source: v.source,
            }),
        inventory: ReplayedInventory {
            slots: player
                .inventory
                .slots
                .iter()
                .map(|s| s.as_ref().map(slot))
                .collect(),
            cursor: player.inventory.cursor.as_ref().map(slot),
            window_id: player.inventory.window_id,
            screen_revision: player.inventory.screen_revision,
        },
        blocks,
        unhandled_packets,
    }
}
impl PacketTrace {
    /// Decode a complete from-connect trace using its exact version, without any
    /// socket, sender, timer-driven game operation, Client or usable operation ID.
    /// Late-start/overflow/gapped histories have no hidden reconstructed baseline.
    pub fn replay(
        &self,
        region: crate::Region,
        maximum_chunks: usize,
    ) -> Result<ReplayedObservation> {
        region.volume()?;
        let version = self.minecraft_version.parse::<crate::MinecraftVersion>()?;
        if !(1..=256).contains(&maximum_chunks)
            || !self.complete
            || self.after_sequence != 0
            || self.records.len() > 65_536
        {
            return Err(invalid(
                "replay requires a complete from-connect trace and 1..256 chunk bound",
            ));
        }
        let mut bytes = 0usize;
        let mut last_tick = 0;
        for (index, record) in self.records.iter().enumerate() {
            if record.sequence != index as u64 + 1 || record.packet_id < 0 {
                return Err(invalid("trace receive order/packet identifier is invalid"));
            }
            if version == crate::MinecraftVersion::Java1_16_1
                && (record.client_tick < last_tick || record.client_tick > self.through_client_tick)
            {
                return Err(invalid("trace local frame order/stop boundary is invalid"));
            }
            last_tick = record.client_tick;
            bytes = bytes
                .checked_add(record.payload.len())
                .ok_or_else(|| invalid("trace byte count overflow"))?;
            if bytes > 16_777_216 {
                return Err(invalid("trace exceeds the 16MiB byte bound"));
            }
            if let Some(basis) = &record.local_player_basis {
                if basis
                    .position
                    .into_iter()
                    .flatten()
                    .chain(basis.velocity.into_iter().flatten())
                    .any(|v| !v.is_finite())
                    || basis.rotation.iter().any(|v| !v.is_finite())
                {
                    return Err(invalid("trace local player basis is non-finite"));
                }
            }
        }
        if self.through_sequence != self.records.len() as u64 {
            return Err(invalid("trace stop boundary has a packet gap"));
        }
        match version {
            crate::MinecraftVersion::Java1_16_1 => {
                crate::versions::java_1_16_1::client::replay_packets(self, region, maximum_chunks)
            }
            crate::MinecraftVersion::Java1_21_11 => {
                crate::versions::java_1_21_11::replay_packets(self, region, maximum_chunks)
            }
        }
    }
}
