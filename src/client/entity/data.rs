//! Version-neutral names for synched entity data. Each version's native indices and
//! defaults come from tables exported from its official server
//! (`data/client_api/entity_data-*.json`).
use super::state::{EntityDataValue, EntityObservation};
use crate::MinecraftVersion;
use crate::client::ValueSource;
use crate::versions::table::{EntityDataDefault, EntityDataRow};

/// A synched entity-data field, named independently of each version's index.
///
/// Names follow the official server's fields. A field a type does not have in the
/// selected version reads as `None`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub enum EntityDataField {
    /// Shared flags byte: on fire 0x01, crouching 0x02, sprinting 0x08, swimming 0x10,
    /// invisible 0x20, glowing 0x40, fall flying 0x80.
    SharedFlags,
    /// Remaining air ticks.
    AirSupply,
    /// Whether the custom name is always shown.
    CustomNameVisible,
    /// Silent entity.
    Silent,
    /// Unaffected by gravity.
    NoGravity,
    /// Ticks frozen in powder snow (Java 1.21.11).
    TicksFrozen,
    /// Living flags: using an item 0x01, off hand 0x02, spin attack 0x04.
    LivingFlags,
    /// Health of a living entity.
    Health,
    /// Arrows stuck in a living entity.
    ArrowCount,
    /// Mob flags: no AI 0x01, left-handed 0x02, aggressive 0x04.
    MobFlags,
    /// Baby variant of an ageable mob, zombie, piglin or zoglin.
    Baby,
    /// Tameable flags: sitting 0x01, tame 0x04.
    TameableFlags,
    /// Creeper swell direction: -1 idle, 1 swelling.
    CreeperSwellDir,
    /// Charged creeper.
    CreeperPowered,
    /// Creeper ignited by flint and steel.
    CreeperIgnited,
    /// Enderman screaming at a target.
    EndermanCreepy,
    /// Enderman stared at by a player.
    EndermanStaredAt,
    /// Piglin charging a crossbow.
    PiglinChargingCrossbow,
    /// Polar bear standing up to attack.
    PolarBearStanding,
    /// Wither invulnerable (spawn) ticks.
    WitherInvulnerableTicks,
    /// Pufferfish puff state: 0 deflated to 2 fully puffed.
    PufferfishPuffState,
    /// Blaze flags: charged (on fire) 0x01.
    BlazeFlags,
    /// Ghast charging a fireball.
    GhastCharging,
    /// Vex flags: charging 0x01.
    VexFlags,
    /// Slime or magma cube size.
    SlimeSize,
    /// Shulker peek amount: 0 closed to 100 open.
    ShulkerPeek,
    /// Bee flags: angry 0x02 (1.16.1), stung 0x04, has nectar 0x08.
    BeeFlags,
    /// Remaining anger ticks of a wolf or bee (Java 1.16.1). See [`EntityObservation::angry`].
    RemainingAngerTime,
    /// Game time at which a wolf's or bee's anger ends, -1 when calm (Java 1.21.11).
    /// See [`EntityObservation::angry`].
    AngerEndTime,
}

impl EntityDataField {
    /// Every field.
    pub const ALL: &'static [Self] = &[
        Self::SharedFlags,
        Self::AirSupply,
        Self::CustomNameVisible,
        Self::Silent,
        Self::NoGravity,
        Self::TicksFrozen,
        Self::LivingFlags,
        Self::Health,
        Self::ArrowCount,
        Self::MobFlags,
        Self::Baby,
        Self::TameableFlags,
        Self::CreeperSwellDir,
        Self::CreeperPowered,
        Self::CreeperIgnited,
        Self::EndermanCreepy,
        Self::EndermanStaredAt,
        Self::PiglinChargingCrossbow,
        Self::PolarBearStanding,
        Self::WitherInvulnerableTicks,
        Self::PufferfishPuffState,
        Self::BlazeFlags,
        Self::GhastCharging,
        Self::VexFlags,
        Self::SlimeSize,
        Self::ShulkerPeek,
        Self::BeeFlags,
        Self::RemainingAngerTime,
        Self::AngerEndTime,
    ];

    /// Official (owner class, field) names in any supported version.
    pub const fn official_names(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::SharedFlags => &[("Entity", "DATA_SHARED_FLAGS_ID")],
            Self::AirSupply => &[("Entity", "DATA_AIR_SUPPLY_ID")],
            Self::CustomNameVisible => &[("Entity", "DATA_CUSTOM_NAME_VISIBLE")],
            Self::Silent => &[("Entity", "DATA_SILENT")],
            Self::NoGravity => &[("Entity", "DATA_NO_GRAVITY")],
            Self::TicksFrozen => &[("Entity", "DATA_TICKS_FROZEN")],
            Self::LivingFlags => &[("LivingEntity", "DATA_LIVING_ENTITY_FLAGS")],
            Self::Health => &[("LivingEntity", "DATA_HEALTH_ID")],
            Self::ArrowCount => &[("LivingEntity", "DATA_ARROW_COUNT_ID")],
            Self::MobFlags => &[("Mob", "DATA_MOB_FLAGS_ID")],
            Self::Baby => &[
                ("AgableMob", "DATA_BABY_ID"),
                ("AgeableMob", "DATA_BABY_ID"),
                ("Zombie", "DATA_BABY_ID"),
                ("Piglin", "DATA_BABY_ID"),
                ("Zoglin", "DATA_BABY_ID"),
            ],
            Self::TameableFlags => &[("TamableAnimal", "DATA_FLAGS_ID")],
            Self::CreeperSwellDir => &[("Creeper", "DATA_SWELL_DIR")],
            Self::CreeperPowered => &[("Creeper", "DATA_IS_POWERED")],
            Self::CreeperIgnited => &[("Creeper", "DATA_IS_IGNITED")],
            Self::EndermanCreepy => &[("EnderMan", "DATA_CREEPY")],
            Self::EndermanStaredAt => &[("EnderMan", "DATA_STARED_AT")],
            Self::PiglinChargingCrossbow => &[("Piglin", "DATA_IS_CHARGING_CROSSBOW")],
            Self::PolarBearStanding => &[("PolarBear", "DATA_STANDING_ID")],
            Self::WitherInvulnerableTicks => &[("WitherBoss", "DATA_ID_INV")],
            Self::PufferfishPuffState => &[("Pufferfish", "PUFF_STATE")],
            Self::BlazeFlags => &[("Blaze", "DATA_FLAGS_ID")],
            Self::GhastCharging => &[("Ghast", "DATA_IS_CHARGING")],
            Self::VexFlags => &[("Vex", "DATA_FLAGS_ID")],
            Self::SlimeSize => &[("Slime", "ID_SIZE")],
            Self::ShulkerPeek => &[("Shulker", "DATA_PEEK_ID")],
            Self::BeeFlags => &[("Bee", "DATA_FLAGS_ID")],
            Self::RemainingAngerTime => &[
                ("Wolf", "DATA_REMAINING_ANGER_TIME"),
                ("Bee", "DATA_REMAINING_ANGER_TIME"),
            ],
            Self::AngerEndTime => &[
                ("Wolf", "DATA_ANGER_END_TIME"),
                ("Bee", "DATA_ANGER_END_TIME"),
            ],
        }
    }
}

/// Where an [`EntityDataReading`] came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum EntityDataSource {
    /// A received entity-data packet.
    Received {
        /// Packet ordinal, not a tick.
        sequence: u64,
    },
    /// Not received; the type's default as the official server defines it. Java
    /// 1.21.11 servers send only values that differ from the default.
    TypeDefault,
}

/// The current value of one entity-data field.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EntityDataReading {
    /// Native index in the selected version.
    pub index: u8,
    /// Received value, or the type default.
    pub value: EntityDataValue,
    /// Where the value came from.
    pub source: EntityDataSource,
}

impl EntityDataValue {
    /// Integral value of a byte, int, long or boolean.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Byte(v) => Some(i64::from(v)),
            Self::Int(v) => Some(i64::from(v)),
            Self::Long(v) => Some(v),
            Self::Bool(v) => Some(i64::from(v)),
            _ => None,
        }
    }
}

impl EntityObservation {
    fn version(&self) -> MinecraftVersion {
        self.motion.entity.id.session().version
    }
    fn data_rows(&self) -> &'static [EntityDataRow] {
        self.motion
            .entity
            .type_name
            .as_deref()
            .map_or(&[], |name| self.version().table().entity_data(name))
    }
    fn reading(&self, row: &EntityDataRow) -> Option<EntityDataReading> {
        if let Some(received) = self.metadata.get(&row.index) {
            let ValueSource::Received { sequence } = received.source else {
                return None;
            };
            return Some(EntityDataReading {
                index: row.index,
                value: received.value.clone(),
                source: EntityDataSource::Received { sequence },
            });
        }
        if !self.metadata_complete {
            return None;
        }
        let value = match row.default {
            EntityDataDefault::Byte(v) => EntityDataValue::Byte(v),
            EntityDataDefault::Int(v) => EntityDataValue::Int(v),
            EntityDataDefault::Long(v) => EntityDataValue::Long(v),
            EntityDataDefault::Float(v) => EntityDataValue::Float(v),
            EntityDataDefault::Bool(v) => EntityDataValue::Bool(v),
            EntityDataDefault::Other | EntityDataDefault::Unknown => return None,
        };
        Some(EntityDataReading {
            index: row.index,
            value,
            source: EntityDataSource::TypeDefault,
        })
    }

    /// Current value of a named field: the latest received value, else the type's
    /// default. `None` when the type is unknown, it has no such field in this version,
    /// or the value cannot be known (a default that needs a world, a serializer the
    /// common layer does not decode, or a received packet that was not fully decoded).
    pub fn data(&self, field: EntityDataField) -> Option<EntityDataReading> {
        let names = field.official_names();
        let row = self
            .data_rows()
            .iter()
            .find(|row| names.contains(&(row.owner, row.field)))?;
        self.reading(row)
    }

    /// [`Self::data`] by official owner class and field name, for fields without an
    /// [`EntityDataField`] variant. Owner classes can be renamed between versions.
    pub fn data_by_name(&self, owner: &str, field: &str) -> Option<EntityDataReading> {
        let row = self
            .data_rows()
            .iter()
            .find(|row| row.owner == owner && row.field == field)?;
        self.reading(row)
    }

    /// Whether a wolf or bee is angry, by the official `NeutralMob.isAngry`:
    /// Java 1.16.1 compares the remaining anger time with zero; Java 1.21.11 compares
    /// the anger end time with `game_time` ([`crate::client::PlayerObservation::world_time`]),
    /// so `None` there
    /// without it. `None` for other types or when the field cannot be known.
    pub fn angry(&self, game_time: Option<i64>) -> Option<bool> {
        if let Some(remaining) = self.data(EntityDataField::RemainingAngerTime) {
            return Some(remaining.value.as_i64()? > 0);
        }
        let end = self.data(EntityDataField::AngerEndTime)?.value.as_i64()?;
        Some(end > 0 && end - game_time? > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::SessionStamp;
    use crate::client::entity::{NativeSpawn, SpawnLedger};

    fn capture(
        version: MinecraftVersion,
        name: &str,
        metadata: &[(u8, EntityDataValue)],
        complete: bool,
    ) -> EntityObservation {
        let session = SessionStamp {
            version,
            connection_id: 1,
            world_generation: 0,
        };
        let type_id = crate::client::registry::Registry::for_version(version)
            .builtin_id("minecraft:entity_type", name)
            .unwrap()
            .value();
        let mut ledger = SpawnLedger::default();
        ledger
            .insert(
                version,
                NativeSpawn {
                    living: None,
                    id: 7,
                    uuid: None,
                    type_id: Some(type_id),
                    dedicated_type_name: None,
                    position: [0.0; 3],
                },
                1,
                16,
            )
            .unwrap();
        ledger.receive_metadata(7, metadata.iter().cloned(), complete, 2);
        ledger.capture_all(version, session, 3).entities.remove(0)
    }

    #[test]
    fn named_fields_resolve_per_version_with_type_defaults() {
        // 1.21.11 index 18 is DATA_IS_IGNITED (17 in 1.16.1).
        let modern = capture(
            MinecraftVersion::Java1_21_11,
            "minecraft:creeper",
            &[(18, EntityDataValue::Bool(true))],
            true,
        );
        let ignited = modern.data(EntityDataField::CreeperIgnited).unwrap();
        assert_eq!(ignited.index, 18);
        assert_eq!(ignited.value, EntityDataValue::Bool(true));
        assert_eq!(ignited.source, EntityDataSource::Received { sequence: 2 });
        let swell = modern.data(EntityDataField::CreeperSwellDir).unwrap();
        assert_eq!(
            (swell.index, swell.value, swell.source),
            (16, EntityDataValue::Int(-1), EntityDataSource::TypeDefault)
        );
        assert_eq!(modern.data(EntityDataField::SlimeSize), None);
        assert_eq!(
            modern
                .data_by_name("Creeper", "DATA_IS_POWERED")
                .unwrap()
                .value,
            EntityDataValue::Bool(false)
        );

        let legacy = capture(
            MinecraftVersion::Java1_16_1,
            "minecraft:creeper",
            &[(17, EntityDataValue::Bool(true))],
            true,
        );
        assert_eq!(
            legacy.data(EntityDataField::CreeperIgnited).unwrap().value,
            EntityDataValue::Bool(true)
        );
        assert_eq!(legacy.data(EntityDataField::TicksFrozen), None);

        // After a packet that was not decoded to its end, absent fields are unknown.
        let truncated = capture(
            MinecraftVersion::Java1_21_11,
            "minecraft:creeper",
            &[(18, EntityDataValue::Bool(true))],
            false,
        );
        assert!(truncated.data(EntityDataField::CreeperIgnited).is_some());
        assert_eq!(truncated.data(EntityDataField::CreeperSwellDir), None);
    }

    #[test]
    fn anger_follows_each_versions_neutral_mob_rule() {
        let calm = capture(MinecraftVersion::Java1_21_11, "minecraft:wolf", &[], true);
        assert_eq!(calm.angry(Some(100)), Some(false)); // Default end time -1.
        let angry = capture(
            MinecraftVersion::Java1_21_11,
            "minecraft:wolf",
            &[(21, EntityDataValue::Long(150))],
            true,
        );
        assert_eq!(angry.angry(Some(100)), Some(true));
        assert_eq!(angry.angry(Some(150)), Some(false));
        assert_eq!(angry.angry(None), None);
        let legacy = capture(
            MinecraftVersion::Java1_16_1,
            "minecraft:bee",
            &[(17, EntityDataValue::Int(40))],
            true,
        );
        assert_eq!(legacy.angry(None), Some(true));
        let pig = capture(MinecraftVersion::Java1_21_11, "minecraft:pig", &[], true);
        assert_eq!(pig.angry(Some(0)), None);
    }

    #[test]
    fn every_field_names_an_exported_accessor() {
        for field in EntityDataField::ALL {
            let found = [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11]
                .iter()
                .any(|version| {
                    version
                        .table()
                        .entities
                        .data
                        .iter()
                        .any(|row| field.official_names().contains(&(row.owner, row.field)))
                });
            assert!(found, "{field:?}");
        }
    }
}
