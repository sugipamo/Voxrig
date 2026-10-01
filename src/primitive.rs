//! Typed low-level primitive operations for the Zen adapter.
//!
//! These types deliberately contain only one packet-level operation. They do
//! not select targets, inventory sources, recipes, routes, or semantic
//! completion. Every dispatch must be accompanied by an [`OperationContext`]
//! at the `Bot` API boundary.

use crate::{
    BlockFace, BlockPos, ClickMode, Hand, ItemStack, OperationContext,
    connection::OperationAdmissionError,
};
use byteorder::{BigEndian, WriteBytesExt};
use std::fmt::{Display, Formatter};

/// Opaque correlation for read-only cross-crate primitive diagnostics.
///
/// It carries no admission or completion authority. The service allocates it
/// only while tracing and the connection actor merely reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimitiveDiagnosticCorrelationId(u64);

impl PrimitiveDiagnosticCorrelationId {
    /// Creates a non-zero diagnostic correlation.
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// Returns the process-local numeric representation for diagnostics.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Result of a client-side primitive dispatch.
///
/// The stages are intentionally not interchangeable. `Dispatched` is a
/// writer fact and `Acknowledged` is a protocol fact. `DeliveryUnknown` means
/// that server-side application could not be proved after the delivery
/// boundary. Fresh observation and semantic completion are Body facts and are
/// intentionally not variants of this client result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PrimitiveDispatchOutcome {
    /// The actor admitted and wrote the packet to the transport.
    Dispatched,
    /// A matching protocol acknowledgement was confirmed.
    Acknowledged,
    /// The peer explicitly rejected the protocol operation.
    Rejected,
    /// Delivery crossed an uncertain boundary and cannot be classified.
    DeliveryUnknown,
}

/// Exact inventory stack precondition selected by the Body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlotExpectation {
    /// Canonical item name.
    pub item_name: String,
    /// Exact stack count before dispatch.
    pub count: i32,
    /// Exact legacy metadata selected from the coherent window observation.
    /// Protocol 736 flattened stacks encode this as zero.
    pub metadata: i32,
}

/// Body-selected main-hand operation without protocol transaction identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EquipOperation {
    /// Select a hotbar slot whose coherent cache precondition is empty.
    SelectEmptyHotbar { hotbar_slot: u8 },
    /// Select an already populated hotbar slot.
    SelectHotbar {
        /// Zero-based hotbar slot selected by the Body.
        hotbar_slot: u8,
        /// Exact stack required in that slot before dispatch.
        expected_item: SlotExpectation,
    },
    /// Swap one inventory slot into the currently selected hotbar slot.
    SwapIntoSelectedHotbar {
        /// Exact player inventory source slot.
        inventory_slot: u16,
        /// Current hotbar slot selected as the swap destination.
        selected_hotbar_slot: u8,
        /// Exact source stack required before dispatch.
        expected_source: SlotExpectation,
        /// Exact destination stack, or `None` when it must be empty.
        expected_destination: Option<SlotExpectation>,
        /// Whether dispatch requires an empty inventory cursor.
        cursor_must_be_empty: bool,
    },
}

/// One exact Body-selected window click in an ordered Craft execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CraftClick {
    /// Exact protocol window slot.
    pub slot: i16,
    /// Exact protocol mouse button.
    pub button: i8,
    /// Exact protocol click mode.
    pub mode: ClickMode,
    /// Exact clicked stack, or an empty-slot requirement.
    pub expected_item: Option<SlotExpectation>,
    /// Exact cursor stack, or an empty-cursor requirement.
    pub expected_cursor: Option<SlotExpectation>,
    /// Deterministic accepted cache effect authored by Body. The client
    /// applies it only after accepted confirmation and only when each current
    /// domain still equals its declared prestate.
    pub accepted_cache_effects: CraftAcceptedCacheEffects,
}

/// Exact per-domain cache effects for an accepted Craft click.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CraftAcceptedCacheEffects {
    /// Slot effects, bounded by the Body contract.
    pub slots: Vec<CraftAcceptedSlotEffect>,
    /// Exact cursor prestate.
    pub cursor_before: Option<SlotExpectation>,
    /// Exact cursor poststate.
    pub cursor_after: Option<SlotExpectation>,
}

/// One exact slot prestate/poststate pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CraftAcceptedSlotEffect {
    /// Window slot identity.
    pub slot: i32,
    /// Required prestate.
    pub before: Option<SlotExpectation>,
    /// Accepted poststate.
    pub after: Option<SlotExpectation>,
}

/// Body-selected ordered Craft click execution for one observed window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CraftExecution {
    /// Exact observed window identity.
    pub window_id: i8,
    /// Close a non-player crafting window through the cleanup barrier.
    pub close_window_after: bool,
    /// Ordered preconditioned clicks.
    pub clicks: Vec<CraftClick>,
}

/// Body-selected phase for one exact furnace window operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FurnaceExecutionPhase {
    /// Move the exact selected input stack into furnace slot zero.
    PutInput,
    /// Move the exact selected fuel stack into furnace slot one.
    PutFuel,
    /// Take the exact furnace output from slot two.
    TakeOutput,
}

/// Body-selected ordered furnace clicks for one observed furnace window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FurnaceExecution {
    /// Exact observed furnace window identity.
    pub window_id: i8,
    /// Exact Body-selected phase.
    pub phase: FurnaceExecutionPhase,
    /// Ordered preconditioned clicks.
    pub clicks: Vec<CraftClick>,
}

/// Result of a finite cleanup dispatch.
///
/// `AppliedLocally` is used only for clearing the local persistent control
/// input; it is not a packet-write or server acknowledgement fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CleanupDispatchOutcome {
    /// The cleanup was committed to the local control state.
    AppliedLocally,
    /// The cleanup packet was written to the transport.
    Dispatched,
    /// A matching protocol acknowledgement was confirmed.
    Acknowledged,
    /// The peer explicitly rejected the cleanup protocol operation.
    Rejected,
    /// Delivery crossed an uncertain boundary and cannot be classified.
    DeliveryUnknown,
}

/// A typed failure that occurs before a primitive packet is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PrimitiveDispatchError {
    /// The connection actor rejected the context or lifecycle class.
    Admission(OperationAdmissionError),
    /// The operation contains a value that cannot be encoded safely.
    InvalidInput,
}

/// Body-selected packet-local sneak requirement for an interaction.
///
/// This is not the continuing movement-control state.  The Zen adapter maps
/// its interaction requirement into this type, and the client preserves it
/// through the actor-owned packet batch without selecting a value itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InteractionSneakRequirement(bool);

impl InteractionSneakRequirement {
    /// Requires the interaction packet to observe a sneaking player.
    #[must_use]
    pub const fn required() -> Self {
        Self(true)
    }

    /// Requires the interaction packet to observe a non-sneaking player.
    #[must_use]
    pub const fn not_required() -> Self {
        Self(false)
    }

    /// Returns the exact packet-local requirement.
    #[must_use]
    pub const fn is_required(self) -> bool {
        self.0
    }
}

impl Display for PrimitiveDispatchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admission(error) => write!(formatter, "primitive admission failed: {error}"),
            Self::InvalidInput => formatter.write_str("primitive input is invalid"),
        }
    }
}

impl std::error::Error for PrimitiveDispatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission(error) => Some(error),
            Self::InvalidInput => None,
        }
    }
}

/// A normal, Body-selected low-level primitive.
///
/// Cleanup operations such as dig-cancel and window-close are represented by
/// [`CleanupPrimitive`] and cannot be passed to this type.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PrimitiveOperation {
    /// Select one exact hotbar slot after its Body precondition was validated.
    SelectHotbar {
        /// Zero-based hotbar slot selected by the Body.
        hotbar_slot: u8,
    },
    /// Send a complete player position-and-look packet using the supplied pose.
    Look {
        /// Player X coordinate.
        x: f64,
        /// Player Y coordinate.
        y: f64,
        /// Player Z coordinate.
        z: f64,
        /// Player yaw.
        yaw: f32,
        /// Player pitch.
        pitch: f32,
        /// Serverbound on-ground bit.
        on_ground: bool,
    },
    /// Send one rotation-only player packet using the supplied Body pose.
    LookRotation {
        /// Player yaw selected by the Body.
        yaw: f32,
        /// Player pitch selected by the Body.
        pitch: f32,
        /// Serverbound on-ground bit selected by the Body.
        on_ground: bool,
    },
    /// Start one block-digging primitive.
    DigStart {
        /// Block selected by the Body.
        position: BlockPos,
        /// Face selected by the Body.
        face: BlockFace,
    },
    /// Finish one block-digging primitive.
    DigFinish {
        /// Block selected by the Body.
        position: BlockPos,
        /// Face selected by the Body.
        face: BlockFace,
    },
    /// Send one exact block interaction using the already selected hand and target.
    ///
    /// This is also the packet-level operation used to open a furnace. The
    /// client never chooses the target, face, cursor, hand, or interaction
    /// phase; those values are supplied by the Body.
    BlockInteraction {
        /// Hand selected by the Body.
        hand: Hand,
        /// Clicked block selected by the Body.
        position: BlockPos,
        /// Face selected by the Body.
        face: BlockFace,
        /// Cursor coordinates selected by the Body.
        cursor: [f32; 3],
        /// Vanilla inside-block flag.
        inside_block: bool,
        /// Packet-local sneak posture selected by the Body.
        sneak: InteractionSneakRequirement,
    },
    /// Send one exact placement interaction without furnace correlation.
    PlacementInteraction {
        /// Hand selected by the Body.
        hand: Hand,
        /// Supporting block selected by the Body.
        position: BlockPos,
        /// Face selected by the Body.
        face: BlockFace,
        /// Cursor coordinates selected by the Body.
        cursor: [f32; 3],
        /// Vanilla inside-block flag.
        inside_block: bool,
        /// Packet-local sneak posture selected by the Body.
        sneak: InteractionSneakRequirement,
    },
    /// Use one item once with the selected hand.
    UseItem {
        /// Hand selected by the Body.
        hand: Hand,
    },
    /// Interact with one exact entity using the Body-selected hand and posture.
    EntityInteraction {
        /// Entity selected by the Body.
        entity_id: i32,
        /// Hand selected by the Body.
        hand: Hand,
        /// Sneak bit encoded in the interaction packet.
        sneak: InteractionSneakRequirement,
    },
    /// Swing one arm once.
    SwingArm {
        /// Hand selected by the Body.
        hand: Hand,
    },
}

/// A low-level primitive whose completion has a matching protocol response.
///
/// Transaction identity is deliberately absent from this type. The client
/// connection actor allocates and tracks it locally.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AcknowledgedPrimitive {
    /// Click one exact window slot using the selected mode.
    WindowClick {
        /// Server window selected by the Body.
        window_id: i8,
        /// Slot selected by the Body.
        slot: i16,
        /// Mouse button selected by the Body.
        button: i8,
        /// Click mode selected by the Body.
        mode: ClickMode,
        /// Slot value captured before the click.
        clicked: Option<ItemStack>,
    },
    /// Finish one dig and await the matching digging acknowledgement.
    DigFinish {
        /// Block selected by the Body.
        position: BlockPos,
        /// Face selected by the Body.
        face: BlockFace,
    },
}

/// A finite cleanup primitive admitted while the connection is disconnecting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CleanupPrimitive {
    /// Clear persistent movement input locally.
    ControlClear,
    /// Cancel a block-digging primitive.
    DigCancel {
        /// Block selected by the Body.
        position: BlockPos,
        /// Face selected by the Body.
        face: BlockFace,
    },
    /// Stop a held use-item primitive.
    UseStop,
    /// Close the selected server-side window.
    CloseWindow {
        /// Server window selected by the Body.
        window_id: i8,
    },
}

impl PrimitiveOperation {
    pub(crate) fn encode(self) -> crate::Result<(i32, Vec<u8>)> {
        let mut payload = Vec::new();
        let packet_id = match self {
            Self::SelectHotbar { hotbar_slot } => {
                if hotbar_slot > 8 {
                    return Err(anyhow::anyhow!("hotbar slot must be 0..=8").into());
                }
                payload.write_i16::<BigEndian>(i16::from(hotbar_slot))?;
                0x24
            }
            Self::Look {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            } => {
                if ![x, y, z].into_iter().all(f64::is_finite)
                    || ![yaw, pitch].into_iter().all(f32::is_finite)
                {
                    return Err(anyhow::anyhow!("primitive look pose must be finite").into());
                }
                payload.write_f64::<BigEndian>(x)?;
                payload.write_f64::<BigEndian>(y)?;
                payload.write_f64::<BigEndian>(z)?;
                payload.write_f32::<BigEndian>(yaw)?;
                payload.write_f32::<BigEndian>(pitch.clamp(-90.0, 90.0))?;
                payload.push(u8::from(on_ground));
                0x13
            }
            Self::LookRotation {
                yaw,
                pitch,
                on_ground,
            } => {
                if ![yaw, pitch].into_iter().all(f32::is_finite) {
                    return Err(anyhow::anyhow!("primitive look rotation must be finite").into());
                }
                payload.write_f32::<BigEndian>(yaw)?;
                payload.write_f32::<BigEndian>(pitch)?;
                payload.push(u8::from(on_ground));
                0x14
            }
            Self::DigStart { position, face } => {
                crate::protocol::put_varint(&mut payload, crate::DiggingStatus::Started as i32);
                payload.write_u64::<BigEndian>(position.packed())?;
                payload.write_i8(face as i8)?;
                0x1b
            }
            Self::DigFinish { position, face } => {
                crate::protocol::put_varint(&mut payload, crate::DiggingStatus::Finished as i32);
                payload.write_u64::<BigEndian>(position.packed())?;
                payload.write_i8(face as i8)?;
                0x1b
            }
            Self::BlockInteraction {
                hand,
                position,
                face,
                cursor,
                inside_block,
                ..
            }
            | Self::PlacementInteraction {
                hand,
                position,
                face,
                cursor,
                inside_block,
                ..
            } => {
                if cursor
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                {
                    return Err(anyhow::anyhow!(
                        "primitive block cursor must be finite and within 0.0..=1.0"
                    )
                    .into());
                }
                crate::protocol::put_varint(&mut payload, hand as i32);
                payload.write_u64::<BigEndian>(position.packed())?;
                crate::protocol::put_varint(&mut payload, face as i32);
                for value in cursor {
                    payload.write_f32::<BigEndian>(value)?;
                }
                payload.push(u8::from(inside_block));
                0x2d
            }
            Self::UseItem { hand } => {
                crate::protocol::put_varint(&mut payload, hand as i32);
                0x2e
            }
            Self::EntityInteraction {
                entity_id,
                hand,
                sneak,
            } => {
                if entity_id < 0 {
                    return Err(anyhow::anyhow!("primitive entity id must be non-negative").into());
                }
                crate::protocol::put_varint(&mut payload, entity_id);
                crate::protocol::put_varint(&mut payload, 0);
                crate::protocol::put_varint(&mut payload, hand as i32);
                payload.push(u8::from(sneak.is_required()));
                0x0e
            }
            Self::SwingArm { hand } => {
                crate::protocol::put_varint(&mut payload, hand as i32);
                0x2b
            }
        };
        Ok((packet_id, payload))
    }
}

impl AcknowledgedPrimitive {
    pub(crate) fn encode_with_action(self, action: i16) -> crate::Result<(i32, Vec<u8>)> {
        let mut payload = Vec::new();
        let packet_id = match self {
            Self::WindowClick {
                window_id,
                slot,
                button,
                mode,
                clicked,
            } => {
                payload.push(window_id as u8);
                payload.write_i16::<BigEndian>(slot)?;
                payload.write_i8(button)?;
                payload.write_i16::<BigEndian>(action)?;
                payload.write_i8(mode as i8)?;
                // Protocol 736 requires a null clicked-item field for the
                // number-key (mode 2) and drop (mode 4) click forms. The
                // source stack is still the Body/client precondition, but it
                // must not be serialized as the packet's clicked item. This
                // matches Mineflayer's protocol adapter and vanilla's
                // server-side mode handling.
                let packet_clicked = matches!(mode, ClickMode::Hotbar | ClickMode::Drop)
                    .then_some(None)
                    .unwrap_or(clicked.as_ref());
                crate::inventory::write_slot(&mut payload, packet_clicked);
                0x09
            }
            Self::DigFinish { position, face } => {
                crate::protocol::put_varint(&mut payload, crate::DiggingStatus::Finished as i32);
                payload.write_u64::<BigEndian>(position.packed())?;
                payload.write_i8(face as i8)?;
                0x1b
            }
        };
        Ok((packet_id, payload))
    }
}

impl CleanupPrimitive {
    pub(crate) fn encode(self) -> crate::Result<Option<(i32, Vec<u8>)>> {
        let mut payload = Vec::new();
        let packet = match self {
            Self::ControlClear => return Ok(None),
            Self::DigCancel { position, face } => {
                crate::protocol::put_varint(&mut payload, crate::DiggingStatus::Cancelled as i32);
                payload.write_u64::<BigEndian>(position.packed())?;
                payload.write_i8(face as i8)?;
                (0x1b, payload)
            }
            Self::UseStop => {
                crate::protocol::put_varint(
                    &mut payload,
                    crate::DiggingStatus::ReleaseUseItem as i32,
                );
                payload.write_u64::<BigEndian>(0)?;
                payload.write_i8(BlockFace::Down as i8)?;
                (0x1b, payload)
            }
            Self::CloseWindow { window_id } => (0x0a, vec![window_id as u8]),
        };
        Ok(Some(packet))
    }
}

/// Correlation required by a new primitive dispatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveRequest {
    /// Operation context binding this request to one observation and connection.
    pub context: OperationContext,
    /// Normal low-level operation.
    pub operation: PrimitiveOperation,
}

/// Correlation required by a cleanup dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupRequest {
    /// Operation context binding this request to one observation and connection.
    pub context: OperationContext,
    /// Finite cleanup operation.
    pub operation: CleanupPrimitive,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_primitive_encoding_is_packet_specific() {
        let (packet_id, payload) = PrimitiveOperation::UseItem { hand: Hand::Main }
            .encode()
            .unwrap();
        assert_eq!(packet_id, 0x2e);
        assert_eq!(payload, vec![0]);
    }

    #[test]
    fn protocol_736_hotbar_swap_serializes_a_null_clicked_item() {
        let clicked = ItemStack {
            item_id: 69,
            count: 1,
            nbt: None,
        };
        let (_, payload) = AcknowledgedPrimitive::WindowClick {
            window_id: 0,
            slot: 9,
            button: 0,
            mode: ClickMode::Hotbar,
            clicked: Some(clicked.clone()),
        }
        .encode_with_action(17)
        .unwrap();
        assert_eq!(&payload[..7], &[0, 0, 9, 0, 0, 17, 2]);
        assert_eq!(payload[7], 0);

        let (_, normal_payload) = AcknowledgedPrimitive::WindowClick {
            window_id: 0,
            slot: 9,
            button: 0,
            mode: ClickMode::Normal,
            clicked: Some(clicked),
        }
        .encode_with_action(18)
        .unwrap();
        assert_eq!(normal_payload.last(), Some(&0));
        assert_eq!(normal_payload[7], 1);
    }

    #[test]
    fn entity_interaction_preserves_the_body_selected_sneak_bit() {
        for (requirement, expected) in [
            (InteractionSneakRequirement::not_required(), 0),
            (InteractionSneakRequirement::required(), 1),
        ] {
            let (packet_id, payload) = PrimitiveOperation::EntityInteraction {
                entity_id: 42,
                hand: Hand::Off,
                sneak: requirement,
            }
            .encode()
            .unwrap();
            assert_eq!(packet_id, 0x0e);
            assert_eq!(payload, vec![42, 0, 1, expected]);
        }
    }

    #[test]
    fn cleanup_encoding_cannot_be_confused_with_normal_use() {
        let (packet_id, payload) = CleanupPrimitive::UseStop.encode().unwrap().unwrap();
        assert_eq!(packet_id, 0x1b);
        assert_eq!(payload, vec![5, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(CleanupPrimitive::ControlClear.encode().unwrap(), None);
    }

    #[test]
    fn invalid_pose_is_rejected_before_dispatch() {
        assert!(
            PrimitiveOperation::Look {
                x: f64::NAN,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            }
            .encode()
            .is_err()
        );
    }

    #[test]
    fn look_rotation_is_rotation_only_and_preserves_body_pitch() {
        let (packet_id, payload) = PrimitiveOperation::LookRotation {
            yaw: 45.0,
            pitch: 120.0,
            on_ground: true,
        }
        .encode()
        .unwrap();
        assert_eq!(packet_id, 0x14);
        assert_eq!(payload.len(), 9);
        assert_eq!(f32::from_be_bytes(payload[0..4].try_into().unwrap()), 45.0);
        assert_eq!(f32::from_be_bytes(payload[4..8].try_into().unwrap()), 120.0);
        assert_eq!(payload[8], 1);
    }

    #[test]
    fn public_dispatch_errors_support_standard_error_chaining() {
        fn assert_error<E: std::error::Error>() {}
        fn into_anyhow() -> anyhow::Result<()> {
            Err(PrimitiveDispatchError::Admission(
                OperationAdmissionError::StaleGeneration,
            ))?
        }

        assert_error::<PrimitiveDispatchError>();
        assert_error::<OperationAdmissionError>();
        let error = into_anyhow().unwrap_err();
        assert!(error.to_string().contains("stale connection generation"));
        assert_eq!(
            error.root_cause().to_string(),
            "stale connection generation"
        );
    }

    #[test]
    fn diagnostic_correlation_rejects_zero_and_preserves_full_nonzero_range() {
        assert_eq!(PrimitiveDiagnosticCorrelationId::new(0), None);
        assert_eq!(PrimitiveDiagnosticCorrelationId::new(1).unwrap().get(), 1);
        assert_eq!(
            PrimitiveDiagnosticCorrelationId::new(u64::MAX)
                .unwrap()
                .get(),
            u64::MAX
        );
    }
}
#[test]
fn select_hotbar_encodes_exact_body_slot_without_transaction_identity() {
    let (packet_id, payload) = PrimitiveOperation::SelectHotbar { hotbar_slot: 4 }
        .encode()
        .unwrap();
    assert_eq!(packet_id, 0x24);
    assert_eq!(payload, 4_i16.to_be_bytes());
    assert!(
        PrimitiveOperation::SelectHotbar { hotbar_slot: 9 }
            .encode()
            .is_err()
    );
}
