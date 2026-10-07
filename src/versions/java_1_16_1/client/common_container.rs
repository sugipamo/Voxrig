//! Owned empty-hand storage activation and opening-bound close; sends are not receipts.
use super::common_motion::CommonOwner;
use super::*;
use crate::client::{self as api, container as contract};
#[derive(Clone)]
pub(super) struct NativeContainerOpen {
    pub(super) record: contract::ContainerOpenRecord,
    pub(super) released: bool,
}

impl Bot {
    async fn open_native_inventory_admission(&self) -> Result<()> {
        let inventory = self.inventory.read().await;
        if inventory.cursor.is_some()
            || inventory.open_window.is_some()
            || !inventory.pending_clicks.is_empty()
        {
            return Err(api::inventory::unavailable(
                "native legacy player UI/mutation unresolved",
            ));
        }
        Ok(())
    }
    pub(super) async fn common_container_open_context_received(&self) -> Result<()> {
        let Some(before) = self
            .common_container_open
            .lock()
            .await
            .as_ref()
            .filter(|o| !o.released && o.record.requires_inspection.is_none())
            .map(|o| o.record.clone())
        else {
            return Ok(());
        };
        let query = self
            .common_target_in_mode(
                4.5,
                Some(CommonOwner::ContainerOpen(before.id)),
                before.mode,
            )
            .await;
        let current = match query {
            Ok(query)
                if query.hit.as_ref().is_some_and(|h| {
                    h.position == before.target.position
                        && (h.state == before.target.state
                            || (before.send.dispatched && h.state.name == "minecraft:barrel"))
                        && h.face == before.target.face
                        && h.point
                            .iter()
                            .zip(before.target.point)
                            .all(|(a, b)| (a - b).abs() < 1e-9)
                }) =>
            {
                let hit = query.hit.as_ref().expect("checked hit");
                let mut guard = self.common_container_open.lock().await;
                let record = &mut guard.as_mut().expect("retained").record;
                if !record.observe_target_state(
                    &hit.state,
                    query.world_revision,
                    query.initial.receive_sequence,
                ) {
                    record.inspection("storage target properties changed");
                    return Ok(());
                }
                query.initial
            }
            Ok(_) => {
                self.common_container_open
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .record
                    .inspection("storage first-outline geometry changed");
                return Ok(());
            }
            Err(e) => {
                self.common_container_open
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .record
                    .inspection(format!("storage native context unavailable: {e}"));
                return Ok(());
            }
        };
        let screen = self
            .common_receipts
            .lock()
            .await
            .container
            .as_ref()
            .map(|s| s.capture(current.session));
        let selected = current.selected_hotbar.as_ref().map(|s| s.value);
        let context = contract::open::OpenContext {
            session: current.session,
            mode: current.game_mode,
            position: current.position.as_ref().map(|p| p.value),
            rotation: current.rotation,
            pose: current.received_pose.as_ref(),
            selected,
            hand: selected
                .filter(|s| *s <= 8)
                .and_then(|s| current.inventory.slots[36 + usize::from(s)].as_ref()),
            offhand: current.inventory.slots[45].as_ref(),
            cursor: current.inventory.cursor.as_ref(),
            player_screen: current.inventory.player_screen,
            screen: screen.as_ref(),
        };
        {
            let mut guard = self.common_container_open.lock().await;
            guard
                .as_mut()
                .expect("retained")
                .record
                .context_received(context);
        }
        if self
            .common_container_open
            .lock()
            .await
            .as_ref()
            .is_some_and(|o| o.record.stage == contract::ContainerOpenStage::ObservedContents)
        {
            let outcome = self
                .connection
                .finish_container_open(before.id.attempt())
                .await;
            let mut guard = self.common_container_open.lock().await;
            let owner = guard.as_mut().expect("retained");
            match outcome {
                Ok(()) => owner.released = true,
                Err(e) => owner
                    .record
                    .inspection(format!("storage owner release unavailable: {e:?}")),
            }
        }
        Ok(())
    }
    async fn close_native_basis(&self, window: i8, player: &api::PlayerObservation) -> Result<()> {
        let inventory = self.inventory.read().await;
        if inventory.open_window.as_ref().map(|w| w.id) != Some(window)
            || !inventory.pending_clicks.is_empty()
            || player.inventory.cursor.as_ref().is_none_or(|c| {
                api::legacy_slot(inventory.cursor.as_ref()).ok().as_ref() != Some(&c.value)
            })
        {
            return Err(api::inventory::unavailable(
                "legacy close actual/cache UI/cursor basis unresolved",
            ));
        }
        Ok(())
    }
    async fn close_current(
        &self,
        id: contract::ContainerCloseId,
    ) -> Result<(api::PlayerObservation, contract::ContainerScreen)> {
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            return Err(api::inventory::unavailable(
                "close source connection unavailable",
            ));
        }
        let current = self.common_player_unlocked().await?;
        let screen = self
            .common_receipts
            .lock()
            .await
            .container
            .as_ref()
            .map(|s| s.capture(current.session));
        let mut guard = self.common_container_close.lock().await;
        let record = guard
            .as_mut()
            .filter(|r| r.id == id)
            .ok_or_else(|| api::inventory::unavailable("close intent superseded"))?;
        let registries = self
            .common_receipts
            .lock()
            .await
            .registries
            .capture(current.session, current.receive_sequence);
        record.return_received_with_registries(&current, screen.as_ref(), &registries);
        if record.requires_inspection.is_some() {
            return Err(api::inventory::unavailable(
                "close context changed; retained without replay",
            ));
        }
        Ok((
            current,
            screen
                .ok_or_else(|| api::inventory::unavailable("close original opening unavailable"))?,
        ))
    }
    async fn return_and_close_owned(
        &self,
        id: contract::ContainerCloseId,
        revision: u64,
        window: i8,
    ) -> Result<contract::ContainerCloseRecord> {
        let count;
        {
            let _gate = self.coherent_state_gate.lock().await;
            let (current, _) = self.close_current(id).await?;
            self.close_native_basis(window, &current).await?;
            count = self
                .common_container_close
                .lock()
                .await
                .as_ref()
                .expect("retained")
                .return_plan
                .len() as u16;
            self.connection
                .begin_cursor_close(id.attempt(), revision, window, count)
                .await
                .map_err(|e| {
                    api::inventory::unavailable(format!("close parent reservation: {e:?}"))
                })?;
        }
        for number in 0..count {
            {
                let _gate = self.coherent_state_gate.lock().await;
                let (current, screen) = self.close_current(id).await?;
                self.close_native_basis(window, &current).await?;
                let registries = self
                    .common_receipts
                    .lock()
                    .await
                    .registries
                    .capture(current.session, current.receive_sequence);
                self.common_container_close
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .begin_return_step_received(current, screen, registries)?;
                let action = self
                    .connection
                    .reserve_cursor_return(id.attempt(), number)
                    .await
                    .map_err(|e| {
                        api::inventory::unavailable(format!("return reservation: {e:?}"))
                    })?;
                let (slot, native) = {
                    let mut guard = self.common_container_close.lock().await;
                    let step = guard
                        .as_mut()
                        .expect("retained")
                        .return_steps
                        .last_mut()
                        .expect("prepared");
                    let comparison = if step.source_before.value == api::SlotKnowledge::Empty {
                        step.cursor_before.value.clone()
                    } else {
                        api::SlotKnowledge::Empty
                    };
                    let native = match &comparison {
                        api::SlotKnowledge::Empty => None,
                        api::SlotKnowledge::Item { item } => Some(ItemStack {
                            item_id: item.id.value(),
                            count: item.count as i8,
                            nbt: match &item.data {
                                api::ItemData::Default => None,
                                api::ItemData::LegacyNbt { bytes } => Some(bytes.clone()),
                                api::ItemData::ModernComponents { .. } => unreachable!(
                                    "legacy received stack cannot contain modern components"
                                ),
                            },
                        }),
                        _ => unreachable!("validated actual cursor"),
                    };
                    step.send.legacy_action = Some(action);
                    step.send.legacy_comparison = Some(comparison);
                    step.send.after_sequence =
                        self.protocol_packet_sequence.load(Ordering::Acquire);
                    (step.source_slot, native)
                };
                self.connection
                    .bounded_cursor_return(id.attempt(), number, slot, native)
                    .await?;
                self.common_container_close
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .return_steps
                    .last_mut()
                    .expect("prepared")
                    .send
                    .dispatched = true;
            }
            tokio::time::timeout(contract::close::RECEIPT_TIMEOUT, async {
                loop {
                    {
                        let _gate = self.coherent_state_gate.lock().await;
                        let (current, _) = self.close_current(id).await?;
                        self.close_native_basis(window, &current).await?;
                        let ready = self
                            .common_container_close
                            .lock()
                            .await
                            .as_ref()
                            .expect("retained")
                            .return_steps
                            .last()
                            .is_some_and(|s| {
                                s.stage == api::inventory::InventoryClickStage::ObservedClicked
                            });
                        if ready {
                            self.connection
                                .finish_cursor_return(id.attempt(), number)
                                .await
                                .map_err(|e| {
                                    api::inventory::unavailable(format!(
                                        "return step release: {e:?}"
                                    ))
                                })?;
                            return Ok::<(), crate::Error>(());
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| {
                api::inventory::unavailable(
                    "return actual receipt deadline elapsed; inspect without replay",
                )
            })??;
        }
        let _gate = self.coherent_state_gate.lock().await;
        let (current, _) = self.close_current(id).await?;
        self.close_native_basis(window, &current).await?;
        if !self
            .common_container_close
            .lock()
            .await
            .as_ref()
            .expect("retained")
            .return_complete()
            || !matches!(
                current.inventory.cursor.as_ref(),
                Some(api::ObservedValue {
                    value: api::SlotKnowledge::Empty,
                    source: api::ValueSource::Received { .. }
                })
            )
        {
            return Err(api::inventory::unavailable(
                "close requires all actual return steps and actual Empty cursor",
            ));
        }
        self.connection.bounded_cursor_close(id.attempt()).await?;
        self.common_container_close
            .lock()
            .await
            .as_mut()
            .expect("retained")
            .sent();
        self.clear_local_window_unlocked(window).await;
        self.connection
            .finish_cursor_close(id.attempt())
            .await
            .map_err(|e| api::inventory::unavailable(format!("close parent release: {e:?}")))?;
        Ok(self
            .common_container_close
            .lock()
            .await
            .as_ref()
            .expect("retained")
            .clone())
    }
    pub(super) async fn common_container_return_reply(&self, reply: WindowTransaction) {
        if let Some(record) = self.common_container_close.lock().await.as_mut() {
            record.return_reply(api::inventory::InventoryTransactionReply {
                window_id: reply.window_id,
                action: reply.action,
                accepted: reply.accepted,
                receive_sequence: reply.packet_sequence,
            });
        }
    }
    pub(super) async fn common_container_close_received(&self, window: i32, sequence: u64) {
        let receipts = self.common_receipts.lock().await;
        let Some(s) = receipts.container.as_ref().filter(|s| s.window == window) else {
            return;
        };
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_id(),
            world_generation: receipts.generation,
        };
        if let Some(r) = self.common_container_close.lock().await.as_mut() {
            r.received_close(s.capture(session).id, sequence);
        }
    }
    pub(super) async fn common_container_close_context_received(&self) -> Result<()> {
        if !self
            .common_container_close
            .lock()
            .await
            .as_ref()
            .is_some_and(|r| {
                matches!(
                    r.stage,
                    contract::ContainerCloseStage::Pending
                        | contract::ContainerCloseStage::ReturningCursor
                )
            })
        {
            return Ok(());
        }
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        let screen = receipts
            .container
            .as_ref()
            .map(|s| s.capture(player.session));
        if let Some(r) = self.common_container_close.lock().await.as_mut() {
            let registries = receipts
                .registries
                .capture(player.session, player.receive_sequence);
            r.return_received_with_registries(&player, screen.as_ref(), &registries);
        }
        Ok(())
    }
}

impl crate::client::adapter::ContainerOps for Bot {
    async fn open_container(
        &self,
        mode: api::GameMode,
        target: [i32; 3],
    ) -> Result<contract::ContainerOpenRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| api::inventory::unavailable(format!("storage open admission: {e:?}")))?;
        self.open_native_inventory_admission().await?;
        let query = self.common_target_in_mode(4.5, None, mode).await?;
        let before = self
            .common_receipts
            .lock()
            .await
            .container
            .as_ref()
            .map(|s| s.capture(query.initial.session).id);
        let record = contract::open::prepare_open(
            query,
            target,
            mode,
            before,
            self.common_container_open
                .lock()
                .await
                .as_ref()
                .map(|o| &o.record),
            None,
        )?;
        let id = record.id;
        *self.common_container_open.lock().await = Some(NativeContainerOpen {
            record,
            released: false,
        });
        let bot = self.clone_internal();
        let (reply, result) = oneshot::channel();
        tokio::spawn(async move {
            let _gate = bot.coherent_state_gate.lock().await;
            let outcome = async {
                bot.common_container_open_context_received().await?;
                let record = bot
                    .common_container_open
                    .lock()
                    .await
                    .as_ref()
                    .filter(|o| o.record.id == id)
                    .map(|o| o.record.clone())
                    .ok_or_else(|| api::inventory::unavailable("storage open intent superseded"))?;
                if record.requires_inspection.is_some() {
                    return Err(api::inventory::unavailable(
                        "storage open context changed before I/O",
                    ));
                }
                bot.open_native_inventory_admission().await?;
                bot.common_container_open
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .record
                    .send
                    .after_sequence = bot.protocol_packet_sequence.load(Ordering::Acquire);
                bot.connection
                    .bounded_container_open(
                        id.attempt(),
                        revision,
                        BlockPos {
                            x: record.target.position[0],
                            y: record.target.position[1],
                            z: record.target.position[2],
                        },
                        record.target.face as u8,
                        record.cursor,
                    )
                    .await
            }
            .await;
            let mut guard = bot.common_container_open.lock().await;
            let record = &mut guard.as_mut().expect("retained").record;
            let value = match outcome {
                Ok(()) => {
                    record.sent();
                    Ok(record.clone())
                }
                Err(e) => {
                    record.inspection(&e);
                    Err(e)
                }
            };
            let _ = reply.send(value);
        });
        drop(gate);
        result
            .await
            .map_err(|_| api::inventory::unavailable("storage open owner result unavailable"))?
    }
    /// Inspect the retained record without queuing behind a stalled owned write.
    async fn container_open_record(&self) -> Result<Option<contract::ContainerOpenRecord>> {
        let mut guard = self.common_container_open.lock().await;
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            if let Some(o) = guard.as_mut().filter(|o| !o.released) {
                o.record
                    .inspection("storage open connection closed or uncertain");
            }
        }
        Ok(guard.as_ref().map(|o| o.record.clone()))
    }
    async fn close_container(
        &self,
        mode: api::GameMode,
        screen: contract::ScreenId,
    ) -> Result<contract::ContainerCloseRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| api::inventory::unavailable(format!("close admission: {e:?}")))?;
        let initial = self.common_player_unlocked().await?;
        let captured = self
            .common_receipts
            .lock()
            .await
            .container
            .as_ref()
            .map(|s| s.capture(initial.session))
            .ok_or_else(|| api::inventory::unavailable("no received container opening"))?;
        let window = i8::try_from(screen.window_id())
            .ok()
            .filter(|w| *w > 0)
            .ok_or_else(|| api::inventory::unavailable("native legacy window unavailable"))?;
        self.close_native_basis(window, &initial).await?;
        let registries = self
            .common_receipts
            .lock()
            .await
            .registries
            .capture(initial.session, initial.receive_sequence);
        let record = contract::prepare_close_received(
            (initial, registries),
            captured,
            screen,
            mode,
            self.common_container_close.lock().await.as_ref(),
        )?;
        let id = record.id;
        *self.common_container_close.lock().await = Some(record);
        let bot = self.clone_internal();
        let (reply, result) = oneshot::channel();
        tokio::spawn(async move {
            let outcome = bot.return_and_close_owned(id, revision, window).await;
            if let Err(e) = &outcome {
                if let Some(record) = bot
                    .common_container_close
                    .lock()
                    .await
                    .as_mut()
                    .filter(|r| r.id == id)
                {
                    record.inspection(e);
                }
            }
            let _ = reply.send(outcome);
        });
        drop(gate);
        result
            .await
            .map_err(|_| api::inventory::unavailable("close owner result unavailable"))?
    }
    async fn container_close_record(&self) -> Result<Option<contract::ContainerCloseRecord>> {
        if let Ok(_gate) = self.coherent_state_gate.try_lock() {
            self.common_container_close_context_received().await?;
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            if let Some(r) = self.common_container_close.lock().await.as_mut() {
                r.inspection("close connection closed or uncertain");
            }
        }
        Ok(self.common_container_close.lock().await.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};
    async fn seed_open(bot: &Bot) {
        super::super::common_motion::tests::seed_motion(bot).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".into());
        let mut full = vec![0, 0, 46];
        full.extend([0; 46]);
        bot.apply_packet(0x14, full).await.unwrap();
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        bot.apply_packet(0x3f, vec![0]).await.unwrap();
        let chest = crate::NativeBlockState {
            name: "minecraft:chest".into(),
            properties: std::collections::BTreeMap::from([
                ("facing".into(), "north".into()),
                ("type".into(), "single".into()),
                ("waterlogged".into(), "false".into()),
            ]),
        };
        bot.world.lock().await.set_block_for_test(
            BlockPos { x: 8, y: 66, z: 11 },
            crate::versions::java_1_16_1::state_id(&chest).unwrap(),
        );
    }
    async fn received_open(bot: &Bot) {
        let mut p = vec![3, 2];
        put_string(&mut p, "{}");
        bot.apply_packet(0x2e, p).await.unwrap();
    }
    async fn contents(bot: &Bot) {
        let mut full = vec![3, 0, 63];
        full.extend([0; 63]);
        bot.apply_packet(0x14, full).await.unwrap();
    }
    #[tokio::test]
    async fn common_container_open_same_consumer_in_both_modes_needs_actual_contents_and_cursor() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_open(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let mut old = None;
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            if mode == api::GameMode::Creative {
                let mut p = vec![3];
                p.extend(1f32.to_be_bytes());
                bot.apply_packet(0x1e, p).await.unwrap();
            }
            let record =
                crate::client::tests::common_open_start_scenario(&client, mode, [8, 66, 11]).await;
            assert_eq!(record.before_screen, old);
            let mut expected = vec![0];
            expected.extend(BlockPos { x: 8, y: 66, z: 11 }.packed().to_be_bytes());
            expected.push(2);
            for cursor in record.cursor {
                expected.extend(cursor.to_be_bytes());
            }
            expected.push(0);
            assert_eq!(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                (0x2d, expected)
            );
            let pose = record.initial.received_pose.as_ref().unwrap();
            let mut refresh = Vec::new();
            for v in pose.position {
                refresh.extend(v.to_be_bytes());
            }
            for v in pose.rotation {
                refresh.extend(v.to_be_bytes());
            }
            refresh.extend([0, 77]);
            bot.apply_packet(0x35, refresh).await.unwrap();
            assert_eq!(packets.recv().await.unwrap(), (0x00, vec![77]));
            assert_eq!(packets.recv().await.unwrap().0, 0x13);
            let refreshed = client
                .survival()
                .container_open_record()
                .await
                .unwrap()
                .unwrap();
            assert!(refreshed.requires_inspection.is_none(), "{refreshed:?}");
            assert_eq!(
                refreshed.initial.received_pose,
                record.initial.received_pose
            );
            assert!(
                client
                    .player_state()
                    .await
                    .unwrap()
                    .received_pose
                    .unwrap()
                    .receive_sequence
                    > pose.receive_sequence
            );
            received_open(&bot).await;
            contents(&bot).await;
            let pending = client
                .survival()
                .container_open_record()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(pending.stage, contract::ContainerOpenStage::ObservedScreen);
            assert!(pending.received_cursor.is_none());
            assert!(client.player_state().await.unwrap().pending_dispatch);
            bot.apply_packet(0x16, vec![255, 255, 255, 0])
                .await
                .unwrap();
            let complete =
                crate::client::tests::common_open_completed_scenario(&client, record.id).await;
            let screen = complete.observed_screen.unwrap().id;
            assert_ne!(Some(screen), old);
            let close =
                crate::client::tests::common_container_close_scenario(&client, mode, screen).await;
            assert_eq!(packets.recv().await.unwrap(), (0x0a, vec![3]));
            crate::client::tests::common_closed_player_screen_scenario(&client, close.id).await;
            old = Some(screen);
        }
        bot.disconnect().await.unwrap();
        let history = client
            .creative()
            .container_open_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            history.stage,
            contract::ContainerOpenStage::ObservedContents
        );
        assert_eq!(history.observed_screen.unwrap().id, old.unwrap());
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_open_reused_numeric_window_cannot_complete_original_attempt() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_open(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let record = client.survival().open_container([8, 66, 11]).await.unwrap();
        packets.recv().await.unwrap();
        received_open(&bot).await;
        let first = client.screen_state().await.unwrap().screen.unwrap().id;
        received_open(&bot).await;
        contents(&bot).await;
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        let history = client
            .survival()
            .container_open_record()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.id, record.id);
        assert_eq!(
            history.stage,
            contract::ContainerOpenStage::RequiresInspection
        );
        assert_eq!(history.observed_screen.unwrap().id, first);
        assert!(client.survival().open_container([8, 66, 11]).await.is_err());
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_open_cancelled_legacy_waiter_preserves_owned_write_and_prompt_history()
     {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_open(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter = tokio::spawn(async move { ops.open_container([8, 66, 11]).await });
        timeout(Duration::from_secs(1), async {
            while bot.common_container_open.lock().await.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        let pending = timeout(
            Duration::from_millis(100),
            client.survival().container_open_record(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(!pending.send.dispatched);
        assert_eq!(pending.stage, contract::ContainerOpenStage::Pending);
        drop(writer);
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap()
                .0,
            0x2d
        );
        timeout(Duration::from_secs(1), async {
            while !client
                .survival()
                .container_open_record()
                .await
                .unwrap()
                .unwrap()
                .send
                .dispatched
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        received_open(&bot).await;
        contents(&bot).await;
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        crate::client::tests::common_open_completed_scenario(&client, pending.id).await;
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        bot.disconnect().await.unwrap();
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_open_mode_conflict_latches_even_after_matching_restoration() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed_open(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().open_container([8, 66, 11]).await.unwrap();
        packets.recv().await.unwrap();
        for mode in [1f32, 0f32] {
            let mut p = vec![3];
            p.extend(mode.to_be_bytes());
            bot.apply_packet(0x1e, p).await.unwrap();
        }
        received_open(&bot).await;
        contents(&bot).await;
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        assert_eq!(
            client
                .survival()
                .container_open_record()
                .await
                .unwrap()
                .unwrap()
                .stage,
            contract::ContainerOpenStage::RequiresInspection
        );
        assert!(client.player_state().await.unwrap().pending_dispatch);
        bot.disconnect().await.unwrap();
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_open_barrel_native_flag_change_is_distinct_from_facing_conflict() {
        for conflict in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed_open(&bot).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            let mut barrel = crate::NativeBlockState {
                name: "minecraft:barrel".into(),
                properties: std::collections::BTreeMap::from([
                    ("facing".into(), "north".into()),
                    ("open".into(), "false".into()),
                ]),
            };
            let packet = |state: &crate::NativeBlockState| {
                let mut p = BlockPos { x: 8, y: 66, z: 11 }
                    .packed()
                    .to_be_bytes()
                    .to_vec();
                put_varint(
                    &mut p,
                    crate::versions::java_1_16_1::state_id(state).unwrap(),
                );
                p
            };
            bot.apply_packet(0x0b, packet(&barrel)).await.unwrap();
            let record = client.survival().open_container([8, 66, 11]).await.unwrap();
            packets.recv().await.unwrap();
            barrel.properties.insert(
                if conflict { "facing" } else { "open" }.into(),
                if conflict { "south" } else { "true" }.into(),
            );
            bot.apply_packet(0x0b, packet(&barrel)).await.unwrap();
            if conflict {
                barrel.properties.insert("facing".into(), "north".into());
                bot.apply_packet(0x0b, packet(&barrel)).await.unwrap();
            }
            received_open(&bot).await;
            contents(&bot).await;
            bot.apply_packet(0x16, vec![255, 255, 255, 0])
                .await
                .unwrap();
            let history = client
                .survival()
                .container_open_record()
                .await
                .unwrap()
                .unwrap();
            if conflict {
                assert_eq!(
                    history.stage,
                    contract::ContainerOpenStage::RequiresInspection
                );
                assert_eq!(
                    history.target_state.unwrap().state.properties["facing"],
                    "south"
                );
            } else {
                crate::client::tests::common_open_completed_scenario(&client, record.id).await;
                assert_eq!(
                    history.target_state.unwrap().state.properties["open"],
                    "true"
                );
                assert_eq!(record.target.state.properties["open"], "false");
            }
            bot.disconnect().await.unwrap();
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
}
