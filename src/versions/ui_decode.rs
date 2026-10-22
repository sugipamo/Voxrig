//! Private UI decoders. Ledgers own retention; adapter readers select wire layouts and text framing.
mod boss_bar;
mod display;
mod player_list;
mod scoreboard;
mod teams;
use crate::client::ui::UiText;
use crate::{MinecraftVersion, Result};
pub(crate) use boss_bar::BossBarUpdate;
pub(crate) use display::DisplayUpdate;
pub(crate) use player_list::{PlayerChanges, PlayerPacket};
pub(crate) use scoreboard::ScoreboardUpdate;
pub(crate) use teams::TeamPacket;

// The concrete adapter reader selects its layout at compile time. Only byte
// primitives are shared; legacy text never passes through the NBT reader.
pub(crate) trait UiReader<'a>:
    std::ops::DerefMut<Target = super::fields::Reader<'a>> + Sized
{
    const MODERN: bool;
    fn new(payload: &'a [u8]) -> Self;
    fn text(&mut self) -> Result<UiText>;
}
pub(crate) fn decode_scoreboard(
    version: MinecraftVersion,
    id: i32,
    payload: &[u8],
) -> Result<ScoreboardUpdate> {
    match version {
        MinecraftVersion::Java1_16_1 => {
            scoreboard::decode::<super::java_1_16_1::ui_decode::Reader<'_>>(id, payload)
        }
        MinecraftVersion::Java1_21_11 => {
            scoreboard::decode::<super::java_1_21_11::ScoreboardReader<'_>>(id, payload)
        }
    }
}
pub(crate) fn decode_display(
    version: MinecraftVersion,
    id: i32,
    payload: &[u8],
) -> Result<DisplayUpdate> {
    match version {
        MinecraftVersion::Java1_16_1 => {
            display::decode::<super::java_1_16_1::ui_decode::Reader<'_>>(id, payload)
        }
        MinecraftVersion::Java1_21_11 => {
            display::decode::<super::java_1_21_11::ScoreboardReader<'_>>(id, payload)
        }
    }
}
pub(crate) fn decode_player_list(
    version: MinecraftVersion,
    id: i32,
    payload: &[u8],
) -> Result<PlayerPacket> {
    match version {
        MinecraftVersion::Java1_16_1 => {
            player_list::decode::<super::java_1_16_1::ui_decode::Reader<'_>>(id, payload)
        }
        MinecraftVersion::Java1_21_11 => {
            player_list::decode::<super::java_1_21_11::ScoreboardReader<'_>>(id, payload)
        }
    }
}
pub(crate) fn decode_boss_bar(
    version: MinecraftVersion,
    payload: &[u8],
) -> Result<([u8; 16], BossBarUpdate)> {
    match version {
        MinecraftVersion::Java1_16_1 => {
            boss_bar::decode::<super::java_1_16_1::ui_decode::Reader<'_>>(payload)
        }
        MinecraftVersion::Java1_21_11 => {
            boss_bar::decode::<super::java_1_21_11::ScoreboardReader<'_>>(payload)
        }
    }
}
pub(crate) fn decode_team(version: MinecraftVersion, payload: &[u8]) -> Result<TeamPacket> {
    match version {
        MinecraftVersion::Java1_16_1 => {
            teams::decode::<super::java_1_16_1::ui_decode::Reader<'_>>(payload)
        }
        MinecraftVersion::Java1_21_11 => {
            teams::decode::<super::java_1_21_11::ScoreboardReader<'_>>(payload)
        }
    }
}

#[cfg(test)]
mod tests;
