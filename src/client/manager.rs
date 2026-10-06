//! Common lifecycle ownership with per-connection version and registry isolation.
use super::{Client, ConnectionConfig};
use crate::{Error, ErrorKind, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, watch};
fn unavailable(message: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::State, anyhow::anyhow!("{message}"))
}
#[derive(PartialEq, Eq)]
struct Profile {
    host: String,
    port: u16,
    username: String,
}
impl Profile {
    fn of(config: &ConnectionConfig) -> Self {
        Self {
            host: config.server.host.to_ascii_lowercase(),
            port: config.server.port,
            username: config.username.to_ascii_lowercase(),
        }
    }
}
struct Entry {
    token: u64,
    profile: Profile,
    client: Option<Client>,
}
struct State {
    closed: bool,
    next_token: u64,
    entries: BTreeMap<String, Entry>,
}
struct Inner {
    state: Mutex<State>,
    maximum_clients: usize,
    shutdown: watch::Sender<bool>,
    changed: Notify,
}
/// Named Client ownership, without choosing a Minecraft version globally.
/// Each connection receives its own config/cache/registry. Clones share this
/// manager. `shutdown` is terminal and explicitly closes retained Clients,
/// including externally held clones; dropping a manager alone is not shutdown.
/// Event aggregation and automatic reconnect are not supplied.
#[derive(Clone)]
pub struct ClientManager {
    inner: Arc<Inner>,
}
impl ClientManager {
    /// Bound active plus pending connections to 1..64, before any I/O.
    pub fn new(maximum_clients: usize) -> Result<Self> {
        if !(1..=64).contains(&maximum_clients) {
            return Err(super::recording::invalid("manager capacity must be 1..64"));
        }
        let (shutdown, _) = watch::channel(false);
        Ok(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    closed: false,
                    next_token: 0,
                    entries: BTreeMap::new(),
                }),
                maximum_clients,
                shutdown,
                changed: Notify::new(),
            }),
        })
    }
    /// Reserve a caller-chosen name before connecting. Duplicate names and
    /// duplicate case-insensitive profiles at the same exact host/port reject
    /// before I/O. Host aliases are not resolved into a global profile lock.
    /// Cancellation removes the reservation and drops the connecting transport.
    /// Return is authenticated connection creation, not playable readiness;
    /// call `Client::wait_until_ready` on the returned Client.
    pub async fn connect(
        &self,
        name: impl Into<String>,
        config: ConnectionConfig,
    ) -> Result<Client> {
        config.validate()?;
        let name = name.into();
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(super::recording::invalid(
                "manager name must be 1..128 bytes without controls",
            ));
        }
        let mut stopping = self.inner.shutdown.subscribe();
        let profile = Profile::of(&config);
        let token = {
            let mut state = self.inner.state.lock().expect("manager state poisoned");
            if state.closed {
                return Err(unavailable("manager is shut down"));
            }
            if state.entries.contains_key(&name)
                || state.entries.values().any(|e| e.profile == profile)
            {
                return Err(unavailable(
                    "name or endpoint/profile already managed or connecting",
                ));
            }
            if state.entries.len() >= self.inner.maximum_clients {
                return Err(unavailable("manager capacity exhausted"));
            }
            state.next_token = state
                .next_token
                .checked_add(1)
                .ok_or_else(|| unavailable("manager identity exhausted"))?;
            let token = state.next_token;
            state.entries.insert(
                name.clone(),
                Entry {
                    token,
                    profile,
                    client: None,
                },
            );
            token
        };
        let mut reservation = Reservation {
            inner: self.inner.clone(),
            name,
            token,
            armed: true,
        };
        let client = tokio::select! {
            biased;
            _=stopping.changed()=>return Err(unavailable("manager shut down during connect")),
            result=Client::connect(config)=>result?,
        };
        {
            let mut state = self.inner.state.lock().expect("manager state poisoned");
            if state.closed {
                return Err(unavailable("manager shut down before installation"));
            }
            let entry = state
                .entries
                .get_mut(&reservation.name)
                .filter(|e| e.token == token && e.client.is_none())
                .ok_or_else(|| unavailable("connection reservation retired"))?;
            entry.client = Some(client.clone());
            reservation.armed = false;
        }
        self.inner.changed.notify_waiters();
        Ok(client)
    }
    /// Clone an installed handle. Pending connections return None. A handle is
    /// not a readiness guarantee; external disconnects remain visible until removal.
    pub fn get(&self, name: &str) -> Option<Client> {
        self.inner
            .state
            .lock()
            .expect("manager state poisoned")
            .entries
            .get(name)
            .and_then(|e| e.client.clone())
    }
    /// Installed names in deterministic order, excluding pending reservations.
    pub fn names(&self) -> Vec<String> {
        self.inner
            .state
            .lock()
            .expect("manager state poisoned")
            .entries
            .iter()
            .filter(|(_, e)| e.client.is_some())
            .map(|(name, _)| name.clone())
            .collect()
    }
    /// Close one installed Client, preserving its entry on interrupted/failed
    /// closure so another explicit call can finish it. Pending connect is an
    /// error; cancel its caller or use shutdown to cancel all pending connects.
    pub async fn disconnect(&self, name: &str) -> Result<bool> {
        let entry = {
            let state = self.inner.state.lock().expect("manager state poisoned");
            match state.entries.get(name) {
                None => return Ok(false),
                Some(e) => match &e.client {
                    Some(c) => (e.token, c.clone()),
                    None => return Err(unavailable("client is still connecting")),
                },
            }
        };
        entry.1.disconnect().await?;
        let mut state = self.inner.state.lock().expect("manager state poisoned");
        if state.entries.get(name).is_some_and(|e| e.token == entry.0) {
            state.entries.remove(name);
        }
        Ok(true)
    }
    /// Stop admission, cancel pending connect futures and close installed
    /// Clients sequentially. Waits until pending reservations retire. A cancelled
    /// shutdown can be resumed explicitly; entries survive incomplete closes.
    pub async fn shutdown(&self) -> Result<()> {
        {
            let mut state = self.inner.state.lock().expect("manager state poisoned");
            state.closed = true;
            self.inner.shutdown.send_replace(true);
        }
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let pending = self
                .inner
                .state
                .lock()
                .expect("manager state poisoned")
                .entries
                .values()
                .any(|e| e.client.is_none());
            if !pending {
                break;
            }
            notified.await;
        }
        let names = self.names();
        let mut errors = vec![];
        for name in names {
            if let Err(e) = self.disconnect(&name).await {
                errors.push(format!("{name}: {e}"));
            }
        }
        if !errors.is_empty() {
            return Err(unavailable(errors.join("; ")));
        }
        Ok(())
    }
}
struct Reservation {
    inner: Arc<Inner>,
    name: String,
    token: u64,
    armed: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.armed {
            let mut state = self.inner.state.lock().expect("manager state poisoned");
            if state
                .entries
                .get(&self.name)
                .is_some_and(|e| e.token == self.token && e.client.is_none())
            {
                state.entries.remove(&self.name);
            }
            drop(state);
            self.inner.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MinecraftVersion, Server,
        protocol::{read_packet, write_packet},
    };
    use tokio::{
        io::AsyncReadExt,
        net::{TcpListener, TcpStream},
        time::{Duration, timeout},
    };
    fn config(port: u16, version: MinecraftVersion, name: &str) -> ConnectionConfig {
        ConnectionConfig::offline(Server::new("127.0.0.1", port), name, version)
    }
    async fn login(stream: &mut TcpStream) -> Vec<u8> {
        read_packet(stream, None).await.unwrap();
        read_packet(stream, None).await.unwrap().1
    }
    async fn eof(stream: &mut TcpStream) {
        timeout(Duration::from_secs(2), async {
            let mut bytes = [0; 1024];
            while stream.read(&mut bytes).await.unwrap() != 0 {}
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn cancelled_reservations_and_shutdown_close_pending_sockets_on_both_versions() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for shutdown in [false, true] {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let manager = ClientManager::new(2).unwrap();
                let cfg = config(port, version, "ManagedPending");
                let pending = tokio::spawn({
                    let m = manager.clone();
                    let c = cfg.clone();
                    async move { m.connect("one", c).await }
                });
                let (mut stream, _) = listener.accept().await.unwrap();
                login(&mut stream).await;
                assert!(manager.get("one").is_none());
                assert!(manager.names().is_empty());
                assert!(
                    manager
                        .connect("one", config(port, version, "Other"))
                        .await
                        .is_err()
                );
                assert!(manager.connect("other-key", cfg.clone()).await.is_err());
                assert!(
                    timeout(Duration::from_millis(30), listener.accept())
                        .await
                        .is_err(),
                    "duplicate admission performed I/O"
                );
                if shutdown {
                    timeout(Duration::from_secs(2), manager.shutdown())
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(pending.await.unwrap().is_err());
                    assert!(manager.connect("new", cfg).await.is_err());
                } else {
                    pending.abort();
                    assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
                    assert!(manager.inner.state.lock().unwrap().entries.is_empty());
                    manager.shutdown().await.unwrap();
                }
                eof(&mut stream).await;
                assert!(manager.inner.state.lock().unwrap().entries.is_empty());
                manager.shutdown().await.unwrap();
            }
        }
    }
    async fn authenticated(version: MinecraftVersion) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let original = login(&mut stream).await;
            let mut success = crate::client::login::test_legacy_success(&original);
            if version == MinecraftVersion::Java1_21_11 {
                success.push(0);
            }
            write_packet(&mut stream, None, 2, &success).await.unwrap();
            eof(&mut stream).await;
        });
        (port, task)
    }
    #[tokio::test]
    async fn mixed_version_handles_and_registry_ids_stay_isolated_and_shutdown_closes_clones() {
        let manager = ClientManager::new(2).unwrap();
        let (old_port, old_server) = authenticated(MinecraftVersion::Java1_16_1).await;
        let old = manager
            .connect(
                "legacy",
                config(old_port, MinecraftVersion::Java1_16_1, "ManagedOld"),
            )
            .await
            .unwrap();
        let (new_port, new_server) = authenticated(MinecraftVersion::Java1_21_11).await;
        let new = manager
            .connect(
                "modern",
                config(new_port, MinecraftVersion::Java1_21_11, "ManagedNew"),
            )
            .await
            .unwrap();
        assert_eq!(manager.names(), ["legacy", "modern"]);
        assert_eq!(
            manager.get("legacy").unwrap().version(),
            MinecraftVersion::Java1_16_1
        );
        assert_eq!(
            manager.get("modern").unwrap().version(),
            MinecraftVersion::Java1_21_11
        );
        let old_item = old.registry().item("minecraft:stone").unwrap();
        let new_item = new.registry().item("minecraft:stone").unwrap();
        assert!(new.registry().item_definition(old_item.id).is_err());
        assert!(old.registry().item_definition(new_item.id).is_err());
        assert!(
            manager
                .connect(
                    "third",
                    config(new_port, MinecraftVersion::Java1_21_11, "Other")
                )
                .await
                .is_err()
        );
        manager.shutdown().await.unwrap();
        assert!(manager.names().is_empty());
        assert!(manager.get("modern").is_none());
        assert!(old.wait_until_ready().await.is_err());
        assert!(new.wait_until_ready().await.is_err());
        old_server.await.unwrap();
        new_server.await.unwrap();
    }
}
