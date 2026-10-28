#[cfg(test)]
use crate::client::registry::Registry;
use crate::{MinecraftVersion, client::EntityHistoryKind};

macro_rules! entities {
    ($($variant:ident => ($name:literal, $legacy:expr, $modern:expr)),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(usize)]
        pub(crate) enum EntityType { $($variant),* }
        impl EntityType {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),*];
            pub(crate) const fn ordinal(self) -> usize { self as usize }
            #[cfg(test)]
            pub(crate) const fn name(self) -> &'static str { match self { $(Self::$variant => $name),* } }
            pub(crate) const fn native_id(self, version: MinecraftVersion) -> Option<i32> {
                match version {
                    MinecraftVersion::Java1_16_1 => match self { $(Self::$variant => $legacy),* },
                    MinecraftVersion::Java1_21_11 => match self { $(Self::$variant => $modern),* },
                }
            }
            pub(crate) fn from_name(version: MinecraftVersion, name: &str) -> Option<Self> {
                let entity = match name { $($name => Self::$variant,)* _ => return None };
                entity.native_id(version).map(|_| entity)
            }
        }
    }
}
entities! {
    AcaciaBoat => ("minecraft:acacia_boat", None, Some(0)),
    AcaciaChestBoat => ("minecraft:acacia_chest_boat", None, Some(1)),
    Allay => ("minecraft:allay", None, Some(2)),
    AreaEffectCloud => ("minecraft:area_effect_cloud", Some(0), Some(3)),
    Armadillo => ("minecraft:armadillo", None, Some(4)),
    ArmorStand => ("minecraft:armor_stand", Some(1), Some(5)),
    Arrow => ("minecraft:arrow", Some(2), Some(6)),
    Axolotl => ("minecraft:axolotl", None, Some(7)),
    BambooChestRaft => ("minecraft:bamboo_chest_raft", None, Some(8)),
    BambooRaft => ("minecraft:bamboo_raft", None, Some(9)),
    Bat => ("minecraft:bat", Some(3), Some(10)),
    Bee => ("minecraft:bee", Some(4), Some(11)),
    BirchBoat => ("minecraft:birch_boat", None, Some(12)),
    BirchChestBoat => ("minecraft:birch_chest_boat", None, Some(13)),
    Blaze => ("minecraft:blaze", Some(5), Some(14)),
    BlockDisplay => ("minecraft:block_display", None, Some(15)),
    Boat => ("minecraft:boat", Some(6), None),
    Bogged => ("minecraft:bogged", None, Some(16)),
    Breeze => ("minecraft:breeze", None, Some(17)),
    BreezeWindCharge => ("minecraft:breeze_wind_charge", None, Some(18)),
    Camel => ("minecraft:camel", None, Some(19)),
    CamelHusk => ("minecraft:camel_husk", None, Some(20)),
    Cat => ("minecraft:cat", Some(7), Some(21)),
    CaveSpider => ("minecraft:cave_spider", Some(8), Some(22)),
    CherryBoat => ("minecraft:cherry_boat", None, Some(23)),
    CherryChestBoat => ("minecraft:cherry_chest_boat", None, Some(24)),
    ChestMinecart => ("minecraft:chest_minecart", Some(46), Some(25)),
    Chicken => ("minecraft:chicken", Some(9), Some(26)),
    Cod => ("minecraft:cod", Some(10), Some(27)),
    CommandBlockMinecart => ("minecraft:command_block_minecart", Some(47), Some(29)),
    CopperGolem => ("minecraft:copper_golem", None, Some(28)),
    Cow => ("minecraft:cow", Some(11), Some(30)),
    Creaking => ("minecraft:creaking", None, Some(31)),
    Creeper => ("minecraft:creeper", Some(12), Some(32)),
    DarkOakBoat => ("minecraft:dark_oak_boat", None, Some(33)),
    DarkOakChestBoat => ("minecraft:dark_oak_chest_boat", None, Some(34)),
    Dolphin => ("minecraft:dolphin", Some(13), Some(35)),
    Donkey => ("minecraft:donkey", Some(14), Some(36)),
    DragonFireball => ("minecraft:dragon_fireball", Some(15), Some(37)),
    Drowned => ("minecraft:drowned", Some(16), Some(38)),
    Egg => ("minecraft:egg", Some(83), Some(39)),
    ElderGuardian => ("minecraft:elder_guardian", Some(17), Some(40)),
    EndCrystal => ("minecraft:end_crystal", Some(18), Some(45)),
    EnderDragon => ("minecraft:ender_dragon", Some(19), Some(43)),
    EnderPearl => ("minecraft:ender_pearl", Some(84), Some(44)),
    Enderman => ("minecraft:enderman", Some(20), Some(41)),
    Endermite => ("minecraft:endermite", Some(21), Some(42)),
    Evoker => ("minecraft:evoker", Some(22), Some(46)),
    EvokerFangs => ("minecraft:evoker_fangs", Some(23), Some(47)),
    ExperienceBottle => ("minecraft:experience_bottle", Some(85), Some(48)),
    ExperienceOrb => ("minecraft:experience_orb", Some(24), Some(49)),
    EyeOfEnder => ("minecraft:eye_of_ender", Some(25), Some(50)),
    FallingBlock => ("minecraft:falling_block", Some(26), Some(51)),
    Fireball => ("minecraft:fireball", Some(39), Some(52)),
    FireworkRocket => ("minecraft:firework_rocket", Some(27), Some(53)),
    FishingBobber => ("minecraft:fishing_bobber", Some(106), Some(156)),
    Fox => ("minecraft:fox", Some(28), Some(54)),
    Frog => ("minecraft:frog", None, Some(55)),
    FurnaceMinecart => ("minecraft:furnace_minecart", Some(48), Some(56)),
    Ghast => ("minecraft:ghast", Some(29), Some(57)),
    Giant => ("minecraft:giant", Some(30), Some(59)),
    GlowItemFrame => ("minecraft:glow_item_frame", None, Some(60)),
    GlowSquid => ("minecraft:glow_squid", None, Some(61)),
    Goat => ("minecraft:goat", None, Some(62)),
    Guardian => ("minecraft:guardian", Some(31), Some(63)),
    HappyGhast => ("minecraft:happy_ghast", None, Some(58)),
    Hoglin => ("minecraft:hoglin", Some(32), Some(64)),
    HopperMinecart => ("minecraft:hopper_minecart", Some(49), Some(65)),
    Horse => ("minecraft:horse", Some(33), Some(66)),
    Husk => ("minecraft:husk", Some(34), Some(67)),
    Illusioner => ("minecraft:illusioner", Some(35), Some(68)),
    Interaction => ("minecraft:interaction", None, Some(69)),
    IronGolem => ("minecraft:iron_golem", Some(36), Some(70)),
    Item => ("minecraft:item", Some(37), Some(71)),
    ItemDisplay => ("minecraft:item_display", None, Some(72)),
    ItemFrame => ("minecraft:item_frame", Some(38), Some(73)),
    JungleBoat => ("minecraft:jungle_boat", None, Some(74)),
    JungleChestBoat => ("minecraft:jungle_chest_boat", None, Some(75)),
    LeashKnot => ("minecraft:leash_knot", Some(40), Some(76)),
    LightningBolt => ("minecraft:lightning_bolt", Some(41), Some(77)),
    LingeringPotion => ("minecraft:lingering_potion", None, Some(106)),
    Llama => ("minecraft:llama", Some(42), Some(78)),
    LlamaSpit => ("minecraft:llama_spit", Some(43), Some(79)),
    MagmaCube => ("minecraft:magma_cube", Some(44), Some(80)),
    MangroveBoat => ("minecraft:mangrove_boat", None, Some(81)),
    MangroveChestBoat => ("minecraft:mangrove_chest_boat", None, Some(82)),
    Mannequin => ("minecraft:mannequin", None, Some(83)),
    Marker => ("minecraft:marker", None, Some(84)),
    Minecart => ("minecraft:minecart", Some(45), Some(85)),
    Mooshroom => ("minecraft:mooshroom", Some(53), Some(86)),
    Mule => ("minecraft:mule", Some(52), Some(87)),
    Nautilus => ("minecraft:nautilus", None, Some(88)),
    OakBoat => ("minecraft:oak_boat", None, Some(89)),
    OakChestBoat => ("minecraft:oak_chest_boat", None, Some(90)),
    Ocelot => ("minecraft:ocelot", Some(54), Some(91)),
    OminousItemSpawner => ("minecraft:ominous_item_spawner", None, Some(92)),
    Painting => ("minecraft:painting", Some(55), Some(93)),
    PaleOakBoat => ("minecraft:pale_oak_boat", None, Some(94)),
    PaleOakChestBoat => ("minecraft:pale_oak_chest_boat", None, Some(95)),
    Panda => ("minecraft:panda", Some(56), Some(96)),
    Parched => ("minecraft:parched", None, Some(97)),
    Parrot => ("minecraft:parrot", Some(57), Some(98)),
    Phantom => ("minecraft:phantom", Some(58), Some(99)),
    Pig => ("minecraft:pig", Some(59), Some(100)),
    Piglin => ("minecraft:piglin", Some(60), Some(101)),
    PiglinBrute => ("minecraft:piglin_brute", None, Some(102)),
    Pillager => ("minecraft:pillager", Some(61), Some(103)),
    Player => ("minecraft:player", Some(105), Some(155)),
    PolarBear => ("minecraft:polar_bear", Some(62), Some(104)),
    Potion => ("minecraft:potion", Some(86), None),
    Pufferfish => ("minecraft:pufferfish", Some(64), Some(107)),
    Rabbit => ("minecraft:rabbit", Some(65), Some(108)),
    Ravager => ("minecraft:ravager", Some(66), Some(109)),
    Salmon => ("minecraft:salmon", Some(67), Some(110)),
    Sheep => ("minecraft:sheep", Some(68), Some(111)),
    Shulker => ("minecraft:shulker", Some(69), Some(112)),
    ShulkerBullet => ("minecraft:shulker_bullet", Some(70), Some(113)),
    Silverfish => ("minecraft:silverfish", Some(71), Some(114)),
    Skeleton => ("minecraft:skeleton", Some(72), Some(115)),
    SkeletonHorse => ("minecraft:skeleton_horse", Some(73), Some(116)),
    Slime => ("minecraft:slime", Some(74), Some(117)),
    SmallFireball => ("minecraft:small_fireball", Some(75), Some(118)),
    Sniffer => ("minecraft:sniffer", None, Some(119)),
    SnowGolem => ("minecraft:snow_golem", Some(76), Some(121)),
    Snowball => ("minecraft:snowball", Some(77), Some(120)),
    SpawnerMinecart => ("minecraft:spawner_minecart", Some(50), Some(122)),
    SpectralArrow => ("minecraft:spectral_arrow", Some(78), Some(123)),
    Spider => ("minecraft:spider", Some(79), Some(124)),
    SplashPotion => ("minecraft:splash_potion", None, Some(105)),
    SpruceBoat => ("minecraft:spruce_boat", None, Some(125)),
    SpruceChestBoat => ("minecraft:spruce_chest_boat", None, Some(126)),
    Squid => ("minecraft:squid", Some(80), Some(127)),
    Stray => ("minecraft:stray", Some(81), Some(128)),
    Strider => ("minecraft:strider", Some(82), Some(129)),
    Tadpole => ("minecraft:tadpole", None, Some(130)),
    TextDisplay => ("minecraft:text_display", None, Some(131)),
    Tnt => ("minecraft:tnt", Some(63), Some(132)),
    TntMinecart => ("minecraft:tnt_minecart", Some(51), Some(133)),
    TraderLlama => ("minecraft:trader_llama", Some(88), Some(134)),
    Trident => ("minecraft:trident", Some(87), Some(135)),
    TropicalFish => ("minecraft:tropical_fish", Some(89), Some(136)),
    Turtle => ("minecraft:turtle", Some(90), Some(137)),
    Vex => ("minecraft:vex", Some(91), Some(138)),
    Villager => ("minecraft:villager", Some(92), Some(139)),
    Vindicator => ("minecraft:vindicator", Some(93), Some(140)),
    WanderingTrader => ("minecraft:wandering_trader", Some(94), Some(141)),
    Warden => ("minecraft:warden", None, Some(142)),
    WindCharge => ("minecraft:wind_charge", None, Some(143)),
    Witch => ("minecraft:witch", Some(95), Some(144)),
    Wither => ("minecraft:wither", Some(96), Some(145)),
    WitherSkeleton => ("minecraft:wither_skeleton", Some(97), Some(146)),
    WitherSkull => ("minecraft:wither_skull", Some(98), Some(147)),
    Wolf => ("minecraft:wolf", Some(99), Some(148)),
    Zoglin => ("minecraft:zoglin", Some(100), Some(149)),
    Zombie => ("minecraft:zombie", Some(101), Some(150)),
    ZombieHorse => ("minecraft:zombie_horse", Some(102), Some(151)),
    ZombieNautilus => ("minecraft:zombie_nautilus", None, Some(152)),
    ZombieVillager => ("minecraft:zombie_villager", Some(103), Some(153)),
    ZombifiedPiglin => ("minecraft:zombified_piglin", Some(104), Some(154)),
}
impl EntityType {
    #[cfg(test)]
    pub(crate) fn from_native_id(version: MinecraftVersion, id: i32) -> Option<Self> {
        let registry = Registry::for_version(version);
        let id = registry
            .builtin_id_by_native_id("minecraft:entity_type", id)
            .ok()?;
        Self::from_name(version, registry.builtin_name(&id).ok()?)
    }
}
macro_rules! axis {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(usize)]
        pub(crate) enum $name { $($variant),* }
        impl $name {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),*];
            pub(crate) const fn ordinal(self) -> usize { self as usize }
        }
    }
}
axis!(OwnerLane {
    OwnedWalking,
    OwnedGlide,
    OwnedBoat,
    PublicGeometry,
    VehicleSample
});
axis!(Effect {
    WalkingPush,
    IncomingPush,
    SolidCollision,
    Geometry,
    Metadata,
    Status,
    Animation
});
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PacketKey {
    Spawn,
    Motion,
    Metadata,
    Equipment,
    Scale,
    Status(i8),
    Animation(u8),
    Removed,
    Passengers,
    WorldChanged,
    OwnPositionCorrection,
}
impl PacketKey {
    pub(crate) const COUNT: usize = 521;
    pub(crate) const ALL: [Self; Self::COUNT] = {
        let mut keys = [Self::Spawn; Self::COUNT];
        keys[1] = Self::Motion;
        keys[2] = Self::Metadata;
        keys[3] = Self::Equipment;
        keys[4] = Self::Scale;
        let mut i = 0;
        while i < 256 {
            keys[5 + i] = Self::Status((i as i16 - 128) as i8);
            keys[261 + i] = Self::Animation(i as u8);
            i += 1;
        }
        keys[517] = Self::Removed;
        keys[518] = Self::Passengers;
        keys[519] = Self::WorldChanged;
        keys[520] = Self::OwnPositionCorrection;
        keys
    };
    pub(crate) const fn ordinal(self) -> usize {
        match self {
            Self::Spawn => 0,
            Self::Motion => 1,
            Self::Metadata => 2,
            Self::Equipment => 3,
            Self::Scale => 4,
            Self::Status(n) => 5 + (n as i16 + 128) as usize,
            Self::Animation(n) => 261 + n as usize,
            Self::Removed => 517,
            Self::Passengers => 518,
            Self::WorldChanged => 519,
            Self::OwnPositionCorrection => 520,
        }
    }
    pub(crate) fn from_history(kind: &EntityHistoryKind) -> Self {
        match kind {
            EntityHistoryKind::Spawn(_) => Self::Spawn,
            EntityHistoryKind::Motion(_) => Self::Motion,
            EntityHistoryKind::Metadata { .. } => Self::Metadata,
            EntityHistoryKind::Equipment { .. } => Self::Equipment,
            EntityHistoryKind::Scale { .. } => Self::Scale,
            EntityHistoryKind::Status { status, .. } => Self::Status(*status),
            EntityHistoryKind::Animation { animation, .. } => Self::Animation(*animation),
            EntityHistoryKind::Removed { .. } => Self::Removed,
            EntityHistoryKind::Passengers { .. } => Self::Passengers,
            EntityHistoryKind::WorldChanged { .. } => Self::WorldChanged,
            EntityHistoryKind::OwnPositionCorrection { .. } => Self::OwnPositionCorrection,
            // Own-player receipt without a remote target: it shares the existing
            // own-player key, which no policy revokes or admits as a callback.
            // The correction fence is the history kind, not this key.
            EntityHistoryKind::OwnVelocity { .. } => Self::OwnPositionCorrection,
        }
    }
}
pub(crate) const VERSIONS: [MinecraftVersion; 2] =
    [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11];
/// Dense full union product, including explicitly absent entity/version pairs.
#[cfg(test)]
pub(crate) const fn index(
    version: MinecraftVersion,
    entity: EntityType,
    lane: OwnerLane,
    effect: Effect,
    packet: PacketKey,
) -> usize {
    let version = match version {
        MinecraftVersion::Java1_16_1 => 0,
        MinecraftVersion::Java1_21_11 => 1,
    };
    ((((version * EntityType::ALL.len() + entity.ordinal()) * OwnerLane::ALL.len()
        + lane.ordinal())
        * Effect::ALL.len()
        + effect.ordinal())
        * PacketKey::COUNT)
        + packet.ordinal()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registries_match_bidirectionally() {
        for (version, source) in VERSIONS.into_iter().zip([
            include_str!("../../../../data/client_api/registry_catalog-1.16.1.json"),
            include_str!("../../../../data/client_api/registry_catalog-1.21.11.json"),
        ]) {
            let source: serde_json::Value = serde_json::from_str(source).unwrap();
            let entries = source["registries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == "minecraft:entity_type")
                .unwrap()["entries"]
                .as_array()
                .unwrap();
            assert_eq!(
                entries.len(),
                EntityType::ALL
                    .iter()
                    .filter(|e| e.native_id(version).is_some())
                    .count()
            );
            for entry in entries {
                let name = entry["name"].as_str().unwrap();
                let id = entry["id"].as_i64().unwrap() as i32;
                let entity = EntityType::from_name(version, name).unwrap();
                assert_eq!(entity.native_id(version), Some(id));
                assert_eq!(EntityType::from_native_id(version, id), Some(entity));
            }
            for &entity in EntityType::ALL {
                assert_eq!(
                    EntityType::from_name(version, entity.name()).is_some(),
                    entity.native_id(version).is_some()
                );
                if let Some(id) = entity.native_id(version) {
                    assert!(
                        entries
                            .iter()
                            .any(|e| e["name"] == entity.name() && e["id"] == id)
                    );
                }
            }
            assert_eq!(EntityType::from_native_id(version, -1), None);
            assert_eq!(EntityType::from_name(version, "minecraft:missing"), None);
        }
    }
    #[test]
    fn dense_product_is_bijective() {
        let mut next = 0;
        for version in VERSIONS {
            for &entity in EntityType::ALL {
                for &lane in OwnerLane::ALL {
                    for &effect in Effect::ALL {
                        for packet in PacketKey::ALL {
                            assert_eq!(index(version, entity, lane, effect, packet), next);
                            next += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(next, 2 * EntityType::ALL.len() * 5 * 7 * 521);
    }
}
