use super::*;
use crate::client::{
    self as api,
    guarded_use::{self as guarded, PhysicalCut},
};

impl Bot {
    pub(crate) async fn guarded_use(
        &self,
        mode: Option<api::GameMode>,
        request: guarded::Request,
        check: guarded::Check,
        deadline: tokio::time::Instant,
        attempt: guarded::Attempt,
    ) -> Result<guarded::GuardedUseResult> {
        self.wait_until_ready().await?;
        let permit = self.connection.reserve_guarded_use().await?;
        let gate = self.coherent_state_gate.clone().lock_owned().await;
        self.common_motion_admission_inner(false).await?;
        if self.connection_state() != ConnectionState::Ready {
            return Err(crate::client::inventory::unavailable(
                "connection not ready",
            ));
        }
        let mut player = self.common_player_unlocked().await?;
        if mode.is_some() && player.game_mode != mode {
            return Err(crate::client::inventory::unavailable(
                "operation requires matching received game mode",
            ));
        }
        if self.common_receipts.lock().await.pending_dispatch {
            return Err(crate::client::inventory::unavailable(
                "prior dispatch remains unresolved; inspect without retry",
            ));
        }
        let actual = self.inventory.read().await.selected_hotbar;
        if player
            .selected_hotbar
            .as_ref()
            .is_some_and(|s| s.value != actual)
        {
            player.selected_hotbar = None;
        }
        let mut payload = vec![request.hand as u8];
        let id = if let Some(target) = request.target {
            crate::body::interaction::validate_server_range(&player, None, target.position)?;
            payload.extend(
                BlockPos {
                    x: target.position[0],
                    y: target.position[1],
                    z: target.position[2],
                }
                .packed()
                .to_be_bytes(),
            );
            put_varint(&mut payload, target.face as i32);
            for value in target.cursor {
                payload.extend(value.to_be_bytes());
            }
            payload.push(0);
            0x2d
        } else {
            0x2e
        };
        self.observe_stop_context().await;
        attempt.freeze(gate);
        // Move, rather than borrow, the gate into the actor command. Dropping this
        // future cannot release physical state while the actor still owns a send.
        permit
            .enqueue(
                self.operation_context(0),
                super::super::lifecycle::GuardedUseCommand {
                    request,
                    check,
                    cut: PhysicalCut::new(player, request.hand),
                    receipts: self.common_receipts.clone(),
                    id,
                    payload,
                    deadline,
                    attempt,
                },
            )
            .wait()
            .await
    }
    /// Native additive use. Guard/deadline/evidence contract is documented in
    /// `client::guarded_use`; no mode is inferred or changed by this native API.
    pub async fn use_item_guarded<F>(
        &self,
        hand: Hand,
        expected: Option<guarded::HeldItemExpectation>,
        check: F,
    ) -> Result<guarded::GuardedUseResult>
    where
        F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
    {
        crate::Client {
            adapter: crate::connection::Adapter::Java1_16_1(Box::new(self.clone())),
        }
        .guarded_use(
            None,
            guarded::Request {
                hand: if hand == Hand::Main {
                    api::Hand::Main
                } else {
                    api::Hand::Off
                },
                target: None,
                expected,
            },
            Box::new(check),
        )
        .await
    }
    /// Native guarded block use, including received empty-hand expectations.
    pub async fn use_on_block_guarded<F>(
        &self,
        target: guarded::BlockUseTarget,
        hand: Hand,
        expected: Option<guarded::HeldItemExpectation>,
        check: F,
    ) -> Result<guarded::GuardedUseResult>
    where
        F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
    {
        crate::Client {
            adapter: crate::connection::Adapter::Java1_16_1(Box::new(self.clone())),
        }
        .guarded_use(
            None,
            guarded::Request {
                hand: if hand == Hand::Main {
                    api::Hand::Main
                } else {
                    api::Hand::Off
                },
                target: Some(target),
                expected,
            },
            Box::new(check),
        )
        .await
    }
}
