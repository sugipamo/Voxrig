//! Connection-owned operational digging. Checks and packet writes share adapter state locks.
use super::dig::{DigContext, DigEstimate, DigRecord};
use super::{BlockFace, GameMode, PlayerObservation, SessionStamp, adapter::DigOps};
use crate::{Error, ErrorKind, NativeBlockState, Result};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

#[derive(Default)]
pub(crate) struct Coordinator {
    operation: Arc<tokio::sync::Mutex<()>>,
    cleanup: Mutex<Option<DigCleanupRecord>>,
}
impl Coordinator {
    pub(crate) fn active(&self) -> bool {
        self.operation.try_lock().is_err()
    }
}
/// Local cancellation cleanup; dispatch is not a server acknowledgement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DigCleanupRecord {
    /// Original transport and world.
    pub session: SessionStamp,
    /// Original target, never a replacement world target.
    pub target: [i32; 3],
    /// Cleanup submission outcome.
    pub status: DigCleanupStatus,
    /// Original cancellation/admission error and any cleanup failure.
    pub reason: String,
    /// ABORT interaction sequence when provided by the protocol.
    pub interaction_sequence: Option<i32>,
}
/// Outcome of cleanup on the original connection and world only.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub enum DigCleanupStatus {
    /// Cleanup has been retained before attempting I/O.
    Pending,
    /// One complete ABORT write; no server acknowledgement claimed.
    Aborted,
    /// No packet sent into a changed world or mode.
    ContextChanged,
    /// Cleanup was refused or I/O failed; inspect instead of retrying.
    Failed,
}
#[derive(Clone)]
pub(crate) struct Inputs {
    pub player: PlayerObservation,
    pub state: NativeBlockState,
    pub estimate: DigEstimate,
    on_ground: bool,
    eye_in_water: bool,
    // Diagnostics only; movement is not an additional validation prerequisite.
    y: f64,
    vertical_velocity: Option<f64>,
}
#[derive(Clone, Copy)]
pub(crate) enum Action {
    Start,
    Finish { elapsed: Duration },
    Abort,
}
impl Action {
    pub(crate) fn status(self) -> i32 {
        match self {
            Self::Start => 0,
            Self::Finish { .. } => 2,
            Self::Abort => 1,
        }
    }
}
impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start => write!(f, "at START"),
            Self::Finish { elapsed } => {
                write!(f, "at STOP, {} ms after START", elapsed.as_millis())
            }
            Self::Abort => write!(f, "at ABORT"),
        }
    }
}
pub(crate) enum Dispatch {
    Sent(Option<i32>),
    Changed(Option<NativeBlockState>),
    /// No packet sent into a changed world or mode.
    ContextChanged,
}
pub(crate) fn unavailable(message: impl std::fmt::Display) -> Error {
    crate::error::state_error(message)
}
pub(crate) fn eye_cell(player: &PlayerObservation) -> Result<([i32; 3], f64)> {
    let feet = player
        .position
        .as_ref()
        .ok_or_else(|| unavailable("dig position unavailable"))?
        .value;
    let eye_y = feet[1] + 1.62;
    let probe = if player.session.version == crate::MinecraftVersion::Java1_16_1 {
        eye_y - f64::from(0.11111111f32)
    } else {
        eye_y
    };
    Ok((
        [
            feet[0].floor() as i32,
            probe.floor() as i32,
            feet[2].floor() as i32,
        ],
        eye_y,
    ))
}
pub(crate) fn inputs(
    player: PlayerObservation,
    state: NativeBlockState,
    on_ground: bool,
    vertical_velocity: Option<f64>,
    at_eye: NativeBlockState,
    above_eye: NativeBlockState,
) -> Result<Inputs> {
    if player.game_mode != Some(GameMode::Survival) {
        return Err(unavailable("dig requires received survival mode"));
    }
    let (cell, eye_y) = eye_cell(&player)?;
    // Diagnostic only: eye_cell has already required the position.
    let y = player.position.as_ref().map_or(f64::NAN, |p| p.value[1]);
    let version = player.session.version;
    let state_id = super::registry::Registry::for_version(version)
        .resolve_block_state(&state)?
        .value();
    let eye_in_water = super::dig::eye_in_water(version, eye_y, &at_eye, &above_eye, cell[1])?;
    let estimate = super::dig::estimate(
        version,
        state_id,
        &DigContext {
            player: &player,
            on_ground,
            eye_in_water,
        },
    )?;
    Ok(Inputs {
        player,
        state,
        estimate,
        on_ground,
        eye_in_water,
        y,
        vertical_velocity,
    })
}
pub(crate) fn validate(initial: &Inputs, current: &Inputs, action: Action) -> Result<()> {
    if initial.player.session != current.player.session
        || current.player.game_mode != Some(GameMode::Survival)
    {
        return Err(unavailable(
            "dig original world or mode changed before dispatch",
        ));
    }
    let selected = initial
        .player
        .selected_hotbar
        .as_ref()
        .ok_or_else(|| unavailable("dig selection unavailable"))?
        .value;
    let selected_now = current
        .player
        .selected_hotbar
        .as_ref()
        .ok_or_else(|| unavailable("dig selection unavailable"))?
        .value;
    let mut changes = Vec::new();
    if selected_now != selected {
        changes.push(format!("selected hotbar {selected} -> {selected_now}"));
    }
    // Compare the original hotbar slot even if the selection also changed.
    let before = super::dig::slot(&initial.player, 36 + usize::from(selected))?;
    let after = super::dig::slot(&current.player, 36 + usize::from(selected))?;
    if !super::dig::mining_hand_compatible(initial.player.session.version, before, after)? {
        let data_changed = matches!(
            (before, after),
            (super::SlotKnowledge::Item { item: a }, super::SlotKnowledge::Item { item: b })
                if a.data != b.data
        );
        changes.push(format!(
            "held item changed in hotbar {selected} ({} -> {}{})",
            hand_summary(before),
            hand_summary(after),
            if data_changed {
                "; item data changed"
            } else {
                ""
            },
        ));
    }
    let estimate_admissible = match action {
        Action::Finish { .. } => stop_estimate_admissible(&initial.estimate, &current.estimate),
        Action::Start | Action::Abort => initial.estimate == current.estimate,
    };
    if !estimate_admissible {
        let (a, b) = (&initial.estimate, &current.estimate);
        changes.push(format!(
            "dig estimate changed (ticks {} -> {}, speed {} -> {}, harvestable {} -> {}; hardness {} -> {}, progress_per_tick {} -> {}; on_ground {} -> {}, eye_in_water {} -> {})",
            a.ticks, b.ticks, a.speed, b.speed, a.harvestable, b.harvestable,
            a.hardness, b.hardness, a.progress_per_tick, b.progress_per_tick,
            initial.on_ground, current.on_ground, initial.eye_in_water, current.eye_in_water,
        ));
    }
    if !changes.is_empty() {
        let vy =
            |value: Option<f64>| value.map_or_else(|| "n/a".into(), |value| format!("{value:.4}"));
        return Err(unavailable(format!(
            "dig selected hand or timing prerequisites changed before dispatch ({action}): {}; y {:.2} -> {:.2}, vy {} -> {}",
            changes.join("; "),
            initial.y,
            current.y,
            vy(initial.vertical_velocity),
            vy(current.vertical_velocity),
        )));
    }
    Ok(())
}

fn stop_estimate_admissible(initial: &DigEstimate, current: &DigEstimate) -> bool {
    initial.hardness == current.hardness
        && initial.harvestable == current.harvestable
        && current.progress_per_tick >= initial.progress_per_tick
        && current.ticks <= initial.ticks
        && [initial, current].iter().all(|estimate| {
            estimate.speed.is_finite()
                && estimate.speed > 0.0
                && estimate.progress_per_tick.is_finite()
                && estimate.progress_per_tick > 0.0
        })
}

fn hand_summary(hand: &super::SlotKnowledge) -> String {
    use super::{ItemData, SlotKnowledge};
    match hand {
        SlotKnowledge::Empty => "Empty".into(),
        SlotKnowledge::Unavailable => "Unavailable".into(),
        SlotKnowledge::Item { item } => {
            // Read only an explicit scalar component; do not decode legacy NBT or
            // resolve/validate the entire modern patch just for a diagnostic.
            let damage = match &item.data {
                ItemData::Default => Some(0),
                ItemData::ModernComponents { patch } => patch
                    .added
                    .iter()
                    .find(|c| c.definition.name == "minecraft:damage")
                    .and_then(|c| {
                        crate::versions::java_1_21_11::item_components::scalar_value(c).ok()
                    }),
                ItemData::LegacyNbt { .. } => None,
            };
            format!(
                "Item({}, id {}, count {}{})",
                item.name,
                item.id.value(),
                item.count,
                damage.map_or_else(String::new, |d| format!(", damage {d}")),
            )
        }
    }
}

struct CancelOnDrop {
    cancelled: Arc<AtomicBool>,
    _signal: oneshot::Sender<()>,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl super::Survival {
    pub(crate) async fn operational_dig(
        &self,
        target: [i32; 3],
        face: BlockFace,
    ) -> Result<DigRecord> {
        let (signal, receive) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop {
            cancelled: cancelled.clone(),
            _signal: signal,
        };
        let owner = self.clone();
        let work = super::dispatch!(&self.client.adapter, a => DigOps::dig_work(a))?;
        super::dispatch!(&self.client.adapter, a => super::adapter::DrainOps::drain_ledger(a))
            .tasks()
            .spawn(
                work.run_result(
                    async move { owner.dig_owned(target, face, cancelled, receive).await },
                ),
            )
            .await
            .map_err(|e| unavailable(format!("dig owner failed: {e}")))?
    }
    /// Latest original cancellation/error cleanup; never permission to replay.
    pub fn dig_cleanup_record(&self) -> Option<DigCleanupRecord> {
        let coordinator = super::dispatch!(&self.client.adapter, a => DigOps::dig_coordinator(a));
        coordinator.cleanup.lock().expect("dig cleanup").clone()
    }
    async fn dig_owned(
        &self,
        target: [i32; 3],
        face: BlockFace,
        cancelled: Arc<AtomicBool>,
        mut receive: oneshot::Receiver<()>,
    ) -> Result<DigRecord> {
        let coordinator = super::dispatch!(&self.client.adapter, a => DigOps::dig_coordinator(a));
        let _exclusive = coordinator
            .operation
            .clone()
            .try_lock_owned()
            .map_err(|_| unavailable("another operational dig owns this connection"))?;
        let initial =
            super::dispatch!(&self.client.adapter, a => DigOps::dig_snapshot(a, target).await)?;
        let unresolved = coordinator
            .cleanup
            .lock()
            .expect("dig cleanup")
            .as_ref()
            .filter(|r| {
                r.session == initial.player.session
                    && matches!(
                        r.status,
                        DigCleanupStatus::Pending | DigCleanupStatus::Failed
                    )
            })
            .cloned();
        if let Some(record) = unresolved {
            return Err(Error::new(
                ErrorKind::OperationUnconfirmed,
                anyhow::anyhow!(
                    "prior dig cleanup is unresolved for {:?} in {:?}: {}; inspect without replay",
                    record.target,
                    record.session,
                    record.reason
                ),
            ));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(unavailable("dig cancelled before START"));
        }
        let start = super::dispatch!(&self.client.adapter, a => DigOps::dig_dispatch(a, &initial, target, face, Action::Start, &cancelled).await)?;
        let mut sequences = Vec::new();
        match start {
            Dispatch::Sent(sequence) => sequences.extend(sequence),
            _ => return Err(unavailable("dig target/context changed before START")),
        }
        // START has completed its packet write. Measure the STOP diagnostic here
        // in the common owner, including the full timing wait below.
        let started = Instant::now();
        let finish = if initial.estimate.ticks > 0 {
            let wait = std::time::Duration::from_millis((initial.estimate.ticks + 1) * 50);
            tokio::select! {
                biased;
                _ = &mut receive => Err(unavailable("dig caller cancelled after START")),
                _ = tokio::time::sleep(wait) => {
                    super::dispatch!(&self.client.adapter, a => DigOps::dig_dispatch(a, &initial, target, face, Action::Finish { elapsed: started.elapsed() }, &cancelled).await)
                }
            }
        } else {
            Ok(Dispatch::Sent(None))
        };
        match finish {
            Err(error) => {
                let cleanup = self
                    .cleanup_dig(
                        &coordinator,
                        &initial,
                        target,
                        face,
                        &cancelled,
                        error.to_string(),
                    )
                    .await;
                if cleanup.status == DigCleanupStatus::Failed {
                    return Err(Error::new(
                        ErrorKind::OperationUnconfirmed,
                        anyhow::Error::new(error).context(format!(
                            "dig cleanup unresolved for {:?} in {:?}: {}",
                            cleanup.target, cleanup.session, cleanup.reason
                        )),
                    ));
                }
                return Err(error);
            }
            Ok(Dispatch::Changed(state)) => {
                return Ok(DigRecord {
                    target,
                    estimate: initial.estimate,
                    removed: super::dig::is_air(state.as_ref()),
                    final_state: state,
                    sequences,
                });
            }
            Ok(Dispatch::ContextChanged) => {
                return Err(unavailable("dig original context changed"));
            }
            Ok(Dispatch::Sent(sequence)) => sequences.extend(sequence),
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(unavailable("dig caller cancelled after completed dispatch"));
            }
            let capture = self
                .client
                .capture(crate::Region {
                    min: target,
                    max: target,
                })
                .await?;
            if capture.player.session != initial.player.session {
                return Err(unavailable("dig world changed while awaiting target"));
            }
            let state = capture
                .world
                .blocks
                .into_iter()
                .next()
                .and_then(|b| b.state);
            if state.as_ref() != Some(&initial.state) || tokio::time::Instant::now() >= deadline {
                if state.as_ref() == Some(&initial.state) {
                    let _ = super::ledger::finite::checked::<(), _>(Err(()));
                }
                return Ok(DigRecord {
                    target,
                    estimate: initial.estimate,
                    removed: super::dig::is_air(state.as_ref()),
                    final_state: state,
                    sequences,
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    async fn cleanup_dig(
        &self,
        coordinator: &Coordinator,
        initial: &Inputs,
        target: [i32; 3],
        face: BlockFace,
        cancelled: &AtomicBool,
        reason: String,
    ) -> DigCleanupRecord {
        let mut record = DigCleanupRecord {
            session: initial.player.session,
            target,
            status: DigCleanupStatus::Pending,
            reason,
            interaction_sequence: None,
        };
        *coordinator.cleanup.lock().expect("dig cleanup") = Some(record.clone());
        let result = super::dispatch!(&self.client.adapter, a => DigOps::dig_dispatch(a, initial, target, face, Action::Abort, cancelled).await);
        match result {
            Ok(Dispatch::Sent(sequence)) => {
                record.status = DigCleanupStatus::Aborted;
                record.interaction_sequence = sequence;
            }
            Ok(_) => record.status = DigCleanupStatus::ContextChanged,
            Err(e) => {
                record.status = DigCleanupStatus::Failed;
                record.reason.push_str(&format!("; ABORT failed: {e}"));
            }
        }
        *coordinator.cleanup.lock().expect("dig cleanup") = Some(record.clone());
        record
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinecraftVersion;
    use crate::client::{InventoryObservation, ItemData, ItemStack, SlotKnowledge, received};

    const PREFIX: &str = "dig selected hand or timing prerequisites changed before dispatch";

    fn state(name: &str) -> NativeBlockState {
        NativeBlockState {
            name: format!("minecraft:{name}"),
            properties: Default::default(),
        }
    }

    fn fixture(version: MinecraftVersion) -> Inputs {
        let player = PlayerObservation {
            session: SessionStamp {
                version,
                connection_id: 7,
                world_generation: 2,
            },
            receive_sequence: 10,
            pending_dispatch: false,
            dimension: None,
            position: Some(received([0.5, 64.0, 0.5], 10)),
            received_pose: None,
            rotation: [0.; 2],
            rotation_source: None,
            on_ground: None,
            game_mode: Some(GameMode::Survival),
            may_fly: None,
            health: None,
            selected_hotbar: Some(received(2, 10)),
            inventory: InventoryObservation {
                slots: vec![Some(received(SlotKnowledge::Empty, 10)); 46],
                ..Default::default()
            },
            using_item: None,
            fall_flying: None,
            entity_id: None,
            attributes: Default::default(),
            effects: Default::default(),
            air_supply: None,
            world_time: None,
        };
        inputs(
            player,
            state("dirt"),
            true,
            Some(-0.0784),
            state("air"),
            state("air"),
        )
        .unwrap()
    }

    fn item(version: MinecraftVersion, count: u32) -> SlotKnowledge {
        let definition = super::super::registry::Registry::for_version(version)
            .item("minecraft:stone_pickaxe")
            .unwrap();
        SlotKnowledge::Item {
            item: ItemStack {
                id: definition.id,
                name: definition.name,
                count,
                data: ItemData::Default,
            },
        }
    }

    fn set_hand(input: &mut Inputs, hand: SlotKnowledge) {
        input.player.inventory.slots[38] = Some(received(hand, 11));
    }

    fn refusal(initial: &Inputs, current: &Inputs) -> String {
        let error = validate(initial, current, Action::Start).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::State);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!("{PREFIX} (at START): ")),
            "{message}"
        );
        message
    }

    fn stop_refusal(initial: &Inputs, current: &Inputs) -> String {
        let error = validate(
            initial,
            current,
            Action::Finish {
                elapsed: Duration::from_millis(5702),
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::State);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "{PREFIX} (at STOP, 5702 ms after START): dig estimate changed (ticks "
            )),
            "{message}"
        );
        message
    }

    #[test]
    fn landing_and_leaving_water_pass_only_at_stop_and_reverse_transitions_refuse() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let current = fixture(version);
            for water in [false, true] {
                let mut at_eye = state(if water { "water" } else { "air" });
                if water {
                    at_eye.properties.insert("level".into(), "0".into());
                }
                let initial = inputs(
                    current.player.clone(),
                    current.state.clone(),
                    water,
                    current.vertical_velocity,
                    at_eye,
                    state("air"),
                )
                .unwrap();
                assert!(current.estimate.progress_per_tick > initial.estimate.progress_per_tick);
                assert!(current.estimate.ticks < initial.estimate.ticks);
                validate(
                    &initial,
                    &current,
                    Action::Finish {
                        elapsed: Duration::from_millis((initial.estimate.ticks + 1) * 50),
                    },
                )
                .unwrap();
                assert!(refusal(&initial, &current).contains("dig estimate changed"));
                let message = stop_refusal(&current, &initial);
                assert!(message.contains(if water {
                    "on_ground true -> true, eye_in_water false -> true"
                } else {
                    "on_ground true -> false, eye_in_water false -> false"
                }));
            }
        }
    }

    #[test]
    fn stop_refuses_slower_progress_even_with_the_same_ticks() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let initial = fixture(version);
            let mut current = initial.clone();
            current.estimate.progress_per_tick =
                f32::from_bits(initial.estimate.progress_per_tick.to_bits() - 1);
            assert_eq!(current.estimate.ticks, initial.estimate.ticks);
            let message = stop_refusal(&initial, &current);
            assert!(message.contains(&format!(
                "progress_per_tick {} -> {}",
                initial.estimate.progress_per_tick, current.estimate.progress_per_tick
            )));
        }
    }

    #[test]
    fn stop_requires_same_hardness_and_harvestability_and_no_more_ticks() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let initial = fixture(version);
            let mut faster = initial.clone();
            faster.estimate.speed *= 2.0;
            faster.estimate.progress_per_tick *= 2.0;
            faster.estimate.ticks /= 2;
            for field in ["hardness", "harvestable", "ticks"] {
                let mut current = faster.clone();
                match field {
                    "hardness" => current.estimate.hardness *= 2.0,
                    "harvestable" => current.estimate.harvestable = !initial.estimate.harvestable,
                    "ticks" => current.estimate.ticks = initial.estimate.ticks + 1,
                    _ => unreachable!(),
                }
                assert!(stop_refusal(&initial, &current).contains(field));
            }
        }
    }

    #[test]
    fn stop_requires_finite_positive_speed_and_progress_in_both_estimates() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
                for field in ["speed", "progress_per_tick"] {
                    let valid = fixture(version);
                    let mut invalid = valid.clone();
                    match field {
                        "speed" => invalid.estimate.speed = value,
                        "progress_per_tick" => invalid.estimate.progress_per_tick = value,
                        _ => unreachable!(),
                    }
                    for (initial, current) in
                        [(&invalid, &valid), (&valid, &invalid), (&invalid, &invalid)]
                    {
                        assert!(stop_refusal(initial, current).contains(field));
                    }
                }
            }
        }
    }

    #[test]
    fn selected_hotbar_refusal_names_before_and_after() {
        let initial = fixture(MinecraftVersion::Java1_21_11);
        let mut current = initial.clone();
        current.player.selected_hotbar = Some(received(5, 11));
        assert_eq!(
            refusal(&initial, &current),
            format!(
                "{PREFIX} (at START): selected hotbar 2 -> 5; y 64.00 -> 64.00, vy -0.0784 -> -0.0784"
            )
        );
    }

    #[test]
    fn stop_refusal_preserves_prefix_and_reports_elapsed_and_movement() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let initial = fixture(version);
            let mut player = initial.player.clone();
            player.position.as_mut().unwrap().value[1] = 66.0;
            player.selected_hotbar = Some(received(5, 11));
            for (velocity, expected) in [(Some(0.0), "0.0000"), (None, "n/a")] {
                let current = inputs(
                    player.clone(),
                    initial.state.clone(),
                    true,
                    velocity,
                    state("air"),
                    state("air"),
                )
                .unwrap();
                let error = validate(
                    &initial,
                    &current,
                    Action::Finish {
                        elapsed: Duration::from_micros(5_702_999),
                    },
                )
                .unwrap_err();
                assert_eq!(error.kind(), ErrorKind::State);
                assert_eq!(
                    error.to_string(),
                    format!(
                        "{PREFIX} (at STOP, 5702 ms after START): selected hotbar 2 -> 5; y 64.00 -> 66.00, vy -0.0784 -> {expected}"
                    )
                );
                let reverse = validate(&current, &initial, Action::Start).unwrap_err();
                assert!(
                    reverse
                        .to_string()
                        .ends_with(&format!("y 66.00 -> 64.00, vy {expected} -> -0.0784"))
                );
            }
        }
    }

    #[test]
    fn held_item_refusal_distinguishes_empty_item_and_count_changes() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut initial = fixture(version);
            let mut current = initial.clone();
            set_hand(&mut current, item(version, 1));
            let message = refusal(&initial, &current);
            assert!(message.contains("held item changed in hotbar 2 (Empty -> Item("));
            assert!(message.contains("minecraft:stone_pickaxe, id "));
            assert!(message.contains("count 1, damage 0"));
            assert!(refusal(&current, &initial).contains(" -> Empty)"));
            set_hand(&mut initial, item(version, 2));
            let message = refusal(&initial, &current);
            assert!(message.contains("count 2, damage 0) -> Item("));
            assert!(message.contains("count 1, damage 0)"));
            assert!(!message.contains("dig estimate changed"));
        }
    }

    #[test]
    fn held_item_data_refusal_is_concise_and_reports_cheap_damage() {
        let version = MinecraftVersion::Java1_21_11;
        let mut initial = fixture(version);
        set_hand(&mut initial, item(version, 1));
        let mut current = initial.clone();
        let SlotKnowledge::Item { item } =
            &mut current.player.inventory.slots[38].as_mut().unwrap().value
        else {
            unreachable!()
        };
        item.data = ItemData::ModernComponents {
            patch: crate::client::ItemComponentPatch {
                added: vec![crate::client::ItemComponent {
                    definition: super::super::registry::Registry::for_version(version)
                        .item_component("minecraft:damage")
                        .unwrap(),
                    bytes: vec![3],
                }],
                removed: vec![],
            },
        };
        let message = refusal(&initial, &current);
        assert!(message.contains("held item changed in hotbar 2"));
        assert!(message.contains("damage 0) -> Item("));
        assert!(message.contains("damage 3); item data changed)"));
        assert!(!message.contains("bytes"));
    }

    #[test]
    fn estimate_refusal_reports_ground_and_water_used_by_the_builder() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let initial = fixture(version);
            for water in [false, true] {
                let mut at_eye = state(if water { "water" } else { "air" });
                if water {
                    at_eye.properties.insert("level".into(), "0".into());
                }
                let current = inputs(
                    initial.player.clone(),
                    initial.state.clone(),
                    water,
                    initial.vertical_velocity,
                    at_eye,
                    state("air"),
                )
                .unwrap();
                let message = refusal(&initial, &current);
                assert!(message.contains(&format!(
                    "dig estimate changed (ticks {} -> {}, speed {} -> {}, harvestable true -> true",
                    initial.estimate.ticks, current.estimate.ticks,
                    initial.estimate.speed, current.estimate.speed,
                )));
                assert!(message.contains(if water {
                    "on_ground true -> true, eye_in_water false -> true"
                } else {
                    "on_ground true -> false, eye_in_water false -> false"
                }));
                assert!(!message.contains("held item changed"));
                assert!(!message.contains("selected hotbar"));
            }
        }
    }

    #[test]
    fn simultaneous_prerequisite_changes_are_all_reported() {
        let version = MinecraftVersion::Java1_21_11;
        let initial = fixture(version);
        let mut current = initial.clone();
        current.player.selected_hotbar = Some(received(5, 11));
        set_hand(&mut current, item(version, 1));
        current.estimate.harvestable = false;
        let message = refusal(&initial, &current);
        assert!(message.contains("selected hotbar 2 -> 5; held item changed in hotbar 2"));
        assert!(message.contains("dig estimate changed"));
        assert!(message.contains("harvestable true -> false"));
    }

    #[test]
    fn unchanged_inputs_pass_and_missing_selection_keeps_its_own_message() {
        let initial = fixture(MinecraftVersion::Java1_21_11);
        validate(&initial, &initial, Action::Start).unwrap();
        let mut missing = initial.clone();
        missing.player.selected_hotbar = None;
        for (a, b) in [(&initial, &missing), (&missing, &initial)] {
            let error = validate(a, b, Action::Start).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::State);
            assert_eq!(error.to_string(), "dig selection unavailable");
        }
        // Diagnostic context is not a new prerequisite: estimates still decide.
        let mut current = initial.clone();
        current.on_ground = false;
        current.eye_in_water = true;
        current.y = 66.0;
        current.vertical_velocity = None;
        for action in [
            Action::Start,
            Action::Finish {
                elapsed: Duration::from_millis(5702),
            },
        ] {
            validate(&initial, &current, action).unwrap();
        }
    }

    #[test]
    fn movement_diagnostics_from_the_builder_do_not_cause_refusal() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let initial = fixture(version);
            let mut player = initial.player.clone();
            player.position.as_mut().unwrap().value[1] = 66.0;
            for velocity in [Some(0.0), None] {
                let current = inputs(
                    player.clone(),
                    initial.state.clone(),
                    true,
                    velocity,
                    state("air"),
                    state("air"),
                )
                .unwrap();
                assert_eq!(current.y, 66.0);
                assert_eq!(current.vertical_velocity, velocity);
                for action in [
                    Action::Start,
                    Action::Finish {
                        elapsed: Duration::ZERO,
                    },
                ] {
                    validate(&initial, &current, action).unwrap();
                }
            }
        }
    }
}
