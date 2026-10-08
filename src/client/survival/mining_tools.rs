//! Native default-item mining facts for the dry geometry already used by Client.
//! These facts do not grant a send, prove elapsed server ticks or promise drops.
use crate::client::registry::{Registry, ServerRegistryObservation};
use crate::client::{ItemData, ItemStack, SlotKnowledge};
use crate::{MinecraftVersion, NativeBlockState, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
/// Local scheduling inputs from the owning version's native default item getters.
pub struct MiningEstimate {
    /// Native state hardness, not an inferred full-cube material value.
    pub hardness: f32,
    /// Default held-stack destroy speed, before player effects or attributes.
    pub tool_speed: f32,
    /// Native correct-tool gate. This does not guarantee a drop or its collection.
    pub harvestable: bool,
    /// Normal dry standing progress model; not actual server progress.
    pub progress_per_tick: f32,
    /// Model ticks before local scheduling reserve; not a completion fence.
    pub model_ticks: u64,
}
crate::diagnostic_projection::identity!(MiningEstimate);
impl MiningEstimate {
    pub(crate) fn wait_ms(&self) -> u64 {
        // Preserve the old empty-hand stone reserve; small operations also get
        // a local cushion. Time alone never clears the retained mining intent.
        (self.model_ticks + if self.model_ticks > 40 { 20 } else { 7 }) * 50
    }
}
#[derive(serde::Deserialize)]
struct Facts {
    block_tags: BTreeMap<String, Vec<i32>>,
    items: Vec<Item>,
    profiles: Vec<Profile>,
    states: Vec<State>,
}
#[derive(serde::Deserialize)]
struct Item {
    name: String,
    native_id: i32,
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
#[derive(serde::Deserialize)]
struct State {
    state: NativeBlockState,
    native_id: i32,
    block_id: i32,
    profile: usize,
}
fn facts(version: MinecraftVersion) -> &'static Facts {
    static FACTS: crate::versions::table::PerVersion<Facts> =
        crate::versions::table::PerVersion::new();
    FACTS.get(version, |table| {
        serde_json::from_reader(flate2::read::GzDecoder::new(table.data.mining_tools))
            .expect("pinned native default mining getters")
    })
}
fn unavailable(reason: &str) -> crate::Error {
    super::mining::unavailable(reason)
}
pub(crate) fn removes_support(
    version: MinecraftVersion,
    state: &NativeBlockState,
    target: [i32; 3],
    position: [f64; 3],
) -> Result<bool> {
    let body = super::model::body(position);
    Ok(super::model::collision_shape(version, state)?
        .iter()
        .any(|b| {
            let offset: [f64; 6] = std::array::from_fn(|i| b[i] + f64::from(target[i % 3]));
            (offset[4] - position[1]).abs() < 1e-7
                && offset[0] < body[3]
                && offset[3] > body[0]
                && offset[2] < body[5]
                && offset[5] > body[2]
        }))
}
pub(crate) fn material(version: MinecraftVersion, state: &NativeBlockState) -> Result<()> {
    lookup(version, state).map(|_| ())
}
fn lookup(version: MinecraftVersion, state: &NativeBlockState) -> Result<&'static State> {
    let native = Registry::for_version(version)
        .resolve_block_state(state)?
        .value();
    facts(version)
        .states
        .iter()
        .find(|s| s.native_id == native && s.state == *state)
        .ok_or_else(|| unavailable("mining material needs native dry-state facts; update Voxrig"))
}
pub(crate) fn default_item(item: &ItemStack) -> Result<()> {
    if Registry::for_version(item.id.version())
        .item_definition(item.id)?
        .name
        != item.name
    {
        return Err(unavailable("mining item name and native ID disagree"));
    }
    match &item.data {
        ItemData::Default => {}
        ItemData::LegacyNbt { .. } if item.id.version() == MinecraftVersion::Java1_16_1 => {
            if let Some(data) = item.custom_data()? {
                if data.root().entries().iter().any(|e| {
                    e.key().text().ok().as_deref() != Some("Damage") || e.value().as_int().is_none()
                }) {
                    return Err(unavailable(
                        "mining modifiers in legacy item data are not integrated",
                    ));
                }
            }
        }
        ItemData::ModernComponents { patch }
            if item.id.version() == MinecraftVersion::Java1_21_11 =>
        {
            if patch
                .added
                .iter()
                .any(|c| c.definition.name != "minecraft:damage")
                || !patch.removed.is_empty()
            {
                return Err(unavailable("modified mining components are not integrated"));
            }
        }
        _ => return Err(unavailable("mining item data belongs to another version")),
    }
    let p = item.properties()?;
    if item.count == 0
        || item.count > p.max_stack_size.max(0) as u32
        || (p.damageable && (p.damage < 0 || p.damage >= p.max_damage))
    {
        return Err(unavailable("mining requires a valid usable received stack"));
    }
    if !facts(item.id.version())
        .items
        .iter()
        .any(|i| i.native_id == item.id.value() && i.name == item.name)
    {
        return Err(unavailable("native default mining item facts unavailable"));
    }
    Ok(())
}
pub(crate) fn estimate(
    version: MinecraftVersion,
    state: &NativeBlockState,
    hand: &SlotKnowledge,
    registries: Option<&ServerRegistryObservation>,
) -> Result<MiningEstimate> {
    let row = lookup(version, state)?;
    let source = facts(version);
    let profile = &source.profiles[row.profile];
    let (speed, harvestable) = match hand {
        SlotKnowledge::Empty => (1.0, profile.hand_harvestable),
        SlotKnowledge::Item { item } if item.id.version() == version => {
            default_item(item)?;
            let tags = registries
                .and_then(ServerRegistryObservation::tags)
                .and_then(|r| r.value.get("minecraft:block"))
                .ok_or_else(|| unavailable("tool mining requires received block tags"))?;
            // Only this target's membership matters. Unrelated datapack changes
            // need not match the oracle, and unknown/missing tags remain unknown.
            for (name, vanilla) in &source.block_tags {
                let live = tags
                    .get(name)
                    .ok_or_else(|| unavailable("tool mining tag membership unavailable"))?;
                if live.contains(&row.block_id) != vanilla.contains(&row.block_id) {
                    return Err(unavailable(
                        "tool mining target tags differ from native default facts",
                    ));
                }
            }
            profile
                .item_overrides
                .iter()
                .find(|t| t.item == item.name)
                .map_or((1.0, profile.hand_harvestable), |t| {
                    (t.speed, t.harvestable)
                })
        }
        _ => {
            return Err(unavailable(
                "mining selected hand unavailable or belongs to another version",
            ));
        }
    };
    let progress = speed / profile.hardness / if harvestable { 30.0 } else { 100.0 };
    Ok(MiningEstimate {
        hardness: profile.hardness,
        tool_speed: speed,
        harvestable,
        progress_per_tick: progress,
        model_ticks: (1.0f32 / progress).ceil().max(1.0) as u64,
    })
}

#[cfg(test)]
pub(crate) fn test_block_tag_packet(version: MinecraftVersion) -> Vec<u8> {
    use crate::protocol::{put_string, put_varint};
    let tags = &facts(version).block_tags;
    let mut out = Vec::new();
    if version == MinecraftVersion::Java1_21_11 {
        put_varint(&mut out, 1);
        put_string(&mut out, "minecraft:block");
    }
    put_varint(&mut out, tags.len() as i32);
    for (name, ids) in tags {
        put_string(&mut out, name);
        put_varint(&mut out, ids.len() as i32);
        for &id in ids {
            put_varint(&mut out, id);
        }
    }
    if version == MinecraftVersion::Java1_16_1 {
        out.extend([0, 0, 0]);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_half_height_support_is_not_mineable_from_its_standing_seed() {
        let state = NativeBlockState {
            name: "minecraft:oak_slab".into(),
            properties: [
                ("type".into(), "bottom".into()),
                ("waterlogged".into(), "false".into()),
            ]
            .into(),
        };
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            assert!(removes_support(version, &state, [0, 0, 0], [0.5, 0.5, 0.5]).unwrap());
            assert!(!removes_support(version, &state, [0, 0, 0], [2.5, 0.5, 0.5]).unwrap());
        }
    }
    #[test]
    fn native_default_tools_distinguish_correct_wrong_and_cross_version_hands() {
        use crate::client::{SessionStamp, registry::received::ReceivedRegistries};
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut received = ReceivedRegistries::default();
            received
                .receive_tags(&test_block_tag_packet(version), 11, version)
                .unwrap();
            let observation = received.capture(
                SessionStamp {
                    version,
                    connection_id: 9,
                    world_generation: 0,
                },
                11,
            );
            let state = NativeBlockState {
                name: "minecraft:stone".into(),
                properties: Default::default(),
            };
            let registry = Registry::for_version(version);
            let held = |name: &str| {
                let definition = registry.item(name).unwrap();
                SlotKnowledge::Item {
                    item: ItemStack {
                        id: definition.id,
                        name: definition.name,
                        count: 1,
                        data: ItemData::Default,
                    },
                }
            };
            let correct = estimate(
                version,
                &state,
                &held("minecraft:iron_pickaxe"),
                Some(&observation),
            )
            .unwrap();
            assert_eq!(
                (correct.tool_speed, correct.harvestable, correct.model_ticks),
                (6.0, true, 8)
            );
            let wrong = estimate(
                version,
                &state,
                &held("minecraft:iron_axe"),
                Some(&observation),
            )
            .unwrap();
            assert_eq!(
                (wrong.tool_speed, wrong.harvestable, wrong.wait_ms()),
                (1.0, false, 8500)
            );
            assert!(estimate(version, &state, &held("minecraft:iron_pickaxe"), None).is_err());
            let other = if version == MinecraftVersion::Java1_16_1 {
                MinecraftVersion::Java1_21_11
            } else {
                MinecraftVersion::Java1_16_1
            };
            assert!(
                estimate(
                    other,
                    &state,
                    &held("minecraft:iron_pickaxe"),
                    Some(&observation)
                )
                .is_err()
            );
            assert_eq!(
                estimate(version, &state, &SlotKnowledge::Empty, None)
                    .unwrap()
                    .wait_ms(),
                8500
            );
        }
    }
}
