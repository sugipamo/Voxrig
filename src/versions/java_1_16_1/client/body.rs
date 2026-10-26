//! Acceptance actions reuse native send paths under the coherent world fence.
//! 736 has no placement sequence ACK: only fresh original target updates are evidence.
use super::*;
use crate::body::evidence::{EvidenceBatch, Update, slot, source, value};
use crate::body::{self, *};
use crate::client as api;

impl Bot {
    pub(crate) async fn body_evidence(
        &self,
        backend: u64,
        request: &ObserveRequest,
        cursor: Option<u64>,
    ) -> Result<EvidenceBatch> {
        let _gate = self.coherent_state_gate.lock().await;
        self.check_received_snapshot()?;
        let p = self.common_player_unlocked().await?;
        let mut positions = request.blocks.clone();
        if let Some(region) = request.region {
            region.volume()?;
            for x in region.min[0]..=region.max[0] {
                for y in region.min[1]..=region.max[1] {
                    for z in region.min[2]..=region.max[2] {
                        positions.push([x, y, z]);
                    }
                }
            }
        }
        positions.sort_unstable();
        positions.dedup();
        let world = self.world.lock().await;
        let blocks = positions
            .into_iter()
            .map(|position| {
                // Outside the native world height is unavailable, not inferred air.
                let state = if (0..256).contains(&position[1]) {
                    world.block(position[0], position[1], position[2])
                } else {
                    None
                };
                Ok(BodyBlock {
                    position,
                    state: state
                        .map(crate::versions::java_1_16_1::native_state)
                        .transpose()?
                        .map(|value| Value {
                            value,
                            source: body::ValueSource::ReceivedCache {
                                revision: p.receive_sequence,
                            },
                        }),
                })
            })
            .collect::<Result<_>>()?;
        let snapshot = BodySnapshot {
            backend,
            version: crate::MinecraftVersion::Java1_16_1,
            world: BodyWorld {
                connection: p.session.connection_id,
                generation: p.session.world_generation,
            },
            revision: p.receive_sequence,
            player: BodyPlayer {
                position: p.position.map(value),
                rotation: p.rotation_source.map(|s| Value {
                    value: p.rotation,
                    source: source(s),
                }),
                game_mode: p.game_mode,
                health: p.health.map(value),
                selected_hotbar: p.selected_hotbar.map(value),
                inventory: p.inventory.slots.into_iter().map(|s| s.map(slot)).collect(),
                cursor: p.inventory.cursor.map(slot),
            },
            blocks,
        };
        let log = self.body_evidence.lock().await;
        Ok(EvidenceBatch {
            cursor: log.next,
            gap: cursor.is_some_and(|c| c < log.dropped),
            events: log
                .events
                .iter()
                .filter(|e| cursor.is_some_and(|c| e.ordinal > c))
                .cloned()
                .collect(),
            ack: None,
            snapshot,
        })
    }
    pub(crate) async fn body_simple_send(
        &self,
        world: BodyWorld,
        mode: api::GameMode,
        action: &Action,
        expected: Option<(u8, api::registry::RegistryId)>,
    ) -> Result<()> {
        self.connection
            .ledger()
            .try_work()?
            .run_scoped(async {
                let _gate = self.coherent_state_gate.lock().await;
                let generation = self.common_receipts.lock().await.generation;
                if world
                    != (BodyWorld {
                        connection: self.connection_id(),
                        generation,
                    })
                {
                    return Err(crate::error::invalid_input(
                        "body world changed before item dispatch",
                    ));
                }
                self.check_received_snapshot()?;
                if self.connection_state() != ConnectionState::Ready {
                    return Err(crate::error::state_error("body connection is not ready"));
                }
                let domain = match action {
                    Action::Look { .. } => api::domain::Domain::Motion,
                    Action::SelectHotbar(_) => api::domain::Domain::Selection,
                    Action::Chat(_) | Action::Command(_) => api::domain::Domain::Chat,
                    _ => api::domain::Domain::ItemUse,
                };
                self.common_domain_admission(false, None, domain).await?;
                match action {
                    Action::Look { yaw, pitch } => {
                        api::operations::validate_rotation([*yaw, *pitch])?;
                        if !*self.positioned.lock().await {
                            return Err(crate::error::state_error(
                                "body look position unavailable",
                            ));
                        }
                        let mut p = self.player.lock().await;
                        p.yaw = *yaw;
                        p.pitch = pitch.clamp(-90., 90.);
                        let snapshot = p.clone();
                        drop(p);
                        self.common_receipts.lock().await.rotation_source =
                            Some(api::ValueSource::Submitted);
                        let mut payload = Vec::new();
                        for v in [snapshot.x, snapshot.y, snapshot.z] {
                            payload.extend(v.to_be_bytes());
                        }
                        for v in [snapshot.yaw, snapshot.pitch] {
                            payload.extend(v.to_be_bytes());
                        }
                        payload.push(u8::from(snapshot.on_ground));
                        self.send_in_domain(domain, 0x13, &payload).await?;
                        self.physics.lock().await.record_movement(snapshot);
                        Ok(())
                    }
                    Action::SelectHotbar(slot) => {
                        if *slot > 8 {
                            return Err(crate::error::invalid_input("hotbar slot must be 0..8"));
                        }
                        self.common_receipts.lock().await.selected_hotbar = None;
                        self.send_in_domain(domain, 0x24, &i16::from(*slot).to_be_bytes())
                            .await?;
                        self.inventory.write().await.selected_hotbar = *slot;
                        self.common_receipts.lock().await.selected_hotbar =
                            Some(api::ObservedValue {
                                value: *slot,
                                source: api::ValueSource::Submitted,
                            });
                        Ok(())
                    }
                    Action::Chat(text) => {
                        api::chat::validate_chat(text)?;
                        let mut payload = Vec::new();
                        put_string(&mut payload, text);
                        self.send_in_domain(domain, 0x03, &payload).await
                    }
                    Action::Command(text) => {
                        let command = api::chat::validate_command(text)?;
                        let mut payload = Vec::new();
                        put_string(&mut payload, &format!("/{command}"));
                        self.send_in_domain(domain, 0x03, &payload).await
                    }
                    Action::UseItem { hand } => {
                        if let Some((selected, item)) = expected {
                            self.validate_use_item_expected_unlocked(*hand, selected, item)
                                .await?;
                            self.common_motion_admission().await?;
                        }
                        if self.survival.read().await.game_mode.map(|id| id & 7)
                            != Some(match mode {
                                api::GameMode::Survival => 0,
                                api::GameMode::Creative => 1,
                                api::GameMode::Adventure => 2,
                                api::GameMode::Spectator => 3,
                            })
                        {
                            return Err(crate::error::state_error(
                                "body item use requires matching received mode",
                            ));
                        }
                        let mut payload = Vec::new();
                        put_varint(&mut payload, *hand as i32);
                        let send_domain = if expected.is_some() {
                            api::domain::Domain::Strict
                        } else {
                            domain
                        };
                        self.send_in_domain(send_domain, 0x2e, &payload).await
                    }
                    _ => Err(crate::error::invalid_input("expected one-shot body action")),
                }
            })
            .await
    }
    pub(super) async fn body_dig_hand_admission(&self, selected: Option<u8>) -> Result<()> {
        use api::domain;
        let click = super::lock_packet_state(&self.common_inventory_click).await;
        if click
            .as_ref()
            .is_some_and(|r| !r.released && domain::click_changes_hand(&r.record, selected))
        {
            return Err(api::inventory::unavailable(
                "pending click can change the retained dig hand",
            ));
        }
        let mut expected = Vec::new();
        if let Some(r) = click.as_ref().filter(|r| !r.released) {
            if let Some(action) = r.record.send.legacy_action {
                expected.push((r.record.window_id() as i8, action));
            }
        }
        drop(click);
        let swap = super::lock_packet_state(&self.common_inventory_swap).await;
        if swap
            .as_ref()
            .is_some_and(|r| !r.released && domain::swap_changes_hand(&r.record, selected))
        {
            return Err(api::inventory::unavailable(
                "pending swap can change the retained dig hand",
            ));
        }
        if let Some(r) = swap.as_ref().filter(|r| !r.released) {
            if let Some(action) = r.record.send.legacy_action {
                expected.push((r.record.window_id() as i8, action));
            }
        }
        drop(swap);
        let transfer = super::lock_packet_state(&self.common_inventory_transfer).await;
        if transfer
            .as_ref()
            .is_some_and(|r| !r.released && domain::transfer_changes_hand(&r.record, selected))
        {
            return Err(api::inventory::unavailable(
                "pending transfer can change the retained dig hand",
            ));
        }
        if let Some(r) = transfer.as_ref().filter(|r| !r.released) {
            if let Some(action) = r.record.send.legacy_action {
                expected.push((r.record.window_id() as i8, action));
            }
        }
        drop(transfer);
        if self
            .inventory
            .read()
            .await
            .pending_clicks
            .keys()
            .any(|key| !expected.contains(key))
        {
            return Err(api::inventory::unavailable(
                "native pending inventory hand is unavailable",
            ));
        }
        if super::lock_packet_state(&self.common_recipe_placement)
            .await
            .as_ref()
            .is_some_and(|r| !r.released)
            || super::lock_packet_state(&self.common_crafting_take)
                .await
                .as_ref()
                .is_some_and(|r| !r.released)
            || super::lock_packet_state(&self.common_container_close)
                .await
                .as_ref()
                .is_some_and(|r| r.unresolved())
            || super::lock_packet_state(&self.common_container_open)
                .await
                .as_ref()
                .is_some_and(|r| !r.released)
        {
            return Err(api::inventory::unavailable(
                "pending cursor return/crafting can change the retained dig hand",
            ));
        }
        Ok(())
    }
    pub(crate) async fn body_break_send(
        &self,
        world: BodyWorld,
        action: &Action,
        status: i32,
        initial: Option<&BodySnapshot>,
    ) -> Result<()> {
        self.connection
            .ledger()
            .try_work()?
            .run_scoped(async {
                let _gate = self.coherent_state_gate.lock().await;
                self.check_received_snapshot()?;
                let generation = self.common_receipts.lock().await.generation;
                if world
                    != (BodyWorld {
                        connection: self.connection_id(),
                        generation,
                    })
                {
                    // An old-world cancellation must never abort a new-world dig.
                    return if status == 1 {
                        Ok(())
                    } else {
                        Err(crate::error::invalid_input(
                            "body world changed before dig dispatch",
                        ))
                    };
                }
                if self.connection_state() != ConnectionState::Ready {
                    return Err(crate::error::state_error(
                        "body dig connection is not ready",
                    ));
                }
                self.common_domain_admission(false, None, api::domain::Domain::Dig)
                    .await?;
                let Action::BreakBlock { pos, face } = action else {
                    return Err(crate::error::invalid_input("expected body break action"));
                };
                if !(-33_554_432..=33_554_431).contains(&pos[0])
                    || !(0..256).contains(&pos[1])
                    || !(-33_554_432..=33_554_431).contains(&pos[2])
                {
                    return Err(crate::error::invalid_input(
                        "body dig position exceeds native world bounds",
                    ));
                }
                let player = self.common_player_unlocked().await?;
                if status != 1 {
                    if player.game_mode == Some(api::GameMode::Survival) {
                        self.body_dig_hand_admission(
                            player.selected_hotbar.as_ref().map(|s| s.value),
                        )
                        .await?;
                    }
                    if !matches!(
                        player.game_mode,
                        Some(api::GameMode::Survival | api::GameMode::Creative)
                    ) {
                        return Err(crate::error::invalid_input(
                            "body break requires survival or creative mode",
                        ));
                    }
                    if let Some(feet) = player.position.as_ref().map(|p| p.value) {
                        let distance: f64 = (0..3)
                            .map(|i| {
                                let offset = if i == 1 { 1.5 } else { 0. };
                                (feet[i] + offset - (f64::from(pos[i]) + 0.5)).powi(2)
                            })
                            .sum();
                        if distance > 36. {
                            return Err(crate::error::invalid_input(
                                "body dig exceeds official six-block server range",
                            ));
                        }
                    }
                    if player.game_mode == Some(api::GameMode::Survival) {
                        let world = self.world.lock().await;
                        crate::body::dig::hardness(
                            crate::MinecraftVersion::Java1_16_1,
                            world.block(pos[0], pos[1], pos[2]),
                        )?;
                    }
                    if let Some(initial) = initial {
                        crate::body::dig::hand(initial, &player)?;
                    }
                }
                let status = match status {
                    0 => DiggingStatus::Started,
                    1 => DiggingStatus::Cancelled,
                    2 => DiggingStatus::Finished,
                    _ => return Err(crate::error::invalid_input("invalid body dig status")),
                };
                let mut payload = Vec::new();
                put_varint(&mut payload, status as i32);
                payload.extend(
                    BlockPos {
                        x: pos[0],
                        y: pos[1],
                        z: pos[2],
                    }
                    .packed()
                    .to_be_bytes(),
                );
                payload.push(*face as u8);
                self.send_in_domain(api::domain::Domain::Dig, 0x1b, &payload)
                    .await
            })
            .await
    }
    pub(crate) async fn body_place_send(&self, world: BodyWorld, action: &Action) -> Result<u64> {
        self.connection
            .ledger()
            .try_work()?
            .run_scoped(async {
                let _gate = self.coherent_state_gate.lock().await;
                self.check_received_snapshot()?;
                if self.connection_state() != ConnectionState::Ready {
                    return Err(crate::error::state_error(
                        "body placement connection is not ready",
                    ));
                }
                let generation = self.common_receipts.lock().await.generation;
                if world
                    != (BodyWorld {
                        connection: self.connection_id(),
                        generation,
                    })
                {
                    return Err(crate::error::invalid_input(
                        "body world changed before placement dispatch",
                    ));
                }
                let domain = if matches!(action, Action::ActivateBlock { .. }) {
                    api::domain::Domain::Screen
                } else {
                    api::domain::Domain::ItemUse
                };
                self.common_domain_admission(false, None, domain).await?;
                let (Action::PlaceBlock {
                    pos,
                    face,
                    cursor,
                    hand,
                    ..
                }
                | Action::ActivateBlock {
                    pos,
                    face,
                    cursor,
                    hand,
                }) = action
                else {
                    return Err(crate::error::invalid_input(
                        "expected body placement or activation action",
                    ));
                };
                crate::client::item_use::validate_cursor(*cursor)?;
                let player = self.common_player_unlocked().await?;
                crate::body::interaction::validate_server_range(&player, None, *pos)?;
                if !(-33_554_432..=33_554_431).contains(&pos[0])
                    || !(-2048..=2047).contains(&pos[1])
                    || !(-33_554_432..=33_554_431).contains(&pos[2])
                {
                    return Err(crate::error::invalid_input(
                        "body placement position exceeds wire bounds",
                    ));
                }
                let boundary = self.body_evidence.lock().await.next;
                // The gate prevents any received event from crossing this boundary
                // until the complete actor-owned write has returned.
                let mut payload = Vec::new();
                put_varint(&mut payload, *hand as i32);
                payload.extend(
                    BlockPos {
                        x: pos[0],
                        y: pos[1],
                        z: pos[2],
                    }
                    .packed()
                    .to_be_bytes(),
                );
                put_varint(&mut payload, *face as i32);
                for value in cursor {
                    payload.extend(value.to_be_bytes());
                }
                payload.push(0);
                self.send_in_domain(domain, 0x2d, &payload).await?;
                Ok(boundary)
            })
            .await
    }
    pub(super) async fn body_block_received(&self, pos: [i32; 3], id: i32, sequence: u64) {
        self.body_evidence
            .lock()
            .await
            .push(sequence, Update::Block(pos, id));
    }
    pub(super) async fn body_chunk_received(&self, chunk: [i32; 2]) {
        self.body_evidence.lock().await.push(
            self.protocol_packet_sequence.load(Ordering::Acquire),
            Update::Chunk(chunk),
        );
    }
    pub(super) async fn body_received(&self, id: i32, sequence: u64) {
        let update = match id {
            0x25 | 0x3a => Some(Update::World),
            0x35 => self
                .common_receipts
                .lock()
                .await
                .pose
                .as_ref()
                .filter(|p| p.receive_sequence == sequence)
                .map(|p| Update::Pose {
                    position: p.position,
                    rotation: p.rotation,
                }),
            0x3f => self
                .common_receipts
                .lock()
                .await
                .selected_hotbar
                .as_ref()
                .map(|s| Update::Selection(s.value)),
            0x14 => Some(Update::Resync),
            _ => None,
        };
        if let Some(update) = update {
            self.body_evidence.lock().await.push(sequence, update);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{Body, HeadlessBody};
    fn block(name: &str) -> crate::NativeBlockState {
        crate::NativeBlockState {
            name: format!("minecraft:{name}"),
            properties: Default::default(),
        }
    }
    fn action() -> Action {
        Action::PlaceBlock {
            pos: [10, 64, 8],
            face: crate::BlockFace::Up,
            cursor: [0.17, 1., 0.83],
            hand: crate::Hand::Off,
            target: [10, 65, 8],
            expected: Some(block("gold_block")),
        }
    }
    fn body(bot: &Bot) -> Body<HeadlessBody> {
        Body::new(
            HeadlessBody::new(
                crate::Client::from_java_1_16_1(bot.clone()),
                BodyConfig {
                    settle_window: Duration::from_millis(20),
                    action_timeout: Duration::from_millis(180),
                    ..Default::default()
                },
            )
            .unwrap(),
        )
    }
    async fn complete(body: &Body<HeadlessBody>, id: ActionId) -> ActionStatus {
        timeout(Duration::from_secs(2), async {
            loop {
                let s = body.action_status(id).await.unwrap();
                if s.is_terminal() {
                    return s;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn change(bot: &Bot, name: &str) {
        let mut p = BlockPos { x: 10, y: 65, z: 8 }
            .packed()
            .to_be_bytes()
            .to_vec();
        put_varint(
            &mut p,
            crate::versions::java_1_16_1::state_id(&block(name)).unwrap(),
        );
        bot.apply_packet(0x0b, p).await.unwrap();
    }
    async fn ack(bot: &Bot, state: &str, status: i32, successful: bool) {
        let mut payload = BlockPos { x: 10, y: 65, z: 8 }
            .packed()
            .to_be_bytes()
            .to_vec();
        put_varint(
            &mut payload,
            crate::versions::java_1_16_1::state_id(&block(state)).unwrap(),
        );
        put_varint(&mut payload, status);
        payload.push(u8::from(successful));
        bot.apply_packet(0x07, payload).await.unwrap();
    }
    async fn wait_release(body: &Body<HeadlessBody>) {
        // Wait for the action task to retire ownership without cancelling it.
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert_eq!(
            body.release_control().await.unwrap(),
            ReleaseStatus::Released
        );
    }
    #[tokio::test]
    async fn legacy_body_simple_actions_send_native_packets_and_accept_without_echo() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let body = body(&bot);
        for (action, packet) in [
            (
                Action::Look {
                    yaw: 37.,
                    pitch: 13.,
                },
                0x13,
            ),
            (Action::SelectHotbar(8), 0x24),
            (Action::Chat("hello".into()), 0x03),
            (Action::Command("/say body".into()), 0x03),
            (
                Action::UseItem {
                    hand: crate::Hand::Off,
                },
                0x2e,
            ),
        ] {
            let id = body.submit(action).await.unwrap();
            let (got, payload) = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got, packet);
            match got {
                0x13 => {
                    assert_eq!(&payload[24..28], &37f32.to_be_bytes());
                    assert_eq!(&payload[28..32], &13f32.to_be_bytes());
                }
                0x24 => assert_eq!(payload, vec![0, 8]),
                0x03 => assert!(payload.ends_with(b"hello") || payload.ends_with(b"/say body")),
                0x2e => assert_eq!(payload, vec![1]),
                _ => unreachable!(),
            }
            assert!(matches!(complete(&body, id).await, ActionStatus::Accepted));
            wait_release(&body).await;
        }
        for action in [
            Action::SelectHotbar(9),
            Action::Chat("bad\nline".into()),
            Action::Command("".into()),
            Action::Look {
                yaw: f32::NAN,
                pitch: 0.,
            },
        ] {
            let id = body.submit(action).await.unwrap();
            assert!(matches!(
                complete(&body, id).await,
                ActionStatus::Rejected { .. }
            ));
            wait_release(&body).await;
        }
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn legacy_body_native_dispatch_keeps_world_fence_for_all_paths() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let body = body(&bot);
        let mut world = body.observe(Default::default()).await.unwrap().world;
        for wrong_connection in [false, true] {
            if wrong_connection {
                world.connection += 1;
            } else {
                world.generation += 1;
            }
            let error = bot
                .body_simple_send(
                    world,
                    api::GameMode::Survival,
                    &Action::UseItem {
                        hand: crate::Hand::Main,
                    },
                    None,
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("world changed"));
            assert!(
                bot.body_start_control(api::GameMode::Survival, world)
                    .await
                    .is_err()
            );
            assert!(
                bot.body_break_send(
                    world,
                    &Action::BreakBlock {
                        pos: [10, 65, 8],
                        face: crate::BlockFace::Up
                    },
                    0,
                    None,
                )
                .await
                .is_err()
            );
        }
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn legacy_body_move_uses_owned_ticks_and_releases_after_acceptance() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let body = Body::new(
            HeadlessBody::new(
                crate::Client::from_java_1_16_1(bot.clone()),
                BodyConfig {
                    settle_window: Duration::from_millis(20),
                    action_timeout: Duration::from_secs(1),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let id = body
            .submit(Action::Move {
                controls: api::control::Controls {
                    forward: 1,
                    ..Default::default()
                },
                ticks: Some(2),
            })
            .await
            .unwrap();
        assert!(matches!(complete(&body, id).await, ActionStatus::Accepted));
        wait_release(&body).await;
        let mut positions = 0;
        while let Ok((packet, _)) = packets.try_recv() {
            if packet == 0x13 {
                positions += 1;
            }
        }
        assert!(positions >= 2);
        let record = api::adapter::ControlOps::control_record(&bot)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            record.status,
            api::control::ControlStatus::Stopped { .. }
        ));
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn legacy_body_dig_rechecks_hand_inside_dispatch_and_keeps_server_range() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        {
            let mut r = bot.common_receipts.lock().await;
            r.selected_hotbar = Some(api::ObservedValue {
                value: 0,
                source: api::ValueSource::Submitted,
            });
            r.inventory.slots[36] = Some(api::ObservedValue {
                value: api::SlotKnowledge::Empty,
                source: api::ValueSource::Received { sequence: 1 },
            });
        }
        let body = body(&bot);
        let initial = body.observe(Default::default()).await.unwrap();
        bot.common_receipts
            .lock()
            .await
            .selected_hotbar
            .as_mut()
            .unwrap()
            .value = 1;
        let action = Action::BreakBlock {
            pos: [10, 65, 8],
            face: crate::BlockFace::Up,
        };
        assert!(
            bot.body_break_send(initial.world, &action, 2, Some(&initial))
                .await
                .unwrap_err()
                .to_string()
                .contains("identity changed")
        );
        let far = Action::BreakBlock {
            pos: [15, 65, 8],
            face: crate::BlockFace::Up,
        };
        assert!(
            bot.body_break_send(initial.world, &far, 0, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("server range")
        );
        let mut old = initial.world;
        old.generation += 1;
        bot.body_break_send(old, &action, 1, None).await.unwrap();
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn legacy_body_break_requires_result_ack_and_reports_rollback_world_loss_or_timeout() {
        for scenario in ["accepted", "block", "rollback", "world", "timeout"] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.survival.write().await.game_mode = Some(1);
            let body = body(&bot);
            let id = body
                .submit(Action::BreakBlock {
                    pos: [10, 65, 8],
                    face: crate::BlockFace::West,
                })
                .await
                .unwrap();
            let (packet, payload) = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(packet, 0x1b);
            assert_eq!(payload[0], 0);
            assert_eq!(payload[9], crate::BlockFace::West as u8);
            match scenario {
                "accepted" => ack(&bot, "air", 0, true).await,
                "rollback" => ack(&bot, "stone", 0, false).await,
                "block" => change(&bot, "air").await,
                "world" => bot.common_receipts.lock().await.generation += 1,
                _ => {} // no server evidence
            }
            let status = complete(&body, id).await;
            match scenario {
                "accepted" | "block" => {
                    assert!(matches!(status, ActionStatus::Accepted), "{status:?}")
                }
                "rollback" => assert!(
                    matches!(status, ActionStatus::Corrected { .. }),
                    "{status:?}"
                ),
                _ => assert!(matches!(status, ActionStatus::Unknown { .. }), "{status:?}"),
            }
            if scenario == "timeout" {
                ack(&bot, "stone", 0, false).await;
                assert!(matches!(
                    body.backend().action_outcome(id).await.unwrap(),
                    ActionStatus::Corrected { .. }
                ));
                assert!(matches!(
                    body.action_status(id).await.unwrap(),
                    ActionStatus::Unknown { .. }
                ));
            }
            wait_release(&body).await;
            drop(release);
            drop(body);
            drop(bot);
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn legacy_body_timed_break_start_ack_is_not_rollback_and_cancel_sends_one_abort() {
        for cancel in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            change(&bot, "dirt").await;
            bot.common_receipts.lock().await.selected_hotbar = Some(api::ObservedValue {
                value: 0,
                source: api::ValueSource::Submitted,
            });
            bot.common_receipts.lock().await.inventory.slots[36] = Some(api::ObservedValue {
                value: api::SlotKnowledge::Empty,
                source: api::ValueSource::Received { sequence: 1 },
            });
            let body = Body::new(
                HeadlessBody::new(
                    crate::Client::from_java_1_16_1(bot.clone()),
                    BodyConfig {
                        settle_window: Duration::from_millis(20),
                        action_timeout: Duration::from_secs(4),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let id = body
                .submit(Action::BreakBlock {
                    pos: [10, 65, 8],
                    face: crate::BlockFace::Up,
                })
                .await
                .unwrap();
            let (packet, payload) = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!((packet, payload[0]), (0x1b, 0));
            if !cancel {
                ack(&bot, "dirt", 0, true).await;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(matches!(
                body.action_status(id).await.unwrap(),
                ActionStatus::Pending
            ));
            if cancel {
                body.cancel(id).await.unwrap();
            }
            let (packet, payload) = timeout(Duration::from_secs(3), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!((packet, payload[0]), (0x1b, if cancel { 1 } else { 2 }));
            if !cancel {
                ack(&bot, "air", 2, true).await;
            }
            let status = complete(&body, id).await;
            assert!(
                if cancel {
                    matches!(status, ActionStatus::Cancelled)
                } else {
                    matches!(status, ActionStatus::Accepted)
                },
                "{status:?}"
            );
            wait_release(&body).await;
            assert!(
                timeout(Duration::from_millis(30), packets.recv())
                    .await
                    .is_err()
            );
            drop(release);
            drop(body);
            drop(bot);
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn legacy_body_dig_unknown_selection_or_slot_and_delayed_stop_are_not_refusals() {
        for scenario in ["selection", "slot", "timeout"] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            change(&bot, "dirt").await;
            {
                let mut r = bot.common_receipts.lock().await;
                r.selected_hotbar = if scenario == "selection" {
                    None
                } else {
                    Some(api::received(0, 1))
                };
                r.inventory.slots[36] = None;
            }
            let client = crate::Client::from_java_1_16_1(bot.clone());
            assert!(client.survival().dig_estimate([10, 65, 8]).await.is_err());
            let body = body(&bot);
            let id = body
                .submit(Action::BreakBlock {
                    pos: [10, 65, 8],
                    face: crate::BlockFace::Up,
                })
                .await
                .unwrap();
            let (packet, payload) = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!((packet, payload[0]), (0x1b, 0));
            // No STOP without a START processing ACK.
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            ack(&bot, "dirt", 0, true).await;
            let (packet, payload) = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!((packet, payload[0]), (0x1b, 2));
            ack(&bot, "dirt", 2, true).await; // official delayed destroy, not a rollback
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(matches!(
                body.action_status(id).await.unwrap(),
                ActionStatus::Pending
            ));
            if scenario != "timeout" {
                change(&bot, "air").await;
            }
            let result = complete(&body, id).await;
            if scenario == "timeout" {
                assert!(matches!(result, ActionStatus::Unknown { .. }), "{result:?}");
                let (packet, payload) = timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!((packet, payload[0]), (0x1b, 1));
                ack(&bot, "dirt", 1, true).await;
                assert!(matches!(
                    body.backend().action_outcome(id).await.unwrap(),
                    ActionStatus::Unknown { .. }
                ));
            } else {
                assert!(matches!(result, ActionStatus::Accepted), "{result:?}");
            }
            wait_release(&body).await;
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            drop(release);
            drop(client);
            drop(body);
            drop(bot);
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn explicit_body_place_omits_strict_material_cursor_pose_ground_and_outline_filters() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.on_ground = false;
        *bot.local_pose.lock().await = Some(5); // swimming; no standing prerequisite
        bot.survival.write().await.game_mode = None;
        bot.inventory.write().await.cursor = Some(ItemStack {
            item_id: crate::item_id("diamond").unwrap(),
            count: 3,
            nbt: None,
        });
        let body = body(&bot);
        assert!(body.capabilities().actions.contains(&ActionKind::Look));
        assert!(body.capabilities().actions.contains(&ActionKind::Move));
        assert!(
            body.capabilities()
                .actions
                .contains(&ActionKind::BreakBlock)
        );
        assert!(body.capabilities().observation);
        assert!(
            crate::Client::from_java_1_16_1(bot.clone())
                .survival()
                .place_cube([10, 64, 8], crate::BlockFace::Up)
                .await
                .is_err()
        );
        let id = body.submit(action()).await.unwrap();
        let (packet, p) = timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(packet, 0x2d);
        assert_eq!(p.len(), 23);
        assert_eq!(p[0], 1);
        assert_eq!(p[9], 1);
        assert_eq!(&p[10..14], &0.17f32.to_be_bytes());
        assert_eq!(&p[14..18], &1f32.to_be_bytes());
        assert_eq!(&p[18..22], &0.83f32.to_be_bytes());
        change(&bot, "gold_block").await;
        assert!(matches!(complete(&body, id).await, ActionStatus::Accepted));
        let captured = body
            .observe(ObserveRequest {
                blocks: vec![[10, 65, 8], [10, -1, 8]],
                region: None,
            })
            .await
            .unwrap();
        assert!(
            captured
                .blocks
                .iter()
                .any(|b| b.position[1] == -1 && b.state.is_none())
        );
        let got = captured
            .blocks
            .iter()
            .find(|b| b.position[1] == 65)
            .unwrap();
        assert_eq!(got.state.as_ref().unwrap().value, block("gold_block"));
        assert!(matches!(
            got.state.as_ref().unwrap().source,
            body::ValueSource::ReceivedCache { .. }
        ));
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn legacy_body_cached_match_is_unknown_and_fresh_rollback_corrects_immutably() {
        for rollback in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            change(&bot, "gold_block").await; // pre-send evidence cannot confirm this action
            let body = body(&bot);
            let id = body.submit(action()).await.unwrap();
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .0,
                0x2d
            );
            if rollback {
                change(&bot, "diamond_block").await;
                change(&bot, "gold_block").await;
            }
            let result = complete(&body, id).await;
            if rollback {
                assert!(
                    matches!(result,ActionStatus::Corrected{correction:ServerCorrection::Block(ref b),..} if b.state.as_ref().unwrap().value==block("diamond_block"))
                );
            } else {
                assert!(matches!(result, ActionStatus::Unknown { .. }));
            }
            change(&bot, "gold_block").await;
            assert_eq!(
                serde_json::to_value(body.action_status(id).await.unwrap()).unwrap(),
                serde_json::to_value(result).unwrap()
            );
            drop(release);
            drop(body);
            drop(bot);
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn legacy_body_activation_is_bounded_without_block_ack_and_keeps_exact_hit() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.on_ground = false;
        let body = body(&bot);
        let id = body
            .submit(Action::ActivateBlock {
                pos: [10, 64, 8],
                face: crate::BlockFace::East,
                cursor: [1., 0.17, 0.83],
                hand: crate::Hand::Off,
            })
            .await
            .unwrap();
        let (packet, payload) = timeout(Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(packet, 0x2d);
        assert_eq!(payload[0], 1);
        assert_eq!(payload[9], crate::BlockFace::East as u8);
        assert_eq!(&payload[10..14], &1f32.to_be_bytes());
        assert_eq!(&payload[14..18], &0.17f32.to_be_bytes());
        assert_eq!(&payload[18..22], &0.83f32.to_be_bytes());
        assert!(matches!(complete(&body, id).await, ActionStatus::Accepted));
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn legacy_body_dispatch_fences_world_and_wire_bounds_before_send() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let body = body(&bot);
        let snapshot = body.observe(Default::default()).await.unwrap();
        let mut stale = snapshot.world;
        stale.generation += 1;
        let id = body
            .submit(ActionRequest {
                action: action(),
                world: Some(stale),
            })
            .await
            .unwrap();
        assert!(matches!(
            complete(&body, id).await,
            ActionStatus::Rejected { .. }
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut invalid = action();
        if let Action::PlaceBlock { pos, .. } = &mut invalid {
            pos[0] = 33_554_432;
        }
        let id = body.submit(invalid).await.unwrap();
        assert!(matches!(
            complete(&body, id).await,
            ActionStatus::Rejected { .. }
        ));
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn send_and_trust_returns_at_write_then_reports_late_correction_once() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let body = body(&bot);
        let sent = body
            .send_and_trust(Action::Look {
                yaw: 12.,
                pitch: 4.,
            })
            .await
            .unwrap();
        assert!(matches!(sent.dispatch, crate::body::SendStatus::Sent));
        assert_eq!(packets.recv().await.unwrap().0, 0x13);
        assert!(matches!(
            body.action_status(sent.id).await.unwrap(),
            ActionStatus::Pending
        ));
        assert!(matches!(
            complete(&body, sent.id).await,
            ActionStatus::Accepted
        ));
        wait_release(&body).await;
        bot.body_evidence.lock().await.push(
            100,
            Update::Pose {
                position: [1., 65., 0.],
                rotation: [0.; 2],
            },
        );
        // events_after alone refreshes late evidence, without a status poll.
        let events = body.events_after(Default::default(), 100).await.unwrap();
        assert_eq!(
            events
                .events
                .iter()
                .filter(
                    |e| e.action == sent.id && matches!(e.status, ActionStatus::Corrected { .. })
                )
                .count(),
            1
        );
        for _ in 0..2 {
            assert!(matches!(
                body.action_status(sent.id).await.unwrap(),
                ActionStatus::Corrected { .. }
            ));
        }
        assert_eq!(
            body.events_after(events.cursor, 100)
                .await
                .unwrap()
                .events
                .len(),
            0
        );
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn send_and_trust_keeps_k1_k2_refusals_and_no_packets() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let body = body(&bot);
        let mut stale = body.observe(Default::default()).await.unwrap().world;
        stale.generation += 1;
        let mut out_of_bounds = action();
        if let Action::PlaceBlock { pos, .. } = &mut out_of_bounds {
            pos[0] = 33_554_432;
        }
        for request in [
            ActionRequest {
                action: action(),
                world: Some(stale),
            },
            out_of_bounds.into(),
            Action::SelectHotbar(9).into(),
            Action::BreakBlock {
                pos: [30, 65, 8],
                face: crate::BlockFace::Up,
            }
            .into(),
        ] {
            let dispatch = body.send_and_trust(request).await.unwrap();
            assert!(matches!(
                dispatch.dispatch,
                crate::body::SendStatus::NotSent {
                    outcome: ActionStatus::Rejected { .. }
                }
            ));
            wait_release(&body).await;
        }
        assert!(packets.try_recv().is_err());
        drop(release);
        drop(body);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
