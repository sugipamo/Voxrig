//! Actual received own-player passenger relationships, separate from motion.
use super::{EntityId, ObservedValue, SessionStamp, entity::SpawnLedger, received};
use crate::protocol::get_varint;
pub mod control;
pub use control::{
    MAX_VEHICLE_CONTROL_TICKS, VehicleControlId, VehicleControlRecord, VehicleControlStage,
    VehicleInput,
};
pub mod dismount;
pub use dismount::{DismountGrounding, DismountId, DismountRecord, DismountStage};

/// One continuous mounted lifetime on one connection/world. It cannot be restored
/// from saved diagnostics or constructed from a reusable numeric vehicle ID.
/// ```compile_fail
/// let mount: voxrig::client::MountId = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MountId {
    session: SessionStamp,
    player_native_id: i32,
    vehicle_native_id: i32,
    receive_sequence: u64,
    vehicle: Option<EntityId>,
}
impl MountId {
    /// Owning connection/world.
    pub fn session(self) -> SessionStamp {
        self.session
    }
    /// Original native vehicle ID; this integer cannot construct a live mount.
    pub fn native_vehicle_id(self) -> i32 {
        self.vehicle_native_id
    }
    /// Original SET_PASSENGERS ordinal, separate from a server tick.
    pub fn receive_sequence(self) -> u64 {
        self.receive_sequence
    }
    /// Spawn lifetime known at the mounted receipt, absent if not received.
    /// A later spawn is never retroactively attached to this mount.
    pub fn vehicle(self) -> Option<EntityId> {
        self.vehicle
    }
}
/// The own player's last explicit passenger relationship. None at the outer
/// observation means unknown, rather than evidence of being unmounted.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VehicleRelation {
    /// The actual list included the own player.
    Mounted {
        /// Original receipt, tied to the vehicle lifetime if it was known.
        mount: MountId,
    },
    /// A later list for the same vehicle excluded its previously observed player.
    /// This is a received relationship change, not a stationary-motion admission.
    Unmounted {
        /// Original mounted receipt to which the absence applies.
        previous_mount: MountId,
    },
}
/// Coherent own-player passenger observation. No position, vehicle physics,
/// server-current pose, complete entity catalogue or dispatch ACK is inferred.
#[derive(Clone, Debug, serde::Serialize)]
pub struct VehicleObservation {
    /// Connection/world at capture.
    pub session: SessionStamp,
    /// Capture boundary, separate from the relation's actual source ordinal.
    pub receive_sequence: u64,
    /// Native own player identity actually received in JOIN/LOGIN.
    pub player_native_id: Option<i32>,
    /// Last explicit own-player relationship; None means no applicable receipt.
    pub relation: Option<ObservedValue<VehicleRelation>>,
    /// Exact ordered native passenger list from the relation's source packet.
    /// Missing spawns are not replaced by invented entity lifetimes.
    pub passengers: Option<ObservedValue<Vec<i32>>>,
}

pub(crate) struct NativePassengers {
    pub vehicle: i32,
    pub passengers: Vec<i32>,
}
impl NativePassengers {
    /// Both pinned protocols encode vehicle VarInt + VarInt-counted passenger IDs.
    /// Fully validate the frame before either adapter mutates its state.
    pub(crate) fn decode(payload: &[u8]) -> anyhow::Result<Self> {
        let mut r = payload;
        let vehicle = get_varint(&mut r)?;
        anyhow::ensure!(vehicle >= 0, "invalid vehicle entity ID");
        let mut passengers = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let count = get_varint(&mut r)?;
        anyhow::ensure!((0..=1024).contains(&count), "invalid passenger count");
        for _ in 0..count {
            let passenger = get_varint(&mut r)?;
            anyhow::ensure!(passenger >= 0, "invalid passenger entity ID");
            anyhow::ensure!(passenger != vehicle, "vehicle cannot be its own passenger");
            anyhow::ensure!(seen.insert(passenger), "duplicate passenger entity ID");
            passengers.push(passenger);
        }
        anyhow::ensure!(r.is_empty(), "trailing passenger bytes");
        Ok(Self {
            vehicle,
            passengers,
        })
    }
}
#[derive(Clone, Copy)]
struct NativeMount {
    player: i32,
    vehicle: i32,
    sequence: u64,
    spawn_sequence: Option<u64>,
}
#[derive(Clone, Copy)]
struct NativeRelation {
    mount: NativeMount,
    mounted: bool,
}
#[derive(Default)]
pub(crate) struct PassengerLedger {
    relation: Option<ObservedValue<NativeRelation>>,
    passengers: Option<ObservedValue<Vec<i32>>>,
    // Removing the player from a list does not prove ordinary standing geometry.
    motion_interrupted: bool,
}
impl PassengerLedger {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn retire(&mut self, vehicle: i32) {
        if self
            .relation
            .as_ref()
            .is_some_and(|r| r.value.mount.vehicle == vehicle)
        {
            // Despawn/reuse is not an explicit unmounted receipt.
            self.relation = None;
            self.passengers = None;
        }
    }
    /// Only a spawn already known at the actual continuous mounted receipt.
    pub(crate) fn mounted_entity(&self, player: Option<i32>, spawns: &SpawnLedger) -> Option<i32> {
        let relation = self.relation.as_ref()?.value;
        let mount = relation.mount;
        (relation.mounted
            && player == Some(mount.player)
            && mount.spawn_sequence.is_some()
            && mount.spawn_sequence == spawns.spawn_sequence(mount.vehicle))
        .then_some(mount.vehicle)
    }
    pub(crate) fn motion_interrupted(&self) -> bool {
        self.motion_interrupted
    }
    /// Only the explicitly validated finite ground owner may clear this local fence.
    /// Actual relation and all receipt ordinals are retained.
    pub(crate) fn admit_ground_after_dismount(&mut self, mount: MountId) -> crate::Result<()> {
        if !self.relation.as_ref().is_some_and(|r| {
            let m = r.value.mount;
            !r.value.mounted
                && m.player == mount.player_native_id
                && m.vehicle == mount.vehicle_native_id
                && m.sequence == mount.receive_sequence
                && m.spawn_sequence == mount.vehicle.map(|e| e.spawn_sequence())
        }) {
            return Err(super::inventory::unavailable(
                "original dismount changed before ground admission",
            ));
        }
        self.motion_interrupted = false;
        Ok(())
    }
    pub(crate) fn receive(
        &mut self,
        update: &NativePassengers,
        player: Option<i32>,
        spawns: &SpawnLedger,
        sequence: u64,
    ) {
        let Some(player) = player else { return };
        if update.passengers.contains(&player) {
            let mount = self
                .relation
                .as_ref()
                .filter(|r| r.value.mounted)
                .map(|r| r.value.mount)
                .filter(|m| {
                    m.player == player
                        && m.vehicle == update.vehicle
                        && m.spawn_sequence == spawns.spawn_sequence(update.vehicle)
                })
                .unwrap_or(NativeMount {
                    player,
                    vehicle: update.vehicle,
                    sequence,
                    spawn_sequence: spawns.spawn_sequence(update.vehicle),
                });
            self.relation = Some(received(
                NativeRelation {
                    mount,
                    mounted: true,
                },
                sequence,
            ));
            self.passengers = Some(received(update.passengers.clone(), sequence));
            self.motion_interrupted = true;
        } else if let Some(mount) = self
            .relation
            .as_ref()
            .map(|r| r.value.mount)
            .filter(|m| m.player == player && m.vehicle == update.vehicle)
        {
            self.relation = Some(received(
                NativeRelation {
                    mount,
                    mounted: false,
                },
                sequence,
            ));
            self.passengers = Some(received(update.passengers.clone(), sequence));
        }
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        sequence: u64,
        player: Option<i32>,
        spawns: &SpawnLedger,
    ) -> VehicleObservation {
        let relation = self
            .relation
            .as_ref()
            .filter(|r| Some(r.value.mount.player) == player)
            .map(|r| {
                let native = r.value.mount;
                let mount = MountId {
                    session,
                    player_native_id: native.player,
                    vehicle_native_id: native.vehicle,
                    receive_sequence: native.sequence,
                    vehicle: spawns
                        .identity(session, native.vehicle)
                        .filter(|e| Some(e.spawn_sequence()) == native.spawn_sequence),
                };
                ObservedValue {
                    value: if r.value.mounted {
                        VehicleRelation::Mounted { mount }
                    } else {
                        VehicleRelation::Unmounted {
                            previous_mount: mount,
                        }
                    },
                    source: r.source,
                }
            });
        VehicleObservation {
            session,
            receive_sequence: sequence,
            player_native_id: player,
            passengers: relation.as_ref().and(self.passengers.clone()),
            relation,
        }
    }
}

#[cfg(test)]
mod tests;
