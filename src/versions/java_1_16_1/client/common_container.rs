//! Opening-bound close intents. Native close sends are not received closure.
use super::*;
use crate::client::{self as api, container as contract};

impl Bot {
    pub(crate) async fn common_close_container(
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
        {
            let inventory = self.inventory.read().await;
            if inventory.open_window.as_ref().map(|w| w.id) != Some(window)
                || inventory.cursor.is_some()
                || !inventory.pending_clicks.is_empty()
            {
                return Err(api::inventory::unavailable(
                    "legacy close inventory context unresolved",
                ));
            }
        }
        let record = contract::prepare_close(
            initial,
            captured,
            screen,
            mode,
            self.common_container_close.lock().await.as_ref(),
        )?;
        let id = record.id;
        *self.common_container_close.lock().await = Some(record);
        // Dropping the waiter cannot cancel the connection-owned single write.
        let bot = self.clone_internal();
        let (reply, result) = oneshot::channel();
        tokio::spawn(async move {
            let _gate = bot.coherent_state_gate.lock().await;
            let outcome = async {
                let player = bot.common_player_unlocked().await?;
                let screen = bot
                    .common_receipts
                    .lock()
                    .await
                    .container
                    .as_ref()
                    .map(|s| s.capture(player.session).id);
                {
                    let mut guard = bot.common_container_close.lock().await;
                    let r = guard
                        .as_mut()
                        .filter(|r| r.id == id)
                        .ok_or_else(|| api::inventory::unavailable("close intent superseded"))?;
                    r.context_received(
                        player.session,
                        player.game_mode,
                        screen,
                        player.inventory.cursor.as_ref(),
                    );
                    if r.requires_inspection.is_some() {
                        return Err(api::inventory::unavailable(
                            "close context interrupted before I/O",
                        ));
                    }
                }
                let inventory = bot.inventory.read().await;
                if inventory.open_window.as_ref().map(|w| w.id) != Some(window)
                    || inventory.cursor.is_some()
                    || !inventory.pending_clicks.is_empty()
                {
                    return Err(api::inventory::unavailable(
                        "legacy close context changed before I/O",
                    ));
                }
                drop(inventory);
                bot.connection
                    .bounded_container_close(revision, window)
                    .await
            }
            .await;
            if outcome.is_ok() {
                bot.common_container_close
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .sent();
                bot.clear_local_window_unlocked(window).await;
            } else if let Err(e) = &outcome {
                bot.common_container_close
                    .lock()
                    .await
                    .as_mut()
                    .expect("retained")
                    .inspection(e);
            }
            let value = match outcome {
                Ok(()) => Ok(bot
                    .common_container_close
                    .lock()
                    .await
                    .as_ref()
                    .expect("retained")
                    .clone()),
                Err(e) => Err(e),
            };
            let _ = reply.send(value);
        });
        drop(gate);
        result
            .await
            .map_err(|_| api::inventory::unavailable("close owner unavailable"))?
    }
    pub(crate) async fn common_container_close_record(
        &self,
    ) -> Result<Option<contract::ContainerCloseRecord>> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() || self.connection_state() != ConnectionState::Ready {
            if let Some(r) = self.common_container_close.lock().await.as_mut() {
                r.inspection("close connection closed or uncertain");
            }
        }
        Ok(self.common_container_close.lock().await.clone())
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
            .is_some_and(|r| r.stage == contract::ContainerCloseStage::Pending)
        {
            return Ok(());
        }
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        let screen = receipts
            .container
            .as_ref()
            .map(|s| s.capture(player.session).id);
        if let Some(r) = self.common_container_close.lock().await.as_mut() {
            r.context_received(
                player.session,
                player.game_mode,
                screen,
                player.inventory.cursor.as_ref(),
            );
        }
        Ok(())
    }
}
