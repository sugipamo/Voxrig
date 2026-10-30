use super::*;
use crate::client::guarded_use::{self as guarded, PhysicalCut};

impl Operations {
    pub(crate) async fn guarded_use(
        &self,
        mode: Option<GameMode>,
        request: guarded::Request,
        check: guarded::Check,
        deadline: tokio::time::Instant,
        attempt: guarded::Attempt,
    ) -> Result<guarded::GuardedUseResult> {
        let work = ScopedWork::new(self.bot.session.ledger.try_work()?);
        self.bot.wait_until_ready().await?;
        // Match existing state -> writer ordering. Both locks remain owned from
        // the final snapshot through full flush, including cancellation guards.
        let state = self.bot.session.state.lock().await;
        let mut writer = self.bot.session.writer.lock().await;
        self.bot.session.check_outbound()?;
        self.mutable_with_ground_requirement(&state, request.target.is_none())?;
        self.require_mode(&state, mode)?;
        if request.target.is_some() && state.operations.inventory.pending_swap.is_some() {
            return Err(crate::client::inventory::unavailable(
                "inventory swap needs inspection",
            ));
        }
        if let Some(target) = request.target {
            self.check_server_block_range(&state, target.position)?;
        }
        let cut = PhysicalCut::new(self.common_player_unlocked(&state)?, request.hand);
        let sequence = self.next_sequence()?;
        let mut payload = Vec::new();
        put_varint(&mut payload, request.hand as i32);
        let id = if let Some(target) = request.target {
            payload.extend(pack_position(target.position).to_be_bytes());
            put_varint(&mut payload, target.face as i32);
            for value in target.cursor {
                payload.extend(value.to_be_bytes());
            }
            payload.extend([0, 0]);
            put_varint(&mut payload, sequence);
            ids::play_serverbound::BLOCK_PLACE
        } else {
            put_varint(&mut payload, sequence);
            for value in state.rotation {
                payload.extend(value.to_be_bytes());
            }
            ids::play_serverbound::USE_ITEM
        };
        if attempt.cancelled() || tokio::time::Instant::now() >= deadline {
            return Ok(guarded::GuardedUseResult::refused(
                guarded::UseNotSentReason::AdmissionDeadline,
                Some(cut),
            ));
        }
        if let Err(reason) = request.check(&cut, check) {
            return Ok(guarded::GuardedUseResult::refused(reason, Some(cut)));
        }
        if tokio::time::Instant::now() >= deadline || !attempt.commit() {
            return Ok(guarded::GuardedUseResult::refused(
                guarded::UseNotSentReason::AdmissionDeadline,
                Some(cut),
            ));
        }
        let compression = writer.compression;
        // The first await after check initiates frame I/O directly; it never
        // reacquires readiness, state, writer or queue capacity.
        self.bot
            .session
            .write_frame(
                &mut writer.stream,
                compression,
                id,
                &payload,
                work.guard(),
                None,
            )
            .await?;
        Ok(guarded::GuardedUseResult::Sent {
            receipt: crate::client::DispatchReceipt {
                version: crate::MinecraftVersion::Java1_21_11,
                connection_id: self.bot.session.id,
                interaction_sequence: Some(sequence),
            },
            cut: Box::new(cut),
        })
    }
    /// Native guarded use; see `client::guarded_use` for callback restrictions,
    /// the five-second budget and dispatch-only evidence. Never retries.
    pub async fn use_item_guarded<F>(
        &self,
        hand: crate::client::Hand,
        expected: Option<guarded::HeldItemExpectation>,
        check: F,
    ) -> Result<guarded::GuardedUseResult>
    where
        F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
    {
        crate::Client::from_java_1_21_11(self.bot.clone())
            .guarded_use(
                None,
                guarded::Request {
                    hand,
                    target: None,
                    expected,
                },
                Box::new(check),
            )
            .await
    }
    /// Native guarded explicit block use, including received empty expectations.
    pub async fn use_on_block_guarded<F>(
        &self,
        target: guarded::BlockUseTarget,
        hand: crate::client::Hand,
        expected: Option<guarded::HeldItemExpectation>,
        check: F,
    ) -> Result<guarded::GuardedUseResult>
    where
        F: FnOnce(&PhysicalCut) -> bool + Send + 'static,
    {
        crate::Client::from_java_1_21_11(self.bot.clone())
            .guarded_use(
                None,
                guarded::Request {
                    hand,
                    target: Some(target),
                    expected,
                },
                Box::new(check),
            )
            .await
    }
}
