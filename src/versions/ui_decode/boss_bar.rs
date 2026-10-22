use super::UiReader;
use crate::Result;
use crate::client::recording::invalid;
use crate::client::ui::{BossBarColor, BossBarFlags, BossBarOverlay, UiText};
#[derive(Clone)]
pub(crate) enum BossBarUpdate {
    Add {
        title: UiText,
        progress: f32,
        color: BossBarColor,
        overlay: BossBarOverlay,
        flags: BossBarFlags,
    },
    Remove,
    Progress(f32),
    Title(UiText),
    Style(BossBarColor, BossBarOverlay),
    Flags(BossBarFlags),
}
pub(super) fn decode<'a, R: UiReader<'a>>(payload: &'a [u8]) -> Result<([u8; 16], BossBarUpdate)> {
    let mut r = R::new(payload);
    let uuid: [u8; 16] = r.take(16)?.try_into().expect("fixed UUID field");

    let update = match r.varint()? {
        0 => BossBarUpdate::Add {
            title: r.text()?,
            progress: r.f32()?,
            color: color(r.varint()?)?,
            overlay: overlay(r.varint()?)?,
            flags: BossBarFlags { raw: r.u8()? },
        },
        1 => BossBarUpdate::Remove,
        2 => BossBarUpdate::Progress(r.f32()?),
        3 => BossBarUpdate::Title(r.text()?),
        4 => BossBarUpdate::Style(color(r.varint()?)?, overlay(r.varint()?)?),
        5 => BossBarUpdate::Flags(BossBarFlags { raw: r.u8()? }),
        _ => {
            return Err(invalid("invalid boss bar operation"));
        }
    };
    r.end()?;
    Ok((uuid, update))
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
        _ => return Err(invalid("invalid boss bar color")),
    })
}
fn overlay(value: i32) -> Result<BossBarOverlay> {
    Ok(match value {
        0 => BossBarOverlay::Progress,
        1 => BossBarOverlay::Notched6,
        2 => BossBarOverlay::Notched10,
        3 => BossBarOverlay::Notched12,
        4 => BossBarOverlay::Notched20,
        _ => return Err(invalid("invalid boss bar overlay")),
    })
}
