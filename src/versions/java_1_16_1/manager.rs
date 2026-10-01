//! Ownership and event aggregation for multiple Bots in one process.

use crate::versions::java_1_16_1::{
    Bot, ChunkStorageStats, ConnectionOptions, Event, PhysicsMetrics, Player, Result, Server,
    SharedChunkStorage,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{RwLock, broadcast};

macro_rules! bail {
    ($($argument:tt)*) => {
        return Err(crate::versions::java_1_16_1::Error::from(anyhow::anyhow!($($argument)*)).into())
    };
}

#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `BotEvent`.
pub struct BotEvent {
    /// The `username` value.
    pub username: String,
    /// The `event` value.
    pub event: Event,
}
struct Inner {
    server: Server,
    bots: RwLock<HashMap<String, Bot>>,
    connecting: tokio::sync::Mutex<HashSet<String>>,
    events: broadcast::Sender<BotEvent>,
    chunk_storage: Arc<SharedChunkStorage>,
    connection_options: ConnectionOptions,
}
#[derive(Clone)]
/// State and protocol data represented by `BotManager`.
pub struct BotManager {
    inner: Arc<Inner>,
}

impl BotManager {
    /// Performs the `new` operation.
    pub fn new(server: Server) -> Self {
        Self::with_options(server, ConnectionOptions::default())
    }

    /// Performs the `with_options` operation.
    pub fn with_options(server: Server, connection_options: ConnectionOptions) -> Self {
        Self::with_chunk_storage_and_options(
            server,
            Arc::new(SharedChunkStorage::default()),
            connection_options,
        )
    }

    /// Performs the `with_chunk_storage` operation.
    pub fn with_chunk_storage(server: Server, chunk_storage: Arc<SharedChunkStorage>) -> Self {
        Self::with_chunk_storage_and_options(server, chunk_storage, ConnectionOptions::default())
    }

    /// Performs the `with_chunk_storage_and_options` operation.
    pub fn with_chunk_storage_and_options(
        server: Server,
        chunk_storage: Arc<SharedChunkStorage>,
        connection_options: ConnectionOptions,
    ) -> Self {
        let (events, _) = broadcast::channel(512);
        Self {
            inner: Arc::new(Inner {
                server,
                bots: RwLock::new(HashMap::new()),
                connecting: tokio::sync::Mutex::new(HashSet::new()),
                events,
                chunk_storage,
                connection_options,
            }),
        }
    }
    /// Performs the `subscribe` operation.
    pub fn subscribe(&self) -> broadcast::Receiver<BotEvent> {
        self.inner.events.subscribe()
    }
    /// Performs the `connect` operation.
    pub async fn connect(&self, player: Player) -> Result<Bot> {
        player.validate()?;
        let username = player.username.clone();
        {
            let mut connecting = self.inner.connecting.lock().await;
            if self.inner.bots.read().await.contains_key(&username)
                || !connecting.insert(username.clone())
            {
                bail!("bot {username} is already managed or connecting");
            }
        }
        let result = Bot::connect(
            self.inner.server.clone(),
            player,
            self.inner.chunk_storage.clone(),
            self.inner.connection_options,
        )
        .await;
        self.inner.connecting.lock().await.remove(&username);
        let bot = result?;
        let mut receiver = bot.subscribe();
        let connection_id = bot.connection_id();
        self.inner
            .bots
            .write()
            .await
            .insert(username.clone(), bot.clone());
        let events = self.inner.events.clone();
        let event_username = username.clone();
        let manager = Arc::downgrade(&self.inner);
        let relay = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let terminal = matches!(
                            event,
                            Event::Disconnected { .. }
                                | Event::Error {
                                    kind: "connection",
                                    ..
                                }
                        );
                        let _ = events.send(BotEvent {
                            username: event_username.clone(),
                            event,
                        });
                        if terminal {
                            if let Some(manager) = manager.upgrade() {
                                let mut bots = manager.bots.write().await;
                                if bots
                                    .get(&event_username)
                                    .is_some_and(|bot| bot.connection_id() == connection_id)
                                {
                                    bots.remove(&event_username);
                                }
                            }
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        if bot.is_stopped() {
            relay.abort();
            let mut bots = self.inner.bots.write().await;
            if bots
                .get(&username)
                .is_some_and(|managed| managed.connection_id() == connection_id)
            {
                bots.remove(&username);
            }
        }
        Ok(bot)
    }
    /// Performs the `get` operation.
    pub async fn get(&self, username: &str) -> Option<Bot> {
        self.inner.bots.read().await.get(username).cloned()
    }
    /// Performs the `usernames` operation.
    pub async fn usernames(&self) -> Vec<String> {
        let mut names: Vec<_> = self.inner.bots.read().await.keys().cloned().collect();
        names.sort();
        names
    }
    /// Performs the `physics_metrics` operation.
    pub async fn physics_metrics(&self) -> HashMap<String, PhysicsMetrics> {
        let bots: Vec<_> = self
            .inner
            .bots
            .read()
            .await
            .iter()
            .map(|(name, bot)| (name.clone(), bot.clone()))
            .collect();
        let mut metrics = HashMap::with_capacity(bots.len());
        for (name, bot) in bots {
            metrics.insert(name, bot.physics_metrics().await);
        }
        metrics
    }
    /// Performs the `chunk_storage_stats` operation.
    pub fn chunk_storage_stats(&self) -> ChunkStorageStats {
        self.inner.chunk_storage.stats()
    }
    /// Performs the `disconnect` operation.
    pub async fn disconnect(&self, username: &str) -> Result<bool> {
        let bot = self.inner.bots.read().await.get(username).cloned();
        if let Some(bot) = bot {
            let connection_id = bot.connection_id();
            bot.disconnect().await?;
            let mut bots = self.inner.bots.write().await;
            if bots
                .get(username)
                .is_some_and(|managed| managed.connection_id() == connection_id)
            {
                bots.remove(username);
            }
            return Ok(true);
        }
        Ok(false)
    }
    /// Performs the `disconnect_all` operation.
    pub async fn disconnect_all(&self) -> Result<()> {
        let bots: Vec<_> = self
            .inner
            .bots
            .read()
            .await
            .iter()
            .map(|(username, bot)| (username.clone(), bot.clone()))
            .collect();
        let mut errors = Vec::new();
        for (username, bot) in bots {
            let connection_id = bot.connection_id();
            if let Err(error) = bot.disconnect().await {
                errors.push(error.to_string());
            }
            let mut managed = self.inner.bots.write().await;
            if managed
                .get(&username)
                .is_some_and(|current| current.connection_id() == connection_id)
            {
                managed.remove(&username);
            }
        }
        if !errors.is_empty() {
            bail!(
                "failed to disconnect one or more bots: {}",
                errors.join("; ")
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{read_packet, write_packet};
    use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot, time::Duration};

    async fn mock_login_server() -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            read_packet(&mut reader, None).await.unwrap();
            write_packet(&mut writer, None, 0x02, &[]).await.unwrap();
            let mut byte = [0_u8; 1];
            let _ = reader.read_exact(&mut byte).await;
            write_packet(&mut writer, None, 0x1a, &[0]).await.unwrap();
        });
        (port, task)
    }

    #[tokio::test]
    async fn empty_manager_has_no_bots() {
        let manager = BotManager::new(Server::default());
        assert!(manager.usernames().await.is_empty());
        assert!(manager.get("Bot01").await.is_none());
        assert!(!manager.disconnect("Bot01").await.unwrap());
    }

    #[tokio::test]
    async fn direct_bot_disconnect_removes_managed_connection() {
        let (port, server) = mock_login_server().await;
        let manager = BotManager::new(Server::new("127.0.0.1", port));
        let bot = manager
            .connect(Player::offline("Reconnectable"))
            .await
            .unwrap();
        assert!(manager.get("Reconnectable").await.is_some());
        bot.disconnect().await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.get("Reconnectable").await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("direct disconnect left a stale managed bot");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pending_disconnect_blocks_same_username_until_terminal() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown_seen_tx, shutdown_seen_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (second_release_tx, second_release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            read_packet(&mut reader, None).await.unwrap();
            write_packet(&mut writer, None, 0x02, &[]).await.unwrap();
            let mut byte = [0_u8; 1];
            let _ = reader.read(&mut byte).await;
            let _ = shutdown_seen_tx.send(());
            release_rx.await.unwrap();
            drop(reader);
            drop(writer);

            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            read_packet(&mut reader, None).await.unwrap();
            write_packet(&mut writer, None, 0x02, &[]).await.unwrap();
            second_release_rx.await.unwrap();
            write_packet(&mut writer, None, 0x1a, &[0]).await.unwrap();
        });
        let manager = BotManager::new(Server::new("127.0.0.1", port));
        let bot = manager
            .connect(Player::offline("SameUsername"))
            .await
            .unwrap();
        let disconnecting = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.disconnect("SameUsername").await })
        };
        shutdown_seen_rx.await.unwrap();
        assert!(
            manager
                .connect(Player::offline("SameUsername"))
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        assert!(disconnecting.await.unwrap().unwrap());
        assert!(manager.get("SameUsername").await.is_none());
        drop(bot);
        let reconnected = manager
            .connect(Player::offline("SameUsername"))
            .await
            .unwrap();
        assert!(manager.get("SameUsername").await.is_some());
        second_release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.get("SameUsername").await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("completed reconnect was not removed after terminal event");
        drop(reconnected);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_all_keeps_registry_until_terminal() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown_seen_tx, shutdown_seen_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();
            read_packet(&mut reader, None).await.unwrap();
            read_packet(&mut reader, None).await.unwrap();
            write_packet(&mut writer, None, 0x02, &[]).await.unwrap();
            let mut byte = [0_u8; 1];
            let _ = reader.read(&mut byte).await;
            let _ = shutdown_seen_tx.send(());
            release_rx.await.unwrap();
        });
        let manager = BotManager::new(Server::new("127.0.0.1", port));
        let bot = manager
            .connect(Player::offline("AllDisconnect"))
            .await
            .unwrap();
        let disconnecting = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.disconnect_all().await })
        };
        shutdown_seen_rx.await.unwrap();
        assert_eq!(manager.usernames().await, vec!["AllDisconnect".to_owned()]);
        release_tx.send(()).unwrap();
        disconnecting.await.unwrap().unwrap();
        assert!(manager.usernames().await.is_empty());
        drop(bot);
        server.await.unwrap();
    }
}
