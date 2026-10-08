//! Survival digging of any block with the held item, timed like the official client
//! (`MultiPlayerGameMode` with `BlockStateBase.getDestroyProgress`). See docs/common-dig.md.
use super::{BlockFace, ObservedValue, PlayerObservation, SlotKnowledge};
use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Result};

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
    /// Target cell.
    pub target: [i32; 3],
    /// The estimate the dig was timed with.
    pub estimate: DigEstimate,
    /// Received state of the target when the dig ended (None if unloaded).
    pub final_state: Option<NativeBlockState>,
    /// The received target state changed from the dug state.
    pub removed: bool,
    /// 1.21.11 interaction sequences of START and STOP.
    pub sequences: Vec<i32>,
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

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}

/// Facts about the player that the official `getDestroySpeed` reads.
pub(crate) struct DigContext<'a> {
    pub player: &'a PlayerObservation,
    pub on_ground: bool,
    pub eye_in_water: bool,
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
        .ok_or_else(|| invalid("block state outside the dig profiles"))?;
    let profile = &data.profiles[row[2] as usize];
    if profile.hardness < 0.0 {
        return Err(invalid("the block cannot be broken in survival"));
    }
    let player = context.player;
    let selected = player
        .selected_hotbar
        .as_ref()
        .map(|s| s.value)
        .ok_or_else(|| invalid("selected hotbar slot is not known"))?;
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
        SlotKnowledge::Unavailable => return Err(invalid("held item is not known")),
    };
    let attribute = |name: &str, default: f64| {
        player
            .attributes
            .get(name)
            .map_or(default, |a| a.value.value)
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
            speed += attribute("minecraft:mining_efficiency", 0.0) as f32;
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
        speed *= attribute("minecraft:block_break_speed", 1.0) as f32;
    }
    if context.eye_in_water {
        if legacy {
            let helmet = match slot(player, 5)? {
                SlotKnowledge::Item { item } => legacy_enchantment(item, "aqua_affinity")?,
                _ => 0,
            };
            if helmet == 0 {
                speed /= 5.0;
            }
        } else {
            speed *= attribute("minecraft:submerged_mining_speed", 0.2) as f32;
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
                return Err(invalid("the dig would take more than an hour"));
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

fn slot(player: &PlayerObservation, index: usize) -> Result<&SlotKnowledge> {
    match player.inventory.slots.get(index).and_then(Option::as_ref) {
        Some(ObservedValue { value, .. }) => Ok(value),
        None => Err(invalid("received inventory slot is not known")),
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

impl super::Survival {
    /// The dig time of `target` with what the player holds now.
    pub async fn dig_estimate(&self, target: [i32; 3]) -> Result<DigEstimate> {
        let (_, estimate) = self.dig_inputs(target).await?;
        Ok(estimate)
    }

    /// Dig `target` like the official client: START, wait the estimated ticks
    /// (`DigEstimate::ticks`, plus one), STOP, then wait up to one second for the
    /// received target state to change. Not removed is not an error: the server decides.
    /// Dropping the future after START leaves the server-side dig to time out.
    pub async fn dig(&self, target: [i32; 3], face: BlockFace) -> Result<DigRecord> {
        let (before, estimate) = self.dig_inputs(target).await?;
        let mut sequences = Vec::new();
        let start = self
            .client
            .execute(
                super::GameMode::Survival,
                super::operations::Action::DigStart(target, face),
            )
            .await?;
        sequences.extend(start.interaction_sequence);
        if estimate.ticks > 0 {
            tokio::time::sleep(std::time::Duration::from_millis((estimate.ticks + 1) * 50)).await;
            let stop = self
                .client
                .execute(
                    super::GameMode::Survival,
                    super::operations::Action::DigFinish(target, face),
                )
                .await?;
            sequences.extend(stop.interaction_sequence);
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let mut final_state;
        loop {
            final_state = self.client.block_state(target).await?;
            if final_state.as_ref() != Some(&before) || tokio::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok(DigRecord {
            target,
            removed: final_state.as_ref() != Some(&before),
            estimate,
            final_state,
            sequences,
        })
    }

    async fn dig_inputs(&self, target: [i32; 3]) -> Result<(NativeBlockState, DigEstimate)> {
        let version = self.client.version();
        let player = self.client.player_state().await?;
        let state = self
            .client
            .block_state(target)
            .await?
            .ok_or_else(|| invalid("dig target is not loaded"))?;
        let state_id = super::registry::Registry::for_version(version)
            .resolve_block_state(&state)?
            .value();
        let feet = player
            .position
            .as_ref()
            .ok_or_else(|| invalid("player position unavailable"))?
            .value;
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
        let eye_in_water = match (
            self.client.block_state(cell).await?,
            self.client
                .block_state([cell[0], cell[1] + 1, cell[2]])
                .await?,
        ) {
            (Some(at), Some(above)) => eye_in_water(version, eye_y, &at, &above, cell[1])?,
            _ => return Err(invalid("cells at the eye are not loaded")),
        };
        let on_ground = crate::client::dispatch!(&self.client.adapter, a => crate::client::adapter::DigOps::own_on_ground(a).await)?;
        let estimate = estimate(
            version,
            state_id,
            &DigContext {
                player: &player,
                on_ground,
                eye_in_water,
            },
        )?;
        Ok((state, estimate))
    }
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
