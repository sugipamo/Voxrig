//! Ordered 1.21.11 receive loop. Local state is committed under one session lock.
mod correction;
#[cfg(test)]
mod edge_native_trials;
mod entity;
mod loading;
mod motion;
mod observations;
/// Explicit Java 1.21.11 operation API and received player state.
pub mod operations;
mod outbound;
mod packet_replay;
pub mod players;
pub mod raycast;
mod recipes;
pub mod recording;
use super::{
    ids,
    reconstruction::{Action, ClientObservation, Direction, Reconstruction},
    wire::Reader,
    world::{Dimension, World},
};
use crate::protocol::{put_string, put_varint, read_packet, write_packet};
use crate::{
    ConnectionConfig, Error, ErrorKind, MinecraftVersion, Observation, ObservedBlock, Region,
    Result,
};
use anyhow::{Context, bail};
pub(crate) use packet_replay::replay_packets;
use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::Instant,
};
use tokio::{
    io::AsyncWriteExt,
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::{Mutex, Notify},
    time::timeout,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Configuration,
    Play,
}

use crate::client::recording::{LocalPlayerBasis, PacketPhase, TraceCapture};
pub use crate::client::recording::{PacketRecord, PacketTrace};

struct State {
    loading: loading::InteractionLoading,
    motion: motion::OwnMotion,
    identity: Option<LoginIdentity>,
    retirement: Option<operations::MiningRetirementRecord>,
    mining: Option<operations::MiningRecord>,
    common_mining: Option<operations::mining::CommonMiningCapture>,
    placement: Option<operations::PlacementRecord>,
    common_placement: Option<operations::placement::CommonPlacementCapture>,
    retired_common_placement: Option<crate::client::survival::PlacementRecord>,
    common_inventory_swap: Option<operations::inventory::common::CommonSwap>,
    common_inventory_click: Option<crate::client::inventory::InventoryClickRecord>,
    common_crafting_take: Option<crate::client::crafting::CraftingTakeRecord>,
    common_recipe_placement: Option<crate::client::crafting::RecipePlacementRecord>,
    common_inventory_transfer: Option<crate::client::inventory::InventoryTransferRecord>,
    common_container_close: Option<crate::client::container::ContainerCloseRecord>,
    pub(crate) flight_history: crate::client::flight::History,
    dismount_history: crate::client::vehicle::dismount::History,
    vehicle_control_history: crate::client::vehicle::control::History,
    close_history: Arc<std::sync::Mutex<Option<crate::client::container::ContainerCloseRecord>>>,
    recipe_placement_history:
        Arc<std::sync::Mutex<Option<crate::client::crafting::RecipePlacementRecord>>>,
    crafting_take_history:
        Arc<std::sync::Mutex<Option<crate::client::crafting::CraftingTakeRecord>>>,
    common_container_open: Option<crate::client::container::ContainerOpenRecord>,
    survival_motion: Option<operations::SurvivalMotionRecord>,
    retired_common_motion: Option<crate::client::survival::MotionRecord>,
    recording: Option<recording::Capture>,
    recording_ordinal: u64,
    operations: operations::OperationState,
    players: players::PlayerTracker,
    entities: crate::client::entity::SpawnLedger,
    vehicles: crate::client::vehicle::PassengerLedger,
    scoreboard: crate::client::ui::ScoreboardLedger,
    boss_bars: crate::client::ui::boss_bar::BossBarLedger,
    phase: Phase,
    world: World,
    reconstruction: Reconstruction,
    observations: observations::RegionCache,
    dimensions: Vec<Dimension>,
    registries: crate::client::registry::received::ReceivedRegistries,
    recipes: crate::client::crafting::recipes::RecipeReceipts,
    recipe_ghost: Option<crate::client::crafting::ghost::GhostReceipts>,
    position: Option<[f64; 3]>,
    rotation: [f32; 2],
    ready: bool,
    sequence: u64,
    failure: Option<Error>,
    trace: Option<TraceCapture>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            loading: loading::InteractionLoading::default(),
            motion: motion::OwnMotion::default(),
            identity: None,
            retirement: None,
            mining: None,
            common_mining: None,
            placement: None,
            common_placement: None,
            retired_common_placement: None,
            common_inventory_swap: None,
            common_inventory_click: None,
            common_crafting_take: None,
            common_recipe_placement: None,
            common_inventory_transfer: None,
            common_container_close: None,
            flight_history: Arc::default(),
            dismount_history: Arc::default(),
            vehicle_control_history: Arc::default(),
            close_history: Arc::default(),
            crafting_take_history: Arc::default(),
            recipe_placement_history: Arc::default(),
            common_container_open: None,
            survival_motion: None,
            retired_common_motion: None,
            recording: None,
            recording_ordinal: 0,
            operations: operations::OperationState::default(),
            players: players::PlayerTracker::default(),
            entities: Default::default(),
            vehicles: Default::default(),
            scoreboard: Default::default(),
            boss_bars: Default::default(),
            phase: Phase::Configuration,
            world: World::default(),
            reconstruction: Reconstruction::default(),
            observations: observations::RegionCache::default(),
            dimensions: Vec::new(),
            registries: Default::default(),
            recipes: Default::default(),
            recipe_ghost: None,
            position: None,
            rotation: [0.0; 2],
            ready: false,
            sequence: 0,
            failure: None,
            trace: None,
        }
    }
}
impl State {
    // Receive, trace and apply share a single boundary for live operation and replay.
    fn receive(&mut self, id: i32, payload: &[u8], max_chunks: usize) -> anyhow::Result<Responses> {
        if self.failure.is_some() {
            bail!("cannot apply packets after a failed receive");
        }
        self.sequence += 1;
        if let Some(trace) = &mut self.trace {
            trace.record(
                self.sequence,
                self.reconstruction.tick,
                match self.phase {
                    Phase::Configuration => PacketPhase::Configuration,
                    Phase::Play => PacketPhase::Play,
                },
                id,
                payload,
                (matches!(self.phase, Phase::Play) && id == ids::play_clientbound::POSITION)
                    .then_some(LocalPlayerBasis {
                        position: self.position,
                        rotation: self.rotation,
                        velocity: if self.motion.position_basis == motion::PositionBasis::Received {
                            self.operations.local_player.velocity.map(|v| v.value)
                        } else {
                            None
                        },
                    }),
            );
        }
        let result = match self.phase {
            Phase::Configuration => apply_configuration(self, id, payload),
            Phase::Play => apply_play(self, id, payload, max_chunks),
        };
        if result.is_ok() {
            operations::vehicle::context_received(self);
            operations::placement_context_received(self);
            operations::placement::common_placement_context_received(self);
            operations::inventory::common::context_received(self);
            operations::inventory::click::context_received(self);
            operations::inventory::crafting::context_received(self);
            operations::inventory::recipe_placement::context_received(self);
            operations::inventory::transfer::context_received(self);
            operations::container::context_received(self);
            operations::mining::common_mining_context_received(self);
        }
        if let Err(error) = &result {
            self.failure = Some(Error::new(
                ErrorKind::Protocol,
                anyhow::anyhow!("{error:#}"),
            ));
            self.ready = false;
        }
        result
    }
}
struct Writer {
    stream: OwnedWriteHalf,
    compression: Option<i32>,
}
#[derive(Clone)]
struct LoginIdentity {
    uuid: [u8; 16],
    name: String,
    server: crate::Server,
}
struct Session {
    id: u64,
    started: Instant,
    writer: Mutex<Writer>,
    state: Mutex<State>,
    changed: Notify,
    cancel: Notify,
    stopped: AtomicBool,
    revoked: AtomicBool,
    receiver_abort: std::sync::OnceLock<tokio::task::AbortHandle>,
    runtime: tokio::runtime::Handle,
    interrupted_packet: AtomicI32,
    limits: crate::client::ClientLimits,
    interaction_sequence: AtomicI32,
}
struct Lease(Weak<Session>);
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(session) = self.0.upgrade() {
            session.stop();
        }
    }
}

#[derive(Clone)]
pub(crate) struct Bot {
    pub(crate) flight_history: crate::client::flight::History,
    dismount_history: crate::client::vehicle::dismount::History,
    vehicle_control_history: crate::client::vehicle::control::History,
    close_history: Arc<std::sync::Mutex<Option<crate::client::container::ContainerCloseRecord>>>,
    recipe_placement_history:
        Arc<std::sync::Mutex<Option<crate::client::crafting::RecipePlacementRecord>>>,
    crafting_take_history:
        Arc<std::sync::Mutex<Option<crate::client::crafting::CraftingTakeRecord>>>,
    session: Arc<Session>,
    _lease: Arc<Lease>,
}

impl Bot {
    pub(crate) async fn common_server_registry_state(
        &self,
    ) -> Result<crate::client::registry::ServerRegistryObservation> {
        let state = self.session.state.lock().await;
        self.session.check(&state)?;
        Ok(state.registries.capture(
            crate::client::SessionStamp {
                version: MinecraftVersion::Java1_21_11,
                connection_id: self.session.id,
                world_generation: state.loading.generation,
            },
            state.sequence,
        ))
    }
    pub(crate) fn connection_id(&self) -> u64 {
        self.session.id
    }
    pub fn operations(&self) -> operations::Operations {
        operations::Operations { bot: self.clone() }
    }
    pub async fn start_packet_trace(&self, maximum_bytes: usize) -> Result<()> {
        crate::client::recording::validate_limit(maximum_bytes)?;
        let mut state = self.session.state.lock().await;
        self.session.check(&state)?;
        if state.trace.is_some() {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("trace already active"),
            ));
        }
        state.trace = Some(TraceCapture::new(state.sequence, maximum_bytes)?);
        Ok(())
    }

    pub async fn stop_packet_trace(&self) -> Result<PacketTrace> {
        let mut state = self.session.state.lock().await;
        let trace = state.trace.take().context("no active packet trace")?;
        Ok(trace.finish(
            crate::MinecraftVersion::Java1_21_11,
            self.session.id,
            state.sequence,
            state.reconstruction.tick,
        ))
    }

    pub async fn interact_block(&self, position: [i32; 3], face: crate::BlockFace) -> Result<()> {
        self.operations()
            .use_on_block(position, face, [0.5; 3])
            .await?;
        Ok(())
    }

    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        Self::connect_with_packet_trace(config, None).await
    }
    pub(crate) async fn connect_with_packet_trace(
        config: ConnectionConfig,
        trace_limit: Option<usize>,
    ) -> Result<Self> {
        let trace = trace_limit
            .map(|limit| TraceCapture::new(0, limit))
            .transpose()?;
        config.validate()?;
        if config.version != crate::MinecraftVersion::Java1_21_11 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("configuration belongs to another adapter"),
            ));
        }
        let stream = timeout(
            config.limits.connect_timeout,
            TcpStream::connect((config.server.host.as_str(), config.server.port)),
        )
        .await
        .context("connect timed out")??;
        let (mut reader, mut writer) = stream.into_split();
        let mut handshake = Vec::new();
        put_varint(&mut handshake, 774);
        put_string(&mut handshake, &config.server.host);
        handshake.extend(config.server.port.to_be_bytes());
        put_varint(&mut handshake, 2);
        write_packet(&mut writer, None, 0, &handshake).await?;
        let mut login = Vec::new();
        put_string(&mut login, &config.username);
        // Offline servers derive the authenticated profile UUID from the name.
        login.extend([0u8; 16]);
        write_packet(
            &mut writer,
            None,
            ids::login_serverbound::LOGIN_START,
            &login,
        )
        .await?;
        let mut compression = None;
        let identity = loop {
            let (id, payload) = timeout(
                config.limits.login_packet_timeout,
                read_packet(&mut reader, compression),
            )
            .await
            .context("login timed out")??;
            let mut r = Reader::new(&payload);
            match id {
                ids::login_clientbound::SUCCESS => {
                    let uuid = r.take(16)?.try_into().context("login UUID length")?;
                    let received = r.string()?;
                    if received != config.username {
                        return Err(Error::new(
                            ErrorKind::Protocol,
                            anyhow::anyhow!("login profile name mismatch"),
                        ));
                    }
                    for _ in 0..r.count(1024)? {
                        r.string()?;
                        r.string()?;
                        if r.bool()? {
                            r.string()?;
                        }
                    }
                    r.end()?;
                    write_packet(
                        &mut writer,
                        compression,
                        ids::login_serverbound::LOGIN_ACKNOWLEDGED,
                        &[],
                    )
                    .await?;
                    break LoginIdentity {
                        uuid,
                        name: received,
                        server: config.server.clone(),
                    };
                }
                ids::login_clientbound::COMPRESS => {
                    let threshold = r.varint()?;
                    r.end()?;
                    if threshold < 0 {
                        return Err(Error::new(
                            ErrorKind::Protocol,
                            anyhow::anyhow!("negative compression threshold"),
                        ));
                    }
                    compression = Some(threshold);
                }
                ids::login_clientbound::ENCRYPTION_BEGIN => {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        anyhow::anyhow!("online-mode authentication is unsupported"),
                    ));
                }
                ids::login_clientbound::DISCONNECT => {
                    return Err(Error::new(
                        ErrorKind::Disconnected,
                        anyhow::anyhow!("server rejected login"),
                    ));
                }
                ids::login_clientbound::COOKIE_REQUEST => {
                    let key = r.string()?;
                    r.end()?;
                    let mut response = Vec::new();
                    put_string(&mut response, &key);
                    response.push(0);
                    write_packet(
                        &mut writer,
                        compression,
                        ids::login_serverbound::COOKIE_RESPONSE,
                        &response,
                    )
                    .await?;
                }
                ids::login_clientbound::LOGIN_PLUGIN_REQUEST => {
                    let request = r.varint()?;
                    let mut response = Vec::new();
                    put_varint(&mut response, request);
                    response.push(0);
                    write_packet(
                        &mut writer,
                        compression,
                        ids::login_serverbound::LOGIN_PLUGIN_RESPONSE,
                        &response,
                    )
                    .await?;
                }
                _ => {
                    return Err(Error::new(
                        ErrorKind::Protocol,
                        anyhow::anyhow!("unexpected login packet {id}"),
                    ));
                }
            }
        };
        let session = Arc::new(Session {
            id: crate::connection::next_connection_id(),
            started: Instant::now(),
            writer: Mutex::new(Writer {
                stream: writer,
                compression,
            }),
            state: Mutex::new(State {
                identity: Some(identity),
                trace,
                ..State::default()
            }),
            changed: Notify::new(),
            cancel: Notify::new(),
            stopped: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
            receiver_abort: std::sync::OnceLock::new(),
            runtime: tokio::runtime::Handle::current(),
            interrupted_packet: AtomicI32::new(-1),
            limits: config.limits,
            interaction_sequence: AtomicI32::new(0),
        });
        session
            .send(ids::configuration_serverbound::SETTINGS, &settings())
            .await?;
        let bot = Self {
            recipe_placement_history: {
                let state = session.state.try_lock().expect("new session");
                state.recipe_placement_history.clone()
            },
            crafting_take_history: {
                let state = session.state.try_lock().expect("new session");
                state.crafting_take_history.clone()
            },
            flight_history: {
                let state = session.state.lock().await;
                state.flight_history.clone()
            },
            vehicle_control_history: {
                let state = session.state.try_lock().expect("new session");
                state.vehicle_control_history.clone()
            },
            dismount_history: {
                let state = session.state.try_lock().expect("new session");
                state.dismount_history.clone()
            },
            close_history: session
                .state
                .try_lock()
                .expect("new session")
                .close_history
                .clone(),
            _lease: Arc::new(Lease(Arc::downgrade(&session))),
            session: session.clone(),
        };
        let receiving = session.clone();
        let receiver = session.runtime.spawn(async move {
            receiving.run_receiver(reader).await;
        });
        let _ = session.receiver_abort.set(receiver.abort_handle());
        Ok(bot)
    }

    pub async fn wait_until_ready(&self) -> Result<()> {
        timeout(self.session.limits.ready_timeout, async {
            loop {
                let notified = self.session.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let state = self.session.state.lock().await;
                    self.session.check(&state)?;
                    if state.ready && state.loading.notification_dispatched() {
                        return Ok(());
                    }
                }
                notified.await;
            }
        })
        .await
        .context("ready timed out")?
    }

    pub async fn observe_region(&self, region: Region) -> Result<Observation> {
        let state = self.session.state.lock().await;
        self.observation(&state, region)
    }

    pub async fn observe_client_region(&self, region: Region) -> Result<ClientObservation> {
        Ok(self
            .observe_shared_client_region(region)
            .await?
            .observation
            .into_owned())
    }

    /// Same-generation acquisitions share cell arrays under the session lock.
    /// The lock also orders packet application and local moving-block advancement.
    pub async fn observe_shared_client_region(
        &self,
        region: Region,
    ) -> Result<super::reconstruction::SharedClientRegion> {
        let mut state = self.session.state.lock().await;
        self.session.check(&state)?;
        state.shared_observation(region, self.session.id, self.session.started.elapsed())
    }

    fn observation(&self, state: &State, region: Region) -> Result<Observation> {
        let volume = region.volume()?;
        self.session.check(state)?;
        let dimension = state
            .world
            .dimension
            .as_ref()
            .context("dimension is not available")?
            .1;
        if region.min[1] < dimension.min_y || region.max[1] >= dimension.min_y + dimension.height {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                anyhow::anyhow!("observation outside dimension"),
            ));
        }
        let mut blocks = Vec::with_capacity(volume);
        for x in region.min[0]..=region.max[0] {
            for y in region.min[1]..=region.max[1] {
                for z in region.min[2]..=region.max[2] {
                    let position = [x, y, z];
                    let native = state
                        .world
                        .block(position)
                        .map(super::native_state)
                        .transpose()?;
                    blocks.push(ObservedBlock {
                        position,
                        state: native,
                    });
                }
            }
        }
        Ok(Observation {
            version: MinecraftVersion::Java1_21_11,
            connection_id: self.session.id,
            revision: state.world.revision,
            receive_sequence: Some(state.sequence),
            captured_at: self.session.started.elapsed(),
            region,
            blocks,
        })
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.session.stop();
        self.session.writer.lock().await.stream.shutdown().await?;
        Ok(())
    }
    pub(crate) fn revoke_connection(&self) {
        self.session.revoke();
    }
}

impl Session {
    fn check(&self, state: &State) -> Result<()> {
        let outbound = self.check_outbound();
        if outbound
            .as_ref()
            .is_err_and(|e| e.kind() == ErrorKind::UncertainDispatch)
        {
            return outbound;
        }
        if let Some(error) = &state.failure {
            return Err(Error::new(error.kind(), anyhow::anyhow!("{error}")));
        }
        outbound
    }
    async fn run_receiver(&self, reader: OwnedReadHalf) {
        let result = self.receive_loop(reader).await;
        self.stop();
        if let Err(error) = result {
            let mut state = self.state.lock().await;
            if state.failure.is_none() {
                state.failure = Some(Error::from(error));
            }
        }
        let _ = self.writer.lock().await.stream.shutdown().await;
    }
    async fn receive_loop(&self, mut reader: OwnedReadHalf) -> anyhow::Result<()> {
        let compression = self.writer.lock().await.compression;
        while !self.stopped.load(Ordering::Acquire) {
            let packet = tokio::select! {
                biased;
                _=self.cancel.notified()=>return Ok(()),
                packet=timeout(self.limits.play_packet_timeout,read_packet(&mut reader,compression))=>packet.context("play packet timed out")??,
            };
            let responses = {
                let mut state = self.state.lock().await;
                if self.stopped.load(Ordering::Acquire) {
                    return Ok(());
                }
                let target = self.started.elapsed().as_millis() as u64 / 50;
                let State {
                    world,
                    reconstruction,
                    ..
                } = &mut *state;
                reconstruction.advance(world, target);
                let mut responses = state.receive(packet.0, &packet.1, self.limits.max_chunks)?;
                if packet.0 == ids::play_clientbound::POSITION && matches!(state.phase, Phase::Play)
                {
                    // Publish a received correction only after its original
                    // teleport confirmation/position response frames complete.
                    // Normal operations share this state lock and cannot
                    // overtake the server's outstanding teleport boundary.
                    for (id, payload) in responses.drain(..) {
                        self.send(id, &payload).await?;
                    }
                }
                responses
            };
            for (id, payload) in responses {
                self.send(id, &payload).await?;
            }
            {
                let mut state = self.state.lock().await;
                self.complete_loading(&mut state).await?;
            }
            self.changed.notify_waiters();
        }
        Ok(())
    }
}

fn settings() -> Vec<u8> {
    let mut payload = Vec::new();
    put_string(&mut payload, "en_us");
    payload.push(4);
    put_varint(&mut payload, 0);
    payload.extend([1, 127]);
    put_varint(&mut payload, 1);
    payload.extend([0, 0]);
    put_varint(&mut payload, 2);
    payload
}

type Responses = Vec<(i32, Vec<u8>)>;

fn apply_configuration(state: &mut State, id: i32, payload: &[u8]) -> anyhow::Result<Responses> {
    use ids::{configuration_clientbound as input, configuration_serverbound as output};
    let mut r = Reader::new(payload);
    let mut responses = Vec::new();
    match id {
        input::FINISH_CONFIGURATION => {
            r.end()?;
            if state.dimensions.is_empty() {
                bail!("configuration supplied no dimension registry");
            }
            state.phase = Phase::Play;
            state.registries.finish();
            responses.push((output::FINISH_CONFIGURATION, Vec::new()));
        }
        input::KEEP_ALIVE => {
            r.take(8)?;
            r.end()?;
            responses.push((output::KEEP_ALIVE, payload.to_vec()));
        }
        input::PING => {
            r.take(4)?;
            r.end()?;
            responses.push((output::PONG, payload.to_vec()));
        }
        input::SELECT_KNOWN_PACKS => {
            for _ in 0..r.count(1024)? {
                r.string()?;
                r.string()?;
                r.string()?;
            }
            r.end()?;
            // Ask for complete registry entries rather than assuming a local known pack.
            responses.push((output::SELECT_KNOWN_PACKS, vec![0]));
        }
        input::REGISTRY_DATA => {
            let name = r.string()?;
            let count = r.count(65_536)?;
            let mut dimensions = Vec::new();
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                let entry_name = r.string()?;
                let present = r.bool()?;
                if !present {
                    bail!("registry data omitted despite empty known-packs response");
                }
                if r.remaining().first() != Some(&10) {
                    bail!("registry entry data must be a compound");
                }
                let data = r.encoded_nbt()?;
                if name == "minecraft:dimension_type" {
                    let (min_y, height) = Reader::new(&data).dimension_nbt()?;
                    dimensions.push(Dimension::new(
                        min_y.context("missing min_y")?,
                        height.context("missing height")?,
                    )?);
                }
                entries.push(crate::client::registry::ServerRegistryEntry {
                    name: entry_name,
                    data,
                });
            }
            r.end()?;
            state.registries.modern_registry(
                name.clone(),
                entries,
                state.sequence,
                payload.len(),
            )?;
            if name == "minecraft:dimension_type" {
                state.dimensions = dimensions;
            }
        }
        input::COOKIE_REQUEST => {
            let name = r.string()?;
            r.end()?;
            let mut response = Vec::new();
            put_string(&mut response, &name);
            response.push(0);
            responses.push((output::COOKIE_RESPONSE, response));
        }
        input::DISCONNECT => bail!("server disconnected during configuration"),
        input::TRANSFER => bail!("server transfer requires a new explicitly configured connection"),
        input::CODE_OF_CONDUCT => bail!("server requires code-of-conduct acceptance"),
        input::ADD_RESOURCE_PACK => bail!("resource-pack negotiation is unsupported"),
        input::FEATURE_FLAGS => {
            let mut features = Vec::new();
            for _ in 0..r.count(1024)? {
                let name = r.string()?;
                if name != "minecraft:vanilla" {
                    bail!("unsupported experimental feature set");
                }
                features.push(name);
            }
            r.end()?;
            state.operations.features = Some(features);
        }
        input::TAGS => {
            state
                .registries
                .receive_tags(payload, state.sequence, MinecraftVersion::Java1_21_11)?
        }
        // Presentation and cookie storage do not change block-state IDs.
        input::CUSTOM_PAYLOAD
        | input::RESET_CHAT
        | input::REMOVE_RESOURCE_PACK
        | input::STORE_COOKIE
        | input::CUSTOM_REPORT_DETAILS
        | input::SERVER_LINKS
        | input::CLEAR_DIALOG
        | input::SHOW_DIALOG => {}
        _ => bail!("unknown configuration packet {id}"),
    }
    Ok(responses)
}

fn spawn_info(state: &mut State, r: &mut Reader<'_>) -> anyhow::Result<()> {
    let id = r.count(1024)?;
    let name = r.string()?;
    r.take(8)?;
    let game_mode = r.u8()?;
    r.u8()?;
    r.bool()?;
    r.bool()?;
    if r.bool()? {
        r.string()?;
        r.take(8)?;
    }
    r.varint()?;
    r.varint()?;
    let dimension = *state
        .dimensions
        .get(id)
        .context("unknown dimension registry ID")?;
    state.world.select_dimension(name, dimension);
    state.loading.reset(state.sequence);
    state
        .motion
        .invalidate(state.sequence, "world generation changed");
    operations::mining_world_changed(state, "world login/respawn changed mining context");
    if let Some(capture) = &mut state.recording {
        capture.invalidate(recording::RecordingIssue::WorldChanged);
    }
    state.reconstruction = Reconstruction::default();
    state.operations.reset_world(game_mode)?;
    state.players.reset_world();
    state.entities.clear();
    state.vehicles.clear();
    state.ready = false;
    state.position = None;
    Ok(())
}

fn apply_play(
    state: &mut State,
    id: i32,
    payload: &[u8],
    max_chunks: usize,
) -> anyhow::Result<Responses> {
    use ids::{play_clientbound as input, play_serverbound as output};
    let mut r = Reader::new(payload);
    let mut responses = Vec::new();
    entity::receive(state, id, payload)?;
    if operations::receive(state, id, payload)? {
        return Ok(responses);
    }
    if recipes::receive(state, id, payload)? {
        return Ok(responses);
    }
    if state.players.receive(id, payload, state.sequence)? {
        operations::retirement_received(state, id, payload)?;
        return Ok(responses);
    }
    match id {
        input::LOGIN => {
            let entity_id = r.i32()?;
            if entity_id < 0 {
                bail!("invalid own entity identifier");
            }
            r.bool()?;
            for _ in 0..r.count(1024)? {
                r.string()?;
            }
            r.varint()?;
            r.varint()?;
            r.varint()?;
            r.bool()?;
            r.bool()?;
            r.bool()?;
            spawn_info(state, &mut r)?;
            r.bool()?;
            r.end()?;
            state.operations.local_player = operations::LocalPlayerState::spawned(entity_id);
        }
        input::RESPAWN => {
            spawn_info(state, &mut r)?;
            r.u8()?;
            r.end()?;
        }
        input::KEEP_ALIVE => {
            r.take(8)?;
            r.end()?;
            responses.push((output::KEEP_ALIVE, payload.to_vec()));
        }
        input::PING => {
            r.take(4)?;
            r.end()?;
            responses.push((output::PONG, payload.to_vec()));
        }
        input::BOSS_BAR => {
            state
                .boss_bars
                .receive(MinecraftVersion::Java1_21_11, payload, state.sequence)?;
        }
        input::SCOREBOARD_OBJECTIVE
        | input::SCOREBOARD_DISPLAY_OBJECTIVE
        | input::SCOREBOARD_SCORE
        | input::RESET_SCORE => {
            state
                .scoreboard
                .receive(MinecraftVersion::Java1_21_11, id, payload, state.sequence)?;
        }
        input::POSITION => {
            let teleport = r.varint()?;
            let correction = correction::Correction::read(&mut r)?;
            r.end()?;
            let resolved = correction.resolve(
                state.position,
                state.rotation,
                if state.motion.position_basis == motion::PositionBasis::Received {
                    state.operations.local_player.velocity.map(|v| v.value)
                } else {
                    None
                },
            )?;
            let position = resolved.position;
            let rotation = resolved.rotation;
            if position.iter().any(|v| v.abs() > 30_000_000.0) {
                bail!("position outside world bounds");
            }
            state.operations.local_player.velocity =
                resolved.velocity.map(|value| operations::VelocitySample {
                    value,
                    receive_sequence: state.sequence,
                });
            state.position = Some(position);
            state.motion.receive(motion::ReceivedPose {
                generation: state.loading.generation,
                receive_sequence: state.sequence,
                position,
                rotation,
                velocity: state.operations.local_player.velocity.map(|v| v.value),
            });
            state.rotation = rotation;
            state.ready = true;
            let mut confirm = Vec::new();
            put_varint(&mut confirm, teleport);
            responses.push((output::TELEPORT_CONFIRM, confirm));
            let mut moved = Vec::new();
            for value in position {
                moved.extend(value.to_be_bytes());
            }
            for value in rotation {
                moved.extend(value.to_be_bytes());
            }
            moved.push(0);
            responses.push((output::POSITION_LOOK, moved));
        }
        input::MAP_CHUNK => {
            let chunk = [r.i32()?, r.i32()?];
            let pistons = state.world.load(payload, max_chunks)?;
            operations::mining_chunk_changed(state, chunk);
            operations::placement_chunk_changed(state, chunk);
            if let Some(capture) = &mut state.recording {
                capture.chunk_changed(chunk);
            }
            state
                .reconstruction
                .chunk_loaded(chunk, pistons, state.sequence);
        }
        input::BLOCK_CHANGE => {
            let changes = state.world.block_change(payload)?;
            if let Some(capture) = &mut state.recording {
                capture.received(&changes, state.sequence, state.reconstruction.tick)?;
            }
            state.reconstruction.received(&changes);
            operations::mining_received(state, &changes)?;
            operations::placement_received(state, &changes)?;
        }
        input::MULTI_BLOCK_CHANGE => {
            let changes = state.world.section_changes(payload)?;
            if let Some(capture) = &mut state.recording {
                capture.received(&changes, state.sequence, state.reconstruction.tick)?;
            }
            state.reconstruction.received(&changes);
            operations::mining_received(state, &changes)?;
            operations::placement_received(state, &changes)?;
        }
        input::UNLOAD_CHUNK => {
            let z = r.i32()?;
            let x = r.i32()?;
            state.world.unload(payload)?;
            if let Some(capture) = &mut state.recording {
                capture.chunk_changed([x, z]);
            }
            state.reconstruction.chunk_replaced([x, z]);
            operations::mining_chunk_changed(state, [x, z]);
            operations::placement_chunk_changed(state, [x, z]);
        }
        input::BLOCK_ACTION => {
            let p = super::wire::unpack_position(r.u64()?);
            let action = r.u8()?;
            let parameter = r.u8()?;
            let block_id = r.varint()?;
            r.end()?;
            let name = super::registry()
                .block_name(block_id)
                .context("unknown block action ID")?;
            if matches!(name, "piston" | "sticky_piston") {
                let action = Action::from_id(action).context("unknown piston action")?;
                let facing =
                    Direction::from_id(parameter & 7).context("invalid piston direction")?;
                state.reconstruction.action(
                    &state.world,
                    p,
                    action,
                    facing,
                    &format!("minecraft:{name}"),
                    state.sequence,
                );
            }
        }
        input::SET_TICKING_STATE => {
            let rate = r.f32()?;
            let frozen = r.bool()?;
            r.end()?;
            if rate != 20.0 || frozen {
                state.reconstruction.unsupported_ticking();
            }
        }
        input::STEP_TICK => {
            let steps = r.count(1_000_000)?;
            r.end()?;
            if steps != 0 {
                state.reconstruction.unsupported_ticking();
            }
        }
        input::CHUNK_BATCH_FINISHED => {
            r.count(65_536)?;
            r.end()?;
            responses.push((output::CHUNK_BATCH_RECEIVED, 8f32.to_be_bytes().to_vec()));
        }
        input::START_CONFIGURATION => {
            r.end()?;
            state.loading.reset(state.sequence);
            state
                .motion
                .invalidate(state.sequence, "world generation changed");
            operations::mining_world_changed(state, "configuration changed mining context");
            state.phase = Phase::Configuration;
            state.ready = false;
            state.position = None;
            state.dimensions.clear();
            state.registries.reset(state.sequence);
            state.recipes = Default::default();
            state.world.reset();
            state.reconstruction = Reconstruction::default();
            state.operations.reset_configuration(state.sequence);
            state.players = players::PlayerTracker::default();
            state.entities.clear();
            state.vehicles.clear();
            if let Some(capture) = &mut state.recording {
                capture.invalidate(recording::RecordingIssue::WorldChanged);
            }
            responses.push((output::CONFIGURATION_ACKNOWLEDGED, vec![]));
            responses.push((ids::configuration_serverbound::SETTINGS, settings()));
        }
        input::KICK_DISCONNECT => bail!("server disconnected"),
        input::TAGS => {
            state
                .registries
                .receive_tags(payload, state.sequence, MinecraftVersion::Java1_21_11)?
        }
        input::TRANSFER => bail!("server transfer requires a new explicitly configured connection"),
        input::ADD_RESOURCE_PACK => bail!("resource-pack negotiation is unsupported"),
        input::COOKIE_REQUEST => {
            let key = r.string()?;
            r.end()?;
            let mut response = Vec::new();
            put_string(&mut response, &key);
            response.push(0);
            responses.push((output::COOKIE_RESPONSE, response));
        }
        // Other known packets currently expose no advertised observation/action capability.
        id if ids::KNOWN_PLAY_CLIENTBOUND.contains(&id) => {}
        _ => bail!("unknown Java 1.21.11 play packet {id}"),
    }
    Ok(responses)
}

#[cfg(test)]
mod mining_native_trials;
#[cfg(test)]
mod movement_native_trials;
#[cfg(test)]
mod placement_native_trials;
#[cfg(test)]
mod tests;

impl Bot {
    pub(crate) async fn common_scoreboard_state(
        &self,
    ) -> Result<crate::client::ui::ScoreboardObservation> {
        let state = self.session.state.lock().await;
        self.session.check(&state)?;
        let player = operations::common_player_in_state(&state, self.session.id, false)?;
        Ok(state.scoreboard.capture(player.session, state.sequence))
    }
}

impl Bot {
    pub(crate) async fn common_boss_bars(&self) -> Result<crate::client::ui::BossBarsObservation> {
        let state = self.session.state.lock().await;
        self.session.check(&state)?;
        let player = operations::common_player_in_state(&state, self.session.id, false)?;
        Ok(state.boss_bars.capture(player.session, state.sequence))
    }
}
