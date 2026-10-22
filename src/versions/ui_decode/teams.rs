use super::UiReader;
use crate::Result;
use crate::client::recording::invalid;
use crate::client::ui::{TeamCollision, TeamColor, TeamParameters, TeamVisibility};
#[derive(Clone)]
pub(crate) struct TeamPacket {
    pub name: String,
    pub operation: u8,
    pub parameters: Option<TeamParameters>,
    pub members: Vec<String>,
}
pub(super) fn decode<'a, R: UiReader<'a>>(payload: &'a [u8]) -> Result<TeamPacket> {
    let modern = R::MODERN;
    let mut r = R::new(payload);
    let name = r.string()?;
    let operation = r.u8()?;
    if operation > 4 {
        return Err(invalid("unknown team operation"));
    }
    let parameters = if operation == 0 || operation == 2 {
        let display = r.text()?;
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
        let color = color(r.varint()?)?;
        let prefix = r.text()?;
        let suffix = r.text()?;
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
fn color(id: i32) -> Result<TeamColor> {
    Ok(match id {
        0 => TeamColor::Black,
        1 => TeamColor::DarkBlue,
        2 => TeamColor::DarkGreen,
        3 => TeamColor::DarkAqua,
        4 => TeamColor::DarkRed,
        5 => TeamColor::DarkPurple,
        6 => TeamColor::Gold,
        7 => TeamColor::Gray,
        8 => TeamColor::DarkGray,
        9 => TeamColor::Blue,
        10 => TeamColor::Green,
        11 => TeamColor::Aqua,
        12 => TeamColor::Red,
        13 => TeamColor::LightPurple,
        14 => TeamColor::Yellow,
        15 => TeamColor::White,
        21 => TeamColor::Reset,
        _ => return Err(invalid("unsupported team color")),
    })
}
