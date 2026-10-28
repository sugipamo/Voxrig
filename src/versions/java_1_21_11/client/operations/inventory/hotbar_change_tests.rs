use super::*;
use crate::client::HotbarChangeOrigin::{Received, Submitted};

#[tokio::test]
async fn received_held_slot_supersedes_submitted_selection_in_change_log() {
    let mut f = CommonFixture::new().await;
    let client = f.client();
    f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[0]).await;
    f.api.select_hotbar(4).await.unwrap();
    let (id, payload) = read_packet(&mut f.peer, None).await.unwrap();
    assert_eq!(
        (id, payload),
        (ids::play_serverbound::HELD_ITEM_SLOT, vec![0, 4])
    );
    let submitted_at = f.api.bot.session.state.lock().await.sequence;
    // Server-side write (pick-item / join / dimension change) with another slot.
    f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[2]).await;
    let override_at = f.api.bot.session.state.lock().await.sequence;
    f.receive(ids::play_clientbound::HELD_ITEM_SLOT, &[2]).await;

    let log = client.selected_hotbar_changes_after(0).await.unwrap();
    assert_eq!(log.sequence, 4);
    let c: Vec<_> = log.changes.iter().map(|c| c.change).collect();
    assert_eq!((c[0].origin, c[0].before, c[0].after), (Received, None, 0));
    assert_eq!(
        (c[1].origin, c[1].before, c[1].after, c[1].receive_sequence),
        (Submitted, Some(0), 4, submitted_at)
    );
    assert_eq!(
        (
            c[2].origin,
            c[2].before,
            c[2].before_origin,
            c[2].after,
            c[2].receive_sequence,
            c[2].superseded_submission
        ),
        (Received, Some(4), Some(Submitted), 2, override_at, Some(2))
    );
    assert!(!c[3].changed());
    assert_eq!(c[3].superseded_submission, None);
    assert!(
        c.iter()
            .all(|c| c.session.version == MinecraftVersion::Java1_21_11
                && c.session.connection_id == f.api.bot.session.id)
    );
    // The received value is the snapshot value; the log keeps the submission.
    let player = client.player_state().await.unwrap();
    assert_eq!(player.selected_hotbar.map(|s| s.value), Some(2));
    assert_eq!(
        client
            .selected_hotbar_changes_after(log.sequence)
            .await
            .unwrap()
            .changes,
        vec![]
    );
    f.stop().await;
    // Readable after disconnect.
    assert_eq!(client.selected_hotbar_changes_after(0).await.unwrap(), log);
}
