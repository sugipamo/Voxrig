//! Structured refusals from the exact movement contact model.
use crate::client::{EntityId, SessionStamp};
use std::fmt;

/// Recovery class, independent of the lifetime of the selected owner.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum MovementRefusal {
    /// Missing received input can arrive; an invalidated owner still needs replacement.
    Transient {
        /// Frozen refusal details.
        context: MovementRefusalContext,
    },
    /// This admission or owner cannot continue (for example after history loss).
    Permanent {
        /// Frozen refusal details.
        context: MovementRefusalContext,
    },
    /// The current implementation does not qualify this input or capability.
    Unsupported {
        /// Frozen refusal details.
        context: MovementRefusalContext,
    },
}
/// Frozen diagnostic context. Unknown identities and types remain absent.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MovementRefusalContext {
    /// Version, connection and world of the contact owner.
    pub session: Option<SessionStamp>,
    /// Selected operation attempt, if assigned.
    pub owner_id: Option<u64>,
    /// Disposition of the selected owner.
    pub owner_state: MovementOwnerState,
    /// Exact spawn lifetime, including its original receive sequence.
    pub entity: Option<EntityId>,
    /// Native ID even when no spawn lifetime is known.
    pub native_entity_id: Option<i32>,
    /// Namespaced received type; unknown types stay absent.
    pub entity_type: Option<String>,
    /// Triggering packet, required for packet-triggered unsupported refusals.
    pub packet: Option<MovementPacketTrigger>,
    /// Original packet receive boundary, when known.
    pub receive_sequence: Option<u64>,
    /// Original history record ordinal, when known.
    pub record_ordinal: Option<u64>,
    /// Owner model tick at the refusal.
    pub model_tick: Option<u64>,
    /// Human-readable diagnostic, not a control-flow contract.
    pub reason: String,
}
/// Disposition of the movement owner when the error reaches its caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum MovementOwnerState {
    /// Admission failed before an owner was established.
    NotAdmitted,
    /// This owner must be replaced.
    Invalidated,
    /// The contact state remains retained by its caller.
    Retained,
}
/// Original packet kind and native sub-opcode, not a wire packet ID.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MovementPacketTrigger {
    /// History packet kind (for example Status or Metadata).
    pub packet_type: String,
    /// Status/animation code; zero for packet kinds without a sub-opcode.
    pub opcode: i32,
}
impl MovementRefusal {
    /// Returns details shared by all recovery classes.
    pub fn context(&self) -> &MovementRefusalContext {
        match self {
            Self::Transient { context }
            | Self::Permanent { context }
            | Self::Unsupported { context } => context,
        }
    }
    pub(crate) fn context_mut(&mut self) -> &mut MovementRefusalContext {
        match self {
            Self::Transient { context }
            | Self::Permanent { context }
            | Self::Unsupported { context } => context,
        }
    }
    pub(crate) fn permanent(reason: String) -> Self {
        Self::Permanent {
            context: MovementRefusalContext {
                session: None,
                owner_id: None,
                owner_state: MovementOwnerState::NotAdmitted,
                entity: None,
                native_entity_id: None,
                entity_type: None,
                packet: None,
                receive_sequence: None,
                record_ordinal: None,
                model_tick: None,
                reason,
            },
        }
    }
    pub(crate) fn into_error(self) -> crate::Error {
        let kind = if matches!(self, Self::Unsupported { .. }) {
            crate::ErrorKind::Unsupported
        } else {
            crate::ErrorKind::State
        };
        crate::Error::new(kind, self)
    }
}
impl fmt::Display for MovementRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.context().reason)
    }
}
impl std::error::Error for MovementRefusal {}
