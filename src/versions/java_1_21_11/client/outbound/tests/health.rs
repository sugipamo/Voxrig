use super::*;
use crate::client::health::tests::{Packet, contract, payload};

#[tokio::test]
async fn health_log_contract_modern_packet_fixture() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    {
        let mut state = session.state.lock().await;
        state.connection_id = session.id;
        state.dimensions = vec![Dimension::new(-64, 384).unwrap()];
    }
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    contract(&client, |packet| {
        let session = session.clone();
        async move {
            if matches!(packet, Packet::Close) {
                session.stop();
                return;
            }
            let mut state = session.state.lock().await;
            let (id, bytes) = match packet {
                Packet::Health(hp) => (ids::play_clientbound::UPDATE_HEALTH, payload(hp)),
                Packet::Noise => (ids::play_clientbound::EXPERIENCE, vec![0; 6]),
                Packet::World => {
                    let mut spawn = vec![0];
                    put_string(&mut spawn, "minecraft:overworld");
                    spawn.extend([0; 8]);
                    spawn.extend([0, 255, 0, 1, 0, 0, 63, 0]);
                    (ids::play_clientbound::RESPAWN, spawn)
                }
                Packet::Close => unreachable!(),
            };
            state.receive(id, &bytes, 256).unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn health_reconfiguration_boundary_is_not_hidden_by_later_login() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    let mut reader = client.subscribe_health().unwrap();
    let mut state = session.state.lock().await;
    state
        .receive(ids::play_clientbound::UPDATE_HEALTH, &payload(4.), 256)
        .unwrap();
    state
        .receive(ids::play_clientbound::START_CONFIGURATION, &[], 256)
        .unwrap();
    assert!(client.subscribe_health().unwrap().baseline().is_none());
    assert_eq!(
        crate::client::health::tests::vitals(reader.try_recv().unwrap())
            .health
            .health,
        4.
    );
    assert!(matches!(
        reader.try_recv().unwrap(),
        crate::client::HealthEvent::WorldChanged(_)
    ));
    assert_eq!(
        reader.try_recv(),
        Err(crate::client::HealthRecvError::Empty)
    );
}

#[tokio::test]
async fn health_connection_logs_and_admission_are_independent() {
    let (one, api_one, _peer_one) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let (two, api_two, _peer_two) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client_one = crate::Client::from_java_1_21_11(api_one.bot.clone());
    let client_two = crate::Client::from_java_1_21_11(api_two.bot.clone());
    let mut reader_one = client_one.subscribe_health().unwrap();
    let mut reader_two = client_two.subscribe_health().unwrap();
    for session in [&one, &two] {
        let mut state = session.state.lock().await;
        state.connection_id = session.id;
        state
            .receive(ids::play_clientbound::UPDATE_HEALTH, &payload(4.), 256)
            .unwrap();
    }
    let first = crate::client::health::tests::vitals(reader_one.recv().await.unwrap());
    let second = crate::client::health::tests::vitals(reader_two.recv().await.unwrap());
    assert_ne!(first.session.connection_id, second.session.connection_id);
    one.stop();
    assert_eq!(
        reader_one.recv().await.unwrap(),
        crate::client::HealthEvent::Disconnected
    );
    assert_eq!(
        reader_two.try_recv(),
        Err(crate::client::HealthRecvError::Empty)
    );
    assert!(!client_two.health_changes_after(0).await.unwrap().closed);
}

#[tokio::test]
async fn health_receipts_cannot_be_applied_after_terminal_publication() {
    let (session, api, _peer) = common_ground_fixture(crate::client::GameMode::Survival).await;
    let client = crate::Client::from_java_1_21_11(api.bot.clone());
    session.stop();
    let before = client.health_changes_after(0).await.unwrap();
    let mut state = session.state.lock().await;
    assert!(
        state
            .receive(ids::play_clientbound::UPDATE_HEALTH, &payload(4.), 256)
            .is_err()
    );
    let after = client.health_changes_after(0).await.unwrap();
    assert_eq!(before.changes, after.changes);
    assert!(after.closed);
}
