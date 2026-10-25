//! Private bridge to existing headless state, writer, ledger and interaction sequences.
use super::*;
use crate::body::{self, BodyBlock, BodyPlayer, BodySnapshot, BodyWorld, ObserveRequest, Value};

pub(crate) use crate::body::evidence::{EvidenceBatch, EvidenceLog, Update};
use crate::body::evidence::{slot, source, value};
pub(in crate::versions::java_1_21_11::client) fn blocks(
    state: &mut State,
    changes: &[([i32; 3], i32)],
) {
    for &(pos, id) in changes {
        state
            .body_evidence
            .push(state.sequence, Update::Block(pos, id));
    }
}
pub(in crate::versions::java_1_21_11::client) fn chunk(state: &mut State, chunk: [i32; 2]) {
    state
        .body_evidence
        .push(state.sequence, Update::Chunk(chunk));
}
pub(in crate::versions::java_1_21_11::client) fn received(state: &mut State, id: i32) {
    use ids::play_clientbound as input;
    match id {
        input::POSITION => {
            if let Some(pose) = &state.motion.received_pose {
                state.body_evidence.push(
                    state.sequence,
                    Update::Pose {
                        position: pose.position,
                        rotation: pose.rotation,
                    },
                );
            }
        }
        input::HELD_ITEM_SLOT => {
            if let Some(slot) = &state.operations.selected_hotbar {
                state
                    .body_evidence
                    .push(state.sequence, Update::Selection(slot.slot));
            }
        }
        input::WINDOW_ITEMS => state.body_evidence.push(state.sequence, Update::Resync),
        input::RESPAWN | input::START_CONFIGURATION | input::LOGIN => {
            state.body_evidence.push(state.sequence, Update::World)
        }
        _ => {}
    }
}
impl Operations {
    pub(super) fn check_body_world(
        &self,
        state: &State,
        expected: Option<BodyWorld>,
    ) -> Result<()> {
        if expected.is_some_and(|world| {
            world
                != (BodyWorld {
                    connection: self.bot.session.id,
                    generation: state.loading.generation,
                })
        }) {
            return Err(invalid("body world changed before dispatch"));
        }
        Ok(())
    }
    pub(crate) async fn body_simple_send(
        &self,
        world: BodyWorld,
        mode: GameMode,
        action: &body::Action,
        expected: Option<(u8, crate::client::registry::RegistryId)>,
    ) -> Result<()> {
        match action {
            body::Action::Look { yaw, pitch } => {
                self.look_body(Some(mode), [*yaw, *pitch], Some(world))
                    .await
            }
            body::Action::SelectHotbar(slot) => {
                self.select_hotbar_body(Some(mode), *slot, Some(world))
                    .await
            }
            body::Action::Chat(text) => self.send_chat_body(text, Some(world)).await,
            body::Action::Command(text) => self.send_command_body(text, Some(world)).await,
            body::Action::UseItem { hand } => self
                .use_item_body(mode, *hand, Some(world), expected)
                .await
                .map(|_| ()),
            _ => Err(invalid("expected one-shot body action")),
        }
    }

    fn body_snapshot_unlocked(
        &self,
        state: &State,
        backend: u64,
        request: &ObserveRequest,
    ) -> Result<BodySnapshot> {
        let p = self.common_player_unlocked(state)?;
        let mut positions = request.blocks.clone();
        if let Some(region) = request.region {
            region.volume()?;
            for x in region.min[0]..=region.max[0] {
                for y in region.min[1]..=region.max[1] {
                    for z in region.min[2]..=region.max[2] {
                        positions.push([x, y, z]);
                    }
                }
            }
        }
        positions.sort_unstable();
        positions.dedup();
        let blocks = positions
            .into_iter()
            .map(|position| {
                let cell = state
                    .world
                    .block(position)
                    .map(crate::versions::java_1_21_11::native_state)
                    .transpose()?;
                Ok(BodyBlock {
                    position,
                    state: cell.map(|value| Value {
                        value,
                        source: body::ValueSource::ReceivedCache {
                            revision: state.sequence,
                        },
                    }),
                })
            })
            .collect::<Result<_>>()?;
        Ok(BodySnapshot {
            backend,
            version: MinecraftVersion::Java1_21_11,
            world: BodyWorld {
                connection: p.session.connection_id,
                generation: p.session.world_generation,
            },
            revision: state.sequence,
            player: BodyPlayer {
                position: p.position.map(value),
                rotation: p.rotation_source.map(|s| Value {
                    value: p.rotation,
                    source: source(s),
                }),
                game_mode: p.game_mode,
                health: p.health.map(value),
                selected_hotbar: p.selected_hotbar.map(value),
                inventory: p.inventory.slots.into_iter().map(|s| s.map(slot)).collect(),
                cursor: p.inventory.cursor.map(slot),
            },
            blocks,
        })
    }
    pub(crate) async fn body_evidence(
        &self,
        backend: u64,
        request: &ObserveRequest,
        cursor: Option<u64>,
    ) -> Result<EvidenceBatch> {
        let state = self.bot.session.state.lock().await;
        let log = &state.body_evidence;
        Ok(EvidenceBatch {
            cursor: log.next,
            gap: cursor.is_some_and(|c| c < log.dropped),
            events: log
                .events
                .iter()
                .filter(|e| cursor.is_some_and(|c| e.ordinal > c))
                .cloned()
                .collect(),
            ack: state.operations.ack,
            snapshot: self.body_snapshot_unlocked(&state, backend, request)?,
        })
    }
    /// Body block dispatch deliberately omits geometric/reach/material filters.
    /// Existing strict operations and their records are untouched.
    pub(crate) async fn body_block_send(
        &self,
        world: BodyWorld,
        action: &body::Action,
        dig_status: i32,
    ) -> Result<i32> {
        self.bot
            .session
            .ledger
            .try_work()?
            .run_scoped(async {
                let state = self.bot.session.state.lock().await;
                self.mutable(&state)?;
                if world
                    != (BodyWorld {
                        connection: self.bot.session.id,
                        generation: state.loading.generation,
                    })
                {
                    return Err(invalid("body world changed before block dispatch"));
                }
                let mut payload = Vec::new();
                let sequence = self.next_sequence()?;
                let packet = match action {
                    body::Action::PlaceBlock {
                        pos,
                        face,
                        cursor,
                        hand,
                        ..
                    } => {
                        crate::client::item_use::validate_cursor(*cursor)?;
                        validate_wire_pos(*pos)?;
                        put_varint(&mut payload, *hand as i32);
                        payload.extend(pack_position(*pos).to_be_bytes());
                        put_varint(&mut payload, *face as i32);
                        for v in cursor {
                            payload.extend(v.to_be_bytes());
                        }
                        payload.extend([0, 0]);
                        put_varint(&mut payload, sequence);
                        ids::play_serverbound::BLOCK_PLACE
                    }
                    body::Action::BreakBlock { pos, face } => {
                        validate_wire_pos(*pos)?;
                        put_varint(&mut payload, dig_status);
                        payload.extend(pack_position(*pos).to_be_bytes());
                        payload.push(*face as u8);
                        put_varint(&mut payload, sequence);
                        ids::play_serverbound::BLOCK_DIG
                    }
                    _ => return Err(invalid("expected body block action")),
                };
                self.bot.session.send(packet, &payload).await?;
                Ok(sequence)
            })
            .await
    }
}

fn validate_wire_pos(pos: [i32; 3]) -> Result<()> {
    if !(-33_554_432..=33_554_431).contains(&pos[0])
        || !(-2048..=2047).contains(&pos[1])
        || !(-33_554_432..=33_554_431).contains(&pos[2])
    {
        return Err(invalid(
            "block position is outside the protocol representation",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_is_bounded_and_gaps_are_explicit() {
        let mut log = EvidenceLog::default();
        for sequence in 0..5000 {
            log.push(sequence, Update::Resync);
        }
        assert_eq!(log.events.len(), 4096);
        assert_eq!(log.dropped, 904);
        assert_eq!(log.events.front().unwrap().ordinal, 905);
    }
    #[test]
    fn protocol_coordinates_do_not_silently_wrap() {
        assert!(validate_wire_pos([30_000_000, -64, -30_000_000]).is_ok());
        assert!(validate_wire_pos([0, 2048, 0]).is_err());
        assert!(validate_wire_pos([33_554_432, 64, 0]).is_err());
    }
}
