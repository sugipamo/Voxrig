use super::evidence::{EvidenceBatch, Update};
use super::state::Acceptance;
use super::*;
use crate::connection::Adapter;
use crate::versions::java_1_16_1::Bot as LegacyBot;
use crate::versions::java_1_21_11::operations::Operations as ModernOperations;

#[derive(Clone)]
enum Operations {
    Modern(ModernOperations),
    Legacy(Box<LegacyBot>),
}
impl Operations {
    async fn body_evidence(
        &self,
        backend: u64,
        request: &ObserveRequest,
        cursor: Option<u64>,
    ) -> Result<EvidenceBatch> {
        match self {
            Self::Modern(ops) => ops.body_evidence(backend, request, cursor).await,
            Self::Legacy(bot) => bot.body_evidence(backend, request, cursor).await,
        }
    }
    async fn body_block_send(
        &self,
        world: BodyWorld,
        action: &Action,
        status: i32,
        initial: Option<&BodySnapshot>,
    ) -> Result<Option<i32>> {
        match self {
            Self::Modern(ops) => ops.body_block_send(world, action, status).await.map(Some),
            Self::Legacy(bot) => bot
                .body_break_send(world, action, status, initial)
                .await
                .map(|_| None),
        }
    }
    async fn body_place_send(
        &self,
        world: BodyWorld,
        action: &Action,
    ) -> Result<(Option<i32>, Option<u64>)> {
        match self {
            Self::Modern(ops) => ops
                .body_block_send(world, action, 0)
                .await
                .map(|s| (Some(s), None)),
            Self::Legacy(bot) => bot
                .body_place_send(world, action)
                .await
                .map(|c| (None, Some(c))),
        }
    }
    async fn body_simple_send(
        &self,
        world: BodyWorld,
        mode: GameMode,
        action: &Action,
        expected: Option<(u8, crate::client::registry::RegistryId)>,
    ) -> Result<()> {
        match self {
            Self::Modern(ops) => ops.body_simple_send(world, mode, action, expected).await,
            Self::Legacy(bot) => bot.body_simple_send(world, mode, action, expected).await,
        }
    }
    async fn body_start_control(
        &self,
        mode: GameMode,
        world: BodyWorld,
    ) -> Result<crate::client::control::ControlRecord> {
        match self {
            Self::Modern(ops) => ops.body_start_control(mode, world).await,
            Self::Legacy(bot) => bot.body_start_control(mode, world).await,
        }
    }
}
use crate::{Client, Error, ErrorKind};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::{
    sync::watch,
    time::{Instant, sleep, timeout, timeout_at},
};

static NEXT_BACKEND: AtomicU64 = AtomicU64::new(1);
fn error(kind: ErrorKind, message: impl Into<String>) -> Error {
    Error::new(kind, anyhow::anyhow!(message.into()))
}
fn invalid(message: &str) -> Error {
    error(ErrorKind::InvalidInput, message)
}
struct Job {
    acceptance: Mutex<Acceptance>,
    cancel: watch::Sender<bool>,
    release: Mutex<Option<ReleaseStatus>>,
    use_item_refusal: Mutex<Option<crate::client::UseItemRefusal>>,
    monitor: Mutex<Option<Monitor>>,
    late_correction: Mutex<Option<ActionStatus>>,
}
#[derive(Clone)]
struct Monitor {
    selection: ObserveRequest,
    world: BodyWorld,
    cursor: u64,
    action: Action,
    expected: Option<([i32; 3], Expected)>,
}
struct Records {
    next: u64,
    jobs: BTreeMap<ActionId, Arc<Job>>,
    active: Option<ActionId>,
    events: VecDeque<BodyEvent>,
    ordinal: u64,
    dropped: u64,
    release_failed: bool,
}
struct Inner {
    client: Client,
    id: u64,
    config: BodyConfig,
    records: Mutex<Records>,
    changed: watch::Sender<u64>,
}
/// Acceptance adapter around an existing headless Client. Clones share action/control ownership.
///
/// Use one wrapper per Client, and avoid concurrent mutations through other Client clones.
/// There is one active body action/lease at a time; overlapping submissions are Rejected.
/// Explicit placement/activation may accompany an existing PlayerControl session;
/// that caller retains movement ownership and must stop it separately. Finite path and
/// same-domain receipt gates still apply. Inventory/cursor and screen receipts
/// do not veto motion, look, chat, selection or dig. Strict Client APIs retain
/// their original admission and receipt semantics.
#[derive(Clone)]
pub struct HeadlessBody {
    inner: Arc<Inner>,
}
impl HeadlessBody {
    /// Submit atomic item use on either native version, returning a normal Body
    /// action ID. Checks selected index 0..8 and received held item ID inside the
    /// dispatch lock; count/data are ignored. Refusal sends nothing, produces
    /// Rejected, and is retained by `Self::use_item_refusal`. Accepted retains the
    /// usual Body meaning and does not prove consumption. Binds the current world.
    pub async fn use_item_expect(
        &self,
        hand: Hand,
        selected_hotbar: u8,
        item: crate::client::registry::RegistryId,
    ) -> Result<ActionId> {
        self.submit_inner(
            Action::UseItem { hand }.into(),
            Some((selected_hotbar, item)),
        )
        .await
    }

    /// Refresh an Accepted/Unknown outcome from retained original server evidence.
    /// This additive view can report a late Corrected rollback notice. Existing
    /// BodyBackend::action_status and its immutable terminal result are unchanged.
    /// Does not replay, reopen ownership, or infer acceptance from late agreement.
    /// Evidence gaps/world changes leave Unknown; retention bounds this facility.
    pub async fn action_outcome(&self, id: ActionId) -> Result<ActionStatus> {
        let status = self.action_status(id).await?;
        if !matches!(
            status,
            ActionStatus::Unknown { .. } | ActionStatus::Accepted
        ) {
            return Ok(status);
        }
        let job = self.inner.job(id)?;
        if let Some(corrected) = job.late_correction.lock().unwrap().clone() {
            return Ok(corrected);
        }
        let monitor = job.monitor.lock().unwrap().clone();
        if let Some(monitor) = monitor {
            let batch = timeout(
                self.inner.config.settle_window,
                self.operations()?.body_evidence(
                    self.inner.id,
                    &monitor.selection,
                    Some(monitor.cursor),
                ),
            )
            .await;
            if let Ok(Ok(batch)) = batch {
                if !batch.gap && batch.snapshot.world == monitor.world {
                    if let Some(corrected @ ActionStatus::Corrected { .. }) =
                        correction(&batch, monitor.expected.as_ref(), &monitor.action)?
                    {
                        *job.late_correction.lock().unwrap() = Some(corrected.clone());
                        return Ok(corrected);
                    }
                }
            }
        }
        Ok(status)
    }

    /// Watch one existing strict PICKUP with a shared absolute deadline. Does
    /// not change its strict record, clear cursor/screen uncertainty, or send.
    /// Use body actions for unrelated work while the receipt is pending.
    pub async fn watch_inventory_click(
        &self,
        id: crate::client::inventory::InventoryClickId,
    ) -> Result<InventoryClickWatch> {
        InventoryClickWatch::new(
            self.inner.client.clone(),
            id,
            self.inner.config.action_timeout,
            self.inner.config.settle_window,
        )
        .await
    }

    /// Retained typed identity refusal for a rejected `HeadlessBody::use_item_expect`.
    /// None until an identity refusal is recorded. Uses the same
    /// retention and foreign/expired-ID checks as action status.
    pub fn use_item_refusal(&self, id: ActionId) -> Result<Option<crate::client::UseItemRefusal>> {
        Ok(self.inner.job(id)?.use_item_refusal.lock().unwrap().clone())
    }

    /// Wrap without reconnecting or changing the existing strict API.
    /// Both native versions support all body actions, including atomic item use.
    /// Break retains the existing survival timing estimate prerequisites.
    pub fn new(client: Client, config: BodyConfig) -> Result<Self> {
        if config.settle_window.is_zero()
            || config.action_timeout <= config.settle_window
            || config.action_timeout > Duration::from_secs(3600)
            || !(2..=4096).contains(&config.retention)
        {
            return Err(invalid(
                "body needs 0 < settle_window < action_timeout <= 1 hour and retention 2..4096",
            ));
        }
        Ok(Self {
            inner: Arc::new(Inner {
                client,
                id: NEXT_BACKEND.fetch_add(1, Ordering::Relaxed),
                config,
                records: Mutex::new(Records {
                    next: 0,
                    jobs: BTreeMap::new(),
                    active: None,
                    events: VecDeque::new(),
                    ordinal: 0,
                    dropped: 0,
                    release_failed: false,
                }),
                changed: watch::channel(0).0,
            }),
        })
    }
    /// Optional cube diagnostics. Failures of the old standing/ray/default-stack
    /// model are returned as warnings and never authorize or veto a send.
    /// Call separately from submission; observations may change in between.
    pub async fn cube_placement_diagnostics(
        &self,
        pos: [i32; 3],
        face: BlockFace,
    ) -> Result<Vec<String>> {
        let player = self.inner.client.player_state().await?;
        let mut warnings = Vec::new();
        if let Err(error) = crate::client::survival::placement::selected_material(&player) {
            warnings.push(error.to_string());
        }
        let query = match player.game_mode {
            Some(GameMode::Creative) => self.inner.client.creative().target_block(4.5).await,
            _ => self.inner.client.survival().target_block(4.5).await,
        };
        match query {
            Ok(query)
                if query
                    .hit
                    .as_ref()
                    .is_some_and(|hit| hit.position == pos && hit.face == face) => {}
            Ok(_) => warnings
                .push("explicit support/face differs from the first outline diagnostic".into()),
            Err(error) => warnings.push(error.to_string()),
        }
        Ok(warnings)
    }

    /// Existing diagnostic API, retaining its original receipt semantics.
    pub fn client(&self) -> &Client {
        &self.inner.client
    }
    fn operations(&self) -> Result<Operations> {
        match &self.inner.client.adapter {
            Adapter::Java1_21_11(bot) => Ok(Operations::Modern(bot.operations())),
            Adapter::Java1_16_1(bot) => Ok(Operations::Legacy(bot.clone())),
        }
    }
}
impl Inner {
    fn event(&self, records: &mut Records, id: ActionId, status: ActionStatus) {
        records.ordinal += 1;
        records.events.push_back(BodyEvent {
            cursor: EventCursor {
                backend: self.id,
                ordinal: records.ordinal,
            },
            action: id,
            status,
        });
        if records.events.len() > self.config.retention {
            records.dropped = records.events.pop_front().unwrap().cursor.ordinal;
        }
    }
    fn update(&self, id: ActionId, update: impl FnOnce(&mut Acceptance)) {
        let mut records = self.records.lock().unwrap();
        if let Some(job) = records.jobs.get(&id).cloned() {
            let mut machine = job.acceptance.lock().unwrap();
            let pending = !machine.status.is_terminal();
            update(&mut machine);
            if pending && machine.status.is_terminal() {
                self.event(&mut records, id, machine.status.clone());
            }
        }
    }
    fn finish(&self, id: ActionId, status: ActionStatus) {
        self.update(id, |m| m.finish(status));
    }
    fn job(&self, id: ActionId) -> Result<Arc<Job>> {
        self.records
            .lock()
            .unwrap()
            .jobs
            .get(&id)
            .cloned()
            .ok_or_else(|| invalid("unknown, expired or foreign body action ID"))
    }
}
impl HeadlessBody {
    async fn submit_inner(
        &self,
        request: ActionRequest,
        expected_use: Option<(u8, crate::client::registry::RegistryId)>,
    ) -> Result<ActionId> {
        let ops = self.operations()?;
        let deadline = Instant::now() + self.inner.config.action_timeout;
        let needs_ack = matches!(
            request.action,
            Action::PlaceBlock { .. } | Action::BreakBlock { .. }
        ) || (matches!(request.action, Action::ActivateBlock { .. })
            && self.inner.client.version() == MinecraftVersion::Java1_21_11);
        let job = Arc::new(Job {
            acceptance: Mutex::new(Acceptance::new(
                deadline,
                self.inner.config.settle_window,
                needs_ack,
            )),
            cancel: watch::channel(false).0,
            release: Mutex::new(None),
            use_item_refusal: Mutex::new(None),
            monitor: Mutex::new(None),
            late_correction: Mutex::new(None),
        });
        let id;
        {
            let mut records = self.inner.records.lock().unwrap();
            records.next = records
                .next
                .checked_add(1)
                .ok_or_else(|| invalid("body action IDs exhausted"))?;
            id = ActionId {
                backend: self.inner.id,
                number: records.next,
            };
            while records.jobs.len() >= self.inner.config.retention {
                let oldest = records
                    .jobs
                    .keys()
                    .copied()
                    .find(|id| Some(*id) != records.active)
                    .ok_or_else(|| invalid("body retention exhausted"))?;
                records.jobs.remove(&oldest);
            }
            records.jobs.insert(id, job.clone());
            self.inner.event(&mut records, id, ActionStatus::Pending);
            if records.active.is_some() || records.release_failed {
                let mut machine = job.acceptance.lock().unwrap();
                machine.finish(ActionStatus::Rejected {
                    reason: "another body action owns control or previous release is unresolved"
                        .into(),
                });
                self.inner.event(&mut records, id, machine.status.clone());
                return Ok(id);
            }
            records.active = Some(id);
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut cancel = job.cancel.subscribe();
            let mut cleanup = Cleanup::default();
            let result = tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => { inner.finish(id, ActionStatus::Cancelled); Ok(()) },
                result = timeout_at(deadline, run(&inner, &ops, id, &request, &mut cleanup, expected_use)) => {
                    match result { Ok(result) => result, Err(_) => {
                        inner.finish(id, ActionStatus::Unknown { reason: "absolute action/lease deadline expired".into() }); Ok(())
                    } }
                }
            };
            if let Err(e) = result {
                *job.use_item_refusal.lock().unwrap() = e.use_item_refusal().cloned();
                // Once any send may have started, failure cannot establish rejection.
                inner.finish(
                    id,
                    if cleanup.attempted
                        && (cleanup.dispatched
                            || !matches!(
                                e.kind(),
                                ErrorKind::InvalidInput
                                    | ErrorKind::Unsupported
                                    | ErrorKind::State
                                    | ErrorKind::Rejected
                            ))
                    {
                        ActionStatus::Unknown {
                            reason: e.to_string(),
                        }
                    } else {
                        ActionStatus::Rejected {
                            reason: e.to_string(),
                        }
                    },
                );
            }
            let cleaned = timeout(
                inner.config.settle_window,
                cleanup.release(&inner.client, &ops),
            )
            .await;
            let released = matches!(cleaned, Ok(Ok(())));
            if !released {
                // The existing disconnect path stops the connection's movement task too.
                // This is a fallback only for uncertain owned cleanup, never a replay.
                let _ = timeout(inner.config.settle_window, inner.client.disconnect()).await;
            }
            *job.release.lock().unwrap() = Some(if released {
                ReleaseStatus::Released
            } else {
                ReleaseStatus::Unknown
            });
            let mut records = inner.records.lock().unwrap();
            records.release_failed |= !released;
            if records.active == Some(id) {
                records.active = None;
            }
            inner.changed.send_modify(|n| *n += 1);
        });
        Ok(id)
    }
}
impl BodyBackend for HeadlessBody {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            backend: "headless".into(),
            version: self.inner.client.version(),
            actions: vec![
                ActionKind::Look,
                ActionKind::Move,
                ActionKind::PlaceBlock,
                ActionKind::BreakBlock,
                ActionKind::UseItem,
                ActionKind::Chat,
                ActionKind::SelectHotbar,
            ],
            observation: true,
            ground_movement_only: true,
        }
    }
    async fn observe(&self, request: ObserveRequest) -> Result<BodySnapshot> {
        validate_observe(&request)?;
        let ops = self.operations()?;
        timeout(
            self.inner.config.action_timeout,
            ops.body_evidence(self.inner.id, &request, None),
        )
        .await
        .map_err(|_| error(ErrorKind::Timeout, "body observation deadline expired"))?
        .map(|b| b.snapshot)
    }
    async fn submit(&self, request: ActionRequest) -> Result<ActionId> {
        self.submit_inner(request, None).await
    }
    async fn action_status(&self, id: ActionId) -> Result<ActionStatus> {
        let job = self.inner.job(id)?;
        // Also enforces the deadline if the worker/runtime was delayed.
        self.inner.update(id, |m| m.expire(Instant::now()));
        let status = job.acceptance.lock().unwrap().status.clone();
        Ok(status)
    }
    async fn cancel(&self, id: ActionId) -> Result<CancelStatus> {
        let job = self.inner.job(id)?;
        self.inner.update(id, |m| m.expire(Instant::now()));
        let status = job.acceptance.lock().unwrap().status.clone();
        job.cancel.send_replace(true);
        if status.is_terminal() {
            Ok(CancelStatus::AlreadyTerminal(status))
        } else {
            self.inner.finish(id, ActionStatus::Cancelled);
            Ok(CancelStatus::Requested)
        }
    }
    async fn events_after(&self, cursor: EventCursor, maximum: usize) -> Result<BodyEvents> {
        if maximum == 0
            || maximum > 4096
            || (cursor.backend != 0 && cursor.backend != self.inner.id)
            || (cursor.backend == 0 && cursor.ordinal != 0)
        {
            return Err(invalid("invalid body event cursor or maximum (1..4096)"));
        }
        let records = self.inner.records.lock().unwrap();
        if cursor.ordinal > records.ordinal {
            return Err(invalid("body event cursor is ahead of this backend"));
        }
        let events: Vec<_> = records
            .events
            .iter()
            .filter(|e| e.cursor.ordinal > cursor.ordinal)
            .take(maximum)
            .cloned()
            .collect();
        Ok(BodyEvents {
            cursor: events.last().map_or(
                EventCursor {
                    backend: self.inner.id,
                    ordinal: cursor.ordinal,
                },
                |e| e.cursor,
            ),
            gap: cursor.ordinal < records.dropped,
            events,
        })
    }
    async fn release_control(&self) -> Result<ReleaseStatus> {
        let mut changed = self.inner.changed.subscribe();
        let active = self.inner.records.lock().unwrap().active;
        if let Some(id) = active {
            let job = self.inner.job(id)?;
            self.cancel(id).await?;
            let wait = async {
                loop {
                    if self.inner.records.lock().unwrap().active != Some(id) {
                        break;
                    }
                    if changed.changed().await.is_err() {
                        break;
                    }
                }
            };
            if timeout(self.inner.config.settle_window * 3, wait)
                .await
                .is_err()
            {
                return Ok(ReleaseStatus::Unknown);
            }
            return Ok(job
                .release
                .lock()
                .unwrap()
                .unwrap_or(ReleaseStatus::Unknown));
        }
        Ok(if self.inner.records.lock().unwrap().release_failed {
            ReleaseStatus::Unknown
        } else {
            ReleaseStatus::Released
        })
    }
}
async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}
fn validate_observe(request: &ObserveRequest) -> Result<()> {
    if request.blocks.len() > 262_144 {
        return Err(invalid("too many requested blocks"));
    }
    for &pos in &request.blocks {
        Region { min: pos, max: pos }.volume()?;
    }
    if request.blocks.len() + request.region.map_or(Ok(0), Region::volume)? > 262_144 {
        return Err(invalid(
            "combined block and region request exceeds 262144 cells",
        ));
    }
    Ok(())
}
#[derive(Default)]
struct Cleanup {
    attempted: bool,
    dispatched: bool,
    control: Option<(GameMode, u64)>,
    dig: Option<(BodyWorld, Action)>,
    dig_owner: Option<tokio::sync::OwnedMutexGuard<()>>,
}
impl Cleanup {
    async fn release(&mut self, client: &Client, ops: &Operations) -> Result<()> {
        if let Some((mode, id)) = self.control.take() {
            client.player_control(mode).stop_control_for(id).await?;
        }
        if let Some((world, action)) = self.dig.take() {
            ops.body_block_send(world, &action, 1, None).await?;
        }
        self.dig_owner.take();
        Ok(())
    }
}
fn observe_action(action: &Action) -> ObserveRequest {
    ObserveRequest {
        blocks: match action {
            Action::PlaceBlock { pos, target, .. } => vec![*pos, *target],
            Action::ActivateBlock { pos, .. } => vec![*pos],
            Action::BreakBlock { pos, .. } => vec![*pos],
            _ => vec![],
        },
        region: None,
    }
}
#[derive(Clone)]
enum Expected {
    Exact(NativeBlockState),
    Name(String),
}
impl Expected {
    fn matches(&self, state: &NativeBlockState) -> bool {
        match self {
            Self::Exact(s) => s == state,
            Self::Name(name) => name == &state.name,
        }
    }
}
fn expected(action: &Action, snapshot: &BodySnapshot) -> Result<Option<([i32; 3], Expected)>> {
    Ok(match action {
        Action::BreakBlock { pos, .. } => Some((*pos, Expected::Name("minecraft:air".into()))),
        Action::PlaceBlock {
            target,
            expected,
            hand,
            ..
        } => {
            let expectation = if let Some(state) = expected {
                Expected::Exact(state.clone())
            } else {
                let slot = match hand { Hand::Off => 45, Hand::Main => 36 + usize::from(snapshot.player.selected_hotbar.as_ref().ok_or_else(|| invalid("selected slot unavailable; select a hotbar slot or provide expected state"))?.value) };
                let name = match snapshot
                    .player
                    .inventory
                    .get(slot)
                    .and_then(Option::as_ref)
                    .map(|s| &s.value)
                {
                    Some(BodySlot::Item(item)) => item.name.clone(),
                    _ => {
                        return Err(invalid(
                            "held item unavailable; provide an explicit expected block state",
                        ));
                    }
                };
                Expected::Name(name)
            };
            Some((*target, expectation))
        }
        _ => None,
    })
}
async fn run(
    inner: &Inner,
    ops: &Operations,
    id: ActionId,
    request: &ActionRequest,
    cleanup: &mut Cleanup,
    expected_use: Option<(u8, crate::client::registry::RegistryId)>,
) -> Result<()> {
    let selection = observe_action(&request.action);
    validate_observe(&selection)?;
    if let Action::Move { controls, ticks } = &request.action {
        if *ticks == Some(0)
            || !(-1..=1).contains(&controls.forward)
            || !(-1..=1).contains(&controls.strafe)
        {
            return Err(invalid("invalid movement inputs/tick count"));
        }
    }
    if let Action::PlaceBlock { cursor, .. } | Action::ActivateBlock { cursor, .. } =
        &request.action
    {
        crate::client::item_use::validate_cursor(*cursor)?;
    }
    let baseline = ops.body_evidence(inner.id, &selection, None).await?;
    let world = request.world.unwrap_or(baseline.snapshot.world);
    if baseline.snapshot.world != world {
        return Err(invalid("stale body world"));
    }
    if let Action::PlaceBlock {
        expected: Some(state),
        ..
    } = &request.action
    {
        (inner.client.version().table().registry.state_id)(state)?;
    }
    let expected = expected(&request.action, &baseline.snapshot)?;
    let mode = || {
        baseline
            .snapshot
            .player
            .game_mode
            .ok_or_else(|| invalid("game mode unavailable"))
    };
    *inner.job(id)?.monitor.lock().unwrap() = Some(Monitor {
        selection: selection.clone(),
        world,
        cursor: baseline.cursor,
        action: request.action.clone(),
        expected: expected.clone(),
    });
    let mut cursor = baseline.cursor;
    let mut sequence = None;
    match &request.action {
        Action::Look { .. }
        | Action::SelectHotbar(_)
        | Action::Chat(_)
        | Action::Command(_)
        | Action::UseItem { .. } => {
            cleanup.attempted = true;
            ops.body_simple_send(world, mode()?, &request.action, expected_use)
                .await?;
            cleanup.dispatched = true;
        }
        Action::PlaceBlock { .. } | Action::ActivateBlock { .. } => {
            cleanup.attempted = true;
            let (sent_sequence, sent_cursor) = ops.body_place_send(world, &request.action).await?;
            sequence = sent_sequence;
            if let Some(sent_cursor) = sent_cursor {
                cursor = sent_cursor;
            }
            cleanup.dispatched = true;
        }
        Action::BreakBlock { pos, .. } => {
            use crate::client::adapter::DigOps;
            let coordinator =
                crate::client::dispatch!(&inner.client.adapter, a => DigOps::dig_coordinator(a));
            cleanup.dig_owner = Some(coordinator.reserve_body(crate::client::SessionStamp {
                version: inner.client.version(),
                connection_id: world.connection,
                world_generation: world.generation,
            })?);
            let ticks = match mode()? {
                GameMode::Creative => 0,
                GameMode::Survival => inner.client.survival().dig_estimate(*pos).await?.ticks,
                _ => return Err(invalid("break requires survival or creative mode")),
            };
            cleanup.attempted = true;
            // Register cleanup before awaiting a potentially interrupted START.
            if ticks > 0 {
                cleanup.dig = Some((world, request.action.clone()));
            }
            sequence = ops
                .body_block_send(world, &request.action, 0, Some(&baseline.snapshot))
                .await?;
            cleanup.dispatched = true;
            if ticks > 0 {
                sleep(
                    Duration::from_millis(50).saturating_mul(ticks.min(u64::from(u32::MAX)) as u32),
                )
                .await;
                let current = ops
                    .body_evidence(inner.id, &selection, Some(cursor))
                    .await?;
                if current.snapshot.world != world {
                    return Err(invalid("world changed during dig"));
                }
                if current.snapshot.version == MinecraftVersion::Java1_16_1 {
                    if current.gap {
                        inner.finish(
                            id,
                            ActionStatus::Unknown {
                                reason: "dig evidence history gap".into(),
                            },
                        );
                        return Ok(());
                    }
                    if let Some(status) = correction(&current, expected.as_ref(), &request.action)?
                    {
                        inner.finish(id, status);
                        return Ok(());
                    }
                }
                let before = baseline
                    .snapshot
                    .blocks
                    .iter()
                    .find(|b| b.position == *pos)
                    .and_then(|b| b.state.as_ref())
                    .map(|s| &s.value);
                let after = current
                    .snapshot
                    .blocks
                    .iter()
                    .find(|b| b.position == *pos)
                    .and_then(|b| b.state.as_ref())
                    .map(|s| &s.value);
                if before == after {
                    sequence = ops
                        .body_block_send(world, &request.action, 2, Some(&baseline.snapshot))
                        .await?;
                }
                if after.is_some_and(|state| state.name == "minecraft:air") || before == after {
                    cleanup.dig = None;
                }
            }
        }
        Action::Move { controls, ticks } => {
            let mode = mode()?;
            let control = inner.client.player_control(mode);
            let record = ops.body_start_control(mode, world).await?;
            cleanup.control = Some((mode, record.session_id));
            cleanup.attempted = true;
            let configured = control
                .set_controls_for(record.session_id, *controls)
                .await?;
            cleanup.dispatched = true;
            let count = configured.dispatched_ticks + u64::from(ticks.unwrap_or(1));
            loop {
                let record = control
                    .control_record()
                    .await?
                    .ok_or_else(|| invalid("movement owner lost"))?;
                if record.session_id != cleanup.control.unwrap().1 {
                    return Err(invalid("movement owner replaced"));
                }
                let batch = ops
                    .body_evidence(inner.id, &selection, Some(cursor))
                    .await?;
                if batch.gap || batch.snapshot.world != world {
                    inner.finish(
                        id,
                        ActionStatus::Unknown {
                            reason: "movement evidence history gap or world lifetime changed"
                                .into(),
                        },
                    );
                    return Ok(());
                }
                if let Some(status) = correction(&batch, None, &request.action)? {
                    inner.finish(id, status);
                    return Ok(());
                }
                cursor = batch.cursor;
                if record.dispatched_ticks >= count {
                    break;
                }
                if matches!(
                    record.status,
                    crate::client::control::ControlStatus::Stopped { .. }
                ) {
                    return Err(invalid("movement stopped before requested ticks"));
                }
                sleep(Duration::from_millis(5)).await;
            }
            if ticks.is_some() {
                cleanup.release(&inner.client, ops).await?;
            }
        }
    }
    inner.update(id, |m| m.sent(Instant::now()));
    loop {
        let batch = ops
            .body_evidence(inner.id, &selection, Some(cursor))
            .await?;
        cursor = batch.cursor;
        if batch.gap {
            inner.finish(
                id,
                ActionStatus::Unknown {
                    reason: "server evidence history gap".into(),
                },
            );
            return Ok(());
        }
        if batch.snapshot.world != world {
            inner.finish(
                id,
                ActionStatus::Unknown {
                    reason: "world lifetime changed".into(),
                },
            );
            return Ok(());
        }
        if expected.as_ref().is_some_and(|(pos, _)| {
            batch
                .snapshot
                .blocks
                .iter()
                .find(|b| b.position == *pos)
                .is_none_or(|b| b.state.is_none())
        }) {
            inner.finish(
                id,
                ActionStatus::Unknown {
                    reason: "target block is unavailable".into(),
                },
            );
            return Ok(());
        }
        if let Some(status) = correction(&batch, expected.as_ref(), &request.action)? {
            inner.finish(id, status);
            return Ok(());
        }
        let legacy_target_received = batch.snapshot.version == MinecraftVersion::Java1_16_1
            && expected.as_ref().is_some_and(|(target, wanted)| {
                batch.events.iter().any(|event| match &event.update {
                    Update::DigAck {
                        pos,
                        state,
                        status: 0 | 2,
                        successful: true,
                    } if pos == target && matches!(request.action, Action::BreakBlock { .. }) => {
                        (batch.snapshot.version.table().registry.native_state)(*state)
                            .is_ok_and(|state| wanted.matches(&state))
                    }
                    Update::Block(pos, state)
                        if pos == target
                            && !matches!(request.action, Action::BreakBlock { .. }) =>
                    {
                        (batch.snapshot.version.table().registry.native_state)(*state)
                            .is_ok_and(|state| wanted.matches(&state))
                    }
                    _ => false,
                })
            });
        if legacy_target_received || sequence.is_some_and(|s| batch.ack.is_some_and(|ack| ack >= s))
        {
            inner.update(id, |m| m.ack(Instant::now()));
        }
        inner.update(id, |m| m.advance(Instant::now()));
        let status = inner.job(id)?.acceptance.lock().unwrap().status.clone();
        if status.is_terminal() {
            if matches!(request.action, Action::Move { ticks: None, .. })
                && matches!(status, ActionStatus::Accepted)
            {
                // Acceptance covers the initial sent input; the lease remains bounded
                // by the outer absolute deadline and explicit release/cancel.
                loop {
                    sleep(Duration::from_millis(20)).await;
                }
            }
            return Ok(());
        }
        sleep(Duration::from_millis(10)).await;
    }
}
fn correction(
    batch: &EvidenceBatch,
    expected: Option<&([i32; 3], Expected)>,
    action: &Action,
) -> Result<Option<ActionStatus>> {
    for evidence in &batch.events {
        let received = ValueSource::Received {
            sequence: evidence.sequence,
        };
        let (reason, actual) = match &evidence.update {
            Update::DigAck {
                pos,
                state,
                status,
                successful,
            } if matches!(action, Action::BreakBlock { .. })
                && expected.is_some_and(|(target, _)| target == pos) =>
            {
                let state = (batch.snapshot.version.table().registry.native_state)(*state)?;
                // START normally acknowledges the original, still-solid block.
                // FINISH (or an instant START) supplies the actual result.
                if *successful && (*status == 0 || expected.unwrap().1.matches(&state)) {
                    continue;
                }
                (
                    "server dig acknowledgement contradicted the requested result",
                    ServerCorrection::Block(BodyBlock {
                        position: *pos,
                        state: Some(Value {
                            value: state,
                            source: received,
                        }),
                    }),
                )
            }
            Update::Block(pos, state) if expected.is_some_and(|(target, _)| target == pos) => {
                let state = (batch.snapshot.version.table().registry.native_state)(*state)?;
                if expected.unwrap().1.matches(&state) {
                    continue;
                }
                (
                    "server block update contradicted the requested result",
                    ServerCorrection::Block(BodyBlock {
                        position: *pos,
                        state: Some(Value {
                            value: state,
                            source: received,
                        }),
                    }),
                )
            }
            Update::Pose { position, rotation }
                if !matches!(
                    action,
                    Action::Chat(_)
                        | Action::Command(_)
                        | Action::SelectHotbar(_)
                        | Action::UseItem { .. }
                ) =>
            {
                (
                    "server position/rotation correction superseded the action",
                    ServerCorrection::Pose {
                        position: Value {
                            value: *position,
                            source: received.clone(),
                        },
                        rotation: Value {
                            value: *rotation,
                            source: received,
                        },
                    },
                )
            }
            Update::Selection(slot) if matches!(action, Action::SelectHotbar(_)) => {
                if matches!(action, Action::SelectHotbar(wanted) if wanted == slot) {
                    continue;
                }
                (
                    "server selected a hotbar slot",
                    ServerCorrection::SelectedHotbar(Value {
                        value: *slot,
                        source: received,
                    }),
                )
            }
            Update::Resync if matches!(action, Action::UseItem { .. }) => (
                "server resynchronized inventory",
                ServerCorrection::InventoryResync,
            ),
            Update::Chunk(chunk)
                if expected.is_some_and(|(pos, _)| {
                    [pos[0].div_euclid(16), pos[2].div_euclid(16)] == *chunk
                }) =>
            {
                return Ok(Some(ActionStatus::Unknown {
                    reason: "target chunk was replaced or unloaded".into(),
                }));
            }
            Update::World => {
                return Ok(Some(ActionStatus::Unknown {
                    reason: "world lifetime changed".into(),
                }));
            }
            _ => continue,
        };
        return Ok(Some(ActionStatus::Corrected {
            reason: reason.into(),
            correction: actual,
            snapshot: Box::new(batch.snapshot.clone()),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::evidence::Evidence;

    fn snapshot() -> BodySnapshot {
        BodySnapshot {
            backend: 1,
            version: MinecraftVersion::Java1_21_11,
            world: BodyWorld {
                connection: 2,
                generation: 3,
            },
            revision: 100,
            player: BodyPlayer {
                position: None,
                rotation: None,
                game_mode: None,
                health: None,
                selected_hotbar: None,
                inventory: vec![],
                cursor: None,
            },
            blocks: vec![],
        }
    }
    fn batch(updates: Vec<Update>) -> EvidenceBatch {
        EvidenceBatch {
            cursor: updates.len() as u64,
            gap: false,
            ack: Some(20),
            snapshot: snapshot(),
            events: updates
                .into_iter()
                .enumerate()
                .map(|(i, update)| Evidence {
                    ordinal: i as u64 + 1,
                    sequence: i as u64 + 10,
                    update,
                })
                .collect(),
        }
    }
    fn block(name: &str) -> NativeBlockState {
        NativeBlockState {
            name: format!("minecraft:{name}"),
            properties: Default::default(),
        }
    }

    #[test]
    fn correction_keeps_original_rollback_even_if_latest_state_was_restored() {
        let pos = [1, 65, 1];
        let mut batch = batch(vec![
            Update::Block(
                pos,
                crate::versions::java_1_21_11::state_id(&block("stone")).unwrap(),
            ),
            Update::Block(
                pos,
                crate::versions::java_1_21_11::state_id(&block("gold_block")).unwrap(),
            ),
        ]);
        batch.snapshot.blocks.push(BodyBlock {
            position: pos,
            state: Some(Value {
                value: block("gold_block"),
                source: ValueSource::ReceivedCache { revision: 100 },
            }),
        });
        let outcome = correction(
            &batch,
            Some(&(pos, Expected::Name("minecraft:gold_block".into()))),
            &Action::Look { yaw: 0., pitch: 0. },
        )
        .unwrap()
        .unwrap();
        let ActionStatus::Corrected {
            correction: ServerCorrection::Block(actual),
            snapshot,
            ..
        } = outcome.clone()
        else {
            panic!("missing rollback");
        };
        assert_eq!(
            actual.state.unwrap(),
            Value {
                value: block("stone"),
                source: ValueSource::Received { sequence: 10 }
            }
        );
        assert_eq!(
            snapshot.blocks[0].state.as_ref().unwrap().value,
            block("gold_block")
        );
        let now = Instant::now();
        let mut machine = Acceptance::new(
            now + Duration::from_secs(1),
            Duration::from_millis(10),
            true,
        );
        machine.finish(outcome);
        machine.sent(now);
        machine.ack(now);
        machine.advance(now + Duration::from_secs(2));
        assert!(matches!(machine.status, ActionStatus::Corrected { .. }));
    }
    #[test]
    fn expected_block_updates_and_other_cells_are_not_corrections() {
        let pos = [1, 65, 1];
        let stone = crate::versions::java_1_21_11::state_id(&block("stone")).unwrap();
        let batch = batch(vec![
            Update::Block(pos, stone),
            Update::Block([2, 65, 1], 0),
        ]);
        assert!(
            correction(
                &batch,
                Some(&(pos, Expected::Exact(block("stone")))),
                &Action::Look { yaw: 0., pitch: 0. }
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn teleport_and_resync_correct_unacknowledged_actions() {
        let pose = crate::versions::java_1_21_11::operations::ReceivedPose {
            generation: 3,
            receive_sequence: 10,
            position: [4., 65., 2.],
            rotation: [40., 2.],
            velocity: None,
        };
        let batch = batch(vec![Update::Pose {
            position: pose.position,
            rotation: pose.rotation,
        }]);
        assert!(
            correction(&batch, None, &Action::Chat("test".into()))
                .unwrap()
                .is_none()
        );
        let actual = correction(&batch, None, &Action::Look { yaw: 0., pitch: 0. })
            .unwrap()
            .unwrap();
        assert!(matches!(
            actual,
            ActionStatus::Corrected {
                correction: ServerCorrection::Pose { .. },
                ..
            }
        ));
        let resync = self::batch(vec![Update::Resync]);
        for action in [
            Action::Chat("test".into()),
            Action::Look { yaw: 0., pitch: 0. },
            Action::SelectHotbar(1),
        ] {
            assert!(correction(&resync, None, &action).unwrap().is_none());
        }
        assert!(matches!(
            correction(&resync, None, &Action::UseItem { hand: Hand::Main }).unwrap(),
            Some(ActionStatus::Corrected { .. })
        ));
    }
    #[test]
    fn activation_contents_are_not_rollback_but_pose_corrections_still_are() {
        let action = Action::ActivateBlock {
            pos: [0, 64, 0],
            face: BlockFace::Up,
            cursor: [0.5; 3],
            hand: Hand::Main,
        };
        assert!(
            correction(&batch(vec![Update::Resync]), None, &action)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            correction(
                &batch(vec![Update::Pose {
                    position: [1., 65., 0.],
                    rotation: [0.; 2]
                }]),
                None,
                &action
            )
            .unwrap(),
            Some(ActionStatus::Corrected { .. })
        ));
        assert!(matches!(
            correction(&batch(vec![Update::World]), None, &action).unwrap(),
            Some(ActionStatus::Unknown { .. })
        ));
    }
    #[test]
    fn slot_echo_is_not_a_correction_but_a_different_slot_is() {
        let batch = batch(vec![Update::Selection(2)]);
        assert!(
            correction(&batch, None, &Action::SelectHotbar(2))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            correction(&batch, None, &Action::SelectHotbar(3)).unwrap(),
            Some(ActionStatus::Corrected { .. })
        ));
    }
    #[test]
    fn world_or_target_chunk_loss_is_unknown() {
        for update in [Update::World, Update::Chunk([0, 0])] {
            assert!(matches!(
                correction(
                    &batch(vec![update]),
                    Some(&([1, 65, 1], Expected::Name("minecraft:air".into()))),
                    &Action::Chat("test".into())
                )
                .unwrap(),
                Some(ActionStatus::Unknown { .. })
            ));
        }
    }
}
