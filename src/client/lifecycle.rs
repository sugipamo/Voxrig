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

/// Where the connection is in its lifecycle, as the client sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum ConnectionStatus {
    /// Login, configuration or the first world synchronization is in progress.
    Joining,
    /// Playable: operations may be admitted (each still checks its own preconditions).
    Ready,
    /// Shutdown began; no new operation is admitted.
    Closing,
    /// The connection ended. See `Client::disconnect_reason` for a server kick.
    Closed,
    /// The transport ended in a way whose delivery effects cannot be classified.
    Unknown,
}
