//! Static choices only; receipt, lifetime, sweep and peer proofs stay in their handlers.
use super::{
    key::{Effect, EntityType, OwnerLane, PacketKey},
    table::{self, Class, EnvelopeHandler, LocalityProfile, Profile},
};
use crate::MinecraftVersion;

fn selected(version: MinecraftVersion, name: &str, lane: OwnerLane) -> Profile {
    EntityType::from_name(version, name).map_or(
        table::PROFILES[table::ProfileId::Unknown as usize],
        |entity| table::lookup(version, entity, lane),
    )
}

pub(in crate::client::entity) fn owned_lane(boat: bool, glide: bool) -> OwnerLane {
    if boat {
        OwnerLane::OwnedBoat
    } else if glide {
        OwnerLane::OwnedGlide
    } else {
        OwnerLane::OwnedWalking
    }
}

pub(in crate::client::entity) fn modeled(
    version: MinecraftVersion,
    name: &str,
    lane: OwnerLane,
) -> bool {
    selected(version, name, lane).effects.modeled()
}

#[derive(Clone, Copy, Debug)]
pub(in crate::client::entity) struct DeferredPolicy {
    walking: Profile,
    boat: Profile,
}
impl DeferredPolicy {
    pub(in crate::client::entity) fn new(version: MinecraftVersion, name: &str) -> Self {
        Self {
            walking: selected(version, name, OwnerLane::OwnedWalking),
            boat: selected(version, name, OwnerLane::OwnedBoat),
        }
    }
    fn profile(self, boat: bool) -> Profile {
        if boat { self.boat } else { self.walking }
    }
    fn locality(self, boat: bool) -> &'static LocalityProfile {
        &table::LOCALITY_PROFILES[self.profile(boat).locality as usize]
    }
    pub(in crate::client::entity) fn relevant(self, boat: bool) -> bool {
        let effects = self.profile(boat).effects;
        effects.solid != Class::ProvenIrrelevant
            || if boat {
                effects.incoming != Class::ProvenIrrelevant
            } else {
                effects.walking != Class::ProvenIrrelevant
            }
    }
    pub(in crate::client::entity) fn envelope(self, boat: bool) -> bool {
        let locality = self.locality(boat);
        match locality.handler {
            EnvelopeHandler::None => false,
            EnvelopeHandler::DeferredContact => {
                locality.lanes.contains(&owned_lane(boat, false))
                    && locality.effects.contains(&Effect::SolidCollision)
                    && locality.effects.contains(&if boat {
                        Effect::IncomingPush
                    } else {
                        Effect::WalkingPush
                    })
            }
        }
    }
    pub(in crate::client::entity) fn revokes(self, packet: PacketKey) -> bool {
        self.locality(false).revokes.contains(&packet)
    }
}
