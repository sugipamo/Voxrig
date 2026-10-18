//! Option B declarations. Production reads capability, packet, semantics and locality choices.
//!
//! Capability and finite packet proofs retain step 1's meaning. Step 6 adds
//! policy PassThrough, distinct from evidence, for unqualified contact/callbacks.
//! Selectors name existing code; no arithmetic or dynamic proof lives here.
use super::key::{Effect, EntityType, OwnerLane, PacketKey};
use crate::MinecraftVersion;

// Keep ordinal inventories and variants in one declaration, as for the key axes.
macro_rules! ids {
    // Attributes apply only to the inventory, not the enum or future methods.
    ($(#[$inventory_attr:meta])* $name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(usize)]
        pub(super) enum $name { $($variant),+ }
        impl $name {
            $(#[$inventory_attr])*
            pub(super) const ALL: &'static [Self] = &[$(Self::$variant),+];
        }
    };
}
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    Class {
        Qualified,
        ProvenIrrelevant,
        BoundedUnknown,
        Unknown
    }
);
ids!(ProfileId {
    Absent,
    Unknown,
    Bounded,
    Irrelevant,
    Living,
    Chicken,
    Player,
    Sheep,
    Spider
});
ids!(PacketPolicyId {
    Refuse,
    LivingCallbacks,
    SheepCallbacks
});
ids!(EvidenceId {
    None,
    CapabilityAudit,
    PacketAudit,
    EnvelopeAudit,
    OwnerOracle
});
ids!(ScenarioSetId {
    None,
    Excluded,
    Living,
    Chicken,
    RemotePlayer,
    SpiderEnvelope
});
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    TickModel {
        NoTickBody,
        InheritedLiving,
        Chicken,
        RemotePlayer
    }
);
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    GeometryModel {
        NotApplicable,
        ContactShape,
        PlayerPose,
        AgeablePose
    }
);
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    MetadataModel {
        NotApplicable,
        OwnedLiving
    }
);
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    TeamModel {
        NotApplicable,
        OwnHolder
    }
);
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    IdentityModel {
        NotApplicable,
        UuidHolder
    }
);
ids!(LocalityId {
    None,
    DefaultSpiderEnvelope
});
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    EnvelopeHandler {
        None,
        DeferredContact
    }
);
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    LaneContract {
        OwnedContact,
        ReceivedGeometry,
        VehicleSampler
    }
);
ids!(
    #[expect(
        dead_code,
        reason = "Retained frozen policy inventory; no current readers"
    )]
    VehicleModel {
        Unknown,
        Noncollidable,
        Rigid,
        OwnedLiving
    }
);

#[derive(Clone, Copy, Debug)]
pub(super) struct EffectPolicy {
    pub walking: Class,
    pub incoming: Class,
    pub solid: Class,
    pub geometry: Class,
    pub metadata: Class,
    pub status: Class,
    pub animation: Class,
}
impl EffectPolicy {
    pub(super) const fn get(self, effect: Effect) -> Class {
        match effect {
            Effect::WalkingPush => self.walking,
            Effect::IncomingPush => self.incoming,
            Effect::SolidCollision => self.solid,
            Effect::Geometry => self.geometry,
            Effect::Metadata => self.metadata,
            Effect::Status => self.status,
            Effect::Animation => self.animation,
        }
    }
    pub(super) const fn modeled(self) -> bool {
        matches!(self.walking, Class::Qualified)
            && matches!(self.incoming, Class::Qualified)
            && matches!(self.solid, Class::ProvenIrrelevant)
            && matches!(self.geometry, Class::Qualified)
            && matches!(self.metadata, Class::Qualified)
    }
}
use Class::{BoundedUnknown as B, ProvenIrrelevant as I, Qualified as Q, Unknown as U};
const UNKNOWN: EffectPolicy = EffectPolicy {
    walking: U,
    incoming: U,
    solid: U,
    geometry: U,
    metadata: U,
    status: U,
    animation: U,
};
const BOUNDED: EffectPolicy = EffectPolicy {
    walking: B,
    ..UNKNOWN
};
const IRRELEVANT: EffectPolicy = EffectPolicy {
    walking: I,
    incoming: I,
    solid: I,
    geometry: U,
    metadata: I,
    status: I,
    animation: I,
};
const LIVING: EffectPolicy = EffectPolicy {
    walking: Q,
    incoming: Q,
    solid: I,
    geometry: Q,
    metadata: Q,
    status: B,
    animation: B,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Semantics {
    pub tick: TickModel,
    pub geometry: GeometryModel,
    pub metadata: MetadataModel,
    pub team: TeamModel,
    pub identity: IdentityModel,
}
const NO_SEMANTICS: Semantics = Semantics {
    tick: TickModel::NoTickBody,
    geometry: GeometryModel::NotApplicable,
    metadata: MetadataModel::NotApplicable,
    team: TeamModel::NotApplicable,
    identity: IdentityModel::NotApplicable,
};
const fn living_semantics(tick: TickModel) -> Semantics {
    Semantics {
        tick,
        geometry: GeometryModel::ContactShape,
        metadata: MetadataModel::OwnedLiving,
        team: TeamModel::OwnHolder,
        identity: IdentityModel::UuidHolder,
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Profile {
    pub id: ProfileId,
    pub effects: EffectPolicy,
    /// These selectors describe owned_contacts, never public geometry/sampling.
    pub owned: Semantics,
    /// Fallback when this owned body has no numerical qualification/locality proof.
    pub unqualified: Decision,
    pub packets: PacketPolicyId,
    pub locality: LocalityId,
    pub evidence: EvidenceId,
    pub scenarios: ScenarioSetId,
}
const fn profile(id: ProfileId, effects: EffectPolicy, evidence: EvidenceId) -> Profile {
    Profile {
        id,
        effects,
        owned: NO_SEMANTICS,
        unqualified: if matches!(id, ProfileId::Absent) {
            Decision::Refuse
        } else {
            Decision::PassThrough
        },
        packets: PacketPolicyId::Refuse,
        locality: LocalityId::None,
        evidence,
        scenarios: ScenarioSetId::None,
    }
}
const fn living(id: ProfileId, tick: TickModel, scenarios: ScenarioSetId) -> Profile {
    Profile {
        owned: living_semantics(tick),
        unqualified: Decision::Refuse,
        packets: PacketPolicyId::LivingCallbacks,
        scenarios,
        ..profile(id, LIVING, EvidenceId::CapabilityAudit)
    }
}
pub(super) const PROFILES: &[Profile] = &[
    profile(ProfileId::Absent, UNKNOWN, EvidenceId::None),
    profile(ProfileId::Unknown, UNKNOWN, EvidenceId::None),
    profile(ProfileId::Bounded, BOUNDED, EvidenceId::CapabilityAudit),
    Profile {
        scenarios: ScenarioSetId::Excluded,
        ..profile(
            ProfileId::Irrelevant,
            IRRELEVANT,
            EvidenceId::CapabilityAudit,
        )
    },
    living(
        ProfileId::Living,
        TickModel::InheritedLiving,
        ScenarioSetId::Living,
    ),
    living(
        ProfileId::Chicken,
        TickModel::Chicken,
        ScenarioSetId::Chicken,
    ),
    living(
        ProfileId::Player,
        TickModel::RemotePlayer,
        ScenarioSetId::RemotePlayer,
    ),
    Profile {
        packets: PacketPolicyId::SheepCallbacks,
        ..living(
            ProfileId::Sheep,
            TickModel::InheritedLiving,
            ScenarioSetId::Living,
        )
    },
    Profile {
        locality: LocalityId::DefaultSpiderEnvelope,
        scenarios: ScenarioSetId::SpiderEnvelope,
        ..profile(ProfileId::Spider, BOUNDED, EvidenceId::EnvelopeAudit)
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Decision {
    Refuse,
    /// Policy omission, not a claim that the original effect is harmless.
    PassThrough,
    Irrelevant(EvidenceId),
}
#[derive(Clone, Copy, Debug)]
pub(super) struct PacketRule {
    pub packet: PacketKey,
    pub effects: &'static [Effect],
    pub decision: Decision,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct PacketPolicy {
    pub id: PacketPolicyId,
    pub default: Decision,
    pub unproven_callbacks: Decision,
    pub exact: &'static [PacketRule],
}
const fn callback(packet: PacketKey) -> PacketRule {
    PacketRule {
        packet,
        effects: Effect::ALL,
        decision: Decision::Irrelevant(EvidenceId::PacketAudit),
    }
}
pub(super) const PACKET_POLICIES: &[PacketPolicy] = &[
    PacketPolicy {
        id: PacketPolicyId::Refuse,
        default: Decision::Refuse,
        unproven_callbacks: Decision::PassThrough,
        exact: &[],
    },
    PacketPolicy {
        id: PacketPolicyId::LivingCallbacks,
        default: Decision::Refuse,
        unproven_callbacks: Decision::PassThrough,
        exact: &[
            callback(PacketKey::Status(35)),
            callback(PacketKey::Animation(4)),
            callback(PacketKey::Animation(5)),
        ],
    },
    PacketPolicy {
        id: PacketPolicyId::SheepCallbacks,
        default: Decision::Refuse,
        unproven_callbacks: Decision::PassThrough,
        exact: &[
            callback(PacketKey::Status(35)),
            callback(PacketKey::Status(10)),
            callback(PacketKey::Animation(4)),
            callback(PacketKey::Animation(5)),
        ],
    },
];

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalityProfile {
    pub id: LocalityId,
    pub handler: EnvelopeHandler,
    pub lanes: &'static [OwnerLane],
    pub effects: &'static [Effect],
    pub requires_pose_and_dimensions: bool,
    pub requires_motion_receipts: bool,
    pub requires_lifetime: bool,
    pub requires_peer_and_query_proof: bool,
    /// Conditional on matching lifetime/affected passenger membership. Motion
    /// may also revoke if packet provenance/codec application fails.
    pub revokes: &'static [PacketKey],
    pub evidence: EvidenceId,
}
const SPIDER_REVOKES: [PacketKey; 516] = {
    let mut keys = [PacketKey::Metadata; 516];
    keys[1] = PacketKey::Equipment;
    keys[2] = PacketKey::Scale;
    let mut i = 0;
    while i < 256 {
        keys[3 + i] = PacketKey::Status((i as i16 - 128) as i8);
        keys[259 + i] = PacketKey::Animation(i as u8);
        i += 1;
    }
    keys[515] = PacketKey::Passengers;
    keys
};
pub(super) const LOCALITY_PROFILES: &[LocalityProfile] = &[
    LocalityProfile {
        id: LocalityId::None,
        handler: EnvelopeHandler::None,
        lanes: &[],
        effects: &[],
        requires_pose_and_dimensions: false,
        requires_motion_receipts: false,
        requires_lifetime: false,
        requires_peer_and_query_proof: false,
        revokes: &[],
        evidence: EvidenceId::None,
    },
    LocalityProfile {
        id: LocalityId::DefaultSpiderEnvelope,
        handler: EnvelopeHandler::DeferredContact,
        lanes: &[
            OwnerLane::OwnedWalking,
            OwnerLane::OwnedGlide,
            OwnerLane::OwnedBoat,
        ],
        effects: &[
            Effect::WalkingPush,
            Effect::IncomingPush,
            Effect::SolidCollision,
        ],
        requires_pose_and_dimensions: true,
        requires_motion_receipts: true,
        requires_lifetime: true,
        requires_peer_and_query_proof: true,
        revokes: &SPIDER_REVOKES,
        evidence: EvidenceId::EnvelopeAudit,
    },
];

#[derive(Clone, Copy, Debug)]
pub(super) struct EvidenceRequirement {
    pub id: EvidenceId,
    pub artifact: Option<&'static str>,
}
pub(super) const EVIDENCE_REQUIREMENTS: &[EvidenceRequirement] = &[
    EvidenceRequirement {
        id: EvidenceId::None,
        artifact: None,
    },
    EvidenceRequirement {
        id: EvidenceId::CapabilityAudit,
        artifact: Some("docs/evidence/contact-capabilities-20261015.json"),
    },
    EvidenceRequirement {
        id: EvidenceId::PacketAudit,
        artifact: Some("data/client_api/contact_packet_audit.json"),
    },
    EvidenceRequirement {
        id: EvidenceId::EnvelopeAudit,
        artifact: Some("docs/evidence/contact-envelopes-20261015.json"),
    },
    EvidenceRequirement {
        id: EvidenceId::OwnerOracle,
        artifact: Some("data/client_api/entity_owner_oracle.json.gz"),
    },
];
#[derive(Clone, Copy, Debug)]
pub(super) struct ScenarioProfile {
    pub id: ScenarioSetId,
    pub evidence: EvidenceId,
}
pub(super) const SCENARIO_PROFILES: &[ScenarioProfile] = &[
    ScenarioProfile {
        id: ScenarioSetId::None,
        evidence: EvidenceId::None,
    },
    ScenarioProfile {
        id: ScenarioSetId::Excluded,
        evidence: EvidenceId::OwnerOracle,
    },
    ScenarioProfile {
        id: ScenarioSetId::Living,
        evidence: EvidenceId::OwnerOracle,
    },
    ScenarioProfile {
        id: ScenarioSetId::Chicken,
        evidence: EvidenceId::OwnerOracle,
    },
    ScenarioProfile {
        id: ScenarioSetId::RemotePlayer,
        evidence: EvidenceId::OwnerOracle,
    },
    ScenarioProfile {
        id: ScenarioSetId::SpiderEnvelope,
        evidence: EvidenceId::EnvelopeAudit,
    },
];

/// Lane contracts do not inherit private qualification into public APIs.
pub(super) const LANE_CONTRACTS: [LaneContract; 5] = [
    LaneContract::OwnedContact,
    LaneContract::OwnedContact,
    LaneContract::OwnedContact,
    LaneContract::ReceivedGeometry,
    LaneContract::VehicleSampler,
];
#[derive(Clone, Copy, Debug)]
pub(super) struct LanePolicies {
    pub walking: ProfileId,
    pub glide: ProfileId,
    pub boat: ProfileId,
    pub public_geometry: ProfileId,
    pub vehicle: ProfileId,
}
const fn lanes(
    walking: ProfileId,
    glide: ProfileId,
    boat: ProfileId,
    public_geometry: ProfileId,
    vehicle: ProfileId,
) -> LanePolicies {
    LanePolicies {
        walking,
        glide,
        boat,
        public_geometry,
        vehicle,
    }
}
impl LanePolicies {
    pub(super) const fn get(self, lane: OwnerLane) -> ProfileId {
        match lane {
            OwnerLane::OwnedWalking => self.walking,
            OwnerLane::OwnedGlide => self.glide,
            OwnerLane::OwnedBoat => self.boat,
            OwnerLane::PublicGeometry => self.public_geometry,
            OwnerLane::VehicleSample => self.vehicle,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct EntityPolicy {
    pub entity: EntityType,
    pub versions: [LanePolicies; 2],
    pub public_geometry: GeometryModel,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Frozen vehicle selector read by baseline tests and the contact policy export"
        )
    )]
    pub vehicle: VehicleModel,
}
const fn row(
    entity: EntityType,
    legacy: LanePolicies,
    modern: LanePolicies,
    public_geometry: GeometryModel,
    vehicle: VehicleModel,
) -> EntityPolicy {
    EntityPolicy {
        entity,
        versions: [legacy, modern],
        public_geometry,
        vehicle,
    }
}
use ProfileId::{
    Absent as A, Bounded as Bp, Chicken as C, Irrelevant as Ip, Living as L, Player as P,
    Sheep as S, Spider as D, Unknown as Up,
};
const ABSENT: LanePolicies = lanes(A, A, A, A, A);
// Every version/entity/lane cell is assigned explicitly, including Unknown and
// absent cells. Helpers carry no species lists and consult no ancestry/registry.
pub(super) const ENTITY_POLICIES: &[EntityPolicy] = &[
    row(
        EntityType::AcaciaBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::AcaciaChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Allay,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::AreaEffectCloud,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::Armadillo,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ArmorStand,
        lanes(Ip, Ip, Ip, Ip, Ip),
        lanes(Ip, Ip, Ip, Ip, Ip),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Arrow,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::Axolotl,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::BambooChestRaft,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::BambooRaft,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Bat,
        lanes(Ip, Ip, Ip, Ip, Ip),
        lanes(Ip, Ip, Ip, Ip, Ip),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Bee,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::BirchBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::BirchChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Blaze,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::BlockDisplay,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Boat,
        lanes(Up, Up, Up, Up, Up),
        ABSENT,
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Bogged,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Breeze,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::BreezeWindCharge,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Camel,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::CamelHusk,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Cat,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::CaveSpider,
        lanes(D, D, D, D, D),
        lanes(D, D, D, D, D),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::CherryBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::CherryChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::ChestMinecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Chicken,
        lanes(C, C, C, C, C),
        lanes(C, C, C, C, C),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Cod,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::CommandBlockMinecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::CopperGolem,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Cow,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::AgeablePose,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Creaking,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Creeper,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::DarkOakBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::DarkOakChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Dolphin,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Donkey,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::DragonFireball,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Drowned,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Egg,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::ElderGuardian,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::EndCrystal,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::EnderDragon,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::EnderPearl,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::Enderman,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Endermite,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Evoker,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::EvokerFangs,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ExperienceBottle,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::ExperienceOrb,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::EyeOfEnder,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::FallingBlock,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Fireball,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::FireworkRocket,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::FishingBobber,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Fox,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Frog,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::FurnaceMinecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Ghast,
        lanes(Bp, Bp, Bp, Bp, Bp),
        lanes(Bp, Bp, Bp, Bp, Bp),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Giant,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::GlowItemFrame,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::GlowSquid,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Goat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Guardian,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::HappyGhast,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Hoglin,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::HopperMinecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Horse,
        lanes(Bp, Bp, Bp, Bp, Bp),
        lanes(Bp, Bp, Bp, Bp, Bp),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Husk,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Illusioner,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Interaction,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::IronGolem,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Item,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::ItemDisplay,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ItemFrame,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::JungleBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::JungleChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::LeashKnot,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::LightningBolt,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::LingeringPotion,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Llama,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::LlamaSpit,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::MagmaCube,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::MangroveBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::MangroveChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Mannequin,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Marker,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Minecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Mooshroom,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Mule,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Nautilus,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::OakBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::OakChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Ocelot,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::OminousItemSpawner,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Painting,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::PaleOakBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::PaleOakChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Panda,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Parched,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Parrot,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Phantom,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Pig,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::AgeablePose,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Piglin,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::PiglinBrute,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Pillager,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Player,
        lanes(P, P, P, P, P),
        lanes(P, P, P, P, P),
        GeometryModel::PlayerPose,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::PolarBear,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Potion,
        lanes(Up, Up, Up, Up, Up),
        ABSENT,
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::Pufferfish,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Rabbit,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Ravager,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Salmon,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Sheep,
        lanes(S, S, S, S, S),
        lanes(S, S, S, S, S),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Shulker,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ShulkerBullet,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Silverfish,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Skeleton,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::SkeletonHorse,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Slime,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::SmallFireball,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Sniffer,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::SnowGolem,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Snowball,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::SpawnerMinecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::SpectralArrow,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::Spider,
        lanes(D, D, D, D, D),
        lanes(D, D, D, D, D),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::SplashPotion,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::SpruceBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::SpruceChestBoat,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::Squid,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Stray,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Strider,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Tadpole,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::TextDisplay,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Tnt,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::TntMinecart,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Rigid,
    ),
    row(
        EntityType::TraderLlama,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Trident,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Noncollidable,
    ),
    row(
        EntityType::TropicalFish,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Turtle,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Vex,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Villager,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Vindicator,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::WanderingTrader,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::NotApplicable,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::Warden,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::WindCharge,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Witch,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Wither,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::WitherSkeleton,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::WitherSkull,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Wolf,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Zoglin,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::Zombie,
        lanes(L, L, L, L, L),
        lanes(L, L, L, L, L),
        GeometryModel::AgeablePose,
        VehicleModel::OwnedLiving,
    ),
    row(
        EntityType::ZombieHorse,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ZombieNautilus,
        ABSENT,
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ZombieVillager,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
    row(
        EntityType::ZombifiedPiglin,
        lanes(Up, Up, Up, Up, Up),
        lanes(Up, Up, Up, Up, Up),
        GeometryModel::NotApplicable,
        VehicleModel::Unknown,
    ),
];

pub(super) const fn version_index(version: MinecraftVersion) -> usize {
    match version {
        MinecraftVersion::Java1_16_1 => 0,
        MinecraftVersion::Java1_21_11 => 1,
    }
}
pub(super) const fn lookup(
    version: MinecraftVersion,
    entity: EntityType,
    lane: OwnerLane,
) -> Profile {
    let row = ENTITY_POLICIES[entity.ordinal()];
    PROFILES[row.versions[version_index(version)].get(lane) as usize]
}
pub(super) const fn semantics(
    version: MinecraftVersion,
    entity: EntityType,
    lane: OwnerLane,
) -> Semantics {
    let row = ENTITY_POLICIES[entity.ordinal()];
    if entity.native_id(version).is_none() {
        return NO_SEMANTICS;
    }
    match LANE_CONTRACTS[lane.ordinal()] {
        LaneContract::OwnedContact => lookup(version, entity, lane).owned,
        LaneContract::ReceivedGeometry => Semantics {
            geometry: row.public_geometry,
            ..NO_SEMANTICS
        },
        LaneContract::VehicleSampler => NO_SEMANTICS,
    }
}
