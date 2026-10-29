//! Survival digging of any block with the held item, timed like the official client
//! (`MultiPlayerGameMode` with `BlockStateBase.getDestroyProgress`). See docs/common-dig.md.
use super::{BlockFace, ObservedValue, PlayerObservation, SlotKnowledge};
use crate::error::invalid_input;
use crate::{MinecraftVersion, NativeBlockState, Result};

/// The dig time of one block with the current hand, effects and attributes.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DigEstimate {
    /// Block hardness (`getDestroySpeed`); 0 breaks at once.
    pub hardness: f32,
    /// The held item is the correct tool (or none is required).
    pub harvestable: bool,
    /// Player destroy speed after enchantment, effects, attributes, water and ground.
    pub speed: f32,
    /// Progress per tick (`getDestroyProgress`).
    pub progress_per_tick: f32,
    /// Client ticks from START until the official client sends STOP; 0 breaks on START.
    pub ticks: u64,
}

/// Outcome of one `dig`.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DigRecord {
    /// Original invocation evidence captured when this call returns.
    /// No live client or replay authority is retained.
    pub snapshot: Option<Box<super::DigSnapshot>>,
    /// Target cell.
    pub target: [i32; 3],
    /// The estimate the dig was timed with.
    pub estimate: DigEstimate,
    /// Received state of the target when the dig ended (None if unloaded).
    pub final_state: Option<NativeBlockState>,
    /// The target was observed as loaded air in the original world.
    /// Unloading and replacement by another non-air block are not removal.
    pub removed: bool,
    /// 1.21.11 interaction sequences of START and STOP.
    pub sequences: Vec<i32>,
    /// How the observation ended; separates a processed no-op from a timeout.
    pub outcome: DigOutcome,
}

/// How a returned `dig` observation ended. New variants may be added.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DigOutcome {
    /// The target was received as loaded air (`removed`).
    Removed,
    /// The target was received as another state or became unloaded.
    Changed,
    /// The server processed the dig and left the target at its predecessor.
    /// Names no server reason (protection, world border, game mode, ...) and is
    /// not a retry instruction.
    Rejected {
        /// Processed-boundary evidence.
        rejection: DigRejection,
    },
    /// Still the predecessor when the one-second window ended, without a processed
    /// boundary that proves no effect. The block may still break later.
    Unconfirmed,
}

/// Server-processed boundary with the target still at its predecessor.
///
/// Java 1.21.11: a fresh exact-target block update equal to the predecessor, then
/// a fresh BlockChangedAck covering the last command. The original handler sends
/// its block update before the tick-time ACK; the ACK alone (delayed destruction,
/// too far) or an echo after it never qualifies. Java 1.16.1: an original START or
/// STOP response with `successful = false` carrying the predecessor, plus the
/// response to the last command still carrying it. A successful STOP response
/// with the predecessor (delayed destruction) never qualifies.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub struct DigRejection {
    /// Receive ordinal of the processed boundary covering the last command: the
    /// 1.21.11 ACK, or the 1.16.1 response to that command.
    pub boundary_receive_sequence: u64,
    /// Latest fresh exact-target receipt, equal to the predecessor.
    pub target_receipt: ObservedValue<NativeBlockState>,
    /// Java 1.21.11 highest acknowledged interaction sequence; None on 1.16.1.
    pub acknowledged_sequence: Option<i32>,
    /// Java 1.16.1 native status of the `successful = false` response (0 START,
    /// 2 STOP); None on 1.21.11.
    pub legacy_status: Option<i32>,
}

/// Exact-target receipts after one dig's START, kept by the owning adapter.
#[derive(Clone, Debug)]
pub(crate) struct DigWatch {
    target: [i32; 3],
    baseline: NativeBlockState,
    start_after: u64,
    last_after: u64,
    last_sequence: Option<i32>,
    /// Native legacy status of the last command written (0 START or 2 STOP).
    last_legacy_status: i32,
    /// Latest fresh exact-target receipt while it equals the baseline.
    echo: Option<u64>,
    /// First 1.16.1 rejecting response carrying the baseline.
    legacy: Option<i32>,
    /// Latest 1.16.1 response matching the last command after its boundary.
    legacy_last: Option<u64>,
}
impl DigWatch {
    /// Retained under the receive gate before START is written.
    pub(crate) fn new(target: [i32; 3], baseline: NativeBlockState, after: u64) -> Self {
        Self {
            target,
            baseline,
            start_after: after,
            last_after: after,
            last_sequence: None,
            last_legacy_status: 0,
            echo: None,
            legacy: None,
            legacy_last: None,
        }
    }
    /// The receive boundary and native sequence before each START/STOP write.
    pub(crate) fn sent(
        &mut self,
        action: super::operational_dig::Action,
        after: u64,
        sequence: Option<i32>,
    ) {
        self.last_after = after;
        self.last_sequence = sequence;
        self.last_legacy_status = action.status();
        self.legacy_last = None;
    }
    /// A block/section/response packet applied at `position`.
    pub(crate) fn received(&mut self, position: [i32; 3], state: &NativeBlockState, receive: u64) {
        if position == self.target && receive > self.start_after {
            self.echo = (*state == self.baseline).then_some(receive);
        }
    }
    /// The target chunk was loaded or unloaded: a chunk is not a handler echo.
    pub(crate) fn chunk_changed(&mut self, chunk: [i32; 2]) {
        if [self.target[0].div_euclid(16), self.target[2].div_euclid(16)] == chunk {
            self.echo = None;
        }
    }
    /// Java 1.16.1 digging response, after its state was applied.
    pub(crate) fn legacy_reply(
        &mut self,
        position: [i32; 3],
        state: &NativeBlockState,
        status: i32,
        successful: bool,
        receive: u64,
    ) {
        self.received(position, state, receive);
        if position != self.target || receive <= self.start_after {
            return;
        }
        if !successful && matches!(status, 0 | 2) && *state == self.baseline {
            self.legacy.get_or_insert(status);
        }
        if receive > self.last_after && status == self.last_legacy_status {
            self.legacy_last = Some(receive);
        }
    }
    /// Retain a START (fresh watch) or STOP boundary before its write; ABORT is cleanup only.
    pub(crate) fn sent_into(
        watch: &mut Option<Self>,
        action: super::operational_dig::Action,
        target: [i32; 3],
        baseline: &NativeBlockState,
        after: u64,
        sequence: Option<i32>,
    ) {
        use super::operational_dig::Action;
        match action {
            Action::Start => *watch = Some(Self::new(target, baseline.clone(), after)),
            Action::Finish { .. } => {}
            Action::Abort => return,
        }
        if let Some(watch) = watch.as_mut().filter(|w| w.target == target) {
            watch.sent(action, after, sequence);
        }
    }
    /// `ack` is the highest received 1.21.11 ACK and its ordinal; None on 1.16.1.
    pub(crate) fn rejection(&self, ack: Option<(i32, u64)>) -> Option<DigRejection> {
        let echo = self.echo?;
        let target_receipt = super::observation::received(self.baseline.clone(), echo);
        if let Some(status) = self.legacy {
            // A stale server-side destroy position could still let STOP break
            // the block; wait for the response to the last command.
            return self.legacy_last.map(|receive| DigRejection {
                boundary_receive_sequence: receive,
                target_receipt,
                acknowledged_sequence: None,
                legacy_status: Some(status),
            });
        }
        let last = self.last_sequence?;
        // The handler's echo precedes the tick-time ACK covering its command.
        let (sequence, receive) = ack.filter(|(sequence, receive)| {
            *sequence >= last && *receive > self.last_after && echo < *receive
        })?;
        Some(DigRejection {
            boundary_receive_sequence: receive,
            target_receipt,
            acknowledged_sequence: Some(sequence),
            legacy_status: None,
        })
    }
}

#[derive(serde::Deserialize)]
struct Profiles {
    profiles: Vec<Profile>,
    /// [native state id, block id, profile index] in state-id order.
    states: Vec<[i32; 3]>,
}
#[derive(serde::Deserialize)]
struct Profile {
    hardness: f32,
    hand_harvestable: bool,
    item_overrides: Vec<Tool>,
}
#[derive(serde::Deserialize)]
struct Tool {
    item: String,
    speed: f32,
    harvestable: bool,
}

fn profiles(version: MinecraftVersion) -> &'static Profiles {
    static PROFILES: crate::versions::table::PerVersion<Profiles> =
        crate::versions::table::PerVersion::new();
    PROFILES.get(version, |table| {
        serde_json::from_reader(flate2::read::GzDecoder::new(table.data.dig_profiles))
            .expect("pinned official dig profiles")
    })
}

/// Facts about the player that the official `getDestroySpeed` reads.
pub(crate) struct DigContext<'a> {
    pub player: &'a PlayerObservation,
    pub on_ground: bool,
    pub eye_in_water: bool,
}

/// A known negative hardness is a server impossibility, independent of timing.
/// Missing profiles are deliberately not interpreted as unbreakable.
pub(crate) fn known_hardness(version: MinecraftVersion, state_id: i32) -> Option<f32> {
    let data = profiles(version);
    let row = usize::try_from(state_id)
        .ok()
        .and_then(|i| data.states.get(i))?;
    (row[0] == state_id).then(|| data.profiles[row[2] as usize].hardness)
}

/// `BlockStateBase.getDestroyProgress` for the player and held item.
pub(crate) fn estimate(
    version: MinecraftVersion,
    state_id: i32,
    context: &DigContext<'_>,
) -> Result<DigEstimate> {
    let data = profiles(version);
    let row = usize::try_from(state_id)
        .ok()
        .and_then(|i| data.states.get(i))
        .filter(|row| row[0] == state_id)
        .ok_or_else(|| invalid_input("block state outside the dig profiles"))?;
    let profile = &data.profiles[row[2] as usize];
    if profile.hardness < 0.0 {
        return Err(invalid_input("the block cannot be broken in survival"));
    }
    let player = context.player;
    let selected = player
        .selected_hotbar
        .as_ref()
        .map(|s| s.value)
        .ok_or_else(|| refusal(DigRefusal::MissingSelection))?;
    let held = slot(player, 36 + usize::from(selected))?;
    let (mut speed, harvestable) = match held {
        SlotKnowledge::Empty => (1.0f32, profile.hand_harvestable),
        SlotKnowledge::Item { item } => profile
            .item_overrides
            .iter()
            .find(|t| t.item == item.name)
            .map_or((1.0, profile.hand_harvestable), |t| {
                (t.speed, t.harvestable)
            }),
        SlotKnowledge::Unavailable => {
            return Err(refusal(DigRefusal::MissingSlot {
                slot: 36 + usize::from(selected),
            }));
        }
    };
    // PlayerAttribute retains the pre-clamp observation; apply native ranges here.
    let attribute = |name: &str, default: f64, max: f64| {
        player
            .attributes
            .get(name)
            .map_or(default, |a| a.value.value)
            .clamp(0.0, max) as f32
    };
    let effect = |name: &str| player.effects.get(name).map(|e| e.value.amplifier);
    let legacy = version == MinecraftVersion::Java1_16_1;
    if speed > 1.0 {
        if legacy {
            let level = match held {
                SlotKnowledge::Item { item } => legacy_enchantment(item, "efficiency")?,
                _ => 0,
            };
            if level > 0 {
                speed += (level * level + 1) as f32;
            }
        } else {
            speed += attribute("minecraft:mining_efficiency", 0.0, 1024.0);
        }
    }
    // MobEffectUtil.getDigSpeedAmplification: the larger of haste and conduit power.
    let haste = [effect("minecraft:haste"), effect("minecraft:conduit_power")]
        .into_iter()
        .flatten()
        .max();
    if let Some(amplifier) = haste {
        speed *= 1.0 + (amplifier + 1) as f32 * 0.2;
    }
    if let Some(amplifier) = effect("minecraft:mining_fatigue") {
        speed *= match amplifier {
            0 => 0.3,
            1 => 0.09,
            2 => 0.0027,
            _ => 8.1e-4,
        };
    }
    if !legacy {
        speed *= attribute("minecraft:block_break_speed", 1.0, 1024.0);
    }
    if context.eye_in_water {
        if legacy {
            let helmet = match slot(player, 5)? {
                SlotKnowledge::Item { item } => legacy_enchantment(item, "aqua_affinity")?,
                SlotKnowledge::Empty => 0,
                SlotKnowledge::Unavailable => {
                    return Err(refusal(DigRefusal::MissingSlot { slot: 5 }));
                }
            };
            if helmet == 0 {
                speed /= 5.0;
            }
        } else {
            speed *= attribute("minecraft:submerged_mining_speed", 0.2, 20.0);
        }
    }
    if !context.on_ground {
        speed /= 5.0;
    }
    let progress = speed / profile.hardness / if harvestable { 30.0 } else { 100.0 };
    // The client breaks on START at full progress, else accumulates it every tick.
    let mut ticks = 0u64;
    if progress < 1.0 {
        let mut accumulated = 0.0f32;
        while accumulated < 1.0 {
            accumulated += progress;
            ticks += 1;
            if ticks > 72_000 {
                return Err(invalid_input("the dig would take more than an hour"));
            }
        }
    }
    Ok(DigEstimate {
        hardness: profile.hardness,
        harvestable,
        speed,
        progress_per_tick: progress,
        ticks,
    })
}

pub(crate) fn slot(player: &PlayerObservation, index: usize) -> Result<&SlotKnowledge> {
    match player.inventory.slots.get(index).and_then(Option::as_ref) {
        Some(ObservedValue {
            value: SlotKnowledge::Unavailable,
            ..
        })
        | None => Err(refusal(DigRefusal::MissingSlot { slot: index })),
        Some(ObservedValue { value, .. }) => Ok(value),
    }
}

/// `EnchantmentHelper.getItemEnchantmentLevel` on 1.16.1 item NBT (`Enchantments`).
fn legacy_enchantment(item: &super::ItemStack, name: &str) -> Result<i32> {
    let Some(data) = item.custom_data()? else {
        return Ok(0);
    };
    let Some(list) = data.root().get("Enchantments").and_then(|v| v.as_list()) else {
        return Ok(0);
    };
    let full = format!("minecraft:{name}");
    for entry in list {
        let Some(compound) = entry.as_compound() else {
            continue;
        };
        let id = compound
            .get("id")
            .and_then(|v| v.as_string())
            .and_then(|s| s.text().ok());
        if id.as_deref() == Some(name) || id.as_deref() == Some(full.as_str()) {
            let level = compound.get("lvl").map_or(0, numeric);
            return Ok(level.clamp(0, 255));
        }
    }
    Ok(0)
}

/// `NumericTag.getAsInt` for byte, short and int tags.
fn numeric(value: &super::nbt::NbtValue) -> i32 {
    use super::nbt::NbtValue as V;
    match value {
        V::Byte(v) => i32::from(*v),
        V::Short(v) => i32::from(*v),
        V::Int(v) => *v,
        _ => 0,
    }
}

/// `Entity.isEyeInFluid(WATER)` from received block states at and above the eye cell.
pub(crate) fn eye_in_water(
    version: MinecraftVersion,
    eye_y: f64,
    cell: &NativeBlockState,
    above: &NativeBlockState,
    cell_y: i32,
) -> Result<bool> {
    use super::physics::blocks::lookup;
    let probe = if version == MinecraftVersion::Java1_16_1 {
        eye_y - f64::from(0.11111111f32)
    } else {
        eye_y
    };
    let (state, _) = lookup(version, cell)?;
    let Some(fluid) = state.fluid.as_ref().filter(|f| !f.lava) else {
        return Ok(false);
    };
    let (above, _) = lookup(version, above)?;
    let height = if above.fluid.as_ref().is_some_and(|f| !f.lava) {
        1.0
    } else {
        f32::from(fluid.amount) / 9.0
    };
    Ok(f64::from(cell_y as f32 + height) > probe)
}

/// [`eye_in_water`] at the standing eye height (1.62) above `feet`; `None` when
/// the eye cell or the one above it is not loaded.
async fn standing_eye_in_water(client: &crate::Client, feet: [f64; 3]) -> Result<Option<bool>> {
    let version = client.version();
    let eye_y = feet[1] + 1.62;
    let probe_y = if version == MinecraftVersion::Java1_16_1 {
        eye_y - f64::from(0.11111111f32)
    } else {
        eye_y
    };
    let cell = [
        feet[0].floor() as i32,
        probe_y.floor() as i32,
        feet[2].floor() as i32,
    ];
    match (
        client.block_state(cell).await?,
        client.block_state([cell[0], cell[1] + 1, cell[2]]).await?,
    ) {
        (Some(at), Some(above)) => eye_in_water(version, eye_y, &at, &above, cell[1]).map(Some),
        _ => Ok(None),
    }
}

impl crate::Client {
    /// Whether the own player's eyes are in water (`Entity.isEyeInFluid(WATER)`).
    /// While a control session runs, this is its latest predicted frame, which
    /// follows the predicted pose. Otherwise it is computed from the received
    /// position and received blocks at the standing eye height, so a crouching or
    /// swimming pose outside a control session is not reflected. `None` when the
    /// position or the cells at the eye are unknown.
    pub async fn eye_in_water(&self) -> Result<Option<bool>> {
        if let Some(record) = self.survival().control_record().await? {
            if let (super::control::ControlStatus::Running, Some(frame)) =
                (&record.status, &record.frame)
            {
                return Ok(Some(frame.eye_in_water));
            }
        }
        let player = self.player_state().await?;
        match player.position {
            Some(position) => standing_eye_in_water(self, position.value).await,
            None => Ok(None),
        }
    }
}

impl super::Survival {
    /// The dig time of `target` with what the player holds now.
    pub async fn dig_estimate(&self, target: [i32; 3]) -> Result<DigEstimate> {
        let (_, estimate, _) = self.dig_inputs(target).await?;
        Ok(estimate)
    }

    /// Dig with coherent START/STOP admission on the original world and hand.
    /// Dropping the waiter requests one owned ABORT after a timed START, unless
    /// STOP was already completely written. Instant START needs no ABORT;
    /// changed context or uncertain I/O never causes replay into another world.
    /// The compatibility call has a 30-second total internal deadline covering
    /// every await. Expiry returns OperationUnconfirmed with Error::dig_snapshot;
    /// it never confirms success, replays the mutation, or frees an uncertain gate.
    /// For longer digs use begin_dig and observe/cancel its original owner explicitly.
    pub async fn dig(&self, target: [i32; 3], face: BlockFace) -> Result<DigRecord> {
        self.operational_dig(target, face).await
    }

    async fn dig_inputs(
        &self,
        target: [i32; 3],
    ) -> Result<(NativeBlockState, DigEstimate, PlayerObservation)> {
        let input = super::dispatch!(&self.client.adapter, a => crate::client::adapter::DigOps::dig_snapshot(a, target).await)?;
        Ok((input.state, input.estimate, input.player))
    }
}

pub(crate) fn mining_hand_compatible(
    version: MinecraftVersion,
    before: &SlotKnowledge,
    after: &SlotKnowledge,
) -> Result<bool> {
    let (SlotKnowledge::Item { item: before }, SlotKnowledge::Item { item: after }) =
        (before, after)
    else {
        return Ok(matches!(
            (before, after),
            (SlotKnowledge::Empty, SlotKnowledge::Empty)
        ));
    };
    if before.count == 0 || after.count == 0 || before.id != after.id || before.name != after.name {
        return Ok(false);
    }
    // Count does not affect mining speed; retain exact data identity except for
    // the legacy delayed-wear exception below.
    if before.data == after.data {
        return Ok(true);
    }
    if version != MinecraftVersion::Java1_16_1 {
        return Ok(false);
    }
    use super::nbt::{self, NbtValue};
    use std::sync::Arc;
    let left = before.custom_data()?;
    let right = after.custom_data()?;
    let damage = |data: Option<&super::nbt::NbtData>| -> Option<i32> {
        match data.and_then(|d| d.root().get("Damage")) {
            None => Some(0),
            Some(NbtValue::Int(value)) if *value >= 0 => Some(*value),
            _ => None,
        }
    };
    let (Some(old), Some(new)) = (damage(left.as_ref()), damage(right.as_ref())) else {
        return Ok(false);
    };
    if new < old {
        return Ok(false);
    }
    let root = |data: Option<super::nbt::NbtData>| {
        data.map(|d| Arc::new(NbtValue::Compound(d.root().clone())))
    };
    Ok(nbt::equivalent(
        &nbt::item_damage(root(left), 0),
        &nbt::item_damage(root(right), 0),
        version,
    ))
}

pub(crate) fn is_air(state: Option<&NativeBlockState>) -> bool {
    state.is_some_and(|s| {
        matches!(
            s.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    })
}

/// Loaded profiles per version, for tests and capability checks.
#[cfg(test)]
pub(crate) fn profile_counts(version: MinecraftVersion) -> (usize, usize) {
    let p = profiles(version);
    (p.states.len(), p.profiles.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_legacy_wear_allows_only_monotonic_damage_on_the_same_stack() {
        let version = MinecraftVersion::Java1_16_1;
        let definition = super::super::registry::Registry::for_version(version)
            .item("minecraft:stone_pickaxe")
            .unwrap();
        let item = super::super::ItemStack {
            id: definition.id,
            name: definition.name,
            count: 1,
            data: super::super::ItemData::Default,
        };
        let pristine = SlotKnowledge::Item { item: item.clone() };
        let worn = |damage: i32, extra: bool| {
            let mut item = item.clone();
            let mut bytes = vec![10, 0, 0, 3, 0, 6];
            bytes.extend(b"Damage");
            bytes.extend(damage.to_be_bytes());
            if extra {
                bytes.extend([3, 0, 3]);
                bytes.extend(b"foo");
                bytes.extend(1_i32.to_be_bytes());
            }
            bytes.push(0);
            item.data = super::super::ItemData::LegacyNbt { bytes };
            SlotKnowledge::Item { item }
        };
        assert!(mining_hand_compatible(version, &pristine, &worn(1, false)).unwrap());
        assert!(mining_hand_compatible(version, &worn(1, false), &worn(2, false)).unwrap());
        assert!(!mining_hand_compatible(version, &worn(2, false), &worn(1, false)).unwrap());
        assert!(!mining_hand_compatible(version, &worn(1, false), &worn(2, true)).unwrap());
        assert!(!mining_hand_compatible(version, &pristine, &SlotKnowledge::Empty).unwrap());
        assert!(!mining_hand_compatible(version, &pristine, &worn(-1, false)).unwrap());
    }

    #[test]
    fn profiles_cover_every_state_in_id_order() {
        assert_eq!(profile_counts(MinecraftVersion::Java1_16_1), (17104, 64));
        assert_eq!(profile_counts(MinecraftVersion::Java1_21_11), (29671, 86));
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for (i, row) in profiles(version).states.iter().enumerate() {
                assert_eq!(row[0] as usize, i);
            }
        }
    }
}

/// A survival dig cannot determine its timing from the received inventory.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum DigRefusal {
    /// Neither a received nor a completely submitted main-hand selection is known.
    MissingSelection,
    /// This required player-screen slot has no complete supported receipt.
    MissingSlot {
        /// Player-screen index: 36..44 for a selected hand, 5 for a legacy underwater helmet.
        slot: usize,
    },
}
impl std::fmt::Display for DigRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSelection => write!(f, "dig selected hotbar slot is not known"),
            Self::MissingSlot { slot } => write!(
                f,
                "dig required received inventory slot {slot} is not known"
            ),
        }
    }
}
impl std::error::Error for DigRefusal {}
fn refusal(detail: DigRefusal) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::Error::new(detail))
}

#[cfg(test)]
mod mo1_merge_regression {
    use super::*;

    #[test]
    fn mo1_delayed_start_does_not_prove_stop_processed() {
        let target = [0, 65, 2];
        let before = NativeBlockState {
            name: "minecraft:dirt".into(),
            properties: Default::default(),
        };
        // START was sent after receive 10, and STOP after receive 20.
        // The old START response arrives late, after STOP was written.
        // No response to STOP has arrived yet. The server may still remove
        // this same cell when it processes STOP (the retained destroyPos case).
        let mut watch = DigWatch::new(target, before.clone(), 10);
        watch.sent(
            super::super::operational_dig::Action::Finish {
                elapsed: std::time::Duration::ZERO,
            },
            20,
            None,
        );
        watch.legacy_reply(target, &before, 0, false, 21);
        assert!(
            watch.rejection(None).is_none(),
            "late START cannot prove that STOP was processed: {:?}",
            watch.rejection(None)
        );
        watch.legacy_reply(target, &before, 1, true, 22);
        assert!(watch.rejection(None).is_none(), "ABORT is not STOP");
        watch.legacy_reply(target, &before, 2, true, 20);
        assert!(watch.rejection(None).is_none(), "stale STOP is not fresh");
        watch.legacy_reply(target, &before, 2, true, 23);
        let rejection = watch.rejection(None).expect("matching STOP was processed");
        assert_eq!(rejection.boundary_receive_sequence, 23);
        assert_eq!(rejection.legacy_status, Some(0));
    }

    #[test]
    fn instant_start_rejection_matches_start_response() {
        let target = [0, 65, 2];
        let before = NativeBlockState {
            name: "minecraft:dirt".into(),
            properties: Default::default(),
        };
        let mut watch = None;
        DigWatch::sent_into(
            &mut watch,
            super::super::operational_dig::Action::Start,
            target,
            &before,
            10,
            None,
        );
        let watch = watch.as_mut().unwrap();
        watch.legacy_reply(target, &before, 2, false, 11);
        assert!(watch.rejection(None).is_none(), "STOP is not START");
        watch.legacy_reply(target, &before, 0, false, 12);
        assert_eq!(watch.rejection(None).unwrap().boundary_receive_sequence, 12);
    }
}
