//! Protocol 736 inventory packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

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
        let confirmation_observed = self
            .connection
            .observe_window_confirmation(
                transaction_context.0,
                transaction_context.1,
                transaction_context.2,
            )
            .await;
        let identity = (transaction.window_id, transaction.action);
        let exact = self
            .exact_window_barriers
            .lock()
            .await
            .contains_key(&identity);
        let mut inventory = self.inventory.write().await;
        if confirmation_observed {
            let pending = inventory
                .pending_clicks
                .remove(&(transaction.window_id, transaction.action));
            if let Some(pending) = &pending {
                if exact && transaction.accepted && pending.mode == ClickMode::Normal {
                    apply_accepted_normal_click(&mut inventory, pending)?;
                } else if !exact && !transaction.accepted {
                    rollback_click(&mut inventory, pending);
                } else if !exact && pending.mode == ClickMode::Normal {
                    rollback_click(&mut inventory, pending);
                    predict_normal_click(
                        &mut inventory,
                        pending.window_id,
                        pending.slot,
                        pending.button,
                    )?;
                    sync_player_inventory_from_window(&mut inventory, pending.window_id);
                }
            }
        }
        drop(inventory);
        if confirmation_observed {
            if !transaction.accepted {
                self.exact_window_barriers.lock().await.remove(&identity);
            } else if exact {
                if let Some(barrier) = self.exact_window_barriers.lock().await.get_mut(&identity) {
                    barrier.confirmation_seen = true;
                }
                self.commit_satisfied_window_barriers().await;
            } else {
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
            self.send_protocol(0x07, &payload).await?;
        }
        self.common_inventory_reply_received(transaction).await;
        self.common_click_reply_received(transaction).await;
        self.common_crafting_reply_received(transaction).await;
        self.common_transfer_reply_received(transaction).await;
        self.common_container_return_reply(transaction).await;
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
        apply_window_items(&mut inventory, window_id, slots);
        drop(inventory);
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
        apply_slot(&mut inventory, &update)?;
        let observed = inventory.map_snapshot(|_| update.clone());
        drop(inventory);
        self.emit(Event::InventorySlotObserved(observed));
        self.commit_satisfied_window_barriers().await;
        self.emit(Event::SlotUpdated(update));
        Ok(())
    }

    pub(super) async fn receive_horse_window(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
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
            inventory.open_window = Some(window.clone());
            let mut receipts = self.common_receipts.lock().await;
            receipts.inventory.window_id = Some(i32::from(window.id));
            receipts.inventory.cursor = None;
            receipts.player_starts.remove(&window.id);
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
            inventory.open_window = Some(window.clone());
            let mut receipts = self.common_receipts.lock().await;
            receipts.inventory.window_id = Some(i32::from(window.id));
            receipts.inventory.cursor = None;
            receipts.player_starts.remove(&window.id);
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
