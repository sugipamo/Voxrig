//! Protocol 736 entities packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

impl Bot {
    pub(super) async fn receive_entity_relative_move(&self, p: &[u8], id: i32) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        if let Some(entity) = write_entity_update(&self.entities)
            .await
            .entities
            .get_mut(&entity_id)
        {
            apply_relative(entity, p, id == 0x29)?;
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }

    pub(super) async fn receive_entity_rotation(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        if let Some(entity) = write_entity_update(&self.entities)
            .await
            .entities
            .get_mut(&entity_id)
        {
            entity.yaw =
                f32::from(*rest.first().context("missing entity yaw")? as i8) * 360.0 / 256.0;
            entity.pitch =
                f32::from(*rest.get(1).context("missing entity pitch")? as i8) * 360.0 / 256.0;
            entity.on_ground = *rest.get(2).context("missing entity ground flag")? != 0;
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }

    pub(super) async fn receive_vehicle_position(&self, p: &[u8]) -> Result<()> {
        let mut cursor = Cursor::new(p);
        let pose = VehiclePose {
            position: Vec3 {
                x: cursor.read_f64::<BigEndian>()?,
                y: cursor.read_f64::<BigEndian>()?,
                z: cursor.read_f64::<BigEndian>()?,
            },
            yaw: cursor.read_f32::<BigEndian>()?,
            pitch: cursor.read_f32::<BigEndian>()?,
        };
        validate_position(pose.position.x, pose.position.y, pose.position.z)?;
        if !pose.yaw.is_finite() || !pose.pitch.is_finite() {
            bail!("vehicle position contains a non-finite rotation");
        }
        self.common_receipts
            .lock()
            .await
            .vehicles
            .interrupt_motion(self.protocol_packet_sequence.load(Ordering::Acquire));
        if let Some(vehicle) = self.vehicle().await {
            if let Some(tracked) = self
                .entities
                .write()
                .await
                .entities
                .get_mut(&vehicle.entity_id)
            {
                tracked.position = pose.position;
                tracked.yaw = pose.yaw;
                tracked.pitch = pose.pitch;
            }
        }
        self.emit(Event::VehiclePosition(pose));
        Ok(())
    }

    pub(super) async fn receive_destroy_entities(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let count = get_varint(&mut rest)?;
        if !(0..=65_536).contains(&count) {
            bail!("invalid destroyed entity count {count}");
        }
        let mut entity_ids = Vec::with_capacity(count as usize);
        let mut entities = self.entities.write().await;
        for _ in 0..count {
            let entity_id = get_varint(&mut rest)?;
            entities.entities.remove(&entity_id);
            let mut receipts = self.common_receipts.lock().await;
            receipts.entities.remove(entity_id);
            receipts.vehicles.retire(entity_id);
            entity_ids.push(entity_id);
        }
        drop(entities);
        self.emit(Event::EntitiesDestroyed { entity_ids });
        Ok(())
    }

    pub(super) async fn receive_entity_head_yaw(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        if let Some(entity) = write_entity_update(&self.entities)
            .await
            .entities
            .get_mut(&entity_id)
        {
            entity.head_yaw =
                f32::from(*rest.first().context("missing entity head yaw")? as i8) * 360.0 / 256.0;
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }

    pub(super) async fn receive_entity_metadata(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let (entity_id, metadata) = parse_metadata(p)?;
        let (_, common) = crate::versions::java_1_16_1::entity::common_metadata(p)?;
        // The legacy decoder reads a packet to its end or fails it.
        self.common_receipts.lock().await.entities.receive_metadata(
            entity_id,
            common,
            true,
            packet_sequence,
        );
        if Some(entity_id) == lock_packet_state(&self.player).await.entity_id {
            if let Some(MetadataValue::VarInt(pose)) = metadata.get(&6) {
                *self.local_pose.lock().await = Some(*pose);
                if *pose != 0 {
                    self.interrupt_common_motion("native posture changed during finite motion")
                        .await;
                }
            }
            if let Some(MetadataValue::VarInt(air_ticks)) = metadata.get(&1) {
                *self.oxygen_level.lock().await = oxygen_level_from_air_ticks(*air_ticks);
                self.common_receipts.lock().await.air_supply =
                    Some(crate::client::received(*air_ticks, packet_sequence));
            }
            let flags_index = crate::MinecraftVersion::Java1_16_1
                .table()
                .entities
                .living_flags_metadata_index;
            if let Some(MetadataValue::Byte(flags)) = metadata.get(&flags_index) {
                self.common_receipts.lock().await.using_item = Some(crate::client::received(
                    crate::client::item_use::hand_from_living_flags(*flags as u8),
                    packet_sequence,
                ));
            }
        }
        let health_index = crate::MinecraftVersion::Java1_16_1
            .table()
            .entities
            .health_metadata_index;
        let health = match metadata.get(&health_index) {
            Some(MetadataValue::Float(health)) => Some(*health),
            _ => None,
        };
        // Read the kind first so no two locks are held at once.
        let living = self
            .entities
            .read()
            .await
            .entities
            .get(&entity_id)
            .is_some_and(|entity| {
                matches!(
                    entity.kind,
                    crate::EntityKind::Living | crate::EntityKind::Player
                )
            });
        if let (true, Some(health)) = (living, health) {
            self.common_receipts.lock().await.entities.receive_health(
                entity_id,
                health,
                packet_sequence,
            );
        }
        if let Some(entity) = self.entities.write().await.entities.get_mut(&entity_id) {
            entity.metadata.extend(metadata);
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }

    pub(super) async fn receive_entity_velocity(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        let mut c = Cursor::new(rest);
        let velocity = Vec3 {
            x: f64::from(c.read_i16::<BigEndian>()?) / 8000.0,
            y: f64::from(c.read_i16::<BigEndian>()?) / 8000.0,
            z: f64::from(c.read_i16::<BigEndian>()?) / 8000.0,
        };
        if Some(entity_id) == lock_packet_state(&self.player).await.entity_id {
            self.motion.lock().await.velocity = velocity;
            *self.own_velocity_receipt.lock().await = Some((
                self.protocol_packet_sequence.load(Ordering::Acquire),
                [velocity.x, velocity.y, velocity.z],
            ));
            self.interrupt_common_motion("native own-player velocity interrupted finite motion")
                .await;
        }
        if let Some(entity) = write_entity_update(&self.entities)
            .await
            .entities
            .get_mut(&entity_id)
        {
            entity.velocity = velocity;
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }

    pub(super) async fn receive_entity_equipment(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        let mut equipment = Vec::new();
        loop {
            if equipment.len() >= 16 {
                bail!("entity equipment packet exceeds 16 entries");
            }
            let raw_slot = *rest.first().context("missing equipment slot")?;
            rest = &rest[1..];
            equipment.push(((raw_slot & 0x7f) as i8, read_slot(&mut rest)?));
            if raw_slot & 0x80 == 0 {
                break;
            }
        }
        {
            let mut receipts = self.common_receipts.lock().await;
            for (slot, stack) in &equipment {
                let Some(slot) = crate::client::EquipmentSlot::from_native(
                    crate::MinecraftVersion::Java1_16_1,
                    *slot as u8,
                ) else {
                    continue;
                };
                let item = crate::client::legacy_slot(stack.as_ref())
                    .unwrap_or(crate::client::SlotKnowledge::Unavailable);
                receipts
                    .entities
                    .receive_equipment(entity_id, slot, item, packet_sequence);
            }
        }
        if let Some(entity) = self.entities.write().await.entities.get_mut(&entity_id) {
            entity.equipment.extend(equipment);
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }

    pub(super) async fn receive_passengers(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let update = crate::client::vehicle::NativePassengers::decode(p)?;
        let player_id = self.player.lock().await.entity_id;
        {
            let mut receipts = self.common_receipts.lock().await;
            let receipts = &mut *receipts;
            receipts
                .vehicles
                .receive(&update, player_id, &receipts.entities, packet_sequence);
        }
        if player_id.is_some_and(|id| update.passengers.contains(&id)) {
            self.retire_common_for_mount().await;
            self.interrupt_common_motion_operations(
                "actual own mount interrupted ground operations",
            )
            .await;
        }
        if let Some(vehicle) = self
            .entities
            .write()
            .await
            .entities
            .get_mut(&update.vehicle)
        {
            vehicle.passengers = update.passengers.clone();
        }
        self.emit(Event::PassengersUpdated {
            vehicle_id: update.vehicle,
            passengers: update.passengers,
        });
        Ok(())
    }

    pub(super) async fn receive_collect_item(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let mut rest = p;
        let collected_entity_id = get_varint(&mut rest)?;
        let collector_entity_id = get_varint(&mut rest)?;
        let count = get_varint(&mut rest)?;
        let collected_item_name = self
            .entities
            .read()
            .await
            .entities
            .get(&collected_entity_id)
            .and_then(EntityState::item_drop)
            .and_then(ItemStack::name);
        self.emit(Event::ItemCollected(ItemCollected {
            collected_entity_id,
            collector_entity_id,
            count,
            packet_sequence,
            collected_item_name,
        }));
        Ok(())
    }

    pub(super) async fn receive_entity_teleport(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let entity_id = get_varint(&mut rest)?;
        let mut c = Cursor::new(rest);
        if let Some(entity) = write_entity_update(&self.entities)
            .await
            .entities
            .get_mut(&entity_id)
        {
            let position = Vec3 {
                x: c.read_f64::<BigEndian>()?,
                y: c.read_f64::<BigEndian>()?,
                z: c.read_f64::<BigEndian>()?,
            };
            validate_position(position.x, position.y, position.z)?;
            entity.position = position;
            entity.yaw = f32::from(c.read_i8()?) * 360.0 / 256.0;
            entity.pitch = f32::from(c.read_i8()?) * 360.0 / 256.0;
            entity.on_ground = c.read_u8()? != 0;
            self.emit(Event::EntityUpdated(entity.clone()));
        }
        Ok(())
    }
}
