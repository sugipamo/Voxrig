//! Protocol 736 inventory packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

/// Player-menu slots that a foreign screen opening neither re-supplies nor alters:
/// the 2x2 crafting result/grid (0..=4), armor (5..=8) and offhand (45).
/// The server keeps the player menu's grid while another menu is open and sends
/// no window-0 contents on close, so dropping these would leave the grid unknown
/// for the rest of the connection. Our own window-0 close is followed by actual
/// slot updates for whatever the server returns from the grid.
pub(super) const PLAYER_SLOTS_KEPT_BY_SCREENS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 45];

impl Bot {
    pub(super) async fn receive_window_confirmation(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let mut c = Cursor::new(p);
        let transaction = WindowTransaction {
            window_id: c.read_i8()?,
            action: c.read_i16::<BigEndian>()?,
            accepted: c.read_u8()? != 0,
            packet_sequence,
        };
        self.inventory.write().await.last_transaction = Some(transaction);
        let transaction_context = (
            transaction.window_id,
            transaction.action,
            transaction.accepted,
        );
        let identity = (transaction.window_id, transaction.action);
        let exact = self
            .exact_window_barriers
            .lock()
            .await
            .contains_key(&identity);
        let (confirmation_observed, response_work) = self
            .connection
            .observe_window_confirmation_work(
                transaction_context.0,
                transaction_context.1,
                transaction_context.2 || exact,
            )
            .await;
        let response_work = response_work.map(crate::client::ledger::ScopedWork::new);
        if confirmation_observed {
            self.inventory
                .write()
                .await
                .pending_clicks
                .remove(&identity);
            if exact {
                if let Some(barrier) = self.exact_window_barriers.lock().await.get_mut(&identity) {
                    barrier.confirmation_seen = true;
                }
            } else if transaction.accepted {
                let _ = self
                    .connection
                    .commit_window_barrier(transaction.window_id, transaction.action)
                    .await;
            }
        }
        if !transaction.accepted {
            let mut payload = Vec::new();
            payload.write_i8(transaction.window_id)?;
            payload.write_i16::<BigEndian>(transaction.action)?;
            payload.push(1);
            let child = response_work
                .as_ref()
                .map(|work| work.guard().child().expect("protocol response child"));
            self.connection
                .dispatch_protocol_work(0x07, &payload, child)
                .await?;
        }
        let execution_matches = self.inventory_protocol_response(transaction).await;
        self.common_inventory_reply_received(transaction).await;
        if execution_matches {
            self.common_click_reply_received(transaction).await;
        }
        if execution_matches {
            self.common_crafting_reply_received(transaction).await;
        }
        if execution_matches {
            self.common_transfer_reply_received(transaction).await;
        }
        self.common_container_return_reply(transaction).await;
        self.commit_satisfied_window_barriers().await;
        self.emit(Event::WindowTransaction(transaction));
        Ok(())
    }

    pub(super) async fn receive_close_window(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        if p.len() != 1 {
            return Err(crate::Error::new(
                crate::ErrorKind::Protocol,
                anyhow::anyhow!("invalid close window payload length"),
            ));
        }
        let window_id = *p.first().context("missing closed window ID")? as i8;
        let mut executor = lock_packet_state(&self.inventory_executor).await;
        if executor
            .record()
            .is_some_and(|r| r.window() == i32::from(window_id))
        {
            executor.invalidate();
        }
        drop(executor);
        if let Some(run) = lock_packet_state(&self.common_inventory_click)
            .await
            .as_mut()
            .filter(|run| run.record.unresolved() && run.record.window_id() == i32::from(window_id))
        {
            let predicate = "click original window closed";
            run.record
                .invalidation
                .get_or_insert(crate::client::inventory::Invalidation {
                    predicate: predicate.into(),
                    change: crate::client::inventory::StateChange::WindowChanged,
                    receipt: packet_sequence,
                });
            run.record.inspection(predicate);
        }
        if let Some(run) = lock_packet_state(&self.common_inventory_swap)
            .await
            .as_mut()
            .filter(|run| {
                run.record.stage == crate::client::inventory::InventorySwapStage::Pending
                    && run.record.window_id() == i32::from(window_id)
            })
        {
            let predicate = "swap original window closed";
            run.record
                .invalidation
                .get_or_insert(crate::client::inventory::Invalidation {
                    predicate: predicate.into(),
                    change: crate::client::inventory::StateChange::WindowChanged,
                    receipt: packet_sequence,
                });
            crate::client::inventory::inspection(&mut run.record, predicate);
        }
        self.common_container_close_received(i32::from(window_id), packet_sequence)
            .await;
        let mut inventory = self.inventory.write().await;
        if inventory
            .open_window
            .as_ref()
            .is_some_and(|window| window.id == window_id)
        {
            inventory.open_window = None;
        }
        {
            let mut receipts = self.common_receipts.lock().await;
            if receipts
                .container
                .as_ref()
                .is_some_and(|screen| screen.window == i32::from(window_id))
            {
                receipts.container = None;
                receipts.inventory.window_id = Some(0);
            }
        }
        inventory.last_transaction = None;
        inventory.merchant_offers = None;
        inventory.windows.remove(&window_id);
        inventory.properties.retain(|(id, _), _| *id != window_id);
        inventory
            .pending_clicks
            .retain(|(id, _), _| *id != window_id);
        let mut furnace_window_position = self.furnace_window_position.lock().await;
        if furnace_window_position.is_some_and(|(id, _)| id == window_id) {
            *furnace_window_position = None;
        }
        drop(furnace_window_position);
        drop(inventory);
        self.emit(Event::WindowClosed { window_id });
        Ok(())
    }

    pub(super) async fn receive_window_items(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let (window_id, slots) = parse_window_items(p)?;
        let mut inventory = self.inventory.write().await;
        self.common_receipts
            .lock()
            .await
            .window_items(window_id, &slots, packet_sequence)?;
        let _ = &mut **inventory; // received-domain revision, no duplicate slot writer
        drop(inventory);
        let (session, source) = self.inventory_execution_scope().await;
        lock_packet_state(&self.inventory_executor).await.contents(
            session,
            source,
            i32::from(window_id),
            packet_sequence,
            false,
        );
        self.commit_satisfied_window_barriers().await;
        self.emit(Event::InventoryUpdated {
            window_id,
            packet_sequence,
        });
        Ok(())
    }

    pub(super) async fn receive_set_slot(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let mut update = parse_set_slot(p)?;
        update.packet_sequence = packet_sequence;
        let mut inventory = self.inventory.write().await;
        self.common_receipts.lock().await.slot(&update)?;
        let _ = &mut **inventory; // received-domain revision; canonical state is above
        let observed = inventory.map_snapshot(|_| update.clone());
        drop(inventory);
        if update.window_id == -1 && update.slot == -1 {
            let (session, source) = self.inventory_execution_scope().await;
            lock_packet_state(&self.inventory_executor).await.cursor(
                session,
                source,
                packet_sequence,
            );
        }
        self.emit(Event::InventorySlotObserved(observed));
        self.commit_satisfied_window_barriers().await;
        self.emit(Event::SlotUpdated(update));
        Ok(())
    }

    pub(super) async fn receive_horse_window(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        lock_packet_state(&self.inventory_executor)
            .await
            .invalidate();
        let mut rest = p;
        let window_id = i8::try_from(*rest.first().context("missing horse window ID")?)
            .context("horse window ID out of range")?;
        rest = &rest[1..];
        let declared_slots = get_varint(&mut rest)?;
        let mut cursor = Cursor::new(rest);
        let entity_id = cursor.read_i32::<BigEndian>()?;
        let window = OpenWindow {
            id: window_id,
            window_type: -1,
            title_json: String::new(),
            entity_id: Some(entity_id),
            declared_slots: Some(declared_slots),
        };
        {
            let mut inventory = self.inventory.write().await;
            inventory.window_player_starts.remove(&window.id);
            inventory.windows.remove(&window.id);
            // Opening authority is ScreenReceipts; native view is a projection.
            let mut receipts = self.common_receipts.lock().await;
            for (i, value) in receipts.inventory.slots.iter_mut().enumerate() {
                if !PLAYER_SLOTS_KEPT_BY_SCREENS.contains(&i) {
                    *value = None;
                }
            }
            receipts.inventory.window_id = Some(i32::from(window.id));
            receipts.inventory.cursor = None;
            receipts.player_starts.remove(&window.id);
            receipts.legacy_open_extra = (window.entity_id, window.declared_slots);
            receipts.container = Some(crate::client::container::ScreenReceipts::open(
                crate::MinecraftVersion::Java1_16_1,
                i32::from(window.id),
                None,
                crate::client::container::ScreenTitle::Unavailable,
                packet_sequence,
            ));
        }
        *self.furnace_window_position.lock().await = None;
        self.emit(Event::WindowOpened(window));
        Ok(())
    }

    pub(super) async fn receive_open_window(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        lock_packet_state(&self.inventory_executor)
            .await
            .invalidate();
        let mut rest = p;
        let id = get_varint(&mut rest)?;
        if !(1..=127).contains(&id) {
            bail!("invalid open window ID {id}");
        }
        let window = OpenWindow {
            id: id as i8,
            window_type: get_varint(&mut rest)?,
            title_json: get_string(&mut rest)?,
            entity_id: None,
            declared_slots: None,
        };
        {
            let mut inventory = self.inventory.write().await;
            inventory.window_player_starts.remove(&window.id);
            inventory.windows.remove(&window.id);
            // Opening authority is ScreenReceipts; native view is a projection.
            let mut receipts = self.common_receipts.lock().await;
            for (i, value) in receipts.inventory.slots.iter_mut().enumerate() {
                if !PLAYER_SLOTS_KEPT_BY_SCREENS.contains(&i) {
                    *value = None;
                }
            }
            receipts.inventory.window_id = Some(i32::from(window.id));
            receipts.inventory.cursor = None;
            receipts.player_starts.remove(&window.id);
            receipts.legacy_open_extra = (window.entity_id, window.declared_slots);
            receipts.container = Some(crate::client::container::ScreenReceipts::open(
                crate::MinecraftVersion::Java1_16_1,
                i32::from(window.id),
                Some(window.window_type),
                crate::client::container::ScreenTitle::LegacyJson {
                    json: window.title_json.clone(),
                },
                packet_sequence,
            ));
        }
        let furnace_position = self
            .connection
            .observe_furnace_window(window.window_type)
            .await;
        *self.furnace_window_position.lock().await =
            furnace_position.map(|position| (window.id, position));
        self.emit(Event::WindowOpened(window));
        Ok(())
    }

    pub(super) async fn receive_recipe_ghost(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let (window_id, recipe_id) = common_recipe_placement::decode_ghost(p)?;
        let close = self.common_container_close.lock().await.clone();
        let mut receipts = self.common_receipts.lock().await;
        let context = crate::client::crafting::ghost::GhostContext {
            generation: receipts.generation,
            sequence: packet_sequence,
            active_window: receipts.inventory.window_id,
            screen: receipts.container.clone(),
            close,
            registries: receipts.registries.clone(),
        };
        receipts.recipe_ghost = Some(crate::client::crafting::ghost::GhostReceipts::named(
            context,
            i32::from(window_id),
            recipe_id.clone(),
            &receipts.recipes,
        ));
        drop(receipts);
        self.emit(Event::CraftRecipeResponse {
            window_id,
            recipe_id,
        });
        Ok(())
    }

    pub(super) async fn receive_recipe_book(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let mut book = self.recipe_book.write().await;
        book.apply(p)?;
        let mut header = p;
        let initial = get_varint(&mut header)? == 0;
        self.common_receipts.lock().await.recipes.legacy_book(
            book.unlocked.iter().cloned().collect(),
            book.displayed.iter().cloned().collect(),
            initial,
            packet_sequence,
        );
        drop(book);
        self.emit(Event::RecipeBookUpdated);
        Ok(())
    }

    pub(super) async fn receive_declare_recipes(
        &self,
        p: &[u8],
        packet_sequence: u64,
    ) -> Result<()> {
        let recipes = parse_recipes(p)?;
        let common = crate::client::crafting::recipes::legacy_entries(&recipes)?;
        **self.server_recipes.write().await = recipes;
        self.common_receipts
            .lock()
            .await
            .recipes
            .declare_legacy(common, packet_sequence);
        self.emit(Event::RecipesDeclared);
        Ok(())
    }
}
