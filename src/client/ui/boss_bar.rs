//! Actual boss-bar fields. Partial updates never refresh unrelated receipts.
use super::{ObservedValue, SessionStamp, UiText, received};
use crate::client::adapter::UiOps;
use crate::{MinecraftVersion, Result};
use std::collections::BTreeMap;

/// Native boss-bar color, independent from text formatting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BossBarColor {
    /// Pink.
    Pink,
    /// Blue.
    Blue,
    /// Red.
    Red,
    /// Green.
    Green,
    /// Yellow.
    Yellow,
    /// Purple.
    Purple,
    /// White.
    White,
}
/// Native visual division style, without rendering the bar.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BossBarOverlay {
    /// Continuous bar.
    Progress,
    /// Six divisions.
    Notched6,
    /// Ten divisions.
    Notched10,
    /// Twelve divisions.
    Notched12,
    /// Twenty divisions.
    Notched20,
}
/// Original property byte; unused bits remain available as raw facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct BossBarFlags {
    /// Original complete property byte.
    pub raw: u8,
}
impl BossBarFlags {
    /// Received darken-screen flag, not an applied visual effect.
    pub fn darken_screen(self) -> bool {
        self.raw & 1 != 0
    }
    /// Received boss-music flag.
    pub fn play_music(self) -> bool {
        self.raw & 2 != 0
    }
    /// Received world-fog flag.
    pub fn create_fog(self) -> bool {
        self.raw & 4 != 0
    }
}
/// One actual ADD followed by actual field updates on this connection.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ReceivedBossBar {
    /// Native UUID bytes in network order. Read-only, not an operation token.
    pub uuid: [u8; 16],
    /// Original legacy JSON or modern unnamed NBT component.
    pub title: ObservedValue<UiText>,
    /// Original finite progress float; never clamped or inferred as entity health.
    pub progress: ObservedValue<f32>,
    /// Actual color receipt.
    pub color: ObservedValue<BossBarColor>,
    /// Actual division-style receipt.
    pub overlay: ObservedValue<BossBarOverlay>,
    /// Actual property receipt.
    pub flags: ObservedValue<BossBarFlags>,
}
/// Surviving received bars, sorted by native UUID. Not a server catalogue.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BossBarsObservation {
    /// Current connection/world provenance.
    pub session: SessionStamp,
    /// Coherent receive boundary, not a server tick.
    pub receive_sequence: u64,
    /// Actual context-reset packet ordinal, separate from UI field receipts.
    /// None means no connection-level context reset has been received.
    pub context_reset_sequence: Option<u64>,
    /// Last actual bar event, including REMOVE and ignored unknown-UUID updates.
    /// None means no bar packet has been received.
    pub last_update_sequence: Option<u64>,
    /// Complete bars established by actual ADD packets.
    pub bars: Vec<ReceivedBossBar>,
}
impl crate::client::Client {
    /// Read boss-bar ADD/update/REMOVE receipts without network I/O or rendering.
    pub async fn boss_bars(&self) -> Result<BossBarsObservation> {
        crate::client::dispatch!(&self.adapter, a => UiOps::boss_bars(a).await)
    }
}
#[derive(Clone, Default)]
pub(crate) struct BossBarLedger {
    context_reset_sequence: Option<u64>,
    bars: BTreeMap<[u8; 16], ReceivedBossBar>,
    last_update_sequence: Option<u64>,
}
enum Update {
    Add(ReceivedBossBar),
    Remove,
    Progress(f32),
    Title(UiText),
    Style(BossBarColor, BossBarOverlay),
    Flags(BossBarFlags),
}
impl BossBarLedger {
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
        let mut r = crate::versions::java_1_21_11::ScoreboardReader::new(payload);
        let uuid: [u8; 16] = r.take(16)?.try_into().expect("fixed UUID field");
        let modern = version == MinecraftVersion::Java1_21_11;
        let update = match r.varint()? {
            0 => Update::Add(ReceivedBossBar {
                uuid,
                title: received(super::text(&mut r, modern)?, sequence),
                progress: received(r.f32()?, sequence),
                color: received(color(r.varint()?)?, sequence),
                overlay: received(overlay(r.varint()?)?, sequence),
                flags: received(BossBarFlags { raw: r.u8()? }, sequence),
            }),
            1 => Update::Remove,
            2 => Update::Progress(r.f32()?),
            3 => Update::Title(super::text(&mut r, modern)?),
            4 => Update::Style(color(r.varint()?)?, overlay(r.varint()?)?),
            5 => Update::Flags(BossBarFlags { raw: r.u8()? }),
            _ => {
                return Err(super::super::recording::invalid(
                    "invalid boss bar operation",
                ));
            }
        };
        r.end()?;
        match update {
            Update::Add(bar) => {
                if !self.bars.contains_key(&uuid) && self.bars.len() >= 4096 {
                    return Err(crate::Error::new(
                        crate::ErrorKind::ResourceLimit,
                        anyhow::anyhow!("boss bars exceed 4096 received entries"),
                    ));
                }
                self.bars.insert(uuid, bar);
            }
            Update::Remove => {
                self.bars.remove(&uuid);
            }
            change => {
                if let Some(bar) = self.bars.get_mut(&uuid) {
                    match change {
                        Update::Progress(value) => bar.progress = received(value, sequence),
                        Update::Title(value) => bar.title = received(value, sequence),
                        Update::Style(color, overlay) => {
                            bar.color = received(color, sequence);
                            bar.overlay = received(overlay, sequence);
                        }
                        Update::Flags(value) => bar.flags = received(value, sequence),
                        Update::Add(_) | Update::Remove => unreachable!(),
                    }
                }
            }
        }
        self.last_update_sequence = Some(sequence);
        Ok(())
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> BossBarsObservation {
        BossBarsObservation {
            session,
            receive_sequence,
            context_reset_sequence: self.context_reset_sequence,
            last_update_sequence: self.last_update_sequence,
            bars: self.bars.values().cloned().collect(),
        }
    }
}
fn color(value: i32) -> Result<BossBarColor> {
    Ok(match value {
        0 => BossBarColor::Pink,
        1 => BossBarColor::Blue,
        2 => BossBarColor::Red,
        3 => BossBarColor::Green,
        4 => BossBarColor::Yellow,
        5 => BossBarColor::Purple,
        6 => BossBarColor::White,
        _ => return Err(super::super::recording::invalid("invalid boss bar color")),
    })
}
fn overlay(value: i32) -> Result<BossBarOverlay> {
    Ok(match value {
        0 => BossBarOverlay::Progress,
        1 => BossBarOverlay::Notched6,
        2 => BossBarOverlay::Notched10,
        3 => BossBarOverlay::Notched12,
        4 => BossBarOverlay::Notched20,
        _ => return Err(super::super::recording::invalid("invalid boss bar overlay")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(version: MinecraftVersion) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 17,
            world_generation: 3,
        }
    }
    fn bytes(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    fn fixtures() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../data/client_api/boss_bar_packets.json"
        ))
        .unwrap()
    }
    #[test]
    fn boss_bar_original_codecs_preserve_native_fields_and_partial_receipt_ordinals() {
        for row in fixtures()["versions"].as_array().unwrap() {
            let version = if row["version"] == "1.16.1" {
                MinecraftVersion::Java1_16_1
            } else {
                MinecraftVersion::Java1_21_11
            };
            let packets = row["packets"].as_array().unwrap();
            for case in packets {
                let mut ledger = BossBarLedger::default();
                ledger
                    .receive(
                        version,
                        &bytes(packets[0]["payload_hex"].as_str().unwrap()),
                        2,
                    )
                    .unwrap();
                let before = ledger.capture(session(version), 2).bars[0].clone();
                ledger
                    .receive(version, &bytes(case["payload_hex"].as_str().unwrap()), 5)
                    .unwrap();
                let observed = ledger.capture(session(version), 8);
                assert_eq!(observed.last_update_sequence, Some(5));
                let op = case["operation"].as_u64().unwrap();
                if op == 1 {
                    assert!(observed.bars.is_empty());
                    continue;
                }
                let bar = &observed.bars[0];
                assert_eq!(bar.uuid, std::array::from_fn(|n| (n + 1) as u8));
                let source = super::super::super::ValueSource::Received { sequence: 5 };
                if op == 0 || op == 3 {
                    assert_eq!(bar.title.source, source);
                    let literal = case["native_title_text"].as_str().unwrap();
                    match &bar.title.value {
                        UiText::LegacyJson { json } => assert_eq!(
                            serde_json::from_str::<serde_json::Value>(json).unwrap()["text"],
                            literal
                        ),
                        UiText::Unavailable => panic!("native text must retain its encoding"),
                        UiText::NativeNbt { bytes } => {
                            assert_eq!(bytes[0], 8);
                            assert_eq!(&bytes[3..], literal.as_bytes());
                        }
                    }
                } else {
                    assert_eq!(bar.title, before.title);
                }
                if op == 0 || op == 2 {
                    assert_eq!(
                        f64::from(bar.progress.value),
                        case["progress"].as_f64().unwrap()
                    );
                    assert_eq!(bar.progress.source, source);
                } else {
                    assert_eq!(bar.progress, before.progress);
                }
                if op == 0 || op == 4 {
                    assert_eq!(
                        bar.color.value,
                        color(case["color"].as_i64().unwrap() as i32).unwrap()
                    );
                    assert_eq!(
                        bar.overlay.value,
                        overlay(case["overlay"].as_i64().unwrap() as i32).unwrap()
                    );
                    assert_eq!(bar.color.source, source);
                    assert_eq!(bar.overlay.source, source);
                } else {
                    assert_eq!(bar.color, before.color);
                    assert_eq!(bar.overlay, before.overlay);
                }
                if op == 0 || op == 5 {
                    let flags = bar.flags.value;
                    assert_eq!(flags.darken_screen(), case["flags"][0].as_bool().unwrap());
                    assert_eq!(flags.play_music(), case["flags"][1].as_bool().unwrap());
                    assert_eq!(flags.create_fog(), case["flags"][2].as_bool().unwrap());
                    assert_eq!(bar.flags.source, source);
                } else {
                    assert_eq!(bar.flags, before.flags);
                }
            }
        }
    }
    #[test]
    fn boss_bar_malformed_packets_are_atomic_and_unknown_updates_never_create_defaults() {
        for row in fixtures()["versions"].as_array().unwrap() {
            let version = if row["version"] == "1.16.1" {
                MinecraftVersion::Java1_16_1
            } else {
                MinecraftVersion::Java1_21_11
            };
            let packets = row["packets"].as_array().unwrap();
            let mut ledger = BossBarLedger::default();
            assert_eq!(
                ledger.capture(session(version), 0).last_update_sequence,
                None
            );
            for case in &packets[1..6] {
                ledger
                    .receive(version, &bytes(case["payload_hex"].as_str().unwrap()), 1)
                    .unwrap();
                assert!(ledger.bars.is_empty());
            }
            ledger
                .receive(
                    version,
                    &bytes(packets[0]["payload_hex"].as_str().unwrap()),
                    2,
                )
                .unwrap();
            let before = serde_json::to_value(ledger.capture(session(version), 2)).unwrap();
            for case in packets {
                let raw = bytes(case["payload_hex"].as_str().unwrap());
                for n in 0..raw.len() {
                    assert!(ledger.receive(version, &raw[..n], 5).is_err());
                    assert_eq!(
                        serde_json::to_value(ledger.capture(session(version), 2)).unwrap(),
                        before
                    );
                }
                let mut trailing = raw;
                trailing.push(0);
                assert!(ledger.receive(version, &trailing, 5).is_err());
            }
            for tail in [
                vec![6],
                vec![4, 7, 0],
                vec![4, 0, 5],
                vec![2, 0x7f, 0x80, 0, 0],
            ] {
                let mut invalid = (1..=16).collect::<Vec<u8>>();
                invalid.extend(tail);
                assert!(ledger.receive(version, &invalid, 5).is_err());
                assert_eq!(
                    serde_json::to_value(ledger.capture(session(version), 2)).unwrap(),
                    before
                );
            }
            let raw = bytes(packets[0]["payload_hex"].as_str().unwrap());
            ledger.receive(version, &raw, 9).unwrap();
            assert_eq!(ledger.bars.len(), 1);
            assert_eq!(
                ledger.bars.values().next().unwrap().title.source,
                super::super::super::ValueSource::Received { sequence: 9 }
            );
        }
    }
}
