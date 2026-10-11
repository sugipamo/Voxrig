//! Explicit local living-body interpolation, separate from received packet targets.
use crate::MinecraftVersion;

/// Feet position and body yaw/pitch at a caller-selected model boundary.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingInterpolationPose {
    /// Model feet position, in blocks.
    pub position: [f64; 3],
    /// Body yaw and pitch in degrees, with original float precision.
    pub rotation: [f32; 2],
}
impl LivingInterpolationPose {
    fn finite(self) -> bool {
        self.position.iter().all(|n| n.is_finite()) && self.rotation.iter().all(|n| n.is_finite())
    }
    fn same_bits(self, other: Self) -> bool {
        self.position.map(f64::to_bits) == other.position.map(f64::to_bits)
            && self.rotation.map(f32::to_bits) == other.rotation.map(f32::to_bits)
    }
}

/// Fixed-size model snapshot; contains no live entity identity or send authority.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingInterpolationFrame {
    /// Original arithmetic version.
    pub version: MinecraftVersion,
    /// Local model pose, not an observed server-current position.
    pub pose: LivingInterpolationPose,
    /// Active target, including modern explicitly admitted displacement.
    pub target: Option<LivingInterpolationPose>,
    /// Remaining caller-scheduled interpolation steps.
    pub remaining_steps: u32,
}

/// An input or model operation outside the qualified finite scenario.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingInterpolationUnavailable {
    /// Input or intermediate/result arithmetic is not finite.
    NonFiniteState,
    /// Length exceeds the fixed 120-step limit.
    UnsupportedLength,
    /// An active modern step requires the original-style target-box collision query.
    MissingCollisionQuery,
    /// Legacy has no modern target-displacement collision query.
    UnsupportedCollisionQuery,
}

/// Caller-owned model of inherited living-body interpolation. Qualified against
/// original cow routines; entity-specific overrides and local authority are not
/// inferred. No clock, packet drain, simulation scheduling or movement I/O occurs.
/// The caller must cancel/discard this model on lifetime loss or incomplete history.
#[derive(Clone, Debug)]
pub struct LivingInterpolation {
    version: MinecraftVersion,
    length: u32,
    pose: LivingInterpolationPose,
    target: LivingInterpolationPose,
    remaining: u32,
    previous: Option<LivingInterpolationPose>,
}
impl LivingInterpolation {
    /// Create a model at an explicitly supplied initial pose. Zero-length legacy
    /// targets do not snap; zero-length modern targets do. Length is not inferred
    /// from a packet or entity type. Ordinary inherited living updates use three.
    pub fn new(
        version: MinecraftVersion,
        pose: LivingInterpolationPose,
        length: u32,
    ) -> Result<Self, LivingInterpolationUnavailable> {
        if !pose.finite() {
            return Err(LivingInterpolationUnavailable::NonFiniteState);
        }
        if length > 120 {
            return Err(LivingInterpolationUnavailable::UnsupportedLength);
        }
        Ok(Self {
            version,
            length,
            pose,
            target: pose,
            remaining: 0,
            previous: None,
        })
    }

    /// Current numerical model diagnostics.
    pub fn frame(&self) -> LivingInterpolationFrame {
        LivingInterpolationFrame {
            version: self.version,
            pose: self.pose,
            target: (self.remaining > 0).then_some(self.target),
            remaining_steps: self.remaining,
        }
    }

    /// Admit an already resolved full target. Native packet dispatch, relative
    /// teleport resolution and legacy tiny-teleport suppression precede this call.
    /// Modern identical active targets retain their remaining steps; legacy resets.
    pub fn set_target(
        &mut self,
        before: LivingInterpolationPose,
        target: LivingInterpolationPose,
    ) -> Result<LivingInterpolationFrame, LivingInterpolationUnavailable> {
        if !before.finite() || !target.finite() {
            return Err(LivingInterpolationUnavailable::NonFiniteState);
        }
        self.pose = before;
        if self.version == MinecraftVersion::Java1_21_11 {
            if self.length == 0 {
                self.pose = target;
                self.pose.rotation[1] = (target.rotation[1] % 360.).clamp(-90., 90.);
                return Ok(self.cancel());
            }
            if self.remaining > 0 && self.target.same_bits(target) {
                return Ok(self.frame());
            }
            self.previous = Some(self.pose);
        }
        self.target = target;
        self.remaining = self.length;
        Ok(self.frame())
    }

    /// Cancel the numerical continuation, preserving the last model pose.
    pub fn cancel(&mut self) -> LivingInterpolationFrame {
        self.remaining = 0;
        self.previous = None;
        self.frame()
    }

    /// Apply an explicitly supplied model pose from another admitted physics
    /// stage without advancing interpolation or changing its target/baseline.
    /// This is not a received packet target or proof of native passenger travel.
    pub fn set_model_pose(
        &mut self,
        pose: LivingInterpolationPose,
    ) -> Result<LivingInterpolationFrame, LivingInterpolationUnavailable> {
        if !pose.finite() {
            return Err(LivingInterpolationUnavailable::NonFiniteState);
        }
        self.pose = pose;
        Ok(self.frame())
    }

    /// Evaluate one explicitly scheduled interpolation stage at `before` (which
    /// may include separately modeled movement since the preceding stage).
    /// Modern active steps require the caller's original-style `noCollision`
    /// result at the target shifted by `before.position - previous.position`.
    /// Rotation displacement is admitted independently of that query.
    /// Missing/invalid inputs leave the entire model unchanged. This stage does
    /// not perform AI, travel, gravity, head-yaw interpolation, pushes or mounts.
    pub fn step(
        &mut self,
        before: LivingInterpolationPose,
        shifted_target_collision_free: Option<bool>,
    ) -> Result<LivingInterpolationFrame, LivingInterpolationUnavailable> {
        use LivingInterpolationUnavailable as U;
        if !before.finite() {
            return Err(U::NonFiniteState);
        }
        let modern = self.version == MinecraftVersion::Java1_21_11;
        if !modern && shifted_target_collision_free.is_some() {
            return Err(U::UnsupportedCollisionQuery);
        }
        let mut next = self.clone();
        next.pose = before;
        if next.remaining == 0 {
            next.previous = None;
        } else {
            if modern {
                let free = shifted_target_collision_free.ok_or(U::MissingCollisionQuery)?;
                if let Some(previous) = next.previous {
                    let delta: [f64; 3] =
                        std::array::from_fn(|i| before.position[i] - previous.position[i]);
                    let shifted: [f64; 3] =
                        std::array::from_fn(|i| next.target.position[i] + delta[i]);
                    if delta.iter().chain(&shifted).any(|n| !n.is_finite()) {
                        return Err(U::NonFiniteState);
                    }
                    if free {
                        next.target.position = shifted;
                    }
                    for i in 0..2 {
                        next.target.rotation[i] += before.rotation[i] - previous.rotation[i];
                    }
                }
            }
            let steps = f64::from(next.remaining);
            let fraction = 1. / steps;
            for i in 0..3 {
                let difference = next.target.position[i] - before.position[i];
                if !difference.is_finite() {
                    return Err(U::NonFiniteState);
                }
                next.pose.position[i] += if modern {
                    fraction * difference
                } else {
                    difference / steps
                };
            }
            for i in 0..2 {
                let mut difference =
                    f64::from(next.target.rotation[i]) - f64::from(before.rotation[i]);
                if i == 0 {
                    difference %= 360.;
                    if difference >= 180. {
                        difference -= 360.;
                    }
                    if difference < -180. {
                        difference += 360.;
                    }
                }
                let change = if modern {
                    fraction * difference
                } else {
                    difference / steps
                };
                next.pose.rotation[i] = ((f64::from(before.rotation[i]) + change) as f32) % 360.;
                if modern && i == 1 {
                    next.pose.rotation[i] = next.pose.rotation[i].clamp(-90., 90.);
                }
            }
            next.remaining -= 1;
            if modern {
                next.previous = Some(next.pose);
            }
        }
        if !next.pose.finite() || !next.target.finite() {
            return Err(U::NonFiniteState);
        }
        *self = next;
        Ok(self.frame())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pose(value: &serde_json::Value) -> LivingInterpolationPose {
        LivingInterpolationPose {
            position: std::array::from_fn(|i| {
                f64::from_bits(
                    u64::from_str_radix(value["position_bits"][i].as_str().unwrap(), 16).unwrap(),
                )
            }),
            rotation: std::array::from_fn(|i| {
                f32::from_bits(value["rotation_bits"][i].as_u64().unwrap() as u32)
            }),
        }
    }
    #[test]
    fn living_interpolation_matches_original_schedules_and_float_bits() {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut json = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_interpolation_oracle.json.gz")[..],
        )
        .read_to_string(&mut json)
        .unwrap();
        let oracle: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            oracle["runner_sha256"],
            format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../scripts/run_entity_interpolation_oracle.py"
                ))
            )
        );
        let mut operations = 0;
        for (version, key, harness) in [
            (
                MinecraftVersion::Java1_16_1,
                "1.16.1",
                &include_bytes!(
                    "../../../scripts/entity_interpolation_oracle/1.16.1/EntityInterpolationOracle.java"
                )[..],
            ),
            (
                MinecraftVersion::Java1_21_11,
                "1.21.11",
                &include_bytes!(
                    "../../../scripts/entity_interpolation_oracle/1.21.11/EntityInterpolationOracle.java"
                )[..],
            ),
        ] {
            let qualified = &oracle["versions"][key];
            assert_eq!(
                qualified["harness_sha256"],
                format!("{:x}", Sha256::digest(harness))
            );
            assert_eq!(
                qualified["original_server_jar_sha1"],
                if key == "1.16.1" {
                    "a412fd69db1f81db3f511c1463fd304675244077"
                } else {
                    "64bb6d763bed0a9f1d632ec347938594144943ed"
                }
            );
            if key == "1.16.1" {
                assert_eq!(
                    qualified["original_client_jar_sha1"],
                    "c9abbe8ee4fa490751ca70635340b7cf00db83ff"
                );
            }
            let rows = qualified["results"].as_array().unwrap();
            assert_eq!(rows.len(), 210);
            for row in rows {
                let mut model = LivingInterpolation::new(
                    version,
                    pose(&row["initial"]),
                    row["length"].as_u64().unwrap() as u32,
                )
                .unwrap();
                for event in row["events"].as_array().unwrap() {
                    let frame = match event["operation"].as_str().unwrap() {
                        "target" => model
                            .set_target(pose(&event["before"]), pose(&event["input"]))
                            .unwrap(),
                        "step" => model
                            .step(pose(&event["input"]), event["collision_free"].as_bool())
                            .unwrap(),
                        "cancel" => model.cancel(),
                        _ => panic!("unknown oracle operation"),
                    };
                    assert!(
                        frame.pose.same_bits(pose(&event["pose"])),
                        "version={key} scenario={} length={} operation={} model={frame:?} native={event}",
                        row["scenario"],
                        row["length"],
                        event["operation"]
                    );
                    assert_eq!(
                        frame.remaining_steps,
                        event["remaining_steps"].as_u64().unwrap() as u32,
                        "{key}: {row}"
                    );
                    operations += 1;
                }
            }
        }
        assert_eq!(operations, 3780);
    }

    fn initial() -> LivingInterpolationPose {
        LivingInterpolationPose {
            position: [0., 65., 0.],
            rotation: [0., 0.],
        }
    }
    #[test]
    fn unavailable_interpolation_leaves_state_unchanged_and_cancel_stops_continuation() {
        use LivingInterpolationUnavailable as U;
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            assert_eq!(
                LivingInterpolation::new(version, initial(), 121).unwrap_err(),
                U::UnsupportedLength
            );
            let mut model = LivingInterpolation::new(version, initial(), 3).unwrap();
            let target = LivingInterpolationPose {
                position: [1., 66., 3.],
                rotation: [179., 45.],
            };
            model.set_target(initial(), target).unwrap();
            let retained = format!("{model:?}");
            let bad = LivingInterpolationPose {
                position: [f64::NAN, 0., 0.],
                ..initial()
            };
            assert_eq!(
                model.set_target(initial(), bad).unwrap_err(),
                U::NonFiniteState
            );
            assert_eq!(model.step(bad, None).unwrap_err(), U::NonFiniteState);
            let query = if version == MinecraftVersion::Java1_16_1 {
                Some(true)
            } else {
                None
            };
            assert_eq!(
                model.step(initial(), query).unwrap_err(),
                if query.is_some() {
                    U::UnsupportedCollisionQuery
                } else {
                    U::MissingCollisionQuery
                }
            );
            assert_eq!(format!("{model:?}"), retained);
            let frame = model.cancel();
            assert!(frame.target.is_none());
            assert_eq!(frame.remaining_steps, 0);
            assert!(
                model
                    .step(initial(), None)
                    .unwrap()
                    .pose
                    .same_bits(initial())
            );
        }
        let mut model = LivingInterpolation::new(
            MinecraftVersion::Java1_21_11,
            LivingInterpolationPose {
                position: [-f64::MAX, 0., 0.],
                ..initial()
            },
            3,
        )
        .unwrap();
        model
            .set_target(
                model.frame().pose,
                LivingInterpolationPose {
                    position: [f64::MAX, 0., 0.],
                    ..initial()
                },
            )
            .unwrap();
        let retained = format!("{model:?}");
        assert_eq!(
            model.step(model.frame().pose, Some(true)).unwrap_err(),
            U::NonFiniteState
        );
        assert_eq!(format!("{model:?}"), retained);
    }
}
