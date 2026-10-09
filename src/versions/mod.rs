//! Explicit version implementations. Wire IDs and registries never select a version implicitly.

#[cfg(feature = "native")]
pub mod java_1_16_1;
#[cfg(not(feature = "native"))]
#[allow(dead_code, unused_imports, clippy::enum_variant_names)] // The native surface.
pub(crate) mod java_1_16_1;
#[cfg(feature = "native")]
pub mod java_1_21_11;
#[cfg(not(feature = "native"))]
#[allow(dead_code, unused_imports, clippy::enum_variant_names)] // The native surface.
pub(crate) mod java_1_21_11;
pub(crate) mod table;

/// Explicit Minecraft wire and registry version for one connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum MinecraftVersion {
    /// Existing Java 1.16.1 implementation (protocol 736).
    Java1_16_1,
    /// Java 1.21.11 (protocol 774); availability is checked before connecting.
    Java1_21_11,
}

impl MinecraftVersion {
    /// The game's exact release name.
    pub const fn name(self) -> &'static str {
        self.table().name
    }

    /// The exact wire protocol number, not a runtime capability declaration.
    pub const fn protocol(self) -> i32 {
        self.table().protocol
    }
}

impl std::fmt::Display for MinecraftVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
impl std::str::FromStr for MinecraftVersion {
    type Err = crate::Error;
    fn from_str(value: &str) -> crate::Result<Self> {
        match value {
            "1.16.1" => Ok(Self::Java1_16_1),
            "1.21.11" => Ok(Self::Java1_21_11),
            _ => Err(crate::Error::new(
                crate::ErrorKind::Unsupported,
                anyhow::anyhow!(
                    "unsupported Minecraft version {value:?}; update Voxrig to add an adapter"
                ),
            )),
        }
    }
}
