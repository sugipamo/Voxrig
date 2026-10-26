//! Live receipt commits; the caller holds coherent_state_gate.
use super::*;
use crate::client::{
    self as api,
    received_snapshot::{InvalidationReason, Presence, Receipt, ReceivedCorrectionFields},
};
impl Bot {
    pub(super) async fn commit_received_snapshot(
        &self,
        id: i32,
        p: &[u8],
        sequence: u64,
    ) -> Result<()> {
        let own_id = lock_packet_state(&self.player).await.entity_id;
        let mut receipts = lock_packet_state(&self.common_receipts).await;
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_id(),
            world_generation: receipts.generation,
        };
        let event = Receipt::new(session, sequence);
        let registry = receipts.registries.capture(session, sequence);
        // Move the ledger out briefly to borrow only the exact applied native
        // constituents. The coherent gate prevents publication in between.
        let mut ledger = std::mem::take(&mut receipts.snapshot);
        let result = (|| -> Result<()> {
            let out = ledger.commit(
                event,
                registry,
                if id == 0x3a {
                    InvalidationReason::RespawnReset
                } else {
                    InvalidationReason::SessionBoundary
                },
            );
            if matches!(id, 0x25 | 0x3a) {
                out.no_opening(event);
            }
            out.health.apply(receipts.health.as_ref(), event);
            out.selected_hotbar
                .apply(receipts.selected_hotbar.as_ref(), event);
            out.abilities
                .apply(receipts.context.abilities.as_ref(), event);
            out.using_item.apply(receipts.using_item.as_ref(), event);
            out.air_supply.apply(receipts.air_supply.as_ref(), event);
            out.fall_flying.apply(receipts.fall_flying.as_ref(), event);
            out.world_time.apply(receipts.world_time.as_ref(), event);
            for (name, attribute) in &receipts.attributes {
                if attribute.source == (api::ValueSource::Received { sequence }) {
                    out.attribute_received(
                        name.clone(),
                        attribute.value.base,
                        attribute.value.modifiers.clone(),
                        event,
                    );
                }
            }
            for (name, effect) in &receipts.effects {
                if effect.source == (api::ValueSource::Received { sequence }) {
                    out.effect_received(name, Presence::Present(effect.value), event);
                }
            }
            if matches!(id, 0x25 | 0x3a) {
                if let Some(entry) = &receipts.context.world_entry {
                    out.entry(&entry.value, if id == 0x25 { own_id } else { None }, event)?;
                }
            }
            if id == 0x1e && p.first() == Some(&3) {
                out.game_mode.set(
                    api::GameMode::decode(f32::from_be_bytes([p[1], p[2], p[3], p[4]]) as u8)?,
                    event,
                );
            }
            if id == 0x35
                && receipts
                    .pose
                    .as_ref()
                    .is_some_and(|pose| pose.receive_sequence == sequence)
            {
                let mut c = Cursor::new(p);
                let position = [
                    c.read_f64::<BigEndian>()?,
                    c.read_f64::<BigEndian>()?,
                    c.read_f64::<BigEndian>()?,
                ];
                let rotation = [c.read_f32::<BigEndian>()?, c.read_f32::<BigEndian>()?];
                let flags = u32::from(c.read_u8()?);
                let teleport = get_varint(&mut &p[c.position() as usize..])?;
                out.correction_received(
                    ReceivedCorrectionFields::from_packet(
                        teleport, position, rotation, None, flags,
                    ),
                    event,
                );
            }
            if id == 0x38 {
                let mut rest = p;
                if Some(get_varint(&mut rest)?) == own_id {
                    if let Some(name) = rest.first().and_then(|id| {
                        api::player_facts::effect_name(session.version, i32::from(*id))
                    }) {
                        out.effect_received(&name, Presence::Absent, event);
                    }
                }
            }
            if id == 0x13 {
                out.close(i32::from(p[0] as i8), event);
            }
            if id == 0x14 && p.first() == Some(&0) && p.get(1..3) != Some(&46u16.to_be_bytes()) {
                out.inventory_unavailable(
                    receipts.inventory.slots.iter().map(Option::is_none),
                    false,
                    InvalidationReason::UnsupportedPayload,
                    event,
                );
            }
            out.inventory_received(&receipts.inventory, event)?;
            out.screen(receipts.screen(session), event)?;
            if id == 0x14 && p.first() == Some(&0) && p.get(1..3) == Some(&46u16.to_be_bytes()) {
                out.inventory.last_full_contents.set((), event);
            }
            Ok(())
        })();
        receipts.snapshot = ledger;
        result
    }
    pub(super) fn check_received_snapshot(&self) -> crate::Result<()> {
        if self.is_stopped()
            || !matches!(
                self.connection_state(),
                ConnectionState::Connecting | ConnectionState::Ready
            )
        {
            return Err(crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection is closing, closed or revoked"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ContainerStatus, Coverage, FieldReceipt, FieldState};
    fn received<T>(state: &FieldState<T>) -> &FieldReceipt<T> {
        match state {
            FieldState::Received(value) => value,
            _ => panic!("expected received"),
        }
    }
    fn join() -> Vec<u8> {
        let mut p = 42i32.to_be_bytes().to_vec();
        p.extend([0, 255, 0, 10, 0, 0, 0]);
        put_string(&mut p, "minecraft:overworld");
        put_string(&mut p, "world");
        p
    }
    fn respawn() -> Vec<u8> {
        let mut p = Vec::new();
        put_string(&mut p, "minecraft:overworld");
        put_string(&mut p, "world");
        p.extend(0i64.to_be_bytes());
        p.extend([0, 0, 0, 0, 1]);
        p
    }
    fn correction(flags: u8) -> Vec<u8> {
        let mut p = Vec::new();
        for value in [1f64, 64., 2.] {
            p.extend(value.to_be_bytes());
        }
        p.extend(30f32.to_be_bytes());
        p.extend(60f32.to_be_bytes());
        p.extend([flags, 1]);
        p
    }
    async fn finish(
        bot: Bot,
        release: tokio::sync::oneshot::Sender<()>,
        server: tokio::task::JoinHandle<()>,
    ) {
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn received_snapshot_container_missing_before_join() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        for _ in 0..2 {
            let snapshot = client.received_player_snapshot().await.unwrap();
            assert!(matches!(snapshot.container_state(), FieldState::Missing));
            assert!(snapshot.container().is_none());
        }
        bot.apply_packet(0x3f, vec![1]).await.unwrap();
        assert!(matches!(
            client
                .received_player_snapshot()
                .await
                .unwrap()
                .container_state(),
            FieldState::Missing
        ));
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_join_sets_no_opening() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let snapshot = client.received_player_snapshot().await.unwrap();
        let state = received(snapshot.container_state());
        assert_eq!(*state.value(), ContainerStatus::NoOpening);
        assert_eq!(state.receipt(), received(snapshot.game_mode()).receipt());
        assert_eq!(state.receipt().session(), snapshot.boundary().session());
        assert_eq!(state.receipt().ordinal(), snapshot.boundary().ordinal());
        assert!(snapshot.container().is_none());
        // Captures and unrelated receipts must preserve the original Join receipt.
        bot.apply_packet(0x3f, vec![1]).await.unwrap();
        let later = client.received_player_snapshot().await.unwrap();
        assert_eq!(received(later.container_state()).receipt(), state.receipt());
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_open_overrides_no_opening_then_server_close() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let joined = client.received_player_snapshot().await.unwrap();
        assert_eq!(
            *received(joined.container_state()).value(),
            ContainerStatus::NoOpening
        );
        let mut open = vec![7, 0];
        put_string(&mut open, "{}");
        bot.apply_packet(0x2e, open).await.unwrap();
        let opened = client.received_player_snapshot().await.unwrap();
        let id = opened.container().unwrap().id();
        assert_eq!(
            *received(opened.container_state()).value(),
            ContainerStatus::Open(id)
        );
        assert!(
            received(opened.container_state()).receipt().ordinal()
                > received(joined.container_state()).receipt().ordinal()
        );
        bot.apply_packet(0x13, vec![7]).await.unwrap();
        let closed = client.received_player_snapshot().await.unwrap();
        assert_eq!(
            *received(closed.container_state()).value(),
            ContainerStatus::Closed(id)
        );
        assert_eq!(
            received(closed.container_state()).receipt().ordinal(),
            closed.boundary().ordinal()
        );
        assert!(closed.container().is_none());
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_unmatched_close_keeps_no_opening() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let joined = client.received_player_snapshot().await.unwrap();
        let join_receipt = received(joined.game_mode()).receipt();
        bot.apply_packet(0x13, vec![7]).await.unwrap();
        let snapshot = client.received_player_snapshot().await.unwrap();
        let state = received(snapshot.container_state());
        assert_eq!(*state.value(), ContainerStatus::NoOpening);
        assert_eq!(state.receipt(), join_receipt);
        assert!(snapshot.container().is_none());
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_local_close_after_open_invalidates() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let mut open = vec![7, 0];
        put_string(&mut open, "{}");
        bot.apply_packet(0x2e, open).await.unwrap();
        let opened = client.received_player_snapshot().await.unwrap();
        assert_eq!(
            *received(opened.container_state()).value(),
            ContainerStatus::Open(opened.container().unwrap().id())
        );
        // Use the same local cache/receipt path as a dispatched close cleanup.
        bot.clear_local_window(7).await;
        let closed = client.received_player_snapshot().await.unwrap();
        let FieldState::Invalidated(event) = closed.container_state() else {
            panic!("local close must invalidate container state")
        };
        assert_eq!(event.reason(), InvalidationReason::ScreenChanged);
        assert_eq!(event.event_session(), opened.boundary().session());
        assert_eq!(event.event_ordinal(), None);
        assert_eq!(event.event_applied_at(), None);
        assert!(closed.container().is_none());
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_respawn_restores_no_opening() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let mut open = vec![7, 0];
        put_string(&mut open, "{}");
        bot.apply_packet(0x2e, open).await.unwrap();
        let opened = client.received_player_snapshot().await.unwrap();
        let open_receipt = received(opened.container_state()).receipt();
        assert!(opened.container().is_some());
        bot.apply_packet(0x3a, respawn()).await.unwrap();
        let reset = client.received_player_snapshot().await.unwrap();
        let state = received(reset.container_state());
        assert_eq!(*state.value(), ContainerStatus::NoOpening);
        assert_eq!(state.receipt(), received(reset.game_mode()).receipt());
        assert_eq!(state.receipt().session(), reset.boundary().session());
        assert_eq!(state.receipt().ordinal(), reset.boundary().ordinal());
        assert!(state.receipt().ordinal() > open_receipt.ordinal());
        assert!(reset.container().is_none());
        // The previous world's opening cannot turn a late close into Closed.
        bot.apply_packet(0x13, vec![7]).await.unwrap();
        let later = client.received_player_snapshot().await.unwrap();
        assert_eq!(
            *received(later.container_state()).value(),
            ContainerStatus::NoOpening
        );
        assert_eq!(received(later.container_state()).receipt(), state.receipt());
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_new_connection_starts_missing() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let old = client.received_player_snapshot().await.unwrap();
        let old_session = received(old.container_state()).receipt().session();
        drop(client);
        finish(bot, release, server).await;

        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let fresh = client.received_player_snapshot().await.unwrap();
        assert!(matches!(fresh.container_state(), FieldState::Missing));
        assert_ne!(
            fresh.boundary().session().connection_id,
            old_session.connection_id
        );
        bot.apply_packet(0x25, join()).await.unwrap();
        let joined = client.received_player_snapshot().await.unwrap();
        let state = received(joined.container_state());
        assert_eq!(*state.value(), ContainerStatus::NoOpening);
        assert_eq!(state.receipt(), received(joined.game_mode()).receipt());
        assert_eq!(state.receipt().session().connection_id, bot.connection_id());
        assert_ne!(
            state.receipt().session().connection_id,
            old_session.connection_id
        );
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_time_repeated_capture_and_same_value_packet() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        assert!(matches!(
            client.received_player_snapshot().await.unwrap().health(),
            FieldState::Missing
        ));
        let mut health = 20f32.to_be_bytes().to_vec();
        health.push(20);
        health.extend(5f32.to_be_bytes());
        bot.apply_packet(0x49, health.clone()).await.unwrap();
        let before_capture = std::time::Instant::now();
        let one = client.received_player_snapshot().await.unwrap();
        let receipt = received(one.health()).receipt();
        assert!(receipt.applied_at() <= before_capture);
        assert!(receipt.applied_at() <= one.captured_at());
        for _ in 0..3 {
            assert_eq!(
                received(client.received_player_snapshot().await.unwrap().health()).receipt(),
                receipt
            );
        }
        bot.apply_packet(0x3f, vec![2]).await.unwrap();
        assert_eq!(
            received(client.received_player_snapshot().await.unwrap().health()).receipt(),
            receipt
        );
        bot.apply_packet(0x49, health).await.unwrap();
        assert!(
            received(client.received_player_snapshot().await.unwrap().health())
                .receipt()
                .ordinal()
                > receipt.ordinal()
        );
        let old = client
            .received_player_snapshot()
            .await
            .unwrap()
            .boundary()
            .session();
        bot.apply_packet(0x25, join()).await.unwrap();
        let replacement = client.received_player_snapshot().await.unwrap();
        let FieldState::Invalidated(event) = replacement.health() else {
            panic!("replacement join")
        };
        assert_eq!(event.reason(), InvalidationReason::SessionBoundary);
        assert_eq!(event.invalidated_session(), old);
        let ordinal = event.event_ordinal();
        let applied_at = event.event_applied_at();
        bot.apply_packet(0x3f, vec![1]).await.unwrap();
        let later = client.received_player_snapshot().await.unwrap();
        let FieldState::Invalidated(event) = later.health() else {
            panic!("persistent invalidation")
        };
        assert_eq!(event.event_ordinal(), ordinal);
        assert_eq!(event.event_applied_at(), applied_at);
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_models_copy_metadata_and_effect_tombstones() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        bot.apply_packet(0x35, correction(0)).await.unwrap();
        let one = client.received_player_snapshot().await.unwrap();
        let position_receipt = received(one.absolute_position()).receipt();
        {
            let mut player = bot.player.lock().await;
            player.x = 99.;
            player.yaw = 99.;
            player.on_ground = true;
        }
        bot.common_receipts.lock().await.selected_hotbar = Some(api::ObservedValue {
            value: 8,
            source: api::ValueSource::Submitted,
        });
        let unchanged = client.received_player_snapshot().await.unwrap();
        assert_eq!(
            received(unchanged.absolute_position()).receipt(),
            position_receipt
        );
        assert_eq!(
            received(unchanged.absolute_position()).value(),
            &[1., 64., 2.]
        );
        assert!(matches!(unchanged.selected_hotbar(), FieldState::Missing));
        bot.apply_packet(0x35, correction(31)).await.unwrap();
        assert!(matches!(
            client
                .received_player_snapshot()
                .await
                .unwrap()
                .absolute_position(),
            FieldState::Invalidated(_)
        ));
        bot.apply_packet(0x38, vec![42, 1]).await.unwrap(); // removal before addition
        let removed = client.received_player_snapshot().await.unwrap();
        let tombstone = received(removed.effect("minecraft:speed").unwrap()).receipt();
        assert!(matches!(
            received(removed.effect("minecraft:speed").unwrap()).value(),
            Presence::Absent
        ));
        assert_eq!(removed.effects_coverage(), Coverage::Partial);
        bot.apply_packet(0x38, vec![42, 1]).await.unwrap();
        assert!(
            received(
                client
                    .received_player_snapshot()
                    .await
                    .unwrap()
                    .effect("minecraft:speed")
                    .unwrap()
            )
            .receipt()
            .ordinal()
                > tombstone.ordinal()
        );
        let mut attributes = vec![42];
        attributes.extend(1i32.to_be_bytes());
        put_string(&mut attributes, "generic.movement_speed");
        attributes.extend(2f64.to_be_bytes());
        attributes.push(1);
        attributes.extend([0; 16]);
        attributes.extend(3f64.to_be_bytes());
        attributes.push(0);
        bot.apply_packet(0x58, attributes).await.unwrap();
        let raw = client.received_player_snapshot().await.unwrap();
        let Presence::Present(attribute) =
            received(raw.attribute("minecraft:movement_speed")).value()
        else {
            panic!("attribute")
        };
        assert_eq!(attribute.base(), 2.);
        assert_eq!(attribute.modifiers().len(), 1);
        assert_eq!(
            bot.common_receipts.lock().await.attributes["minecraft:movement_speed"]
                .value
                .value,
            5.
        );
        bot.apply_packet(0x3f, vec![3]).await.unwrap();
        let mut slots = vec![0, 0, 46];
        slots.extend([0; 46]);
        bot.apply_packet(0x14, slots).await.unwrap();
        let before = client.received_player_snapshot().await.unwrap();
        assert_eq!(before.inventory().slots_coverage(), Coverage::Complete);
        assert!(matches!(before.inventory().cursor(), FieldState::Missing));
        let mut partial = vec![0, 0, 45];
        partial.extend([0; 45]);
        bot.apply_packet(0x14, partial).await.unwrap();
        let withdrawn = client.received_player_snapshot().await.unwrap();
        assert!(matches!(
            withdrawn.inventory().slot(45).unwrap(),
            FieldState::Invalidated(_)
        ));
        assert!(matches!(
            withdrawn.inventory().last_full_contents(),
            FieldState::Invalidated(_)
        ));
        bot.apply_packet(0x3a, respawn()).await.unwrap();
        let reset = client.received_player_snapshot().await.unwrap();
        assert!(bot.common_receipts.lock().await.inventory.slots[0].is_some());
        assert!(matches!(
            reset.inventory().slot(0).unwrap(),
            FieldState::Invalidated(_)
        ));
        let FieldState::Invalidated(event) = reset.selected_hotbar() else {
            panic!("copied hotbar")
        };
        assert_eq!(event.reason(), InvalidationReason::RespawnReset);
        assert_eq!(event.invalidated_session(), before.boundary().session());
        assert_eq!(event.event_session(), reset.boundary().session());
        assert_eq!(event.event_ordinal(), Some(reset.boundary().ordinal()));
        assert!(event.event_applied_at().unwrap() <= reset.captured_at());
        assert!(matches!(reset.entity_id(), FieldState::Invalidated(_)));
        assert!(matches!(
            reset.effect("minecraft:speed").unwrap(),
            FieldState::Invalidated(_)
        ));
        assert_eq!(reset.effects_coverage(), Coverage::Unknown);
        assert_eq!(reset.effects().count(), removed.effects().count());
        assert!(matches!(
            reset.attribute("never:received"),
            FieldState::Invalidated(_)
        ));
        assert!(matches!(reset.dimension_geometry(), FieldState::Missing));
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_opening_reuse_and_matching_close() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        let mut open = vec![7, 0];
        put_string(&mut open, "{}");
        bot.apply_packet(0x2e, open.clone()).await.unwrap();
        let first = client
            .received_player_snapshot()
            .await
            .unwrap()
            .container()
            .unwrap()
            .id();
        bot.apply_packet(0x2e, open).await.unwrap();
        let second = client
            .received_player_snapshot()
            .await
            .unwrap()
            .container()
            .unwrap()
            .id();
        assert_ne!(first, second);
        bot.apply_packet(0x13, vec![6]).await.unwrap();
        assert!(
            matches!(received(client.received_player_snapshot().await.unwrap().container_state()).value(), ContainerStatus::Open(id) if *id == second)
        );
        bot.apply_packet(0x13, vec![7]).await.unwrap();
        let closed = client.received_player_snapshot().await.unwrap();
        assert!(
            matches!(received(closed.container_state()).value(), ContainerStatus::Closed(id) if *id == second)
        );
        assert!(closed.container().is_none());
        drop(client);
        finish(bot, release, server).await;
    }
    #[tokio::test]
    async fn received_snapshot_fails_closed_after_reader_failure_and_revoke() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x25, join()).await.unwrap();
        assert!(bot.apply_packet(0x49, vec![]).await.is_err());
        assert!(client.received_player_snapshot().await.is_err());
        let _revocation = bot.revoke_connection();
        assert!(client.received_player_snapshot().await.is_err());
        drop(client);
        finish(bot, release, server).await;
    }
}
