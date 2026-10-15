//! Thin adapters for existing handlers. Callers retain admission and usage gates.
use super::{
    key::{EntityType, OwnerLane},
    table::{self, GeometryModel, IdentityModel, MetadataModel, TeamModel, TickModel},
};
use crate::{
    MinecraftVersion, Result,
    client::{
        entity::{EntityGeometryModel, EntityPose, contact_tick::ContactDispatch},
        ui::{TeamCollision, TeamsObservation},
    },
};

fn selected(version: MinecraftVersion, name: &str, lane: OwnerLane) -> table::Semantics {
    EntityType::from_name(version, name).map_or(
        table::PROFILES[table::ProfileId::Unknown as usize].owned,
        |entity| table::semantics(version, entity, lane),
    )
}

// These shared selectors observe the OwnedWalking column, as capabilities do.
// Walking/glide/boat eligibility remains the caller's existing responsibility.
fn owned(version: MinecraftVersion, name: &str) -> table::Semantics {
    selected(version, name, OwnerLane::OwnedWalking)
}

pub(in crate::client::entity) fn dispatch(
    version: MinecraftVersion,
    name: &str,
) -> Option<ContactDispatch> {
    match owned(version, name).tick {
        TickModel::NoTickBody => None,
        TickModel::InheritedLiving => Some(ContactDispatch::InheritedLiving),
        TickModel::Chicken => Some(ContactDispatch::Chicken),
        TickModel::RemotePlayer => Some(ContactDispatch::RemotePlayer),
    }
}

pub(in crate::client::entity) fn shape(
    version: MinecraftVersion,
    name: &str,
    pose: EntityPose,
    baby: f32,
    scale: f32,
) -> (f32, f32) {
    match owned(version, name).geometry {
        GeometryModel::ContactShape => {
            crate::client::entity::geometry::shape(version, name, pose, baby, scale)
        }
        GeometryModel::NotApplicable | GeometryModel::PlayerPose | GeometryModel::AgeablePose => {
            unreachable!("owned shape requires an admitted contact body")
        }
    }
}

pub(in crate::client::entity) fn public_geometry(
    version: MinecraftVersion,
    name: &str,
) -> Option<EntityGeometryModel> {
    match selected(version, name, OwnerLane::PublicGeometry).geometry {
        GeometryModel::PlayerPose => Some(EntityGeometryModel::PlayerPose),
        GeometryModel::AgeablePose => Some(EntityGeometryModel::AgeablePose),
        GeometryModel::NotApplicable => None,
        GeometryModel::ContactShape => unreachable!("private shape in public geometry contract"),
    }
}

pub(in crate::client::entity) struct MetadataSelector(MetadataModel);

pub(in crate::client::entity) fn metadata(
    version: MinecraftVersion,
    name: &str,
) -> MetadataSelector {
    MetadataSelector(owned(version, name).metadata)
}

impl MetadataSelector {
    pub(in crate::client::entity) fn apply(
        self,
        apply_living: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        match self.0 {
            MetadataModel::OwnedLiving => apply_living(),
            MetadataModel::NotApplicable => {
                unreachable!("metadata requires an admitted contact body")
            }
        }
    }
}

pub(in crate::client::entity) fn identity_uses_profile(
    version: MinecraftVersion,
    name: &str,
) -> bool {
    let selectors = owned(version, name);
    match selectors.identity {
        // RemotePlayer resolves the retained UUID through the registered profile;
        // other living bodies format that UUID. Keep both algorithms in the reducer.
        IdentityModel::UuidHolder => match selectors.tick {
            TickModel::RemotePlayer => true,
            TickModel::InheritedLiving | TickModel::Chicken => false,
            TickModel::NoTickBody => unreachable!("identity requires an admitted contact body"),
        },
        IdentityModel::NotApplicable => unreachable!("identity requires an admitted contact body"),
    }
}

pub(in crate::client::entity) fn team(
    version: MinecraftVersion,
    name: &str,
    teams: &TeamsObservation,
    holder: &str,
) -> Option<(String, TeamCollision)> {
    match owned(version, name).team {
        TeamModel::OwnHolder => crate::client::entity::owned_contacts::team(teams, holder),
        TeamModel::NotApplicable => unreachable!("team requires an admitted contact body"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::entity::contact_policy::{frozen_selectors, key::VERSIONS};

    fn teams(version: MinecraftVersion) -> TeamsObservation {
        use crate::client::{ObservedValue, SessionStamp, ValueSource, ui::*};
        let received = |value| ObservedValue {
            value,
            source: ValueSource::Received { sequence: 1 },
        };
        TeamsObservation {
            session: SessionStamp {
                version,
                connection_id: 9,
                world_generation: 3,
            },
            receive_sequence: 1,
            context_reset_sequence: None,
            last_update_sequence: Some(1),
            teams: vec![ReceivedTeam {
                name: "frozen-team".into(),
                parameters: ObservedValue {
                    source: ValueSource::Received { sequence: 1 },
                    value: TeamParameters {
                        display: UiText::Unavailable,
                        friendly_flags: 0,
                        visibility: TeamVisibility::Always,
                        collision: TeamCollision::PushOwnTeam,
                        color: TeamColor::Reset,
                        prefix: UiText::Unavailable,
                        suffix: UiText::Unavailable,
                    },
                },
                members: vec![received("holder".into())],
                created_sequence: 1,
                last_members_update_sequence: Some(1),
            }],
        }
    }

    #[test]
    fn selectors_match_independent_old_source_for_every_species() {
        let mut present = 0;
        for version in VERSIONS {
            for &entity in EntityType::ALL {
                let name = entity.name();
                let old = frozen_selectors::owned(version, name);
                for lane in [
                    OwnerLane::OwnedWalking,
                    OwnerLane::OwnedGlide,
                    OwnerLane::OwnedBoat,
                ] {
                    assert_eq!(
                        selected(version, name, lane),
                        old,
                        "{version:?}/{name}/{lane:?}"
                    );
                }
                let expected_dispatch = match old.tick {
                    TickModel::NoTickBody => None,
                    TickModel::InheritedLiving => Some(ContactDispatch::InheritedLiving),
                    TickModel::Chicken => Some(ContactDispatch::Chicken),
                    TickModel::RemotePlayer => Some(ContactDispatch::RemotePlayer),
                };
                assert_eq!(dispatch(version, name), expected_dispatch);
                if entity.native_id(version).is_none() {
                    assert_eq!(public_geometry(version, name), None);
                    continue;
                }
                present += 1;
                let public = selected(version, name, OwnerLane::PublicGeometry);
                assert_eq!(public.geometry, frozen_selectors::public_geometry(name));
                let actual_public =
                    public_geometry(version, name).map_or(GeometryModel::NotApplicable, |model| {
                        match model {
                            EntityGeometryModel::PlayerPose => GeometryModel::PlayerPose,
                            EntityGeometryModel::AgeablePose => GeometryModel::AgeablePose,
                        }
                    });
                assert_eq!(actual_public, frozen_selectors::public_geometry(name));
                if expected_dispatch.is_none() {
                    continue;
                }
                assert_eq!(
                    identity_uses_profile(version, name),
                    frozen_selectors::identity_uses_profile(name)
                );
                let teams = teams(version);
                assert_eq!(
                    team(version, name, &teams, "holder"),
                    Some(("frozen-team".into(), TeamCollision::PushOwnTeam))
                );
                assert_eq!(team(version, name, &teams, "missing"), None);
                let mut applied = 0;
                metadata(version, name)
                    .apply(|| {
                        applied += 1;
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(applied, 1);
                let error = metadata(version, name)
                    .apply(|| {
                        Err(crate::Error::new(
                            crate::ErrorKind::Unsupported,
                            anyhow::anyhow!("frozen metadata sentinel"),
                        ))
                    })
                    .unwrap_err();
                assert!(error.to_string().contains("frozen metadata sentinel"));
                // Every native pose and representative float factors still call
                // the original shape arithmetic, including its multiplication order.
                for pose in (0..18).filter_map(|id| EntityPose::from_native(version, id)) {
                    for baby in [0.5, 1.] {
                        for scale in [0.0625, 0.7, 1., 16.] {
                            let actual = shape(version, name, pose, baby, scale);
                            let old = crate::client::entity::geometry::shape(
                                version, name, pose, baby, scale,
                            );
                            assert_eq!(
                                (actual.0.to_bits(), actual.1.to_bits()),
                                (old.0.to_bits(), old.1.to_bits())
                            );
                        }
                    }
                }
            }
            for name in ["", "cow", "minecraft:missing", "other:cow", "minecraft:Cow"] {
                assert_eq!(dispatch(version, name), None);
                assert_eq!(public_geometry(version, name), None);
            }
        }
        assert_eq!(present, 264);
    }
}
