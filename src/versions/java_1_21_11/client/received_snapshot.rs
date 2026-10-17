//! Receipt updates committed while State's ordered receive lock is held.
use super::*;
use crate::client::{
    self as api,
    received_snapshot::{
        FieldReceipt, InvalidationReason, Presence, Receipt, ReceivedCorrectionFields,
        ReceivedDimensionGeometry,
    },
};
impl State {
    pub(super) fn snapshot_session(&self) -> api::SessionStamp {
        api::SessionStamp {
            version: MinecraftVersion::Java1_21_11,
            connection_id: self.connection_id,
            world_generation: self.loading.generation,
        }
    }
    pub(super) fn commit_received_snapshot(
        &mut self,
        id: i32,
        payload: &[u8],
        in_play: bool,
    ) -> anyhow::Result<()> {
        use ids::play_clientbound as input;
        let session = self.snapshot_session();
        let event = Receipt::new(session, self.sequence);
        let registries = self.registries.capture(session, self.sequence);
        if !in_play && id == ids::configuration_clientbound::REGISTRY_DATA {
            let mut r = Reader::new(payload);
            if r.string()? == "minecraft:dimension_type" {
                self.geometry = self
                    .dimensions
                    .iter()
                    .map(|dimension| FieldReceipt {
                        value: ReceivedDimensionGeometry {
                            min_y: dimension.min_y,
                            height: dimension.height,
                            registry_owner: registries.stamp(),
                        },
                        receipt: event,
                    })
                    .collect();
            }
        }
        let out = self.snapshot.commit(
            event,
            registries,
            if in_play && id == input::RESPAWN {
                InvalidationReason::RespawnReset
            } else if in_play && id == input::START_CONFIGURATION {
                InvalidationReason::Reconfiguration
            } else {
                InvalidationReason::SessionBoundary
            },
        );
        if !in_play {
            return Ok(());
        }
        if id == input::START_CONFIGURATION {
            self.geometry.clear();
            return Ok(());
        }
        let native = &self.operations;
        let own = &native.local_player;
        let sequence = self.sequence;
        if let Some(health) = &own.health {
            if health.receive_sequence == sequence {
                out.health.set(
                    api::Health {
                        health: health.health,
                        food: health.food,
                        saturation: health.saturation,
                    },
                    event,
                );
            }
        }
        if let Some(selection) = &native.selected_hotbar {
            if selection.from_server && selection.sequence == sequence {
                out.selected_hotbar.set(selection.slot, event);
            }
        }
        out.abilities.apply(self.context.abilities.as_ref(), event);
        if let Some(value) = own
            .using_item
            .filter(|value| value.receive_sequence == sequence)
        {
            out.using_item.set(value.hand, event);
        }
        if let Some((value, _)) = own.air_supply.filter(|(_, seq)| *seq == sequence) {
            out.air_supply.set(value, event);
        }
        if let Some((value, _)) = own.fall_flying.filter(|(_, seq)| *seq == sequence) {
            out.fall_flying.set(value, event);
        }
        if let Some(value) = native
            .server_time
            .as_ref()
            .filter(|v| v.receive_sequence == sequence)
        {
            out.world_time.set(
                api::WorldTime {
                    game_time: value.game_age,
                    day_time: value.day_time,
                },
                event,
            );
        }
        for (id, value) in &own.received_attributes {
            if own.attribute_sequences.get(id) == Some(&sequence) {
                if let Some(name) = api::player_facts::modern_attribute_name(*id) {
                    out.attribute_received(name, value.base, value.modifiers.clone(), event);
                }
            }
        }
        for value in own
            .effect_updates
            .values()
            .filter(|v| v.receive_sequence == sequence)
        {
            if let Some(name) = api::player_facts::effect_name(session.version, value.effect_id) {
                out.effect_received(
                    &name,
                    Presence::Present(api::player_facts::effect(
                        value.amplifier,
                        value.duration_at_receipt,
                        value.flags,
                    )),
                    event,
                );
            }
        }
        if id == input::REMOVE_ENTITY_EFFECT {
            let mut r = Reader::new(payload);
            if Some(r.varint()?) == own.entity_id {
                if let Some(name) = api::player_facts::effect_name(session.version, r.varint()?) {
                    out.effect_received(&name, Presence::Absent, event);
                }
            }
        }
        if matches!(id, input::LOGIN | input::RESPAWN) {
            if let Some(entry) = &self.context.world_entry {
                out.entry(
                    &entry.value,
                    if id == input::LOGIN {
                        own.entity_id
                    } else {
                        None
                    },
                    event,
                )?;
                if let Some(geometry) = entry
                    .value
                    .dimension_type_id
                    .and_then(|id| self.geometry.get(id as usize))
                {
                    if geometry.value.registry_owner == out.registries.stamp() {
                        out.dimension_geometry = api::FieldState::Received(geometry.clone());
                    }
                }
            }
        }
        if id == input::GAME_STATE_CHANGE && payload.first() == Some(&3) {
            if let Some(mode) = native.game_mode {
                out.game_mode.set(mode, event);
            }
        }
        if id == input::POSITION {
            let mut r = Reader::new(payload);
            let teleport = r.varint()?;
            let correction = correction::Correction::read(&mut r)?;
            out.correction_received(
                ReceivedCorrectionFields::from_packet(
                    teleport,
                    correction.position,
                    correction.rotation,
                    Some(correction.delta),
                    correction.flags,
                ),
                event,
            );
        }
        if id == input::CLOSE_WINDOW {
            out.close(Reader::new(payload).varint()?, event);
        }
        let inventory = &native.inventory;
        if inventory.receive_sequence == Some(sequence)
            || id == input::OPEN_WINDOW
            || id == input::CLOSE_WINDOW
        {
            let slots = inventory
                .slots
                .iter()
                .zip(&inventory.slot_sequences)
                .map(|(slot, seq)| {
                    seq.map(|seq| {
                        operations::common_slot(slot).map(|value| api::received(value, seq))
                    })
                    .transpose()
                })
                .collect::<crate::Result<Vec<_>>>()?;
            let cursor = inventory
                .cursor_sequence
                .map(|seq| {
                    operations::common_slot(&inventory.cursor)
                        .map(|value| api::received(value, seq))
                })
                .transpose()?;
            // Only actual scope withdrawals invalidate omitted constituents.
            // A matching foreign full packet supplies appended player slots,
            // not missing equipment or a discarded player-zero cursor.
            let closed_here = matches!(out.container_state(), api::FieldState::Received(field)
                if field.receipt().ordinal() == sequence && matches!(field.value(), api::ContainerStatus::Closed(_)));
            let full_withdrawal = id == input::WINDOW_ITEMS
                && inventory.slot_sequences.iter().all(Option::is_none)
                && inventory.cursor_sequence.is_none();
            let screen_withdrawal =
                id == input::OPEN_WINDOW || (id == input::CLOSE_WINDOW && closed_here);
            if screen_withdrawal || full_withdrawal {
                out.inventory_unavailable(
                    inventory.slots.iter().enumerate().map(|(index, slot)| {
                        matches!(slot, operations::InventorySlot::Unavailable)
                            && (!screen_withdrawal || ![5, 6, 7, 8, 45].contains(&index))
                    }),
                    matches!(inventory.cursor, operations::InventorySlot::Unavailable),
                    if inventory.unsupported_components && full_withdrawal {
                        InvalidationReason::UnsupportedPayload
                    } else {
                        InvalidationReason::ScreenChanged
                    },
                    event,
                );
            } else if id == input::SET_CURSOR_ITEM && inventory.cursor_sequence.is_none() {
                out.inventory
                    .cursor
                    .invalidate(&api::received_snapshot::Invalidation::packet(
                        InvalidationReason::UnsupportedPayload,
                        session,
                        event,
                    ));
            }
            out.inventory_received(
                &api::InventoryObservation {
                    slots,
                    cursor,
                    player_screen_revision: inventory.player_revision.clone(),
                    ..Default::default()
                },
                event,
            )?;
            if id == input::WINDOW_ITEMS
                && Reader::new(payload).varint()? == 0
                && !inventory.unsupported_components
            {
                out.inventory.last_full_contents.set((), event);
            }
        }
        out.screen(
            inventory
                .container
                .as_ref()
                .map(|screen| screen.capture(session)),
            event,
        )?;
        Ok(())
    }
}

impl Bot {
    pub(super) fn check_received_snapshot(&self, state: &State) -> crate::Result<()> {
        if let Some(error) = &state.failure {
            return Err(Error::new(error.kind(), anyhow::anyhow!("{error}")));
        }
        if self.session.stopped.load(Ordering::Acquire)
            || self.session.revoked.load(Ordering::Acquire)
        {
            return Err(Error::new(
                ErrorKind::State,
                anyhow::anyhow!("connection is closing, closed or revoked"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ContainerStatus, CorrectionComponent, Coverage, FieldState};
    use ids::play_clientbound as packet;
    fn received<T>(state: &FieldState<T>) -> &FieldReceipt<T> {
        match state {
            FieldState::Received(value) => value,
            _ => panic!("expected received"),
        }
    }
    fn capture(state: &State) -> api::ReceivedPlayerSnapshot {
        let session = state.snapshot_session();
        state.snapshot.capture(
            session,
            state.sequence,
            state.registries.capture(session, state.sequence),
        )
    }
    fn spawn() -> Vec<u8> {
        let mut p = vec![0];
        put_string(&mut p, "minecraft:overworld");
        p.extend(0i64.to_be_bytes());
        p.extend([0, 255, 0, 0, 0, 0, 63]);
        p
    }
    fn state() -> State {
        let mut state = State {
            connection_id: 917,
            ..State::default()
        };
        let mut registry = Vec::new();
        put_string(&mut registry, "minecraft:dimension_type");
        registry.push(1);
        put_string(&mut registry, "minecraft:overworld");
        registry.extend([1, 10]);
        for (name, value) in [("min_y", -64i32), ("height", 384)] {
            registry.push(3);
            registry.extend((name.len() as u16).to_be_bytes());
            registry.extend(name.as_bytes());
            registry.extend(value.to_be_bytes());
        }
        registry.push(0);
        state
            .receive(ids::configuration_clientbound::REGISTRY_DATA, &registry, 64)
            .unwrap();
        state
            .receive(
                ids::configuration_clientbound::FINISH_CONFIGURATION,
                &[],
                64,
            )
            .unwrap();
        let mut login = 42i32.to_be_bytes().to_vec();
        login.extend([0, 0, 10, 3, 3, 0, 1, 0]);
        login.extend(spawn());
        login.push(0);
        state.receive(packet::LOGIN, &login, 64).unwrap();
        state
    }
    fn health() -> Vec<u8> {
        let mut p = 20f32.to_be_bytes().to_vec();
        p.push(20);
        p.extend(5f32.to_be_bytes());
        p
    }
    fn correction(flags: u32) -> Vec<u8> {
        let mut p = vec![7];
        for value in [1f64, 64., 2., 0., 0., 0.] {
            p.extend(value.to_be_bytes());
        }
        p.extend(30f32.to_be_bytes());
        p.extend(120f32.to_be_bytes());
        p.extend(flags.to_be_bytes());
        p
    }
    #[test]
    fn received_snapshot_original_apply_times_and_same_value_packets() {
        let mut s = state();
        assert!(matches!(capture(&s).health(), FieldState::Missing));
        s.receive(packet::UPDATE_HEALTH, &health(), 64).unwrap();
        let before_capture = Instant::now();
        let one = capture(&s);
        let receipt = received(one.health()).receipt();
        assert!(receipt.applied_at() <= before_capture);
        assert!(receipt.applied_at() <= one.captured_at());
        for _ in 0..3 {
            let two = capture(&s);
            assert_eq!(received(two.health()).receipt(), receipt);
        }
        s.receive(packet::HELD_ITEM_SLOT, &[2], 64).unwrap();
        assert_eq!(received(capture(&s).health()).receipt(), receipt);
        s.receive(packet::UPDATE_HEALTH, &health(), 64).unwrap();
        let two = capture(&s);
        assert!(received(two.health()).receipt().ordinal() > receipt.ordinal());
        assert!(received(two.health()).receipt().applied_at() >= receipt.applied_at());
        assert_eq!(two.boundary().session().connection_id, 917);
        assert_eq!(received(one.health()).receipt(), receipt);
    }
    #[test]
    fn received_snapshot_excludes_models_and_retains_literal_corrections() {
        let mut s = state();
        s.position = Some([8., 9., 10.]);
        s.rotation = [90., 0.];
        s.motion.on_ground = Some(true);
        s.operations.selected_hotbar = Some(operations::HotbarSelection {
            slot: 8,
            sequence: s.sequence,
            dispatched: true,
            from_server: false,
        });
        let initial = capture(&s);
        assert!(matches!(initial.absolute_position(), FieldState::Missing));
        assert!(matches!(initial.absolute_rotation(), FieldState::Missing));
        assert!(matches!(initial.selected_hotbar(), FieldState::Missing));
        s.receive(packet::POSITION, &correction(0), 64).unwrap();
        let absolute = capture(&s);
        assert_eq!(received(absolute.absolute_rotation()).value(), &[30., 120.]);
        assert_eq!(s.rotation[1], 90.); // Native clamp has no received provenance.
        s.rotation = [1., 2.];
        s.position = Some([4., 5., 6.]);
        assert_eq!(
            received(capture(&s).absolute_position()).value(),
            &[1., 64., 2.]
        );
        s.receive(packet::POSITION, &correction(31), 64).unwrap();
        let relative = capture(&s);
        assert!(matches!(
            received(relative.correction()).value().position()[0],
            CorrectionComponent::Relative(1.)
        ));
        let FieldState::Invalidated(event) = relative.absolute_position() else {
            panic!("relative projection")
        };
        assert_eq!(event.reason(), InvalidationReason::ProjectionUnavailable);
        assert_eq!(event.event_ordinal(), Some(relative.boundary().ordinal()));
        assert_eq!(
            event.event_applied_at(),
            Some(received(relative.correction()).receipt().applied_at())
        );
        assert!(matches!(
            relative.absolute_rotation(),
            FieldState::Invalidated(_)
        ));
        let mut attributes = vec![42, 1, 22];
        attributes.extend(2f64.to_be_bytes());
        attributes.push(1);
        put_string(&mut attributes, "minecraft:test");
        attributes.extend(3f64.to_be_bytes());
        attributes.push(0);
        s.receive(packet::ENTITY_UPDATE_ATTRIBUTES, &attributes, 64)
            .unwrap();
        let raw = capture(&s);
        let Presence::Present(attribute) =
            received(raw.attribute("minecraft:movement_speed")).value()
        else {
            panic!("attribute")
        };
        assert_eq!(attribute.base(), 2.);
        assert_eq!(attribute.modifiers().len(), 1);
        // Raw attributes have no effective-value getter; changing the computed
        // native attribute model cannot create any raw attribute receipt.
        assert!(matches!(
            relative.attribute("minecraft:movement_speed"),
            FieldState::Missing
        ));
    }
    #[test]
    fn received_snapshot_removals_are_bounded_and_session_scoped() {
        let mut s = state();
        let count = capture(&s).effects().count();
        assert!(count > 0);
        let mut unknown = vec![42];
        put_varint(&mut unknown, 65535);
        s.receive(packet::REMOVE_ENTITY_EFFECT, &unknown, 64)
            .unwrap();
        assert_eq!(capture(&s).effects_coverage(), Coverage::Unknown);
        assert_eq!(capture(&s).effects().count(), count);
        s.receive(packet::REMOVE_ENTITY_EFFECT, &[42, 0], 64)
            .unwrap();
        let one = capture(&s);
        assert!(matches!(
            received(one.effect("minecraft:speed").unwrap()).value(),
            Presence::Absent
        ));
        let receipt = received(one.effect("minecraft:speed").unwrap()).receipt();
        assert_eq!(one.effects_coverage(), Coverage::Partial);
        s.receive(packet::REMOVE_ENTITY_EFFECT, &[42, 0], 64)
            .unwrap();
        assert!(
            received(capture(&s).effect("minecraft:speed").unwrap())
                .receipt()
                .ordinal()
                > receipt.ordinal()
        );
        s.receive(packet::ENTITY_EFFECT, &[42, 0, 1, 100, 7], 64)
            .unwrap();
        assert!(matches!(
            received(capture(&s).effect("minecraft:speed").unwrap()).value(),
            Presence::Present(_)
        ));
        s.receive(packet::REMOVE_ENTITY_EFFECT, &[42, 0], 64)
            .unwrap();
        assert!(one.effect("minecraft:not_supported").is_err());
        let old_session = capture(&s).boundary().session();
        let mut respawn = spawn();
        respawn.push(3);
        s.receive(packet::RESPAWN, &respawn, 64).unwrap();
        let reset = capture(&s);
        let FieldState::Invalidated(event) = reset.effect("minecraft:speed").unwrap() else {
            panic!("reset")
        };
        assert_eq!(event.reason(), InvalidationReason::RespawnReset);
        assert_eq!(event.invalidated_session(), old_session);
        assert_eq!(event.event_session(), reset.boundary().session());
        assert_eq!(event.event_ordinal(), Some(reset.boundary().ordinal()));
        assert!(event.event_applied_at().unwrap() <= reset.captured_at());
        assert_eq!(reset.effects().count(), count);
        assert_eq!(reset.effects_coverage(), Coverage::Unknown);
        assert!(matches!(reset.entity_id(), FieldState::Invalidated(_)));
        assert!(matches!(
            reset.attribute("never:received"),
            FieldState::Invalidated(_)
        ));
        assert!(matches!(
            received(one.effect("minecraft:speed").unwrap()).value(),
            Presence::Absent
        ));
        s.receive(packet::START_CONFIGURATION, &[], 64).unwrap();
        let config = capture(&s);
        let FieldState::Invalidated(event) = config.game_mode() else {
            panic!("configuration")
        };
        assert_eq!(event.reason(), InvalidationReason::Reconfiguration);
        assert!(matches!(
            config.dimension_geometry(),
            FieldState::Invalidated(_)
        ));
    }
    #[test]
    fn received_snapshot_geometry_keeps_registry_apply_time_and_origin() {
        let mut s = state();
        let one = capture(&s);
        let geometry = received(one.dimension_geometry());
        assert_eq!(geometry.receipt().ordinal(), 1);
        assert_eq!(geometry.receipt().session().world_generation, 0);
        assert_eq!(geometry.value().min_y(), -64);
        assert_eq!(one.dimension_descriptor_coverage(), Coverage::Complete);
        let mut respawn = spawn();
        respawn.push(0);
        s.receive(packet::RESPAWN, &respawn, 64).unwrap();
        assert_eq!(
            received(capture(&s).dimension_geometry()).receipt(),
            geometry.receipt()
        );
    }
    #[test]
    fn received_snapshot_window_reuse_close_and_inventory_markers() {
        let mut s = state();
        let mut full = vec![0, 5, 46];
        full.extend([0; 47]);
        s.receive(packet::WINDOW_ITEMS, &full, 64).unwrap();
        let player = capture(&s);
        assert_eq!(player.inventory().bundle_coverage(), Coverage::Complete);
        let full_receipt = received(player.inventory().last_full_contents()).receipt();
        assert_eq!(
            received(player.inventory().slot(10).unwrap())
                .receipt()
                .ordinal(),
            received(player.inventory().slot(10).unwrap())
                .value()
                .receive_sequence()
        );
        assert!(player.inventory().slot(46).is_err());
        let opening = [7, 0, 10, 0]; // generic_9x1, empty anonymous title compound
        s.receive(packet::OPEN_WINDOW, &opening, 64).unwrap();
        let first = capture(&s).container().unwrap().id();
        let opened = capture(&s);
        let FieldState::Invalidated(cursor_event) = opened.inventory().cursor() else {
            panic!("opening cursor")
        };
        let cursor_ordinal = cursor_event.event_ordinal();
        let cursor_time = cursor_event.event_applied_at();
        s.receive(packet::WINDOW_ITEMS, &full, 64).unwrap(); // player-zero cursor is not published here
        let omitted = capture(&s);
        let FieldState::Invalidated(cursor_event) = omitted.inventory().cursor() else {
            panic!("discarded cursor")
        };
        assert_eq!(cursor_event.event_ordinal(), cursor_ordinal);
        assert_eq!(cursor_event.event_applied_at(), cursor_time);
        let refreshed_full = received(omitted.inventory().last_full_contents()).receipt();
        assert!(refreshed_full.ordinal() > full_receipt.ordinal());
        let full_receipt = refreshed_full;

        s.receive(packet::OPEN_WINDOW, &opening, 64).unwrap();
        let second = capture(&s).container().unwrap().id();
        assert_ne!(first, second);
        assert!(second.opened_sequence() > first.opened_sequence());
        s.receive(packet::CLOSE_WINDOW, &[6], 64).unwrap();
        assert!(
            matches!(received(capture(&s).container_state()).value(), ContainerStatus::Open(id) if *id == second)
        );
        s.receive(packet::CLOSE_WINDOW, &[7], 64).unwrap();
        let closed = capture(&s);
        assert!(closed.container().is_none());
        assert!(
            matches!(received(closed.container_state()).value(), ContainerStatus::Closed(id) if *id == second)
        );
        assert_eq!(
            received(closed.inventory().last_full_contents()).receipt(),
            full_receipt
        );
        assert_ne!(closed.inventory().bundle_coverage(), Coverage::Complete);
    }
    #[test]
    fn received_snapshot_replacement_login_and_exhaustion_never_reuse_ordinals() {
        let mut s = state();
        s.receive(packet::UPDATE_HEALTH, &health(), 64).unwrap();
        let old = capture(&s).boundary().session();
        let mut login = 42i32.to_be_bytes().to_vec();
        login.extend([0, 0, 10, 3, 3, 0, 1, 0]);
        login.extend(spawn());
        login.push(0);
        s.receive(packet::LOGIN, &login, 64).unwrap();
        let reset = capture(&s);
        let FieldState::Invalidated(event) = reset.health() else {
            panic!("replacement login")
        };
        assert_eq!(event.reason(), InvalidationReason::SessionBoundary);
        assert_eq!(event.invalidated_session(), old);
        let ordinal = event.event_ordinal();
        let applied_at = event.event_applied_at();
        s.receive(packet::HELD_ITEM_SLOT, &[1], 64).unwrap();
        let later = capture(&s);
        let FieldState::Invalidated(event) = later.health() else {
            panic!("persistent invalidation")
        };
        assert_eq!(event.event_ordinal(), ordinal);
        assert_eq!(event.event_applied_at(), applied_at);
        s.sequence = u64::MAX;
        assert!(s.receive(packet::UPDATE_HEALTH, &health(), 64).is_err());
        assert_eq!(s.sequence, u64::MAX);
        assert!(s.failure.is_some());
    }
}
