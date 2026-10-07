//! Received chat history and chat/command dispatch common to both versions.
//!
//! Received messages keep their native encoding (`UiText`); rendering, language
//! handling and command-result interpretation belong to the caller. Dispatch is
//! not delivery: the server may still reject, filter or rewrite a message.
use super::SessionStamp;
use super::ui::UiText;
use crate::Result;
use crate::versions::java_1_21_11::ScoreboardReader as Reader;
use std::collections::VecDeque;

/// Maximum retained messages. Older entries are dropped and reported as truncation.
pub const MAX_RETAINED_CHAT: usize = 256;
const MAX_RETAINED_BYTES: usize = 4 << 20;
/// Longest chat line accepted for dispatch, in characters (native limit).
pub const MAX_CHAT_LENGTH: usize = 256;

/// Where the server placed a received message.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum ChatKind {
    /// Player chat. Java 1.21.11 `PLAYER_CHAT`; Java 1.16.1 chat position 0.
    Player,
    /// Chat attributed to a name but not to a player profile (Java 1.21.11 only).
    Profileless,
    /// Server or command feedback in the chat area.
    System,
    /// Text above the hotbar.
    ActionBar,
}

/// Message text as received.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub enum ChatText {
    /// A native text component (Java 1.16.1 JSON or Java 1.21.11 NBT).
    Component(UiText),
    /// The plain body of a Java 1.21.11 player message without server decoration.
    Plain(String),
    /// The packet framing was valid for its kind but the content could not be decoded.
    Undecoded,
}

/// One received chat packet.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ReceivedChat {
    /// Connection-local receive ordinal of the packet.
    pub receive_sequence: u64,
    /// Placement chosen by the server.
    pub kind: ChatKind,
    /// Sender profile, when the packet names a non-nil one.
    pub sender: Option<[u8; 16]>,
    /// Decorated sender name from the chat type (Java 1.21.11).
    pub sender_name: Option<UiText>,
    /// Displayed content; server-decorated content is preferred when present.
    pub message: ChatText,
}

/// Messages received after a caller cursor.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ChatLog {
    /// Current connection/world provenance.
    pub session: SessionStamp,
    /// Receive boundary of this read; pass it as the next cursor.
    pub receive_sequence: u64,
    /// Messages strictly after the requested cursor, oldest first.
    pub messages: Vec<ReceivedChat>,
}

/// Bounded per-connection chat history fed by each adapter's receive loop.
#[derive(Default)]
pub(crate) struct ChatLedger {
    messages: VecDeque<ReceivedChat>,
    bytes: usize,
    dropped_through: u64,
}

impl ChatLedger {
    /// Record a Java 1.16.1 `CHAT` packet.
    pub(crate) fn receive_legacy(&mut self, payload: &[u8], sequence: u64) -> Result<()> {
        let mut r = Reader::new(payload);
        let json = r.string()?;
        let position = r.u8()?;
        let sender: [u8; 16] = r.take(16)?.try_into().expect("fixed UUID field");
        r.end()?;
        let kind = match position {
            0 => ChatKind::Player,
            1 => ChatKind::System,
            2 => ChatKind::ActionBar,
            _ => return Err(super::recording::invalid("unknown chat position")),
        };
        self.push(ReceivedChat {
            receive_sequence: sequence,
            kind,
            sender: (sender != [0; 16]).then_some(sender),
            sender_name: None,
            message: ChatText::Component(UiText::LegacyJson { json }),
        });
        Ok(())
    }

    /// Record a Java 1.21.11 `SYSTEM_CHAT`, `PLAYER_CHAT` or `PROFILELESS_CHAT` packet.
    pub(crate) fn receive_modern(&mut self, id: i32, payload: &[u8], sequence: u64) -> Result<()> {
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
            _ => return Err(super::recording::invalid("not a chat packet")),
        };
        self.push(message);
        Ok(())
    }

    fn push(&mut self, message: ReceivedChat) {
        self.bytes += retained_size(&message);
        self.messages.push_back(message);
        while self.messages.len() > MAX_RETAINED_CHAT || self.bytes > MAX_RETAINED_BYTES {
            let Some(oldest) = self.messages.pop_front() else {
                break;
            };
            self.bytes -= retained_size(&oldest);
            self.dropped_through = oldest.receive_sequence;
        }
    }

    /// Messages after `cursor`. A cursor older than the retained window is an
    /// error so that a gap is never presented as complete history.
    pub(crate) fn after(
        &self,
        cursor: u64,
        session: SessionStamp,
        receive_sequence: u64,
    ) -> Result<ChatLog> {
        if cursor < self.dropped_through {
            return Err(super::recording::invalid(
                "chat history before the cursor was truncated",
            ));
        }
        if cursor > receive_sequence {
            return Err(super::recording::invalid(
                "chat cursor is ahead of the receive state",
            ));
        }
        Ok(ChatLog {
            session,
            receive_sequence,
            messages: self
                .messages
                .iter()
                .filter(|m| m.receive_sequence > cursor)
                .cloned()
                .collect(),
        })
    }
}

fn retained_size(message: &ReceivedChat) -> usize {
    fn text(t: &UiText) -> usize {
        match t {
            UiText::LegacyJson { json } => json.len(),
            UiText::NativeNbt { bytes } => bytes.len(),
            UiText::Unavailable => 0,
        }
    }
    64 + message.sender_name.as_ref().map_or(0, text)
        + match &message.message {
            ChatText::Component(t) => text(t),
            ChatText::Plain(s) => s.len(),
            ChatText::Undecoded => 0,
        }
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

/// Validate a chat line before any adapter-specific encoding.
pub(crate) fn validate_chat(message: &str) -> Result<()> {
    if message.is_empty()
        || message.chars().count() > MAX_CHAT_LENGTH
        || message.starts_with('/')
        || message.chars().any(|c| c.is_control() || c == '\u{a7}')
    {
        return Err(super::recording::invalid(
            "chat must be 1..=256 characters without control characters, section signs or a leading slash",
        ));
    }
    Ok(())
}

/// Validate a command and return it without a leading slash.
pub(crate) fn validate_command(command: &str) -> Result<&str> {
    let command = command.strip_prefix('/').unwrap_or(command);
    if command.is_empty()
        || command.chars().count() > MAX_CHAT_LENGTH - 1
        || command.chars().any(|c| c.is_control() || c == '\u{a7}')
    {
        return Err(super::recording::invalid(
            "command must be 1..=255 characters without control characters or section signs",
        ));
    }
    Ok(command)
}

impl super::Client {
    /// Send one chat line. Dispatch is not delivery or acceptance.
    pub async fn send_chat(&self, message: &str) -> Result<super::DispatchReceipt> {
        validate_chat(message)?;
        super::dispatch!(&self.adapter, a => super::adapter::ChatOps::send_chat(a, message).await)?;
        Ok(self.unsequenced_receipt())
    }
    /// Send one command, with or without a leading slash. Server permissions
    /// apply; dispatch does not imply that the command ran.
    pub async fn send_command(&self, command: &str) -> Result<super::DispatchReceipt> {
        let command = validate_command(command)?;
        super::dispatch!(&self.adapter, a => super::adapter::ChatOps::send_command(a, command).await)?;
        Ok(self.unsequenced_receipt())
    }
    /// Received chat strictly after `cursor` (use 0 for everything retained, then
    /// the returned `receive_sequence`). Fails if older messages were dropped.
    pub async fn chat_after(&self, cursor: u64) -> Result<ChatLog> {
        super::dispatch!(&self.adapter, a => super::adapter::ChatOps::chat_after(a, cursor).await)
    }
    fn unsequenced_receipt(&self) -> super::DispatchReceipt {
        super::DispatchReceipt {
            version: self.version(),
            connection_id: super::dispatch!(&self.adapter, a => super::adapter::SessionOps::connection_id(a)),
            interaction_sequence: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinecraftVersion;

    fn session() -> SessionStamp {
        SessionStamp {
            version: MinecraftVersion::Java1_16_1,
            connection_id: 1,
            world_generation: 0,
        }
    }

    fn legacy(json: &str, position: u8, sender: [u8; 16]) -> Vec<u8> {
        let mut p = Vec::new();
        crate::protocol::put_string(&mut p, json);
        p.push(position);
        p.extend_from_slice(&sender);
        p
    }

    #[test]
    fn legacy_positions_and_senders_are_classified() {
        let mut ledger = ChatLedger::default();
        ledger
            .receive_legacy(&legacy("{\"text\":\"hi\"}", 0, [7; 16]), 3)
            .unwrap();
        ledger
            .receive_legacy(&legacy("{\"text\":\"bar\"}", 2, [0; 16]), 4)
            .unwrap();
        assert!(ledger.receive_legacy(&legacy("{}", 9, [0; 16]), 5).is_err());
        let log = ledger.after(0, session(), 5).unwrap();
        assert_eq!(log.messages.len(), 2);
        assert_eq!(log.messages[0].kind, ChatKind::Player);
        assert_eq!(log.messages[0].sender, Some([7; 16]));
        assert_eq!(log.messages[1].kind, ChatKind::ActionBar);
        assert_eq!(log.messages[1].sender, None);
        assert_eq!(ledger.after(3, session(), 5).unwrap().messages.len(), 1);
    }

    #[test]
    fn truncated_history_is_never_presented_as_complete() {
        let mut ledger = ChatLedger::default();
        for sequence in 1..=(MAX_RETAINED_CHAT as u64 + 10) {
            ledger
                .receive_legacy(&legacy("{\"text\":\"x\"}", 1, [0; 16]), sequence)
                .unwrap();
        }
        let end = MAX_RETAINED_CHAT as u64 + 10;
        assert!(ledger.after(0, session(), end).is_err());
        assert!(ledger.after(end + 1, session(), end).is_err());
        assert_eq!(
            ledger.after(10, session(), end).unwrap().messages.len(),
            MAX_RETAINED_CHAT
        );
    }

    #[test]
    fn malformed_player_chat_is_recorded_without_failing_the_connection() {
        let mut ledger = ChatLedger::default();
        let id = crate::versions::java_1_21_11::ClientboundIds::PLAYER_CHAT;
        ledger.receive_modern(id, &[0, 1, 2], 9).unwrap();
        let log = ledger.after(0, session(), 9).unwrap();
        assert_eq!(log.messages[0].message, ChatText::Undecoded);
    }

    // Captured from an unmodified Java 1.21.11 server after `send_chat` and
    // `send_command("me ...")` from an offline client.
    const NATIVE_PLAYER_CHAT: &str = "005437405e29403a7d80d1be97767ab17c00001168656c6c6f2066726f6d20766f78726967000001a114e42a350000000000000000000000010a0a000b636c69636b5f6576656e74080006616374696f6e000f737567676573745f636f6d6d616e64080007636f6d6d616e6400102f74656c6c204368617450726f62652000080009696e73657274696f6e00094368617450726f62650800047465787400094368617450726f62650a000b686f7665725f6576656e740800046e616d6500094368617450726f6265080006616374696f6e000b73686f775f656e74697479080002696400106d696e6563726166743a706c617965720b000475756964000000045437405e29403a7d80d1be97767ab17c000000";
    const NATIVE_PROFILELESS_CHAT: &str = "08001177617665732066726f6d20766f78726967020a0a000b636c69636b5f6576656e74080006616374696f6e000f737567676573745f636f6d6d616e64080007636f6d6d616e6400102f74656c6c204368617450726f62652000080009696e73657274696f6e00094368617450726f62650800047465787400094368617450726f62650a000b686f7665725f6576656e740800046e616d6500094368617450726f6265080006616374696f6e000b73686f775f656e74697479080002696400106d696e6563726166743a706c617965720b000475756964000000045437405e29403a7d80d1be97767ab17c000000";

    #[test]
    fn native_modern_chat_packets_decode_completely() {
        use crate::versions::java_1_21_11::ClientboundIds as ids;
        let mut ledger = ChatLedger::default();
        ledger
            .receive_modern(
                ids::PLAYER_CHAT,
                &hex::decode(NATIVE_PLAYER_CHAT).unwrap(),
                1,
            )
            .unwrap();
        ledger
            .receive_modern(
                ids::PROFILELESS_CHAT,
                &hex::decode(NATIVE_PROFILELESS_CHAT).unwrap(),
                2,
            )
            .unwrap();
        let log = ledger.after(0, session(), 2).unwrap();
        let player = &log.messages[0];
        assert_eq!(player.kind, ChatKind::Player);
        assert_eq!(player.message, ChatText::Plain("hello from voxrig".into()));
        assert!(player.sender.is_some() && player.sender_name.is_some());
        let emote = &log.messages[1];
        assert_eq!(emote.kind, ChatKind::Profileless);
        assert!(matches!(
            &emote.message,
            ChatText::Component(UiText::NativeNbt { .. })
        ));
        assert!(emote.sender_name.is_some());
    }

    #[test]
    fn dispatch_text_is_validated_before_io() {
        assert!(validate_chat("hello").is_ok());
        assert!(validate_chat("").is_err());
        assert!(validate_chat("/op me").is_err());
        assert!(validate_chat("a\u{a7}b").is_err());
        assert!(validate_chat(&"x".repeat(257)).is_err());
        assert_eq!(validate_command("/time set day").unwrap(), "time set day");
        assert!(validate_command("/").is_err());
        assert!(validate_command("say\nhi").is_err());
    }
}
