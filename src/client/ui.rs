//! Received scoreboard and boss-bar facts; rendering and other UI remain separate.
pub mod boss_bar;
/// Lossless native UI text, without rendering or resolving server references.
/// The legacy ScreenTitle name remains compatible with existing consumers.
pub use super::container::ScreenTitle as UiText;
use super::{ObservedValue, SessionStamp, received};
use crate::{MinecraftVersion, Result, connection::Adapter};
pub use boss_bar::{
    BossBarColor, BossBarFlags, BossBarOverlay, BossBarsObservation, ReceivedBossBar,
};
use std::collections::BTreeMap;
/// Native modern score-number presentation, preserving optional overrides.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScoreNumberFormat {
    /// Native blank number format.
    Blank,
    /// Original unnamed NBT style encoding.
    Styled {
        /// Native style bytes, including the root tag.
        bytes: Vec<u8>,
    },
    /// Original native display component.
    Fixed {
        /// Received UI component.
        text: UiText,
    },
}
/// Common native objective presentation mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScoreboardRenderType {
    /// Render the score as a number.
    Integer,
    /// Render the score as health hearts.
    Hearts,
}
/// One received objective declaration.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ScoreboardObjective {
    /// Wire objective name, not a rendered title.
    pub name: String,
    /// Native text encoding.
    pub display: UiText,
    /// Common integer/hearts presentation.
    pub render_type: ScoreboardRenderType,
    /// Modern optional number format; absent on legacy.
    pub number_format: Option<ScoreNumberFormat>,
}
/// One received score entry; optional modern display/format are retained.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ScoreboardScore {
    /// Native score holder name.
    pub owner: String,
    /// Wire objective name.
    pub objective: String,
    /// Signed native integer score.
    pub value: i32,
    /// Optional modern custom display component.
    pub display: Option<UiText>,
    /// Optional modern per-entry format override.
    pub number_format: Option<ScoreNumberFormat>,
}
/// Coherent received scoreboard history on one transport. It is not a complete
/// server objective catalogue: the server may send only displayed objectives.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScoreboardObservation {
    /// Current connection/world provenance. No Deserialize restores a session.
    pub session: SessionStamp,
    /// Shared receive boundary at capture, not a server tick.
    pub receive_sequence: u64,
    /// Last actual scoreboard event, including clears/removes. None means no
    /// scoreboard packet has been received, not an acknowledged empty server.
    pub last_update_sequence: Option<u64>,
    /// Surviving actual declarations, sorted by wire name.
    pub objectives: Vec<ObservedValue<ScoreboardObjective>>,
    /// Native display-slot bindings; clearing a slot removes its binding.
    pub displays: BTreeMap<i32, ObservedValue<String>>,
    /// Surviving entries, sorted by holder/objective.
    pub scores: Vec<ObservedValue<ScoreboardScore>>,
}
impl super::Client {
    /// Inspect received scoreboard facts without I/O. Each value retains its
    /// original packet ordinal; reading or rendering does not establish scores.
    pub async fn scoreboard_state(&self) -> Result<ScoreboardObservation> {
        match &self.adapter {
            Adapter::Java1_16_1(bot) => bot.common_scoreboard_state().await,
            Adapter::Java1_21_11(bot) => bot.common_scoreboard_state().await,
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct ScoreboardLedger {
    objectives: BTreeMap<String, ObservedValue<ScoreboardObjective>>,
    displays: BTreeMap<i32, ObservedValue<String>>,
    scores: BTreeMap<(String, String), ObservedValue<ScoreboardScore>>,
    last_update_sequence: Option<u64>,
}
pub(crate) enum ScoreboardUpdate {
    Objective(ScoreboardObjective),
    RemoveObjective(String),
    Display(i32, String),
    Score(ScoreboardScore),
    Reset {
        owner: String,
        objective: Option<String>,
    },
}
impl ScoreboardLedger {
    /// Parse the whole original packet before mutating any cache field.
    pub(crate) fn receive(
        &mut self,
        version: MinecraftVersion,
        id: i32,
        payload: &[u8],
        sequence: u64,
    ) -> Result<()> {
        let update = decode(version, id, payload)?;
        let adds = match &update {
            ScoreboardUpdate::Objective(o) => !self.objectives.contains_key(&o.name),
            ScoreboardUpdate::Display(slot, name) => {
                !name.is_empty() && !self.displays.contains_key(slot)
            }
            ScoreboardUpdate::Score(s) => !self
                .scores
                .contains_key(&(s.owner.clone(), s.objective.clone())),
            _ => false,
        };
        if adds && self.objectives.len() + self.displays.len() + self.scores.len() >= 4096 {
            return Err(crate::Error::new(
                crate::ErrorKind::ResourceLimit,
                anyhow::anyhow!("scoreboard exceeds 4096 received entries"),
            ));
        }
        match update {
            ScoreboardUpdate::Objective(o) => {
                self.objectives
                    .insert(o.name.clone(), received(o, sequence));
            }
            ScoreboardUpdate::RemoveObjective(name) => {
                self.objectives.remove(&name);
                self.displays.retain(|_, d| d.value != name);
                self.scores.retain(|(_, o), _| o != &name);
            }
            ScoreboardUpdate::Display(slot, name) => {
                if name.is_empty() {
                    self.displays.remove(&slot);
                } else {
                    self.displays.insert(slot, received(name, sequence));
                }
            }
            ScoreboardUpdate::Score(s) => {
                self.scores.insert(
                    (s.owner.clone(), s.objective.clone()),
                    received(s, sequence),
                );
            }
            ScoreboardUpdate::Reset { owner, objective } => self.scores.retain(|(p, o), _| {
                p != &owner || objective.as_ref().is_some_and(|name| name != o)
            }),
        }
        self.last_update_sequence = Some(sequence);
        Ok(())
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> ScoreboardObservation {
        ScoreboardObservation {
            session,
            receive_sequence,
            last_update_sequence: self.last_update_sequence,
            objectives: self.objectives.values().cloned().collect(),
            displays: self.displays.clone(),
            scores: self.scores.values().cloned().collect(),
        }
    }
}
fn decode(version: MinecraftVersion, id: i32, payload: &[u8]) -> Result<ScoreboardUpdate> {
    use crate::versions::java_1_21_11::ScoreboardReader as Reader;
    let mut r = Reader::new(payload);
    let modern = version == MinecraftVersion::Java1_21_11;
    let objective_id = if modern { 0x68 } else { 0x4a };
    let display_id = if modern { 0x60 } else { 0x43 };
    let score_id = if modern { 0x6c } else { 0x4d };
    let update = if id == objective_id {
        let name = r.string()?;
        let action = r.u8()?;
        if action == 1 {
            ScoreboardUpdate::RemoveObjective(name)
        } else if action == 0 || action == 2 {
            let display = text(&mut r, modern)?;
            let render_type = match r.varint()? {
                0 => ScoreboardRenderType::Integer,
                1 => ScoreboardRenderType::Hearts,
                _ => return Err(super::recording::invalid("invalid scoreboard render type")),
            };
            let number_format = if modern { format(&mut r)? } else { None };
            ScoreboardUpdate::Objective(ScoreboardObjective {
                name,
                display,
                render_type,
                number_format,
            })
        } else {
            return Err(super::recording::invalid(
                "invalid scoreboard objective action",
            ));
        }
    } else if id == display_id {
        let slot = if modern {
            r.varint()?
        } else {
            i32::from(r.u8()?)
        };
        if !(0..=18).contains(&slot) {
            return Err(super::recording::invalid("invalid scoreboard display slot"));
        }
        ScoreboardUpdate::Display(slot, r.string()?)
    } else if id == score_id {
        let owner = r.string()?;
        let remove = if modern {
            false
        } else {
            match r.varint()? {
                0 => false,
                1 => true,
                _ => return Err(super::recording::invalid("invalid scoreboard score action")),
            }
        };
        let objective = r.string()?;
        if remove {
            ScoreboardUpdate::Reset {
                owner,
                objective: (!objective.is_empty()).then_some(objective),
            }
        } else {
            let value = r.varint()?;
            let display = if modern && r.bool()? {
                Some(text(&mut r, true)?)
            } else {
                None
            };
            let number_format = if modern { format(&mut r)? } else { None };
            ScoreboardUpdate::Score(ScoreboardScore {
                owner,
                objective,
                value,
                display,
                number_format,
            })
        }
    } else if modern && id == 0x4d {
        let owner = r.string()?;
        let objective = if r.bool()? { Some(r.string()?) } else { None };
        ScoreboardUpdate::Reset { owner, objective }
    } else {
        return Err(super::recording::invalid(
            "not a supported native scoreboard packet",
        ));
    };
    r.end()?;
    Ok(update)
}
fn text(
    r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>,
    modern: bool,
) -> Result<UiText> {
    Ok(if modern {
        UiText::NativeNbt {
            bytes: r.encoded_nbt()?,
        }
    } else {
        UiText::LegacyJson { json: r.string()? }
    })
}
fn format(
    r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>,
) -> Result<Option<ScoreNumberFormat>> {
    if !r.bool()? {
        return Ok(None);
    }
    Ok(Some(match r.varint()? {
        0 => ScoreNumberFormat::Blank,
        1 => ScoreNumberFormat::Styled {
            bytes: r.encoded_nbt()?,
        },
        2 => ScoreNumberFormat::Fixed {
            text: text(r, true)?,
        },
        _ => {
            return Err(super::recording::invalid(
                "unknown score number format requires an adapter update",
            ));
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{put_string, put_varint};
    fn session(version: MinecraftVersion) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 9,
            world_generation: 4,
        }
    }
    fn encoded_text(p: &mut Vec<u8>, version: MinecraftVersion, value: &str) {
        if version == MinecraftVersion::Java1_16_1 {
            put_string(p, value);
        } else {
            p.push(8);
            p.extend((value.len() as u16).to_be_bytes());
            p.extend(value.as_bytes());
        }
    }
    fn objective(version: MinecraftVersion, name: &str) -> Vec<u8> {
        let mut p = vec![];
        put_string(&mut p, name);
        p.push(0);
        encoded_text(&mut p, version, "Title");
        p.push(0);
        if version == MinecraftVersion::Java1_21_11 {
            p.push(0);
        }
        p
    }
    fn score(version: MinecraftVersion, owner: &str, name: &str, value: i32) -> Vec<u8> {
        let mut p = vec![];
        put_string(&mut p, owner);
        if version == MinecraftVersion::Java1_16_1 {
            p.push(0);
        }
        put_string(&mut p, name);
        put_varint(&mut p, value);
        if version == MinecraftVersion::Java1_21_11 {
            p.extend([0, 0]);
        }
        p
    }
    #[test]
    fn received_scoreboard_removals_and_origins_preserve_connection_scope_on_both_versions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let (oid, sid, did, rid) = if version == MinecraftVersion::Java1_16_1 {
                (0x4a, 0x4d, 0x43, 0x4d)
            } else {
                (0x68, 0x6c, 0x60, 0x4d)
            };
            let mut ledger = ScoreboardLedger::default();
            assert_eq!(
                ledger.capture(session(version), 0).last_update_sequence,
                None
            );
            ledger
                .receive(version, oid, &objective(version, "alpha"), 1)
                .unwrap();
            ledger
                .receive(version, oid, &objective(version, "beta"), 2)
                .unwrap();
            let mut display = vec![1];
            put_string(&mut display, "alpha");
            ledger.receive(version, did, &display, 3).unwrap();
            ledger
                .receive(version, sid, &score(version, "Alice", "alpha", 7), 4)
                .unwrap();
            ledger
                .receive(version, sid, &score(version, "Alice", "beta", 11), 5)
                .unwrap();
            let snapshot = ledger.capture(session(version), 6);
            assert_eq!(
                snapshot.scores[0].source,
                super::super::ValueSource::Received { sequence: 4 }
            );
            assert_eq!(
                snapshot.displays[&1].source,
                super::super::ValueSource::Received { sequence: 3 }
            );
            let mut reset = vec![];
            put_string(&mut reset, "Alice");
            if version == MinecraftVersion::Java1_16_1 {
                reset.extend([1, 0]);
            } else {
                reset.push(0);
            }
            ledger.receive(version, rid, &reset, 7).unwrap();
            assert!(ledger.capture(session(version), 7).scores.is_empty());
            if version == MinecraftVersion::Java1_16_1 {
                let mut native = crate::versions::java_1_16_1::UiState::default();
                native
                    .apply_score(&score(version, "Alice", "alpha", 7))
                    .unwrap();
                native
                    .apply_score(&score(version, "Alice", "beta", 11))
                    .unwrap();
                native.apply_score(&reset).unwrap();
                assert!(native.scores.is_empty());
            }
            let mut remove = vec![];
            put_string(&mut remove, "alpha");
            remove.push(1);
            ledger.receive(version, oid, &remove, 8).unwrap();
            let end = ledger.capture(session(version), 10);
            assert_eq!(end.objectives.len(), 1);
            assert_eq!(end.objectives[0].value.name, "beta");
            assert!(end.displays.is_empty());
            assert_eq!(end.last_update_sequence, Some(8));
        }
    }
    #[test]
    fn partial_malformed_and_unknown_modern_format_packets_never_commit_partial_fields() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let oid = if version == MinecraftVersion::Java1_16_1 {
                0x4a
            } else {
                0x68
            };
            let p = objective(version, "alpha");
            let mut ledger = ScoreboardLedger::default();
            let before = serde_json::to_value(ledger.capture(session(version), 0)).unwrap();
            for end in 0..p.len() {
                assert!(ledger.receive(version, oid, &p[..end], 1).is_err());
                assert_eq!(
                    serde_json::to_value(ledger.capture(session(version), 0)).unwrap(),
                    before
                );
            }
            let mut extra = p.clone();
            extra.push(0);
            assert!(ledger.receive(version, oid, &extra, 1).is_err());
            if version == MinecraftVersion::Java1_21_11 {
                let mut unknown = p;
                unknown.pop();
                unknown.extend([1, 3]);
                assert!(ledger.receive(version, oid, &unknown, 1).is_err());
                assert_eq!(
                    serde_json::to_value(ledger.capture(session(version), 0)).unwrap(),
                    before
                );
            }
        }
    }
    #[test]
    fn modern_number_formats_and_per_score_component_fields_keep_exact_native_bytes() {
        let version = MinecraftVersion::Java1_21_11;
        let mut ledger = ScoreboardLedger::default();
        for format_id in 0..3 {
            let mut p = objective(version, "alpha");
            p.pop();
            p.extend([1, format_id]);
            match format_id {
                1 => p.extend([10, 0]),
                2 => encoded_text(&mut p, version, "Fixed"),
                _ => {}
            }
            ledger.receive(version, 0x68, &p, 1).unwrap();
            let declared = &ledger.capture(session(version), 1).objectives[0]
                .value
                .number_format;
            assert!(matches!(
                (format_id, declared),
                (0, Some(ScoreNumberFormat::Blank))
                    | (1, Some(ScoreNumberFormat::Styled { .. }))
                    | (2, Some(ScoreNumberFormat::Fixed { .. }))
            ));
        }
        let mut p = score(version, "Alice", "alpha", -7);
        p.truncate(p.len() - 2);
        p.push(1);
        encoded_text(&mut p, version, "Custom");
        p.extend([1, 1, 10, 0]);
        ledger.receive(version, 0x6c, &p, 2).unwrap();
        let observed = ledger.capture(session(version), 3);
        let entry = &observed.scores[0].value;
        assert_eq!(entry.value, -7);
        assert_eq!(
            entry.number_format,
            Some(ScoreNumberFormat::Styled { bytes: vec![10, 0] })
        );
        assert!(
            matches!(&entry.display,Some(UiText::NativeNbt{bytes}) if bytes.ends_with(b"Custom"))
        );
    }
}
