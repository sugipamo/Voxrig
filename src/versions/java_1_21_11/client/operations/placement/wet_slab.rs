//! Finite native fluid states for liquid-aware oak slabs; no fluid simulation.
use super::*;
use crate::client::survival::{FluidSlabPlacementRecord, SceneSource, SlabFluidState};

// LiquidBlock.LEVEL -> native FluidState source/falling/amount. In particular,
// level 8 is falling amount-eight FLOWING_WATER, not the WATER source type.
const WATER: &[(bool, bool, u8)] = &[
    (true, false, 8),
    (false, false, 7),
    (false, false, 6),
    (false, false, 5),
    (false, false, 4),
    (false, false, 3),
    (false, false, 2),
    (false, false, 1),
    (false, true, 8),
    (false, true, 8),
    (false, true, 8),
    (false, true, 8),
    (false, true, 8),
    (false, true, 8),
    (false, true, 8),
    (false, true, 8),
];
pub(super) fn fluid(block: &crate::NativeBlockState) -> Option<SlabFluidState> {
    if ((air(block) || survival::DRY_CUBES.contains(&block.name.as_str()))
        && crate::versions::java_1_21_11::state_id(block).is_ok())
        || crate::client::survival::terrain::lookup(crate::MinecraftVersion::Java1_21_11, block)
            .is_some()
    {
        return Some(SlabFluidState::Empty);
    }
    if slab::kind_with_fluid(block).is_some() {
        return Some(if block.properties["waterlogged"] == "true" {
            SlabFluidState::Water {
                source: true,
                falling: false,
                amount: 8,
            }
        } else {
            SlabFluidState::Empty
        });
    }
    if block.name != "minecraft:water" || block.properties.len() != 1 {
        return None;
    }
    let level = block.properties.get("level")?;
    let index = level.parse::<usize>().ok()?;
    // Exact complete registered properties, not a permissive integer parser.
    if index.to_string() != *level {
        return None;
    }
    let &(source, falling, amount) = WATER.get(index)?;
    Some(SlabFluidState::Water {
        source,
        falling,
        amount,
    })
}
pub(super) fn destination(block: &crate::NativeBlockState) -> bool {
    fluid(block).is_some()
        && ((air(block) && block.properties.is_empty())
            || block.name == "minecraft:water"
            || matches!(slab::kind_with_fluid(block), Some("top" | "bottom")))
}

#[derive(Clone)]
pub(super) struct Capture {
    source: SceneSource,
    support_fluid: SlabFluidState,
    before_fluid: SlabFluidState,
    expected_fluid: SlabFluidState,
}
pub(super) fn capture(state: &State, intent: &PlacementIntent) -> Result<Capture> {
    for (position, block) in [
        (intent.support, &intent.support_state),
        (intent.target, &intent.before),
    ] {
        let cell = state.reconstruction.cell(&state.world, position);
        if !matches!(
            cell.origin,
            super::super::super::super::reconstruction::StateOrigin::Received
        ) || state
            .world
            .block(position)
            .and_then(|id| super::super::super::super::native_state(id).ok())
            .as_ref()
            != Some(block)
        {
            return Err(unavailable(
                "fluid slab placement requires raw received block/fluid predecessors",
            ));
        }
    }
    Ok(Capture {
        source: SceneSource {
            version: crate::MinecraftVersion::Java1_21_11,
            connection_id: intent.connection_id,
            receive_sequence: state.sequence,
            world_revision: state.world.revision,
            region: crate::Region {
                min: std::array::from_fn(|i| intent.support[i].min(intent.target[i])),
                max: std::array::from_fn(|i| intent.support[i].max(intent.target[i])),
            },
        },
        support_fluid: fluid(&intent.support_state)
            .ok_or_else(|| unavailable("unsupported clicked fluid state"))?,
        before_fluid: fluid(&intent.before)
            .ok_or_else(|| unavailable("unsupported destination fluid state"))?,
        expected_fluid: fluid(&intent.expected).expect("qualified expected slab"),
    })
}
pub(super) fn record_in(state: &State) -> Result<Option<FluidSlabPlacementRecord>> {
    let Some(common) = state
        .common_placement
        .as_ref()
        .filter(|c| c.fluid_slab.is_some())
    else {
        return Ok(state.retired_fluid_slab_placement.clone());
    };
    let capture = common.fluid_slab.as_ref().expect("filtered");
    Ok(Some(FluidSlabPlacementRecord {
        placement: slab::make_record(
            state,
            common.replacement.as_ref().expect("fluid predecessor"),
        )?,
        source: capture.source.clone(),
        support_fluid: capture.support_fluid,
        before_fluid: capture.before_fluid,
        expected_fluid: capture.expected_fluid,
    }))
}
