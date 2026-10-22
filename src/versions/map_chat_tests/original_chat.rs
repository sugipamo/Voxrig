//! Frozen common chat parsers from origin/main 9810cbd; differential oracles only.
use crate::Result;
use crate::client::chat::{ChatKind, ChatText, ReceivedChat};
use crate::client::ui::UiText;
use crate::versions::java_1_21_11::ScoreboardReader as Reader;
pub(super) fn legacy(payload: &[u8], sequence: u64) -> Result<ReceivedChat> {
    crate::versions::java_1_16_1::chat::parse_chat(payload)?;
    let mut r = Reader::new(payload);
    let json = r.string()?;
    let position = r.u8()?;
    let sender: [u8; 16] = r.take(16)?.try_into().expect("fixed UUID field");
    r.end()?;
    let kind = match position {
        0 => ChatKind::Player,
        1 => ChatKind::System,
        2 => ChatKind::ActionBar,
        _ => return Err(crate::client::recording::invalid("unknown chat position")),
    };
    Ok(ReceivedChat {
        receive_sequence: sequence,
        kind,
        sender: (sender != [0; 16]).then_some(sender),
        sender_name: None,
        message: ChatText::Component(UiText::LegacyJson { json }),
    })
}
pub(super) fn modern(id: i32, payload: &[u8], sequence: u64) -> Result<ReceivedChat> {
    use crate::versions::java_1_21_11::ClientboundIds as ids;
    let message = match id {
        ids::SYSTEM_CHAT => {
            let mut r = Reader::new(payload);
            let text = UiText::NativeNbt {
                bytes: r.encoded_nbt()?,
            };
            let overlay = r.bool()?;
            r.end()?;
            ReceivedChat {
                receive_sequence: sequence,
                kind: if overlay {
                    ChatKind::ActionBar
                } else {
                    ChatKind::System
                },
                sender: None,
                sender_name: None,
                message: ChatText::Component(text),
            }
        }
        ids::PLAYER_CHAT => player_chat(payload, sequence),
        ids::PROFILELESS_CHAT => profileless_chat(payload, sequence),
        _ => return Err(crate::client::recording::invalid("not a chat packet")),
    };
    Ok(message)
}
/// A chat-type binding: registry holder, decorated name and optional target.
fn bound_chat_type(r: &mut Reader<'_>) -> anyhow::Result<UiText> {
    if r.varint()? == 0 {
        // Inline chat-type definitions have no fixed length we model here.
        anyhow::bail!("inline chat type");
    }
    let name = UiText::NativeNbt {
        bytes: r.encoded_nbt()?,
    };
    if r.bool()? {
        r.skip_nbt()?;
    }
    Ok(name)
}

fn player_chat(payload: &[u8], sequence: u64) -> ReceivedChat {
    let mut sender = None;
    let decoded = (|| -> anyhow::Result<(ChatText, UiText)> {
        let mut r = Reader::new(payload);
        r.varint()?; // Global message index.
        sender = Some(<[u8; 16]>::try_from(r.take(16)?).expect("fixed UUID field"));
        r.varint()?; // Per-sender index.
        if r.bool()? {
            r.take(256)?; // Signature.
        }
        let body = r.string()?;
        r.u64()?; // Timestamp.
        r.u64()?; // Salt.
        for _ in 0..r.count(20)? {
            if r.varint()? == 0 {
                r.take(256)?;
            }
        }
        let unsigned = if r.bool()? {
            Some(UiText::NativeNbt {
                bytes: r.encoded_nbt()?,
            })
        } else {
            None
        };
        match r.varint()? {
            0 | 1 => {}
            2 => {
                for _ in 0..r.count(4096)? {
                    r.u64()?;
                }
            }
            _ => anyhow::bail!("unknown filter mask"),
        }
        let name = bound_chat_type(&mut r)?;
        r.end()?;
        let message = unsigned.map_or(ChatText::Plain(body), ChatText::Component);
        Ok((message, name))
    })();
    let (message, sender_name) = match decoded {
        Ok((message, name)) => (message, Some(name)),
        Err(_) => (ChatText::Undecoded, None),
    };
    ReceivedChat {
        receive_sequence: sequence,
        kind: ChatKind::Player,
        sender: sender.filter(|s| *s != [0; 16]),
        sender_name,
        message,
    }
}

fn profileless_chat(payload: &[u8], sequence: u64) -> ReceivedChat {
    let decoded = (|| -> anyhow::Result<(UiText, UiText)> {
        let mut r = Reader::new(payload);
        let message = UiText::NativeNbt {
            bytes: r.encoded_nbt()?,
        };
        let name = bound_chat_type(&mut r)?;
        r.end()?;
        Ok((message, name))
    })();
    let (message, sender_name) = match decoded {
        Ok((message, name)) => (ChatText::Component(message), Some(name)),
        Err(_) => (ChatText::Undecoded, None),
    };
    ReceivedChat {
        receive_sequence: sequence,
        kind: ChatKind::Profileless,
        sender: None,
        sender_name,
        message,
    }
}
