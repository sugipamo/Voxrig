//! Independent pre-step-4a selector source (origin/main a55cc4e).
//! Keep the species branches and frozen capability dependency; never read table
//! assignments or production selector adapters to construct expected choices.
use super::{
    frozen_capabilities,
    table::{GeometryModel, IdentityModel, MetadataModel, Semantics, TeamModel, TickModel},
};
use crate::MinecraftVersion;

pub(super) fn owned(version: MinecraftVersion, name: &str) -> Semantics {
    // owned_contacts::reduce_record_inner's original modeled() boundary.
    if !frozen_capabilities::lookup(version, name).modeled() {
        return Semantics {
            tick: TickModel::NoTickBody,
            geometry: GeometryModel::NotApplicable,
            metadata: MetadataModel::NotApplicable,
            team: TeamModel::NotApplicable,
            identity: IdentityModel::NotApplicable,
        };
    }
    Semantics {
        // Original Spawn expression, before ContactTickBody::new.
        tick: if name == "minecraft:player" {
            TickModel::RemotePlayer
        } else if name == "minecraft:chicken" {
            TickModel::Chicken
        } else {
            TickModel::InheritedLiving
        },
        // All admitted inputs used geometry::shape, metadata(), teams() and
        // the mandatory UUID holder. No per-species metadata/team dispatch.
        geometry: GeometryModel::ContactShape,
        metadata: MetadataModel::OwnedLiving,
        team: TeamModel::OwnHolder,
        identity: IdentityModel::UuidHolder,
    }
}

pub(super) fn public_geometry(name: &str) -> GeometryModel {
    // Original EntityObservation::derived_geometry species match.
    match name {
        "minecraft:player" => GeometryModel::PlayerPose,
        "minecraft:cow" | "minecraft:pig" | "minecraft:zombie" => GeometryModel::AgeablePose,
        _ => GeometryModel::NotApplicable,
    }
}

pub(super) fn identity_uses_profile(name: &str) -> bool {
    // Original teams() holder selection, after requiring input.uuid.
    name == "minecraft:player"
}
