//! Explicit contact selection and ordered inherited living push calls.
use super::{LivingPairPushInput, LivingPairPushModel, LivingPairPushUnavailable, LivingPushBody};
use crate::{
    MinecraftVersion,
    client::{Aabb, ui::TeamCollision},
};

/// Maximum explicit candidates in one local query; larger inputs are refused.
pub const MAX_LIVING_CONTACT_CANDIDATES: usize = 32;

/// Model-time inputs for the qualified inherited living routine.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingContactBody {
    /// Explicit model position, velocity and original push predicates.
    pub push: LivingPushBody,
    /// Model-time box, not a last received target's current hitbox.
    pub bounds: Aabb,
    /// Original isSleeping predicate, including sleeping-position state.
    pub sleeping: bool,
    /// This body's legacy pushthrough; modern requires zero.
    pub legacy_pushthrough: f32,
    /// Model-time team rule; None explicitly means no team.
    pub team: Option<TeamCollision>,
}

/// Candidate identity class at the explicitly chosen client model step.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LivingContactPlayer {
    /// Original Player.isLocalPlayer is true.
    LocalPlayer,
    /// Player whose original isLocalPlayer is false.
    RemotePlayer,
    /// A qualified nonplayer living body.
    NonPlayer,
}

/// One candidate in caller-supplied query order. No live target is constructed.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingContactCandidate {
    /// Explicit candidate model state before this batch.
    pub body: LivingContactBody,
    /// Original identity equality with the querying body.
    pub same_body: bool,
    /// Original candidate isSpectator predicate.
    pub spectator: bool,
    /// Original candidate player/local-player classification.
    pub player: LivingContactPlayer,
    /// Original querying team's isAlliedTo(candidate team).
    pub allied: bool,
    /// Original candidate.isPassengerOfSameVehicle(querying body).
    pub same_vehicle: bool,
}

/// Contact selection outcome. Reasons use the documented model check order;
/// they are not evidence that the native query evaluated these predicates.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingContactDisposition {
    /// Query excludes its own identity.
    SameBody,
    /// Boxes do not strictly intersect, including face/edge-only contact.
    Separated,
    /// Candidate is a spectator.
    Spectator,
    /// Original candidate isPushable is false.
    Unpushable,
    /// Client queries accept only the local player as a candidate.
    NonLocalPlayer,
    /// Either team's collision rule is never.
    TeamNever,
    /// Original pushOwnTeam branch suppresses allied contact.
    AlliedSuppressed,
    /// Original pushOtherTeams branch suppresses nonallied contact.
    OtherTeamSuppressed,
    /// Qualified selector admits this candidate; push may still return early.
    Admitted,
}

/// One candidate's numerical result in original supplied order.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingContactResult {
    /// Zero-based index in the supplied candidate slice.
    pub candidate_index: usize,
    /// Explicit local selection outcome.
    pub disposition: LivingContactDisposition,
    /// Original doPush(other) invokes other.push(this): first is the candidate,
    /// second is the querying body with velocity accumulated from earlier calls.
    pub push: Option<LivingPairPushModel>,
}

/// Bounded local query model, with no packet mutation, send or tick authority.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingContactsModel {
    /// Native rule/arithmetic version.
    pub version: MinecraftVersion,
    /// Explicit client/server selector branch supplied by the caller.
    pub client_side: bool,
    /// Querying body's model velocity after the ordered calls.
    pub querying_velocity: [f64; 3],
    /// One result per explicit candidate, never silently truncated or sorted.
    pub candidates: Vec<LivingContactResult>,
}

/// This batch cannot be calculated; no partial result or mutation is returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingContactsUnavailable {
    /// Candidate count exceeds the fixed query budget.
    TooManyCandidates,
    /// A supplied box/state or calculated result is nonfinite.
    NonFiniteState,
    /// A box has reversed bounds; no implicit normalization is performed.
    InvalidBounds,
    /// Allied was true without both teams present.
    InconsistentTeams,
    /// Modern input contains nonzero legacy-only pushthrough.
    UnsupportedPushthrough,
}

/// Select contacts and evaluate inherited LivingEntity.doPush in the supplied
/// order. Qualified for player/cow/pig/zombie inherited push routines. The
/// caller supplies a complete, unique, original-style candidate order and all
/// model-time predicates. This function neither discovers entities nor assigns
/// receive ordinals to native ticks. No cramming damage, RNG, world query,
/// entity-specific override or selected movement owner is simulated.
///
/// All candidates are validated before any calculation, including excluded
/// candidates. Failure returns no partially calculated batch. Repeat calls
/// represent new model calculations; scheduling/deduplication belongs to the
/// caller. Discard inputs on lifetime/world/history gaps before further steps.
pub fn model_living_contacts(
    version: MinecraftVersion,
    client_side: bool,
    querying: &LivingContactBody,
    candidates: &[LivingContactCandidate],
) -> Result<LivingContactsModel, LivingContactsUnavailable> {
    use LivingContactDisposition as D;
    use LivingContactsUnavailable as U;
    if candidates.len() > MAX_LIVING_CONTACT_CANDIDATES {
        return Err(U::TooManyCandidates);
    }
    for body in std::iter::once(querying).chain(candidates.iter().map(|c| &c.body)) {
        let b = body.bounds;
        if [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
            .iter()
            .chain(body.push.position.iter())
            .chain(body.push.velocity.iter())
            .any(|v| !v.is_finite())
            || !body.legacy_pushthrough.is_finite()
        {
            return Err(U::NonFiniteState);
        }
        if b.min_x > b.max_x || b.min_y > b.max_y || b.min_z > b.max_z {
            return Err(U::InvalidBounds);
        }
        if version == MinecraftVersion::Java1_21_11 && body.legacy_pushthrough != 0. {
            return Err(U::UnsupportedPushthrough);
        }
    }
    if candidates
        .iter()
        .any(|c| c.allied && (querying.team.is_none() || c.body.team.is_none()))
    {
        return Err(U::InconsistentTeams);
    }
    let mut result = LivingContactsModel {
        version,
        client_side,
        querying_velocity: querying.push.velocity,
        candidates: Vec::with_capacity(candidates.len()),
    };
    for (candidate_index, c) in candidates.iter().enumerate() {
        let a = querying.bounds;
        let b = c.body.bounds;
        let qrule = querying.team.unwrap_or(TeamCollision::Always);
        let crule = c.body.team.unwrap_or(TeamCollision::Always);
        let disposition = if c.same_body {
            D::SameBody
        } else if !(a.min_x < b.max_x
            && a.max_x > b.min_x
            && a.min_y < b.max_y
            && a.max_y > b.min_y
            && a.min_z < b.max_z
            && a.max_z > b.min_z)
        {
            D::Separated
        } else if c.spectator {
            D::Spectator
        } else if !c.body.push.pushable {
            D::Unpushable
        } else if client_side && c.player != LivingContactPlayer::LocalPlayer {
            D::NonLocalPlayer
        } else if qrule == TeamCollision::Never || crule == TeamCollision::Never {
            D::TeamNever
        } else if (qrule == TeamCollision::PushOwnTeam || crule == TeamCollision::PushOwnTeam)
            && c.allied
        {
            D::AlliedSuppressed
        } else if (qrule == TeamCollision::PushOtherTeams || crule == TeamCollision::PushOtherTeams)
            && !c.allied
        {
            D::OtherTeamSuppressed
        } else {
            D::Admitted
        };
        let push = if disposition == D::Admitted {
            let mut second = querying.push.clone();
            second.velocity = result.querying_velocity;
            let input = LivingPairPushInput {
                first: c.body.push.clone(),
                second,
                first_sleeping: c.body.sleeping,
                same_vehicle: c.same_vehicle,
                legacy_pushthrough: c.body.legacy_pushthrough,
            };
            let push = super::model_living_pair_push(version, &input).map_err(|e| match e {
                LivingPairPushUnavailable::NonFiniteState => U::NonFiniteState,
                LivingPairPushUnavailable::UnsupportedPushthrough => U::UnsupportedPushthrough,
            })?;
            result.querying_velocity = push.second_velocity;
            Some(push)
        } else {
            None
        };
        result.candidates.push(LivingContactResult {
            candidate_index,
            disposition,
            push,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Read;

    fn bits(v: &serde_json::Value) -> [f64; 3] {
        std::array::from_fn(|i| {
            f64::from_bits(u64::from_str_radix(v[i].as_str().unwrap(), 16).unwrap())
        })
    }
    fn decode(v: &serde_json::Value) -> LivingContactBody {
        let mut b: LivingContactBody = serde_json::from_value(v.clone()).unwrap();
        b.push.position = bits(&v["push"]["position_bits"]);
        b.push.velocity = bits(&v["push"]["velocity_bits"]);
        let f = |i| {
            f64::from_bits(u64::from_str_radix(v["bounds_bits"][i].as_str().unwrap(), 16).unwrap())
        };
        b.bounds = Aabb {
            min_x: f(0),
            min_y: f(1),
            min_z: f(2),
            max_x: f(3),
            max_y: f(4),
            max_z: f(5),
        };
        b
    }
    fn equal_bits(a: [f64; 3], b: &serde_json::Value) {
        assert_eq!(a.map(f64::to_bits), bits(b).map(f64::to_bits));
    }
    #[test]
    fn contacts_and_ordered_reversed_receiver_calls_match_original_methods() {
        let mut json = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_contact_oracle.json.gz")[..],
        )
        .read_to_string(&mut json)
        .unwrap();
        let oracle: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            oracle["runner_sha256"].as_str().unwrap(),
            format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../scripts/run_entity_contact_oracle.py"
                ))
            )
        );
        let mut total = 0;
        for (version, key, harness, jar, mapping) in [
            (
                MinecraftVersion::Java1_16_1,
                "1.16.1",
                &include_bytes!(
                    "../../../scripts/entity_contact_oracle/1.16.1/EntityContactOracle.java"
                )[..],
                "a412fd69db1f81db3f511c1463fd304675244077",
                "11120c39da4df293c4bd020896391fb9ddd6c2ba",
            ),
            (
                MinecraftVersion::Java1_21_11,
                "1.21.11",
                &include_bytes!(
                    "../../../scripts/entity_contact_oracle/1.21.11/EntityContactOracle.java"
                )[..],
                "64bb6d763bed0a9f1d632ec347938594144943ed",
                "5621e9253f05fd57872bbe7f8ddf5f9a7d525955",
            ),
        ] {
            let original = &oracle["versions"][key];
            assert_eq!(original["original_server_jar_sha1"], jar);
            assert_eq!(original["mappings_sha1"], mapping);
            assert_eq!(
                original["harness_sha256"].as_str().unwrap(),
                format!("{:x}", Sha256::digest(harness))
            );
            let rows = original["results"].as_array().unwrap();
            assert_eq!(rows.len(), 300);
            for row in rows {
                let querying = decode(&row["querying"]);
                let candidates: Vec<_> = row["candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| {
                        let mut c: LivingContactCandidate =
                            serde_json::from_value(v.clone()).unwrap();
                        c.body = decode(&v["body"]);
                        c
                    })
                    .collect();
                let model = model_living_contacts(
                    version,
                    row["client_side"].as_bool().unwrap(),
                    &querying,
                    &candidates,
                )
                .unwrap();
                let mut velocity = querying.push.velocity;
                for (i, result) in model.candidates.iter().enumerate() {
                    let expected = &row["results"][i];
                    assert_eq!(result.candidate_index, i);
                    assert_eq!(
                        result.disposition == LivingContactDisposition::Admitted,
                        expected["admitted"].as_bool().unwrap(),
                        "{key} {row}"
                    );
                    if let Some(push) = &result.push {
                        equal_bits(push.first_velocity, &expected["candidate_velocity_bits"]);
                        equal_bits(push.second_velocity, &expected["querying_velocity_bits"]);
                        assert_eq!(
                            push.first_pushed,
                            expected["candidate_pushed"].as_bool().unwrap(),
                            "{key} {row}"
                        );
                        assert_eq!(
                            push.second_pushed,
                            expected["querying_pushed"].as_bool().unwrap(),
                            "{key} {row}"
                        );
                        velocity = push.second_velocity;
                    } else {
                        equal_bits(velocity, &expected["querying_velocity_bits"]);
                    }
                }
                equal_bits(
                    model.querying_velocity,
                    &row["results"][1]["querying_velocity_bits"],
                );
                total += 1;
            }
        }
        assert_eq!(total, 600);
    }

    fn body() -> LivingContactBody {
        LivingContactBody {
            push: LivingPushBody {
                position: [0., 65., 0.],
                velocity: [0.125, -0., 0.25],
                no_physics: false,
                vehicle: false,
                pushable: true,
            },
            bounds: Aabb {
                min_x: -0.5,
                min_y: 65.,
                min_z: -0.5,
                max_x: 0.5,
                max_y: 66.,
                max_z: 0.5,
            },
            sleeping: false,
            legacy_pushthrough: 0.,
            team: None,
        }
    }
    fn candidate() -> LivingContactCandidate {
        let mut b = body();
        b.push.position = [0.4, 65., 0.2];
        LivingContactCandidate {
            body: b,
            same_body: false,
            spectator: false,
            player: LivingContactPlayer::LocalPlayer,
            allied: false,
            same_vehicle: false,
        }
    }
    #[test]
    fn late_invalid_candidates_and_budgets_refuse_whole_batch_without_input_mutation() {
        use LivingContactsUnavailable as U;
        let q = body();
        let first = candidate();
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let check = |late: LivingContactCandidate, expected| {
                let before = q.push.velocity.map(f64::to_bits);
                assert_eq!(
                    model_living_contacts(version, true, &q, &[first.clone(), late]).unwrap_err(),
                    expected
                );
                assert_eq!(q.push.velocity.map(f64::to_bits), before);
            };
            for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut c = candidate();
                c.same_body = true;
                c.body.push.velocity[1] = value;
                check(c, U::NonFiniteState);
                let mut c = candidate();
                c.body.bounds.max_y = value;
                check(c, U::NonFiniteState);
            }
            let mut c = candidate();
            c.body.bounds.min_x = 2.;
            check(c, U::InvalidBounds);
            let mut c = candidate();
            c.allied = true;
            check(c, U::InconsistentTeams);
            assert_eq!(
                model_living_contacts(version, true, &q, &vec![first.clone(); 33]).unwrap_err(),
                U::TooManyCandidates
            );
            assert_eq!(
                model_living_contacts(version, true, &q, &vec![first.clone(); 32])
                    .unwrap()
                    .candidates
                    .len(),
                32
            );
            assert_eq!(
                model_living_contacts(version, true, &q, &[])
                    .unwrap()
                    .querying_velocity
                    .map(f64::to_bits),
                q.push.velocity.map(f64::to_bits)
            );
        }
        let mut c = candidate();
        c.body.legacy_pushthrough = 0.25;
        assert_eq!(
            model_living_contacts(MinecraftVersion::Java1_21_11, true, &q, &[c]).unwrap_err(),
            U::UnsupportedPushthrough
        );
    }
}
