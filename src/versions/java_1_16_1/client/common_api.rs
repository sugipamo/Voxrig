//! Translation into the common Client contract, with native actor-owned sends.
use super::*;
use crate::client::{self as api, operations::Action};

impl Bot {
    pub(crate) async fn common_connection_identity(&self) -> Result<api::ConnectionIdentity> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        Ok(api::ConnectionIdentity {
            session: player.session,
            uuid: self.login_profile.uuid,
            name: self.login_profile.name.clone(),
        })
    }

    pub(crate) async fn common_entity_spawns(&self) -> Result<api::EntitySpawns> {
        let _gate = self.coherent_state_gate.lock().await;
        let player = self.common_player_unlocked().await?;
        Ok(self
            .common_receipts
            .lock()
            .await
            .entities
            .capture(player.session, player.receive_sequence))
    }
    pub(crate) async fn common_vehicle_state(&self) -> Result<api::VehicleObservation> {
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

    pub(crate) async fn common_server_registry_state(
        &self,
    ) -> Result<api::registry::ServerRegistryObservation> {
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
    pub(crate) async fn common_screen_state(&self) -> Result<api::container::ScreenObservation> {
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
    pub(crate) async fn common_player_state(&self) -> Result<api::PlayerObservation> {
        let _gate = self.coherent_state_gate.lock().await;
        self.common_player_unlocked().await
    }
    pub(crate) async fn common_received_recipes(&self) -> Result<api::ReceivedRecipes> {
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
    pub(crate) async fn common_received_crafting_context(
        &self,
    ) -> Result<Option<api::ReceivedCraftingContext>> {
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
    pub(crate) async fn common_recipe_book_materials(
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
    pub(crate) async fn common_received_crafting(&self) -> Result<Option<api::ReceivedCrafting>> {
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
    pub(crate) async fn common_received_inventory(&self) -> Result<api::ReceivedInventory> {
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
            self.common_container_close.lock().await.as_ref(),
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
        })
    }
    pub(crate) async fn common_capture(&self, region: crate::Region) -> Result<api::Capture> {
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
    pub(crate) async fn execute_common(
        &self,
        mode: api::GameMode,
        action: Action<'_>,
    ) -> Result<Option<i32>> {
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
            return self.common_flight(command).await.map(|_| None);
        }
        self.execute_common_inner(mode, action, None).await
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
            Action::UseOnBlock(position, face, cursor) => {
                require_creative(mode)?;
                self.common_reach(position).await?;
                if cursor
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(api::registry::invalid("invalid block hit"));
                }
                let mut payload = vec![0];
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

#[cfg(test)]
mod tests {
    use super::*;
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
        for _ in 0..6 {
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
        assert_eq!(emitted.iter().map(|p| p.0).collect::<Vec<_>>(), expected);
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
        let state = bot.common_player_state().await.unwrap();
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
}
