//! Legacy chat is decoded once, retaining native validation before common framing checks.
use super::chat::ChatMessage;
use crate::Result;
use crate::client::chat::{ChatKind, ChatText, ReceivedChat};
use crate::client::ui::UiText;
use crate::versions::fields::Reader;
use anyhow::Context;

pub(crate) fn decode_chat(payload: &[u8], sequence: u64) -> Result<(ReceivedChat, ChatMessage)> {
    let mut r = Reader::new(payload);
    let json = r.string()?;
    // The native parser historically ran first. Keep its errors for these two
    // fields; the common parser then checked trailing data before placement.
    let position = *r.remaining().first().context("missing chat position")?;
    r.u8()?;
    if r.remaining().len() < 16 {
        return Err(anyhow::anyhow!("truncated player UUID").into());
    }
    let sender: [u8; 16] = r.take(16)?.try_into().expect("fixed UUID field");
    r.end()?;
    let kind = match position {
        0 => ChatKind::Player,
        1 => ChatKind::System,
        2 => ChatKind::ActionBar,
        _ => return Err(crate::client::recording::invalid("unknown chat position")),
    };
    let message = ReceivedChat {
        receive_sequence: sequence,
        kind,
        sender: (sender != [0; 16]).then_some(sender),
        sender_name: None,
        message: ChatText::Component(UiText::LegacyJson { json: json.clone() }),
    };
    Ok((
        message,
        ChatMessage {
            json,
            position: position as i8,
            sender,
        },
    ))
}
