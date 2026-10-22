use super::UiReader;
use crate::Result;
use crate::client::recording::invalid;
use crate::client::ui::{TabListText, TitleTiming, UiText, WorldBorderDuration, WorldBorderSize};
#[derive(Clone)]
pub(crate) enum DisplayUpdate {
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
pub(super) fn decode<'a, R: UiReader<'a>>(id: i32, payload: &'a [u8]) -> Result<DisplayUpdate> {
    use crate::versions::java_1_21_11::ClientboundIds as ids;
    let modern = R::MODERN;
    let mut r = R::new(payload);
    let update = if !modern {
        match id {
            0x4f => match r.varint()? {
                0 => DisplayUpdate::Title(r.text()?),
                1 => DisplayUpdate::Subtitle(r.text()?),
                2 => DisplayUpdate::ActionBar(r.text()?),
                3 => DisplayUpdate::Timing(timing(&mut r)?),
                4 => DisplayUpdate::Clear(false),
                5 => DisplayUpdate::Clear(true),
                _ => return Err(invalid("invalid title operation")),
            },
            0x53 => DisplayUpdate::Tab(TabListText {
                header: r.text()?,
                footer: r.text()?,
            }),
            0x3d => match r.varint()? {
                0 => DisplayUpdate::Size(WorldBorderSize::Set { diameter: r.f64()? }),
                1 => DisplayUpdate::Size(lerp(&mut r, false, modern)?),
                2 => DisplayUpdate::Center([r.f64()?, r.f64()?]),
                3 => initialize(&mut r, modern)?,
                4 => DisplayUpdate::Delay(r.varint()?),
                5 => DisplayUpdate::Distance(r.varint()?),
                _ => return Err(invalid("invalid border operation")),
            },
            _ => return Err(invalid("unsupported display packet")),
        }
    } else {
        match id {
            ids::SET_TITLE_TEXT => DisplayUpdate::Title(r.text()?),
            ids::SET_TITLE_SUBTITLE => DisplayUpdate::Subtitle(r.text()?),
            ids::ACTION_BAR => DisplayUpdate::ActionBar(r.text()?),
            ids::SET_TITLE_TIME => DisplayUpdate::Timing(timing(&mut r)?),
            ids::CLEAR_TITLES => DisplayUpdate::Clear(r.bool()?),
            ids::PLAYERLIST_HEADER => DisplayUpdate::Tab(TabListText {
                header: r.text()?,
                footer: r.text()?,
            }),
            ids::WORLD_BORDER_CENTER => DisplayUpdate::Center([r.f64()?, r.f64()?]),
            ids::WORLD_BORDER_SIZE => {
                DisplayUpdate::Size(WorldBorderSize::Set { diameter: r.f64()? })
            }
            ids::WORLD_BORDER_LERP_SIZE => DisplayUpdate::Size(lerp(&mut r, false, modern)?),
            ids::INITIALIZE_WORLD_BORDER => initialize(&mut r, modern)?,
            ids::WORLD_BORDER_WARNING_DELAY => DisplayUpdate::Delay(r.varint()?),
            ids::WORLD_BORDER_WARNING_REACH => DisplayUpdate::Distance(r.varint()?),
            _ => return Err(invalid("unsupported display packet")),
        }
    };
    r.end()?;
    Ok(update)
}
fn timing<'a, R: UiReader<'a>>(r: &mut R) -> Result<TitleTiming> {
    Ok(TitleTiming::Set {
        fade_in: r.i32()?,
        stay: r.i32()?,
        fade_out: r.i32()?,
    })
}
fn varlong<'a, R: UiReader<'a>>(r: &mut R) -> Result<i64> {
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
fn lerp<'a, R: UiReader<'a>>(r: &mut R, initialize: bool, modern: bool) -> Result<WorldBorderSize> {
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
fn initialize<'a, R: UiReader<'a>>(r: &mut R, modern: bool) -> Result<DisplayUpdate> {
    let center = [r.f64()?, r.f64()?];
    let size = lerp(r, true, modern)?;
    let absolute = r.varint()?;
    let first = r.varint()?;
    let second = r.varint()?;
    Ok(DisplayUpdate::Initialize {
        center,
        size,
        absolute,
        delay: second,
        distance: first,
    })
}
