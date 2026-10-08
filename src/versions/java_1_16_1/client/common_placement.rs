//! Legacy one-shot placement, retained through cancellation and exact receipts.
use super::common_motion::CommonOwner;
use super::*;
use crate::client::{
    ObservedValue, SlotKnowledge, ValueSource,
    survival::{PlacementId, PlacementRecord, PlacementSend, PlacementStage, placement},
};
#[derive(Clone)]
pub(super) struct NativePlacementRun {
    pub(super) record: PlacementRecord,
    pub(super) released: bool,
    movement_revision: u64,
    movement_attribute: Option<Attribute>,
}
fn inspection(run: &mut NativePlacementRun, reason: impl std::fmt::Display) {
    if run.record.stage != PlacementStage::ObservedPlaced {
        run.record
            .requires_inspection
            .get_or_insert_with(|| reason.to_string());
        run.record.stage = PlacementStage::RequiresInspection;
    }
}
impl Bot {
    async fn placement_prepared(
        &self,
        support: [i32; 3],
        face: crate::BlockFace,
        owner: Option<PlacementId>,
    ) -> Result<PlacementRecord> {
        let query = self
            .common_target_with_owner(4.5, owner.map(CommonOwner::Placement))
            .await?;
        let hit = query
            .hit
            .ok_or_else(|| placement::unavailable("no first native outline for placement"))?;
        if hit.position != support
            || hit.face != face
            || !crate::client::survival::model::DRY_CUBES.contains(&hit.state.name.as_str())
        {
            return Err(placement::unavailable(
                "placement support/face must be the first admitted native outline",
            ));
        }
        let (held_before, held_receive_sequence) = placement::selected_material(&query.initial)?;
        {
            let inventory = self.inventory.read().await;
            if inventory.cursor.is_some()
                || inventory.open_window.is_some()
                || !inventory.pending_clicks.is_empty()
            {
                return Err(placement::unavailable(
                    "placement refuses unresolved legacy inventory operations",
                ));
            }
        }
        let target = placement::adjacent(support, face)?;
        if !(0..=255).contains(&target[1])
            || target[0].abs_diff(0) > 30_000_000
            || target[2].abs_diff(0) > 30_000_000
        {
            return Err(placement::unavailable(
                "placement target outside legacy world bounds",
            ));
        }
        let before = crate::versions::java_1_16_1::native_state(
            self.world
                .lock()
                .await
                .block(target[0], target[1], target[2])
                .ok_or_else(|| placement::unavailable("placement destination is unloaded"))?,
        )?;
        let position = query
            .initial
            .position
            .as_ref()
            .ok_or_else(|| placement::unavailable("placement feet unavailable"))?
            .value;
        let body = crate::client::survival::model::body(position);
        if !placement::air(&before)
            || (0..3)
                .all(|i| body[i] < f64::from(target[i] + 1) && body[i + 3] > f64::from(target[i]))
        {
            return Err(placement::unavailable(
                "placement needs received air outside the standing body",
            ));
        }
        let expected = crate::NativeBlockState {
            name: held_before.name.clone(),
            properties: Default::default(),
        };
        crate::versions::java_1_16_1::state_id(&expected)?;
        let cursor =
            std::array::from_fn(|i| (hit.point[i] - f64::from(support[i])).clamp(0.0, 1.0) as f32);
        Ok(PlacementRecord {
            id: PlacementId::new(query.initial.session, 0),
            send: PlacementSend {
                after_sequence: query.initial.receive_sequence,
                interaction_sequence: None,
                dispatched: false,
            },
            initial: query.initial,
            support,
            support_state: hit.state,
            face,
            cursor,
            target,
            before,
            expected,
            held_before,
            held_receive_sequence,
            target_receipt: None,
            material_receipt: None,
            processing: None,
            requires_inspection: None,
            stage: PlacementStage::Pending,
        })
    }
    async fn placement_send_owned(
        &self,
        id: PlacementId,
        revision: u64,
    ) -> Result<PlacementRecord> {
        let _gate = self.coherent_state_gate.lock().await;
        let result = async {
            let run = super::lock_packet_state(&self.common_placement)
                .await
                .clone()
                .filter(|p| p.record.id == id)
                .ok_or_else(|| placement::unavailable("placement intent superseded"))?;
            if run.record.requires_inspection.is_some() {
                return Err(placement::unavailable(
                    "placement context interrupted before I/O",
                ));
            }
            let current = self
                .placement_prepared(run.record.support, run.record.face, Some(id))
                .await?;
            let initial = &run.record.initial;
            if current.initial.session != initial.session
                || current.initial.position != initial.position
                || current.initial.received_pose != initial.received_pose
                || current.initial.rotation != initial.rotation
                || current.initial.selected_hotbar != initial.selected_hotbar
                || current.initial.inventory.player_screen != initial.inventory.player_screen
                || current.support_state != run.record.support_state
                || current.before != run.record.before
                || current.held_before != run.record.held_before
                || current.held_receive_sequence != run.record.held_receive_sequence
                || current.cursor != run.record.cursor
            {
                return Err(placement::unavailable(
                    "placement capture changed before I/O",
                ));
            }
            super::lock_packet_state(&self.common_placement)
                .await
                .as_mut()
                .expect("retained")
                .record
                .send
                .after_sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
            self.connection
                .bounded_placement(
                    id.attempt(),
                    revision,
                    BlockPos {
                        x: run.record.support[0],
                        y: run.record.support[1],
                        z: run.record.support[2],
                    },
                    run.record.face as u8,
                    run.record.cursor,
                )
                .await?;
            let mut guard = super::lock_packet_state(&self.common_placement).await;
            let record = &mut guard.as_mut().expect("retained").record;
            record.send.dispatched = true;
            Ok(record.clone())
        }
        .await;
        if let Err(error) = &result {
            self.interrupt_common_placement(error).await;
        }
        result
    }
    pub(super) async fn interrupt_common_placement(&self, reason: impl std::fmt::Display) {
        if let Some(p) = super::lock_packet_state(&self.common_placement)
            .await
            .as_mut()
        {
            inspection(p, reason);
        }
    }
    async fn reconcile_common_placement(&self, confirm: bool) -> Result<()> {
        let Some(snapshot) = super::lock_packet_state(&self.common_placement)
            .await
            .clone()
        else {
            return Ok(());
        };
        if snapshot.record.stage == PlacementStage::ObservedPlaced {
            return Ok(());
        }
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            self.interrupt_common_placement("placement connection closed or uncertain")
                .await;
            return Ok(());
        }
        let current = self.common_player_unlocked().await?;
        let initial = &snapshot.record.initial;
        if current.session != initial.session
            || current.position != initial.position
            || current.received_pose != initial.received_pose
            || current.rotation != initial.rotation
            || current.selected_hotbar != initial.selected_hotbar
            || self.motion.lock().await.revision() != snapshot.movement_revision
            || super::common_motion::legacy_movement_attribute(&**self.survival.read().await)
                != snapshot.movement_attribute.as_ref()
        {
            self.interrupt_common_placement("placement player/world/selection context changed")
                .await;
        }
        if let Err(error) = self
            .common_preview_with_owner(
                &[crate::client::survival::SurvivalControl {
                    yaw: current.rotation[0],
                    input: Default::default(),
                }],
                Some(CommonOwner::Placement(snapshot.record.id)),
            )
            .await
        {
            self.interrupt_common_placement(format!(
                "placement standing geometry changed: {error}"
            ))
            .await;
        }
        let selection = initial
            .selected_hotbar
            .as_ref()
            .expect("prepared selection")
            .value;
        let hand = current.inventory.slots[36 + usize::from(selection)].as_ref();
        let before = SlotKnowledge::Item {
            item: snapshot.record.held_before.clone(),
        };
        let after = placement::remaining(&snapshot.record.held_before);
        let valid_cursor = matches!(
            current.inventory.cursor.as_ref(),
            Some(ObservedValue {
                value: SlotKnowledge::Empty,
                source: ValueSource::Received { .. }
            })
        );
        let inventory = self.inventory.read().await;
        if current.inventory.player_screen != initial.inventory.player_screen
            || current.inventory.player_screen.is_none()
            || !valid_cursor
            || inventory.cursor.is_some()
            || inventory.open_window.is_some()
            || !inventory.pending_clicks.is_empty()
            || !matches!(hand,Some(ObservedValue{value,source:ValueSource::Received{..}}) if *value==before || *value==after)
        {
            self.interrupt_common_placement("placement received inventory context changed")
                .await;
        }
        drop(inventory);
        let mut guard = super::lock_packet_state(&self.common_placement).await;
        let run = guard.as_mut().expect("retained");
        if let Some(ObservedValue {
            value,
            source: ValueSource::Received { sequence },
        }) = hand
        {
            if *sequence > run.record.send.after_sequence {
                if *value == after {
                    run.record
                        .material_receipt
                        .get_or_insert_with(|| crate::client::received(value.clone(), *sequence));
                } else if run.record.material_receipt.is_some() {
                    inspection(run, "consumed material changed before target confirmation");
                }
            }
        }
        let mut complete = false;
        if run.record.requires_inspection.is_none() {
            let world = self.world.lock().await;
            let support = run.record.support;
            let target = run.record.target;
            let support_state = world
                .block(support[0], support[1], support[2])
                .map(crate::versions::java_1_16_1::native_state)
                .transpose()?;
            let target_state = world
                .block(target[0], target[1], target[2])
                .map(crate::versions::java_1_16_1::native_state)
                .transpose()?;
            if support_state.as_ref() != Some(&run.record.support_state)
                || target_state
                    .as_ref()
                    .is_none_or(|s| s != &run.record.before && s != &run.record.expected)
            {
                inspection(run, "placement loaded support/target context changed");
            } else {
                complete = confirm
                    && run.record.send.dispatched
                    && run.record.target_receipt.is_some()
                    && run.record.material_receipt.is_some()
                    && target_state.as_ref() == Some(&run.record.expected);
            }
        }
        drop(guard);
        if complete {
            // No gameplay packet. Exact placement owner is released once after both receipts.
            match self
                .connection
                .finish_bounded_placement(snapshot.record.id.attempt())
                .await
            {
                Ok(()) => {
                    let mut guard = super::lock_packet_state(&self.common_placement).await;
                    let p = guard.as_mut().expect("retained");
                    p.released = true;
                    p.record.stage = PlacementStage::ObservedPlaced;
                }
                Err(error) => {
                    self.interrupt_common_placement(format!(
                        "placement completion gate unavailable: {error:?}"
                    ))
                    .await
                }
            }
        }
        Ok(())
    }
    pub(super) async fn common_placement_context_received(&self) -> Result<()> {
        self.reconcile_common_placement(false).await
    }
    pub(super) async fn common_placement_chunk_changed(&self, chunk: [i32; 2]) {
        let mut guard = super::lock_packet_state(&self.common_placement).await;
        if let Some(run) = guard.as_mut().filter(|p| {
            [p.record.support, p.record.target]
                .iter()
                .any(|v| [v[0].div_euclid(16), v[2].div_euclid(16)] == chunk)
        }) {
            inspection(run, "placement target/support chunk unloaded or replaced");
        }
    }
    pub(super) async fn common_placement_block_received(
        &self,
        position: [i32; 3],
        state_id: i32,
        sequence: u64,
    ) -> Result<()> {
        let mut guard = super::lock_packet_state(&self.common_placement).await;
        let Some(run) = guard.as_mut().filter(|p| {
            p.record.stage != PlacementStage::ObservedPlaced
                && (position == p.record.target || position == p.record.support)
        }) else {
            return Ok(());
        };
        if sequence <= run.record.send.after_sequence {
            return Ok(());
        }
        let state = crate::versions::java_1_16_1::native_state(state_id)?;
        if self
            .world
            .lock()
            .await
            .block(position[0], position[1], position[2])
            != Some(state_id)
            || (position == run.record.support && state != run.record.support_state)
            || (position == run.record.target
                && state != run.record.before
                && state != run.record.expected)
        {
            inspection(run, "placement received support/target conflict");
        }
        if position == run.record.target {
            if run.record.target_receipt.is_some() && state != run.record.expected {
                inspection(run, "placed target changed before material confirmation");
            }
            if state == run.record.expected {
                run.record
                    .target_receipt
                    .get_or_insert_with(|| crate::client::received(state, sequence));
            }
        }
        Ok(())
    }
}

impl crate::client::adapter::PlacementOps for Bot {
    async fn place_cube(
        &self,
        support: [i32; 3],
        face: crate::BlockFace,
    ) -> Result<PlacementRecord> {
        let gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission().await?;
        let revision = self
            .connection
            .motion_admission_revision()
            .await
            .map_err(|e| placement::unavailable(format!("placement admission: {e:?}")))?;
        let mut record = self.placement_prepared(support, face, None).await?;
        let attempt = super::lock_packet_state(&self.common_placement)
            .await
            .as_ref()
            .map_or(Some(1), |p| p.record.id.attempt().checked_add(1))
            .ok_or_else(|| placement::unavailable("placement attempts exhausted"))?;
        record.id = PlacementId::new(record.initial.session, attempt);
        let id = record.id;
        *super::lock_packet_state(&self.common_placement).await = Some(NativePlacementRun {
            record,
            released: false,
            movement_revision: self.motion.lock().await.revision(),
            movement_attribute: super::common_motion::legacy_movement_attribute(
                &**self.survival.read().await,
            )
            .cloned(),
        });
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _ = reply.send(bot.placement_send_owned(id, revision).await);
        });
        drop(gate);
        result
            .await
            .map_err(|_| placement::unavailable("placement owner result missing"))?
    }
    async fn placement_record(&self) -> Result<Option<PlacementRecord>> {
        // The connection owns completion/release even if this read waiter is cancelled.
        let (reply, result) = tokio::sync::oneshot::channel();
        let bot = self.clone_internal();
        tokio::spawn(async move {
            let _gate = bot.coherent_state_gate.lock().await;
            let value = async {
                bot.reconcile_common_placement(true).await?;
                Ok(bot
                    .common_placement
                    .lock()
                    .await
                    .as_ref()
                    .map(|p| p.record.clone()))
            }
            .await;
            let _ = reply.send(value);
        });
        result
            .await
            .map_err(|_| placement::unavailable("placement inspection owner unavailable"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SUPPORT: [i32; 3] = [10, 66, 8];
    const TARGET: [i32; 3] = [9, 66, 8];
    async fn seed(bot: &Bot) {
        super::super::common_motion::tests::seed_motion(bot).await;
        bot.survival.write().await.dimension = Some("minecraft:overworld".into());
        {
            let mut p = bot.player.lock().await;
            p.yaw = -90.0;
            p.pitch = 3.0;
        }
        let mut slots = vec![0, 0, 46];
        for slot in 0..46 {
            if slot == 36 {
                write_slot(
                    &mut slots,
                    Some(&ItemStack {
                        item_id: crate::item_id("dirt").unwrap(),
                        count: 5,
                        nbt: None,
                    }),
                );
            } else {
                slots.push(0);
            }
        }
        bot.apply_packet(0x14, slots).await.unwrap();
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        bot.world.lock().await.set_block_for_test(
            BlockPos {
                x: SUPPORT[0],
                y: SUPPORT[1],
                z: SUPPORT[2],
            },
            1,
        );
    }
    async fn change(bot: &Bot, p: [i32; 3], state: i32) {
        let mut payload = BlockPos {
            x: p[0],
            y: p[1],
            z: p[2],
        }
        .packed()
        .to_be_bytes()
        .to_vec();
        put_varint(&mut payload, state);
        bot.apply_packet(0x0b, payload).await.unwrap();
    }
    async fn stack(bot: &Bot, count: i8) {
        let mut payload = vec![0, 0, 36];
        write_slot(
            &mut payload,
            (count > 0).then_some(&ItemStack {
                item_id: crate::item_id("dirt").unwrap(),
                count,
                nbt: None,
            }),
        );
        bot.apply_packet(0x16, payload).await.unwrap();
    }
    #[tokio::test]
    async fn common_placement_same_consumer_requires_fresh_target_and_material_and_releases_exact_actor()
     {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = crate::client::tests::common_placement_start_scenario(
            &client,
            SUPPORT,
            crate::BlockFace::West,
            TARGET,
        )
        .await;
        assert!(record.send.interaction_sequence.is_none() && record.processing.is_none());
        let (id, p) = packets.recv().await.unwrap();
        assert_eq!(id, 0x2d);
        assert_eq!(p.len(), 23);
        assert_eq!(p[0], 0);
        assert_eq!(p[9], 4);
        assert_eq!(p[22], 0);
        assert_eq!(&p[10..14], &record.cursor[0].to_be_bytes());
        assert_eq!(&p[14..18], &record.cursor[1].to_be_bytes());
        assert_eq!(&p[18..22], &record.cursor[2].to_be_bytes());
        assert!(
            bot.connection
                .finish_bounded_motion(record.id.attempt())
                .await
                .is_err()
        );
        change(
            &bot,
            TARGET,
            crate::versions::java_1_16_1::state_id(&record.expected).unwrap(),
        )
        .await;
        crate::client::tests::common_placement_pending_scenario(&client).await;
        stack(&bot, 4).await;
        let completed =
            crate::client::tests::common_placement_completed_scenario(&client, record.id).await;
        packets.recv().await.unwrap();
        assert!(completed.processing.is_none());
        change(&bot, TARGET, 0).await;
        let historical = client.survival().placement_record().await.unwrap().unwrap();
        assert_eq!(historical.stage, PlacementStage::ObservedPlaced);
        assert_eq!(
            historical.target_receipt.unwrap().source,
            completed.target_receipt.unwrap().source
        );
        // A new site is revalidated on the same connection; no one-shot limitation.
        change(&bot, [6, 66, 8], 1).await;
        client.survival().look([90.0, 3.0]).await.unwrap();
        packets.recv().await.unwrap();
        let next = client
            .survival()
            .place_cube([6, 66, 8], crate::BlockFace::East)
            .await
            .unwrap();
        assert_ne!(next.id, record.id);
        assert_eq!(next.held_before.count, 4);
        assert_eq!(packets.recv().await.unwrap().0, 0x2d);
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_placement_after_no_echo_close_keeps_opening_history_and_reopen_conflicts() {
        for reopen in [false, true] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            seed(&bot).await;
            let mut open = vec![3, 2];
            put_string(&mut open, "{}");
            bot.apply_packet(0x2e, open.clone()).await.unwrap();
            let mut full = vec![3, 0, 63];
            for slot in 0..63 {
                write_slot(
                    &mut full,
                    (slot == 54).then_some(&ItemStack {
                        item_id: crate::item_id("dirt").unwrap(),
                        count: 5,
                        nbt: None,
                    }),
                );
            }
            bot.apply_packet(0x14, full).await.unwrap();
            bot.apply_packet(0x16, vec![255, 255, 255, 0])
                .await
                .unwrap();
            let client = crate::Client::from_java_1_16_1(bot.clone());
            client.survival().select_hotbar(0).await.unwrap();
            packets.recv().await.unwrap();
            let screen = client.screen_state().await.unwrap().screen.unwrap().id;
            assert!(
                client
                    .survival()
                    .place_cube(SUPPORT, crate::BlockFace::West)
                    .await
                    .is_err()
            );
            assert!(packets.try_recv().is_err());
            let close = client.survival().close_container(screen).await.unwrap();
            assert_eq!(packets.recv().await.unwrap(), (0x0a, vec![3]));
            crate::client::tests::common_closed_player_screen_scenario(&client, close.id).await;
            let record = client
                .survival()
                .place_cube(SUPPORT, crate::BlockFace::West)
                .await
                .unwrap();
            assert_eq!(packets.recv().await.unwrap().0, 0x2d);
            assert_eq!(record.initial.inventory.window_id, Some(3));
            assert_eq!(
                record.initial.inventory.player_screen,
                Some(
                    crate::client::container::PlayerScreenAccess::SubmittedClose {
                        close: close.id
                    }
                )
            );
            if reopen {
                bot.apply_packet(0x2e, open).await.unwrap();
                // Reusing the native window ID does not restore the old close basis.
                assert!(
                    client
                        .player_state()
                        .await
                        .unwrap()
                        .inventory
                        .player_screen
                        .is_none()
                );
            }
            change(
                &bot,
                TARGET,
                crate::versions::java_1_16_1::state_id(&record.expected).unwrap(),
            )
            .await;
            stack(&bot, 4).await;
            let final_record = client.survival().placement_record().await.unwrap().unwrap();
            assert_eq!(
                final_record.stage,
                if reopen {
                    PlacementStage::RequiresInspection
                } else {
                    PlacementStage::ObservedPlaced
                }
            );
            if reopen {
                assert!(client.survival().select_hotbar(1).await.is_err());
            }
            drop(release);
            drop(client);
            drop(bot);
            timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn common_placement_cached_target_and_material_cannot_confirm_and_transient_receipts_stay_conflicted()
     {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .place_cube(SUPPORT, crate::BlockFace::West)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        bot.world.lock().await.set_block_for_test(
            BlockPos {
                x: TARGET[0],
                y: TARGET[1],
                z: TARGET[2],
            },
            crate::versions::java_1_16_1::state_id(&record.expected).unwrap(),
        );
        change(&bot, [12, 66, 12], 0).await;
        stack(&bot, 4).await;
        crate::client::tests::common_placement_pending_scenario(&client).await;
        stack(&bot, 5).await;
        stack(&bot, 4).await;
        change(
            &bot,
            TARGET,
            crate::versions::java_1_16_1::state_id(&record.expected).unwrap(),
        )
        .await;
        let conflict = client.survival().placement_record().await.unwrap().unwrap();
        assert_eq!(conflict.stage, PlacementStage::RequiresInspection);
        assert!(
            conflict
                .requires_inspection
                .unwrap()
                .contains("consumed material")
        );
        assert!(client.survival().select_hotbar(0).await.is_err());
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_placement_cancelled_waiter_keeps_single_owned_write() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let waiter =
            tokio::spawn(async move { ops.place_cube(SUPPORT, crate::BlockFace::West).await });
        timeout(Duration::from_secs(1), async {
            while bot.common_placement.lock().await.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !bot.common_placement
                .lock()
                .await
                .as_ref()
                .unwrap()
                .record
                .send
                .dispatched
        );
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(writer);
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap()
                .0,
            0x2d
        );
        let record = client.survival().placement_record().await.unwrap().unwrap();
        assert!(record.send.dispatched);
        assert!(
            client
                .survival()
                .place_cube(SUPPORT, crate::BlockFace::West)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn common_placement_support_loss_restoration_does_not_release_on_later_outcomes() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        seed(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        client.survival().select_hotbar(0).await.unwrap();
        packets.recv().await.unwrap();
        let record = client
            .survival()
            .place_cube(SUPPORT, crate::BlockFace::West)
            .await
            .unwrap();
        packets.recv().await.unwrap();
        change(&bot, [8, 64, 8], 0).await;
        change(&bot, [8, 64, 8], 1).await;
        stack(&bot, 4).await;
        change(
            &bot,
            TARGET,
            crate::versions::java_1_16_1::state_id(&record.expected).unwrap(),
        )
        .await;
        let conflict = client.survival().placement_record().await.unwrap().unwrap();
        assert_eq!(conflict.stage, PlacementStage::RequiresInspection);
        assert!(
            conflict
                .requires_inspection
                .unwrap()
                .contains("standing geometry")
        );
        drop(release);
        drop(client);
        drop(bot);
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
