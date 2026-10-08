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

/// Resource limits with the same meaning on every Client adapter.
/// Version-specific cache/event/ACK limits stay in the version-specific API.
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
    /// Maximum retained chunks per connection.
    pub max_chunks: usize,
}
impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            login_packet_timeout: Duration::from_secs(15),
            play_packet_timeout: Duration::from_secs(60),
            ready_timeout: Duration::from_secs(15),
            max_chunks: 256,
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
            max_chunks: self.max_chunks,
            ..Default::default()
        }
    }
}
