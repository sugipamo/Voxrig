//! Connection settings applied by every supported adapter.

use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `Server`.
pub struct Server {
    /// The `host` value.
    pub host: String,
    /// The `port` value.
    pub port: u16,
}
impl Server {
    /// Performs the `new` operation.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}
impl Default for Server {
    fn default() -> Self {
        Self::new("127.0.0.1", 25565)
    }
}

/// Resource and timeout limits for Client connections.
/// Native-only limits state their supported adapters explicitly.
///
/// ```no_run
/// use voxrig::client::prelude::*;
/// async fn connect() -> Result<Client> {
///     let mut config = ConnectionConfig::offline(
///         Server::default(), "AgentOne", MinecraftVersion::Java1_16_1,
///     );
///     config.limits.chunk_capacity = voxrig::client::ChunkCapacity::Fixed(1058);
///     config.limits.native_event_channel_capacity = Some(8192);
///     Client::connect(config).await
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ClientLimits {
    /// TCP connection deadline.
    pub connect_timeout: Duration,
    /// Maximum idle interval between login/configuration packets.
    pub login_packet_timeout: Duration,
    /// Maximum idle interval between play packets.
    pub play_packet_timeout: Duration,
    /// Deadline for initial playable state.
    pub ready_timeout: Duration,
    /// Received-view column retention, including light-only columns.
    pub chunk_capacity: crate::client::ChunkCapacity,
    /// Maximum remote bodies in the per-tick player contact scope (default 256).
    /// Excluded native slots do not consume this budget. Zero refuses any contact.
    pub max_contact_bodies: usize,
    /// Capacity requested for the native event broadcast source, currently
    /// supported only by Java 1.16.1. `None` preserves the native default (256).
    /// `Some(8192)` preserves an 8192-event source during migration to Client.
    /// Zero and values above `usize::MAX / 2` fail before network I/O; other
    /// versions reject an explicit setting instead of silently ignoring it.
    /// Tokio may round up to a power of two. This does not change the common
    /// 4096-event notification log or turn it into a history of native payloads.
    pub native_event_channel_capacity: Option<usize>,
}
impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            login_packet_timeout: Duration::from_secs(15),
            play_packet_timeout: Duration::from_secs(60),
            ready_timeout: Duration::from_secs(15),
            chunk_capacity: crate::client::ChunkCapacity::default(),
            max_contact_bodies: 256,
            native_event_channel_capacity: None,
        }
    }
}
impl ClientLimits {
    pub(crate) fn legacy(self) -> crate::versions::java_1_16_1::ConnectionOptions {
        crate::versions::java_1_16_1::ConnectionOptions {
            connect_timeout: self.connect_timeout,
            login_packet_timeout: self.login_packet_timeout,
            play_packet_timeout: self.play_packet_timeout,
            ready_timeout: self.ready_timeout,
            chunk_capacity: self.chunk_capacity,
            max_contact_bodies: self.max_contact_bodies,
            event_channel_capacity: self.native_event_channel_capacity.unwrap_or_else(|| {
                crate::versions::java_1_16_1::ConnectionOptions::default().event_channel_capacity
            }),
            ..Default::default()
        }
    }
}
