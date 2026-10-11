//! One explicitly admitted local LivingEntity.push call, without contact discovery.
use crate::MinecraftVersion;

/// Explicit local body inputs. None of these flags is inferred from packet
/// targets, health samples, passenger history or default dimensions.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingPushBody {
    /// Model feet position at the caller's chosen simulation step.
    pub position: [f64; 3],
    /// Model velocity before this call, in blocks per tick.
    pub velocity: [f64; 3],
    /// Original noPhysics predicate at this model step.
    pub no_physics: bool,
    /// Original isVehicle predicate (has passengers), not isPassenger.
    pub vehicle: bool,
    /// Original isPushable predicate; used only on Java 1.21.11.
    /// Alive, climbing and spectator rules must be evaluated by the caller.
    pub pushable: bool,
}

/// Inputs to one original-style LivingEntity.push(other) invocation. The
/// caller admits the pair and supplies model-time predicates explicitly.
/// Qualified for the player/cow/pig/zombie inherited routine; entity-specific
/// overrides, team/contact queries and per-tick pair order are outside scope.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingPairPushInput {
    /// Receiver of push(other); order matters for sleeping and legacy pushthrough.
    pub first: LivingPushBody,
    /// Argument of push(other).
    pub second: LivingPushBody,
    /// Original receiver isSleeping predicate.
    pub first_sleeping: bool,
    /// Original isPassengerOfSameVehicle predicate, including nested attachments.
    pub same_vehicle: bool,
    /// Receiver's legacy pushthrough float. Modern has no such field and
    /// requires zero rather than silently discarding a legacy-only input.
    pub legacy_pushthrough: f32,
}

/// Original early return, or a completed pair calculation. Evaluated does
/// not imply either body was pushable, contacted or moved on the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingPairPushDisposition {
    /// Receiver is sleeping.
    Sleeping,
    /// Bodies belong to the same vehicle tree.
    SameVehicle,
    /// Either body has noPhysics.
    NoPhysics,
    /// Maximum horizontal separation is below the original float threshold.
    TooClose,
    /// Horizontal impulse was calculated; individual predicates decide writes.
    Evaluated,
}

/// Fixed-size result of one local call. This is not a received velocity,
/// contact acknowledgement, execution receipt or permission to send movement.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingPairPushModel {
    /// Version whose original arithmetic and predicates were applied.
    pub version: MinecraftVersion,
    /// Explicit inputs, preserved separately from the result.
    pub input: LivingPairPushInput,
    /// Original early return or evaluated pair.
    pub disposition: LivingPairPushDisposition,
    /// First body's model velocity after the call.
    pub first_velocity: [f64; 3],
    /// Second body's model velocity after the call.
    pub second_velocity: [f64; 3],
    /// Whether this call invokes push(double,double,double) on the first body.
    pub first_pushed: bool,
    /// Whether this call invokes push(double,double,double) on the second body.
    pub second_pushed: bool,
}

/// Inputs or arithmetic for which this finite local model is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingPairPushUnavailable {
    /// A position, velocity, reduction or calculated result is not finite.
    NonFiniteState,
    /// Modern input supplied a legacy-only nonzero pushthrough field.
    UnsupportedPushthrough,
}

/// Evaluate one admitted local pair, without I/O, receipt mutation, automatic
/// replay or pair discovery. Positions must come from the caller's simulation;
/// packet targets alone do not establish current collision inputs. Repeated
/// calls represent repeated pushes, not idempotent packet processing.
pub fn model_living_pair_push(
    version: MinecraftVersion,
    input: &LivingPairPushInput,
) -> Result<LivingPairPushModel, LivingPairPushUnavailable> {
    use LivingPairPushDisposition as D;
    use LivingPairPushUnavailable as U;
    let first = &input.first;
    let second = &input.second;
    if !input.legacy_pushthrough.is_finite()
        || [
            first.position,
            first.velocity,
            second.position,
            second.velocity,
        ]
        .iter()
        .flatten()
        .any(|n| !n.is_finite())
    {
        return Err(U::NonFiniteState);
    }
    if version == MinecraftVersion::Java1_21_11 && input.legacy_pushthrough != 0. {
        return Err(U::UnsupportedPushthrough);
    }
    let mut model = LivingPairPushModel {
        version,
        input: input.clone(),
        disposition: D::Evaluated,
        first_velocity: first.velocity,
        second_velocity: second.velocity,
        first_pushed: false,
        second_pushed: false,
    };
    model.disposition = if input.first_sleeping {
        D::Sleeping
    } else if input.same_vehicle {
        D::SameVehicle
    } else if first.no_physics || second.no_physics {
        D::NoPhysics
    } else {
        D::Evaluated
    };
    if model.disposition != D::Evaluated {
        return Ok(model);
    }
    let mut x = second.position[0] - first.position[0];
    let mut z = second.position[2] - first.position[2];
    if !x.is_finite() || !z.is_finite() {
        return Err(U::NonFiniteState);
    }
    let distance = x.abs().max(z.abs());
    if distance < f64::from(0.01_f32) {
        model.disposition = D::TooClose;
        return Ok(model);
    }
    let root = match version {
        MinecraftVersion::Java1_16_1 => f64::from(distance.sqrt() as f32),
        MinecraftVersion::Java1_21_11 => distance.sqrt(),
    };
    if !root.is_finite() {
        return Err(U::NonFiniteState);
    }
    x /= root;
    z /= root;
    let inverse = (1. / root).min(1.);
    x *= inverse;
    z *= inverse;
    x *= f64::from(0.05_f32);
    z *= f64::from(0.05_f32);
    if version == MinecraftVersion::Java1_16_1 {
        // Original subtraction is float, independently rounded before multiply.
        let reduction = f64::from(1_f32 - input.legacy_pushthrough);
        x *= reduction;
        z *= reduction;
    }
    model.first_pushed =
        !first.vehicle && (version == MinecraftVersion::Java1_16_1 || first.pushable);
    model.second_pushed =
        !second.vehicle && (version == MinecraftVersion::Java1_16_1 || second.pushable);
    if model.first_pushed {
        model.first_velocity = [
            first.velocity[0] + -x,
            first.velocity[1] + 0.,
            first.velocity[2] + -z,
        ];
    }
    if model.second_pushed {
        model.second_velocity = [
            second.velocity[0] + x,
            second.velocity[1] + 0.,
            second.velocity[2] + z,
        ];
    }
    if [model.first_velocity, model.second_velocity]
        .iter()
        .flatten()
        .any(|n| !n.is_finite())
    {
        return Err(U::NonFiniteState);
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> LivingPairPushInput {
        let body = LivingPushBody {
            position: [0., 65., 0.],
            velocity: [0.; 3],
            no_physics: false,
            vehicle: false,
            pushable: true,
        };
        LivingPairPushInput {
            first: body.clone(),
            second: LivingPushBody {
                position: [0.1, 65., 0.1],
                ..body
            },
            first_sleeping: false,
            same_vehicle: false,
            legacy_pushthrough: 0.,
        }
    }

    fn vector_bits(value: &serde_json::Value) -> [f64; 3] {
        std::array::from_fn(|i| {
            f64::from_bits(u64::from_str_radix(value[i].as_str().unwrap(), 16).unwrap())
        })
    }

    #[test]
    fn inherited_pair_push_matches_unchanged_original_velocities_and_impulse_flags() {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut json = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_push_oracle.json.gz")[..],
        )
        .read_to_string(&mut json)
        .unwrap();
        let oracle: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            oracle["runner_sha256"].as_str().unwrap(),
            format!(
                "{:x}",
                Sha256::digest(include_bytes!("../../../scripts/run_entity_push_oracle.py"))
            )
        );
        let mut cases = 0;
        for (version, key, count) in [
            (MinecraftVersion::Java1_16_1, "1.16.1", 560),
            (MinecraftVersion::Java1_21_11, "1.21.11", 480),
        ] {
            let (harness, jar, mappings): (&[u8], _, _) = match version {
                MinecraftVersion::Java1_16_1 => (
                    include_bytes!(
                        "../../../scripts/entity_push_oracle/1.16.1/EntityPushOracle.java"
                    ),
                    "a412fd69db1f81db3f511c1463fd304675244077",
                    "11120c39da4df293c4bd020896391fb9ddd6c2ba",
                ),
                MinecraftVersion::Java1_21_11 => (
                    include_bytes!(
                        "../../../scripts/entity_push_oracle/1.21.11/EntityPushOracle.java"
                    ),
                    "64bb6d763bed0a9f1d632ec347938594144943ed",
                    "5621e9253f05fd57872bbe7f8ddf5f9a7d525955",
                ),
            };
            assert_eq!(oracle["versions"][key]["original_server_jar_sha1"], jar);
            assert_eq!(oracle["versions"][key]["mappings_sha1"], mappings);
            assert_eq!(
                oracle["versions"][key]["harness_sha256"].as_str().unwrap(),
                format!("{:x}", Sha256::digest(harness))
            );
            let rows = oracle["versions"][key]["results"].as_array().unwrap();
            assert_eq!(rows.len(), count);
            for row in rows {
                let raw = &row["input"];
                // Decimal JSON is diagnostic. Reconstruct original input bits
                // to avoid a JSON parser's last-bit rounding obscuring the audit.
                let body = |name: &str| LivingPushBody {
                    position: vector_bits(&raw[name]["position_bits"]),
                    velocity: vector_bits(&raw[name]["velocity_bits"]),
                    no_physics: raw[name]["no_physics"].as_bool().unwrap(),
                    vehicle: raw[name]["vehicle"].as_bool().unwrap(),
                    pushable: raw[name]["pushable"].as_bool().unwrap(),
                };
                let input = LivingPairPushInput {
                    first: body("first"),
                    second: body("second"),
                    first_sleeping: raw["first_sleeping"].as_bool().unwrap(),
                    same_vehicle: raw["same_vehicle"].as_bool().unwrap(),
                    legacy_pushthrough: f32::from_bits(
                        raw["pushthrough_bits"].as_u64().unwrap() as u32
                    ),
                };
                let model = model_living_pair_push(version, &input).unwrap();
                assert_eq!(
                    model.first_velocity.map(f64::to_bits),
                    vector_bits(&row["first_velocity_bits"]).map(f64::to_bits),
                    "{key} {row}"
                );
                assert_eq!(
                    model.second_velocity.map(f64::to_bits),
                    vector_bits(&row["second_velocity_bits"]).map(f64::to_bits),
                    "{key} {row}"
                );
                assert_eq!(
                    model.first_pushed,
                    row["first_pushed"].as_bool().unwrap(),
                    "{key} {row}"
                );
                assert_eq!(
                    model.second_pushed,
                    row["second_pushed"].as_bool().unwrap(),
                    "{key} {row}"
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 1040);
    }

    #[test]
    fn first_receiver_sleeping_and_vehicle_predicates_do_not_become_pair_contact_facts() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let mut input = input();
            input.first_sleeping = true;
            input.same_vehicle = true;
            input.second.no_physics = true;
            assert_eq!(
                model_living_pair_push(version, &input).unwrap().disposition,
                LivingPairPushDisposition::Sleeping
            );
            input.first_sleeping = false;
            assert_eq!(
                model_living_pair_push(version, &input).unwrap().disposition,
                LivingPairPushDisposition::SameVehicle
            );
            input.same_vehicle = false;
            assert_eq!(
                model_living_pair_push(version, &input).unwrap().disposition,
                LivingPairPushDisposition::NoPhysics
            );
            input.second.no_physics = false;
            input.first.vehicle = true;
            let model = model_living_pair_push(version, &input).unwrap();
            assert!(!model.first_pushed && model.second_pushed);
            assert_eq!(model.first_velocity, input.first.velocity);
            input.second.vehicle = true;
            let model = model_living_pair_push(version, &input).unwrap();
            assert_eq!(model.disposition, LivingPairPushDisposition::Evaluated);
            assert!(!model.first_pushed && !model.second_pushed);
        }
    }

    #[test]
    fn original_float_threshold_and_version_arithmetic_are_preserved() {
        let mut input = input();
        input.second.position = [f64::from(0.01_f32), 65., 0.];
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            assert!(
                model_living_pair_push(version, &input)
                    .unwrap()
                    .first_pushed
            );
            input.second.position[0] -= f64::EPSILON;
            assert_eq!(
                model_living_pair_push(version, &input).unwrap().disposition,
                LivingPairPushDisposition::TooClose
            );
            input.second.position[0] = f64::from(0.01_f32);
        }
        let mut input = self::input();
        let legacy = model_living_pair_push(MinecraftVersion::Java1_16_1, &input).unwrap();
        let modern = model_living_pair_push(MinecraftVersion::Java1_21_11, &input).unwrap();
        assert_ne!(
            legacy.first_velocity.map(f64::to_bits),
            modern.first_velocity.map(f64::to_bits)
        );
        input.first.pushable = false;
        assert!(
            model_living_pair_push(MinecraftVersion::Java1_16_1, &input)
                .unwrap()
                .first_pushed
        );
        assert!(
            !model_living_pair_push(MinecraftVersion::Java1_21_11, &input)
                .unwrap()
                .first_pushed
        );
    }

    #[test]
    fn nonfinite_inputs_and_overflow_are_refused_without_changing_the_input() {
        let valid = input();
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                for body in 0..2 {
                    for field in 0..2 {
                        for axis in 0..3 {
                            let mut input = valid.clone();
                            let target = if body == 0 {
                                &mut input.first
                            } else {
                                &mut input.second
                            };
                            if field == 0 {
                                target.position[axis] = number;
                            } else {
                                target.velocity[axis] = number;
                            }
                            assert!(matches!(
                                model_living_pair_push(version, &input),
                                Err(LivingPairPushUnavailable::NonFiniteState)
                            ));
                        }
                    }
                }
            }
            let mut input = valid.clone();
            input.first.position[0] = -f64::MAX;
            input.second.position[0] = f64::MAX;
            assert!(matches!(
                model_living_pair_push(version, &input),
                Err(LivingPairPushUnavailable::NonFiniteState)
            ));
        }
        assert_eq!(valid.first.velocity, [0.; 3]);
        let mut invalid = valid;
        invalid.legacy_pushthrough = f32::NAN;
        assert!(matches!(
            model_living_pair_push(MinecraftVersion::Java1_16_1, &invalid),
            Err(LivingPairPushUnavailable::NonFiniteState)
        ));
        invalid.legacy_pushthrough = 0.25;
        assert!(matches!(
            model_living_pair_push(MinecraftVersion::Java1_21_11, &invalid),
            Err(LivingPairPushUnavailable::UnsupportedPushthrough)
        ));
    }
}
