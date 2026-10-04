//! Connection-generation lifecycle and operation admission.
//!
//! This actor is the sole mutable owner of whether a connection may accept
//! normal or cleanup operations. Packet/cache ownership moves behind the same
//! actor boundary in the coherent-observation phase.

mod bounded_motion;
use bounded_motion::MotionGate;

use std::{
    collections::{HashMap, HashSet},
    fmt::{Display, Formatter},
    sync::atomic::{AtomicU64, Ordering},
};

use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, RwLock, mpsc, oneshot, watch},
    time::{Duration, Instant, sleep},
};

use std::sync::Arc;

fn emit_connection_diagnostic(
    generation: ConnectionGeneration,
    previous: ConnectionState,
    event: &'static str,
    site: &'static str,
    detail: impl FnOnce() -> String,
) {
    // Terminal causes must survive runs without an optional event subscriber.
    // The owning actor emits at most once before exiting its terminal branch.
    eprintln!(
        "connection_owner_diagnostic {}",
        serde_json::json!({
            "event": event, "generation": generation.get(), "previous": format!("{previous:?}"),
            "site": site, "detail": detail().chars().take(2048).collect::<String>(),
        })
    );
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
const MAX_RETIRED_TRANSACTIONS: usize = 1024;
// Pending acknowledgements are actor-owned until the peer confirms them or
// their deadline retires them.  Keep the admission set bounded even when a
// caller chooses distinct connection-local identities (for example, many
// different dig positions) and drops every returned Future.
const MAX_PENDING_TRANSACTIONS: usize = 1024;
const FURNACE_CORRELATION_TTL: Duration = Duration::from_secs(2);
const FURNACE_WINDOW_TYPE: i32 = 13;

/// Process-local identity of one Minecraft transport connection.
///
/// Values are never intentionally reused. They are correlation identities,
/// not application state revisions or protocol transaction numbers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionGeneration(u64);

impl ConnectionGeneration {
    pub(crate) fn allocate() -> Self {
        let value = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        assert!(value != 0, "client connection generation exhausted");
        Self(value)
    }

    /// Returns the process-local numeric representation for diagnostics.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Generation binding required by every externally requested operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationContext {
    /// Connection generation observed by the caller.
    pub generation: ConnectionGeneration,
    /// Client observation sequence used to choose the operation.
    pub source_observation_sequence: u64,
}

/// Lifecycle fact owned by the connection actor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    /// TCP/login or initial spawn/position synchronization is in progress.
    Connecting,
    /// Initial synchronization completed and normal operations may be admitted.
    Ready,
    /// Shutdown began; normal operations are closed and finite cleanup may run.
    Disconnecting,
    /// Reader/writer termination was observed without unresolved delivery.
    Disconnected,
    /// Transport termination or possible delivery could not be classified safely.
    ConnectionStateUnknown,
}

impl ConnectionState {
    /// Returns whether no later lifecycle transition is permitted.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Disconnected | Self::ConnectionStateUnknown)
    }
}

/// Operation class used by the disconnect barrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationClass {
    /// Ordinary gameplay operation selected by the caller.
    Normal,
    /// Finite cleanup selected by the caller before disconnect completion.
    Cleanup,
}

/// Typed rejection from the connection actor before packet write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationAdmissionError {
    /// A retained cursor-return/close pipeline owns normal dispatch.
    BoundedContainerCloseInProgress,
    /// A retained common storage activation owns ordinary gameplay dispatch.
    BoundedContainerOpenInProgress,
    /// A finite common motion run exclusively owns normal gameplay dispatch.
    BoundedMotionInProgress,
    /// A retained common mining attempt owns normal dispatch until fresh recovery.
    BoundedMiningInProgress,
    /// Retained placement owns gameplay dispatch until received outcomes agree.
    BoundedPlacementInProgress,
    /// A retained common inventory exchange owns ordinary gameplay dispatch.
    BoundedInventorySwapInProgress,
    /// A retained common ordinary click owns gameplay dispatch.
    BoundedInventoryClickInProgress,
    /// Retained one-shot shift transfer owns normal gameplay dispatch.
    BoundedInventoryTransferInProgress,
    /// The operation belongs to a previous or different connection.
    StaleGeneration,
    /// The connection has not reached its initial ready boundary.
    Connecting,
    /// Normal operations are closed because disconnect has started.
    Disconnecting,
    /// The transport is confirmed ended.
    Disconnected,
    /// Delivery and connection state cannot be classified safely.
    ConnectionStateUnknown,
    /// Another transaction for the same connection-local scope is pending.
    TransactionInProgress,
    /// The operation cannot be represented by the protocol transaction API.
    InvalidOperation,
}

impl Display for OperationAdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::BoundedContainerCloseInProgress => {
                "retained cursor return/close owns gameplay dispatch"
            }
            Self::BoundedContainerOpenInProgress => {
                "retained common container open owns gameplay dispatch"
            }
            Self::BoundedMotionInProgress => "finite common motion owns gameplay dispatch",
            Self::BoundedPlacementInProgress => "retained common placement owns gameplay dispatch",
            Self::BoundedInventoryTransferInProgress => {
                "retained common inventory transfer owns gameplay dispatch"
            }
            Self::BoundedInventoryClickInProgress => {
                "retained common inventory click owns gameplay dispatch"
            }
            Self::BoundedInventorySwapInProgress => {
                "retained common inventory swap owns gameplay dispatch"
            }
            Self::BoundedMiningInProgress => "retained common mining owns gameplay dispatch",
            Self::StaleGeneration => "stale connection generation",
            Self::Connecting => "connection is not ready",
            Self::Disconnecting => "connection is disconnecting",
            Self::Disconnected => "connection is disconnected",
            Self::ConnectionStateUnknown => "connection state is unknown",
            Self::TransactionInProgress => "a protocol transaction is already in progress",
            Self::InvalidOperation => "protocol transaction operation is invalid",
        };
        formatter.write_str(name)
    }
}

impl std::error::Error for OperationAdmissionError {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TransactionIdentity {
    WindowClick {
        window_id: i8,
        action: i16,
    },
    Dig {
        position: crate::BlockPos,
        status: i32,
    },
}

struct PendingTransaction {
    completion: oneshot::Sender<crate::DispatchOutcome>,
    confirmation_seen: bool,
    diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
}

#[derive(Clone, Copy)]
enum PendingDigDiagnostic {
    Unique(crate::DiagnosticCorrelationId),
    Retired,
}

#[derive(Clone, Copy)]
struct PendingFurnaceInteraction {
    position: crate::BlockPos,
    expires_at: Instant,
}

/// A connection-local protocol transaction returned by an acknowledged
/// primitive dispatch.
///
/// The protocol transaction number is intentionally not exposed. Awaiting
/// this value reports protocol acknowledgement or rejection, never semantic
/// game completion. Dropping the value does not cancel the actor-owned
/// pending transaction or make an already written packet disappear.
pub struct ProtocolTransaction {
    identity: TransactionIdentity,
    dispatched: bool,
    completion: oneshot::Receiver<crate::DispatchOutcome>,
}

impl ProtocolTransaction {
    /// Waits for acknowledgement, rejection, or the actor-owned delivery
    /// deadline. A dropped actor is treated as delivery unknown.
    pub async fn wait(self) -> crate::DispatchOutcome {
        self.completion
            .await
            .unwrap_or(crate::DispatchOutcome::DeliveryUnknown)
    }

    pub(crate) const fn window_action(&self) -> Option<i16> {
        match self.identity {
            TransactionIdentity::WindowClick { action, .. } => Some(action),
            TransactionIdentity::Dig { .. } => None,
        }
    }

    pub(crate) const fn was_dispatched(&self) -> bool {
        self.dispatched
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalClassification {
    Disconnected,
    ConnectionStateUnknown,
}

enum Command {
    Motion(bounded_motion::MotionCommand),
    BeginCursorClose {
        identity: (u64, i8, u16),
        expected_revision: u64,
        reply: oneshot::Sender<Result<(), OperationAdmissionError>>,
    },
    ReserveCursorReturn {
        identity: (u64, u16),
        reply: oneshot::Sender<Result<i16, OperationAdmissionError>>,
    },
    BeginInventorySwap {
        run_id: u64,
        expected_revision: u64,
        window: i8,
        reply: oneshot::Sender<Result<i16, OperationAdmissionError>>,
    },
    BeginInventoryClick {
        run_id: u64,
        expected_revision: u64,
        window: i8,
        reply: oneshot::Sender<Result<i16, OperationAdmissionError>>,
    },
    BeginInventoryTransfer {
        run_id: u64,
        expected_revision: u64,
        window: i8,
        reply: oneshot::Sender<Result<i16, OperationAdmissionError>>,
    },
    RecordObservation {
        sequence: u64,
        reply: oneshot::Sender<Result<(), OperationAdmissionError>>,
    },
    MarkReady {
        reply: oneshot::Sender<()>,
    },
    BeginDisconnect {
        reply: oneshot::Sender<Result<(), OperationAdmissionError>>,
    },
    Admit {
        context: OperationContext,
        class: OperationClass,
        reply: oneshot::Sender<Result<(), OperationAdmissionError>>,
    },
    ConsumeControl {
        tick: Duration,
        reply: oneshot::Sender<(crate::ControlState, f64)>,
    },
    ReplaceControl {
        context: OperationContext,
        class: OperationClass,
        control: crate::ControlState,
        reply: oneshot::Sender<Result<(), OperationAdmissionError>>,
    },
    Dispatch {
        context: OperationContext,
        class: OperationClass,
        packet_id: i32,
        payload: Vec<u8>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    DispatchPrimitive {
        context: OperationContext,
        class: OperationClass,
        packet_id: i32,
        payload: Vec<u8>,
        dig_diagnostic: Option<DigWriteDiagnostic>,
        reply:
            oneshot::Sender<std::result::Result<crate::DispatchOutcome, OperationAdmissionError>>,
    },
    DispatchInteractionBatch {
        context: OperationContext,
        class: OperationClass,
        furnace_position: Option<crate::BlockPos>,
        packets: Vec<(i32, Vec<u8>)>,
        reply:
            oneshot::Sender<std::result::Result<crate::DispatchOutcome, OperationAdmissionError>>,
    },
    DispatchAcknowledged {
        context: OperationContext,
        class: OperationClass,
        operation: crate::AcknowledgedOperation,
        diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
        reply: oneshot::Sender<std::result::Result<ProtocolTransaction, OperationAdmissionError>>,
    },
    ObserveWindowConfirmation {
        window_id: i8,
        action: i16,
        accepted: bool,
        reply: oneshot::Sender<bool>,
    },
    CommitWindowBarrier {
        window_id: i8,
        action: i16,
        reply: oneshot::Sender<bool>,
    },
    ConfirmDig {
        position: crate::BlockPos,
        status: i32,
        successful: bool,
        reply: oneshot::Sender<DigConfirmation>,
    },
    ObserveFurnaceWindow {
        window_type: i32,
        reply: oneshot::Sender<Option<crate::BlockPos>>,
    },
    ExpireTransaction(TransactionIdentity),
    ExpireDigDiagnostic(TransactionIdentity, crate::DiagnosticCorrelationId),
    DispatchProtocol {
        packet_id: i32,
        payload: Vec<u8>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    ShutdownWriter {
        reply: oneshot::Sender<crate::Result<()>>,
    },
    MarkTerminal(TerminalClassification, Option<(&'static str, String)>),
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DigConfirmation {
    #[allow(dead_code)]
    pub(crate) matched: bool,
    pub(crate) diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
}

#[derive(Clone, Copy)]
pub(crate) struct DigWriteDiagnostic {
    correlation: crate::DiagnosticCorrelationId,
    phase: &'static str,
    status: i32,
    position: crate::BlockPos,
    face: crate::BlockFace,
}

impl DigWriteDiagnostic {
    pub(crate) fn new(
        correlation: crate::DiagnosticCorrelationId,
        phase: &'static str,
        status: i32,
        position: crate::BlockPos,
        face: crate::BlockFace,
    ) -> Self {
        Self {
            correlation,
            phase,
            status,
            position,
            face,
        }
    }
}

pub(crate) fn emit_dig_lifecycle(make_value: impl FnOnce() -> serde_json::Value) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static EMITTED: AtomicU32 = AtomicU32::new(0);
    if std::env::var_os("VOXRIG_TRACE_OPERATIONS").is_none() {
        return;
    }
    // Atomic::try_update is unavailable on our Rust 1.85 MSRV.
    #[allow(deprecated)]
    let emitted = EMITTED.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        (value < 4096).then_some(value + 1)
    });
    if emitted.is_ok() {
        let value = make_value();
        eprintln!("voxrig_dig_lifecycle {value}");
    } else if EMITTED
        .compare_exchange(4096, 4097, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        eprintln!(
            "voxrig_dig_lifecycle {}",
            serde_json::json!({"stage":"capacity_exhausted","capacity":4096})
        );
    }
}

pub(crate) fn dig_lifecycle_trace_enabled() -> bool {
    std::env::var_os("VOXRIG_TRACE_OPERATIONS").is_some()
}

#[derive(Clone)]
pub(crate) struct ConnectionActor {
    generation: ConnectionGeneration,
    control: Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
    commands: mpsc::Sender<Command>,
    lifecycle: watch::Receiver<ConnectionState>,
}

impl ConnectionActor {
    pub(crate) fn spawn(
        writer: Arc<Mutex<crate::versions::java_1_16_1::client::PacketWriter>>,
        acknowledgement_timeout: Duration,
        control: Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
    ) -> Self {
        let generation = ConnectionGeneration::allocate();
        let (commands, mut receiver) = mpsc::channel(64);
        let expiry_commands = commands.clone();
        let (lifecycle_tx, lifecycle) = watch::channel(ConnectionState::Connecting);
        let actor_control = control.clone();
        tokio::spawn(async move {
            let mut state = ConnectionState::Connecting;
            let mut next_actions = HashMap::<i8, i16>::new();
            let mut pending_transactions =
                HashMap::<TransactionIdentity, PendingTransaction>::new();
            let mut pending_dig_diagnostics =
                HashMap::<TransactionIdentity, PendingDigDiagnostic>::new();
            let mut retired_transactions = HashSet::<TransactionIdentity>::new();
            let mut pending_furnace_interaction: Option<PendingFurnaceInteraction> = None;
            let mut furnace_interaction_ambiguous = false;
            let mut latest_observation_sequence = None;
            let mut active_output_sequence = None;
            let mut motion_gate = MotionGate::default();
            while let Some(command) = receiver.recv().await {
                match command {
                    Command::BeginCursorClose {
                        identity,
                        expected_revision,
                        reply,
                    } => {
                        let result = motion_gate
                            .begin_cursor_close(
                                identity,
                                expected_revision,
                                state,
                                &actor_control,
                                !pending_transactions.is_empty()
                                    || pending_furnace_interaction
                                        .is_some_and(|p| p.expires_at > Instant::now()),
                            )
                            .await;
                        let _ = reply.send(result);
                    }
                    Command::ReserveCursorReturn { identity, reply } => {
                        let _ = reply.send(motion_gate.reserve_cursor_return(
                            identity,
                            state,
                            &mut next_actions,
                        ));
                    }
                    Command::BeginInventorySwap {
                        run_id,
                        expected_revision,
                        window,
                        reply,
                    } => {
                        let result = motion_gate
                            .begin_inventory_swap(
                                (run_id, window),
                                expected_revision,
                                state,
                                &actor_control,
                                !pending_transactions.is_empty()
                                    || pending_furnace_interaction
                                        .is_some_and(|p| p.expires_at > Instant::now()),
                                &mut next_actions,
                            )
                            .await;
                        let _ = reply.send(result);
                    }
                    Command::BeginInventoryClick {
                        run_id,
                        expected_revision,
                        window,
                        reply,
                    } => {
                        let result = motion_gate
                            .begin_inventory_click(
                                (run_id, window),
                                expected_revision,
                                state,
                                &actor_control,
                                !pending_transactions.is_empty()
                                    || pending_furnace_interaction
                                        .is_some_and(|p| p.expires_at > Instant::now()),
                                &mut next_actions,
                            )
                            .await;
                        let _ = reply.send(result);
                    }
                    Command::BeginInventoryTransfer {
                        run_id,
                        expected_revision,
                        window,
                        reply,
                    } => {
                        let result = motion_gate
                            .begin_inventory_transfer(
                                (run_id, window),
                                expected_revision,
                                state,
                                &actor_control,
                                !pending_transactions.is_empty()
                                    || pending_furnace_interaction
                                        .is_some_and(|p| p.expires_at > Instant::now()),
                                &mut next_actions,
                            )
                            .await;
                        let _ = reply.send(result);
                    }
                    Command::Motion(command) => {
                        if motion_gate
                            .process(
                                command,
                                state,
                                &writer,
                                &actor_control,
                                !pending_transactions.is_empty()
                                    || pending_furnace_interaction
                                        .is_some_and(|p| p.expires_at > Instant::now()),
                            )
                            .await
                        {
                            emit_connection_diagnostic(
                                generation,
                                state,
                                "unknown_transition",
                                "bounded_gameplay_write_failed",
                                || "bounded gameplay write failed".to_string(),
                            );
                            state = ConnectionState::ConnectionStateUnknown;
                            lifecycle_tx.send_replace(state);
                            break;
                        }
                    }
                    Command::RecordObservation { sequence, reply } => {
                        let result = if state != ConnectionState::Ready || sequence == 0 {
                            Err(admit_lifecycle(state, OperationClass::Normal)
                                .err()
                                .unwrap_or(OperationAdmissionError::InvalidOperation))
                        } else if latest_observation_sequence
                            .is_some_and(|latest| sequence <= latest)
                        {
                            Err(OperationAdmissionError::InvalidOperation)
                        } else {
                            latest_observation_sequence = Some(sequence);
                            Ok(())
                        };
                        let _ = reply.send(result);
                    }
                    Command::MarkReady { reply } => {
                        if state == ConnectionState::Connecting {
                            state = ConnectionState::Ready;
                            lifecycle_tx.send_replace(state);
                        }
                        let _ = reply.send(());
                    }
                    Command::BeginDisconnect { reply } => {
                        let result = match state {
                            ConnectionState::Connecting | ConnectionState::Ready => {
                                state = ConnectionState::Disconnecting;
                                lifecycle_tx.send_replace(state);
                                Ok(())
                            }
                            ConnectionState::Disconnecting => {
                                Err(OperationAdmissionError::Disconnecting)
                            }
                            ConnectionState::Disconnected => {
                                Err(OperationAdmissionError::Disconnected)
                            }
                            ConnectionState::ConnectionStateUnknown => {
                                Err(OperationAdmissionError::ConnectionStateUnknown)
                            }
                        };
                        let _ = reply.send(result);
                    }
                    Command::Admit {
                        context,
                        class,
                        reply,
                    } => {
                        let result = motion_gate.normal_admission(class).and_then(|()| {
                            activate_output(
                                generation,
                                state,
                                latest_observation_sequence,
                                &mut active_output_sequence,
                                context,
                                class,
                            )
                        });
                        let _ = reply.send(result);
                    }
                    Command::ConsumeControl { tick, reply } => {
                        let mut held = actor_control.write().await;
                        let mut sample = **held;
                        let fraction = if state != ConnectionState::Ready {
                            sample = crate::ControlState::default();
                            **held = sample;
                            0.0
                        } else if let Some(remaining) = sample.movement_press_duration {
                            let consumed = remaining.min(tick);
                            held.movement_press_duration = Some(remaining.saturating_sub(consumed));
                            if remaining == consumed {
                                held.forward = false;
                                held.back = false;
                                held.left = false;
                                held.right = false;
                            }
                            if tick.is_zero() {
                                0.0
                            } else {
                                consumed.as_secs_f64() / tick.as_secs_f64()
                            }
                        } else {
                            1.0
                        };
                        let _ = reply.send((sample, fraction));
                    }
                    Command::ReplaceControl {
                        context,
                        class,
                        control: replacement,
                        reply,
                    } => {
                        let result = motion_gate.admit(
                            generation,
                            state,
                            active_output_sequence,
                            context,
                            class,
                        );
                        if result.is_ok() {
                            **actor_control.write().await = replacement;
                        }
                        let _ = reply.send(result);
                    }
                    Command::Dispatch {
                        context,
                        class,
                        packet_id,
                        payload,
                        reply,
                    } => {
                        let admission = motion_gate.admit(
                            generation,
                            state,
                            active_output_sequence,
                            context,
                            class,
                        );
                        let result = match admission {
                            Ok(()) => {
                                let mut writer = writer.lock().await;
                                let compression = writer.compression;
                                crate::protocol::write_packet(
                                    &mut writer.inner,
                                    compression,
                                    packet_id,
                                    &payload,
                                )
                                .await
                                .map_err(crate::Error::from)
                            }
                            Err(error) => Err(crate::Error::new(
                                crate::ErrorKind::State,
                                anyhow::anyhow!("packet dispatch rejected: {error:?}"),
                            )),
                        };
                        let write_failed = result.is_err() && admission.is_ok();
                        if write_failed {
                            emit_connection_diagnostic(
                                generation,
                                state,
                                "unknown_transition",
                                "normal_write_failed",
                                || {
                                    result
                                        .as_ref()
                                        .err()
                                        .map(ToString::to_string)
                                        .unwrap_or_default()
                                },
                            );
                            state = ConnectionState::ConnectionStateUnknown;
                            lifecycle_tx.send_replace(state);
                        }
                        let _ = reply.send(result);
                        if write_failed {
                            break;
                        }
                    }
                    Command::DispatchPrimitive {
                        context,
                        class,
                        packet_id,
                        payload,
                        dig_diagnostic,
                        reply,
                    } => {
                        let admission = motion_gate.admit(
                            generation,
                            state,
                            active_output_sequence,
                            context,
                            class,
                        );
                        let result = match admission {
                            Ok(()) => {
                                let write_result = {
                                    let mut writer = writer.lock().await;
                                    let compression = writer.compression;
                                    crate::protocol::write_packet(
                                        &mut writer.inner,
                                        compression,
                                        packet_id,
                                        &payload,
                                    )
                                    .await
                                };
                                match write_result {
                                    Ok(()) => {
                                        if let Some(diagnostic) = dig_diagnostic {
                                            emit_dig_lifecycle(|| {
                                                serde_json::json!({
                                                    "stage": "writer_completed",
                                                    "correlation_id": diagnostic.correlation.get(),
                                                    "generation": context.generation.get(),
                                                    "source_observation_sequence": context.source_observation_sequence,
                                                    "phase": diagnostic.phase,
                                                    "status": diagnostic.status,
                                                    "position": {"x": diagnostic.position.x, "y": diagnostic.position.y, "z": diagnostic.position.z},
                                                    "face": diagnostic.face as i8,
                                                    "packet_id": packet_id,
                                                    "outcome": "written",
                                                })
                                            });
                                            if diagnostic.status
                                                == crate::DiggingStatus::Started as i32
                                            {
                                                let identity = TransactionIdentity::Dig {
                                                    position: diagnostic.position,
                                                    status: diagnostic.status,
                                                };
                                                if let Some(previous) =
                                                    pending_dig_diagnostics.get_mut(&identity)
                                                {
                                                    if let PendingDigDiagnostic::Unique(previous) =
                                                        *previous
                                                    {
                                                        emit_dig_lifecycle(
                                                            || serde_json::json!({"stage":"ack_tracking_unavailable","correlation_id":previous.get(),"cause":"superseded_by_duplicate"}),
                                                        );
                                                    }
                                                    let cause = if matches!(
                                                        previous,
                                                        PendingDigDiagnostic::Retired
                                                    ) {
                                                        "retired_identity"
                                                    } else {
                                                        "duplicate_identity"
                                                    };
                                                    emit_dig_lifecycle(
                                                        || serde_json::json!({"stage":"ack_tracking_unavailable","correlation_id":diagnostic.correlation.get(),"cause":cause}),
                                                    );
                                                    *previous = PendingDigDiagnostic::Retired;
                                                } else if pending_dig_diagnostics.len() < 1024 {
                                                    pending_dig_diagnostics.insert(
                                                        identity,
                                                        PendingDigDiagnostic::Unique(
                                                            diagnostic.correlation,
                                                        ),
                                                    );
                                                    let expiry_commands = expiry_commands.clone();
                                                    let correlation = diagnostic.correlation;
                                                    tokio::spawn(async move {
                                                        sleep(acknowledgement_timeout).await;
                                                        let _ = expiry_commands
                                                            .send(Command::ExpireDigDiagnostic(
                                                                identity,
                                                                correlation,
                                                            ))
                                                            .await;
                                                    });
                                                } else {
                                                    emit_dig_lifecycle(
                                                        || serde_json::json!({"stage":"ack_tracking_unavailable","correlation_id":diagnostic.correlation.get(),"cause":"capacity"}),
                                                    );
                                                }
                                            }
                                        }
                                        Ok(crate::DispatchOutcome::Dispatched)
                                    }
                                    Err(error) => {
                                        emit_connection_diagnostic(
                                            generation,
                                            state,
                                            "unknown_transition",
                                            "primitive_write_failed",
                                            || error.to_string(),
                                        );
                                        state = ConnectionState::ConnectionStateUnknown;
                                        lifecycle_tx.send_replace(state);
                                        let _ =
                                            reply.send(Ok(crate::DispatchOutcome::DeliveryUnknown));
                                        break;
                                    }
                                }
                            }
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }
                    Command::DispatchInteractionBatch {
                        context,
                        class,
                        furnace_position,
                        packets,
                        reply,
                    } => {
                        let admission = motion_gate.admit(
                            generation,
                            state,
                            active_output_sequence,
                            context,
                            class,
                        );
                        match admission {
                            Err(error) => {
                                let _ = reply.send(Err(error));
                            }
                            Ok(()) => {
                                let write_result = {
                                    let mut writer = writer.lock().await;
                                    let compression = writer.compression;
                                    let mut result = Ok(());
                                    for (packet_id, payload) in packets {
                                        if let Err(error) = crate::protocol::write_packet(
                                            &mut writer.inner,
                                            compression,
                                            packet_id,
                                            &payload,
                                        )
                                        .await
                                        {
                                            result = Err(error);
                                            break;
                                        }
                                    }
                                    result
                                };
                                if write_result.is_err() {
                                    emit_connection_diagnostic(
                                        generation,
                                        state,
                                        "unknown_transition",
                                        "interaction_batch_write_failed",
                                        || {
                                            write_result
                                                .as_ref()
                                                .err()
                                                .map(ToString::to_string)
                                                .unwrap_or_default()
                                        },
                                    );
                                    state = ConnectionState::ConnectionStateUnknown;
                                    lifecycle_tx.send_replace(state);
                                    let _ = reply.send(Ok(crate::DispatchOutcome::DeliveryUnknown));
                                    break;
                                }
                                if let Some(position) = furnace_position {
                                    let now = Instant::now();
                                    if pending_furnace_interaction
                                        .is_some_and(|pending| pending.expires_at > now)
                                    {
                                        furnace_interaction_ambiguous = true;
                                    }
                                    pending_furnace_interaction = Some(PendingFurnaceInteraction {
                                        position,
                                        expires_at: now + FURNACE_CORRELATION_TTL,
                                    });
                                }
                                let _ = reply.send(Ok(crate::DispatchOutcome::Dispatched));
                            }
                        }
                    }
                    Command::DispatchAcknowledged {
                        context,
                        class,
                        operation,
                        diagnostic_correlation,
                        reply,
                    } => {
                        let admission = motion_gate.admit(
                            generation,
                            state,
                            active_output_sequence,
                            context,
                            class,
                        );
                        if let Err(error) = admission {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                        let identity = match &operation {
                            crate::AcknowledgedOperation::WindowClick { window_id, .. } => {
                                if pending_transactions.keys().any(|identity| {
                                    matches!(
                                        identity,
                                        TransactionIdentity::WindowClick {
                                            window_id: pending_window,
                                            ..
                                        } if pending_window == window_id
                                    )
                                }) {
                                    let _ = reply
                                        .send(Err(OperationAdmissionError::TransactionInProgress));
                                    continue;
                                }
                                let next = next_actions.entry(*window_id).or_insert(0);
                                *next = next.wrapping_add(1);
                                TransactionIdentity::WindowClick {
                                    window_id: *window_id,
                                    action: *next,
                                }
                            }
                            crate::AcknowledgedOperation::DigFinish { position, .. } => {
                                let identity = TransactionIdentity::Dig {
                                    position: *position,
                                    status: crate::DiggingStatus::Finished as i32,
                                };
                                if pending_transactions.contains_key(&identity)
                                    || retired_transactions.contains(&identity)
                                {
                                    let _ = reply
                                        .send(Err(OperationAdmissionError::TransactionInProgress));
                                    continue;
                                }
                                identity
                            }
                        };
                        if pending_transactions.contains_key(&identity)
                            || retired_transactions.contains(&identity)
                        {
                            let _ = reply.send(Err(OperationAdmissionError::TransactionInProgress));
                            continue;
                        }
                        if pending_transactions.len() >= MAX_PENDING_TRANSACTIONS {
                            // The existing admission error intentionally
                            // covers both per-identity overlap and bounded
                            // actor capacity.  No packet or timer is created
                            // for a rejected admission.
                            let _ = reply.send(Err(OperationAdmissionError::TransactionInProgress));
                            continue;
                        }
                        let action = match identity {
                            TransactionIdentity::WindowClick { action, .. } => action,
                            TransactionIdentity::Dig { .. } => 0,
                        };
                        let acknowledged_dig = match &operation {
                            crate::AcknowledgedOperation::DigFinish { position, face } => {
                                Some((*position, *face))
                            }
                            _ => None,
                        };
                        let encoded = operation.encode_with_action(action);
                        let Ok((packet_id, payload)) = encoded else {
                            let _ = reply.send(Err(OperationAdmissionError::InvalidOperation));
                            continue;
                        };
                        let write_result = {
                            let mut writer = writer.lock().await;
                            let compression = writer.compression;
                            crate::protocol::write_packet(
                                &mut writer.inner,
                                compression,
                                packet_id,
                                &payload,
                            )
                            .await
                        };
                        if write_result.is_err() {
                            emit_connection_diagnostic(
                                generation,
                                state,
                                "unknown_transition",
                                "transaction_write_failed",
                                || {
                                    write_result
                                        .as_ref()
                                        .err()
                                        .map(ToString::to_string)
                                        .unwrap_or_default()
                                },
                            );
                            state = ConnectionState::ConnectionStateUnknown;
                            lifecycle_tx.send_replace(state);
                            let (completion_tx, completion_rx) = oneshot::channel();
                            let _ = completion_tx.send(crate::DispatchOutcome::DeliveryUnknown);
                            let _ = reply.send(Ok(ProtocolTransaction {
                                identity,
                                dispatched: false,
                                completion: completion_rx,
                            }));
                            break;
                        }
                        if let TransactionIdentity::WindowClick { window_id, .. } = identity {
                            next_actions.insert(window_id, action);
                        }
                        let (completion_tx, completion_rx) = oneshot::channel();
                        pending_transactions.insert(
                            identity,
                            PendingTransaction {
                                completion: completion_tx,
                                confirmation_seen: false,
                                diagnostic_correlation: diagnostic_correlation
                                    .filter(|_| dig_lifecycle_trace_enabled()),
                            },
                        );
                        if let (Some(correlation), Some((position, face))) =
                            (diagnostic_correlation, acknowledged_dig)
                        {
                            emit_dig_lifecycle(|| {
                                serde_json::json!({
                                    "stage": "writer_completed",
                                    "correlation_id": correlation.get(),
                                    "generation": context.generation.get(),
                                    "source_observation_sequence": context.source_observation_sequence,
                                    "phase": "finish",
                                    "status": crate::DiggingStatus::Finished as i32,
                                    "position": {"x": position.x, "y": position.y, "z": position.z},
                                    "face": face as i8,
                                    "packet_id": packet_id,
                                    "outcome": "written",
                                })
                            });
                        }
                        let expiry_commands = expiry_commands.clone();
                        tokio::spawn(async move {
                            sleep(acknowledgement_timeout).await;
                            let _ = expiry_commands
                                .send(Command::ExpireTransaction(identity))
                                .await;
                        });
                        let _ = reply.send(Ok(ProtocolTransaction {
                            identity,
                            dispatched: true,
                            completion: completion_rx,
                        }));
                    }
                    Command::ObserveWindowConfirmation {
                        window_id,
                        action,
                        accepted,
                        reply,
                    } => {
                        let identity = TransactionIdentity::WindowClick { window_id, action };
                        let observed = if accepted {
                            if let Some(pending) = pending_transactions.get_mut(&identity) {
                                pending.confirmation_seen = true;
                                true
                            } else {
                                false
                            }
                        } else if let Some(pending) = pending_transactions.remove(&identity) {
                            let _ = pending.completion.send(crate::DispatchOutcome::Rejected);
                            true
                        } else {
                            false
                        };
                        let _ = reply.send(observed);
                    }
                    Command::CommitWindowBarrier {
                        window_id,
                        action,
                        reply,
                    } => {
                        let identity = TransactionIdentity::WindowClick { window_id, action };
                        let committed = pending_transactions
                            .get(&identity)
                            .is_some_and(|pending| pending.confirmation_seen);
                        if committed {
                            if let Some(pending) = pending_transactions.remove(&identity) {
                                let _ = pending
                                    .completion
                                    .send(crate::DispatchOutcome::Acknowledged);
                            }
                        }
                        let _ = reply.send(committed);
                    }
                    Command::ConfirmDig {
                        position,
                        status,
                        successful,
                        reply,
                    } => {
                        let identity = TransactionIdentity::Dig { position, status };
                        let confirmation =
                            if let Some(pending) = pending_transactions.remove(&identity) {
                                let correlation = pending.diagnostic_correlation;
                                let outcome = if successful {
                                    crate::DispatchOutcome::Acknowledged
                                } else {
                                    crate::DispatchOutcome::Rejected
                                };
                                let _ = pending.completion.send(outcome);
                                DigConfirmation {
                                    matched: true,
                                    diagnostic_correlation: correlation,
                                }
                            } else {
                                DigConfirmation {
                                    matched: false,
                                    diagnostic_correlation: match pending_dig_diagnostics
                                        .get(&identity)
                                        .copied()
                                    {
                                        Some(PendingDigDiagnostic::Unique(correlation)) => {
                                            pending_dig_diagnostics.remove(&identity);
                                            Some(correlation)
                                        }
                                        Some(PendingDigDiagnostic::Retired) | None => None,
                                    },
                                }
                            };
                        let _ = reply.send(confirmation);
                    }
                    Command::ObserveFurnaceWindow { window_type, reply } => {
                        // Every server window consumes the one-shot interaction context. A
                        // non-furnace window must not leave a marker that a later furnace window
                        // could accidentally claim.
                        let pending = pending_furnace_interaction.take();
                        let ambiguous = std::mem::take(&mut furnace_interaction_ambiguous);
                        let result = if window_type != FURNACE_WINDOW_TYPE || ambiguous {
                            None
                        } else {
                            let now = Instant::now();
                            pending
                                .filter(|pending| pending.expires_at > now)
                                .map(|pending| pending.position)
                        };
                        let _ = reply.send(result);
                    }
                    Command::ExpireTransaction(identity) => {
                        if let Some(pending) = pending_transactions.remove(&identity) {
                            if let Some(correlation) = pending.diagnostic_correlation {
                                emit_dig_lifecycle(|| {
                                    serde_json::json!({
                                        "stage": "ack_timeout",
                                        "correlation_id": correlation.get(),
                                    })
                                });
                            }
                            let _ = pending
                                .completion
                                .send(crate::DispatchOutcome::DeliveryUnknown);
                            retired_transactions.insert(identity);
                            if retired_transactions.len() >= MAX_RETIRED_TRANSACTIONS {
                                emit_connection_diagnostic(
                                    generation,
                                    state,
                                    "unknown_transition",
                                    "retired_transaction_capacity",
                                    || {
                                        format!(
                                            "{} / {}",
                                            retired_transactions.len(),
                                            MAX_RETIRED_TRANSACTIONS
                                        )
                                    },
                                );
                                state = ConnectionState::ConnectionStateUnknown;
                                lifecycle_tx.send_replace(state);
                                break;
                            }
                        }
                    }
                    Command::ExpireDigDiagnostic(identity, correlation) => {
                        let expires = matches!(pending_dig_diagnostics.get(&identity), Some(PendingDigDiagnostic::Unique(value)) if *value == correlation);
                        if expires {
                            pending_dig_diagnostics.insert(identity, PendingDigDiagnostic::Retired);
                            emit_dig_lifecycle(|| {
                                serde_json::json!({
                                    "stage":"start_ack_timeout",
                                    "correlation_id":correlation.get(),
                                })
                            });
                        }
                    }
                    Command::DispatchProtocol {
                        packet_id,
                        payload,
                        reply,
                    } => {
                        let admitted =
                            matches!(state, ConnectionState::Connecting | ConnectionState::Ready);
                        let result = if admitted {
                            let mut writer = writer.lock().await;
                            let compression = writer.compression;
                            crate::protocol::write_packet(
                                &mut writer.inner,
                                compression,
                                packet_id,
                                &payload,
                            )
                            .await
                            .map_err(crate::Error::from)
                        } else {
                            Err(crate::Error::new(
                                crate::ErrorKind::State,
                                anyhow::anyhow!(
                                    "protocol dispatch rejected after operation barrier"
                                ),
                            ))
                        };
                        let write_failed = result.is_err() && admitted;
                        if write_failed {
                            emit_connection_diagnostic(
                                generation,
                                state,
                                "unknown_transition",
                                "protocol_write_failed",
                                || {
                                    result
                                        .as_ref()
                                        .err()
                                        .map(ToString::to_string)
                                        .unwrap_or_default()
                                },
                            );
                            state = ConnectionState::ConnectionStateUnknown;
                            lifecycle_tx.send_replace(state);
                        }
                        let _ = reply.send(result);
                        if write_failed {
                            break;
                        }
                    }
                    Command::ShutdownWriter { reply } => {
                        let result = AsyncWriteExt::shutdown(&mut writer.lock().await.inner)
                            .await
                            .map_err(crate::Error::from);
                        let _ = reply.send(result);
                    }
                    Command::MarkTerminal(classification, reason) => {
                        if classification == TerminalClassification::ConnectionStateUnknown {
                            let (site, detail) =
                                reason.unwrap_or(("unspecified_terminal", String::new()));
                            emit_connection_diagnostic(
                                generation,
                                state,
                                "unknown_transition",
                                site,
                                || detail,
                            );
                        }
                        state = match classification {
                            TerminalClassification::Disconnected => ConnectionState::Disconnected,
                            TerminalClassification::ConnectionStateUnknown => {
                                ConnectionState::ConnectionStateUnknown
                            }
                        };
                        lifecycle_tx.send_replace(state);
                        break;
                    }
                }
            }
            for pending in pending_dig_diagnostics.into_values() {
                if let PendingDigDiagnostic::Unique(correlation) = pending {
                    emit_dig_lifecycle(|| {
                        serde_json::json!({
                            "stage":"ack_tracking_unavailable",
                            "correlation_id":correlation.get(),
                            "cause":"connection_terminal",
                        })
                    });
                }
            }
        });
        Self {
            generation,
            control,
            commands,
            lifecycle,
        }
    }

    pub(crate) const fn generation(&self) -> ConnectionGeneration {
        self.generation
    }

    pub(crate) fn lifecycle(&self) -> ConnectionState {
        *self.lifecycle.borrow()
    }

    pub(crate) async fn mark_ready(&self) {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::MarkReady { reply })
            .await
            .is_ok()
        {
            let _ = result.await;
        }
    }

    pub(crate) async fn begin_disconnect(&self) -> Result<(), OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::BeginDisconnect { reply })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    /// Publishes a successfully captured coherent observation into the same
    /// generation-local actor that serializes operation admission and writes.
    pub(crate) async fn record_observation(
        &self,
        sequence: u64,
    ) -> Result<(), OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::RecordObservation { sequence, reply })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    pub(crate) async fn admit(
        &self,
        context: OperationContext,
        class: OperationClass,
    ) -> Result<(), OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Admit {
                context,
                class,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    pub(crate) async fn replace_control(
        &self,
        context: OperationContext,
        class: OperationClass,
        control: crate::ControlState,
    ) -> Result<(), OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::ReplaceControl {
                context,
                class,
                control,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    pub(crate) async fn consume_control_for_tick(
        &self,
        tick: Duration,
    ) -> (crate::ControlState, f64) {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::ConsumeControl { tick, reply })
            .await
            .is_err()
        {
            return (crate::ControlState::default(), 0.0);
        }
        result
            .await
            .unwrap_or((crate::ControlState::default(), 0.0))
    }

    pub(crate) async fn control_snapshot(&self) -> crate::Snapshot<crate::ControlState> {
        self.control.read().await.snapshot()
    }

    pub(crate) async fn dispatch(
        &self,
        context: OperationContext,
        class: OperationClass,
        packet_id: i32,
        payload: &[u8],
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Dispatch {
                context,
                class,
                packet_id,
                payload: payload.to_vec(),
                reply,
            })
            .await
            .map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("connection actor unavailable before packet dispatch"),
                )
            })?;
        result.await.map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection actor dropped packet dispatch result"),
            )
        })?
    }

    pub(crate) async fn shutdown_writer(&self) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::ShutdownWriter { reply })
            .await
            .map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("connection actor unavailable before writer shutdown"),
                )
            })?;
        result.await.map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection actor dropped writer shutdown result"),
            )
        })?
    }

    pub(crate) async fn dispatch_operation(
        &self,
        context: OperationContext,
        class: OperationClass,
        packet_id: i32,
        payload: &[u8],
    ) -> std::result::Result<crate::DispatchOutcome, OperationAdmissionError> {
        self.dispatch_operation_with_diagnostic(context, class, packet_id, payload, None)
            .await
    }

    pub(crate) async fn dispatch_operation_with_diagnostic(
        &self,
        context: OperationContext,
        class: OperationClass,
        packet_id: i32,
        payload: &[u8],
        dig_diagnostic: Option<DigWriteDiagnostic>,
    ) -> std::result::Result<crate::DispatchOutcome, OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::DispatchPrimitive {
                context,
                class,
                packet_id,
                payload: payload.to_vec(),
                dig_diagnostic,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    pub(crate) async fn dispatch_interaction_batch(
        &self,
        context: OperationContext,
        class: OperationClass,
        furnace_position: Option<crate::BlockPos>,
        packets: Vec<(i32, Vec<u8>)>,
    ) -> std::result::Result<crate::DispatchOutcome, OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::DispatchInteractionBatch {
                context,
                class,
                furnace_position,
                packets,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    #[cfg(test)]
    pub(crate) async fn dispatch_acknowledged(
        &self,
        context: OperationContext,
        class: OperationClass,
        operation: crate::AcknowledgedOperation,
    ) -> std::result::Result<ProtocolTransaction, OperationAdmissionError> {
        self.dispatch_acknowledged_with_diagnostic(context, class, operation, None)
            .await
    }

    pub(crate) async fn dispatch_acknowledged_with_diagnostic(
        &self,
        context: OperationContext,
        class: OperationClass,
        operation: crate::AcknowledgedOperation,
        diagnostic_correlation: Option<crate::DiagnosticCorrelationId>,
    ) -> std::result::Result<ProtocolTransaction, OperationAdmissionError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::DispatchAcknowledged {
                context,
                class,
                operation,
                diagnostic_correlation,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }

    pub(crate) async fn observe_window_confirmation(
        &self,
        window_id: i8,
        action: i16,
        accepted: bool,
    ) -> bool {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::ObserveWindowConfirmation {
                window_id,
                action,
                accepted,
                reply,
            })
            .await
            .is_err()
        {
            return false;
        }
        result.await.unwrap_or(false)
    }

    pub(crate) async fn commit_window_barrier(&self, window_id: i8, action: i16) -> bool {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::CommitWindowBarrier {
                window_id,
                action,
                reply,
            })
            .await
            .is_err()
        {
            return false;
        }
        result.await.unwrap_or(false)
    }

    #[cfg(test)]
    async fn confirm_window_transaction(&self, window_id: i8, action: i16, accepted: bool) -> bool {
        self.observe_window_confirmation(window_id, action, accepted)
            .await
            && (!accepted || self.commit_window_barrier(window_id, action).await)
    }

    #[cfg(test)]
    pub(crate) async fn confirm_dig_transaction(
        &self,
        position: crate::BlockPos,
        status: i32,
        successful: bool,
    ) -> bool {
        self.confirm_dig_transaction_detailed(position, status, successful)
            .await
            .matched
    }

    pub(crate) async fn confirm_dig_transaction_detailed(
        &self,
        position: crate::BlockPos,
        status: i32,
        successful: bool,
    ) -> DigConfirmation {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::ConfirmDig {
                position,
                status,
                successful,
                reply,
            })
            .await
            .is_err()
        {
            return DigConfirmation::default();
        }
        result.await.unwrap_or_default()
    }

    pub(crate) async fn observe_furnace_window(&self, window_type: i32) -> Option<crate::BlockPos> {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::ObserveFurnaceWindow { window_type, reply })
            .await
            .is_err()
        {
            return None;
        }
        result.await.unwrap_or(None)
    }

    pub(crate) async fn dispatch_protocol(
        &self,
        packet_id: i32,
        payload: &[u8],
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::DispatchProtocol {
                packet_id,
                payload: payload.to_vec(),
                reply,
            })
            .await
            .map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("connection actor unavailable before protocol dispatch"),
                )
            })?;
        result.await.map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection actor dropped protocol dispatch result"),
            )
        })?
    }

    pub(crate) async fn mark_terminal(&self, classification: TerminalClassification) {
        let _ = self
            .commands
            .send(Command::MarkTerminal(classification, None))
            .await;
    }

    /// Diagnostic context is consumed by the existing state owner, not a second
    /// lifecycle or retained terminal-reason store.
    pub(crate) async fn mark_unknown(&self, site: &'static str, detail: String) {
        let _ = self
            .commands
            .send(Command::MarkTerminal(
                TerminalClassification::ConnectionStateUnknown,
                Some((site, detail)),
            ))
            .await;
    }

    pub(crate) async fn wait_for_terminal(&self) -> ConnectionState {
        let mut lifecycle = self.lifecycle.clone();
        loop {
            let state = *lifecycle.borrow_and_update();
            if state.is_terminal() {
                return state;
            }
            if lifecycle.changed().await.is_err() {
                emit_connection_diagnostic(
                    self.generation,
                    state,
                    "terminal_observation",
                    "lifecycle_watch_closed",
                    || "lifecycle sender ended without a terminal value".to_owned(),
                );
                return ConnectionState::ConnectionStateUnknown;
            }
        }
    }

    fn terminal_admission_error(&self) -> OperationAdmissionError {
        match self.lifecycle() {
            ConnectionState::Connecting => OperationAdmissionError::Connecting,
            ConnectionState::Ready | ConnectionState::Disconnecting => {
                OperationAdmissionError::ConnectionStateUnknown
            }
            ConnectionState::Disconnected => OperationAdmissionError::Disconnected,
            ConnectionState::ConnectionStateUnknown => {
                OperationAdmissionError::ConnectionStateUnknown
            }
        }
    }
}

fn admit(
    generation: ConnectionGeneration,
    state: ConnectionState,
    active_output_sequence: Option<u64>,
    context: OperationContext,
    class: OperationClass,
) -> Result<(), OperationAdmissionError> {
    if context.generation != generation {
        return Err(OperationAdmissionError::StaleGeneration);
    }
    // Sequence zero is retained only for legacy convenience methods. Formal
    // Context-bound packet operations must remain bound to the output sequence activated
    // by the actor preflight, even if a newer observation has since published.
    if context.source_observation_sequence != 0
        && active_output_sequence != Some(context.source_observation_sequence)
    {
        return Err(OperationAdmissionError::InvalidOperation);
    }
    admit_lifecycle(state, class)
}

fn activate_output(
    generation: ConnectionGeneration,
    state: ConnectionState,
    latest_observation_sequence: Option<u64>,
    active_output_sequence: &mut Option<u64>,
    context: OperationContext,
    class: OperationClass,
) -> Result<(), OperationAdmissionError> {
    if context.generation != generation {
        return Err(OperationAdmissionError::StaleGeneration);
    }
    admit_lifecycle(state, class)?;
    let sequence = context.source_observation_sequence;
    if sequence == 0 {
        return Ok(());
    }
    if latest_observation_sequence.is_none_or(|latest| sequence > latest)
        || active_output_sequence.is_some_and(|active| sequence < active)
    {
        return Err(OperationAdmissionError::InvalidOperation);
    }
    // Repeating the same preflight is actor-idempotent. Caller output identity
    // remains the higher-level duplicate owner and prevents replayed Outputs.
    *active_output_sequence = Some(sequence);
    Ok(())
}

fn admit_lifecycle(
    state: ConnectionState,
    class: OperationClass,
) -> Result<(), OperationAdmissionError> {
    match (state, class) {
        (ConnectionState::Ready, _) | (ConnectionState::Disconnecting, OperationClass::Cleanup) => {
            Ok(())
        }
        (ConnectionState::Connecting, _) => Err(OperationAdmissionError::Connecting),
        (ConnectionState::Disconnecting, OperationClass::Normal) => {
            Err(OperationAdmissionError::Disconnecting)
        }
        (ConnectionState::Disconnected, _) => Err(OperationAdmissionError::Disconnected),
        (ConnectionState::ConnectionStateUnknown, _) => {
            Err(OperationAdmissionError::ConnectionStateUnknown)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::read_packet;
    use tokio::{
        io::{AsyncRead, AsyncReadExt},
        net::{TcpListener, TcpStream},
        time::{Duration, timeout},
    };

    async fn actor_fixture() -> (ConnectionActor, TcpStream) {
        let (actor, server, _) = actor_fixture_with_writer().await;
        (actor, server)
    }

    async fn actor_fixture_with_writer() -> (
        ConnectionActor,
        TcpStream,
        Arc<Mutex<crate::versions::java_1_16_1::client::PacketWriter>>,
    ) {
        actor_fixture_with_ack_timeout(Duration::from_secs(5)).await
    }

    async fn actor_fixture_with_ack_timeout(
        acknowledgement_timeout: Duration,
    ) -> (
        ConnectionActor,
        TcpStream,
        Arc<Mutex<crate::versions::java_1_16_1::client::PacketWriter>>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let (_, writer) = client.into_split();
        let writer = Arc::new(Mutex::new(
            crate::versions::java_1_16_1::client::PacketWriter {
                inner: writer,
                compression: None,
            },
        ));
        let control = Arc::new(RwLock::new(crate::snapshot::Versioned::new(
            crate::ControlState::default(),
            std::time::Instant::now(),
        )));
        (
            ConnectionActor::spawn(writer.clone(), acknowledgement_timeout, control),
            server,
            writer,
        )
    }

    include!("lifecycle/bounded_motion_tests.rs");

    async fn no_packet<R: AsyncRead + Unpin>(reader: &mut R) {
        match timeout(Duration::from_millis(50), read_packet(reader, None)).await {
            Err(_) | Ok(Err(_)) => {}
            Ok(Ok(packet)) => panic!("unexpected packet after barrier: {packet:?}"),
        }
    }

    #[tokio::test]
    async fn generation_and_disconnect_barrier_are_fail_closed() {
        let (actor, _server) = actor_fixture().await;
        let current = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor.admit(current, OperationClass::Normal).await,
            Err(OperationAdmissionError::Connecting)
        );
        actor.mark_ready().await;
        assert_eq!(actor.admit(current, OperationClass::Normal).await, Ok(()));

        let stale = OperationContext {
            generation: ConnectionGeneration(current.generation.get() + 1),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor.admit(stale, OperationClass::Normal).await,
            Err(OperationAdmissionError::StaleGeneration)
        );

        actor.begin_disconnect().await.unwrap();
        assert_eq!(
            actor.admit(current, OperationClass::Normal).await,
            Err(OperationAdmissionError::Disconnecting)
        );
        assert_eq!(actor.admit(current, OperationClass::Cleanup).await, Ok(()));
        actor
            .mark_terminal(TerminalClassification::Disconnected)
            .await;
        assert_eq!(
            actor.wait_for_terminal().await,
            ConnectionState::Disconnected
        );
        assert_eq!(
            actor.admit(current, OperationClass::Cleanup).await,
            Err(OperationAdmissionError::Disconnected)
        );
    }

    #[tokio::test]
    async fn unknown_is_terminal_and_never_becomes_disconnected() {
        let (actor, _server) = actor_fixture().await;
        actor.mark_ready().await;
        actor
            .mark_unknown("test_first_reason", "first diagnostic".to_owned())
            .await;
        assert_eq!(
            actor.wait_for_terminal().await,
            ConnectionState::ConnectionStateUnknown
        );
        actor
            .mark_unknown("test_later_reason", "must not replace first".to_owned())
            .await;
        actor
            .mark_terminal(TerminalClassification::Disconnected)
            .await;
        assert_eq!(actor.lifecycle(), ConnectionState::ConnectionStateUnknown);
    }

    #[tokio::test]
    async fn each_actor_allocates_a_fresh_generation() {
        let (first, _first_server) = actor_fixture().await;
        let (second, _second_server) = actor_fixture().await;
        assert_ne!(first.generation(), second.generation());
        assert!(second.generation().get() > first.generation().get());
    }

    #[tokio::test]
    async fn output_activation_survives_newer_observation_then_retires_on_next_output() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        assert_eq!(actor.record_observation(1).await, Ok(()));
        assert_eq!(actor.record_observation(2).await, Ok(()));

        let first = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 1,
        };
        assert_eq!(actor.admit(first, OperationClass::Normal).await, Ok(()));
        // Repeating the same explicit preflight is actor-idempotent; The caller owns
        // duplicate Output rejection above this packet boundary.
        assert_eq!(actor.admit(first, OperationClass::Normal).await, Ok(()));
        assert_eq!(
            actor
                .dispatch_operation(first, OperationClass::Normal, 0x01, &[1])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            actor
                .dispatch_operation(first, OperationClass::Normal, 0x02, &[2])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );

        let future = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 3,
        };
        assert_eq!(
            actor.admit(future, OperationClass::Normal).await,
            Err(OperationAdmissionError::InvalidOperation)
        );

        let second = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 2,
        };
        assert_eq!(actor.admit(second, OperationClass::Normal).await, Ok(()));
        assert_eq!(
            actor.admit(first, OperationClass::Normal).await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        assert_eq!(
            actor
                .dispatch_operation(first, OperationClass::Normal, 0x03, &[])
                .await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        actor.begin_disconnect().await.unwrap();
        assert_eq!(
            actor
                .dispatch_operation(second, OperationClass::Cleanup, 0x04, &[4])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x01, vec![1])
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x02, vec![2])
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x04, vec![4])
        );
    }

    #[tokio::test]
    async fn observation_publication_is_ready_only_and_monotonic() {
        let (actor, _server) = actor_fixture().await;
        assert_eq!(
            actor.record_observation(1).await,
            Err(OperationAdmissionError::Connecting)
        );
        actor.mark_ready().await;
        assert_eq!(
            actor.record_observation(0).await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        assert_eq!(actor.record_observation(4).await, Ok(()));
        assert_eq!(
            actor.record_observation(4).await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        assert_eq!(
            actor.record_observation(3).await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        assert_eq!(actor.record_observation(5).await, Ok(()));
    }

    #[tokio::test]
    async fn zero_sequence_legacy_context_remains_compatible() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let legacy = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_operation(legacy, OperationClass::Normal, 0x04, &[9])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x04, vec![9])
        );
    }

    #[tokio::test]
    async fn a_new_actor_starts_its_observation_sequence_at_one() {
        let (first, _first_server) = actor_fixture().await;
        first.mark_ready().await;
        first.record_observation(9).await.unwrap();
        let first_context = OperationContext {
            generation: first.generation(),
            source_observation_sequence: 9,
        };

        let (second, mut second_server) = actor_fixture().await;
        second.mark_ready().await;
        assert_eq!(second.record_observation(1).await, Ok(()));
        let second_context = OperationContext {
            generation: second.generation(),
            source_observation_sequence: 1,
        };
        assert_eq!(
            second.admit(second_context, OperationClass::Normal).await,
            Ok(())
        );
        assert_eq!(
            second
                .dispatch_operation(second_context, OperationClass::Normal, 0x05, &[])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            second
                .dispatch_operation(first_context, OperationClass::Normal, 0x06, &[])
                .await,
            Err(OperationAdmissionError::StaleGeneration)
        );
        let old_sequence_on_new_generation = OperationContext {
            generation: second.generation(),
            source_observation_sequence: 9,
        };
        assert_eq!(
            second
                .dispatch_operation(
                    old_sequence_on_new_generation,
                    OperationClass::Normal,
                    0x07,
                    &[],
                )
                .await,
            Err(OperationAdmissionError::InvalidOperation)
        );
        assert_eq!(
            read_packet(&mut second_server, None).await.unwrap(),
            (0x05, vec![])
        );
        no_packet(&mut second_server).await;
    }

    #[tokio::test]
    async fn stale_dispatch_writes_zero_tcp_bytes() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let stale = OperationContext {
            generation: ConnectionGeneration(actor.generation().get() + 1),
            source_observation_sequence: 0,
        };
        assert!(
            actor
                .dispatch(stale, OperationClass::Normal, 0x01, &[])
                .await
                .is_err()
        );
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn typed_dispatch_reports_dispatch_without_claiming_acknowledgement() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_operation(context, OperationClass::Normal, 0x01, &[8])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x01, vec![8])
        );
    }

    #[tokio::test]
    async fn typed_stale_dispatch_is_rejected_before_write() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let stale = OperationContext {
            generation: ConnectionGeneration(actor.generation().get() + 1),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_operation(stale, OperationClass::Normal, 0x02, &[])
                .await,
            Err(OperationAdmissionError::StaleGeneration)
        );
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn interaction_batch_is_ordered_once_and_placement_has_no_furnace_correlation() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_interaction_batch(
                    context,
                    OperationClass::Normal,
                    None,
                    vec![(0x1c, vec![1]), (0x2d, vec![2]), (0x1c, vec![3])],
                )
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x1c, vec![1])
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x2d, vec![2])
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x1c, vec![3])
        );
        no_packet(&mut server).await;
        assert_eq!(actor.observe_furnace_window(13).await, None);
    }

    #[tokio::test]
    async fn stale_interaction_batch_writes_zero_tcp_bytes() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let stale = OperationContext {
            generation: ConnectionGeneration(actor.generation().get() + 1),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_interaction_batch(
                    stale,
                    OperationClass::Normal,
                    None,
                    vec![(0x1c, vec![1]), (0x2d, vec![2]), (0x1c, vec![3])],
                )
                .await,
            Err(OperationAdmissionError::StaleGeneration)
        );
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn interaction_batch_write_failure_is_delivery_unknown() {
        let (actor, _server, writer) = actor_fixture_with_writer().await;
        writer.lock().await.inner.shutdown().await.unwrap();
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_interaction_batch(
                    context,
                    OperationClass::Normal,
                    None,
                    vec![(0x1c, vec![1]), (0x2d, vec![2]), (0x1c, vec![3])],
                )
                .await,
            Ok(crate::DispatchOutcome::DeliveryUnknown)
        );
        assert_eq!(actor.lifecycle(), ConnectionState::ConnectionStateUnknown);
    }

    #[tokio::test]
    async fn dropping_interaction_batch_waiter_does_not_cancel_actor_owned_writes() {
        let (actor, mut server, writer) = actor_fixture_with_writer().await;
        actor.mark_ready().await;
        let writer_guard = writer.lock().await;
        let dispatch_actor = actor.clone();
        let dispatch = tokio::spawn(async move {
            dispatch_actor
                .dispatch_interaction_batch(
                    OperationContext {
                        generation: dispatch_actor.generation(),
                        source_observation_sequence: 0,
                    },
                    OperationClass::Normal,
                    None,
                    vec![(0x1c, vec![1]), (0x2d, vec![2]), (0x1c, vec![3])],
                )
                .await
        });
        tokio::task::yield_now().await;
        dispatch.abort();
        drop(writer_guard);

        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x1c, vec![1])
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x2d, vec![2])
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x1c, vec![3])
        );
    }

    #[tokio::test]
    async fn control_replacement_is_actor_owned_and_barriered() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        let moving = crate::ControlState {
            forward: true,
            sprint: true,
            ..crate::ControlState::default()
        };
        actor
            .replace_control(context, OperationClass::Normal, moving)
            .await
            .unwrap();
        let moving_snapshot = actor.control_snapshot().await;
        assert_eq!(moving_snapshot.value, moving);
        assert_eq!(moving_snapshot.revision, 1);

        actor.begin_disconnect().await.unwrap();
        assert_eq!(
            actor
                .replace_control(
                    context,
                    OperationClass::Normal,
                    crate::ControlState::default()
                )
                .await,
            Err(OperationAdmissionError::Disconnecting)
        );
        assert_eq!(actor.control_snapshot().await.value, moving);
        actor
            .replace_control(
                context,
                OperationClass::Cleanup,
                crate::ControlState::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            actor.control_snapshot().await.value,
            crate::ControlState::default()
        );
        assert_eq!(actor.control_snapshot().await.revision, 2);
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn movement_press_duration_is_consumed_and_replaced_at_tick_boundaries() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };

        let mut moving = crate::ControlState {
            forward: true,
            sprint: true,
            movement_press_duration: Some(Duration::from_millis(12) + Duration::from_micros(500)),
            ..crate::ControlState::default()
        };
        actor
            .replace_control(context, OperationClass::Normal, moving)
            .await
            .unwrap();
        let (sample, fraction) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert!(sample.forward);
        assert_eq!(
            sample.movement_press_duration,
            Some(Duration::from_micros(12_500))
        );
        assert!((fraction - 0.25).abs() < 1e-12);
        let held_after_expiry = actor.control_snapshot().await.value;
        assert!(
            !held_after_expiry.forward,
            "expired movement must release held forward"
        );
        assert!(
            held_after_expiry.sprint,
            "posture controls remain held across movement expiry"
        );
        let (sample, fraction) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert!(
            !sample.forward,
            "the next tick observes the released movement controls"
        );
        assert_eq!(sample.movement_press_duration, Some(Duration::ZERO));
        assert_eq!(fraction, 0.0);

        moving.movement_press_duration = Some(Duration::from_millis(75));
        actor
            .replace_control(context, OperationClass::Normal, moving)
            .await
            .unwrap();
        let (sample, fraction) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert_eq!(
            sample.movement_press_duration,
            Some(Duration::from_millis(75))
        );
        assert_eq!(fraction, 1.0);
        let (sample, fraction) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert_eq!(
            sample.movement_press_duration,
            Some(Duration::from_millis(25))
        );
        assert_eq!(fraction, 0.5);
        let (sample, fraction) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert_eq!(sample.movement_press_duration, Some(Duration::ZERO));
        assert_eq!(fraction, 0.0);

        moving.forward = false;
        moving.right = true;
        moving.movement_press_duration = Some(Duration::from_millis(75));
        actor
            .replace_control(context, OperationClass::Normal, moving)
            .await
            .unwrap();
        let (sample, _) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert!(!sample.forward && sample.right);

        actor
            .replace_control(
                context,
                OperationClass::Cleanup,
                crate::ControlState::default(),
            )
            .await
            .unwrap();
        let (sample, fraction) = actor
            .consume_control_for_tick(Duration::from_millis(50))
            .await;
        assert_eq!(sample, crate::ControlState::default());
        assert_eq!(
            fraction, 1.0,
            "legacy None duration remains held-key compatible"
        );
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn typed_write_failure_is_delivery_unknown_and_lifecycle_unknown() {
        let (actor, mut server, writer) = actor_fixture_with_writer().await;
        writer.lock().await.inner.shutdown().await.unwrap();
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert_eq!(
            actor
                .dispatch_interaction_batch(
                    context,
                    OperationClass::Normal,
                    Some(crate::BlockPos { x: 1, y: 2, z: 3 }),
                    vec![(0x03, vec![1])],
                )
                .await,
            Ok(crate::DispatchOutcome::DeliveryUnknown)
        );
        assert_eq!(actor.lifecycle(), ConnectionState::ConnectionStateUnknown);
        assert_eq!(actor.observe_furnace_window(13).await, None);
        assert!(
            actor
                .dispatch_operation(context, OperationClass::Normal, 0x04, &[])
                .await
                .is_err()
        );
        let mut byte = [0_u8; 1];
        assert_eq!(server.read(&mut byte).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn acknowledged_write_failure_is_delivery_unknown() {
        let (actor, _server, writer) = actor_fixture_with_writer().await;
        writer.lock().await.inner.shutdown().await.unwrap();
        actor.mark_ready().await;
        let transaction = actor
            .dispatch_acknowledged(
                OperationContext {
                    generation: actor.generation(),
                    source_observation_sequence: 0,
                },
                OperationClass::Normal,
                crate::AcknowledgedOperation::DigFinish {
                    position: crate::BlockPos { x: 1, y: 2, z: 3 },
                    face: crate::BlockFace::Up,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            transaction.wait().await,
            crate::DispatchOutcome::DeliveryUnknown
        );
        assert_eq!(actor.lifecycle(), ConnectionState::ConnectionStateUnknown);
    }

    #[tokio::test]
    async fn typed_cleanup_dispatch_is_allowed_after_disconnect_barrier() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        actor.begin_disconnect().await.unwrap();
        assert_eq!(
            actor
                .dispatch_operation(context, OperationClass::Normal, 0x0b, &[1])
                .await,
            Err(OperationAdmissionError::Disconnecting)
        );
        assert_eq!(
            actor
                .dispatch_operation(context, OperationClass::Cleanup, 0x0c, &[2])
                .await,
            Ok(crate::DispatchOutcome::Dispatched)
        );
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x0c, vec![2])
        );
    }

    #[tokio::test]
    async fn acknowledged_window_transaction_owns_action_and_confirmation() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        let transaction = actor
            .dispatch_acknowledged(
                context,
                OperationClass::Normal,
                crate::AcknowledgedOperation::WindowClick {
                    window_id: 0,
                    slot: 9,
                    button: 0,
                    mode: crate::ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        let (packet_id, payload) = read_packet(&mut server, None).await.unwrap();
        assert_eq!(packet_id, 0x09);
        let action = i16::from_be_bytes([payload[4], payload[5]]);
        assert_eq!(transaction.window_action(), Some(action));
        actor.confirm_window_transaction(0, action, true).await;
        assert_eq!(
            transaction.wait().await,
            crate::DispatchOutcome::Acknowledged
        );
    }

    #[tokio::test]
    async fn accepted_window_confirmation_waits_for_opaque_barrier_commit() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let transaction = actor
            .dispatch_acknowledged(
                OperationContext {
                    generation: actor.generation(),
                    source_observation_sequence: 0,
                },
                OperationClass::Normal,
                crate::AcknowledgedOperation::WindowClick {
                    window_id: 0,
                    slot: 9,
                    button: 0,
                    mode: crate::ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        let (_, payload) = read_packet(&mut server, None).await.unwrap();
        let action = i16::from_be_bytes([payload[4], payload[5]]);
        let mut waiter = tokio::spawn(transaction.wait());
        assert!(actor.observe_window_confirmation(0, action, true).await);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiter)
                .await
                .is_err()
        );
        assert!(actor.commit_window_barrier(0, action).await);
        assert_eq!(waiter.await.unwrap(), crate::DispatchOutcome::Acknowledged);
        assert!(!actor.commit_window_barrier(0, action).await);
    }

    #[tokio::test]
    async fn stale_acknowledged_dispatch_is_rejected_before_write() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let stale = OperationContext {
            generation: ConnectionGeneration(actor.generation().get() + 1),
            source_observation_sequence: 0,
        };
        assert!(matches!(
            actor
                .dispatch_acknowledged(
                    stale,
                    OperationClass::Normal,
                    crate::AcknowledgedOperation::WindowClick {
                        window_id: 0,
                        slot: 0,
                        button: 0,
                        mode: crate::ClickMode::Normal,
                        clicked: None,
                    },
                )
                .await,
            Err(OperationAdmissionError::StaleGeneration)
        ));
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn negative_ack_is_rejected_and_dropped_future_stays_pending() {
        let (actor, mut server, _writer) =
            actor_fixture_with_ack_timeout(Duration::from_millis(10)).await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        let first = actor
            .dispatch_acknowledged(
                context,
                OperationClass::Normal,
                crate::AcknowledgedOperation::WindowClick {
                    window_id: 1,
                    slot: 0,
                    button: 0,
                    mode: crate::ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        let (_, payload) = read_packet(&mut server, None).await.unwrap();
        let action = i16::from_be_bytes([payload[4], payload[5]]);
        actor.confirm_window_transaction(1, action, false).await;
        assert_eq!(first.wait().await, crate::DispatchOutcome::Rejected);
        let dropped = actor
            .dispatch_acknowledged(
                context,
                OperationClass::Normal,
                crate::AcknowledgedOperation::WindowClick {
                    window_id: 1,
                    slot: 1,
                    button: 0,
                    mode: crate::ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        let (_, payload) = read_packet(&mut server, None).await.unwrap();
        let dropped_action = i16::from_be_bytes([payload[4], payload[5]]);
        drop(dropped);
        assert!(matches!(
            actor
                .dispatch_acknowledged(
                    context,
                    OperationClass::Normal,
                    crate::AcknowledgedOperation::WindowClick {
                        window_id: 1,
                        slot: 1,
                        button: 0,
                        mode: crate::ClickMode::Normal,
                        clicked: None,
                    },
                )
                .await,
            Err(OperationAdmissionError::TransactionInProgress)
        ));
        actor
            .confirm_window_transaction(1, dropped_action, true)
            .await;
        // The actor processes confirmation before the next dispatch because
        // both commands share one FIFO mailbox.
        let second = actor
            .dispatch_acknowledged(
                context,
                OperationClass::Normal,
                crate::AcknowledgedOperation::WindowClick {
                    window_id: 1,
                    slot: 1,
                    button: 0,
                    mode: crate::ClickMode::Normal,
                    clicked: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(second.window_action(), Some(dropped_action.wrapping_add(1)));
        assert_eq!(second.wait().await, crate::DispatchOutcome::DeliveryUnknown);
    }

    #[tokio::test]
    async fn acknowledgement_deadline_is_actor_owned() {
        let (actor, mut server, _writer) =
            actor_fixture_with_ack_timeout(Duration::from_millis(10)).await;
        actor.mark_ready().await;
        let position = crate::BlockPos { x: 3, y: 4, z: 5 };
        let transaction = actor
            .dispatch_acknowledged(
                OperationContext {
                    generation: actor.generation(),
                    source_observation_sequence: 0,
                },
                OperationClass::Normal,
                crate::AcknowledgedOperation::DigFinish {
                    position,
                    face: crate::BlockFace::Up,
                },
            )
            .await
            .unwrap();
        let (packet_id, _) = read_packet(&mut server, None).await.unwrap();
        assert_eq!(packet_id, 0x1b);
        assert_eq!(
            transaction.wait().await,
            crate::DispatchOutcome::DeliveryUnknown
        );
        assert!(matches!(
            actor
                .dispatch_acknowledged(
                    OperationContext {
                        generation: actor.generation(),
                        source_observation_sequence: 0,
                    },
                    OperationClass::Normal,
                    crate::AcknowledgedOperation::DigFinish {
                        position,
                        face: crate::BlockFace::Up,
                    },
                )
                .await,
            Err(OperationAdmissionError::TransactionInProgress)
        ));
    }

    #[tokio::test]
    async fn pending_acknowledgements_have_a_bounded_actor_owned_capacity() {
        let (actor, _server, _writer) =
            actor_fixture_with_ack_timeout(Duration::from_secs(5)).await;
        actor.mark_ready().await;
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        let mut pending = Vec::with_capacity(MAX_PENDING_TRANSACTIONS);
        for index in 0..MAX_PENDING_TRANSACTIONS {
            pending.push(
                actor
                    .dispatch_acknowledged(
                        context,
                        OperationClass::Normal,
                        crate::AcknowledgedOperation::DigFinish {
                            position: crate::BlockPos {
                                x: index as i32,
                                y: 4,
                                z: 5,
                            },
                            face: crate::BlockFace::Up,
                        },
                    )
                    .await
                    .expect("each distinct identity fits the pending bound"),
            );
        }
        assert!(matches!(
            actor
                .dispatch_acknowledged(
                    context,
                    OperationClass::Normal,
                    crate::AcknowledgedOperation::DigFinish {
                        position: crate::BlockPos {
                            x: MAX_PENDING_TRANSACTIONS as i32,
                            y: 4,
                            z: 5,
                        },
                        face: crate::BlockFace::Up,
                    },
                )
                .await,
            Err(OperationAdmissionError::TransactionInProgress)
        ));

        actor
            .mark_terminal(TerminalClassification::ConnectionStateUnknown)
            .await;
        assert_eq!(
            pending
                .pop()
                .expect("at least one pending transaction")
                .wait()
                .await,
            crate::DispatchOutcome::DeliveryUnknown
        );
        drop(pending);
    }

    #[tokio::test]
    async fn acknowledged_dig_transaction_routes_positive_ack_without_semantic_claim() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let position = crate::BlockPos { x: -3, y: 4, z: 5 };
        actor.record_observation(1).await.unwrap();
        let context = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 1,
        };
        actor.admit(context, OperationClass::Normal).await.unwrap();
        // A newer capture does not replace the active Output or its client-owned
        // transaction identity before the next explicit activation.
        actor.record_observation(2).await.unwrap();
        let transaction = actor
            .dispatch_acknowledged(
                context,
                OperationClass::Normal,
                crate::AcknowledgedOperation::DigFinish {
                    position,
                    face: crate::BlockFace::North,
                },
            )
            .await
            .unwrap();
        let (packet_id, _) = read_packet(&mut server, None).await.unwrap();
        assert_eq!(packet_id, 0x1b);
        assert!(
            !actor
                .confirm_dig_transaction(position, crate::DiggingStatus::Started as i32, true)
                .await
        );
        assert!(matches!(
            actor
                .dispatch_acknowledged(
                    OperationContext {
                        generation: actor.generation(),
                        source_observation_sequence: 0,
                    },
                    OperationClass::Normal,
                    crate::AcknowledgedOperation::DigFinish {
                        position,
                        face: crate::BlockFace::North,
                    },
                )
                .await,
            Err(OperationAdmissionError::TransactionInProgress)
        ));
        actor
            .confirm_dig_transaction(position, crate::DiggingStatus::Finished as i32, true)
            .await;
        assert_eq!(
            transaction.wait().await,
            crate::DispatchOutcome::Acknowledged
        );
    }

    #[tokio::test]
    async fn dispatch_before_disconnect_is_written_and_barrier_blocks_later_packets() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let current = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        actor
            .dispatch(current, OperationClass::Normal, 0x01, &[1])
            .await
            .unwrap();
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x01, vec![1])
        );
        actor.begin_disconnect().await.unwrap();
        assert!(
            actor
                .dispatch(current, OperationClass::Normal, 0x02, &[])
                .await
                .is_err()
        );
        assert!(actor.dispatch_protocol(0x03, &[]).await.is_err());
        no_packet(&mut server).await;
    }

    #[tokio::test]
    async fn disconnecting_cleanup_dispatch_is_written() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let current = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        actor.begin_disconnect().await.unwrap();
        actor
            .dispatch(current, OperationClass::Cleanup, 0x04, &[2])
            .await
            .unwrap();
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x04, vec![2])
        );
    }

    #[tokio::test]
    async fn dispatch_then_shutdown_preserves_command_order() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        let current = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        actor
            .dispatch(current, OperationClass::Normal, 0x05, &[3])
            .await
            .unwrap();
        actor.shutdown_writer().await.unwrap();
        assert_eq!(
            read_packet(&mut server, None).await.unwrap(),
            (0x05, vec![3])
        );
        assert!(
            timeout(Duration::from_millis(100), read_packet(&mut server, None))
                .await
                .unwrap()
                .is_err()
        );
    }

    #[tokio::test]
    async fn normal_write_failure_becomes_unknown_and_blocks_later_bytes() {
        let (actor, mut server, writer) = actor_fixture_with_writer().await;
        writer.lock().await.inner.shutdown().await.unwrap();
        actor.mark_ready().await;
        let current = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert!(
            actor
                .dispatch(current, OperationClass::Normal, 0x08, &[4])
                .await
                .is_err()
        );
        assert_eq!(actor.lifecycle(), ConnectionState::ConnectionStateUnknown);
        assert!(
            actor
                .dispatch(current, OperationClass::Normal, 0x09, &[5])
                .await
                .is_err()
        );
        let mut byte = [0_u8; 1];
        assert_eq!(server.read(&mut byte).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn protocol_write_failure_becomes_unknown_and_blocks_later_bytes() {
        let (actor, mut server, writer) = actor_fixture_with_writer().await;
        writer.lock().await.inner.shutdown().await.unwrap();
        actor.mark_ready().await;
        assert!(actor.dispatch_protocol(0x0a, &[6]).await.is_err());
        assert_eq!(actor.lifecycle(), ConnectionState::ConnectionStateUnknown);
        assert!(actor.dispatch_protocol(0x0b, &[7]).await.is_err());
        let mut byte = [0_u8; 1];
        assert_eq!(server.read(&mut byte).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn terminal_state_blocks_normal_and_protocol_dispatch() {
        let (actor, mut server) = actor_fixture().await;
        actor.mark_ready().await;
        actor
            .mark_terminal(TerminalClassification::Disconnected)
            .await;
        let current = OperationContext {
            generation: actor.generation(),
            source_observation_sequence: 0,
        };
        assert!(
            actor
                .dispatch(current, OperationClass::Normal, 0x06, &[])
                .await
                .is_err()
        );
        assert!(actor.dispatch_protocol(0x07, &[]).await.is_err());
        no_packet(&mut server).await;
    }
}
