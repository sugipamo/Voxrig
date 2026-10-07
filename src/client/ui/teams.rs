//! Received team declarations and scoreboard-holder membership, not permissions.
use super::{UiText, received};
use crate::client::VersionAdapter;
use crate::client::{ObservedValue, SessionStamp};
use crate::{MinecraftVersion, Result};
use std::collections::{BTreeMap, BTreeSet};

/// Common native team name-tag visibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamVisibility {
    /// Always show name tags.
    Always,
    /// Never show name tags.
    Never,
    /// Hide name tags from other teams.
    HideForOtherTeams,
    /// Hide name tags from one's own team.
    HideForOwnTeam,
}
impl TeamVisibility {
    pub(crate) fn legacy_name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::HideForOtherTeams => "hideForOtherTeams",
            Self::HideForOwnTeam => "hideForOwnTeam",
        }
    }
}
/// Common native team collision rule; this observation does not enforce physics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamCollision {
    /// Always collide.
    Always,
    /// Never collide.
    Never,
    /// Push other teams.
    PushOtherTeams,
    /// Push one's own team.
    PushOwnTeam,
}
impl TeamCollision {
    pub(crate) fn legacy_name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::PushOtherTeams => "pushOtherTeams",
            Self::PushOwnTeam => "pushOwnTeam",
        }
    }
}
/// Native color formatting permitted for a team.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum TeamColor {
    /// Black.
    Black = 0,
    /// Dark blue.
    DarkBlue = 1,
    /// Dark green.
    DarkGreen = 2,
    /// Dark aqua.
    DarkAqua = 3,
    /// Dark red.
    DarkRed = 4,
    /// Dark purple.
    DarkPurple = 5,
    /// Gold.
    Gold = 6,
    /// Gray.
    Gray = 7,
    /// Dark gray.
    DarkGray = 8,
    /// Blue.
    Blue = 9,
    /// Green.
    Green = 10,
    /// Aqua.
    Aqua = 11,
    /// Red.
    Red = 12,
    /// Light purple.
    LightPurple = 13,
    /// Yellow.
    Yellow = 14,
    /// White.
    White = 15,
    /// Reset to default color.
    Reset = 21,
}
impl TeamColor {
    pub(crate) fn native_id(self) -> i32 {
        self as i32
    }
    fn decode(id: i32) -> Result<Self> {
        Ok(match id {
            0 => Self::Black,
            1 => Self::DarkBlue,
            2 => Self::DarkGreen,
            3 => Self::DarkAqua,
            4 => Self::DarkRed,
            5 => Self::DarkPurple,
            6 => Self::Gold,
            7 => Self::Gray,
            8 => Self::DarkGray,
            9 => Self::Blue,
            10 => Self::Green,
            11 => Self::Aqua,
            12 => Self::Red,
            13 => Self::LightPurple,
            14 => Self::Yellow,
            15 => Self::White,
            21 => Self::Reset,
            _ => return Err(invalid("unsupported team color")),
        })
    }
}
/// Complete received team parameters, supplied together in ADD or CHANGE.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct TeamParameters {
    /// Original component encoding.
    pub display: UiText,
    /// Original flag byte, including unrecognized bits.
    pub friendly_flags: u8,
    /// Name-tag rule.
    pub visibility: TeamVisibility,
    /// Collision rule, without inferring player collision permissions.
    pub collision: TeamCollision,
    /// Native team color.
    pub color: TeamColor,
    /// Original player-prefix component.
    pub prefix: UiText,
    /// Original player-suffix component.
    pub suffix: UiText,
}
/// One declared team and its surviving received scoreboard holders.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedTeam {
    /// Original wire team name.
    pub name: String,
    /// Actual ADD/CHANGE origin for the complete parameter group.
    pub parameters: ObservedValue<TeamParameters>,
    /// Surviving members in sorted holder-name order. These can be offline holders.
    pub members: Vec<ObservedValue<String>>,
    /// Actual first ADD for this surviving team declaration.
    pub created_sequence: u64,
    /// Last membership event, including removal or a move into another team.
    pub last_members_update_sequence: Option<u64>,
}
/// Received team state on this connection; not a server-wide catalogue or roster.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TeamsObservation {
    /// Current capture boundary; ordinary play-world changes retain team receipts.
    pub session: SessionStamp,
    /// Coherent capture boundary.
    pub receive_sequence: u64,
    /// Actual context-reset packet ordinal, separate from UI field receipts.
    /// None means no connection-level context reset has been received.
    pub context_reset_sequence: Option<u64>,
    /// Last actual team packet, including removals and unknown-team updates.
    pub last_update_sequence: Option<u64>,
    /// Declared surviving teams sorted by name.
    pub teams: Vec<ReceivedTeam>,
}
impl crate::Client {
    /// Inspect received team parameters/membership without I/O or rendering.
    pub async fn teams(&self) -> Result<TeamsObservation> {
        crate::client::dispatch!(&self.adapter, a => VersionAdapter::teams(a).await)
    }
}
#[derive(Clone)]
struct TeamState {
    parameters: ObservedValue<TeamParameters>,
    members: BTreeMap<String, ObservedValue<String>>,
    created_sequence: u64,
    members_sequence: Option<u64>,
}
#[derive(Clone, Default)]
pub(crate) struct TeamLedger {
    context_reset_sequence: Option<u64>,
    teams: BTreeMap<String, TeamState>,
    owners: BTreeMap<String, String>,
    sequence: Option<u64>,
}
pub(crate) struct TeamPacket {
    pub name: String,
    pub operation: u8,
    pub parameters: Option<TeamParameters>,
    pub members: Vec<String>,
}
fn invalid(message: &str) -> crate::Error {
    super::super::recording::invalid(message)
}
pub(crate) fn decode(version: MinecraftVersion, payload: &[u8]) -> Result<TeamPacket> {
    let modern = version == MinecraftVersion::Java1_21_11;
    let mut r = crate::versions::java_1_21_11::ScoreboardReader::new(payload);
    let name = r.string()?;
    let operation = r.u8()?;
    if operation > 4 {
        return Err(invalid("unknown team operation"));
    }
    let parameters = if operation == 0 || operation == 2 {
        let display = super::text(&mut r, modern)?;
        let friendly_flags = r.u8()?;
        let visibility = if modern {
            match r.varint()? {
                0 => TeamVisibility::Always,
                1 => TeamVisibility::Never,
                2 => TeamVisibility::HideForOtherTeams,
                3 => TeamVisibility::HideForOwnTeam,
                _ => return Err(invalid("unsupported team visibility")),
            }
        } else {
            match r.string()?.as_str() {
                "always" => TeamVisibility::Always,
                "never" => TeamVisibility::Never,
                "hideForOtherTeams" => TeamVisibility::HideForOtherTeams,
                "hideForOwnTeam" => TeamVisibility::HideForOwnTeam,
                _ => return Err(invalid("unsupported team visibility")),
            }
        };
        let collision = if modern {
            match r.varint()? {
                0 => TeamCollision::Always,
                1 => TeamCollision::Never,
                2 => TeamCollision::PushOtherTeams,
                3 => TeamCollision::PushOwnTeam,
                _ => return Err(invalid("unsupported team collision")),
            }
        } else {
            match r.string()?.as_str() {
                "always" => TeamCollision::Always,
                "never" => TeamCollision::Never,
                "pushOtherTeams" => TeamCollision::PushOtherTeams,
                "pushOwnTeam" => TeamCollision::PushOwnTeam,
                _ => return Err(invalid("unsupported team collision")),
            }
        };
        let color = TeamColor::decode(r.varint()?)?;
        let prefix = super::text(&mut r, modern)?;
        let suffix = super::text(&mut r, modern)?;
        Some(TeamParameters {
            display,
            friendly_flags,
            visibility,
            collision,
            color,
            prefix,
            suffix,
        })
    } else {
        None
    };
    let mut members = Vec::new();
    if operation == 0 || operation == 3 || operation == 4 {
        for _ in 0..r.count(4096)? {
            members.push(r.string()?);
        }
    }
    r.end()?;
    Ok(TeamPacket {
        name,
        operation,
        parameters,
        members,
    })
}
impl TeamLedger {
    pub(crate) fn reset_context(&mut self, sequence: u64) {
        *self = Self {
            context_reset_sequence: Some(sequence),
            ..Self::default()
        };
    }

    pub(crate) fn receive(
        &mut self,
        version: MinecraftVersion,
        payload: &[u8],
        sequence: u64,
    ) -> Result<()> {
        let packet = decode(version, payload)?;
        // The native legacy scoreboard rejects duplicate ADD; modern reuses it.
        if packet.operation == 0
            && self.teams.contains_key(&packet.name)
            && version == MinecraftVersion::Java1_16_1
        {
            return Err(invalid("duplicate legacy team declaration"));
        }
        if packet.operation == 4
            && self.teams.contains_key(&packet.name)
            && (packet.members.iter().collect::<BTreeSet<_>>().len() != packet.members.len()
                || packet
                    .members
                    .iter()
                    .any(|m| self.owners.get(m) != Some(&packet.name)))
        {
            return Err(invalid("team leave does not match received ownership"));
        }
        let mut next = self.clone();
        next.apply(packet, sequence);
        if next.teams.len() + next.owners.len() > 4096 || next.encoded_bytes() > 16 * 1024 * 1024 {
            return Err(crate::Error::new(
                crate::ErrorKind::ResourceLimit,
                anyhow::anyhow!("team observation capacity exceeded"),
            ));
        }
        next.sequence = Some(sequence);
        *self = next;
        Ok(())
    }
    fn apply(&mut self, p: TeamPacket, sequence: u64) {
        if p.operation == 1 {
            self.teams.remove(&p.name);
            self.owners.retain(|_, owner| owner != &p.name);
            return;
        }
        if p.operation == 0 {
            self.teams
                .entry(p.name.clone())
                .or_insert_with(|| TeamState {
                    parameters: received(p.parameters.as_ref().unwrap().clone(), sequence),
                    members: BTreeMap::new(),
                    created_sequence: sequence,
                    members_sequence: None,
                });
        }
        if !self.teams.contains_key(&p.name) {
            return;
        }
        if let Some(params) = p.parameters {
            self.teams.get_mut(&p.name).unwrap().parameters = received(params, sequence);
        }
        if p.operation == 0 || p.operation == 3 {
            for member in p.members.into_iter().collect::<BTreeSet<_>>() {
                if let Some(previous) = self.owners.insert(member.clone(), p.name.clone()) {
                    let old = self.teams.get_mut(&previous).unwrap();
                    old.members.remove(&member);
                    old.members_sequence = Some(sequence);
                }
                let team = self.teams.get_mut(&p.name).unwrap();
                team.members
                    .insert(member.clone(), received(member, sequence));
            }
            self.teams.get_mut(&p.name).unwrap().members_sequence = Some(sequence);
        } else if p.operation == 4 {
            let team = self.teams.get_mut(&p.name).unwrap();
            for member in p.members {
                self.owners.remove(&member);
                team.members.remove(&member);
            }
            team.members_sequence = Some(sequence);
        }
    }
    fn encoded_bytes(&self) -> usize {
        self.teams
            .iter()
            .map(|(name, t)| {
                name.len()
                    + text_bytes(&t.parameters.value.display)
                    + text_bytes(&t.parameters.value.prefix)
                    + text_bytes(&t.parameters.value.suffix)
                    + t.members.keys().map(|m| 2 * m.len()).sum::<usize>()
            })
            .sum::<usize>()
            + self
                .owners
                .iter()
                .map(|(member, owner)| member.len() + owner.len())
                .sum::<usize>()
    }
    pub(crate) fn capture(&self, session: SessionStamp, receive_sequence: u64) -> TeamsObservation {
        TeamsObservation {
            session,
            receive_sequence,
            context_reset_sequence: self.context_reset_sequence,
            last_update_sequence: self.sequence,
            teams: self
                .teams
                .iter()
                .map(|(name, t)| ReceivedTeam {
                    name: name.clone(),
                    parameters: t.parameters.clone(),
                    members: t.members.values().cloned().collect(),
                    created_sequence: t.created_sequence,
                    last_members_update_sequence: t.members_sequence,
                })
                .collect(),
        }
    }
}
pub(crate) fn text_bytes(text: &UiText) -> usize {
    match text {
        UiText::LegacyJson { json } => json.len(),
        UiText::NativeNbt { bytes } => bytes.len(),
        UiText::Unavailable => 0,
    }
}
