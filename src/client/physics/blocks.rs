//! Per-version movement facts for every block state, exported from each official
//! server (`scripts/movement_oracle`, `--blocks`), plus the review of each block
//! hook that can affect a client player's own movement.
use crate::versions::table::PerVersion;
use crate::{Error, ErrorKind, MinecraftVersion, NativeBlockState, Result};
use serde_json::Value;

/// What a block does to a player standing on, falling onto or inside it,
/// beyond its collision shape. Anything not reviewed is `Unsupported`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Effect {
    /// No client-side movement effect (damage, sounds or server-only state).
    None,
    /// Slime: bounce on landing, slow walking.
    Slime,
    /// Bed: damped bounce on landing.
    Bed,
    /// Cobweb or berry bush: stuck speed multiplier while inside.
    Stuck,
    /// Honey: wall sliding while inside.
    Honey,
    /// Legacy liquid block: marks the player as touching lava.
    Liquid,
    /// Not reproduced by this engine; movement near it is refused.
    Unsupported(&'static str),
}

pub(crate) struct Block {
    pub name: String,
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
    pub climbable: bool,
    pub fences: bool,
    pub walls: bool,
    pub fence_gate: bool,
    pub trapdoor: bool,
    /// Effect while inside the block (`entityInside`).
    pub inside: Effect,
    /// Effect while walking on it (`stepOn`).
    pub step_on: Effect,
    /// Effect after a vertical collision with it (`updateEntity…AfterFallOn`).
    pub after_fall_on: Effect,
    /// Shape problems (positional, context-dependent or block-entity shapes).
    pub shape: Effect,
}

/// Fluid held by a block state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fluid {
    pub lava: bool,
    /// FluidState.getAmount: 8 for a source, less for flowing fluid.
    pub amount: u8,
    pub falling: bool,
}

pub(crate) struct State {
    pub block: u16,
    pub shape: u16,
    pub fluid: Option<Fluid>,
    pub suffocating: bool,
    pub positional: bool,
    /// Material/state `blocksMotion`, used by fluid flow.
    pub blocks_motion: bool,
    /// Sturdy horizontal faces, bits N, E, S, W; bit 4 marks ice, whose
    /// faces flow ignores.
    pub sturdy_faces: u8,
}

pub(crate) struct Table {
    pub blocks: Vec<Block>,
    pub states: Vec<State>,
    pub shapes: Vec<Vec<[f64; 6]>>,
}

static TABLES: PerVersion<Table> = PerVersion::new();

pub(crate) fn table(version: MinecraftVersion) -> &'static Table {
    TABLES.get(version, |table| parse(table.data.movement_blocks))
}

/// Movement facts of one block state.
pub(crate) fn lookup(
    version: MinecraftVersion,
    state: &NativeBlockState,
) -> Result<(&'static State, &'static Block)> {
    let table = table(version);
    let id = (version.table().registry.state_id)(state)?;
    let entry = usize::try_from(id)
        .ok()
        .and_then(|id| table.states.get(id))
        .ok_or_else(|| unsupported(format!("state {id} outside the movement table")))?;
    Ok((entry, &table.blocks[usize::from(entry.block)]))
}

pub(crate) fn unsupported(message: String) -> Error {
    Error::new(ErrorKind::Unsupported, anyhow::anyhow!(message))
}

fn exact(value: &Value) -> f64 {
    value
        .as_str()
        .expect("decimal text")
        .parse()
        .expect("number")
}

fn parse(text: &str) -> Table {
    let data: Value = serde_json::from_str(text).expect("valid movement block table");
    let shapes = data["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shape| {
            shape
                .as_array()
                .unwrap()
                .iter()
                .map(|b| std::array::from_fn(|i| exact(&b[i])))
                .collect()
        })
        .collect();
    let count = data["state_count"].as_u64().unwrap() as usize;
    let flags = |key: &str| {
        let mut set = vec![false; count];
        for id in data[key].as_array().unwrap() {
            set[id.as_u64().unwrap() as usize] = true;
        }
        set
    };
    let (suffocating, positional, blocks_motion) = (
        flags("suffocating_states"),
        flags("positional_shape_states"),
        flags("blocks_motion_states"),
    );
    let mut fluid = vec![None; count];
    for f in data["fluids"].as_array().unwrap() {
        let v: Vec<u64> = (0..4).map(|i| f[i].as_u64().unwrap()).collect();
        fluid[v[0] as usize] = Some(Fluid {
            lava: v[1] == 2,
            amount: v[2] as u8,
            falling: v[3] == 1,
        });
    }
    let mut sturdy = vec![0u8; count];
    for f in data["sturdy_faces"].as_array().unwrap() {
        sturdy[f[0].as_u64().unwrap() as usize] = f[1].as_u64().unwrap() as u8;
    }
    let mut states: Vec<State> = data["state_shapes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(id, shape)| State {
            block: u16::MAX,
            shape: shape.as_u64().unwrap() as u16,
            fluid: fluid[id],
            suffocating: suffocating[id],
            positional: positional[id],
            blocks_motion: blocks_motion[id],
            sturdy_faces: sturdy[id],
        })
        .collect();
    let mut blocks = Vec::new();
    for (index, b) in data["blocks"].as_array().unwrap().iter().enumerate() {
        let first = b["first_state"].as_u64().unwrap() as usize;
        for state in &mut states[first..first + b["states"].as_u64().unwrap() as usize] {
            state.block = index as u16;
        }
        let tags: Vec<&str> = b["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        let hook = |key: &str| b["hooks"][key].as_str().map(simple_name);
        let factor = |key: &str| b[key].as_str().unwrap().parse::<f32>().unwrap();
        blocks.push(Block {
            name: b["name"].as_str().unwrap().to_owned(),
            friction: factor("friction"),
            speed_factor: factor("speed_factor"),
            jump_factor: factor("jump_factor"),
            climbable: tags.contains(&"climbable"),
            fences: tags.contains(&"fences"),
            walls: tags.contains(&"walls"),
            fence_gate: tags.contains(&"fence_gate"),
            trapdoor: tags.contains(&"trapdoor"),
            inside: hook("entity_inside").map_or(Effect::None, inside_effect),
            step_on: hook("step_on").map_or(Effect::None, step_effect),
            after_fall_on: hook("after_fall_on").map_or(Effect::None, after_fall_effect),
            shape: shape_effect(hook("collision_shape"), hook("inside_shape")),
        });
    }
    assert!(states.iter().all(|s| s.block != u16::MAX));
    Table {
        blocks,
        states,
        shapes,
    }
}

fn simple_name(class: &str) -> &str {
    class.rsplit('.').next().unwrap_or(class)
}

// Reviewed against both official versions (docs/physics-engine.md lists the reasons).
fn inside_effect(class: &str) -> Effect {
    match class {
        // Damage, fire, item/arrow, redstone or server-only block changes.
        "BaseFireBlock"
        | "BasePressurePlateBlock"
        | "ButtonBlock"
        | "CactusBlock"
        | "CampfireBlock"
        | "CropBlock"
        | "DetectorRailBlock"
        | "HopperBlock"
        | "PitcherCropBlock"
        | "TripWireBlock"
        | "WaterlilyBlock"
        | "WitherRoseBlock"
        | "FrogspawnBlock"
        | "EyeblossomBlock"
        | "BigDripleafBlock" => Effect::None,
        "WebBlock" | "SweetBerryBushBlock" => Effect::Stuck,
        "HoneyBlock" => Effect::Honey,
        "LiquidBlock" => Effect::Liquid,
        _ => Effect::Unsupported("block interior effect"),
    }
}

fn step_effect(class: &str) -> Effect {
    match class {
        // Damage, lighting redstone ore, server-only egg/sculk behaviour; legacy
        // stairs delegate to their plain base block.
        "MagmaBlock" | "RedStoneOreBlock" | "TurtleEggBlock" | "SculkSensorBlock"
        | "SculkShriekerBlock" | "StairBlock" => Effect::None,
        "SlimeBlock" => Effect::Slime,
        _ => Effect::Unsupported("block step effect"),
    }
}

fn after_fall_effect(class: &str) -> Effect {
    match class {
        "SlimeBlock" => Effect::Slime,
        "BedBlock" => Effect::Bed,
        _ => Effect::Unsupported("block landing effect"),
    }
}

fn shape_effect(collision: Option<&str>, inside: Option<&str>) -> Effect {
    if inside.is_some() {
        return Effect::Unsupported("block interior shape");
    }
    match collision {
        // Pure functions of the block state.
        None
        | Some(
            "BellBlock"
            | "BigDripleafBlock"
            | "CactusBlock"
            | "ComposterBlock"
            | "CrossCollisionBlock"
            | "FenceGateBlock"
            | "GrindstoneBlock"
            | "HoneyBlock"
            | "LecternBlock"
            | "MossyCarpetBlock"
            | "MudBlock"
            | "PitcherCropBlock"
            | "SculkShriekerBlock"
            | "SnowLayerBlock"
            | "SoulSandBlock"
            | "WallBlock"
            | "WallHangingSignBlock",
        ) => Effect::None,
        // Solid only for entities that can stand on fluid; empty for players.
        Some("LiquidBlock") => Effect::None,
        // Entity context (scaffolding, powder snow, fluids), offsets (bamboo) or
        // block entities (moving pistons).
        Some(_) => Effect::Unsupported("context-dependent collision shape"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_cover_every_registered_state_with_matching_ids() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let t = table(version);
            for (name, expected_shape) in [
                ("minecraft:stone", vec![[0., 0., 0., 1., 1., 1.]]),
                ("minecraft:air", vec![]),
                ("minecraft:soul_sand", vec![[0., 0., 0., 1., 0.875, 1.]]),
            ] {
                let state = NativeBlockState {
                    name: name.into(),
                    properties: Default::default(),
                };
                let (entry, block) = lookup(version, &state).unwrap();
                assert_eq!(block.name, name);
                assert_eq!(t.shapes[usize::from(entry.shape)], expected_shape);
            }
            let ice = t.blocks.iter().find(|b| b.name == "minecraft:ice").unwrap();
            assert_eq!(ice.friction, 0.98);
            let slime = t
                .blocks
                .iter()
                .find(|b| b.name == "minecraft:slime_block")
                .unwrap();
            assert_eq!(
                (slime.step_on, slime.after_fall_on),
                (Effect::Slime, Effect::Slime)
            );
            let scaffolding = t
                .blocks
                .iter()
                .find(|b| b.name == "minecraft:scaffolding")
                .unwrap();
            assert!(scaffolding.climbable);
            assert!(matches!(scaffolding.shape, Effect::Unsupported(_)));
            let bamboo = t
                .blocks
                .iter()
                .find(|b| b.name == "minecraft:bamboo")
                .unwrap();
            assert!(matches!(bamboo.shape, Effect::Unsupported(_)));
        }
    }
}
