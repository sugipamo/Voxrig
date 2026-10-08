//! Received title, tab-list and border instructions, without local rendering.
use super::{ObservedValue, SessionStamp, UiText, received};
use crate::client::adapter::UiOps;
use crate::{MinecraftVersion, Result};

/// Original title-animation instruction, distinct from elapsed display time.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TitleTiming {
    /// Original signed tick fields; negative fields are not replaced by defaults.
    Set {
        /// Original fade-in ticks.
        fade_in: i32,
        /// Original stay ticks.
        stay: i32,
        /// Original fade-out ticks.
        fade_out: i32,
    },
    /// Actual RESET instructs the client to restore its own defaults.
    ResetToDefaults,
}
/// Last received title instructions. Expiration and displayed pixels are absent.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TitlesObservation {
    /// Current connection/world provenance.
    pub session: SessionStamp,
    /// Coherent receive boundary.
    pub receive_sequence: u64,
    /// Actual context-reset packet ordinal, separate from UI field receipts.
    /// None means no connection-level context reset has been received.
    pub context_reset_sequence: Option<u64>,
    /// Last actual title-family event, including clear/reset.
    pub last_update_sequence: Option<u64>,
    /// Received title or received clear. Outer None means never received.
    pub title: Option<ObservedValue<Option<UiText>>>,
    /// Received subtitle or received clear.
    pub subtitle: Option<ObservedValue<Option<UiText>>>,
    /// Last actual action-bar message; clear/reset of titles does not erase it.
    pub action_bar: Option<ObservedValue<UiText>>,
    /// Last explicit timing/reset instruction. No initial defaults are invented.
    pub timing: Option<ObservedValue<TitleTiming>>,
    /// Last clear command; true means its reset-times flag was set.
    pub clear: Option<ObservedValue<bool>>,
}
/// The two native components in one complete tab-list packet.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct TabListText {
    /// Original header encoding.
    pub header: UiText,
    /// Original footer encoding.
    pub footer: UiText,
}
/// Received tab-list header/footer, distinct from player-list entries.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TabListObservation {
    /// Current connection/world provenance.
    pub session: SessionStamp,
    /// Coherent receive boundary.
    pub receive_sequence: u64,
    /// Actual context-reset packet ordinal, separate from UI field receipts.
    /// None means no connection-level context reset has been received.
    pub context_reset_sequence: Option<u64>,
    /// Complete received pair. None means no header/footer packet received.
    pub text: Option<ObservedValue<TabListText>>,
}
/// Original border duration and native unit, without assuming wall-clock progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum WorldBorderDuration {
    /// Original signed milliseconds (Java 1.16.1).
    Milliseconds(i64),
    /// Original signed game ticks (Java 1.21.11).
    Ticks(i64),
}
impl WorldBorderDuration {
    /// Nominal milliseconds at 20 ticks/second; None on conversion overflow.
    /// This does not measure elapsed wall time or create another received field.
    pub fn nominal_milliseconds(self) -> Option<i64> {
        match self {
            Self::Milliseconds(value) => Some(value),
            Self::Ticks(value) => value.checked_mul(50),
        }
    }
}
/// Original border-size instruction. No current interpolation is calculated.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldBorderSize {
    /// Complete immediate size packet.
    Set {
        /// Original finite target diameter.
        diameter: f64,
    },
    /// Original time-based transition fields.
    Lerp {
        /// Original starting diameter, not a local current-size estimate.
        from_diameter: f64,
        /// Original target diameter.
        to_diameter: f64,
        /// Original signed duration with its native unit.
        duration: WorldBorderDuration,
    },
    /// Size fields in an actual initialize packet, including zero duration.
    Initialize {
        /// Original current diameter in that packet.
        from_diameter: f64,
        /// Original target diameter.
        to_diameter: f64,
        /// Original remaining signed duration with its native unit.
        duration: WorldBorderDuration,
    },
}
/// Received border fields, without inferring collision, damage or a render timer.
#[derive(Clone, Debug, serde::Serialize)]
pub struct WorldBorderObservation {
    /// Current connection/world provenance.
    pub session: SessionStamp,
    /// Coherent receive boundary.
    pub receive_sequence: u64,
    /// Actual context-reset packet ordinal, separate from UI field receipts.
    /// None means no connection-level context reset has been received.
    pub context_reset_sequence: Option<u64>,
    /// Last actual border-family event.
    pub last_update_sequence: Option<u64>,
    /// Received center x/z.
    pub center: Option<ObservedValue<[f64; 2]>>,
    /// Received size or transition command.
    pub size: Option<ObservedValue<WorldBorderSize>>,
    /// Absolute limit from an actual initialize packet.
    pub absolute_max_size: Option<ObservedValue<i32>>,
    /// Received warning time in seconds.
    pub warning_delay: Option<ObservedValue<i32>>,
    /// Received warning distance in blocks.
    pub warning_distance: Option<ObservedValue<i32>>,
}
impl crate::client::Client {
    /// Inspect title/subtitle/action-bar and clear/timing receipts without I/O.
    pub async fn titles(&self) -> Result<TitlesObservation> {
        crate::client::dispatch!(&self.adapter, a => UiOps::titles(a).await)
    }
    /// Inspect the last complete native tab-list header/footer pair.
    pub async fn tab_list(&self) -> Result<TabListObservation> {
        crate::client::dispatch!(&self.adapter, a => UiOps::tab_list(a).await)
    }
    /// Inspect border instructions without predicting current size or constraints.
    pub async fn world_border(&self) -> Result<WorldBorderObservation> {
        crate::client::dispatch!(&self.adapter, a => UiOps::world_border(a).await)
    }
}
#[derive(Clone, Default)]
pub(crate) struct DisplayLedger {
    context_reset_sequence: Option<u64>,
    title: Option<ObservedValue<Option<UiText>>>,
    subtitle: Option<ObservedValue<Option<UiText>>>,
    action_bar: Option<ObservedValue<UiText>>,
    timing: Option<ObservedValue<TitleTiming>>,
    clear: Option<ObservedValue<bool>>,
    title_sequence: Option<u64>,
    tab: Option<ObservedValue<TabListText>>,
    center: Option<ObservedValue<[f64; 2]>>,
    size: Option<ObservedValue<WorldBorderSize>>,
    absolute_max_size: Option<ObservedValue<i32>>,
    warning_delay: Option<ObservedValue<i32>>,
    warning_distance: Option<ObservedValue<i32>>,
    border_sequence: Option<u64>,
    border_generation: Option<u64>,
}
enum Update {
    Title(UiText),
    Subtitle(UiText),
    ActionBar(UiText),
    Timing(TitleTiming),
    Clear(bool),
    Tab(TabListText),
    Center([f64; 2]),
    Size(WorldBorderSize),
    Delay(i32),
    Distance(i32),
    Initialize {
        center: [f64; 2],
        size: WorldBorderSize,
        absolute: i32,
        delay: i32,
        distance: i32,
    },
}
impl DisplayLedger {
    // Original Gui.onDisconnected resets titles/times, tab and boss overlay,
    // but does not erase the last action-bar message. Keep its actual origin.
    pub(crate) fn reset_context(&mut self, sequence: u64) {
        let action_bar = self.action_bar.take();
        let title_sequence = action_bar.as_ref().and_then(|v| match v.source {
            crate::client::ValueSource::Received { sequence } => Some(sequence),
            _ => None,
        });
        *self = Self {
            context_reset_sequence: Some(sequence),
            action_bar,
            title_sequence,
            ..Self::default()
        };
    }

    pub(crate) fn receive(
        &mut self,
        version: MinecraftVersion,
        id: i32,
        payload: &[u8],
        sequence: u64,
        world_generation: u64,
    ) -> Result<()> {
        use crate::versions::java_1_21_11::ClientboundIds as ids;
        let modern = version == MinecraftVersion::Java1_21_11;
        let mut r = crate::versions::java_1_21_11::ScoreboardReader::new(payload);
        let update = if !modern {
            match id {
                0x4f => match r.varint()? {
                    0 => Update::Title(super::text(&mut r, false)?),
                    1 => Update::Subtitle(super::text(&mut r, false)?),
                    2 => Update::ActionBar(super::text(&mut r, false)?),
                    3 => Update::Timing(timing(&mut r)?),
                    4 => Update::Clear(false),
                    5 => Update::Clear(true),
                    _ => return Err(invalid("invalid title operation")),
                },
                0x53 => Update::Tab(TabListText {
                    header: super::text(&mut r, false)?,
                    footer: super::text(&mut r, false)?,
                }),
                0x3d => match r.varint()? {
                    0 => Update::Size(WorldBorderSize::Set { diameter: r.f64()? }),
                    1 => Update::Size(lerp(&mut r, false, modern)?),
                    2 => Update::Center([r.f64()?, r.f64()?]),
                    3 => initialize(&mut r, modern)?,
                    4 => Update::Delay(r.varint()?),
                    5 => Update::Distance(r.varint()?),
                    _ => return Err(invalid("invalid border operation")),
                },
                _ => return Err(invalid("unsupported display packet")),
            }
        } else {
            match id {
                ids::SET_TITLE_TEXT => Update::Title(super::text(&mut r, true)?),
                ids::SET_TITLE_SUBTITLE => Update::Subtitle(super::text(&mut r, true)?),
                ids::ACTION_BAR => Update::ActionBar(super::text(&mut r, true)?),
                ids::SET_TITLE_TIME => Update::Timing(timing(&mut r)?),
                ids::CLEAR_TITLES => Update::Clear(r.bool()?),
                ids::PLAYERLIST_HEADER => Update::Tab(TabListText {
                    header: super::text(&mut r, true)?,
                    footer: super::text(&mut r, true)?,
                }),
                ids::WORLD_BORDER_CENTER => Update::Center([r.f64()?, r.f64()?]),
                ids::WORLD_BORDER_SIZE => Update::Size(WorldBorderSize::Set { diameter: r.f64()? }),
                ids::WORLD_BORDER_LERP_SIZE => Update::Size(lerp(&mut r, false, modern)?),
                ids::INITIALIZE_WORLD_BORDER => initialize(&mut r, modern)?,
                ids::WORLD_BORDER_WARNING_DELAY => Update::Delay(r.varint()?),
                ids::WORLD_BORDER_WARNING_REACH => Update::Distance(r.varint()?),
                _ => return Err(invalid("unsupported display packet")),
            }
        };
        r.end()?;
        if matches!(
            &update,
            Update::Center(_)
                | Update::Size(_)
                | Update::Delay(_)
                | Update::Distance(_)
                | Update::Initialize { .. }
        ) {
            if self.border_generation != Some(world_generation) {
                self.center = None;
                self.size = None;
                self.absolute_max_size = None;
                self.warning_delay = None;
                self.warning_distance = None;
            }
            self.border_generation = Some(world_generation);
        }
        match update {
            Update::Title(text) => {
                self.title = Some(received(Some(text), sequence));
                self.title_sequence = Some(sequence);
            }
            Update::Subtitle(text) => {
                self.subtitle = Some(received(Some(text), sequence));
                self.title_sequence = Some(sequence);
            }
            Update::ActionBar(text) => {
                self.action_bar = Some(received(text, sequence));
                self.title_sequence = Some(sequence);
            }
            Update::Timing(timing) => {
                self.timing = Some(received(timing, sequence));
                self.title_sequence = Some(sequence);
            }
            Update::Clear(reset) => {
                self.title = Some(received(None, sequence));
                self.subtitle = Some(received(None, sequence));
                self.clear = Some(received(reset, sequence));
                self.title_sequence = Some(sequence);
                if reset {
                    self.timing = Some(received(TitleTiming::ResetToDefaults, sequence));
                }
            }
            Update::Tab(text) => self.tab = Some(received(text, sequence)),
            Update::Center(center) => {
                self.center = Some(received(center, sequence));
                self.border_sequence = Some(sequence);
            }
            Update::Size(size) => {
                self.size = Some(received(size, sequence));
                self.border_sequence = Some(sequence);
            }
            Update::Delay(delay) => {
                self.warning_delay = Some(received(delay, sequence));
                self.border_sequence = Some(sequence);
            }
            Update::Distance(distance) => {
                self.warning_distance = Some(received(distance, sequence));
                self.border_sequence = Some(sequence);
            }
            Update::Initialize {
                center,
                size,
                absolute,
                delay,
                distance,
            } => {
                self.center = Some(received(center, sequence));
                self.size = Some(received(size, sequence));
                self.absolute_max_size = Some(received(absolute, sequence));
                self.warning_delay = Some(received(delay, sequence));
                self.warning_distance = Some(received(distance, sequence));
                self.border_sequence = Some(sequence);
            }
        }
        Ok(())
    }
    pub(crate) fn titles(&self, session: SessionStamp, receive_sequence: u64) -> TitlesObservation {
        TitlesObservation {
            session,
            receive_sequence,
            context_reset_sequence: self.context_reset_sequence,
            last_update_sequence: self.title_sequence,
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            action_bar: self.action_bar.clone(),
            timing: self.timing.clone(),
            clear: self.clear.clone(),
        }
    }
    pub(crate) fn tab_list(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> TabListObservation {
        TabListObservation {
            session,
            receive_sequence,
            context_reset_sequence: self.context_reset_sequence,
            text: self.tab.clone(),
        }
    }
    pub(crate) fn world_border(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> WorldBorderObservation {
        let empty = Self::default();
        let view = if self.border_generation == Some(session.world_generation) {
            self
        } else {
            &empty
        };
        WorldBorderObservation {
            session,
            receive_sequence,
            context_reset_sequence: self.context_reset_sequence,
            last_update_sequence: view.border_sequence,
            center: view.center.clone(),
            size: view.size.clone(),
            absolute_max_size: view.absolute_max_size.clone(),
            warning_delay: view.warning_delay.clone(),
            warning_distance: view.warning_distance.clone(),
        }
    }
}
fn invalid(message: &str) -> crate::Error {
    super::super::recording::invalid(message)
}
fn timing(r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>) -> Result<TitleTiming> {
    Ok(TitleTiming::Set {
        fade_in: r.i32()?,
        stay: r.i32()?,
        fade_out: r.i32()?,
    })
}
fn varlong(r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>) -> Result<i64> {
    let mut value = 0u64;
    for i in 0..10 {
        let b = r.u8()?;
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value as i64);
        }
    }
    Err(invalid("overlong border duration"))
}
fn lerp(
    r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>,
    initialize: bool,
    modern: bool,
) -> Result<WorldBorderSize> {
    let from_diameter = r.f64()?;
    let to_diameter = r.f64()?;
    let value = varlong(r)?;
    let duration = if modern {
        WorldBorderDuration::Ticks(value)
    } else {
        WorldBorderDuration::Milliseconds(value)
    };
    Ok(if initialize {
        WorldBorderSize::Initialize {
            from_diameter,
            to_diameter,
            duration,
        }
    } else {
        WorldBorderSize::Lerp {
            from_diameter,
            to_diameter,
            duration,
        }
    })
}
fn initialize(
    r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>,
    modern: bool,
) -> Result<Update> {
    let center = [r.f64()?, r.f64()?];
    let size = lerp(r, true, modern)?;
    let absolute = r.varint()?;
    let first = r.varint()?;
    let second = r.varint()?;
    Ok(Update::Initialize {
        center,
        size,
        absolute,
        delay: second,
        distance: first,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(version: MinecraftVersion, generation: u64) -> SessionStamp {
        SessionStamp {
            connection_id: 81,
            version,
            world_generation: generation,
        }
    }
    fn raw(row: &serde_json::Value) -> Vec<u8> {
        row["payload_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    fn fixtures() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../data/client_api/display_packets.json"
        ))
        .unwrap()
    }
    fn apply(
        ledger: &mut DisplayLedger,
        version: MinecraftVersion,
        row: &serde_json::Value,
        seq: u64,
        generation: u64,
    ) {
        ledger
            .receive(
                version,
                row["packet_id"].as_i64().unwrap() as i32,
                &raw(row),
                seq,
                generation,
            )
            .unwrap();
    }
    fn text(value: &UiText, literal: &serde_json::Value) {
        let literal = literal.as_str().unwrap();
        match value {
            UiText::LegacyJson { json } => assert_eq!(
                serde_json::from_str::<serde_json::Value>(json).unwrap()["text"],
                literal
            ),
            UiText::NativeNbt { bytes } => {
                assert_eq!(bytes[0], 8);
                assert_eq!(&bytes[3..], literal.as_bytes());
            }
            UiText::Unavailable => panic!("original text encoding missing"),
        }
    }
    fn snapshot(
        ledger: &DisplayLedger,
        version: MinecraftVersion,
        generation: u64,
    ) -> serde_json::Value {
        serde_json::json!({"titles":ledger.titles(session(version,generation),40),"tab":ledger.tab_list(session(version,generation),40),"border":ledger.world_border(session(version,generation),40)})
    }
    #[test]
    fn display_original_codecs_preserve_fields_signed_values_and_partial_origins() {
        for group in fixtures()["versions"].as_array().unwrap() {
            let version = if group["version"] == "1.16.1" {
                MinecraftVersion::Java1_16_1
            } else {
                MinecraftVersion::Java1_21_11
            };
            let rows = group["packets"].as_array().unwrap();
            for row in rows {
                let mut ledger = DisplayLedger::default();
                for initial in rows
                    .iter()
                    .filter(|r| r["group"] == "title" && r["operation"].as_u64().unwrap() < 4)
                {
                    apply(&mut ledger, version, initial, 2, 3);
                }
                let initial = rows
                    .iter()
                    .find(|r| r["group"] == "border" && r["operation"] == 3)
                    .unwrap();
                apply(&mut ledger, version, initial, 2, 3);
                let before_title = ledger.titles(session(version, 3), 2);
                let before_border = ledger.world_border(session(version, 3), 2);
                apply(&mut ledger, version, row, 9, 3);
                let titles = ledger.titles(session(version, 3), 12);
                let border = ledger.world_border(session(version, 3), 12);
                let op = row["operation"].as_u64().unwrap();
                let source = crate::client::ValueSource::Received { sequence: 9 };
                match row["group"].as_str().unwrap() {
                    "title" => {
                        assert_eq!(titles.last_update_sequence, Some(9));
                        match op {
                            0 => {
                                let received = titles.title.as_ref().unwrap();
                                text(received.value.as_ref().unwrap(), &row["text"]);
                                assert_eq!(received.source, source);
                                assert_eq!(titles.subtitle, before_title.subtitle);
                            }
                            1 => {
                                let received = titles.subtitle.as_ref().unwrap();
                                text(received.value.as_ref().unwrap(), &row["text"]);
                                assert_eq!(received.source, source);
                                assert_eq!(titles.title, before_title.title);
                            }
                            2 => {
                                let received = titles.action_bar.as_ref().unwrap();
                                text(&received.value, &row["text"]);
                                assert_eq!(received.source, source);
                                assert_eq!(titles.title, before_title.title);
                            }
                            3 => {
                                assert_eq!(
                                    titles.timing.as_ref().unwrap().value,
                                    TitleTiming::Set {
                                        fade_in: row["fade_in"].as_i64().unwrap() as i32,
                                        stay: row["stay"].as_i64().unwrap() as i32,
                                        fade_out: row["fade_out"].as_i64().unwrap() as i32
                                    }
                                );
                                assert_eq!(titles.timing.as_ref().unwrap().source, source);
                            }
                            4 | 5 => {
                                assert_eq!(
                                    titles.clear.as_ref().unwrap().value,
                                    row["reset_times"].as_bool().unwrap()
                                );
                                assert_eq!(titles.title.as_ref().unwrap().value, None);
                                assert_eq!(titles.subtitle.as_ref().unwrap().source, source);
                                assert_eq!(titles.action_bar, before_title.action_bar);
                                if op == 4 {
                                    assert_eq!(titles.timing, before_title.timing);
                                } else {
                                    assert_eq!(
                                        titles.timing.as_ref().unwrap().value,
                                        TitleTiming::ResetToDefaults
                                    );
                                }
                            }
                            _ => panic!(),
                        }
                        assert_eq!(border.center, before_border.center);
                        assert_eq!(border.size, before_border.size);
                        assert_eq!(border.warning_delay, before_border.warning_delay);
                        assert_eq!(border.warning_distance, before_border.warning_distance);
                    }
                    "tab" => {
                        let tab = ledger.tab_list(session(version, 3), 12);
                        let pair = tab.text.unwrap();
                        assert_eq!(pair.source, source);
                        text(&pair.value.header, &row["header"]);
                        text(&pair.value.footer, &row["footer"]);
                    }
                    "border" => {
                        assert_eq!(border.last_update_sequence, Some(9));
                        if op == 0 {
                            assert_eq!(
                                border.size.as_ref().unwrap().value,
                                WorldBorderSize::Set {
                                    diameter: row["diameter"].as_f64().unwrap()
                                }
                            );
                        }
                        if op == 1 || op == 3 {
                            let from_diameter = row["from_diameter"].as_f64().unwrap();
                            let to_diameter = row["to_diameter"].as_f64().unwrap();
                            let value = row["duration"]["value"].as_i64().unwrap();
                            let duration = if version == MinecraftVersion::Java1_21_11 {
                                WorldBorderDuration::Ticks(value)
                            } else {
                                WorldBorderDuration::Milliseconds(value)
                            };
                            assert_eq!(serde_json::to_value(duration).unwrap(), row["duration"]);
                            let nominal = if version == MinecraftVersion::Java1_21_11 {
                                match value {
                                    0 => Some(0),
                                    10000 => Some(500_000),
                                    35_184_372_088_832 => Some(1_759_218_604_441_600),
                                    i64::MAX => None,
                                    -1 => Some(-50),
                                    _ => panic!("unexpected original duration sample"),
                                }
                            } else {
                                Some(value)
                            };
                            assert_eq!(duration.nominal_milliseconds(), nominal);
                            assert_eq!(
                                border.size.as_ref().unwrap().value,
                                if op == 3 {
                                    WorldBorderSize::Initialize {
                                        from_diameter,
                                        to_diameter,
                                        duration,
                                    }
                                } else {
                                    WorldBorderSize::Lerp {
                                        from_diameter,
                                        to_diameter,
                                        duration,
                                    }
                                }
                            );
                            assert_eq!(border.size.as_ref().unwrap().source, source);
                        }
                        if op == 2 || op == 3 {
                            assert_eq!(
                                border.center.as_ref().unwrap().value,
                                [
                                    row["center_x"].as_f64().unwrap(),
                                    row["center_z"].as_f64().unwrap()
                                ]
                            );
                            assert_eq!(border.center.as_ref().unwrap().source, source);
                        } else {
                            assert_eq!(border.center, before_border.center);
                        }
                        if op == 3 {
                            assert_eq!(
                                border.absolute_max_size.as_ref().unwrap().value,
                                row["absolute_max_size"].as_i64().unwrap() as i32
                            );
                        }
                        if op == 3 || op == 4 {
                            assert_eq!(
                                border.warning_delay.as_ref().unwrap().value,
                                row["warning_delay"].as_i64().unwrap() as i32
                            );
                            assert_eq!(border.warning_delay.as_ref().unwrap().source, source);
                        } else {
                            assert_eq!(border.warning_delay, before_border.warning_delay);
                        }
                        if op == 3 || op == 5 {
                            assert_eq!(
                                border.warning_distance.as_ref().unwrap().value,
                                row["warning_distance"].as_i64().unwrap() as i32
                            );
                            assert_eq!(border.warning_distance.as_ref().unwrap().source, source);
                        } else {
                            assert_eq!(border.warning_distance, before_border.warning_distance);
                        }
                    }
                    _ => panic!(),
                }
            }
        }
    }
    #[test]
    fn display_malformed_packets_are_atomic_and_border_never_rebinds_to_new_world() {
        for group in fixtures()["versions"].as_array().unwrap() {
            let version = if group["version"] == "1.16.1" {
                MinecraftVersion::Java1_16_1
            } else {
                MinecraftVersion::Java1_21_11
            };
            let rows = group["packets"].as_array().unwrap();
            let mut ledger = DisplayLedger::default();
            assert!(ledger.titles(session(version, 3), 0).timing.is_none());
            assert!(ledger.tab_list(session(version, 3), 0).text.is_none());
            assert!(ledger.world_border(session(version, 3), 0).size.is_none());
            for row in rows {
                apply(&mut ledger, version, row, 2, 3);
            }
            let before = snapshot(&ledger, version, 3);
            for row in rows {
                let bytes = raw(row);
                let id = row["packet_id"].as_i64().unwrap() as i32;
                for n in 0..bytes.len() {
                    assert!(ledger.receive(version, id, &bytes[..n], 8, 3).is_err());
                    assert_eq!(snapshot(&ledger, version, 3), before);
                }
                let mut trailing = bytes;
                trailing.push(0);
                assert!(ledger.receive(version, id, &trailing, 8, 3).is_err());
                assert_eq!(snapshot(&ledger, version, 3), before);
            }
            if version == MinecraftVersion::Java1_21_11 {
                assert!(ledger.receive(version, 0x0e, &[2], 8, 3).is_err());
            } else {
                assert!(ledger.receive(version, 0x4f, &[6], 8, 3).is_err());
                assert!(ledger.receive(version, 0x3d, &[6], 8, 3).is_err());
            }
            assert_eq!(snapshot(&ledger, version, 3), before);
            let unknown = ledger.world_border(session(version, 4), 40);
            assert!(
                unknown.center.is_none()
                    && unknown.size.is_none()
                    && unknown.warning_delay.is_none()
            );
            let warning = rows
                .iter()
                .find(|r| r["group"] == "border" && r["operation"] == 4)
                .unwrap();
            apply(&mut ledger, version, warning, 11, 4);
            let fresh = ledger.world_border(session(version, 4), 40);
            assert!(
                fresh.center.is_none() && fresh.size.is_none() && fresh.absolute_max_size.is_none()
            );
            assert_eq!(
                fresh.warning_delay.unwrap().source,
                crate::client::ValueSource::Received { sequence: 11 }
            );
            assert_eq!(fresh.last_update_sequence, Some(11));
        }
    }
}
