//! Translation into the common Client contract, with native actor-owned sends.
use super::*;
use crate::client::adapter::FlightOps;
use crate::client::{self as api, operations::Action};

impl Bot {
    pub(super) async fn common_player_unlocked(&self) -> Result<api::PlayerObservation> {
        if self.is_stopped() {
            return Err(crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection closed"),
            ));
        }
        let player = self.player.lock().await.clone();
        let survival = self.survival.read().await;
        let inventory = self.inventory.read().await;
        let receipts = self.common_receipts.lock().await;
        let mut received_inventory = receipts.inventory.clone();
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_id(),
            world_generation: receipts.generation,
        };
        received_inventory.player_screen = api::container::player_screen_access(
            session,
            received_inventory.window_id,
            receipts.container.as_ref().map(|s| s.capture(session).id),
            super::lock_packet_state(&self.common_container_close)
                .await
                .as_ref(),
        );
        if !inventory.player_slots().is_empty() {
            received_inventory.local_cache = Some(
                inventory
                    .player_slots()
                    .iter()
                    .map(|item| api::legacy_slot(item.as_ref()))
                    .collect::<Result<_>>()?,
            );
        }
        let dismount_pending = self
            .dismount_history
            .lock()
            .expect("dismount history")
            .as_ref()
            .is_some_and(|r| r.unresolved());
        Ok(api::PlayerObservation {
            session: api::SessionStamp {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: self.connection_id(),
                world_generation: receipts.generation,
            },
            receive_sequence: self.protocol_packet_sequence.load(Ordering::Acquire),
            pending_dispatch: api::flight::unresolved(&self.flight_history)
                || dismount_pending
                || receipts.pending_dispatch
                || self.common_motion_pauses_physics().await,
            dimension: survival.dimension.as_ref().map(|name| api::Dimension {
                name: name.clone(),
                min_y: 0,
                height: 256,
            }),
            // The compatibility player cache is updated by local 20Hz physics.
            // Retain the actual correction separately; equality is not provenance.
            position: (*self.positioned.lock().await).then_some(api::ObservedValue {
                value: [player.x, player.y, player.z],
                source: receipts
                    .position_source
                    .unwrap_or(api::ValueSource::Predicted),
            }),
            received_pose: receipts.pose.clone(),
            rotation: [player.yaw, player.pitch],
            game_mode: survival
                .game_mode
                .map(|id| api::GameMode::decode(id & 7))
                .transpose()?,
            may_fly: receipts.may_fly,
            health: receipts.health.clone(),
            selected_hotbar: receipts.selected_hotbar.clone(),
            inventory: received_inventory,
            using_item: receipts.using_item.clone(),
            entity_id: player.entity_id,
            attributes: receipts.attributes.clone(),
            effects: receipts.effects.clone(),
            air_supply: receipts.air_supply.clone(),
            world_time: receipts.world_time.clone(),
        })
    }
    pub(super) async fn execute_common_inner(
        &self,
        mode: api::GameMode,
        action: Action<'_>,
        flight_owner: Option<u64>,
    ) -> Result<Option<i32>> {
        let _gate = self.coherent_state_gate.lock().await;
        self.common_motion_admission_inner(flight_owner.is_some())
            .await?;
        if let Some(attempt) = flight_owner {
            if self.control().await != ControlState::default()
                || self
                    .common_receipts
                    .lock()
                    .await
                    .vehicles
                    .motion_interrupted()
            {
                return Err(common_state(
                    "flight motion context changed before dispatch",
                ));
            }
            let record = self.flight_snapshot(attempt)?;
            let player = self.common_player_unlocked().await?;
            let requested = self.common_receipts.lock().await.requested_flying;
            api::flight::validate_before(&record, &player, requested)?;
        }
        if self.connection_state() != ConnectionState::Ready {
            return Err(common_state("connection not ready"));
        }
        let state = self.survival.read().await;
        if state.game_mode.map(|id| id & 7) != Some(mode_id(mode)) {
            return Err(common_state(
                "operation requires matching received game mode",
            ));
        }
        let can_fly = state.flying_allowed;
        drop(state);
        if self.common_receipts.lock().await.pending_dispatch {
            return Err(common_state(
                "prior dispatch remains unresolved; inspect and reconnect",
            ));
        }
        let (id, payload) = match action {
            Action::Entity(target, interaction) => {
                let receipts = self.common_receipts.lock().await;
                receipts.entities.validate(
                    api::SessionStamp {
                        version: crate::MinecraftVersion::Java1_16_1,
                        connection_id: self.connection_id(),
                        world_generation: receipts.generation,
                    },
                    target,
                )?;
                (0x0e, interaction.payload(target))
            }

            Action::Look(rotation) => {
                api::operations::validate_rotation(rotation)?;
                let player = self.player.lock().await;
                let mut payload = pose_payload([player.x, player.y, player.z], rotation);
                payload.push(u8::from(player.on_ground));
                (0x13, payload)
            }
            Action::SelectHotbar(slot) => {
                if slot > 8 {
                    return Err(api::registry::invalid("hotbar slot must be 0..8"));
                }
                (0x24, i16::from(slot).to_be_bytes().to_vec())
            }
            Action::SetFlying(flying) => {
                require_creative(mode)?;
                if flying && (!can_fly || self.common_receipts.lock().await.may_fly != Some(true)) {
                    return Err(common_state("server has not granted flight"));
                }
                (0x1a, vec![if flying { 2 } else { 0 }])
            }
            Action::MoveFlying(position, rotation) => {
                require_creative(mode)?;
                api::operations::validate_rotation(rotation)?;
                validate_position(position[0], position[1], position[2])?;
                if !can_fly || !self.common_receipts.lock().await.requested_flying {
                    return Err(common_state("flight must be permitted and requested"));
                }
                let player = self.player.lock().await;
                if !*self.positioned.lock().await {
                    return Err(common_state("position unavailable"));
                }
                if [player.x, player.y, player.z]
                    .iter()
                    .zip(position)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
                    > 16.0
                {
                    return Err(api::registry::invalid("flight step exceeds four blocks"));
                }
                let mut payload = pose_payload(position, rotation);
                payload.push(0);
                (0x13, payload)
            }
            Action::SetHotbar(slot, item) => {
                require_creative(mode)?;
                if slot > 8 {
                    return Err(api::registry::invalid("hotbar slot must be 0..8"));
                }
                let item = if let Some((name, count)) = item {
                    let definition =
                        api::registry::Registry::for_version(crate::MinecraftVersion::Java1_16_1)
                            .item(name)?;
                    if count == 0 || u32::from(count) > definition.max_stack_size {
                        return Err(api::registry::invalid("invalid default item stack count"));
                    }
                    Some(ItemStack {
                        item_id: definition.id.value(),
                        count: count as i8,
                        nbt: None,
                    })
                } else {
                    None
                };
                let mut payload = (36 + i16::from(slot)).to_be_bytes().to_vec();
                write_slot(&mut payload, item.as_ref());
                (0x27, payload)
            }
            Action::Dig(position, face) => {
                require_creative(mode)?;
                self.common_reach(position).await?;
                let mut payload = Vec::new();
                put_varint(&mut payload, 0);
                payload.extend(
                    BlockPos {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    }
                    .packed()
                    .to_be_bytes(),
                );
                payload.push(face as u8);
                (0x1b, payload)
            }
            Action::UseOnBlock(position, face, cursor, hand) => {
                api::item_use::validate_cursor(cursor)?;
                self.common_reach(position).await?;
                let mut payload = Vec::new();
                put_varint(&mut payload, hand as i32);
                payload.extend(
                    BlockPos {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    }
                    .packed()
                    .to_be_bytes(),
                );
                put_varint(&mut payload, face as i32);
                for value in cursor {
                    payload.extend(value.to_be_bytes());
                }
                payload.push(0);
                (0x2d, payload)
            }
            Action::UseItem(hand) => {
                let mut payload = Vec::new();
                put_varint(&mut payload, hand as i32);
                (0x2e, payload)
            }
            Action::DigStart(position, face) | Action::DigFinish(position, face) => {
                self.common_reach(position).await?;
                let status = if matches!(action, Action::DigStart(..)) {
                    0
                } else {
                    2
                };
                let mut payload = Vec::new();
                put_varint(&mut payload, status);
                payload.extend(
                    BlockPos {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    }
                    .packed()
                    .to_be_bytes(),
                );
                payload.push(face as u8);
                (0x1b, payload)
            }
            Action::Swing(hand) => {
                let mut payload = Vec::new();
                put_varint(&mut payload, hand as i32);
                (0x2b, payload)
            }
            Action::ReleaseUseItem => {
                // PLAYER_ACTION RELEASE_USE_ITEM with the zero position and face DOWN.
                let mut payload = Vec::new();
                put_varint(&mut payload, 5);
                payload.extend(0u64.to_be_bytes());
                payload.push(0);
                (0x1b, payload)
            }
        };
        // Retain uncertainty before any actor admission/write. Cancellation cannot
        // silently authorize a replay, even if the actor finishes after this future drops.
        {
            let mut receipts = self.common_receipts.lock().await;
            receipts.pending_dispatch = true;
            if let Action::SetHotbar(slot, _) = &action {
                receipts.inventory.slots[36 + usize::from(*slot)] = None;
            }
        }
        self.send(id, &payload).await?;
        if matches!(action, Action::ReleaseUseItem) {
            let received = self.control_received().await;
            self.common_control.lock().await.item_released(&received);
        }
        match action {
            Action::Look(rotation) => {
                let mut player = self.player.lock().await;
                player.yaw = rotation[0];
                player.pitch = rotation[1];
            }
            Action::SelectHotbar(slot) => {
                self.inventory.write().await.selected_hotbar = slot;
                self.common_receipts.lock().await.selected_hotbar = Some(api::ObservedValue {
                    value: slot,
                    source: api::ValueSource::Submitted,
                });
            }
            Action::SetFlying(flying) => {
                self.common_receipts.lock().await.requested_flying = flying
            }
            Action::MoveFlying(position, rotation) => {
                self.common_receipts.lock().await.position_source =
                    Some(api::ValueSource::Submitted);
                let mut player = self.player.lock().await;
                player.x = position[0];
                player.y = position[1];
                player.z = position[2];
                player.yaw = rotation[0];
                player.pitch = rotation[1];
                player.on_ground = false;
            }
            _ => {}
        }
        self.common_receipts.lock().await.pending_dispatch = false;
        Ok(None)
    }
    async fn common_reach(&self, position: [i32; 3]) -> Result<()> {
        if !(0..=255).contains(&position[1])
            || position[0].abs_diff(0) > 30_000_000
            || position[2].abs_diff(0) > 30_000_000
        {
            return Err(common_state(
                "interaction target outside dimension or world bounds",
            ));
        }
        let state = self
            .world
            .lock()
            .await
            .block(position[0], position[1], position[2])
            .ok_or_else(|| common_state("interaction target is not loaded"))?;
        crate::versions::java_1_16_1::native_state(state)?;
        let player = self.player.lock().await;
        if !*self.positioned.lock().await {
            return Err(common_state("position unavailable"));
        }
        let eye = [player.x, player.y + 1.62, player.z];
        if (0..3)
            .map(|i| (eye[i] - f64::from(position[i]) - 0.5).powi(2))
            .sum::<f64>()
            > 4.5f64.powi(2)
        {
            return Err(common_state("interaction target out of reach"));
        }
        Ok(())
    }
}
fn pose_payload(position: [f64; 3], rotation: [f32; 2]) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in position {
        payload.extend(value.to_be_bytes());
    }
    for value in rotation {
        payload.extend(value.to_be_bytes());
    }
    payload
}
fn mode_id(mode: api::GameMode) -> u8 {
    match mode {
        api::GameMode::Survival => 0,
        api::GameMode::Creative => 1,
        api::GameMode::Adventure => 2,
        api::GameMode::Spectator => 3,
    }
}
fn require_creative(mode: api::GameMode) -> Result<()> {
    if mode != api::GameMode::Creative {
        return Err(common_state("creative operation required"));
    }
    Ok(())
}
fn common_state(message: &str) -> crate::Error {
    crate::Error::new(crate::ErrorKind::State, anyhow::anyhow!("{message}"))
}

impl crate::client::adapter::CoreOps for Bot {
    async fn respawn(&self) -> Result<api::RespawnRecord> {
        let bot = self.clone();
        tokio::spawn(async move {
            let _gate = bot.coherent_state_gate.lock().await;
            if bot.connection_state() != ConnectionState::Ready {
                return Err(common_state("respawn requires ready play context"));
            }
            let player = bot.common_player_unlocked().await?;
            api::respawn::prepare(&bot.respawn_history, &player)?;
            let result = bot.respawn().await;
            api::respawn::dispatched(&bot.respawn_history, &result);
            result?;
            Ok(api::respawn::snapshot(&bot.respawn_history).expect("owned respawn"))
        })
        .await
        .map_err(|error| crate::Error::from(anyhow::anyhow!("respawn owner failed: {error}")))?
    }
    async fn connection_identity(&self) -> Result<api::ConnectionIdentity> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        Ok(api::ConnectionIdentity {
            session: player.session,
            uuid: self.login_profile.uuid,
            name: self.login_profile.name.clone(),
        })
    }
    async fn entity_spawns(&self) -> Result<api::EntitySpawns> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_receipts
            .lock()
            .await
            .entities
            .capture(player.session, player.receive_sequence))
    }
    async fn entity_motion(&self, target: api::EntityId) -> Result<api::EntityMotionObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        self.common_receipts.lock().await.entities.capture_motion(
            player.session,
            target,
            player.receive_sequence,
        )
    }
    async fn entities(&self) -> Result<api::EntitiesObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        Ok(self.common_receipts.lock().await.entities.capture_all(
            crate::MinecraftVersion::Java1_16_1,
            player.session,
            player.receive_sequence,
        ))
    }
    async fn vehicle_state(&self) -> Result<api::VehicleObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let native_id = self.player.lock().await.entity_id;
        let receipts = self.common_receipts.lock().await;
        Ok(receipts.vehicles.capture(
            player.session,
            player.receive_sequence,
            native_id,
            &receipts.entities,
        ))
    }
    async fn server_registry_state(&self) -> Result<api::registry::ServerRegistryObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection closed"),
            ));
        }
        let receipts = self.common_receipts.lock().await;
        Ok(receipts.registries.capture(
            api::SessionStamp {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: self.connection_id(),
                world_generation: receipts.generation,
            },
            self.protocol_packet_sequence.load(Ordering::Acquire),
        ))
    }
    async fn screen_state(&self) -> Result<api::container::ScreenObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        Ok(api::container::ScreenObservation {
            session: player.session,
            receive_sequence: player.receive_sequence,
            active_window: receipts.inventory.window_id,
            player_screen: player.inventory.player_screen,
            screen: receipts
                .container
                .as_ref()
                .map(|s| s.capture(player.session)),
            cursor: receipts.inventory.cursor.clone(),
        })
    }
    async fn player_state(&self) -> Result<api::PlayerObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        self.common_player_unlocked().await
    }
    async fn received_recipes(&self) -> Result<api::ReceivedRecipes> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        receipts.recipes.capture(
            player.session,
            player.receive_sequence,
            receipts
                .registries
                .capture(player.session, player.receive_sequence),
        )
    }
    async fn received_recipe_ghost(&self) -> Result<Option<api::ReceivedRecipeGhost>> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        receipts
            .recipe_ghost
            .as_ref()
            .map(|ghost| ghost.capture(player.session))
            .transpose()
            .map(Option::flatten)
    }
    async fn received_crafting_context(&self) -> Result<Option<api::ReceivedCraftingContext>> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        let screen = api::container::ScreenObservation {
            session: player.session,
            receive_sequence: player.receive_sequence,
            active_window: receipts.inventory.window_id,
            player_screen: player.inventory.player_screen,
            screen: receipts
                .container
                .as_ref()
                .map(|s| s.capture(player.session)),
            cursor: receipts.inventory.cursor.clone(),
        };
        let registries = receipts
            .registries
            .capture(player.session, player.receive_sequence);
        let recipes = receipts.recipes.capture(
            player.session,
            player.receive_sequence,
            registries.clone(),
        )?;
        api::ReceivedCraftingContext::capture(player, screen, registries, recipes)
    }
    async fn recipe_book_materials(
        &self,
        recipe: &api::RecipeId,
        crafts: u32,
        maximum_bound: u32,
    ) -> Result<api::RecipeBookMaterials> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        let registries = receipts
            .registries
            .capture(player.session, player.receive_sequence);
        let catalogue = receipts.recipes.capture(
            player.session,
            player.receive_sequence,
            registries.clone(),
        )?;
        let inventory = api::ReceivedInventory::capture(
            player.session,
            player.receive_sequence,
            &player.inventory,
            registries,
        )?;
        api::RecipeBookMaterials::capture(catalogue, inventory, recipe, crafts, maximum_bound)
    }
    async fn received_crafting(&self) -> Result<Option<api::ReceivedCrafting>> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let receipts = self.common_receipts.lock().await;
        let screen = api::container::ScreenObservation {
            session: player.session,
            receive_sequence: player.receive_sequence,
            active_window: receipts.inventory.window_id,
            player_screen: player.inventory.player_screen,
            screen: receipts
                .container
                .as_ref()
                .map(|s| s.capture(player.session)),
            cursor: receipts.inventory.cursor.clone(),
        };
        let registries = receipts
            .registries
            .capture(player.session, player.receive_sequence);
        api::ReceivedCrafting::capture(&player, &screen, registries)
    }
    async fn received_inventory(&self) -> Result<api::ReceivedInventory> {
        let _gate = self.coherent_state_gate.lock().await;
        if self.is_stopped() {
            return Err(crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("connection closed"),
            ));
        }
        let receipts = self.common_receipts.lock().await;
        let session = api::SessionStamp {
            version: crate::MinecraftVersion::Java1_16_1,
            connection_id: self.connection_id(),
            world_generation: receipts.generation,
        };
        let sequence = self.protocol_packet_sequence.load(Ordering::Acquire);
        let registries = receipts.registries.capture(session, sequence);
        api::ReceivedInventory::capture(session, sequence, &receipts.inventory, registries)
    }
    async fn capture(&self, region: crate::Region) -> Result<api::Capture> {
        region.volume()?;
        if region.min[1] < 0 || region.max[1] > 255 {
            return Err(api::registry::invalid("region outside dimension height"));
        }
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        let snapshot = self.observe_region_snapshot(region).await?;
        let blocks = snapshot
            .value
            .into_iter()
            .map(|block| {
                Ok(crate::ObservedBlock {
                    position: [block.x, block.y, block.z],
                    state: block
                        .state_id
                        .map(crate::versions::java_1_16_1::native_state)
                        .transpose()?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(api::Capture {
            world: crate::Observation {
                version: crate::MinecraftVersion::Java1_16_1,
                connection_id: self.connection_id(),
                revision: snapshot.revision,
                receive_sequence: Some(player.receive_sequence),
                captured_at: snapshot.captured_at,
                region,
                blocks,
            },
            player,
        })
    }
    async fn execute(&self, mode: api::GameMode, action: Action<'_>) -> Result<Option<i32>> {
        if let Some(command) = match action {
            Action::SetFlying(flying) => Some(api::FlightCommand::SetFlying { flying }),
            Action::MoveFlying(position, rotation) => {
                Some(api::FlightCommand::Move { position, rotation })
            }
            _ => None,
        } {
            if mode != api::GameMode::Creative {
                return Err(common_state("creative operation required"));
            }
            return self.flight(command).await.map(|_| None);
        }
        self.execute_common_inner(mode, action, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::adapter::CoreOps;

    #[tokio::test]
    async fn received_vehicle_correction_stops_remaining_mounted_inputs() {
        use api::{VehicleControlStage, VehicleInput, VehicleRelation};
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.survival.write().await.game_mode = Some(0);
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        let VehicleRelation::Mounted { mount } = client
            .vehicle_state()
            .await
            .unwrap()
            .relation
            .unwrap()
            .value
        else {
            panic!()
        };
        let inputs = vec![VehicleInput::default(); 20];
        let ops = client.survival();
        let attempt = tokio::spawn(async move { ops.start_vehicle_control(mount, &inputs).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), packets.recv())
            .await
            .unwrap()
            .unwrap();
        bot.apply_packet(0x2c, vec![0; 32]).await.unwrap();
        assert!(attempt.await.unwrap().is_err());
        let stopped = client.vehicle_control_record().await.unwrap().unwrap();
        assert_eq!(stopped.stage, VehicleControlStage::RequiresInspection);
        assert!(stopped.dispatched_ticks < 20);
        assert!(
            stopped
                .requires_inspection
                .as_deref()
                .unwrap()
                .contains("correction")
        );
        assert!(
            client
                .vehicle_state()
                .await
                .unwrap()
                .motion_correction_sequence
                .is_some()
        );
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(
            client
                .vehicle_control_record()
                .await
                .unwrap()
                .unwrap()
                .dispatched_ticks,
            stopped.dispatched_ticks
        );
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn common_display_receipts_clear_reset_and_atomic_tab_match_native_packets() {
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let samples: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/display_packets.json"
        ))
        .unwrap();
        let rows = &samples["versions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["version"] == "1.16.1")
            .unwrap()["packets"];
        let decode = |r: &serde_json::Value| {
            r["payload_hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        assert!(client.titles().await.unwrap().timing.is_none());
        assert!(client.tab_list().await.unwrap().text.is_none());
        assert!(client.world_border().await.unwrap().size.is_none());
        for row in rows.as_array().unwrap() {
            bot.apply_packet(row["packet_id"].as_i64().unwrap() as i32, decode(row))
                .await
                .unwrap();
        }
        let titles = client.titles().await.unwrap();
        assert!(titles.title.unwrap().value.is_none());
        assert!(titles.action_bar.is_some());
        assert!(titles.clear.unwrap().value);
        assert_eq!(
            titles.timing.unwrap().value,
            crate::client::ui::TitleTiming::ResetToDefaults
        );
        let cache = bot.ui.read().await.clone();
        assert_eq!(
            (cache.title.fade_in, cache.title.stay, cache.title.fade_out),
            (10, 70, 20)
        );
        assert!(cache.title.title_json.is_none() && cache.title.subtitle_json.is_none());
        assert!(
            cache
                .title
                .action_bar_json
                .as_ref()
                .unwrap()
                .contains("Overlay")
        );
        assert_eq!(
            (
                cache.world_border.warning_time,
                cache.world_border.warning_blocks
            ),
            (17, 3)
        );
        let tab = client.tab_list().await.unwrap();
        assert!(matches!(
            tab.text.as_ref().unwrap().value.header,
            crate::client::ui::UiText::LegacyJson { .. }
        ));
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["group"] == "tab")
            .unwrap();
        let mut truncated = decode(row);
        truncated.pop();
        assert!(bot.apply_packet(0x53, truncated).await.is_err());
        assert_eq!(client.tab_list().await.unwrap().text, tab.text);
        assert_eq!(bot.ui.read().await.tab_footer_json, cache.tab_footer_json);
        bot.common_receipts.lock().await.generation += 1;
        let border = client.world_border().await.unwrap();
        assert!(border.center.is_none() && border.size.is_none() && border.warning_delay.is_none());
        client.disconnect().await.unwrap();
        assert!(
            client.titles().await.is_err()
                && client.tab_list().await.is_err()
                && client.world_border().await.is_err()
        );
        let _ = release.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn common_boss_bar_transport_preserves_fields_and_actual_removal() {
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/boss_bar_packets.json"
        ))
        .unwrap();
        let rows = fixtures["versions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["version"] == "1.16.1")
            .unwrap();
        let decode = |r: &serde_json::Value| {
            r["payload_hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        assert!(
            client
                .boss_bars()
                .await
                .unwrap()
                .last_update_sequence
                .is_none()
        );
        bot.apply_packet(0x0c, decode(&rows["packets"][0]))
            .await
            .unwrap();
        let before = client.boss_bars().await.unwrap();
        bot.apply_packet(0x0c, decode(&rows["packets"][2]))
            .await
            .unwrap();
        let updated = client.boss_bars().await.unwrap();
        assert_eq!(before.bars[0].title, updated.bars[0].title);
        assert_ne!(
            before.bars[0].progress.source,
            updated.bars[0].progress.source
        );
        assert_eq!(bot.ui.read().await.boss_bars.len(), 1);
        bot.apply_packet(0x0c, decode(&rows["packets"][1]))
            .await
            .unwrap();
        let removed = client.boss_bars().await.unwrap();
        assert!(removed.bars.is_empty());
        assert!(removed.last_update_sequence > updated.last_update_sequence);
        assert!(bot.ui.read().await.boss_bars.is_empty());
        client.disconnect().await.unwrap();
        assert!(client.boss_bars().await.is_err());
        let _ = release.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn common_dismount_ground_owned_cancel_remount_and_revoke_preserve_history() {
        use api::{VehicleRelation, survival::MotionStatus};
        for kind in ["cancel", "remount", "retire", "revoke"] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.player.lock().await.entity_id = Some(42);
            let client = crate::Client::from_java_1_16_1(bot.clone());
            bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
            let VehicleRelation::Mounted { mount } = client
                .vehicle_state()
                .await
                .unwrap()
                .relation
                .unwrap()
                .value
            else {
                panic!();
            };
            let record = client.survival().dismount(mount).await.unwrap();
            let mut position = Vec::new();
            for value in [8.5f64, 65., 8.5] {
                position.extend(value.to_be_bytes());
            }
            position.extend([0; 8]);
            position.push(0);
            put_varint(&mut position, 99);
            bot.apply_packet(0x35, position).await.unwrap();
            bot.apply_packet(0x4b, vec![10, 0]).await.unwrap();
            client
                .survival()
                .complete_dismount(record.id)
                .await
                .unwrap();
            for _ in 0..4 {
                packets.recv().await.unwrap();
            }
            if matches!(kind, "remount" | "retire") {
                let owned = client.clone();
                let waiter =
                    tokio::spawn(async move { owned.survival().resume_ground(record.id).await });
                assert_eq!(packets.recv().await.unwrap().0, 0x13);
                if kind == "remount" {
                    bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
                } else {
                    bot.apply_packet(0x37, vec![1, 10]).await.unwrap();
                }
                assert!(waiter.await.unwrap().is_err());
                let failed = client
                    .dismount_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .grounding
                    .unwrap()
                    .motion;
                assert_eq!(failed.status, MotionStatus::RequiresInspection);
                assert_eq!((failed.attempted_tick, failed.dispatched_ticks), (1, 1));
                let _ = client.revoke_connection();
                assert_eq!(
                    client
                        .dismount_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .grounding
                        .unwrap()
                        .motion
                        .problem,
                    failed.problem
                );
            } else {
                let writer = bot.writer.lock().await;
                let ops = client.survival();
                let mut waiter = Box::pin(ops.resume_ground(record.id));
                assert!(
                    timeout(Duration::from_millis(20), waiter.as_mut())
                        .await
                        .is_err()
                );
                drop(waiter);
                let intent = timeout(Duration::from_millis(50), client.dismount_record())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(intent.id, record.id);
                assert_eq!(
                    intent.grounding.as_ref().unwrap().motion.dispatched_ticks,
                    0
                );
                if kind == "revoke" {
                    let _ = client.revoke_connection();
                    let failed = client
                        .dismount_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .grounding
                        .unwrap()
                        .motion;
                    assert_eq!(failed.status, MotionStatus::RequiresInspection);
                    drop(writer);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    let again = client
                        .dismount_record()
                        .await
                        .unwrap()
                        .unwrap()
                        .grounding
                        .unwrap()
                        .motion;
                    assert_eq!(again.dispatched_ticks, 0);
                    assert_eq!(again.problem, failed.problem);
                } else {
                    drop(writer);
                    for _ in 0..2 {
                        assert_eq!(packets.recv().await.unwrap().0, 0x13);
                    }
                    timeout(Duration::from_secs(2), async {
                        loop {
                            let r = client.dismount_record().await.unwrap().unwrap();
                            let g = r.grounding.unwrap();
                            assert!(g.motion.problem.is_none(), "{:?}", g.motion.problem);
                            if g.motion.status == MotionStatus::Predicted {
                                assert_eq!(g.motion.dispatched_ticks, 2);
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                }
            }
            assert!(client.survival().resume_ground(record.id).await.is_err());
            assert!(packets.try_recv().is_err());
            let _ = release.send(());
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn common_dismount_ground_retains_received_pose_and_continues_both_modes() {
        use api::{GameMode, VehicleRelation};
        for mode in [GameMode::Survival, GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.survival.write().await.game_mode =
                Some(if mode == GameMode::Survival { 0 } else { 1 });
            bot.player.lock().await.entity_id = Some(42);
            let client = crate::Client::from_java_1_16_1(bot.clone());
            bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
            let VehicleRelation::Mounted { mount } = client
                .vehicle_state()
                .await
                .unwrap()
                .relation
                .unwrap()
                .value
            else {
                panic!();
            };
            let record = if mode == GameMode::Survival {
                client.survival().dismount(mount).await.unwrap()
            } else {
                client.creative().dismount(mount).await.unwrap()
            };
            assert_eq!(packets.recv().await.unwrap().0, 0x1d);
            // Actual dismount position can precede the actual absence packet.
            let mut position = Vec::new();
            for value in [8.5f64, 65., 8.5] {
                position.extend(value.to_be_bytes());
            }
            position.extend([0; 8]);
            position.push(0);
            put_varint(&mut position, 99);
            bot.apply_packet(0x35, position).await.unwrap();
            assert_eq!(packets.recv().await.unwrap(), (0x00, vec![99]));
            assert_eq!(packets.recv().await.unwrap().0, 0x13);
            assert!(client.survival().resume_ground(record.id).await.is_err());
            assert!(
                client
                    .dismount_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .grounding
                    .is_none()
            );
            bot.apply_packet(0x4b, vec![10, 0]).await.unwrap();
            if mode == GameMode::Survival {
                client
                    .survival()
                    .complete_dismount(record.id)
                    .await
                    .unwrap();
            } else {
                client
                    .creative()
                    .complete_dismount(record.id)
                    .await
                    .unwrap();
            }
            assert_eq!(packets.recv().await.unwrap(), (0x1d, vec![0; 9]));
            crate::client::tests::common_dismount_ground_scenario(&client, record.id).await;
            bot.apply_packet(0x37, vec![1, 10]).await.unwrap();
            assert!(client.vehicle_state().await.unwrap().relation.is_none());
            let controls = [api::survival::SurvivalControl {
                yaw: 0.,
                input: Default::default(),
            }];
            let preview = if mode == GameMode::Survival {
                client.survival().preview_path(&controls).await
            } else {
                client.creative().preview_path(&controls).await
            };
            assert!(
                preview.is_ok(),
                "ground stop must survive retired vehicle: {preview:?}"
            );
            crate::client::tests::common_retired_vehicle_ground_scenario(&client).await;
            for _ in 0..6 {
                assert_eq!(packets.recv().await.unwrap().0, 0x13);
            }
            assert!(packets.try_recv().is_err());
            release.send(()).unwrap();
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn common_dismount_requires_receipt_before_release_and_never_replays() {
        use api::vehicle::{DismountStage, VehicleRelation};
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        let VehicleRelation::Mounted { mount } = client
            .vehicle_state()
            .await
            .unwrap()
            .relation
            .unwrap()
            .value
        else {
            panic!()
        };
        assert!(client.creative().dismount(mount).await.is_err());
        let record = client.survival().dismount(mount).await.unwrap();
        assert_eq!(record.stage, DismountStage::Submitted);
        assert_eq!(
            packets.recv().await.unwrap(),
            (0x1d, vec![0, 0, 0, 0, 0, 0, 0, 0, 2])
        );
        assert!(client.survival().dismount(mount).await.is_err());
        assert!(
            client
                .survival()
                .complete_dismount(record.id)
                .await
                .is_err()
        );
        assert!(
            !client
                .dismount_record()
                .await
                .unwrap()
                .unwrap()
                .release_claimed
        );
        bot.apply_packet(0x4b, vec![10, 2, 43, 42]).await.unwrap();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::Submitted
        );
        bot.apply_packet(0x4b, vec![11, 0]).await.unwrap();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::Submitted
        );
        bot.apply_packet(0x4b, vec![10, 1, 43]).await.unwrap();
        assert_eq!(
            client.dismount_record().await.unwrap().unwrap().stage,
            DismountStage::ObservedUnmounted
        );
        assert!(
            client
                .creative()
                .complete_dismount(record.id)
                .await
                .is_err()
        );
        let completed = client
            .survival()
            .complete_dismount(record.id)
            .await
            .unwrap();
        assert_eq!(completed.stage, DismountStage::Completed);
        assert!(completed.observed_unmounted.is_some());
        assert_eq!(packets.recv().await.unwrap(), (0x1d, vec![0; 9]));
        assert!(
            client
                .survival()
                .complete_dismount(record.id)
                .await
                .is_err()
        );
        assert!(client.survival().dismount(mount).await.is_err());
        assert!(
            timeout(Duration::from_millis(30), packets.recv())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_vehicle_control_waiter_keeps_finite_owner_and_final_neutral() {
        use api::{GameMode, VehicleControlStage, VehicleInput, VehicleRelation};
        for mode in [GameMode::Survival, GameMode::Creative] {
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            bot.survival.write().await.game_mode =
                Some(if mode == GameMode::Survival { 0 } else { 1 });
            bot.player.lock().await.entity_id = Some(42);
            let client = crate::Client::from_java_1_16_1(bot.clone());
            bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
            let VehicleRelation::Mounted { mount } = client
                .vehicle_state()
                .await
                .unwrap()
                .relation
                .unwrap()
                .value
            else {
                panic!()
            };
            let inputs = [
                VehicleInput {
                    forward: 1,
                    ..Default::default()
                },
                VehicleInput {
                    forward: 1,
                    ..Default::default()
                },
                VehicleInput::default(),
            ];
            let writer = bot.writer.lock().await;
            let ops = client.survival();
            let creative = client.creative();
            let mut attempt = Box::pin(async {
                if mode == GameMode::Survival {
                    ops.start_vehicle_control(mount, &inputs).await
                } else {
                    creative.start_vehicle_control(mount, &inputs).await
                }
            });
            assert!(
                timeout(Duration::from_millis(20), attempt.as_mut())
                    .await
                    .is_err()
            );
            drop(attempt);
            let pending = timeout(Duration::from_millis(50), client.vehicle_control_record())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(pending.stage, VehicleControlStage::Running);
            assert_eq!(pending.attempted_tick, 1);
            assert_eq!(pending.dispatched_ticks, 0);
            drop(writer);
            let forward = (0x1d, vec![0, 0, 0, 0, 63, 128, 0, 0, 0]);
            assert_eq!(packets.recv().await.unwrap(), forward);
            let busy = if mode == GameMode::Survival {
                client.survival().dismount(mount).await
            } else {
                client.creative().dismount(mount).await
            };
            assert!(busy.is_err());
            assert_eq!(packets.recv().await.unwrap(), forward);
            assert_eq!(packets.recv().await.unwrap(), (0x1d, vec![0; 9]));
            let record = timeout(Duration::from_secs(1), async {
                loop {
                    let r = client.vehicle_control_record().await.unwrap().unwrap();
                    if r.stage == VehicleControlStage::Submitted {
                        break r;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(record.id, pending.id);
            assert_eq!((record.attempted_tick, record.dispatched_ticks), (3, 3));
            assert!(
                timeout(Duration::from_millis(20), packets.recv())
                    .await
                    .is_err()
            );
            let _ = client.revoke_connection();
            assert_eq!(
                client
                    .vehicle_control_record()
                    .await
                    .unwrap()
                    .unwrap()
                    .stage,
                VehicleControlStage::Submitted
            );
            drop(release);
            drop(client);
            drop(bot);
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn vehicle_control_cannot_retire_partially_sent_ground_run() {
        use api::{
            VehicleInput, VehicleRelation,
            survival::{MotionStatus, SurvivalControl},
        };
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let controls = [SurvivalControl {
            yaw: 0.0,
            input: Default::default(),
        }; 20];
        let started = client
            .survival()
            .start_predicted_path(&controls)
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap()
                .0,
            0x13
        );
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        let VehicleRelation::Mounted { mount } = client
            .vehicle_state()
            .await
            .unwrap()
            .relation
            .unwrap()
            .value
        else {
            panic!()
        };
        assert!(
            client
                .survival()
                .start_vehicle_control(mount, &[VehicleInput::default()])
                .await
                .is_err()
        );
        let retained = timeout(Duration::from_secs(1), async {
            loop {
                let record = client.survival().motion_record().await.unwrap().unwrap();
                if record.status != MotionStatus::Running {
                    break record;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(retained.run_id, started.run_id);
        assert_eq!(retained.dispatched_ticks, 1);
        assert_eq!(retained.status, MotionStatus::RequiresInspection);
        assert!(client.vehicle_control_record().await.unwrap().is_none());
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        let _ = client.revoke_connection();
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn vehicle_control_revocation_while_writer_waits_preserves_first_failure() {
        use api::{VehicleControlStage, VehicleInput, VehicleRelation};
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        let VehicleRelation::Mounted { mount } = client
            .vehicle_state()
            .await
            .unwrap()
            .relation
            .unwrap()
            .value
        else {
            panic!()
        };
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let inputs = [VehicleInput::default()];
        let mut attempt = Box::pin(ops.start_vehicle_control(mount, &inputs));
        assert!(
            timeout(Duration::from_millis(20), attempt.as_mut())
                .await
                .is_err()
        );
        let before = timeout(Duration::from_millis(50), client.vehicle_control_record())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!((before.attempted_tick, before.dispatched_ticks), (1, 0));
        let _ = client.revoke_connection();
        let revoked = timeout(Duration::from_millis(50), client.vehicle_control_record())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(revoked.stage, VehicleControlStage::RequiresInspection);
        let first = revoked.requires_inspection.clone();
        assert!(first.is_some());
        drop(writer);
        assert!(
            timeout(Duration::from_secs(1), attempt.as_mut())
                .await
                .unwrap()
                .is_err()
        );
        let after = client.vehicle_control_record().await.unwrap().unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.stage, VehicleControlStage::RequiresInspection);
        assert_eq!(after.requires_inspection, first);
        assert_eq!(after.dispatched_ticks, 0);
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .ok()
                .flatten()
                .is_none()
        );
        drop(attempt);
        drop(ops);
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn vehicle_control_latches_unmount_before_same_numeric_vehicle_reappears() {
        use api::{VehicleControlStage, VehicleInput, VehicleRelation};
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        let VehicleRelation::Mounted { mount } = client
            .vehicle_state()
            .await
            .unwrap()
            .relation
            .unwrap()
            .value
        else {
            panic!()
        };
        let ops = client.survival();
        let waiter = tokio::spawn(async move {
            ops.start_vehicle_control(
                mount,
                &[
                    VehicleInput {
                        forward: 1,
                        ..Default::default()
                    },
                    VehicleInput {
                        forward: 1,
                        ..Default::default()
                    },
                    VehicleInput::default(),
                ],
            )
            .await
        });
        assert_eq!(
            packets.recv().await.unwrap(),
            (0x1d, vec![0, 0, 0, 0, 63, 128, 0, 0, 0])
        );
        bot.apply_packet(0x4b, vec![10, 0]).await.unwrap();
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        assert!(waiter.await.unwrap().is_err());
        let record = client.vehicle_control_record().await.unwrap().unwrap();
        assert_eq!(record.stage, VehicleControlStage::RequiresInspection);
        assert_eq!(record.dispatched_ticks, 1);
        assert!(
            client
                .survival()
                .start_vehicle_control(mount, &[VehicleInput::default()])
                .await
                .is_err()
        );
        assert!(client.survival().dismount(mount).await.is_err());
        assert!(
            timeout(Duration::from_millis(50), packets.recv())
                .await
                .is_err()
        );
        let _ = client.revoke_connection();
        assert_eq!(
            client
                .vehicle_control_record()
                .await
                .unwrap()
                .unwrap()
                .requires_inspection,
            record.requires_inspection
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_vehicle_receipts_refuse_stale_ground_authority() {
        use api::vehicle::VehicleRelation;
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let controls = [api::survival::SurvivalControl {
            yaw: 0.0,
            input: Default::default(),
        }];
        client.survival().preview_path(&controls).await.unwrap();
        assert!(client.vehicle_state().await.unwrap().relation.is_none());
        bot.apply_packet(0x4b, vec![10, 1, 42]).await.unwrap();
        let mounted = client.vehicle_state().await.unwrap();
        let VehicleRelation::Mounted { mount } = mounted.relation.as_ref().unwrap().value else {
            panic!()
        };
        assert_eq!(mount.session(), mounted.session);
        assert_eq!(mount.native_vehicle_id(), 10);
        assert!(mount.vehicle().is_none());
        assert!(client.survival().preview_path(&controls).await.is_err());
        for payload in [vec![10, 0, 0], vec![10, 2, 42, 42]] {
            assert!(bot.apply_packet(0x4b, payload).await.is_err());
            assert_eq!(
                client
                    .vehicle_state()
                    .await
                    .unwrap()
                    .relation
                    .unwrap()
                    .value,
                VehicleRelation::Mounted { mount }
            );
        }
        bot.apply_packet(0x4b, vec![11, 0]).await.unwrap();
        assert_eq!(
            client
                .vehicle_state()
                .await
                .unwrap()
                .relation
                .unwrap()
                .value,
            VehicleRelation::Mounted { mount }
        );
        bot.apply_packet(0x4b, vec![10, 0]).await.unwrap();
        let unmounted = client.vehicle_state().await.unwrap();
        assert_eq!(
            unmounted.relation.unwrap().value,
            VehicleRelation::Unmounted {
                previous_mount: mount
            }
        );
        assert!(unmounted.passengers.unwrap().value.is_empty());
        // Receipt of absence and zero local motion are not a new standing basis.
        assert!(client.survival().preview_path(&controls).await.is_err());
        bot.apply_packet(0x37, vec![1, 10]).await.unwrap();
        assert!(client.vehicle_state().await.unwrap().relation.is_none());
        assert!(client.survival().preview_path(&controls).await.is_err());
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_entity_lifetime_rejects_reused_id_and_wrong_mode() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        bot.survival.write().await.game_mode = Some(0);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let mut spawn = vec![42];
        spawn.extend([7; 16]);
        put_varint(
            &mut spawn,
            client
                .registry()
                .builtin_id("minecraft:entity_type", "minecraft:sheep")
                .unwrap()
                .value(),
        );
        for v in [2.0f64, 65.0, 0.5] {
            spawn.extend(v.to_be_bytes());
        }
        spawn.extend([0; 9]); // native living yaw/pitch/head yaw + three short velocities.
        bot.apply_packet(0x02, spawn.clone()).await.unwrap();
        let received = client.entity_spawns().await.unwrap();
        let target = received.entities[0].id;
        assert_eq!(
            received.entities[0].type_name.as_deref(),
            Some("minecraft:sheep")
        );
        let initial = client.entity_motion(target).await.unwrap();
        assert!(initial.on_ground.is_none());
        assert_eq!(initial.velocity.as_ref().unwrap().value, [0.0; 3]);
        let mut relative = vec![42];
        for value in [4096i16, 0, 0] {
            relative.extend(value.to_be_bytes());
        }
        relative.push(1);
        bot.apply_packet(0x28, relative.clone()).await.unwrap();
        let moved = client.entity_motion(target).await.unwrap();
        assert_eq!(
            moved.position.as_ref().unwrap().value.position,
            [3.0, 65.0, 0.5]
        );
        assert_eq!(moved.entity.spawn_position, initial.entity.spawn_position);
        assert_eq!(moved.velocity, initial.velocity);
        let stored = serde_json::to_value(&moved).unwrap();
        relative.push(0);
        assert!(bot.apply_packet(0x28, relative).await.is_err());
        let after = client.entity_motion(target).await.unwrap();
        assert_eq!(
            serde_json::to_value(&after.position).unwrap(),
            stored["position"]
        );
        bot.apply_packet(0x2b, vec![42]).await.unwrap();
        assert_eq!(
            client.entity_motion(target).await.unwrap().on_ground,
            moved.on_ground
        );
        assert!(
            client
                .creative()
                .attack_entity(target, false)
                .await
                .is_err()
        );
        let dispatch = client
            .survival()
            .attack_entity(target, false)
            .await
            .unwrap();
        assert_eq!(dispatch.connection_id, received.session.connection_id);
        assert_eq!(dispatch.interaction_sequence, None);
        bot.apply_packet(0x37, vec![1, 42]).await.unwrap();
        assert!(client.entity_spawns().await.unwrap().entities.is_empty());
        assert!(client.entity_motion(target).await.is_err());
        assert!(
            client
                .survival()
                .attack_entity(target, false)
                .await
                .is_err()
        );
        bot.apply_packet(0x02, spawn).await.unwrap();
        let next = client.entity_spawns().await.unwrap().entities[0].id;
        assert_ne!(target, next);
        assert!(
            client
                .survival()
                .interact_entity(target, api::Hand::Main, false)
                .await
                .is_err()
        );
        client
            .survival()
            .interact_entity(next, api::Hand::Main, false)
            .await
            .unwrap();
        // Before actor I/O cancellation, retain uncertainty and never silently replay.
        let writer = bot.writer.lock().await;
        let ops = client.survival();
        let mut attempt = Box::pin(ops.attack_entity(next, false));
        assert!(
            timeout(Duration::from_millis(10), attempt.as_mut())
                .await
                .is_err()
        );
        drop(attempt);
        drop(writer);
        assert!(client.player_state().await.unwrap().pending_dispatch);
        assert!(ops.attack_entity(next, false).await.is_err());
        release.send(()).unwrap();
        drop(ops);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn common_server_registries_keep_original_join_codec_and_replace_actual_tags() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        assert!(!client.server_registry_state().await.unwrap().complete());
        let mut join = vec![0; 4];
        join.extend([0, 255, 0]);
        let codec = vec![10, 0, 0, 3, 0, 1, b'x', 0, 0, 0, 7, 0];
        join.extend(&codec);
        put_string(&mut join, "minecraft:overworld");
        put_string(&mut join, "minecraft:overworld");
        bot.apply_packet(0x25, join).await.unwrap();
        let before = client.server_registry_state().await.unwrap();
        assert!(before.complete());
        assert!(before.registries().is_empty());
        assert_eq!(&**before.legacy_codec().unwrap().value, &codec);
        assert_eq!(
            before.legacy_codec().unwrap().source,
            api::ValueSource::Received { sequence: 1 }
        );
        assert!(before.tags().is_none());
        let mut tags = vec![0, 1];
        put_string(&mut tags, "example:tag");
        tags.extend([1, 3, 0, 0]);
        bot.apply_packet(0x5b, tags).await.unwrap();
        let captured = client.server_registry_state().await.unwrap();
        assert_eq!(
            captured.tags().unwrap().value["minecraft:item"]["example:tag"],
            [3]
        );
        assert_eq!(
            captured.tags().unwrap().source,
            api::ValueSource::Received { sequence: 2 }
        );
        assert_eq!(captured.stamp(), before.stamp());
        assert!(captured.bind("minecraft:item", 3).is_err());
        let mut respawn = Vec::new();
        put_string(&mut respawn, "minecraft:overworld");
        put_string(&mut respawn, "minecraft:overworld");
        respawn.extend([0; 8]);
        respawn.extend([0, 255, 0, 0, 1]);
        bot.apply_packet(0x3a, respawn).await.unwrap();
        let respawned = client.server_registry_state().await.unwrap();
        assert_eq!(respawned.stamp(), captured.stamp());
        assert_ne!(
            respawned.session().world_generation,
            captured.session().world_generation
        );
        assert_eq!(respawned.tags(), captured.tags());
        bot.apply_packet(0x5b, vec![0; 4]).await.unwrap();
        assert!(
            client
                .server_registry_state()
                .await
                .unwrap()
                .tags()
                .unwrap()
                .value
                .values()
                .all(std::collections::BTreeMap::is_empty)
        );
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_container_receipts_keep_opening_identity_and_ignore_cache_predictions() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        assert!(client.screen_state().await.unwrap().screen.is_none());
        let mut open = vec![3, 2];
        put_string(&mut open, "{\"text\":\"Storage\"}");
        bot.apply_packet(0x2e, open.clone()).await.unwrap();
        let before = crate::client::tests::common_container_capture_scenario(&client, false).await;
        let stone = ItemStack {
            item_id: crate::item_id("stone").unwrap(),
            count: 3,
            nbt: None,
        };
        let dirt = ItemStack {
            item_id: crate::item_id("dirt").unwrap(),
            count: 2,
            nbt: None,
        };
        let mut full = vec![3];
        full.extend(63i16.to_be_bytes());
        for index in 0..63 {
            write_slot(
                &mut full,
                match index {
                    0 => Some(&stone),
                    27 => Some(&dirt),
                    _ => None,
                },
            );
        }
        bot.apply_packet(0x14, full).await.unwrap();
        // A full legacy content packet does not contain a cursor.
        assert!(client.screen_state().await.unwrap().cursor.is_none());
        bot.apply_packet(0x16, vec![255, 255, 255, 0])
            .await
            .unwrap();
        assert_eq!(
            crate::client::tests::common_container_capture_scenario(&client, true).await,
            before
        );
        bot.inventory.write().await.windows.get_mut(&3).unwrap()[0] = None;
        crate::client::tests::common_container_capture_scenario(&client, true).await;
        let received = client.screen_state().await.unwrap();
        assert!(received.screen.as_ref().unwrap().revision.is_none());
        bot.apply_packet(0x2e, open).await.unwrap();
        let reopened =
            crate::client::tests::common_container_capture_scenario(&client, false).await;
        assert_ne!(reopened, before);
        let mut stale = vec![4];
        stale.extend(0i16.to_be_bytes());
        write_slot(&mut stale, Some(&stone));
        bot.apply_packet(0x16, stale).await.unwrap();
        crate::client::tests::common_container_capture_scenario(&client, false).await;
        bot.apply_packet(0x13, vec![4]).await.unwrap();
        assert_eq!(
            client.screen_state().await.unwrap().screen.unwrap().id,
            reopened
        );
        bot.apply_packet(0x13, vec![3]).await.unwrap();
        assert!(client.screen_state().await.unwrap().screen.is_none());
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn own_living_flags_report_the_item_in_use() {
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        bot.player.lock().await.entity_id = Some(42);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        assert_eq!(client.player_state().await.unwrap().using_item, None);
        let flags = |value: u8| {
            let mut packet = Vec::new();
            put_varint(&mut packet, 42);
            // Living flags: index 7, byte serializer 0.
            packet.extend([7, 0, value, 255]);
            packet
        };
        bot.apply_packet(0x44, flags(3)).await.unwrap();
        let using = client.player_state().await.unwrap().using_item.unwrap();
        assert_eq!(using.value, Some(api::Hand::Off));
        assert!(matches!(using.source, api::ValueSource::Received { .. }));
        bot.apply_packet(0x44, flags(0)).await.unwrap();
        let using = client.player_state().await.unwrap().using_item.unwrap();
        assert_eq!(using.value, None);
        // Attributes (renamed to 1.21.11 keys), effects and air supply.
        let mut attributes = Vec::new();
        put_varint(&mut attributes, 42);
        attributes.extend(1i32.to_be_bytes());
        crate::protocol::put_string(&mut attributes, "minecraft:generic.attack_speed");
        attributes.extend(4.0f64.to_be_bytes());
        put_varint(&mut attributes, 1);
        attributes.extend([7; 16]);
        attributes.extend((-1.6f64).to_be_bytes());
        attributes.push(0);
        bot.apply_packet(0x58, attributes).await.unwrap();
        let mut effect = Vec::new();
        put_varint(&mut effect, 42);
        effect.extend([1, 1]); // speed, amplifier 1
        put_varint(&mut effect, 200);
        effect.push(2);
        bot.apply_packet(0x59, effect).await.unwrap();
        let mut air = Vec::new();
        put_varint(&mut air, 42);
        air.extend([1, 1, 120, 255]); // index 1, VarInt, 120 ticks
        bot.apply_packet(0x44, air).await.unwrap();
        let state = client.player_state().await.unwrap();
        assert_eq!(state.entity_id, Some(42));
        let speed = &state.attributes["minecraft:attack_speed"].value;
        assert_eq!((speed.base, speed.value), (4.0, 4.0 - 1.6));
        assert_eq!(
            speed.modifiers[0].id,
            "07070707-0707-0707-0707-070707070707"
        );
        let speed_effect = state.effects["minecraft:speed"].value;
        assert_eq!(
            (
                speed_effect.amplifier,
                speed_effect.duration_at_receipt,
                speed_effect.visible
            ),
            (1, 200, true)
        );
        assert_eq!(state.air_supply.unwrap().value, 120);
        let mut removal = Vec::new();
        put_varint(&mut removal, 42);
        removal.push(1);
        bot.apply_packet(0x38, removal).await.unwrap();
        assert!(client.player_state().await.unwrap().effects.is_empty());
        // Another entity's flags are not the local player's.
        let mut other = Vec::new();
        put_varint(&mut other, 43);
        other.extend([7, 0, 1, 255]);
        bot.apply_packet(0x44, other).await.unwrap();
        assert_eq!(
            client
                .player_state()
                .await
                .unwrap()
                .using_item
                .unwrap()
                .value,
            None
        );
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_creative_contract_dispatches_legacy_packets_without_inventory_echo() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        bot.survival.write().await.game_mode = Some(1);
        let mut abilities = vec![4];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        bot.apply_packet(0x31, abilities).await.unwrap();
        {
            let mut player = bot.player.lock().await;
            player.x = 0.5;
            player.y = 1.0;
            player.z = 0.5;
        }
        bot.common_receipts.lock().await.pose = Some(api::ReceivedPose {
            position: [0.5, 1.0, 0.5],
            rotation: [0.0; 2],
            receive_sequence: 1,
        });
        bot.world.lock().await.apply_chunk(&[0; 14], 256).unwrap();
        bot.world
            .lock()
            .await
            .set_block_for_test(BlockPos { x: 0, y: 0, z: 1 }, 1);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        crate::client::tests::common_creative_scenario(&client).await;
        let mut emitted = Vec::new();
        loop {
            let packet = timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap();
            if packet.0 == 0x27 {
                emitted.push(packet);
                break;
            }
        }
        for _ in 0..9 {
            emitted.push(
                timeout(Duration::from_secs(1), packets.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        // Independent protocol metadata, pinned separately from the Rust encoder.
        let wire: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/java_1_16_1_wire.json"
        ))
        .unwrap();
        let expected = [
            "set_creative_slot",
            "position_look",
            "held_item_slot",
            "abilities",
            "position_look",
            "block_dig",
            "block_place",
        ]
        .map(|name| wire["packets"][name]["id"].as_i64().unwrap() as i32);
        assert_eq!(
            emitted[..7].iter().map(|p| p.0).collect::<Vec<_>>(),
            expected
        );
        // Off hand, target, face UP (varint), cursor, not inside.
        let mut place = vec![1];
        place.extend(BlockPos { x: 0, y: 0, z: 1 }.packed().to_be_bytes());
        place.push(1);
        for v in [0.5f32; 3] {
            place.extend(v.to_be_bytes());
        }
        place.push(0);
        assert_eq!(emitted[6].1, place);
        // ServerboundUseItemPacket (0x2e): off hand.
        assert_eq!(emitted[7], (0x2e, vec![1]));
        // PLAYER_ACTION (block_dig) RELEASE_USE_ITEM, BlockPos.ZERO, Direction.DOWN.
        assert_eq!(
            emitted[8],
            (expected[5], vec![5, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        // ServerboundSwingPacket (0x2b): main hand.
        assert_eq!(emitted[9], (0x2b, vec![0]));
        assert_eq!(&emitted[0].1[..2], &36i16.to_be_bytes());
        let item = read_slot(&mut &emitted[0].1[2..]).unwrap().unwrap();
        assert_eq!(item.name(), Some("stone"));
        assert_eq!(item.count, 1);
        assert_eq!(emitted[3].1, [2]);
        assert_eq!(emitted[4].1.last(), Some(&0));
        // A new packet establishes inventory, rather than the creative write.
        let mut slot = vec![0];
        slot.extend(36i16.to_be_bytes());
        write_slot(&mut slot, Some(&item));
        bot.apply_packet(0x16, slot).await.unwrap();
        let state = client.player_state().await.unwrap();
        assert!(
            matches!(&state.inventory.slots[36], Some(value) if matches!(&value.value, api::SlotKnowledge::Item { item } if item.name == "minecraft:stone"))
        );
        drop(release);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_observation_preserves_receipts_across_cache_predictions() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        let item = ItemStack {
            item_id: crate::item_id("stone").unwrap(),
            count: 2,
            nbt: None,
        };
        let mut payload = vec![0];
        payload.extend(46i16.to_be_bytes());
        for index in 0..46 {
            write_slot(&mut payload, (index == 9).then_some(&item));
        }
        bot.apply_packet(0x14, payload).await.unwrap();
        bot.inventory.write().await.windows.get_mut(&0).unwrap()[9] = None;
        let state = bot.player_state().await.unwrap();
        assert!(
            matches!(&state.inventory.slots[9], Some(value) if matches!(value.value, api::SlotKnowledge::Item { .. }))
        );
        assert_eq!(
            state.inventory.local_cache.unwrap()[9],
            api::SlotKnowledge::Empty
        );
        assert!(state.inventory.cursor.is_none());
        assert!(state.health.is_none());
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let receipts = client.received_inventory().await.unwrap();
        let received = receipts.slot(9).unwrap().unwrap().item().unwrap();
        assert_eq!(received.stack().name, "minecraft:stone");
        assert_eq!(received.stack().count, 2);
        assert!(received.native_equivalent(&received).unwrap());
        assert_eq!(received.registry_state().session(), receipts.session());
        assert!(received.receive_sequence() <= receipts.receive_sequence());
        assert!(receipts.cursor().is_none());
        assert!(receipts.slot(10).unwrap().unwrap().item().is_none());
        assert!(receipts.slot(46).is_err());
        bot.common_receipts.lock().await.inventory.slots[9] = None;
        assert_eq!(received.stack().count, 2);
        assert!(
            client
                .received_inventory()
                .await
                .unwrap()
                .slot(9)
                .unwrap()
                .is_none()
        );
        drop(client);
        release.send(()).unwrap();
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn cancelled_common_dispatch_cannot_be_replayed() {
        let (bot, server, release) =
            super::super::tests::ready_test_bot(ConnectionOptions::default()).await;
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        bot.survival.write().await.game_mode = Some(1);
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let writer = bot.writer.lock().await;
        let ops = client.creative();
        let mut attempt = Box::pin(ops.set_hotbar(0, Some(("minecraft:stone", 1))));
        assert!(
            timeout(Duration::from_millis(10), attempt.as_mut())
                .await
                .is_err()
        );
        drop(attempt);
        drop(writer);
        assert!(client.player_state().await.unwrap().pending_dispatch);
        assert!(ops.select_hotbar(0).await.is_err());
        release.send(()).unwrap();
        drop(ops);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn cancelled_common_flight_wait_keeps_one_owner_and_nonblocking_record() {
        use api::{FlightCommand, FlightStage};
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.survival.write().await.game_mode = Some(1);
        let mut abilities = vec![4];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        bot.apply_packet(0x31, abilities).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let writer = bot.writer.lock().await;
        let ops = client.creative();
        let mut waiter = Box::pin(ops.set_flying(true));
        assert!(
            timeout(Duration::from_millis(20), waiter.as_mut())
                .await
                .is_err()
        );
        drop(waiter);
        let pending = client.flight_record().unwrap();
        assert_eq!(pending.stage, FlightStage::Prepared);
        assert_eq!(pending.command, FlightCommand::SetFlying { flying: true });
        assert!(!pending.dispatched);
        assert_eq!(client.flight_record().unwrap().attempt, pending.attempt);
        drop(writer);
        assert_eq!(
            timeout(Duration::from_secs(1), packets.recv())
                .await
                .unwrap()
                .unwrap(),
            (0x1a, vec![2])
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while client.flight_record().unwrap().stage != FlightStage::Submitted {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(client.flight_record().unwrap().dispatched);
        assert_eq!(client.flight_record().unwrap().attempt, pending.attempt);
        ops.move_flying([8.5, 67.0, 8.5], [10.0, 0.0])
            .await
            .unwrap();
        assert_eq!(packets.recv().await.unwrap().0, 0x13);
        assert_eq!(client.flight_record().unwrap().attempt, pending.attempt + 1);
        assert!(
            timeout(Duration::from_millis(20), packets.recv())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        drop(ops);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_landing_retains_received_flight_flag_and_ground_continuation() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.survival.write().await.game_mode = Some(1);
        let mut abilities = vec![6];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        bot.apply_packet(0x31, abilities.clone()).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        crate::client::tests::common_creative_landing_scenario(&client).await;
        let mut frames = Vec::new();
        while let Ok(Some(f)) = timeout(Duration::from_millis(20), packets.recv()).await {
            frames.push(f);
        }
        assert_eq!(
            frames
                .iter()
                .filter(|f| f.0 == 0x1a)
                .map(|f| f.1.clone())
                .collect::<Vec<_>>(),
            vec![vec![2], vec![0]]
        );
        assert_eq!(frames.iter().filter(|f| f.0 == 0x13).count(), 6);
        assert_eq!(
            frames
                .iter()
                .filter(|f| f.0 == 0x1d)
                .map(|f| f.1.clone())
                .collect::<Vec<_>>(),
            vec![vec![0; 9]]
        );
        assert!(bot.survival.read().await.flying);
        assert!(!bot.common_receipts.lock().await.requested_flying);
        bot.apply_packet(0x31, abilities).await.unwrap();
        assert!(
            client
                .creative()
                .preview_path(&[api::survival::SurvivalControl {
                    yaw: 0.,
                    input: Default::default()
                }])
                .await
                .is_err()
        );
        release.send(()).unwrap();
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn cancelled_common_landing_wait_retains_disable_neutral_and_two_ground_ticks() {
        let (bot, mut packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        bot.survival.write().await.game_mode = Some(1);
        let mut abilities = vec![4];
        abilities.extend(0.05f32.to_be_bytes());
        abilities.extend(0.1f32.to_be_bytes());
        bot.apply_packet(0x31, abilities).await.unwrap();
        let client = crate::Client::from_java_1_16_1(bot.clone());
        let ops = client.creative();
        let position = client.player_state().await.unwrap().position.unwrap().value;
        ops.set_flying(true).await.unwrap();
        ops.move_flying(position, [0.; 2]).await.unwrap();
        packets.recv().await.unwrap();
        packets.recv().await.unwrap();
        let writer = bot.writer.lock().await;
        let mut wait = Box::pin(ops.land());
        assert!(
            timeout(Duration::from_millis(20), wait.as_mut())
                .await
                .is_err()
        );
        drop(wait);
        let pending = client.flight_record().unwrap();
        assert_eq!(pending.command, api::FlightCommand::Land);
        assert_eq!(pending.stage, api::FlightStage::Prepared);
        assert!(!pending.landing.as_ref().unwrap().disable_dispatched);
        drop(writer);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let r = client.flight_record().unwrap();
                assert!(r.requires_inspection.is_none());
                if r.stage == api::FlightStage::Submitted {
                    let l = r.landing.unwrap();
                    assert!(l.disable_dispatched && l.neutral_dispatched);
                    assert_eq!(l.motion.dispatched_ticks, 2);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut frames = Vec::new();
        while let Ok(Some(f)) = timeout(Duration::from_millis(20), packets.recv()).await {
            frames.push(f);
        }
        assert_eq!(
            frames
                .iter()
                .filter(|f| f.0 == 0x1a)
                .map(|f| f.1.clone())
                .collect::<Vec<_>>(),
            vec![vec![0]]
        );
        assert_eq!(frames.iter().filter(|f| f.0 == 0x13).count(), 2);
        assert!(ops.land().await.is_err());
        release.send(()).unwrap();
        drop(ops);
        drop(client);
        drop(bot);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn social_common_bridge_applies_every_original_team_and_player_info_packet() {
        use crate::client::ui::social_tests::{assert_bridge, bridge_bytes, bridge_cases};
        let (bot, _packets, release, server) =
            super::super::tests::operation_test_bot(0x7fff, 0, vec![]).await;
        super::super::common_motion::tests::seed_motion(&bot).await;
        let client = crate::Client::from_java_1_16_1(bot.clone());
        assert!(client.teams().await.unwrap().last_update_sequence.is_none());
        assert!(
            client
                .player_list()
                .await
                .unwrap()
                .last_update_sequence
                .is_none()
        );
        for row in bridge_cases(crate::MinecraftVersion::Java1_16_1) {
            bot.apply_packet(
                row["packet_id"].as_i64().unwrap() as i32,
                bridge_bytes(&row),
            )
            .await
            .unwrap();
            assert_bridge(&client, &row).await;
        }
        client.disconnect().await.unwrap();
        assert!(client.teams().await.is_err() && client.player_list().await.is_err());
        let _ = release.send(());
        server.await.unwrap();
    }
    #[tokio::test]
    async fn common_respawn_records_native_request_and_new_world_without_retry() {
        for mode in [api::GameMode::Survival, api::GameMode::Creative] {
            let mut spawn = Vec::new();
            put_string(&mut spawn, "minecraft:overworld");
            put_string(&mut spawn, "world");
            spawn.extend(0i64.to_be_bytes());
            spawn.extend([
                if mode == api::GameMode::Creative {
                    1
                } else {
                    0
                },
                0,
                0,
                0,
                0,
            ]);
            let (bot, mut packets, release, server) =
                super::super::tests::operation_test_bot(0x04, 0x3a, spawn).await;
            super::super::common_motion::tests::seed_motion(&bot).await;
            let client = crate::Client::from_java_1_16_1(bot.clone());
            assert!(client.respawn().await.is_err());
            let mut health = 0f32.to_be_bytes().to_vec();
            health.push(20);
            health.extend(5f32.to_be_bytes());
            bot.apply_packet(0x49, health).await.unwrap();
            let sent = client.respawn().await.unwrap();
            assert!(sent.dispatched);
            assert_eq!(sent.stage, api::RespawnStage::Submitted);
            assert_eq!(packets.recv().await.unwrap(), (0x04, vec![0]));
            assert!(client.clone().respawn().await.is_err());
            release.send(()).unwrap();
            timeout(Duration::from_secs(2), async {
                while client.respawn_record().unwrap().received_spawn.is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let fresh = client.respawn_record().unwrap();
            assert_eq!(fresh.stage, api::RespawnStage::RespawnReceived);
            assert!(
                fresh.received_spawn.unwrap().value.world_generation
                    > sent.session.world_generation
            );
            assert!(sent.received_spawn.is_none());
            assert!(client.respawn().await.is_err());
            bot.disconnect().await.unwrap();
            assert!(client.respawn_record().unwrap().dispatched);
            server.await.unwrap();
        }
    }
}
