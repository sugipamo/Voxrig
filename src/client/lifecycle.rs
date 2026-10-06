//! Local lifecycle facts without rebuilding observation or execution authority.
use crate::MinecraftVersion;

/// Local, irreversible fencing of one transport connection, shared by all clones.
///
/// This is not clean logout, confirmed transport closure, cancellation of server
/// effects, or permission to reuse the old connection. Already admitted writes
/// can have partial or unknown effects. A new Client needs a new connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ConnectionRevocation {
    version: MinecraftVersion,
    connection_id: u64,
}
impl ConnectionRevocation {
    pub(crate) fn new(version: MinecraftVersion, connection_id: u64) -> Self {
        Self {
            version,
            connection_id,
        }
    }
    /// Immutable protocol and registry version of the fenced transport.
    #[must_use]
    pub const fn version(self) -> MinecraftVersion {
        self.version
    }
    /// Process-local transport identity, independent of world/configuration changes.
    #[must_use]
    pub const fn connection_id(self) -> u64 {
        self.connection_id
    }
}
crate::diagnostic_projection::identity!(ConnectionRevocation);
