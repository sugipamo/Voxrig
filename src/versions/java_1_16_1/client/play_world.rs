//! Protocol 736 world packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

impl Bot {
    pub(super) async fn receive_digging_acknowledgement(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let acknowledgement = parse_digging_ack(p)?;
        let geometry_revision_before = self.block_geometry_revision.load(Ordering::Acquire);
        self.world.lock().await.apply_acknowledged_block_state(
            acknowledgement.position,
            acknowledgement.block_state_id,
        );
        self.advance_block_geometry_revision();
        let geometry_revision_after = self.block_geometry_revision.load(Ordering::Acquire);
        self.world_updated.notify_waiters();
        let confirmation = self
            .connection
            .confirm_dig_transaction_detailed(
                acknowledgement.position,
                acknowledgement.status,
                acknowledgement.successful,
            )
            .await;
        if let Some(correlation) = confirmation.diagnostic_correlation {
            crate::lifecycle::emit_dig_lifecycle(|| {
                serde_json::json!({
                    "stage": "acknowledgement_received",
                    "correlation_id": correlation.get(),
                    "position": {"x": acknowledgement.position.x, "y": acknowledgement.position.y, "z": acknowledgement.position.z},
                    "status": acknowledgement.status,
                    "successful": acknowledgement.successful,
                    "block_state_id": acknowledgement.block_state_id,
                    "geometry_revision_before": geometry_revision_before,
                    "geometry_revision_after": geometry_revision_after,
                    "pending_matched": true,
                })
            });
        }
        self.common_mining_target_received(
            [
                acknowledgement.position.x,
                acknowledgement.position.y,
                acknowledgement.position.z,
            ],
            acknowledgement.block_state_id,
            packet_sequence,
            Some((acknowledgement.status, acknowledgement.successful)),
        )
        .await?;
        self.emit(Event::DiggingAcknowledged(acknowledgement));
        Ok(())
    }

    pub(super) async fn receive_block_change(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let (x, y, z, state_id) = self.world.lock().await.apply_block_change(p)?;
        self.advance_block_geometry_revision();
        self.world_updated.notify_waiters();
        self.common_mining_target_received([x, y, z], state_id, packet_sequence, None)
            .await?;
        self.common_placement_block_received([x, y, z], state_id, packet_sequence)
            .await?;
        self.emit(Event::BlockChanged { x, y, z, state_id });
        Ok(())
    }

    pub(super) async fn receive_multi_block_change(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let changes = self
            .world
            .lock()
            .await
            .apply_multi_block_change_with_changes(p)?;
        let count = changes.len();
        if let Some(kind) =
            crate::client::events::bounds(changes.iter().map(|(p, _)| [p.x, p.y, p.z]))
        {
            self.common_events
                .lock()
                .expect("event ledger poisoned")
                .record(packet_sequence, kind);
        }
        for (position, state_id) in changes {
            self.common_placement_block_received(
                [position.x, position.y, position.z],
                state_id,
                packet_sequence,
            )
            .await?;
            self.common_mining_target_received(
                [position.x, position.y, position.z],
                state_id,
                packet_sequence,
                None,
            )
            .await?;
        }
        self.advance_block_geometry_revision();
        self.world_updated.notify_waiters();
        self.emit(Event::MultiBlockChanged { count });
        Ok(())
    }

    pub(super) async fn receive_explosion(&self, p: &[u8]) -> Result<()> {
        let explosion = parse_explosion(p)?;
        self.common_receipts
            .lock()
            .await
            .vehicles
            .interrupt_motion(self.protocol_packet_sequence.load(Ordering::Acquire));
        self.world
            .lock()
            .await
            .apply_explosion_blocks(&explosion.affected_blocks);
        self.advance_block_geometry_revision();
        self.world_updated.notify_waiters();
        let mut motion = self.motion.lock().await;
        motion.velocity.x += explosion.player_motion.x;
        motion.velocity.y += explosion.player_motion.y;
        motion.velocity.z += explosion.player_motion.z;
        drop(motion);
        self.interrupt_common_motion("native explosion interrupted finite motion")
            .await;
        self.emit(Event::Explosion(explosion));
        Ok(())
    }

    pub(super) async fn receive_unload_chunk(&self, p: &[u8]) -> Result<()> {
        let (x, z) = self.world.lock().await.unload_chunk(p)?;
        self.advance_block_geometry_revision();
        self.common_mining_chunk_changed([x, z]).await;
        self.common_placement_chunk_changed([x, z]).await;
        self.emit(Event::ChunkUnloaded { x, z });
        Ok(())
    }

    pub(super) async fn receive_chunk_data(&self, p: &[u8]) -> Result<()> {
        match self
            .world
            .lock()
            .await
            .apply_chunk(p, self.connection_options.max_chunks)
        {
            Ok((x, z)) => {
                self.advance_block_geometry_revision();
                self.world_updated.notify_waiters();
                self.common_mining_chunk_changed([x, z]).await;
                self.common_placement_chunk_changed([x, z]).await;
                self.emit(Event::ChunkLoaded { x, z });
            }
            Err(e) => self.emit(Event::Error {
                kind: "chunk_decode",
                message: e.to_string(),
            }),
        }
        Ok(())
    }

    pub(super) async fn receive_light_update(&self, p: &[u8]) -> Result<()> {
        match self
            .world
            .lock()
            .await
            .apply_light(p, self.connection_options.max_chunks)
        {
            Ok((x, z)) => {
                self.world_updated.notify_waiters();
                self.emit(Event::ChunkLightUpdated { x, z });
            }
            Err(e) => self.emit(Event::Error {
                kind: "light_decode",
                message: e.to_string(),
            }),
        }
        Ok(())
    }
}
