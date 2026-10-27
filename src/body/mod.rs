//! Acceptance-based actions shared by headless and future attached clients.
//!
//! This API is additive. Existing receipt APIs retain their stricter meaning.
//! See `docs/design/body-api.md` for evidence, deadlines and cancellation.
use crate::client::{BlockFace, GameMode, Hand, Health, ItemData};
use crate::{MinecraftVersion, NativeBlockState, Region, Result};
use std::future::Future;
use std::time::Duration;

use crate::client::control::Controls;

pub(crate) mod dig;
pub(crate) mod evidence;
mod headless;
mod monitoring;
pub use monitoring::*;
pub(crate) mod interaction;
mod state;
pub use headless::HeadlessBody;

/// Backend-neutral source; acceptance never changes an observation's source.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum ValueSource {
    /// Original packet supplying this field.
    Received {
        /// Connection receive ordinal, not a server tick.
        sequence: u64,
    },
    /// Server-supplied block cache; revision is a capture boundary, not a field receipt.
    ReceivedCache {
        /// Local receive high-water mark.
        revision: u64,
    },
    /// Completely submitted local value.
    Submitted,
    /// Headless physics prediction.
    Predicted,
    /// Compatibility cache.
    LocalCache,
    /// Future attached client's live state, including its local prediction.
    ClientState,
}
/// A value and its original source.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Value<T> {
    /// Captured value.
    pub value: T,
    /// Origin; no implication of server currentness.
    pub source: ValueSource,
}
/// Connection/world lifetime. Backends must refuse stale requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct BodyWorld {
    /// Backend-local connection identity.
    pub connection: u64,
    /// Changes on respawn, world change or reconfiguration.
    pub generation: u64,
}
/// Neutral inventory item. Optional native data is interpreted using snapshot.version.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BodyItem {
    /// Namespaced item name, without a registry-bound authority token.
    pub name: String,
    /// Stack count.
    pub count: u32,
    /// Optional lossless native data; absence means unavailable, not default.
    pub data: Option<ItemData>,
}
/// Knowledge of one inventory location.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum BodySlot {
    /// Not established or unsupported.
    Unavailable,
    /// Established empty.
    Empty,
    /// Known item.
    Item(BodyItem),
}
/// Player state; missing fields never imply zero, air or an empty slot.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BodyPlayer {
    /// Feet position.
    pub position: Option<Value<[f64; 3]>>,
    /// Yaw/pitch in degrees.
    pub rotation: Option<Value<[f32; 2]>>,
    /// Received mode when known.
    pub game_mode: Option<GameMode>,
    /// Health, food and saturation.
    pub health: Option<Value<Health>>,
    /// Main hand slot, 0..8.
    pub selected_hotbar: Option<Value<u8>>,
    /// Player screen slots 0..45. None means no knowledge.
    pub inventory: Vec<Option<Value<BodySlot>>>,
    /// Carried cursor item.
    pub cursor: Option<Value<BodySlot>>,
}
/// One requested block, including explicit unavailability.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BodyBlock {
    /// Absolute block coordinate.
    pub position: [i32; 3],
    /// Missing means unloaded/unknown, never inferred air.
    pub state: Option<Value<NativeBlockState>>,
}
/// Observation selection; the player is always included.
#[derive(Clone, Debug, Default)]
pub struct ObserveRequest {
    /// Individual cells, useful without a rectangular region.
    pub blocks: Vec<[i32; 3]>,
    /// Optional inclusive region, bounded by Region::volume.
    pub region: Option<Region>,
}
/// State captured on one backend boundary.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BodySnapshot {
    /// Owning backend instance.
    pub backend: u64,
    /// Protocol/data version.
    pub version: MinecraftVersion,
    /// Connection/world lifetime.
    pub world: BodyWorld,
    /// Backend revision; not a server tick or per-field receipt.
    pub revision: u64,
    /// Own player.
    pub player: BodyPlayer,
    /// Requested cells, with unavailability and source preserved.
    pub blocks: Vec<BodyBlock>,
}
/// Initial action families; more can be added without changing existing Client APIs.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Action {
    /// Explicit native yaw/pitch, without mouse emulation.
    Look {
        /// Yaw in degrees.
        yaw: f32,
        /// Pitch in degrees (-90..90).
        pitch: f32,
    },
    /// Hold inputs for at least N fully dispatched client ticks, then release.
    /// None holds until release_control/cancel or the configured lease deadline.
    /// Both versions use best-effort ground control: exact floor support and
    /// zero velocity are not admission requirements. Unknown terrain uses small
    /// bounded probes, with warnings through PlayerControl::walking_diagnostics_for.
    /// A received pose rollback returns Corrected; evidence exhaustion reaches
    /// bounded Unknown.
    Move {
        /// Digital inputs and explicit rotation.
        controls: Controls,
        /// Tick count, or a deadline-bounded lease.
        ticks: Option<u32>,
    },
    /// Use any held block item at an exact hit, without client reach/raycast filtering.
    PlaceBlock {
        /// Clicked block (not the resulting cell).
        pos: [i32; 3],
        /// Clicked face.
        face: BlockFace,
        /// Exact block-relative hit coordinates, each 0..1.
        cursor: [f32; 3],
        /// Hand holding the item.
        hand: Hand,
        /// Result cell whose server updates are monitored.
        target: [i32; 3],
        /// Expected result; None uses the held item's block name.
        expected: Option<NativeBlockState>,
    },
    /// Send an explicit block activation (tables, furnaces, storage, etc.).
    /// No first-outline, standing, empty-hand or default-item proof is required.
    /// Accepted means processing/quiet-window acceptance, not that a menu opened.
    ActivateBlock {
        /// Exact clicked cell.
        pos: [i32; 3],
        /// Exact clicked face, independent of the view ray.
        face: BlockFace,
        /// Block-relative hit, each coordinate 0..1.
        cursor: [f32; 3],
        /// Explicit hand; the held item may affect the server's result.
        hand: Hand,
    },
    /// Send START without requiring a dig estimate. Survival sends STOP once
    /// START processing is acknowledged, then awaits a received air result.
    /// Timeout sends ABORT and ends Unknown; server rollback ends Corrected.
    /// Strict `Survival::dig` / `begin_dig` keep their timing/receipt contract.
    BreakBlock {
        /// Block to break.
        pos: [i32; 3],
        /// Explicit hit face.
        face: BlockFace,
    },
    /// Use the item held in the given hand.
    UseItem {
        /// Selected hand.
        hand: Hand,
    },
    /// Send an unsigned chat line (headless offline connections).
    Chat(String),
    /// Send a command without the leading slash; acceptance does not imply command success.
    Command(String),
    /// Select index 0..8.
    SelectHotbar(u8),
}
/// Action with an optional world fence; submit binds an omitted fence to the current world.
#[derive(Clone, Debug)]
pub struct ActionRequest {
    /// Desired action.
    pub action: Action,
    /// Expected lifetime, obtained from observe.
    pub world: Option<BodyWorld>,
}
impl From<Action> for ActionRequest {
    fn from(action: Action) -> Self {
        Self {
            action,
            world: None,
        }
    }
}
/// Backend-scoped action identity, never a wire sequence or replay token.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct ActionId {
    /// Backend instance.
    pub backend: u64,
    /// Local monotonically increasing action number.
    pub number: u64,
}
/// Original server value which superseded an action, separate from the current snapshot.
#[derive(Clone, Debug, serde::Serialize)]
pub enum ServerCorrection {
    /// Exact block update, including unchanged state rejecting a prediction.
    Block(BodyBlock),
    /// Resolved received teleport/correction pose. A rotation-only correction
    /// (Java 1.21.11 `PLAYER_ROTATION`) reports the unchanged position here
    /// with its own source, which is then not that packet's receipt.
    Pose {
        /// Received feet position, or the unchanged position for rotation-only corrections.
        position: Value<[f64; 3]>,
        /// Received yaw/pitch.
        rotation: Value<[f32; 2]>,
    },
    /// Server-selected slot.
    SelectedHotbar(Value<u8>),
    /// Full inventory resynchronization; contents are in the accompanying snapshot.
    InventoryResync,
}
/// Acceptance outcome. Ordinary submit results are immutable; send-and-trust
/// IDs can additionally expose a later Corrected rollback notice.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ActionStatus {
    /// Admitted locally; sending or evidence collection is still in progress.
    Pending,
    /// Complete send plus the documented bounded evidence policy, not perpetual authority.
    Accepted,
    /// The server superseded the action. Contains the observed rollback result.
    Corrected {
        /// Explanation of the server update.
        reason: String,
        /// Original correcting value (may predate the current snapshot).
        correction: ServerCorrection,
        /// Current coherent state after observing the correction.
        snapshot: Box<BodySnapshot>,
    },
    /// Explicit local/server refusal, with no implied successful effect.
    Rejected {
        /// Reason for refusal.
        reason: String,
    },
    /// Missing evidence, history gap, world loss or uncertain write at the deadline.
    Unknown {
        /// Why acceptance could not be established.
        reason: String,
    },
    /// Cancellation requested; releases held control but cannot undo server effects.
    Cancelled,
}
impl ActionStatus {
    /// Whether acceptance work has reached a result. A late rollback notice
    /// on a send-and-trust ID does not reopen work or ownership.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}
/// Result of waiting only for the first complete dispatch boundary.
/// This is separate from `ActionStatus`: Sent never asserts server acceptance.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SendStatus {
    /// All packets at the action's initial dispatch boundary were fully written.
    /// Move: first configured client tick; BreakBlock: START; others: full action.
    Sent,
    /// The action terminated before a complete dispatch was established.
    NotSent {
        /// Refusal, uncertainty, cancellation or deadline result; never success.
        outcome: ActionStatus,
    },
}
/// An owned action and its initial dispatch result. Continue observing this ID
/// through status/events for acceptance or later server corrections.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActionDispatch {
    /// Retained action identity, also available for NotSent results.
    pub id: ActionId,
    /// Dispatch result, independent of the eventual acceptance outcome.
    pub dispatch: SendStatus,
}
/// Supported action families.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum ActionKind {
    /// Explicit view.
    Look,
    /// Held ground movement.
    Move,
    /// Exact block hit, including explicit activation.
    PlaceBlock,
    /// Creative and survival breaking.
    BreakBlock,
    /// Item use.
    UseItem,
    /// Chat and commands.
    Chat,
    /// Slot selection.
    SelectHotbar,
}
/// Runtime-independent advertised support; readiness is established at submission.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BackendCapabilities {
    /// Implementation name.
    pub backend: String,
    /// Exact game version.
    pub version: MinecraftVersion,
    /// Implemented action families.
    pub actions: Vec<ActionKind>,
    /// Player, inventory and loaded block observation available.
    pub observation: bool,
    /// Ground movement uses the headless model and its geometry coverage.
    pub ground_movement_only: bool,
}
/// Bounded acceptance and resource policy.
#[derive(Clone, Debug)]
pub struct BodyConfig {
    /// Quiet interval after complete send and required block evidence. Must be positive.
    pub settle_window: Duration,
    /// Absolute deadline from submit, including sending/digging and settling.
    /// Also the maximum duration of an until-released input lease.
    pub action_timeout: Duration,
    /// Maximum retained actions and events; old IDs/cursors explicitly expire.
    pub retention: usize,
}
impl Default for BodyConfig {
    fn default() -> Self {
        Self {
            settle_window: Duration::from_millis(300),
            action_timeout: Duration::from_secs(10),
            retention: 256,
        }
    }
}
/// Result of cancellation; server effects are never rolled back locally.
#[derive(Clone, Debug, serde::Serialize)]
pub enum CancelStatus {
    /// Cancellation/release requested.
    Requested,
    /// Result was already terminal (a held lease is still released).
    AlreadyTerminal(ActionStatus),
}
/// Input release result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum ReleaseStatus {
    /// No active body action/control remains.
    Released,
    /// Cleanup deadline expired; do not assume inputs are neutral.
    Unknown,
}
/// Backend-scoped cursor for retained status transitions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct EventCursor {
    /// Instance, zero for initial read.
    pub backend: u64,
    /// Event ordinal, zero for initial read.
    pub ordinal: u64,
}
/// One local status transition, not a Minecraft packet receipt.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BodyEvent {
    /// Cursor after this event.
    pub cursor: EventCursor,
    /// Action identity.
    pub action: ActionId,
    /// New status.
    pub status: ActionStatus,
}
/// Bounded transition log. A gap requires a fresh observation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BodyEvents {
    /// Next cursor; advances only through returned events.
    pub cursor: EventCursor,
    /// Caller fell behind retention.
    pub gap: bool,
    /// Retained transitions in order.
    pub events: Vec<BodyEvent>,
}
/// Small common boundary. Implementations own admitted work independently of waiter lifetimes.
pub trait BodyBackend: Send + Sync {
    /// Advertised operation and observation support.
    fn capabilities(&self) -> BackendCapabilities;
    /// Capture available state, preserving sources and missing fields.
    fn observe(&self, request: ObserveRequest)
    -> impl Future<Output = Result<BodySnapshot>> + Send;
    /// Admit once and return an ID; do not silently replay a request.
    fn submit(&self, request: ActionRequest) -> impl Future<Output = Result<ActionId>> + Send;
    /// Read retained bounded outcome, including after disconnect.
    fn action_status(&self, id: ActionId) -> impl Future<Output = Result<ActionStatus>> + Send;
    /// Refresh acceptance and late-monitor completeness. Unsupported by default.
    fn action_observation(
        &self,
        _id: ActionId,
    ) -> impl Future<Output = Result<ActionObservation>> + Send {
        async {
            Err(crate::Error::new(
                crate::ErrorKind::Unsupported,
                anyhow::anyhow!("body monitoring is unsupported"),
            ))
        }
    }
    /// Read the bounded monitor log. Unsupported by default.
    fn monitor_changes_after(
        &self,
        _cursor: MonitorCursor,
        _maximum: usize,
    ) -> impl Future<Output = Result<MonitorChanges>> + Send {
        async {
            Err(crate::Error::new(
                crate::ErrorKind::Unsupported,
                anyhow::anyhow!("body monitoring is unsupported"),
            ))
        }
    }
    /// Cancel work/release inputs; never undo effects already sent.
    fn cancel(&self, id: ActionId) -> impl Future<Output = Result<CancelStatus>> + Send;
    /// Read local transitions, with explicit retention gaps.
    fn events_after(
        &self,
        cursor: EventCursor,
        maximum: usize,
    ) -> impl Future<Output = Result<BodyEvents>> + Send;
    /// Release owned held input without disconnecting the underlying game client.
    fn release_control(&self) -> impl Future<Output = Result<ReleaseStatus>> + Send;
}
/// Backend-independent facade. Existing clients opt in explicitly.
pub struct Body<B: BodyBackend> {
    backend: B,
}
impl<B: BodyBackend> Body<B> {
    /// Wrap a backend.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
    /// Access backend-specific extensions.
    pub fn backend(&self) -> &B {
        &self.backend
    }
    /// Advertised support.
    pub fn capabilities(&self) -> BackendCapabilities {
        self.backend.capabilities()
    }
    /// Capture a snapshot.
    pub async fn observe(&self, request: ObserveRequest) -> Result<BodySnapshot> {
        self.backend.observe(request).await
    }
    /// Submit an action exactly once.
    pub async fn submit(&self, request: impl Into<ActionRequest>) -> Result<ActionId> {
        self.backend.submit(request.into()).await
    }
    /// Submit an adjacent cube placement with an explicit hit. No default-only
    /// material, standing or ray diagnostics are prerequisites. Use the existing
    /// `Survival::placement_check` or strict `Survival::place_cube` separately when
    /// those diagnostics are wanted. The server chooses the actual result.
    pub async fn place_cube(
        &self,
        pos: [i32; 3],
        face: BlockFace,
        cursor: [f32; 3],
        hand: Hand,
    ) -> Result<ActionId> {
        let target = crate::client::survival::placement::adjacent(pos, face)?;
        self.submit(Action::PlaceBlock {
            pos,
            face,
            cursor,
            hand,
            target,
            expected: None,
        })
        .await
    }
    /// Read a retained outcome.
    pub async fn action_status(&self, id: ActionId) -> Result<ActionStatus> {
        self.backend.action_status(id).await
    }
    /// Refresh the original result, late rollback and monitoring completeness.
    pub async fn action_observation(&self, id: ActionId) -> Result<ActionObservation> {
        self.backend.action_observation(id).await
    }
    /// Refresh monitors and read their separate bounded transition log.
    pub async fn monitor_changes_after(
        &self,
        cursor: MonitorCursor,
        maximum: usize,
    ) -> Result<MonitorChanges> {
        self.backend.monitor_changes_after(cursor, maximum).await
    }
    /// Request cancellation.
    pub async fn cancel(&self, id: ActionId) -> Result<CancelStatus> {
        self.backend.cancel(id).await
    }
    /// Read transitions.
    pub async fn events_after(&self, cursor: EventCursor, maximum: usize) -> Result<BodyEvents> {
        self.backend.events_after(cursor, maximum).await
    }
    /// Release held control.
    pub async fn release_control(&self) -> Result<ReleaseStatus> {
        self.backend.release_control().await
    }
}

mod receipt;
pub use receipt::{InventoryClickWatch, ReceiptStatus};

impl Body<HeadlessBody> {
    /// Submit once and return at full initial dispatch, without ACK/settle waits.
    /// The backend keeps ownership, deadlines and cleanup. For this opt-in ID,
    /// status/events also expose late Corrected notices within retained evidence.
    pub async fn send_and_trust(
        &self,
        request: impl Into<ActionRequest>,
    ) -> Result<ActionDispatch> {
        self.backend.send_and_trust(request).await
    }
}
