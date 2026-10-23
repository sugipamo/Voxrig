use super::*;
use crate::client::{EntityId, GameMode, Hand, operations::DispatchReceipt};

async fn spawn(session: &Session, client: &crate::Client) -> EntityId {
    let mut packet = Vec::new();
    put_varint(&mut packet, 300);
    packet.extend([7; 16]);
    put_varint(
        &mut packet,
        client
            .registry()
            .builtin_id("minecraft:entity_type", "minecraft:armor_stand")
            .unwrap()
            .value(),
    );
    for v in [9.0f64, 65.0, 8.5] {
        packet.extend(v.to_be_bytes());
    }
    packet.extend([0; 5]);
    session
        .state
        .lock()
        .await
        .receive(ids::play_clientbound::SPAWN_ENTITY, &packet, 256)
        .unwrap();
    client.entity_spawns().await.unwrap().entities[0].id
}

async fn interact(
    client: &crate::Client,
    mode: GameMode,
    target: EntityId,
    hit: [f32; 3],
    hand: Hand,
    sneaking: bool,
) -> Result<DispatchReceipt> {
    match mode {
        GameMode::Survival => {
            client
                .survival()
                .interact_entity_at(target, hit, hand, sneaking)
                .await
        }
        _ => {
            client
                .creative()
                .interact_entity_at(target, hit, hand, sneaking)
                .await
        }
    }
}

#[tokio::test]
async fn entity_at_roundtrip_and_received_lifetime_guards() {
    for mode in [GameMode::Survival, GameMode::Creative] {
        let (session, api, mut peer) = common_ground_fixture(mode).await;
        let client = crate::Client::from_java_1_21_11(api.bot.clone());
        let target = spawn(&session, &client).await;
        let opposite = if mode == GameMode::Survival {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        assert!(
            interact(&client, opposite, target, [0.0; 3], Hand::Main, false)
                .await
                .is_err()
        );
        // Every component is checked, including all non-finite float classes.
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for axis in 0..3 {
                let mut hit = [0.0; 3];
                hit[axis] = invalid;
                assert_eq!(
                    interact(&client, mode, target, hit, Hand::Main, false)
                        .await
                        .unwrap_err()
                        .kind(),
                    ErrorKind::InvalidInput
                );
            }
        }
        let (other_session, other_api, mut other_peer) = common_ground_fixture(mode).await;
        let other = crate::Client::from_java_1_21_11(other_api.bot.clone());
        spawn(&other_session, &other).await;
        assert!(
            interact(&other, mode, target, [0.0; 3], Hand::Main, false)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(10), other_peer.read_u8())
                .await
                .is_err()
        );
        // Roundtrip signed zero, negative coordinates, both hands/flags and finite extremes.
        for hit in [[-0.0, 1.5, -0.25], [f32::MAX, f32::MIN, f32::from_bits(1)]] {
            for (hand, sneaking) in [(Hand::Main, false), (Hand::Off, true)] {
                let receipt = interact(&client, mode, target, hit, hand, sneaking)
                    .await
                    .unwrap();
                assert_eq!(receipt.version, MinecraftVersion::Java1_21_11);
                assert_eq!(receipt.connection_id, session.id);
                assert_eq!(receipt.interaction_sequence, None);
                let (id, body) = read_packet(&mut peer, None).await.unwrap();
                assert_eq!(id, ids::play_serverbound::USE_ENTITY);
                let mut reader = crate::versions::java_1_21_11::wire::Reader::new(&body);
                assert_eq!(reader.varint().unwrap(), 300);
                assert_eq!(reader.varint().unwrap(), 2);
                for expected in hit {
                    assert_eq!(reader.f32().unwrap().to_bits(), expected.to_bits());
                }
                assert_eq!(reader.varint().unwrap(), hand as i32);
                assert_eq!(reader.bool().unwrap(), sneaking);
                assert!(reader.remaining().is_empty());
            }
        }
        let mut destroy = vec![1];
        put_varint(&mut destroy, 300);
        session
            .state
            .lock()
            .await
            .receive(ids::play_clientbound::ENTITY_DESTROY, &destroy, 256)
            .unwrap();
        assert!(
            interact(&client, mode, target, [0.0; 3], Hand::Main, false)
                .await
                .is_err()
        );
        let fresh = spawn(&session, &client).await;
        assert_ne!(target, fresh);
        assert!(
            interact(&client, mode, target, [0.0; 3], Hand::Main, false)
                .await
                .is_err()
        );
        interact(&client, mode, fresh, [0.0; 3], Hand::Main, false)
            .await
            .unwrap();
        read_packet(&mut peer, None).await.unwrap();
        session.state.lock().await.loading.generation += 1;
        assert!(
            interact(&client, mode, fresh, [0.0; 3], Hand::Main, false)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(10), peer.read_u8())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn entity_at_cancellation_and_revocation_do_not_retry() {
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let target = spawn(&session, &client).await;
    let ops = client.survival();
    let writer = session.writer.lock().await;
    let mut attempt = Box::pin(ops.interact_entity_at(target, [0.0; 3], Hand::Main, false));
    pending(attempt.as_mut()).await;
    drop(attempt);
    drop(writer);
    assert_eq!(session.interrupted_packet.load(Ordering::Acquire), -1);
    assert!(
        timeout(Duration::from_millis(10), peer.read_u8())
            .await
            .is_err()
    );
    // An explicit subsequent request can send once after a pre-write cancellation.
    ops.interact_entity_at(target, [0.0; 3], Hand::Main, false)
        .await
        .unwrap();
    read_packet(&mut peer, None).await.unwrap();
    let writer = session.writer.lock().await;
    let mut attempt = Box::pin(ops.interact_entity_at(target, [0.0; 3], Hand::Main, false));
    pending(attempt.as_mut()).await;
    let _ = client.revoke_connection();
    drop(writer);
    assert!(attempt.await.is_err());
    assert!(
        ops.interact_entity_at(target, [0.0; 3], Hand::Main, false)
            .await
            .is_err()
    );
    assert_eq!(session.ledger.test_written_through(), 1);
}

#[tokio::test]
async fn entity_at_partial_frame_closes_sender_without_replay() {
    let (session, api, _peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let target = spawn(&session, &client).await;
    let writer = session.writer.lock().await;
    let (limited, mut prefix) = tokio::io::duplex(1);
    let mut limited = SocketWriter::new(limited);
    let body = crate::versions::java_1_21_11::wire::interact_entity_at(
        target.native_id(),
        [0.0, 1.5, 0.0],
        Hand::Off,
        true,
    )
    .unwrap();
    let mut attempt = Box::pin(session.write_test_frame(
        &mut limited,
        None,
        ids::play_serverbound::USE_ENTITY,
        &body,
    ));
    pending(attempt.as_mut()).await;
    drop(attempt);
    drop(writer);
    assert_eq!(
        session.interrupted_packet.load(Ordering::Acquire),
        ids::play_serverbound::USE_ENTITY
    );
    assert_eq!(
        interact(
            &client,
            GameMode::Survival,
            target,
            [0.0; 3],
            Hand::Main,
            false
        )
        .await
        .unwrap_err()
        .kind(),
        ErrorKind::UncertainDispatch
    );
    assert_eq!(session.ledger.test_written_through(), 0);
    assert!(prefix.read_u8().await.is_ok());
    assert!(
        timeout(Duration::from_millis(10), prefix.read_u8())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn entity_at_rechecks_received_mode_and_mutation_admission() {
    let (session, api, mut peer) = common_ground_fixture(GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let target = spawn(&session, &client).await;
    let ops = client.survival();
    let mut state = session.state.lock().await;
    let mut attempt = Box::pin(ops.interact_entity_at(target, [0.0; 3], Hand::Main, false));
    pending(attempt.as_mut()).await;
    let mut changed = vec![3];
    changed.extend(1.0f32.to_be_bytes());
    state
        .receive(ids::play_clientbound::GAME_STATE_CHANGE, &changed, 256)
        .unwrap();
    drop(state);
    assert!(attempt.await.is_err());
    session.state.lock().await.motion.position_basis =
        super::super::super::motion::PositionBasis::PendingSubmission;
    assert!(
        client
            .creative()
            .interact_entity_at(target, [0.0; 3], Hand::Main, false)
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(10), peer.read_u8())
            .await
            .is_err()
    );
}
