use super::UiReader;
use crate::Result;
use crate::client::ui::{
    ScoreNumberFormat, ScoreboardObjective, ScoreboardRenderType, ScoreboardScore, UiText,
};
#[derive(Clone)]
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
pub(super) fn decode<'a, R: UiReader<'a>>(id: i32, payload: &'a [u8]) -> Result<ScoreboardUpdate> {
    let mut r = R::new(payload);
    let modern = R::MODERN;
    let objective_id = if modern { 0x68 } else { 0x4a };
    let display_id = if modern { 0x60 } else { 0x43 };
    let score_id = if modern { 0x6c } else { 0x4d };
    let update = if id == objective_id {
        let name = r.string()?;
        let action = r.u8()?;
        if action == 1 {
            ScoreboardUpdate::RemoveObjective(name)
        } else if action == 0 || action == 2 {
            let display = r.text()?;
            let render_type = match r.varint()? {
                0 => ScoreboardRenderType::Integer,
                1 => ScoreboardRenderType::Hearts,
                _ => {
                    return Err(crate::client::recording::invalid(
                        "invalid scoreboard render type",
                    ));
                }
            };
            let number_format = if modern { format(&mut r)? } else { None };
            ScoreboardUpdate::Objective(ScoreboardObjective {
                name,
                display,
                render_type,
                number_format,
            })
        } else {
            return Err(crate::client::recording::invalid(
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
            return Err(crate::client::recording::invalid(
                "invalid scoreboard display slot",
            ));
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
                _ => {
                    return Err(crate::client::recording::invalid(
                        "invalid scoreboard score action",
                    ));
                }
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
                Some(r.text()?)
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
        return Err(crate::client::recording::invalid(
            "not a supported native scoreboard packet",
        ));
    };
    r.end()?;
    Ok(update)
}
fn format<'a, R: UiReader<'a>>(r: &mut R) -> Result<Option<ScoreNumberFormat>> {
    if !r.bool()? {
        return Ok(None);
    }
    Ok(Some(match r.varint()? {
        0 => ScoreNumberFormat::Blank,
        1 => ScoreNumberFormat::Styled {
            bytes: match r.text()? {
                UiText::NativeNbt { bytes } => bytes,
                _ => unreachable!("modern score format"),
            },
        },
        2 => ScoreNumberFormat::Fixed { text: r.text()? },
        _ => {
            return Err(crate::client::recording::invalid(
                "unknown score number format requires an adapter update",
            ));
        }
    }))
}
