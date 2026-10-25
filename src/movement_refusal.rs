//! Structured refusals and best-effort diagnostics from the movement contact model.
use crate::client::{EntityId, SessionStamp};
use std::fmt;

/// Recovery class, independent of the lifetime of the selected owner.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum MovementRefusal {
    /// Warning from an opted-in ground owner: the entire remote numerical
    /// contact state was dropped. The same owner and movement input continue
    /// using local terrain; a received correction re-anchors the local pose.
    /// The inner refusal describes the strict model, not the running control.
    RemoteStateDropped {
        /// Original strict cause, including its receipt and geometry context.
        refusal: Box<MovementRefusal>,
    },
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
    /// A refusal with additional contact/terrain diagnostics. The inner refusal
    /// preserves its recovery class and the original context layout.
    Scoped {
        /// Original recovery class and receipt context.
        refusal: Box<MovementRefusal>,
        /// Entity placement and terrain cause at this boundary.
        geometry: MovementGeometryContext,
    },
    /// The current implementation does not qualify this input or capability.
    /// Unqualified creature contact, failed deferred envelopes and unlisted
    /// contact status/animation callbacks pass through without this refusal
    /// (Issue #128 option 3). Modeled input/admission failures may still emit it.
    Unsupported {
        /// Frozen refusal details.
        context: MovementRefusalContext,
    },
}
/// Additional contact diagnostics, separate from the constructible original
/// `MovementRefusalContext` so existing struct literals remain source compatible.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub struct MovementGeometryContext {
    /// Last received entity feet, or the declared model feet for a terrain query.
    pub entity_position: Option<[f64; 3]>,
    /// Missing or refused terrain cell, when the failure concerns geometry.
    pub block_position: Option<[i32; 3]>,
    /// Chunk containing `block_position` (Euclidean division, including negatives).
    pub chunk_position: Option<[i32; 2]>,
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
            Self::Scoped { refusal, .. } | Self::RemoteStateDropped { refusal } => {
                refusal.context()
            }
        }
    }
    pub(crate) fn context_mut(&mut self) -> &mut MovementRefusalContext {
        match self {
            Self::Transient { context }
            | Self::Permanent { context }
            | Self::Unsupported { context } => context,
            Self::Scoped { refusal, .. } | Self::RemoteStateDropped { refusal } => {
                refusal.context_mut()
            }
        }
    }
    /// Geometry diagnostics for scoped contacts and missing terrain.
    pub fn geometry(&self) -> Option<&MovementGeometryContext> {
        match self {
            Self::Scoped { geometry, .. } => Some(geometry),
            Self::RemoteStateDropped { refusal } => refusal.geometry(),
            _ => None,
        }
    }
    pub(crate) fn geometry_mut(&mut self) -> &mut MovementGeometryContext {
        if let Self::RemoteStateDropped { refusal } = self {
            return refusal.geometry_mut();
        }
        if !matches!(self, Self::Scoped { .. }) {
            *self = Self::Scoped {
                refusal: Box::new(self.clone()),
                geometry: Default::default(),
            };
        }
        match self {
            Self::Scoped { geometry, .. } => geometry,
            _ => unreachable!(),
        }
    }
    pub(crate) fn unsupported_kind(&self) -> bool {
        match self {
            Self::Unsupported { .. } => true,
            Self::Scoped { refusal, .. } | Self::RemoteStateDropped { refusal } => {
                refusal.unsupported_kind()
            }
            _ => false,
        }
    }
    pub(crate) fn terrain(error: crate::Error, p: [i32; 3]) -> crate::Error {
        let mut value = error.movement_refusal().cloned().unwrap_or_else(|| {
            let value = Self::permanent(error.to_string());
            if error.kind() == crate::ErrorKind::Unsupported {
                Self::Unsupported {
                    context: value.context().clone(),
                }
            } else {
                value
            }
        });
        let geometry = value.geometry_mut();
        geometry.block_position = Some(p);
        geometry.chunk_position = Some([p[0].div_euclid(16), p[2].div_euclid(16)]);
        value.into_error()
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
        let kind = if self.unsupported_kind() {
            crate::ErrorKind::Unsupported
        } else {
            crate::ErrorKind::State
        };
        crate::Error::new(kind, self)
    }
}
impl fmt::Display for MovementRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::RemoteStateDropped { refusal } = self {
            return write!(
                f,
                "remote numerical contact state dropped; continuing with the same control owner and movement input; strict contact cause: {refusal}"
            );
        }
        f.write_str(&self.context().reason)?;
        if let Some(g) = self.geometry() {
            write!(
                f,
                " [entity={:?} type={:?} position={:?} block={:?} chunk={:?}]",
                self.context().native_entity_id,
                self.context().entity_type,
                g.entity_position,
                g.block_position,
                g.chunk_position
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for MovementRefusal {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_context_literal_and_recovery_survive_added_geometry() {
        // Preserve the original public constructible struct without new fields.
        let context = MovementRefusalContext {
            session: None,
            owner_id: None,
            owner_state: MovementOwnerState::NotAdmitted,
            entity: None,
            native_entity_id: Some(7),
            entity_type: Some("minecraft:cow".into()),
            packet: None,
            receive_sequence: None,
            record_ordinal: None,
            model_tick: None,
            reason: "missing geometry".into(),
        };
        let original = MovementRefusal::Unsupported {
            context: context.clone(),
        }
        .into_error();
        let error = MovementRefusal::terrain(original, [-17, 65, -1]);
        assert_eq!(error.kind(), crate::ErrorKind::Unsupported);
        let value = error.movement_refusal().unwrap();
        assert_eq!(value.context(), &context);
        assert_eq!(value.geometry().unwrap().chunk_position, Some([-2, -1]));
        assert!(value.to_string().contains("[-17, 65, -1]"));
    }
}
