//! Shared coordinates and interaction selectors. Physics rules remain version-specific.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
/// Version-independent BlockPos representation.
pub struct BlockPos {
    /// The `x` value.
    pub x: i32,
    /// The `y` value.
    pub y: i32,
    /// The `z` value.
    pub z: i32,
}

impl BlockPos {
    /// Performs the `packed` operation.
    pub fn packed(self) -> u64 {
        ((self.x as u64 & 0x3ff_ffff) << 38)
            | ((self.z as u64 & 0x3ff_ffff) << 12)
            | (self.y as u64 & 0xfff)
    }

    /// Performs the `unpack` operation.
    pub fn unpack(value: u64) -> Self {
        let x = ((value as i64) >> 38) as i32;
        let y = ((value & 0xfff) as i32) << 20 >> 20;
        let z = (((value >> 12) & 0x3ff_ffff) as i32) << 6 >> 6;
        Self { x, y, z }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[repr(i32)]
/// Version-independent Hand representation.
pub enum Hand {
    /// Documentation for this public variant.
    Main = 0,
    /// Documentation for this public variant.
    Off = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[repr(i8)]
/// Version-independent BlockFace representation.
pub enum BlockFace {
    /// Documentation for this public variant.
    Down = 0,
    /// Documentation for this public variant.
    Up = 1,
    /// Documentation for this public variant.
    North = 2,
    /// Documentation for this public variant.
    South = 3,
    /// Documentation for this public variant.
    West = 4,
    /// Documentation for this public variant.
    East = 5,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
/// Version-independent Vec3 representation.
pub struct Vec3 {
    /// The `x` value.
    pub x: f64,
    /// The `y` value.
    pub y: f64,
    /// The `z` value.
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
/// Version-independent Aabb representation.
pub struct Aabb {
    /// The `min_x` value.
    pub min_x: f64,
    /// The `min_y` value.
    pub min_y: f64,
    /// The `min_z` value.
    pub min_z: f64,
    /// The `max_x` value.
    pub max_x: f64,
    /// The `max_y` value.
    pub max_y: f64,
    /// The `max_z` value.
    pub max_z: f64,
}
