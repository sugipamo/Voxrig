//! Original living packet application at an explicit model boundary.
use super::{
    LivingInterpolation, LivingInterpolationFrame, LivingInterpolationPose,
    LivingInterpolationUnavailable,
};
use crate::MinecraftVersion;

/// Original decoded motion operation, distinct from sticky latest fields.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum LivingMotionPacket {
    /// Original short deltas and optional original body rotation.
    Relative {
        /// Signed original 1/4096-block components.
        delta: [i16; 3],
        /// Body rotation supplied by this packet, if any.
        rotation: Option<[f32; 2]>,
        /// Original packet's ground flag.
        ground: bool,
    },
    /// Original rotation-only update.
    Rotation {
        /// Supplied yaw/pitch in original float precision.
        rotation: [f32; 2],
        /// Original packet's ground flag.
        ground: bool,
    },
    /// Original legacy remote teleport (not own-player correction).
    LegacyTeleport {
        /// Original absolute coordinates, distinct from codec lattice values.
        position: [f64; 3],
        /// Supplied yaw/pitch.
        rotation: [f32; 2],
        /// Original packet's ground flag.
        ground: bool,
    },
    /// Original modern entity-position synchronization (not relative teleport).
    ModernPositionSync {
        /// Original absolute coordinates.
        position: [f64; 3],
        /// Supplied yaw/pitch.
        rotation: [f32; 2],
        /// Movement sample supplied by the synchronization packet.
        velocity: [f64; 3],
        /// Original packet's ground flag.
        ground: bool,
    },
    /// Original instantaneous velocity sample; it does not advance model feet.
    Velocity([f64; 3]),
    /// Original head yaw, separate from body interpolation.
    HeadYaw(f32),
}

/// Explicit packet-dispatch predicates supplied for this model boundary.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingPacketGuards {
    /// Original entity's local movement authority at application.
    pub local_authoritative: bool,
    /// Original modern entity tick-list membership at application.
    pub ticking: bool,
}

/// Complete numerical state. Contains no live target or send authority.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingPacketFrame {
    /// Separate interpolation model and target.
    pub interpolation: LivingInterpolationFrame,
    /// Native delta codec base, never silently replaced by interpolated feet.
    pub codec_base: [f64; 3],
    /// Last explicitly applied sample; no implicit stop is synthesized.
    pub velocity: Option<[f64; 3]>,
    /// Last explicitly applied head sample.
    pub head_yaw: Option<f32>,
    /// Last original ground flag.
    pub on_ground: Option<bool>,
    /// Last applied receipt ordinal in this numerical input stream.
    pub applied_ordinal: u64,
}

/// Application outside the qualified lane. Errors preserve the previous model.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingPacketUnavailable {
    /// Model explicitly lost its lifetime or complete input history.
    Invalidated,
    /// Earlier ordinal cannot overwrite a newer application.
    OutOfOrder,
    /// Operation belongs to another protocol version.
    UnsupportedPacket,
    /// Numerical coordinates exceed the admitted native codec range.
    UnsupportedCoordinates,
    /// An input contains non-finite values.
    NonFiniteState,
    /// Original interpolation input or collision evidence is unavailable.
    Interpolation(LivingInterpolationUnavailable),
}

/// Numerical inherited-living packet model. The caller chooses packet/tick
/// boundaries explicitly; receive ordinals do not manufacture a native clock.
/// Missing original operation, relative teleport semantics, special species,
/// passenger travel and incomplete history must be refused before this lane.
#[derive(Clone, Debug)]
pub struct LivingPacketModel {
    version: MinecraftVersion,
    valid: bool,
    interpolation: LivingInterpolation,
    base: [f64; 3],
    velocity: Option<[f64; 3]>,
    head: Option<f32>,
    ground: Option<bool>,
    applied: u64,
}
impl LivingPacketModel {
    /// Admit explicit spawn coordinates/body rotation and the original input
    /// ordinal. No received identity or unreceived velocity is constructed.
    pub fn new(
        version: MinecraftVersion,
        pose: LivingInterpolationPose,
        ordinal: u64,
    ) -> Result<Self, LivingPacketUnavailable> {
        position(pose.position)?;
        let interpolation = LivingInterpolation::new(version, pose, 3)
            .map_err(LivingPacketUnavailable::Interpolation)?;
        let base = if version == MinecraftVersion::Java1_16_1 {
            pose.position.map(|x| (x * 4096.).floor() / 4096.)
        } else {
            pose.position
        };
        Ok(Self {
            version,
            valid: true,
            interpolation,
            base,
            velocity: None,
            head: None,
            ground: None,
            applied: ordinal,
        })
    }
    /// Fixed-size diagnostics, independent of latest receive observations.
    pub fn frame(&self) -> LivingPacketFrame {
        LivingPacketFrame {
            interpolation: self.interpolation.frame(),
            codec_base: self.base,
            velocity: self.velocity,
            head_yaw: self.head,
            on_ground: self.ground,
            applied_ordinal: self.applied,
        }
    }
    pub(super) fn initialize_samples(
        &mut self,
        velocity: Option<[f64; 3]>,
        head: Option<f32>,
    ) -> Result<(), LivingPacketUnavailable> {
        if let Some(v) = velocity {
            finite_velocity(v)?;
        }
        if head.is_some_and(|n| !n.is_finite()) {
            return Err(LivingPacketUnavailable::NonFiniteState);
        }
        self.velocity = velocity;
        self.head = head;
        Ok(())
    }
    /// Irreversibly disable on lifetime/history loss; read-only diagnostics remain.
    pub fn invalidate(&mut self) {
        self.valid = false;
        self.interpolation.cancel();
    }
    /// Cancel interpolation at a supplied model boundary without inventing a packet.
    pub fn cancel(&mut self) -> Result<LivingPacketFrame, LivingPacketUnavailable> {
        self.ready()?;
        self.interpolation.cancel();
        Ok(self.frame())
    }
    /// Supply an explicit model pose after another admitted physics stage.
    /// Target, native codec base, remaining steps and receipt ordinal are kept.
    /// An original dismount placement supplied by a harness is model input;
    /// this method does not implement or acknowledge the dismount itself.
    pub fn set_model_pose(
        &mut self,
        pose: LivingInterpolationPose,
    ) -> Result<LivingPacketFrame, LivingPacketUnavailable> {
        self.ready()?;
        position(pose.position)?;
        self.interpolation
            .set_model_pose(pose)
            .map_err(LivingPacketUnavailable::Interpolation)?;
        Ok(self.frame())
    }
    fn ready(&self) -> Result<(), LivingPacketUnavailable> {
        if self.valid {
            Ok(())
        } else {
            Err(LivingPacketUnavailable::Invalidated)
        }
    }
    /// Apply a complete original operation once. Same ordinal is a no-op;
    /// lower ordinal is refused. Duplicate native targets at new ordinals retain
    /// the native version's own interpolation behavior.
    pub fn apply(
        &mut self,
        ordinal: u64,
        packet: LivingMotionPacket,
        guards: LivingPacketGuards,
    ) -> Result<bool, LivingPacketUnavailable> {
        self.ready()?;
        if ordinal < self.applied {
            return Err(LivingPacketUnavailable::OutOfOrder);
        }
        if ordinal == self.applied {
            return Ok(false);
        }
        let mut next = self.clone();
        next.apply_inner(packet, guards)?;
        next.applied = ordinal;
        *self = next;
        Ok(true)
    }
    fn apply_inner(
        &mut self,
        packet: LivingMotionPacket,
        guards: LivingPacketGuards,
    ) -> Result<(), LivingPacketUnavailable> {
        use LivingMotionPacket as P;
        use LivingPacketUnavailable as U;
        let modern = self.version == MinecraftVersion::Java1_21_11;
        let before = self.interpolation.frame().pose;
        let inherited = self.interpolation.frame().target.unwrap_or(before);
        let target = match packet {
            P::Relative {
                delta,
                rotation,
                ground,
            } => {
                if let Some(rotation) = rotation {
                    finite_rotation(rotation)?
                }
                if modern || !guards.local_authoritative {
                    for (i, d) in delta.into_iter().enumerate() {
                        if modern && d == 0 {
                            continue;
                        }
                        self.base[i] = ((self.base[i] * 4096. + if modern { 0.5 } else { 0. })
                            .floor()
                            + f64::from(d))
                            / 4096.;
                    }
                    position(self.base)?;
                }
                if guards.local_authoritative {
                    return Ok(());
                }
                self.ground = Some(ground);
                Some(LivingInterpolationPose {
                    position: self.base,
                    rotation: rotation.unwrap_or(if modern {
                        inherited.rotation
                    } else {
                        before.rotation
                    }),
                })
            }
            P::Rotation { rotation, ground } => {
                finite_rotation(rotation)?;
                if guards.local_authoritative {
                    return Ok(());
                }
                self.ground = Some(ground);
                Some(LivingInterpolationPose {
                    position: if modern {
                        inherited.position
                    } else {
                        before.position
                    },
                    rotation,
                })
            }
            P::LegacyTeleport {
                position: p,
                rotation,
                ground,
            } => {
                if modern {
                    return Err(U::UnsupportedPacket);
                }
                position(p)?;
                finite_rotation(rotation)?;
                self.base = p.map(|x| (x * 4096.).floor() / 4096.);
                if guards.local_authoritative {
                    return Ok(());
                }
                self.ground = Some(ground);
                let tiny = (before.position[0] - p[0]).abs() < 0.03125
                    && (before.position[1] - p[1]).abs() < 0.015625
                    && (before.position[2] - p[2]).abs() < 0.03125;
                Some(LivingInterpolationPose {
                    position: if tiny { before.position } else { p },
                    rotation,
                })
            }
            P::ModernPositionSync {
                position: p,
                rotation,
                velocity,
                ground,
            } => {
                if !modern {
                    return Err(U::UnsupportedPacket);
                }
                position(p)?;
                finite_rotation(rotation)?;
                finite_velocity(velocity)?;
                self.base = p;
                if guards.local_authoritative {
                    return Ok(());
                }
                self.velocity = Some(velocity);
                self.ground = Some(ground);
                let distant = (0..3)
                    .map(|i| (before.position[i] - p[i]).powi(2))
                    .sum::<f64>()
                    > 4096.;
                let pose = LivingInterpolationPose {
                    position: p,
                    rotation,
                };
                if !guards.ticking || distant {
                    self.interpolation = LivingInterpolation::new(
                        self.version,
                        LivingInterpolationPose {
                            position: p,
                            rotation: [rotation[0], rotation[1].clamp(-90., 90.)],
                        },
                        3,
                    )
                    .map_err(U::Interpolation)?;
                    return Ok(());
                }
                Some(pose)
            }
            P::Velocity(v) => {
                finite_velocity(v)?;
                self.velocity = Some(v);
                None
            }
            P::HeadYaw(v) => {
                if !v.is_finite() {
                    return Err(U::NonFiniteState);
                }
                self.head = Some(v);
                None
            }
        };
        if let Some(target) = target {
            self.interpolation
                .set_target(before, target)
                .map_err(U::Interpolation)?;
        }
        Ok(())
    }
    /// Advance one admitted interpolation stage. The original-style modern
    /// target collision query is required while active; this never advances
    /// packet codec coordinates or the received sample's ordinal.
    pub fn step(
        &mut self,
        collision_free: Option<bool>,
    ) -> Result<LivingPacketFrame, LivingPacketUnavailable> {
        self.ready()?;
        self.interpolation
            .step(self.interpolation.frame().pose, collision_free)
            .map_err(LivingPacketUnavailable::Interpolation)?;
        Ok(self.frame())
    }
}
fn position(p: [f64; 3]) -> Result<(), LivingPacketUnavailable> {
    if p.iter().any(|n| !n.is_finite()) {
        return Err(LivingPacketUnavailable::NonFiniteState);
    }
    if p[0].abs() > 32_000_000. || p[2].abs() > 32_000_000. || p[1].abs() > 8_000_000. {
        return Err(LivingPacketUnavailable::UnsupportedCoordinates);
    }
    Ok(())
}
fn finite_rotation(r: [f32; 2]) -> Result<(), LivingPacketUnavailable> {
    if r.iter().all(|n| n.is_finite()) {
        Ok(())
    } else {
        Err(LivingPacketUnavailable::NonFiniteState)
    }
}
fn finite_velocity(v: [f64; 3]) -> Result<(), LivingPacketUnavailable> {
    if v.iter().all(|n| n.is_finite()) {
        Ok(())
    } else {
        Err(LivingPacketUnavailable::NonFiniteState)
    }
}
