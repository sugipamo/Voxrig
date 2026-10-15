//! Numerical client-world contacts in the original entity dispatch order.
//! Receipt application and owner admission belong to `owned_contacts`.
use super::{
    LivingContactBody, LivingContactCandidate, LivingContactPlayer, LivingEntityTickOrder,
    LivingInterpolationPose, LivingMotionPacket, LivingPacketGuards, LivingPacketModel,
    LivingPushBody, LivingTickBody, LivingTickDispatch, model_living_contacts,
};
use crate::{MinecraftVersion, Result, client::Aabb};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContactDispatch {
    InheritedLiving,
    RemotePlayer,
    Chicken,
    Wolf,
}

#[derive(Clone, Debug, Default)]
struct WolfAnimation {
    interested: bool,
    angle: f32,
    old_angle: f32,
    wet: bool,
    shaking: bool,
    shake: f32,
    old_shake: f32,
}
impl WolfAnimation {
    fn tick(&mut self, version: MinecraftVersion, wet_input: bool) {
        self.old_angle = self.angle;
        self.angle += (if self.interested { 1. } else { 0. } - self.angle) * 0.4_f32;
        if wet_input {
            self.wet = true;
            if version == MinecraftVersion::Java1_16_1 {
                self.cancel_shake();
            }
        } else if self.shaking {
            // Original (isWet || isShaking) && isShaking has this same truth table.
            self.old_shake = self.shake;
            self.shake += 0.05_f32;
            if self.old_shake >= 2. {
                self.wet = false;
                self.cancel_shake();
            }
        }
    }
    fn cancel_shake(&mut self) {
        self.shaking = false;
        self.shake = 0.;
        self.old_shake = 0.;
    }
}

#[derive(Clone, Debug)]
struct ChickenFlap {
    flap: f32,
    flap_speed: f32,
    o_flap: f32,
    o_flap_speed: f32,
    flapping: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct ContactTickBody {
    pub key: u64,
    pub native_id: i32,
    pub dispatch: ContactDispatch,
    pub motion: LivingPacketModel,
    /// Derived delta movement, distinct from the last received velocity sample.
    pub velocity: [f64; 3],
    pub width: f32,
    pub height: f32,
    pub sleeping: bool,
    pub alive: bool,
    pub pushable: bool,
    pub team: Option<(String, crate::client::ui::TeamCollision)>,
    velocity_target: [f64; 3],
    velocity_steps: u8,
    // Legacy refreshDimensions anchors a nonshrinking box at its old minimum
    // until an original setPos (interpolation or received bed placement).
    refreshed_bounds: Option<Aabb>,
    chicken: Option<ChickenFlap>,
    wolf: Option<WolfAnimation>,
}
impl ContactTickBody {
    pub(crate) fn new(
        version: MinecraftVersion,
        identity: (u64, i32),
        dispatch: ContactDispatch,
        pose: LivingInterpolationPose,
        velocity: [f64; 3],
        ordinal: u64,
        dimensions: [f32; 2],
    ) -> Result<Self> {
        Ok(Self {
            key: identity.0,
            native_id: identity.1,
            dispatch,
            motion: LivingPacketModel::new(version, pose, ordinal).map_err(failure)?,
            velocity,
            width: dimensions[0],
            height: dimensions[1],
            sleeping: false,
            alive: true,
            pushable: true,
            team: None,
            velocity_target: [0.; 3],
            velocity_steps: 0,
            refreshed_bounds: None,
            wolf: (dispatch == ContactDispatch::Wolf).then(WolfAnimation::default),
            chicken: (dispatch == ContactDispatch::Chicken).then_some(ChickenFlap {
                flap: 0.,
                flap_speed: 0.,
                o_flap: 0.,
                o_flap_speed: 0.,
                flapping: 1.,
            }),
        })
    }

    pub(crate) fn apply(
        &mut self,
        version: MinecraftVersion,
        ordinal: u64,
        packet: LivingMotionPacket,
    ) -> Result<()> {
        // The caller deduplicates the complete ordered stream. A duplicate
        // operation must not restart RemotePlayer's velocity interpolation.
        if ordinal == self.motion.frame().applied_ordinal {
            return Ok(());
        }
        self.motion
            .apply(
                ordinal,
                packet,
                LivingPacketGuards {
                    local_authoritative: false,
                    ticking: true,
                },
            )
            .map_err(failure)?;
        match packet {
            LivingMotionPacket::Velocity(v)
                if version == MinecraftVersion::Java1_21_11
                    && self.dispatch == ContactDispatch::RemotePlayer =>
            {
                self.velocity_target = v;
                self.velocity_steps = 3;
            }
            LivingMotionPacket::Velocity(v) => self.velocity = v,
            // PositionSync carries a movement receipt, but its original
            // handler never calls setDeltaMovement/lerpMotion. Keep it solely
            // in motion.frame().velocity, independent of derived movement.
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn refresh_dimensions(&mut self, version: MinecraftVersion, dimensions: [f32; 2]) {
        let old_bounds = self.contact().bounds;
        let old_width = self.width;
        [self.width, self.height] = dimensions;
        self.refreshed_bounds =
            if version == MinecraftVersion::Java1_16_1 && self.width >= old_width {
                Some(Aabb {
                    min_x: old_bounds.min_x,
                    min_y: old_bounds.min_y,
                    min_z: old_bounds.min_z,
                    max_x: old_bounds.min_x + f64::from(self.width),
                    max_y: old_bounds.min_y + f64::from(self.height),
                    max_z: old_bounds.min_z + f64::from(self.width),
                })
            } else {
                None
            };
    }

    pub(crate) fn set_model_pose(&mut self, pose: LivingInterpolationPose) -> Result<()> {
        self.motion.set_model_pose(pose).map_err(failure)?;
        self.refreshed_bounds = None;
        Ok(())
    }

    pub(crate) fn centered_bounds(&self) -> Aabb {
        let position = self.motion.frame().interpolation.pose.position;
        let half = f64::from(self.width / 2.);
        Aabb {
            min_x: position[0] - half,
            min_y: position[1],
            min_z: position[2] - half,
            max_x: position[0] + half,
            max_y: position[1] + f64::from(self.height),
            max_z: position[2] + half,
        }
    }

    pub(crate) fn contact(&self) -> LivingContactBody {
        let position = self.motion.frame().interpolation.pose.position;
        LivingContactBody {
            push: LivingPushBody {
                position,
                velocity: self.velocity,
                // Both original RemotePlayer constructors set noPhysics.
                no_physics: self.dispatch == ContactDispatch::RemotePlayer,
                vehicle: false,
                pushable: self.pushable,
            },
            bounds: self
                .refreshed_bounds
                .unwrap_or_else(|| self.centered_bounds()),
            sleeping: self.sleeping,
            legacy_pushthrough: 0.,
            team: self.team.as_ref().map(|t| t.1),
        }
    }

    pub(crate) fn wolf_interest(&mut self, interested: bool) {
        if let Some(wolf) = &mut self.wolf {
            wolf.interested = interested;
        }
    }

    // Species wrappers run after the inherited living step and pair pushes.
    // The owner isolates baseTick/fluid inputs. Wolf wet/shake fields cannot
    // affect contacts; the full version-specific wrapper is oracle-checked below.
    fn species_after_ai(&mut self, version: MinecraftVersion) {
        if self.alive {
            if let Some(wolf) = &mut self.wolf {
                wolf.tick(version, false);
            }
        }
        let Some(state) = &mut self.chicken else {
            return;
        };
        let ground = self.motion.frame().on_ground.unwrap_or(false);
        state.o_flap = state.flap;
        state.o_flap_speed = state.flap_speed;
        state.flap_speed = if version == MinecraftVersion::Java1_16_1 {
            (f64::from(state.flap_speed) + if ground { -1. } else { 4. } * 0.3) as f32
        } else {
            state.flap_speed + if ground { -1. } else { 4. } * 0.3_f32
        }
        .clamp(0., 1.);
        if !ground && state.flapping < 1. {
            state.flapping = 1.;
        }
        state.flapping = if version == MinecraftVersion::Java1_16_1 {
            (f64::from(state.flapping) * 0.9) as f32
        } else {
            state.flapping * 0.9_f32
        };
        if !ground && self.velocity[1] < 0. {
            self.velocity[1] *= 0.6;
        }
        state.flap += state.flapping * 2.;
    }

    fn ai_step(&mut self, version: MinecraftVersion, collision_free: Option<bool>) -> Result<()> {
        let interpolating = self.motion.frame().interpolation.remaining_steps != 0;
        self.motion.step(collision_free).map_err(failure)?;
        if interpolating {
            self.refreshed_bounds = None;
        }
        if self.dispatch == ContactDispatch::RemotePlayer {
            if self.velocity_steps != 0 {
                for (current, target) in self.velocity.iter_mut().zip(self.velocity_target) {
                    *current += (target - *current) / f64::from(self.velocity_steps);
                }
                self.velocity_steps -= 1;
            }
        } else {
            // Original nonauthoritative client living lane: no travel/AI.
            // Interpolation and the else-only .98 damping precede clamping.
            if !interpolating {
                for current in &mut self.velocity {
                    *current *= 0.98;
                }
            }
            for current in &mut self.velocity {
                if current.abs() < 0.003 {
                    *current = 0.;
                }
            }
        }
        let _ = version;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ContactTickWorld {
    pub version: MinecraftVersion,
    pub order: LivingEntityTickOrder,
    pub bodies: BTreeMap<u64, ContactTickBody>,
    pub own_key: u64,
    pub spatial: super::LivingContactIndex,
}
impl ContactTickWorld {
    /// Geometry-only remote stage around a controlled boat's native slot.
    /// Remote velocity is not exposed as full entity physics in this lane.
    pub(crate) fn tick_geometry<T>(
        &mut self,
        controlled: u64,
        mut controlled_tick: impl FnMut(
            &BTreeMap<u64, ContactTickBody>,
            &mut super::LivingContactIndex,
        ) -> Result<T>,
        mut collision_free: impl FnMut(&ContactTickBody) -> Result<Option<bool>>,
        mut predicates_after_ai: impl FnMut(&mut ContactTickBody) -> Result<()>,
    ) -> Result<T> {
        self.order.begin_tick(false).map_err(failure)?;
        let mut result = None;
        while let Some(instruction) = self.order.next_tick().map_err(failure)? {
            if instruction.key == controlled {
                if result.is_some() || instruction.dispatch != LivingTickDispatch::Tick {
                    return Err(failure("invalid controlled boat dispatch"));
                }
                result = Some(controlled_tick(&self.bodies, &mut self.spatial)?);
            } else if let Some(body) = self.bodies.get_mut(&instruction.key) {
                if instruction.dispatch != LivingTickDispatch::Tick {
                    return Err(failure("unqualified living passenger geometry"));
                }
                let collision = collision_free(body)?;
                body.ai_step(self.version, collision)?;
                body.species_after_ai(self.version);
                predicates_after_ai(body)?;
                index_body(&mut self.spatial, body)?;
            }
            // Received own passengers and PR74 rigid bodies have no living
            // geometry here; their existing mount/fixed-box lane is retained.
        }
        self.order.finish_tick().map_err(failure)?;
        result.ok_or_else(|| failure("controlled boat missing from native order"))
    }
    pub(crate) fn new(version: MinecraftVersion, own_key: u64) -> Self {
        Self {
            version,
            order: LivingEntityTickOrder::new(version),
            bodies: BTreeMap::new(),
            own_key,
            spatial: super::LivingContactIndex::new(version),
        }
    }

    pub(crate) fn admit(
        &mut self,
        body: LivingTickBody,
        model: Option<ContactTickBody>,
    ) -> Result<()> {
        self.order.admit(body).map_err(failure)?;
        if let Some(model) = model {
            index_body(&mut self.spatial, &model)?;
            self.bodies.insert(model.key, model);
        }
        Ok(())
    }

    /// One explicit local model tick. The own-player callback performs travel
    /// at its native slot; later NPC pushes persist into the next local tick.
    /// The caller stages/clones both this world and the local body before I/O.
    pub(crate) fn tick(
        &mut self,
        local: &mut LivingContactBody,
        local_context: (Option<&str>, bool),
        frozen: bool,
        mut travel: impl FnMut(&mut LivingContactBody) -> Result<()>,
        mut collision_free: impl FnMut(&ContactTickBody) -> Result<Option<bool>>,
        mut predicates_after_ai: impl FnMut(&mut ContactTickBody) -> Result<()>,
    ) -> Result<usize> {
        self.order.begin_tick(frozen).map_err(failure)?;
        let mut admitted = 0;
        let mut travelled = false;
        while let Some(instruction) = self.order.next_tick().map_err(failure)? {
            if instruction.dispatch != LivingTickDispatch::Tick {
                return Err(failure(
                    "passenger travel is outside living contact dispatch",
                ));
            }
            if instruction.key == self.own_key {
                if travelled {
                    return Err(failure("duplicate local player dispatch"));
                }
                travel(local)?;
                travelled = true;
                // Native client selector excludes self and every nonlocal
                // candidate. Do not push NPCs from LocalPlayer.pushEntities.
                continue;
            }
            let Some(body) = self.bodies.get_mut(&instruction.key) else {
                continue;
            };
            let collision = collision_free(body)?;
            body.ai_step(self.version, collision)?;
            predicates_after_ai(body)?;
            index_body(&mut self.spatial, body)?;
            let querying = body.contact();
            let result = model_living_contacts(
                self.version,
                true,
                &querying,
                &[LivingContactCandidate {
                    body: local.clone(),
                    same_body: false,
                    spectator: local_context.1,
                    player: LivingContactPlayer::LocalPlayer,
                    allied: body
                        .team
                        .as_ref()
                        .is_some_and(|team| Some(team.0.as_str()) == local_context.0),
                    same_vehicle: false,
                }],
            )
            .map_err(failure)?;
            body.velocity = result.querying_velocity;
            if let Some(push) = &result.candidates[0].push {
                local.push.velocity = push.first_velocity;
                admitted += 1;
            }
            body.species_after_ai(self.version);
        }
        self.order.finish_tick().map_err(failure)?;
        if !travelled {
            return Err(failure("local player missing from contact dispatch"));
        }
        Ok(admitted)
    }
}

pub(crate) fn failure(error: impl std::fmt::Debug) -> crate::Error {
    crate::Error::new(
        crate::ErrorKind::Unsupported,
        anyhow::anyhow!("entity contact model unavailable: {error:?}"),
    )
}

pub(crate) fn index_geometry(
    index: &mut super::LivingContactIndex,
    key: u64,
    position: [f64; 3],
    bounds: Aabb,
) -> Result<()> {
    // Unqualified placeholder: only section membership and geometry are
    // supplied. The public living-selector path refuses kind=None.
    index
        .upsert(super::LivingContactIndexedBody {
            key,
            kind: None,
            body: LivingContactBody {
                push: LivingPushBody {
                    position,
                    velocity: [0.; 3],
                    no_physics: false,
                    vehicle: false,
                    pushable: false,
                },
                bounds,
                sleeping: false,
                legacy_pushthrough: 0.,
                team: None,
            },
            spectator: false,
            player: LivingContactPlayer::NonPlayer,
            team_key: None,
            vehicle_root: None,
        })
        .map_err(failure)
}

fn index_body(index: &mut super::LivingContactIndex, body: &ContactTickBody) -> Result<()> {
    let contact = body.contact();
    index_geometry(index, body.key, contact.push.position, contact.bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::physics::{self, Body, Controls, Environment};
    use serde_json::Value;

    fn pose(p: &Value) -> LivingInterpolationPose {
        LivingInterpolationPose {
            position: std::array::from_fn(|i| {
                f64::from_bits(
                    u64::from_str_radix(p["position_bits"][i].as_str().unwrap(), 16).unwrap(),
                )
            }),
            rotation: std::array::from_fn(|i| {
                f32::from_bits(p["rotation_bits"][i].as_u64().unwrap() as u32)
            }),
        }
    }
    fn velocity(p: &Value) -> [f64; 3] {
        std::array::from_fn(|i| {
            f64::from_bits(
                u64::from_str_radix(p["velocity_bits"][i].as_str().unwrap(), 16).unwrap(),
            )
        })
    }
    fn assert_vector(actual: [f64; 3], expected: [f64; 3], context: &str) {
        assert_eq!(
            actual.map(f64::to_bits),
            expected.map(f64::to_bits),
            "{context}: {actual:?} != {expected:?}"
        );
    }
    fn tick_body(key: u64, native_id: i32, player: bool) -> LivingTickBody {
        LivingTickBody {
            key,
            native_id,
            player,
            removed: false,
            passenger_of: None,
            passengers: Vec::new(),
            legacy_ticking_chunk: Some(true),
            legacy_in_chunk: Some(true),
            player_passengers: 0,
        }
    }

    // Raw oracle rows also contain WIP species that were never qualified.
    // Horse still lacks a complete hook/variant proof after the shared legacy
    // dimension-refresh correction; matching a raw scene would not qualify it.
    // Admission refusals for unmodeled types are tested in owned_contacts;
    // numerical equality is asserted only for the declared qualified lane.
    fn qualified_scene(version: MinecraftVersion, row: &Value) -> bool {
        row["steps"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|step| step["actions"].as_array().unwrap())
            .filter(|action| action["op"] == "spawn")
            .all(|action| {
                super::super::contact_capabilities::capabilities(
                    version,
                    action["type_name"].as_str().unwrap(),
                )
                .modeled()
            })
    }

    #[test]
    fn native_client_world_contacts_and_neutral_travel_in_original_order() {
        let fixture: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        let mut stages = 0;
        let mut unmodeled_stages = 0;
        let mut pushed_slots = [[[false; 2]; 2]; 2];
        for (name, version) in [
            ("1.16.1", MinecraftVersion::Java1_16_1),
            ("1.21.11", MinecraftVersion::Java1_21_11),
        ] {
            for row in fixture["versions"][name]["results"].as_array().unwrap() {
                if row["scene"] == "boat" {
                    continue;
                }
                if !qualified_scene(version, row) {
                    unmodeled_stages += row["steps"].as_array().unwrap().len();
                    continue;
                }
                let seed = row["seed"].as_u64().unwrap();
                let first = &row["steps"][0];
                let spawn_player = first["actions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["op"] == "spawn_player")
                    .unwrap();
                let own_key = spawn_player["key"].as_u64().unwrap();
                let mut world = ContactTickWorld::new(version, own_key);
                let mut own = Body::new(pose(&spawn_player["pose"]).position);
                own.on_ground = false;
                own.velocity = velocity(&spawn_player["pose"]);
                let mut local = LivingContactBody {
                    push: LivingPushBody {
                        position: own.position,
                        velocity: own.velocity,
                        no_physics: false,
                        vehicle: false,
                        pushable: true,
                    },
                    bounds: box_value(own.bounds),
                    sleeping: false,
                    legacy_pushthrough: 0.,
                    team: None,
                };
                let mut ordinal = 0;
                for step in row["steps"].as_array().unwrap() {
                    let stage = step["stage"].as_u64().unwrap();
                    let context = format!("{name} seed {seed} stage {stage}");
                    for a in step["actions"].as_array().unwrap() {
                        ordinal += 1;
                        let id = a["native_id"].as_i64().unwrap() as i32;
                        match a["op"].as_str().unwrap() {
                            "spawn" => {
                                let key = a["key"].as_u64().unwrap();
                                let name = a["type_name"].as_str().unwrap();
                                let player = name == "minecraft:player";
                                let dimensions = super::super::geometry::shape(
                                    version,
                                    name,
                                    super::super::EntityPose::Standing,
                                    1.,
                                    1.,
                                );
                                let body = ContactTickBody::new(
                                    version,
                                    (key, id),
                                    if player {
                                        ContactDispatch::RemotePlayer
                                    } else if name == "minecraft:wolf" {
                                        ContactDispatch::Wolf
                                    } else if name == "minecraft:chicken" {
                                        ContactDispatch::Chicken
                                    } else {
                                        ContactDispatch::InheritedLiving
                                    },
                                    pose(&a["pose"]),
                                    velocity(&a["pose"]),
                                    ordinal,
                                    [dimensions.0, dimensions.1],
                                )
                                .unwrap();
                                world.admit(tick_body(key, id, player), Some(body)).unwrap();
                            }
                            "spawn_player" => {
                                world.admit(tick_body(own_key, id, true), None).unwrap()
                            }
                            "relative" | "rotation" | "absolute" => {
                                let body = world
                                    .bodies
                                    .values_mut()
                                    .find(|body| body.native_id == id)
                                    .unwrap();
                                let rotation = a["rotation"].as_array().map(|r| {
                                    std::array::from_fn(|i| r[i].as_f64().unwrap() as f32)
                                });
                                let packet = match a["op"].as_str().unwrap() {
                                    "relative" => LivingMotionPacket::Relative {
                                        delta: std::array::from_fn(|i| {
                                            a["delta"][i].as_i64().unwrap() as i16
                                        }),
                                        rotation: rotation.map(|r| r.map(|v| v * 360. / 256.)),
                                        ground: a["ground"].as_bool().unwrap(),
                                    },
                                    "rotation" => LivingMotionPacket::Rotation {
                                        rotation: rotation.unwrap().map(|v| v * 360. / 256.),
                                        ground: a["ground"].as_bool().unwrap(),
                                    },
                                    _ if version == MinecraftVersion::Java1_16_1 => {
                                        LivingMotionPacket::LegacyTeleport {
                                            position: std::array::from_fn(|i| {
                                                a["position"][i].as_f64().unwrap()
                                            }),
                                            rotation: rotation.unwrap(),
                                            ground: true,
                                        }
                                    }
                                    _ => LivingMotionPacket::ModernPositionSync {
                                        position: std::array::from_fn(|i| {
                                            a["position"][i].as_f64().unwrap()
                                        }),
                                        rotation: rotation.unwrap(),
                                        velocity: std::array::from_fn(|i| {
                                            a["velocity"][i].as_f64().unwrap()
                                        }),
                                        ground: true,
                                    },
                                };
                                body.apply(version, ordinal, packet).unwrap();
                                let key = body.key;
                                update_received_geometry(&mut world, key);
                            }
                            "cancel" => {
                                world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap()
                                    .motion
                                    .cancel()
                                    .unwrap();
                            }
                            "velocity" => world
                                .bodies
                                .values_mut()
                                .find(|b| b.native_id == id)
                                .unwrap()
                                .apply(
                                    version,
                                    ordinal,
                                    LivingMotionPacket::Velocity(std::array::from_fn(|i| {
                                        a["velocity"][i].as_f64().unwrap()
                                    })),
                                )
                                .unwrap(),
                            "team" => {
                                let b = world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap();
                                b.team = Some((
                                    "OwnerContact".into(),
                                    if a["rule"] == "never" {
                                        crate::client::ui::TeamCollision::Never
                                    } else {
                                        crate::client::ui::TeamCollision::Always
                                    },
                                ));
                            }
                            "wolf_flags" => {
                                let b = world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap();
                                b.wolf_interest(a["interested"].as_bool().unwrap());
                                assert_bounds(b.contact().bounds, &a["pose"], &context);
                            }
                            "variant" => assert_variant_geometry(&world, id, a),
                            "pose_metadata" => apply_pose_metadata(&mut world, id, a),
                            "baby" => apply_ageable_baby(&mut world, id, a),
                            "sleep" => apply_sleep(&mut world, id, &a["position"]),
                            "local" | "freeze" => {}
                            "unticking" | "reticking" => {
                                let key = world
                                    .bodies
                                    .values()
                                    .find(|b| b.native_id == id)
                                    .unwrap()
                                    .key;
                                let mut body = world.order.body(key).unwrap().clone();
                                body.legacy_ticking_chunk = Some(a["op"] == "reticking");
                                world.order.update(body).unwrap();
                            }
                            op => panic!("unhandled oracle operation {op}"),
                        }
                    }
                    let player_tick = step["ticks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|t| t["dispatch"] == "player_tick")
                        .unwrap();
                    if stage == 0 && (16..24).contains(&seed) {
                        let actors = step["ticks"].as_array().unwrap();
                        let own_slot = actors
                            .iter()
                            .position(|t| t["dispatch"] == "player_tick")
                            .unwrap();
                        let spawn = first["actions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|a| a["op"] == "spawn")
                            .unwrap();
                        let bounds = &spawn["pose"]["bounds"];
                        let own_bounds = &spawn_player["pose"]["bounds"];
                        assert!((0..3).all(|i| bounds[i].as_f64().unwrap()
                            < own_bounds[i + 3].as_f64().unwrap()
                            && bounds[i + 3].as_f64().unwrap() > own_bounds[i].as_f64().unwrap()));
                        let v = usize::from(version == MinecraftVersion::Java1_21_11);
                        let species = usize::from(seed >= 20);
                        if own_slot > 0
                            && velocity(&player_tick["before"]) != velocity(&spawn_player["pose"])
                        {
                            pushed_slots[v][species][0] = true;
                        }
                        if own_slot + 1 < actors.len()
                            && velocity(&step["local_after"]) != velocity(&player_tick["after"])
                        {
                            pushed_slots[v][species][1] = true;
                        }
                    }
                    world
                        .tick(
                            &mut local,
                            (None, false),
                            version == MinecraftVersion::Java1_21_11 && stage >= 8,
                            |local| {
                                assert_vector(
                                    local.push.position,
                                    pose(&player_tick["before"]).position,
                                    &context,
                                );
                                assert_vector(
                                    local.push.velocity,
                                    velocity(&player_tick["before"]),
                                    &context,
                                );
                                own.velocity = local.push.velocity;
                                let p = pose(&player_tick["before"]);
                                physics::tick(
                                    version,
                                    &mut own,
                                    &Environment::defaults(version),
                                    Controls {
                                        yaw: p.rotation[0],
                                        pitch: p.rotation[1],
                                        ..Default::default()
                                    },
                                    &mut |p| {
                                        Ok(crate::NativeBlockState {
                                            name: if p[1] == 64
                                                && (-16..=32).contains(&p[0])
                                                && (-16..=16).contains(&p[2])
                                            {
                                                "minecraft:stone"
                                            } else {
                                                "minecraft:air"
                                            }
                                            .into(),
                                            properties: Default::default(),
                                        })
                                    },
                                )
                                .unwrap();
                                local.push.position = own.position;
                                local.push.velocity = own.velocity;
                                local.bounds = box_value(own.bounds);
                                assert_vector(
                                    own.position,
                                    pose(&player_tick["after"]).position,
                                    &context,
                                );
                                assert_vector(
                                    own.velocity,
                                    velocity(&player_tick["after"]),
                                    &context,
                                );
                                Ok(())
                            },
                            |body| {
                                Ok((version == MinecraftVersion::Java1_21_11
                                    && body.motion.frame().interpolation.remaining_steps != 0)
                                    .then_some(true))
                            },
                            |_| Ok(()),
                        )
                        .unwrap();
                    own.velocity = local.push.velocity;
                    assert_vector(
                        local.push.velocity,
                        velocity(&step["local_after"]),
                        &context,
                    );
                    for state in step["states"].as_array().unwrap() {
                        let body = &world.bodies[&state["key"].as_u64().unwrap()];
                        assert_vector(
                            body.motion.frame().interpolation.pose.position,
                            pose(&state["pose"]).position,
                            &context,
                        );
                        assert_vector(body.velocity, velocity(&state["pose"]), &context);
                        assert_bounds(body.contact().bounds, &state["pose"], &context);
                        assert_chicken(body, &state["pose"], &context);
                    }
                    stages += 1;
                }
            }
        }
        assert_eq!((stages, unmodeled_stages), (1440, 672));
        assert_eq!(pushed_slots, [[[true; 2]; 2]; 2]);
    }

    #[test]
    fn native_boat_uses_living_geometry_at_its_original_dispatch_slot() {
        let fixture: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        let mut stages = 0;
        let mut unmodeled_stages = 0;
        for (name, version) in [
            ("1.16.1", MinecraftVersion::Java1_16_1),
            ("1.21.11", MinecraftVersion::Java1_21_11),
        ] {
            for row in fixture["versions"][name]["results"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["scene"] == "boat")
            {
                if !qualified_scene(version, row) {
                    unmodeled_stages += row["steps"].as_array().unwrap().len();
                    continue;
                }
                let actions = row["steps"][0]["actions"].as_array().unwrap();
                let own = actions.iter().find(|a| a["op"] == "spawn_player").unwrap();
                let root = actions.iter().find(|a| a["op"] == "spawn_boat").unwrap();
                let own_key = own["key"].as_u64().unwrap();
                let root_key = root["key"].as_u64().unwrap();
                let mut world = ContactTickWorld::new(version, own_key);
                let mut seed = crate::client::BoatFrame::new(
                    pose(&root["pose"]).position,
                    pose(&root["pose"]).rotation,
                    velocity(&root["pose"]),
                );
                let mut ordinal = 0;
                for step in row["steps"].as_array().unwrap() {
                    let context =
                        format!("{name} boat seed {} stage {}", row["seed"], step["stage"]);
                    let mut input = crate::client::VehicleInput::default();
                    for a in step["actions"].as_array().unwrap() {
                        ordinal += 1;
                        let id = a["native_id"].as_i64().unwrap() as i32;
                        match a["op"].as_str().unwrap() {
                            "spawn" => {
                                let key = a["key"].as_u64().unwrap();
                                let name = a["type_name"].as_str().unwrap();
                                let player = name == "minecraft:player";
                                let d = super::super::geometry::shape(
                                    version,
                                    name,
                                    super::super::EntityPose::Standing,
                                    1.,
                                    1.,
                                );
                                world
                                    .admit(
                                        tick_body(key, id, player),
                                        Some(
                                            ContactTickBody::new(
                                                version,
                                                (key, id),
                                                if player {
                                                    ContactDispatch::RemotePlayer
                                                } else if name == "minecraft:wolf" {
                                                    ContactDispatch::Wolf
                                                } else if name == "minecraft:chicken" {
                                                    ContactDispatch::Chicken
                                                } else {
                                                    ContactDispatch::InheritedLiving
                                                },
                                                pose(&a["pose"]),
                                                velocity(&a["pose"]),
                                                ordinal,
                                                [d.0, d.1],
                                            )
                                            .unwrap(),
                                        ),
                                    )
                                    .unwrap();
                            }
                            "spawn_player" => {
                                world.admit(tick_body(own_key, id, true), None).unwrap()
                            }
                            "spawn_boat" => {
                                let mut root = tick_body(root_key, id, false);
                                root.passengers = vec![own_key];
                                root.player_passengers = 1;
                                world.admit(root, None).unwrap();
                                let mut own = world.order.body(own_key).unwrap().clone();
                                own.passenger_of = Some(root_key);
                                world.order.update(own).unwrap();
                            }
                            "boat_input" => {
                                input.forward = i8::from(a["forward"].as_bool().unwrap())
                            }
                            "relative" | "rotation" | "absolute" | "velocity" => {
                                let b = world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap();
                                let rotation = a["rotation"].as_array().map(|r| {
                                    std::array::from_fn(|i| r[i].as_f64().unwrap() as f32)
                                });
                                let packet = match a["op"].as_str().unwrap() {
                                    "velocity" => {
                                        LivingMotionPacket::Velocity(std::array::from_fn(|i| {
                                            a["velocity"][i].as_f64().unwrap()
                                        }))
                                    }
                                    "relative" => LivingMotionPacket::Relative {
                                        delta: std::array::from_fn(|i| {
                                            a["delta"][i].as_i64().unwrap() as i16
                                        }),
                                        rotation: rotation.map(|r| r.map(|v| v * 360. / 256.)),
                                        ground: a["ground"].as_bool().unwrap(),
                                    },
                                    "rotation" => LivingMotionPacket::Rotation {
                                        rotation: rotation.unwrap().map(|v| v * 360. / 256.),
                                        ground: a["ground"].as_bool().unwrap(),
                                    },
                                    _ if version == MinecraftVersion::Java1_16_1 => {
                                        LivingMotionPacket::LegacyTeleport {
                                            position: std::array::from_fn(|i| {
                                                a["position"][i].as_f64().unwrap()
                                            }),
                                            rotation: rotation.unwrap(),
                                            ground: true,
                                        }
                                    }
                                    _ => LivingMotionPacket::ModernPositionSync {
                                        position: std::array::from_fn(|i| {
                                            a["position"][i].as_f64().unwrap()
                                        }),
                                        rotation: rotation.unwrap(),
                                        velocity: std::array::from_fn(|i| {
                                            a["velocity"][i].as_f64().unwrap()
                                        }),
                                        ground: true,
                                    },
                                };
                                b.apply(version, ordinal, packet).unwrap();
                                let key = b.key;
                                update_received_geometry(&mut world, key);
                            }
                            "cancel" => {
                                world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap()
                                    .motion
                                    .cancel()
                                    .unwrap();
                            }
                            "team" => {
                                let b = world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap();
                                b.team = Some((
                                    "OwnerContact".into(),
                                    if a["rule"] == "never" {
                                        crate::client::ui::TeamCollision::Never
                                    } else {
                                        crate::client::ui::TeamCollision::Always
                                    },
                                ));
                            }
                            "wolf_flags" => {
                                let b = world
                                    .bodies
                                    .values_mut()
                                    .find(|b| b.native_id == id)
                                    .unwrap();
                                b.wolf_interest(a["interested"].as_bool().unwrap());
                                assert_bounds(b.contact().bounds, &a["pose"], &context);
                            }
                            "variant" => assert_variant_geometry(&world, id, a),
                            "pose_metadata" => apply_pose_metadata(&mut world, id, a),
                            "baby" => apply_ageable_baby(&mut world, id, a),
                            "sleep" => apply_sleep(&mut world, id, &a["position"]),
                            "local" => {}
                            "unticking" | "reticking" => {
                                let b = world.bodies.values().find(|b| b.native_id == id).unwrap();
                                let mut tick = world.order.body(b.key).unwrap().clone();
                                tick.legacy_ticking_chunk = Some(a["op"] == "reticking");
                                world.order.update(tick).unwrap();
                            }
                            op => panic!("unhandled {op}"),
                        }
                    }
                    let expected = step["ticks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|t| t["dispatch"] == "boat_tick")
                        .unwrap();
                    let own_box = std::cell::RefCell::new(seed.bounds);
                    let frame = world
                        .tick_geometry(
                            root_key,
                            |bodies, index| {
                                for x in -4..=4 {
                                    for z in -4..=4 {
                                        index.set_chunk_known([x, z], true).unwrap();
                                    }
                                }
                                let query = super::super::owned_contacts::boat_query(seed.bounds);
                                let keys = index
                                    .geometry_keys(query, &[root_key])
                                    .unwrap()
                                    .into_iter()
                                    .filter(|key| bodies[key].pushable)
                                    .collect::<Vec<_>>();
                                let expected_keys = expected["geometry_query"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|key| key.as_u64().unwrap())
                                    .collect::<Vec<_>>();
                                assert_eq!(keys, expected_keys, "{context} original spatial query");
                                assert_vector(
                                    seed.position,
                                    pose(&expected["before"]).position,
                                    &context,
                                );
                                let boxes = bodies
                                    .values()
                                    .filter(|b| b.pushable)
                                    .map(|b| {
                                        let b = b.contact().bounds;
                                        [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
                                    })
                                    .collect::<Vec<_>>();
                                let frame =
                                    physics::boat::tick(version, &seed, input, &boxes, &mut block)
                                        .unwrap();
                                *own_box.borrow_mut() = frame.bounds;
                                Ok(frame)
                            },
                            |b| {
                                if version == MinecraftVersion::Java1_16_1
                                    || b.motion.frame().interpolation.remaining_steps == 0
                                {
                                    return Ok(None);
                                }
                                let Some(q) = b.motion.collision_query_position() else {
                                    return Ok(Some(true));
                                };
                                let h = f64::from(b.width / 2.);
                                let box_ = [
                                    q[0] - h,
                                    q[1],
                                    q[2] - h,
                                    q[0] + h,
                                    q[1] + f64::from(b.height),
                                    q[2] + h,
                                ];
                                let other = *own_box.borrow();
                                Ok(Some(
                                    physics::contact_block_collision_free(
                                        version,
                                        box_,
                                        b.motion.frame().interpolation.pose.position[1],
                                        &mut block,
                                    )
                                    .unwrap()
                                        && !(0..3).all(|i| {
                                            box_[i] < other[i + 3] && box_[i + 3] > other[i]
                                        }),
                                ))
                            },
                            |_| Ok(()),
                        )
                        .unwrap();
                    assert_vector(frame.position, pose(&expected["after"]).position, &context);
                    assert_vector(frame.velocity, velocity(&expected["after"]), &context);
                    seed = frame;
                    for state in step["states"].as_array().unwrap() {
                        let body = &world.bodies[&state["key"].as_u64().unwrap()];
                        assert_chicken(body, &state["pose"], &context);
                        assert_bounds(
                            world.bodies[&state["key"].as_u64().unwrap()]
                                .contact()
                                .bounds,
                            &state["pose"],
                            &context,
                        );
                        assert_vector(
                            world.bodies[&state["key"].as_u64().unwrap()]
                                .motion
                                .frame()
                                .interpolation
                                .pose
                                .position,
                            pose(&state["pose"]).position,
                            &context,
                        );
                    }
                    stages += 1;
                }
            }
        }
        assert_eq!((stages, unmodeled_stages), (1440, 672));
    }
    fn update_received_geometry(world: &mut ContactTickWorld, key: u64) {
        let b = &world.bodies[&key];
        if world.version == MinecraftVersion::Java1_16_1 {
            world
                .spatial
                .set_bounds_in_listed_section(key, b.contact().bounds)
                .unwrap();
        } else {
            index_body(&mut world.spatial, b).unwrap();
        }
    }

    #[test]
    fn owner_oracle_binds_unchanged_original_inputs_and_own_sources() {
        use sha2::Digest;
        let x: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        assert_eq!(
            x["runner_sha256"],
            format!(
                "{:x}",
                sha2::Sha256::digest(include_bytes!(
                    "../../../scripts/run_entity_owner_oracle.py"
                ))
            )
        );
        for (v, source, jar) in [
            (
                "1.16.1",
                &include_bytes!(
                    "../../../scripts/entity_owner_oracle/1.16.1/EntityTickOracle.java"
                )[..],
                "c9abbe8ee4fa490751ca70635340b7cf00db83ff",
            ),
            (
                "1.21.11",
                &include_bytes!(
                    "../../../scripts/entity_owner_oracle/1.21.11/EntityTickOracle.java"
                )[..],
                "ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd",
            ),
        ] {
            assert_eq!(
                x["versions"][v]["harness_sha256"],
                format!("{:x}", sha2::Sha256::digest(source))
            );
            assert_eq!(x["versions"][v]["original_client_jar_sha1"], jar);
            let policy: Value = serde_json::from_str(include_str!(
                "../../../docs/evidence/contact-capabilities-20261015.json"
            ))
            .unwrap();
            let types = x["versions"][v]["results"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|row| row["steps"].as_array().unwrap())
                .flat_map(|step| step["actions"].as_array().unwrap())
                .filter(|action| action["op"] == "spawn")
                .map(|action| action["type_name"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>();
            for (name, entry) in policy["versions"][v]["types"].as_object().unwrap() {
                if entry["classifications"]["walking"] == "Qualified" {
                    assert!(
                        types.contains(format!("minecraft:{name}").as_str()),
                        "{v} {name} missing from numerical oracle"
                    );
                }
            }
        }
    }

    #[test]
    fn original_client_exclusions_keep_bat_and_armor_stand_distinct_from_vehicle_hooks() {
        let x: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        for version in ["1.16.1", "1.21.11"] {
            let rows = x["versions"][version]["exclusions"].as_array().unwrap();
            assert_eq!(rows.len(), 4);
            for row in rows {
                assert_eq!(row["pushable"], false, "{version} {row}");
                if version == "1.21.11" {
                    assert_eq!(row["solid"], false);
                }
                assert_vector(velocity(&row["before"]), velocity(&row["after"]), version);
            }
        }
    }

    #[test]
    fn original_wolf_tick_wrapper_states_have_no_contact_motion() {
        let fixture: Value = serde_json::from_reader(flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_owner_oracle.json.gz")[..],
        ))
        .unwrap();
        for (name, version) in [
            ("1.16.1", MinecraftVersion::Java1_16_1),
            ("1.21.11", MinecraftVersion::Java1_21_11),
        ] {
            let mut state = WolfAnimation::default();
            let rows = fixture["versions"][name]["wolf_states"].as_array().unwrap();
            assert_eq!(rows.len(), 80);
            for (tick, row) in rows.iter().enumerate() {
                match row["event"].as_i64().unwrap() {
                    8 => {
                        state.shaking = true;
                        state.shake = 0.;
                        state.old_shake = 0.;
                    }
                    56 => state.cancel_shake(),
                    0 => {}
                    _ => panic!("unknown wolf event"),
                }
                state.interested = row["interested"].as_bool().unwrap();
                state.tick(version, row["wet_input"].as_bool().unwrap());
                let expected: Vec<u32> = row["bits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_u64().unwrap() as u32)
                    .collect();
                assert_eq!(
                    [state.angle, state.old_angle, state.shake, state.old_shake]
                        .map(f32::to_bits)
                        .as_slice(),
                    expected,
                    "{name} tick {tick}"
                );
                assert_eq!(
                    state.wet,
                    row["wet"].as_bool().unwrap(),
                    "{name} tick {tick}"
                );
                assert_eq!(
                    state.shaking,
                    row["shaking"].as_bool().unwrap(),
                    "{name} tick {tick}"
                );
                for field in ["position_bits", "velocity_bits", "bounds_bits"] {
                    assert_eq!(
                        row["before"][field], row["after"][field],
                        "{name} tick {tick} {field}"
                    );
                }
            }
        }
    }

    fn assert_chicken(body: &ContactTickBody, expected: &Value, context: &str) {
        assert_eq!(
            body.chicken.is_some(),
            expected["chicken_bits"].is_array(),
            "{context}"
        );
        if let Some(s) = &body.chicken {
            let actual = [s.flap, s.flap_speed, s.o_flap, s.o_flap_speed, s.flapping];
            let original: Vec<u32> = expected["chicken_bits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect();
            assert_eq!(actual.map(f32::to_bits).as_slice(), original, "{context}");
        }
    }

    fn assert_bounds(actual: Aabb, expected: &Value, context: &str) {
        let original = std::array::from_fn::<_, 6, _>(|i| {
            f64::from_bits(
                u64::from_str_radix(expected["bounds_bits"][i].as_str().unwrap(), 16).unwrap(),
            )
        });
        let actual = [
            actual.min_x,
            actual.min_y,
            actual.min_z,
            actual.max_x,
            actual.max_y,
            actual.max_z,
        ];
        assert_eq!(
            actual.map(f64::to_bits),
            original.map(f64::to_bits),
            "{context}"
        );
    }

    fn apply_pose_metadata(world: &mut ContactTickWorld, id: i32, action: &Value) {
        let b = world
            .bodies
            .values_mut()
            .find(|b| b.native_id == id)
            .unwrap();
        // Pose changes occur after the oracle's baby -> adult transition.
        let dimensions = super::super::geometry::shape(
            world.version,
            action["type_name"].as_str().unwrap(),
            super::super::EntityPose::from_native(
                world.version,
                action["native_pose"].as_i64().unwrap() as i32,
            )
            .unwrap(),
            1.,
            1.,
        );
        b.refresh_dimensions(world.version, [dimensions.0, dimensions.1]);
        for (axis, expected) in [(0, b.width), (1, b.height), (2, b.width)] {
            let extent = action["pose"]["bounds"][axis + 3].as_f64().unwrap()
                - action["pose"]["bounds"][axis].as_f64().unwrap();
            assert!((extent - f64::from(expected)).abs() < 1e-12);
        }
        let key = b.key;
        update_received_geometry(world, key);
    }

    fn assert_variant_geometry(world: &ContactTickWorld, id: i32, action: &Value) {
        let b = world.bodies.values().find(|b| b.native_id == id).unwrap();
        // Species metadata is numerically inert in the audited lane. Compare
        // the unchanged SDK body with the original packet callback boundary.
        assert_vector(
            b.motion.frame().interpolation.pose.position,
            pose(&action["pose"]).position,
            "species variant packet position",
        );
        assert_bounds(
            b.contact().bounds,
            &action["pose"],
            "species variant packet bounds",
        );
    }

    fn apply_ageable_baby(world: &mut ContactTickWorld, id: i32, action: &Value) {
        let b = world
            .bodies
            .values_mut()
            .find(|b| b.native_id == id)
            .unwrap();
        let factor = if action["value"].as_bool().unwrap() {
            0.5
        } else {
            1.
        };
        let dimensions = super::super::geometry::shape(
            world.version,
            action["type_name"].as_str().unwrap_or("minecraft:sheep"),
            super::super::EntityPose::Standing,
            factor,
            1.,
        );
        b.refresh_dimensions(world.version, [dimensions.0, dimensions.1]);
        // Legacy refreshDimensions temporarily anchors a growing box at its
        // old minimum; original aiStep/setPos rebuilds it before contact.
        // Compare native dimension extents at the metadata boundary.
        for (axis, expected) in [
            (0, f64::from(b.width)),
            (1, f64::from(b.height)),
            (2, f64::from(b.width)),
        ] {
            let extent = action["bounds"][axis + 3].as_f64().unwrap()
                - action["bounds"][axis].as_f64().unwrap();
            assert!((extent - expected).abs() < 1e-12);
        }
        let key = b.key;
        update_received_geometry(world, key);
    }

    fn apply_sleep(world: &mut ContactTickWorld, id: i32, position: &Value) {
        let b = world
            .bodies
            .values_mut()
            .find(|b| b.native_id == id)
            .unwrap();
        b.sleeping = !position.is_null();
        if b.sleeping {
            let mut pose = b.motion.frame().interpolation.pose;
            pose.position = [
                position[0].as_f64().unwrap() + 0.5,
                position[1].as_f64().unwrap() + 0.6875,
                position[2].as_f64().unwrap() + 0.5,
            ];
            b.set_model_pose(pose).unwrap();
            let contact = b.contact();
            if world.version == MinecraftVersion::Java1_16_1 {
                world
                    .spatial
                    .set_bounds_in_listed_section(b.key, contact.bounds)
                    .unwrap();
            } else {
                index_body(&mut world.spatial, b).unwrap();
            }
        }
    }
    fn block(p: [i32; 3]) -> crate::Result<crate::NativeBlockState> {
        Ok(crate::NativeBlockState {
            name: if p[1] == 64 && (-16..=32).contains(&p[0]) && (-16..=16).contains(&p[2]) {
                "minecraft:stone"
            } else {
                "minecraft:air"
            }
            .into(),
            properties: Default::default(),
        })
    }

    fn box_value(b: [f64; 6]) -> Aabb {
        Aabb {
            min_x: b[0],
            min_y: b[1],
            min_z: b[2],
            max_x: b[3],
            max_y: b[4],
            max_z: b[5],
        }
    }
}
