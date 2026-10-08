//! Whole loaded chunk columns in the common Client contract.
use super::*;
use crate::client as api;

impl crate::client::adapter::ChunkOps for Bot {
    async fn loaded_chunks(&self) -> Result<api::LoadedChunks> {
        let _gate = self.coherent_state_gate.lock().await;
        let (session, receive_sequence) = self.chunk_session().await?;
        let chunks = self
            .world
            .lock()
            .await
            .loaded_chunks()
            .into_iter()
            .map(|p| [p.x, p.z])
            .collect();
        Ok(api::LoadedChunks {
            session,
            receive_sequence,
            chunks,
        })
    }

    async fn chunk(&self, position: [i32; 2]) -> Result<Option<api::ChunkObservation>> {
        let _gate = self.coherent_state_gate.lock().await;
        let (session, receive_sequence) = self.chunk_session().await?;
        let Some(snapshot) =
            self.world
                .lock()
                .await
                .chunk_snapshot(crate::versions::java_1_16_1::ChunkPos {
                    x: position[0],
                    z: position[1],
                })
        else {
            return Ok(None);
        };
        // Legacy columns are 16 sections (0..256); absent sections are air. Light keys
        // run from -1 (below) to 16 (above).
        let sections = (0..16)
            .map(|y| {
                snapshot
                    .sections
                    .get(&y)
                    .cloned()
                    .unwrap_or_else(api::chunks::air_section)
            })
            .collect();
        let light = |layers: &std::collections::HashMap<i32, std::sync::Arc<[u8; 2048]>>| {
            (-1..=16).map(|y| layers.get(&y).cloned()).collect()
        };
        Ok(Some(api::ChunkObservation {
            session,
            receive_sequence,
            position,
            min_y: 0,
            height: 256,
            sections,
            sky_light: light(&snapshot.sky_light),
            block_light: light(&snapshot.block_light),
        }))
    }
}

impl Bot {
    async fn chunk_session(&self) -> Result<(api::SessionStamp, u64)> {
        if self.is_stopped() {
            return Err(crate::Error::new(
                crate::ErrorKind::Disconnected,
                anyhow::anyhow!("connection closed"),
            ));
        }
        let generation = self.common_receipts.lock().await.generation;
        Ok((
            api::SessionStamp {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: self.connection_id(),
                world_generation: generation,
            },
            self.protocol_packet_sequence.load(Ordering::Acquire),
        ))
    }
}

impl crate::client::adapter::DigOps for Bot {
    async fn own_on_ground(&self) -> Result<bool> {
        // A running control session writes its predicted ground flag here too.
        Ok(self.player.lock().await.on_ground)
    }
}
