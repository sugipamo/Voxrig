//! Const guarantees cover the *factored Cartesian product*: a unique complete
//! entity/version/lane assignment times a complete effect/packet expansion.
//! Each shared packet profile is expanded once, rather than re-evaluating the
//! same 3,647 cells for each of 1,320 present assignments. No giant product is
//! retained in the executable. Registry JSON and semantic parity need tests.
use super::{key::*, table::*};

pub(super) type PacketExpansion = [[Decision; PacketKey::COUNT]; Effect::ALL.len()];

pub(super) const fn expand_packets(policy: PacketPolicy, evidence_count: usize) -> PacketExpansion {
    assert!(
        matches!(policy.default, Decision::Refuse),
        "packet default must refuse"
    );
    let mut result = [[policy.default; PacketKey::COUNT]; Effect::ALL.len()];
    assert!(
        matches!(policy.unproven_callbacks, Decision::PassThrough),
        "callback fallback must pass through"
    );
    let mut p = 0;
    while p < PacketKey::COUNT {
        if matches!(
            PacketKey::ALL[p],
            PacketKey::Status(_) | PacketKey::Animation(_)
        ) {
            let mut e = 0;
            while e < Effect::ALL.len() {
                result[e][p] = policy.unproven_callbacks;
                e += 1;
            }
        }
        p += 1;
    }
    let mut seen = [[false; PacketKey::COUNT]; Effect::ALL.len()];
    let mut r = 0;
    while r < policy.exact.len() {
        let rule = policy.exact[r];
        assert!(!rule.effects.is_empty(), "empty packet effect set");
        match rule.decision {
            Decision::Refuse | Decision::PassThrough => panic!("exact rules must carry proof"),
            Decision::Irrelevant(id) => {
                assert!((id as usize) < evidence_count, "packet evidence reference");
                assert!(!matches!(id, EvidenceId::None), "packet proof missing");
            }
        }
        let mut e = 0;
        while e < rule.effects.len() {
            let effect = rule.effects[e].ordinal();
            let packet = rule.packet.ordinal();
            // Equal duplicate decisions are errors too; there is no precedence.
            assert!(!seen[effect][packet], "duplicate expanded packet rule");
            seen[effect][packet] = true;
            result[effect][packet] = rule.decision;
            e += 1;
        }
        r += 1;
    }
    // All 256 signed Status and 256 unsigned Animation values participate,
    // including each explicitly defaulted cell and each non-opcode family.
    let mut e = 0;
    while e < Effect::ALL.len() {
        let mut p = 0;
        while p < PacketKey::COUNT {
            assert!(
                PacketKey::ALL[p].ordinal() == p,
                "packet axis gap/duplicate"
            );
            match result[e][p] {
                Decision::Refuse | Decision::PassThrough => {}
                Decision::Irrelevant(id) => assert!((id as usize) < evidence_count),
            }
            p += 1;
        }
        e += 1;
    }
    result
}

pub(super) const fn validate_policy(
    entities: &[EntityPolicy],
    profiles: &[Profile],
    packets: &[PacketPolicy],
    localities: &[LocalityProfile],
    evidence: &[EvidenceRequirement],
    scenarios: &[ScenarioProfile],
) {
    assert!(
        entities.len() == EntityType::ALL.len(),
        "entity coverage gap"
    );
    assert!(
        profiles.len() == ProfileId::ALL.len(),
        "profile coverage gap"
    );
    assert!(
        packets.len() == PacketPolicyId::ALL.len(),
        "packet profile gap"
    );
    assert!(
        localities.len() == LocalityId::ALL.len(),
        "locality profile gap"
    );
    assert!(
        evidence.len() == EvidenceId::ALL.len(),
        "evidence coverage gap"
    );
    assert!(
        scenarios.len() == ScenarioSetId::ALL.len(),
        "scenario coverage gap"
    );
    assert!(
        LANE_CONTRACTS.len() == OwnerLane::ALL.len(),
        "lane coverage gap"
    );
    let mut i = 0;
    while i < evidence.len() {
        assert!(
            evidence[i].id as usize == i,
            "duplicate/missing evidence ID"
        );
        assert!(
            evidence[i].artifact.is_some() == (i != EvidenceId::None as usize),
            "evidence descriptor"
        );
        i += 1;
    }
    i = 0;
    while i < scenarios.len() {
        assert!(
            scenarios[i].id as usize == i,
            "duplicate/missing scenario ID"
        );
        assert!(
            (scenarios[i].evidence as usize) < evidence.len(),
            "scenario evidence reference"
        );
        assert!(
            matches!(scenarios[i].evidence, EvidenceId::None) == (i == 0),
            "scenario proof missing"
        );
        i += 1;
    }
    i = 0;
    while i < localities.len() {
        let local = localities[i];
        assert!(local.id as usize == i, "duplicate/missing locality ID");
        assert!(
            (local.evidence as usize) < evidence.len(),
            "locality evidence reference"
        );
        let enabled = matches!(local.handler, EnvelopeHandler::DeferredContact);
        assert!(
            enabled == matches!(local.id, LocalityId::DefaultSpiderEnvelope),
            "locality handler"
        );
        assert!(
            enabled == local.requires_pose_and_dimensions,
            "locality pose requirement"
        );
        assert!(
            enabled == local.requires_motion_receipts,
            "locality receipt requirement"
        );
        assert!(
            enabled == local.requires_lifetime,
            "locality lifetime requirement"
        );
        assert!(
            enabled == local.requires_peer_and_query_proof,
            "locality proof requirement"
        );
        assert!(
            enabled != matches!(local.evidence, EvidenceId::None),
            "locality proof missing"
        );
        assert!(
            enabled == !local.lanes.is_empty(),
            "locality lane applicability"
        );
        assert!(
            enabled == !local.effects.is_empty(),
            "locality effect applicability"
        );
        assert!(
            enabled == !local.revokes.is_empty(),
            "locality revocation missing"
        );
        let mut seen_lanes = [false; OwnerLane::ALL.len()];
        let mut j = 0;
        while j < local.lanes.len() {
            let lane = local.lanes[j].ordinal();
            assert!(!seen_lanes[lane], "duplicate locality lane");
            seen_lanes[lane] = true;
            assert!(
                matches!(LANE_CONTRACTS[lane], LaneContract::OwnedContact),
                "locality outside owned lane"
            );
            j += 1;
        }
        let mut seen_effects = [false; Effect::ALL.len()];
        j = 0;
        while j < local.effects.len() {
            let effect = local.effects[j].ordinal();
            assert!(!seen_effects[effect], "duplicate locality effect");
            seen_effects[effect] = true;
            assert!(
                matches!(
                    local.effects[j],
                    Effect::WalkingPush | Effect::IncomingPush | Effect::SolidCollision
                ),
                "locality effect incompatible"
            );
            j += 1;
        }
        let mut seen_packets = [false; PacketKey::COUNT];
        j = 0;
        while j < local.revokes.len() {
            let packet = local.revokes[j].ordinal();
            assert!(!seen_packets[packet], "duplicate locality revocation");
            seen_packets[packet] = true;
            j += 1;
        }
        i += 1;
    }
    i = 0;
    while i < packets.len() {
        assert!(
            packets[i].id as usize == i,
            "duplicate/missing packet profile ID"
        );
        let _ = expand_packets(packets[i], evidence.len());
        i += 1;
    }
    i = 0;
    while i < profiles.len() {
        let profile = profiles[i];
        assert!(profile.id as usize == i, "duplicate/missing profile ID");
        assert!(
            (profile.packets as usize) < packets.len(),
            "packet profile reference"
        );
        assert!(
            (profile.locality as usize) < localities.len(),
            "locality reference"
        );
        assert!(
            (profile.evidence as usize) < evidence.len(),
            "evidence reference"
        );
        assert!(
            (profile.scenarios as usize) < scenarios.len(),
            "scenario reference"
        );
        let modeled = profile.effects.modeled();
        assert!(
            if modeled || matches!(profile.id, ProfileId::Absent) {
                matches!(profile.unqualified, Decision::Refuse)
            } else {
                matches!(profile.unqualified, Decision::PassThrough)
            },
            "unqualified fallback conflicts with numerical/presence contract"
        );
        assert!(
            modeled != matches!(profile.owned.tick, TickModel::NoTickBody),
            "tick/classification conflict"
        );
        assert!(
            modeled == matches!(profile.owned.geometry, GeometryModel::ContactShape),
            "geometry selector conflict"
        );
        if !modeled {
            assert!(
                matches!(profile.owned.geometry, GeometryModel::NotApplicable),
                "public geometry selector in owned profile"
            );
        }
        assert!(
            modeled == matches!(profile.owned.metadata, MetadataModel::OwnedLiving),
            "metadata selector conflict"
        );
        assert!(
            modeled == matches!(profile.owned.team, TeamModel::OwnHolder),
            "team selector conflict"
        );
        assert!(
            modeled == matches!(profile.owned.identity, IdentityModel::UuidHolder),
            "identity selector conflict"
        );
        assert!(
            modeled != matches!(profile.packets, PacketPolicyId::Refuse),
            "packet selector conflict"
        );
        if modeled {
            assert!(
                matches!(profile.effects.status, Class::BoundedUnknown),
                "status proof domain"
            );
            assert!(
                matches!(profile.effects.animation, Class::BoundedUnknown),
                "animation proof domain"
            );
            assert!(
                matches!(profile.locality, LocalityId::None),
                "numerical/envelope conflict"
            );
            assert!(
                matches!(profile.evidence, EvidenceId::CapabilityAudit),
                "numerical evidence"
            );
            assert!(
                matches!(
                    scenarios[profile.scenarios as usize].evidence,
                    EvidenceId::OwnerOracle
                ),
                "numerical scenarios"
            );
        }
        match profile.owned.tick {
            TickModel::NoTickBody => {}
            TickModel::InheritedLiving => assert!(
                matches!(profile.scenarios, ScenarioSetId::Living),
                "living scenario selector"
            ),
            TickModel::Chicken => assert!(
                matches!(profile.scenarios, ScenarioSetId::Chicken),
                "chicken scenario selector"
            ),
            TickModel::RemotePlayer => assert!(
                matches!(profile.scenarios, ScenarioSetId::RemotePlayer),
                "player scenario selector"
            ),
        }
        if !matches!(profile.locality, LocalityId::None) {
            assert!(
                matches!(profile.effects.walking, Class::BoundedUnknown),
                "locality classification"
            );
            assert!(
                matches!(profile.evidence, EvidenceId::EnvelopeAudit),
                "locality evidence"
            );
            assert!(
                matches!(profile.scenarios, ScenarioSetId::SpiderEnvelope),
                "locality scenarios"
            );
        }
        let mut e = 0;
        while e < Effect::ALL.len() {
            assert!(Effect::ALL[e].ordinal() == e, "effect axis gap/duplicate");
            if !matches!(profile.effects.get(Effect::ALL[e]), Class::Unknown) {
                assert!(
                    !matches!(profile.evidence, EvidenceId::None),
                    "classification proof missing"
                );
            }
            e += 1;
        }
        i += 1;
    }
    i = 0;
    let mut expanded_present = 0;
    let mut expanded_absent = 0;
    while i < entities.len() {
        let row = entities[i];
        // Ordered dense assignments detect both equal duplicates and gaps. The
        // two named version columns and five named lane fields cannot be omitted.
        assert!(row.entity.ordinal() == i, "duplicate/missing entity key");
        let mut v = 0;
        while v < VERSIONS.len() {
            let present = row.entity.native_id(VERSIONS[v]).is_some();
            let mut l = 0;
            while l < OwnerLane::ALL.len() {
                assert!(OwnerLane::ALL[l].ordinal() == l, "lane axis gap/duplicate");
                let id = row.versions[v].get(OwnerLane::ALL[l]);
                assert!((id as usize) < profiles.len(), "entity profile reference");
                assert!(
                    present != matches!(id, ProfileId::Absent),
                    "presence/coverage conflict"
                );
                let profile = profiles[id as usize];
                match profile.owned.tick {
                    TickModel::Chicken => assert!(
                        matches!(row.entity, EntityType::Chicken),
                        "chicken species conflict"
                    ),
                    TickModel::RemotePlayer => assert!(
                        matches!(row.entity, EntityType::Player),
                        "player species conflict"
                    ),
                    TickModel::NoTickBody | TickModel::InheritedLiving => {}
                }
                if matches!(profile.packets, PacketPolicyId::SheepCallbacks) {
                    assert!(
                        matches!(row.entity, EntityType::Sheep),
                        "sheep packet conflict"
                    );
                }
                if matches!(profile.locality, LocalityId::DefaultSpiderEnvelope) {
                    assert!(
                        matches!(row.entity, EntityType::Spider | EntityType::CaveSpider),
                        "spider species conflict"
                    );
                }
                let mut e = 0;
                while e < Effect::ALL.len() {
                    let class = profile.effects.get(Effect::ALL[e]);
                    if !present || matches!(id, ProfileId::Unknown) {
                        assert!(
                            matches!(class, Class::Unknown),
                            "unknown/absent classification conflict"
                        );
                    }
                    if present {
                        expanded_present += PacketKey::COUNT;
                    } else {
                        expanded_absent += PacketKey::COUNT;
                    }
                    e += 1;
                }
                // Each profile has all seven named effects, each packet profile
                // has all 7 x 521 cells above. Thus every assigned Cartesian
                // product key has exactly one class and one packet decision.
                l += 1;
            }
            v += 1;
        }
        match row.public_geometry {
            GeometryModel::NotApplicable => {}
            GeometryModel::PlayerPose => assert!(
                matches!(row.entity, EntityType::Player),
                "public player geometry"
            ),
            GeometryModel::AgeablePose => assert!(
                matches!(
                    row.entity,
                    EntityType::Cow | EntityType::Pig | EntityType::Zombie
                ),
                "public ageable geometry"
            ),
            GeometryModel::ContactShape => panic!("private shape in public contract"),
        }
        i += 1;
    }
    assert!(
        expanded_present + expanded_absent
            == VERSIONS.len()
                * EntityType::ALL.len()
                * OwnerLane::ALL.len()
                * Effect::ALL.len()
                * PacketKey::COUNT,
        "Cartesian coverage gap"
    );
}

const _: () = validate_policy(
    ENTITY_POLICIES,
    PROFILES,
    PACKET_POLICIES,
    LOCALITY_PROFILES,
    EVIDENCE_REQUIREMENTS,
    SCENARIO_PROFILES,
);

#[cfg(test)]
// The golden and test-only harmless_packet expose booleans, not proof IDs.
// Keep this test cache small; the validator above checks the full decisions.
pub(super) const EXPANDED_PACKETS: [[[bool; PacketKey::COUNT]; Effect::ALL.len()];
    PacketPolicyId::ALL.len()] = {
    let mut expanded = [[[false; PacketKey::COUNT]; Effect::ALL.len()]; PacketPolicyId::ALL.len()];
    let mut i = 0;
    while i < PACKET_POLICIES.len() {
        let decisions = expand_packets(PACKET_POLICIES[i], EVIDENCE_REQUIREMENTS.len());
        let mut e = 0;
        while e < Effect::ALL.len() {
            let mut p = 0;
            while p < PacketKey::COUNT {
                expanded[i][e][p] = matches!(decisions[e][p], Decision::Irrelevant(_));
                p += 1;
            }
            e += 1;
        }
        i += 1;
    }
    expanded
};

#[cfg(test)]
mod tests {
    use super::*;
    fn validate(entities: &[EntityPolicy], profiles: &[Profile], packets: &[PacketPolicy]) {
        validate_policy(
            entities,
            profiles,
            packets,
            LOCALITY_PROFILES,
            EVIDENCE_REQUIREMENTS,
            SCENARIO_PROFILES,
        );
    }
    #[test]
    fn rejects_gaps_duplicates_conflicts_and_invalid_references() {
        assert!(
            std::panic::catch_unwind(|| validate(&ENTITY_POLICIES[1..], PROFILES, PACKET_POLICIES))
                .is_err()
        );
        let mut entities = ENTITY_POLICIES.to_vec();
        entities[1] = entities[0];
        assert!(
            std::panic::catch_unwind(|| validate(&entities, PROFILES, PACKET_POLICIES)).is_err()
        );
        let mut profiles = PROFILES.to_vec();
        profiles[ProfileId::Living as usize].owned.team = TeamModel::NotApplicable;
        assert!(
            std::panic::catch_unwind(|| validate(ENTITY_POLICIES, &profiles, PACKET_POLICIES))
                .is_err()
        );
        let mut profiles = PROFILES.to_vec();
        profiles[ProfileId::Bounded as usize].owned.geometry = GeometryModel::AgeablePose;
        assert!(
            std::panic::catch_unwind(|| validate(ENTITY_POLICIES, &profiles, PACKET_POLICIES))
                .is_err()
        );
        // A typed ID can still refer to a missing descriptor in a bad table.
        assert!(
            std::panic::catch_unwind(|| validate_policy(
                ENTITY_POLICIES,
                PROFILES,
                PACKET_POLICIES,
                LOCALITY_PROFILES,
                &EVIDENCE_REQUIREMENTS[..2],
                SCENARIO_PROFILES
            ))
            .is_err()
        );
        for id in [
            ProfileId::Unknown,
            ProfileId::Spider,
            ProfileId::Living,
            ProfileId::Absent,
        ] {
            let mut profiles = PROFILES.to_vec();
            profiles[id as usize].unqualified =
                if profiles[id as usize].unqualified == Decision::Refuse {
                    Decision::PassThrough
                } else {
                    Decision::Refuse
                };
            assert!(
                std::panic::catch_unwind(|| validate(ENTITY_POLICIES, &profiles, PACKET_POLICIES))
                    .is_err()
            );
        }
        let mut packets = PACKET_POLICIES.to_vec();
        packets[1].unproven_callbacks = Decision::Refuse;
        assert!(
            std::panic::catch_unwind(|| validate(ENTITY_POLICIES, PROFILES, &packets)).is_err()
        );
        const DUPLICATE: PacketRule = PacketRule {
            packet: PacketKey::Status(35),
            effects: Effect::ALL,
            decision: Decision::Irrelevant(EvidenceId::PacketAudit),
        };
        let mut packets = PACKET_POLICIES.to_vec();
        packets[1].exact = &[DUPLICATE, DUPLICATE];
        assert!(
            std::panic::catch_unwind(|| validate(ENTITY_POLICIES, PROFILES, &packets)).is_err()
        );
        // Conflict after effect-set expansion, even when the proof differs.
        const CONFLICT: PacketRule = PacketRule {
            packet: PacketKey::Status(35),
            effects: &[Effect::Status],
            decision: Decision::Irrelevant(EvidenceId::CapabilityAudit),
        };
        packets[1].exact = &[DUPLICATE, CONFLICT];
        assert!(
            std::panic::catch_unwind(|| validate(ENTITY_POLICIES, PROFILES, &packets)).is_err()
        );
    }
}
