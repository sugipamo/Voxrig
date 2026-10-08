//! Protocol 736 player packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

impl Bot {
    pub(super) async fn receive_game_state_change(&self, p: &[u8]) -> Result<()> {
        let mut c = Cursor::new(p);
        let change = GameStateChange {
            reason: c.read_u8()?,
            value: c.read_f32::<BigEndian>()?,
        };
        let mut state = self.survival.write().await;
        match change.reason {
            1 => state.raining = Some(false),
            2 => state.raining = Some(true),
            3 => state.game_mode = Some(change.value as u8),
            7 => state.rain_level = Some(change.value),
            8 => state.thunder_level = Some(change.value),
            _ => {}
        }
        drop(state);
        if change.reason == 3 {
            self.interrupt_common_motion("native game mode changed after finite motion started")
                .await;
        }
        self.emit(Event::GameStateChange(change));
        Ok(())
    }

    pub(super) async fn receive_join_game(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let join = parse_join(p)?;
        {
            let mut receipts = self.common_receipts.lock().await;
            receipts.generation = packet_sequence;
            receipts.ground_source = None;
            receipts.rotation_source = None;
            receipts.entities.clear();
            receipts.vehicles.clear();
            receipts
                .registries
                .legacy_join(join.registry_codec.clone(), packet_sequence)?;
            receipts.recipes = Default::default();
            receipts.container = None;
            receipts.inventory.window_id = None;
            receipts.inventory.cursor = None;
            receipts.player_starts.clear();
        }
        let mut player = self.player.lock().await;
        player.entity_id = Some(join.entity_id);
        player.spawned = true;
        drop(player);
        *self.oxygen_level.lock().await = Some(protocol_default_oxygen_level());
        *self.local_pose.lock().await = Some(0);
        let mut survival = self.survival.write().await;
        survival.game_mode = Some(join.game_mode);
        survival.previous_game_mode = Some(join.previous_game_mode);
        survival.dimension = Some(join.dimension);
        survival.world_name = Some(join.world_name);
        drop(survival);
        self.ready.notify_waiters();
        self.emit(Event::Spawn);
        self.set_client_settings_protocol(self.client_settings().await)
            .await?;
        Ok(())
    }

    pub(super) async fn receive_player_abilities(&self, p: &[u8]) -> Result<()> {
        let mut c = Cursor::new(p);
        let flags = c.read_i8()? as u8;
        let mut state = self.survival.write().await;
        state.invulnerable = flags & 0x01 != 0;
        state.flying = flags & 0x02 != 0;
        state.flying_allowed = flags & 0x04 != 0;
        let mut receipts = self.common_receipts.lock().await;
        receipts.may_fly = Some(state.flying_allowed);
        receipts.abilities = Some(crate::client::received(
            flags,
            self.protocol_packet_sequence.load(Ordering::Acquire),
        ));
        if !state.flying_allowed {
            receipts.requested_flying = false;
        }
        drop(receipts);
        state.creative_mode = flags & 0x08 != 0;
        state.flying_speed = c.read_f32::<BigEndian>()?;
        state.walking_speed = c.read_f32::<BigEndian>()?;
        drop(state);
        self.emit(Event::SurvivalStateUpdated);
        Ok(())
    }

    pub(super) async fn receive_combat_event(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let event = parse_combat_event(p)?;
        if let crate::versions::java_1_16_1::CombatEvent::Death {
            player_id,
            message_json,
            ..
        } = &event
        {
            if Some(*player_id) == lock_packet_state(&self.player).await.entity_id {
                self.common_receipts.lock().await.death_message = Some(crate::client::received(
                    crate::client::ui::UiText::LegacyJson {
                        json: message_json.clone(),
                    },
                    packet_sequence,
                ));
            }
        }
        self.emit(Event::Combat(event));
        Ok(())
    }

    pub(super) async fn receive_face_player(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let source_anchor = get_varint(&mut rest)?;
        let mut cursor = Cursor::new(rest);
        let target = Vec3 {
            x: cursor.read_f64::<BigEndian>()?,
            y: cursor.read_f64::<BigEndian>()?,
            z: cursor.read_f64::<BigEndian>()?,
        };
        let consumed = cursor.position() as usize;
        rest = &rest[consumed..];
        let is_entity = *rest.first().context("missing face-player entity flag")? != 0;
        rest = &rest[1..];
        if is_entity {
            let _ = get_varint(&mut rest)?;
            let _ = get_varint(&mut rest)?;
        }
        let mut player = self.player.lock().await;
        let source_y = player.y + if source_anchor == 1 { 1.62 } else { 0.0 };
        let dx = target.x - player.x;
        let dy = target.y - source_y;
        let dz = target.z - player.z;
        player.yaw = (-dx).atan2(dz).to_degrees() as f32;
        player.pitch = (-dy).atan2(dx.hypot(dz)).to_degrees() as f32;
        // A direction computed from a target is not a received yaw/pitch value.
        self.common_receipts.lock().await.rotation_source =
            Some(crate::client::ValueSource::Predicted);
        self.emit(Event::Position(player.clone()));
        Ok(())
    }

    pub(super) async fn receive_remove_effect(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        let effect_id = *rest.first().context("missing removed effect ID")? as i8;
        if Some(entity_id) == self.player.lock().await.entity_id {
            if let Some(name) = crate::client::player_facts::effect_name(
                crate::MinecraftVersion::Java1_16_1,
                i32::from(effect_id),
            ) {
                self.common_receipts.lock().await.effects.remove(&name);
            }
            self.survival.write().await.effects.remove(&effect_id);
            self.emit(Event::SurvivalStateUpdated);
        }
        Ok(())
    }

    pub(super) async fn receive_respawn(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let respawn = parse_respawn(p)?;
        {
            let mut receipts = self.common_receipts.lock().await;
            receipts.generation = packet_sequence;
            receipts.entities.clear();
            receipts.vehicles.clear();
            receipts.pose = None;
            receipts.position_source = None;
            receipts.ground_source = None;
            receipts.rotation_source = None;
            receipts.health = None;
            receipts.using_item = None;
            receipts.attributes.clear();
            receipts.effects.clear();
            receipts.air_supply = None;
            receipts.may_fly = None;
            receipts.requested_flying = false;
            receipts.container = None;
            receipts.inventory.window_id = None;
            receipts.inventory.cursor = None;
            receipts.player_starts.clear();
            if !respawn.copy_metadata {
                receipts.inventory = Default::default();
                receipts.selected_hotbar = None;
                receipts.player_starts.clear();
            }
        }
        if !respawn.copy_metadata {
            *self.local_pose.lock().await = Some(0);
        }
        let mut state = self.survival.write().await;
        state.dimension = Some(respawn.dimension.clone());
        state.world_name = Some(respawn.world_name.clone());
        state.game_mode = Some(respawn.game_mode);
        state.previous_game_mode = Some(respawn.previous_game_mode);
        self.world_time_observed.store(false, Ordering::Release);
        let mut oxygen_level = self.oxygen_level.lock().await;
        *oxygen_level = oxygen_level_after_respawn(*oxygen_level, respawn.copy_metadata);
        drop(oxygen_level);
        if !respawn.copy_metadata {
            state.effects.clear();
            state.attributes.clear();
        }
        drop(state);
        self.world.lock().await.clear();
        self.advance_block_geometry_revision();
        **self.motion.lock().await = MotionState::default();
        *self.positioned.lock().await = false;
        **self.entities.write().await = EntityTracker::default();
        if !respawn.copy_metadata {
            **self.inventory.write().await = InventoryState::default();
        } else {
            self.inventory.write().await.last_transaction = None;
        }
        crate::client::respawn::received(&self.respawn_history, packet_sequence, packet_sequence);
        self.emit(Event::Respawn(respawn));
        Ok(())
    }

    pub(super) async fn receive_vitals(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let vitals = parse_vitals(p)?;
        self.survival.write().await.vitals = Some(vitals);
        self.common_receipts.lock().await.health = Some(crate::client::received(
            crate::client::Health {
                health: vitals.health,
                food: vitals.food,
                saturation: vitals.saturation,
            },
            packet_sequence,
        ));
        self.emit(Event::Vitals(vitals));
        Ok(())
    }

    pub(super) async fn receive_world_time(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let mut c = Cursor::new(p);
        let mut state = self.survival.write().await;
        state.world_age = c.read_i64::<BigEndian>()?;
        state.time_of_day = c.read_i64::<BigEndian>()?;
        self.world_time_observed.store(true, Ordering::Release);
        let time = crate::client::WorldTime {
            game_time: state.world_age,
            day_time: state.time_of_day,
        };
        drop(state);
        self.common_receipts.lock().await.world_time =
            Some(crate::client::received(time, packet_sequence));
        self.emit(Event::SurvivalStateUpdated);
        Ok(())
    }

    pub(super) async fn receive_entity_attributes(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let (entity_id, attributes) = parse_attributes(p)?;
        if Some(entity_id) == lock_packet_state(&self.player).await.entity_id {
            {
                let mut receipts = self.common_receipts.lock().await;
                for attribute in &attributes {
                    let modifiers = attribute
                        .modifiers
                        .iter()
                        .map(common_control::legacy_modifier)
                        .collect();
                    receipts.attributes.insert(
                        crate::client::player_facts::legacy_attribute_name(&attribute.key),
                        crate::client::received(
                            crate::client::player_facts::attribute(
                                crate::MinecraftVersion::Java1_16_1,
                                attribute.base,
                                modifiers,
                            ),
                            packet_sequence,
                        ),
                    );
                }
            }
            let mut state = self.survival.write().await;
            for attribute in attributes {
                state.attributes.insert(attribute.key.clone(), attribute);
            }
            drop(state);
            self.emit(Event::SurvivalStateUpdated);
        }
        Ok(())
    }

    pub(super) async fn receive_entity_effect(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let (entity_id, effect) = parse_effect(p)?;
        if Some(entity_id) == self.player.lock().await.entity_id {
            self.interrupt_common_motion("native effect interrupted finite motion")
                .await;
            if let Some(name) = crate::client::player_facts::effect_name(
                crate::MinecraftVersion::Java1_16_1,
                i32::from(effect.id),
            ) {
                self.common_receipts.lock().await.effects.insert(
                    name,
                    crate::client::received(
                        crate::client::player_facts::effect(
                            i32::from(effect.amplifier),
                            effect.duration_ticks,
                            effect.flags as u8,
                        ),
                        packet_sequence,
                    ),
                );
            }
            self.survival
                .write()
                .await
                .effects
                .insert(effect.id, effect);
            self.emit(Event::SurvivalStateUpdated);
        }
        Ok(())
    }
}
