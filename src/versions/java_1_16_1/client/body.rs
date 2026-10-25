//! Explicit placement uses the normal actor writer, without strict cube diagnostics.
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
                let (Action::UseItem { hand }, Some((selected, item))) = (action, expected) else {
                    return Err(crate::error::invalid_input(
                        "expected atomic body item-use action",
                    ));
                };
                let action = api::operations::Action::UseItemExpect(*hand, selected, item);
                self.execute_common_unlocked(mode, action, None)
                    .await
                    .map(|_| ())
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
                let Action::PlaceBlock {
                    pos,
                    face,
                    cursor,
                    hand,
                    ..
                } = action
                else {
                    return Err(crate::Error::new(
                        crate::ErrorKind::Unsupported,
                        anyhow::anyhow!("legacy body supports placement only"),
                    ));
                };
                crate::client::item_use::validate_cursor(*cursor)?;
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
                self.place_block(
                    *hand,
                    BlockPos {
                        x: pos[0],
                        y: pos[1],
                        z: pos[2],
                    },
                    *face,
                    *cursor,
                    false,
                )
                .await?;
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
        assert_eq!(body.capabilities().actions, vec![ActionKind::PlaceBlock]);
        assert!(body.capabilities().observation);
        assert!(
            body.submit(Action::Look { yaw: 0., pitch: 0. })
                .await
                .is_err()
        );
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
}
