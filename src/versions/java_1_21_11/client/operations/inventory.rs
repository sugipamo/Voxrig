//! Ordinary player-inventory swaps. Submitted clicks never predict received slots.
use super::*;
use crate::diagnostic_projection::diagnostic_record;
use std::time::Duration;
pub(in crate::versions::java_1_21_11::client) mod click;
pub(in crate::versions::java_1_21_11::client) mod common;
pub(in crate::versions::java_1_21_11::client) mod crafting;
pub(in crate::versions::java_1_21_11::client) mod drop;
pub(in crate::versions::java_1_21_11::client) mod hands;
pub(in crate::versions::java_1_21_11::client) mod recipe_placement;
pub(in crate::versions::java_1_21_11::client) mod transfer;

diagnostic_record! {
    /// One submitted SWAP click, tied to this connection and received baseline.
    /// It is not an acknowledgement and cannot be restored from serialized history.
    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    pub struct InventorySwap => RecordedInventorySwap {
        /// Owning native connection.
        pub connection_id: u64,
        /// Receive boundary immediately before submission.
        pub after_sequence: u64,
        /// Received screen revision sent with the click.
        pub screen_revision: i32,
        /// Main-inventory screen slot, 9..35.
        pub main_slot: u8,
        /// Hotbar index, 0..8 (screen slot 36..44).
        pub hotbar: u8,
        /// Received main-inventory stack before submission.
        pub main_before: InventorySlot,
        /// Received hotbar stack before submission.
        pub hotbar_before: InventorySlot,
    }
    diagnostic_serde {}
}

diagnostic_record! {
    /// Both swap destinations were received after submission with the expected stacks.
    /// This is client receive evidence, not an independent server-side receipt.
    #[derive(Clone, Debug, Serialize)]
    pub struct InventorySwapObservation => RecordedInventorySwapObservation {
        /// Original submission, with its received predecessor.
        pub submission: InventorySwap,
        /// Receive boundary at verification.
        pub receive_sequence: u64,
        /// Received main destination after submission.
        pub main_sequence: u64,
        /// Received hotbar destination after submission.
        pub hotbar_sequence: u64,
    }
    diagnostic_serde {}
}

fn unavailable(message: &str) -> Error {
    crate::error::state_error(message)
}

fn prepare(
    inventory: &Inventory,
    connection_id: u64,
    after_sequence: u64,
    main_slot: u8,
    hotbar: u8,
) -> Result<(InventorySwap, Vec<u8>)> {
    prepare_with_player_revision(
        inventory,
        connection_id,
        after_sequence,
        main_slot,
        hotbar,
        None,
    )
}
// Only the common owner may provide an actual historical player revision after
// a coherently validated, completely dispatched close. Native-only entry stays strict.
fn prepare_with_player_revision(
    inventory: &Inventory,
    connection_id: u64,
    after_sequence: u64,
    main_slot: u8,
    hotbar: u8,
    after_close_revision: Option<i32>,
) -> Result<(InventorySwap, Vec<u8>)> {
    prepare_with_item_data(
        inventory,
        connection_id,
        after_sequence,
        main_slot,
        hotbar,
        after_close_revision,
        false,
    )
}
fn prepare_with_item_data(
    inventory: &Inventory,
    connection_id: u64,
    after_sequence: u64,
    main_slot: u8,
    hotbar: u8,
    after_close_revision: Option<i32>,
    allow_components: bool,
) -> Result<(InventorySwap, Vec<u8>)> {
    if !(9..=35).contains(&main_slot) || hotbar > 8 {
        return Err(invalid("swap requires main slot 9..35 and hotbar 0..8"));
    }
    if inventory.pending_swap.is_some() || !inventory.pending_creative.is_empty() {
        return Err(unavailable("prior inventory mutation needs inspection"));
    }
    if (inventory.window_id != Some(0) && after_close_revision.is_none())
        || inventory.cursor != InventorySlot::Empty
        || inventory.unsupported_components
    {
        return Err(unavailable(
            "received player screen and empty supported cursor required",
        ));
    }
    let screen_revision = after_close_revision
        .or(inventory.screen_revision)
        .ok_or_else(|| unavailable("player screen revision unavailable"))?;
    let main_before = inventory.slots[usize::from(main_slot)].clone();
    let hotbar_before = inventory.slots[36 + usize::from(hotbar)].clone();
    if matches!(main_before, InventorySlot::Unavailable)
        || matches!(hotbar_before, InventorySlot::Unavailable)
    {
        return Err(unavailable("swap slot contents unavailable"));
    }
    for stack in [&main_before, &hotbar_before] {
        if matches!(stack, InventorySlot::ItemWithComponents { .. }) && !allow_components {
            return Err(crate::Error::new(
                crate::ErrorKind::Unsupported,
                anyhow::anyhow!("native default swap does not support component-bearing stacks"),
            ));
        }
        if allow_components {
            let value = super::common_slot(stack)?;
            if let crate::client::SlotKnowledge::Item { item } = value {
                if item.count
                    > u32::try_from(item.properties()?.max_stack_size)
                        .map_err(|_| invalid("invalid effective swap capacity"))?
                {
                    return Err(invalid("swap item exceeds its effective stack capacity"));
                }
            }
        } else if let InventorySlot::Item { item } = stack {
            if items()
                .iter()
                .find(|definition| definition.id == item.item_id)
                .is_none_or(|definition| {
                    item.count <= 0
                        || item.count > definition.stack_size
                        || item.name != format!("minecraft:{}", definition.name)
                })
            {
                return Err(invalid(
                    "ordinary swap requires a valid default stack within its native stack limit",
                ));
            }
        }
    }
    if main_before == hotbar_before {
        return Err(invalid("identical slot contents do not require a swap"));
    }
    let submission = InventorySwap {
        connection_id,
        after_sequence,
        screen_revision,
        main_slot,
        hotbar,
        main_before,
        hotbar_before,
    };
    // Native 1.21.11: container 0, received revision, clicked screen slot,
    // hotbar button, SWAP (2), empty modified-hash map, empty cursor hash.
    // Do not advertise predicted hashes: the native handler sends the actual
    // changed slots against its retained client baseline. A revision mismatch
    // causes a resync AFTER applying the click, not a rejection before mutation.
    let mut payload = vec![0];
    put_varint(&mut payload, screen_revision);
    payload.extend(i16::from(main_slot).to_be_bytes());
    payload.extend([hotbar, 2, 0]);
    put_default_cursor_hash(&mut payload, &InventorySlot::Empty)?;
    Ok((submission, payload))
}

// The native hash of a default stack is its holder ID/count and empty patch
// additions/removals, not a predicted cursor result. Capacity/mode/slot checks
// belong to click admission; the codec itself also represents overstacks.
fn put_default_cursor_hash(payload: &mut Vec<u8>, cursor: &InventorySlot) -> Result<()> {
    match cursor {
        InventorySlot::ItemWithComponents { .. } => {
            return Err(crate::Error::new(
                crate::ErrorKind::Unsupported,
                anyhow::anyhow!("default cursor hash cannot encode a component-bearing stack"),
            ));
        }
        InventorySlot::Empty => payload.push(0),
        InventorySlot::Unavailable => {
            return Err(unavailable("actual supported cursor unavailable"));
        }
        InventorySlot::Item { item } => {
            if item.count <= 0
                || items()
                    .iter()
                    .find(|d| d.id == item.item_id)
                    .is_none_or(|d| item.name != format!("minecraft:{}", d.name) || d.name == "air")
            {
                return Err(invalid(
                    "default cursor hash requires known nonempty native item identity",
                ));
            }
            payload.push(1);
            put_varint(payload, item.item_id);
            put_varint(payload, item.count);
            payload.extend([0, 0]);
        }
    }
    Ok(())
}

fn observed(
    inventory: &Inventory,
    submission: &InventorySwap,
    receive_sequence: u64,
) -> Option<InventorySwapObservation> {
    if inventory.pending_swap.as_ref() != Some(submission)
        || inventory.window_id != Some(0)
        || inventory.cursor != InventorySlot::Empty
        || inventory.unsupported_components
    {
        return None;
    }
    let main = usize::from(submission.main_slot);
    let hotbar = 36 + usize::from(submission.hotbar);
    let main_sequence = inventory.slot_sequences[main]?;
    let hotbar_sequence = inventory.slot_sequences[hotbar]?;
    if main_sequence <= submission.after_sequence
        || hotbar_sequence <= submission.after_sequence
        || inventory.slots[main] != submission.hotbar_before
        || inventory.slots[hotbar] != submission.main_before
    {
        return None;
    }
    Some(InventorySwapObservation {
        submission: submission.clone(),
        receive_sequence,
        main_sequence,
        hotbar_sequence,
    })
}

impl Operations {
    /// Submit one ordinary SWAP click in received survival mode. Both occupied
    /// and empty hotbar destinations are supported for plain stacks.
    /// The pending marker is set before writing and remains on errors/cancellation.
    /// Call `wait_inventory_swap` to establish the received result; do not retry
    /// an unresolved click, since a stale revision does not prevent its execution.
    pub async fn swap_player_hotbar(&self, main_slot: u8, hotbar: u8) -> Result<InventorySwap> {
        let mut state = self.bot.session.state.lock().await;
        self.mutable(&state)?;
        if state.operations.game_mode != Some(GameMode::Survival) {
            return Err(unavailable("swap requires received survival mode"));
        }
        let (submission, payload) = prepare(
            &state.operations.inventory,
            self.bot.session.id,
            state.sequence,
            main_slot,
            hotbar,
        )?;
        let work = self.bot.session.ledger.try_work()?;
        state.pending_swap_work = Some(PendingWork::new(work));
        state.operations.inventory.pending_swap = Some(submission.clone());
        let send_work = ScopedWork::new(
            state
                .pending_swap_work
                .as_ref()
                .expect("pending swap owner")
                .child()
                .expect("pending swap write child"),
        );
        self.bot
            .session
            .send_work(
                ids::play_serverbound::WINDOW_CLICK,
                &payload,
                send_work.guard(),
            )
            .await?;
        Ok(submission)
    }

    /// Wait for both received destination slots, without sending another click.
    /// Timeout or cancellation leaves the swap pending. An owning caller may
    /// resume this read-only wait on the same connection using the same submission.
    pub async fn wait_inventory_swap(
        &self,
        submission: &InventorySwap,
        maximum_wait: Duration,
    ) -> Result<InventorySwapObservation> {
        if maximum_wait.is_zero() || maximum_wait > Duration::from_secs(30) {
            return Err(invalid(
                "inventory wait must be greater than zero and at most 30 seconds",
            ));
        }
        if submission.connection_id != self.bot.session.id {
            return Err(unavailable(
                "inventory submission belongs to another connection",
            ));
        }
        timeout(maximum_wait, async {
            loop {
                let notified = self.bot.session.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut state = self.bot.session.state.lock().await;
                    self.ready(&state)?;
                    if state.common_inventory_swap.as_ref().is_some_and(|s| {
                        s.record.stage
                            != crate::client::inventory::InventorySwapStage::ObservedSwapped
                    }) {
                        return Err(unavailable(
                            "common attempt owns this swap; use inventory_swap_record",
                        ));
                    }
                    if state.operations.game_mode != Some(GameMode::Survival)
                        || state.operations.inventory.pending_swap.as_ref() != Some(submission)
                    {
                        return Err(unavailable(
                            "inventory swap context changed; inspect received state",
                        ));
                    }
                    if let Some(result) =
                        observed(&state.operations.inventory, submission, state.sequence)
                    {
                        state.operations.inventory.pending_swap = None;
                        if let Some(work) = state.pending_swap_work.take() {
                            work.finish(Outcome::Complete);
                        }
                        return Ok(result);
                    }
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| {
            self.bot.session.ledger.fail(Failure::TransactionExpired);
            Error::new(
                ErrorKind::Timeout,
                anyhow::anyhow!(
                    "inventory swap result not received; inspect pending swap without retry"
                ),
            )
        })?
    }
}

fn invalidate(inventory: &mut Inventory, window: Option<i32>) {
    inventory.window_id = window;
    inventory.screen_revision = None;
    inventory.cursor = InventorySlot::Unavailable;
    inventory.cursor_sequence = None;
    inventory.slots.fill(InventorySlot::Unavailable);
    inventory.slot_sequences.fill(None);
}

/// Player-menu slots that regular screen OPEN/CLOSE neither supplies nor alters:
/// the 2x2 crafting result/grid (0..=4), armor (5..=8) and offhand (45). The
/// server keeps the player menu's grid while another menu is open and sends no
/// player-menu contents on close, so dropping these would leave the grid unknown.
pub(crate) const PLAYER_SLOTS_KEPT_BY_SCREENS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 45];

// Keep their last actual values/ordinals, independently of foreign appended-player layouts.
// Unsupported packet invalidation still clears all knowledge; world reset uses Default.
fn invalidate_screen(inventory: &mut Inventory, window: Option<i32>) {
    let kept = PLAYER_SLOTS_KEPT_BY_SCREENS
        .map(|i| (i, inventory.slots[i].clone(), inventory.slot_sequences[i]));
    invalidate(inventory, window);
    for (i, value, sequence) in kept {
        inventory.slots[i] = value;
        inventory.slot_sequences[i] = sequence;
    }
}

fn invalidate_container_contents(inventory: &mut Inventory) {
    if let Some(screen) = &mut inventory.container {
        screen.slots.fill(None);
        screen.full_contents_sequence = None;
        screen.revision = None;
    }
}
fn common_received_slot(
    value: &InventorySlot,
    sequence: u64,
) -> anyhow::Result<Option<crate::client::ObservedValue<crate::client::SlotKnowledge>>> {
    if matches!(value, InventorySlot::Unavailable) {
        return Ok(None);
    }
    Ok(Some(crate::client::received(
        super::common_slot(value)?,
        sequence,
    )))
}

pub(super) fn receive(
    inventory: &mut Inventory,
    id: i32,
    payload: &[u8],
    sequence: u64,
    player_access_after_close: bool,
) -> anyhow::Result<()> {
    use ids::play_clientbound as input;
    let mut r = Reader::new(payload);
    match id {
        input::OPEN_WINDOW => {
            let window = r.count(i32::MAX as usize)? as i32;
            if window == 0 {
                bail!("open container cannot use player screen ID");
            }
            let kind = r.count(i32::MAX as usize)? as i32;
            let title = r.encoded_nbt()?;
            r.end()?;
            invalidate_screen(inventory, Some(window));
            inventory.container = Some(crate::client::container::ScreenReceipts::open(
                crate::MinecraftVersion::Java1_21_11,
                window,
                Some(kind),
                crate::client::container::ScreenTitle::NativeNbt { bytes: title },
                sequence,
            ));
        }
        input::CRAFT_PROGRESS_BAR => {
            // Original ClientboundContainerSetDataPacket: VAR_INT, SHORT, SHORT.
            let window = r.varint()?;
            let property = r.u16()? as i16;
            let value = r.u16()? as i16;
            r.end()?;
            if let Some(properties) = inventory
                .container
                .as_mut()
                .filter(|s| s.window == window)
                .and_then(|s| s.properties.as_mut())
            {
                properties.receive(property, value, sequence);
            }
        }
        input::CLOSE_WINDOW => {
            let window = r.varint()?;
            r.end()?;
            // Received player contents after a submitted close can leave a
            // historical ContainerScreen. Retire its properties independently.
            if let Some(screen) = inventory.container.as_mut().filter(|s| s.window == window) {
                screen.properties = None;
            }
            if inventory.window_id == Some(window) {
                invalidate_screen(inventory, None);
                inventory.container = None;
            }
        }
        input::WINDOW_ITEMS => {
            let window = r.varint()?;
            let revision = r.count(i32::MAX as usize)? as i32;
            let count = r.count(1024)?;
            if window < 0 {
                bail!("negative container ID");
            }
            if window == 0 && count != 46 {
                bail!("unexpected player inventory size");
            }
            if window != 0
                && inventory
                    .container
                    .as_ref()
                    .is_none_or(|screen| screen.window != window)
            {
                return Ok(()); // retired opening; its payload cannot invalidate the new opening
            }
            let mut slots = Vec::with_capacity(count);
            for _ in 0..count {
                let Some(value) = slot(&mut r)? else {
                    invalidate(inventory, inventory.window_id);
                    inventory.unsupported_components = true;
                    invalidate_container_contents(inventory);
                    inventory.receive_sequence = Some(sequence);
                    return Ok(());
                };
                slots.push(value);
            }
            let Some(cursor) = slot(&mut r)? else {
                invalidate(inventory, inventory.window_id);
                inventory.unsupported_components = true;
                invalidate_container_contents(inventory);
                inventory.receive_sequence = Some(sequence);
                return Ok(());
            };
            r.end()?;
            if window == 0 {
                inventory.player_revision = Some(crate::client::received(revision, sequence));
                inventory.slots = slots;
                inventory.slot_sequences.fill(Some(sequence));
                if inventory.window_id.is_none_or(|w| w == 0) || player_access_after_close {
                    inventory.window_id = Some(0);
                    inventory.screen_revision = Some(revision);
                    inventory.cursor = cursor;
                    inventory.cursor_sequence = Some(sequence);
                }
                inventory.pending_creative.clear();
                inventory.unsupported_components = false;
            } else if inventory
                .container
                .as_ref()
                .is_some_and(|screen| screen.window == window)
            {
                let values = slots
                    .iter()
                    .map(|value| common_received_slot(value, sequence))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let screen = inventory.container.as_mut().expect("matching opening");
                screen.full_items(values, Some(revision), sequence)?;
                if let Some(layout) = &screen.layout {
                    for mapping in &layout.player_slots {
                        inventory.slots[mapping.player_slot] = slots[mapping.screen_slot].clone();
                        inventory.slot_sequences[mapping.player_slot] = Some(sequence);
                        screen.slots[mapping.screen_slot] = None;
                    }
                }
                inventory.cursor = cursor;
                inventory.cursor_sequence = Some(sequence);
                inventory.unsupported_components = false;
            }
            // Untagged contents for a different/retired window cannot establish
            // an opening or invalidate the currently received player inventory.
            inventory.receive_sequence = Some(sequence);
        }
        input::SET_CURSOR_ITEM => {
            let value = slot(&mut r)?;
            inventory.cursor = if let Some(value) = value {
                r.end()?;
                value
            } else {
                inventory.unsupported_components = true;
                InventorySlot::Unavailable
            };
            inventory.cursor_sequence =
                (!matches!(inventory.cursor, InventorySlot::Unavailable)).then_some(sequence);
            inventory.receive_sequence = Some(sequence);
        }
        input::SET_SLOT | input::SET_PLAYER_INVENTORY => {
            let (window, revision, index) = if id == input::SET_SLOT {
                let window = r.varint()?;
                if window < 0 {
                    bail!("negative container ID");
                }
                let revision = r.count(i32::MAX as usize)? as i32;
                let index = usize::try_from(r.u16()? as i16).context("negative screen slot")?;
                (window, Some(revision), index)
            } else {
                let index = match r.count(40)? {
                    v @ 0..=8 => v + 36,
                    v @ 9..=35 => v,
                    v @ 36..=39 => 44 - v,
                    40 => 45,
                    _ => unreachable!(),
                };
                (0, None, index)
            };
            if (window == 0 && index >= 46) || index >= 4096 {
                bail!("invalid screen slot");
            }
            if window != 0
                && inventory
                    .container
                    .as_ref()
                    .is_none_or(|screen| screen.window != window)
            {
                return Ok(());
            }
            let decoded = slot(&mut r)?;
            let value = if let Some(value) = decoded {
                r.end()?;
                value
            } else {
                inventory.unsupported_components = true;
                InventorySlot::Unavailable
            };
            if window == 0 {
                if let Some(revision) = revision {
                    inventory.player_revision = Some(crate::client::received(revision, sequence));
                }
                if inventory.window_id == Some(0) && revision.is_some() {
                    inventory.screen_revision = revision;
                }
                inventory.slots[index] = value.clone();
                inventory.slot_sequences[index] = Some(sequence);
                if !matches!(value, InventorySlot::Unavailable) {
                    inventory
                        .pending_creative
                        .retain(|s| 36 + usize::from(*s) != index);
                }
                // A raw player update is an actual value for the corresponding
                // slot in the active menu, but does not invent a menu revision.
                if id == input::SET_PLAYER_INVENTORY {
                    let screen_index = inventory
                        .container
                        .as_ref()
                        .and_then(|screen| screen.layout.as_ref())
                        .and_then(|layout| {
                            layout.player_slots.iter().find(|m| m.player_slot == index)
                        })
                        .map(|m| m.screen_slot);
                    if let Some(screen_index) = screen_index {
                        let _ = common_received_slot(&value, sequence)?;
                        inventory.container.as_mut().expect("active layout").slot(
                            screen_index,
                            None,
                            None,
                            sequence,
                        )?;
                    }
                }
            } else if inventory
                .container
                .as_ref()
                .is_some_and(|screen| screen.window == window)
            {
                let received = common_received_slot(&value, sequence)?;
                let screen = inventory.container.as_mut().expect("matching opening");
                let shared = screen
                    .layout
                    .as_ref()
                    .is_some_and(|l| l.player_slots.iter().any(|m| m.screen_slot == index));
                screen.slot(
                    index,
                    if shared { None } else { received },
                    revision,
                    sequence,
                )?;
                if let Some(mapping) = screen
                    .layout
                    .as_ref()
                    .and_then(|layout| layout.player_slots.iter().find(|m| m.screen_slot == index))
                {
                    inventory.slots[mapping.player_slot] = value;
                    inventory.slot_sequences[mapping.player_slot] = Some(sequence);
                }
            } else if inventory.container.is_none() {
                // A foreign menu update without an observed opening preserves
                // uncertainty, without inventing metadata or a player layout.
                invalidate(inventory, Some(window));
            }
            inventory.receive_sequence = Some(sequence);
        }
        _ => unreachable!("inventory packet dispatch"),
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub(in crate::versions::java_1_21_11::client) fn creative_pending(state: &State, slot: u8) -> bool {
    state.operations.inventory.pending_creative.contains(&slot)
}

pub(in crate::versions::java_1_21_11::client) fn execution_scope(
    state: &State,
) -> (
    crate::client::SessionStamp,
    crate::client::inventory::InventorySource,
) {
    use crate::client::inventory::InventorySource;
    let session = crate::client::SessionStamp {
        version: crate::MinecraftVersion::Java1_21_11,
        connection_id: state.connection_id,
        world_generation: state.loading.generation,
    };
    let mut source =
        state
            .operations
            .inventory
            .container
            .as_ref()
            .map_or(InventorySource::Player, |s| InventorySource::Container {
                screen: s.id(session),
            });
    if state.operations.inventory.window_id == Some(0)
        || matches!(
            crate::client::container::player_screen_access(
                session,
                state.operations.inventory.window_id,
                state
                    .operations
                    .inventory
                    .container
                    .as_ref()
                    .map(|s| s.id(session)),
                state.common_container_close.as_ref()
            ),
            Some(crate::client::container::PlayerScreenAccess::SubmittedClose { .. })
        )
    {
        source = InventorySource::Player;
    }
    (session, source)
}
impl Operations {
    pub(super) fn begin_inventory_execution(
        &self,
        state: &mut State,
        session: crate::client::SessionStamp,
        resync: bool,
    ) -> Result<crate::client::inventory::InventorySendId> {
        let (current, source) = execution_scope(state);
        if current != session {
            return Err(unavailable("inventory generation changed before send"));
        }
        state.inventory_executor.begin(
            session,
            source,
            state.sequence,
            None,
            self.bot.session.limits.inventory_click_receipt_timeout,
            resync,
        )
    }
    pub(super) async fn send_inventory_fence(
        &self,
        state: &mut State,
        id: crate::client::inventory::InventorySendId,
    ) -> Result<()> {
        state
            .inventory_executor
            .written(id, self.bot.session.limits.inventory_click_receipt_timeout);
        use crate::client::ledger::finite::Family;
        let after = state
            .inventory_executor
            .record()
            .expect("retained send")
            .after_sequence;
        let deadline =
            tokio::time::Instant::now() + self.bot.session.limits.inventory_click_receipt_timeout;
        for (family, entry) in [
            (
                Family::Click,
                state
                    .common_inventory_click
                    .as_ref()
                    .map(|r| (r.id.attempt(), r.send.after_sequence)),
            ),
            (
                Family::Transfer,
                state
                    .common_inventory_transfer
                    .as_ref()
                    .map(|r| (r.id.attempt(), r.send.after_sequence)),
            ),
            (
                Family::Swap,
                state
                    .common_inventory_swap
                    .as_ref()
                    .map(|r| (r.record.id.attempt(), r.record.send.after_sequence)),
            ),
        ] {
            if let Some((attempt, _)) = entry.filter(|(_, boundary)| *boundary == after) {
                state.finite_work.expire_at(family, attempt, deadline);
            }
        }
        if state.inventory_executor.record().is_some_and(|r| r.resync) {
            state
                .finite_work
                .expire_at(Family::InventoryResync, id.number(), deadline);
        }

        let token = i32::try_from(id.number())
            .map_err(|_| unavailable("inventory fence identities exhausted"))?;
        let mut payload = Vec::new();
        put_varint(&mut payload, token);
        put_string(&mut payload, "/");
        let result = self
            .bot
            .session
            .send(ids::play_serverbound::TAB_COMPLETE, &payload)
            .await;
        if result.is_err() {
            state.inventory_executor.invalidate_owned(id);
        }
        result
    }
}
pub(in crate::versions::java_1_21_11::client) fn execution_received(
    state: &mut State,
    id: i32,
    payload: &[u8],
) -> anyhow::Result<()> {
    use ids::play_clientbound as input;
    let (session, source) = execution_scope(state);
    match id {
        input::OPEN_WINDOW | input::RESPAWN | input::START_CONFIGURATION => {
            state.inventory_executor.invalidate()
        }
        input::CLOSE_WINDOW => {
            let window = Reader::new(payload).varint()?;
            if state
                .inventory_executor
                .record()
                .is_some_and(|r| r.window() == window)
            {
                state.inventory_executor.invalidate();
            }
        }
        input::WINDOW_ITEMS => {
            let mut r = Reader::new(payload);
            let window = r.varint()?;
            state
                .inventory_executor
                .contents(session, source, window, state.sequence, true);
        }
        input::COLLECT => {
            let mut reader = Reader::new(payload);
            let entity = reader.varint()?;
            let collector = reader.varint()?;
            if state.operations.local_player.entity_id == Some(collector) {
                state
                    .inventory_executor
                    .changes
                    .collect(entity, state.sequence);
                state.inventory_executor.collected(session, source, entity);
            }
        }
        input::TAB_COMPLETE => {
            let token = u64::try_from(Reader::new(payload).varint()?)
                .context("invalid inventory fence token")?;
            state
                .inventory_executor
                .fence(session, source, token, state.sequence);
        }
        _ => {}
    }
    Ok(())
}
pub(in crate::versions::java_1_21_11::client) fn retire_execution(state: &mut State) {
    if let Some(execution) = state
        .inventory_executor
        .record()
        .filter(|r| r.status == crate::client::inventory::InventorySendStatus::Received)
    {
        if state.common_inventory_swap.as_ref().is_some_and(|s| {
            s.record.send.after_sequence == execution.after_sequence
                && s.record.id.session() == execution.id.session()
        }) {
            state.operations.inventory.pending_swap = None;
            if let Some(work) = state.pending_swap_work.take() {
                work.finish(Outcome::Complete);
            }
        }
    }
}
impl crate::client::adapter::InventorySendOps for Operations {
    async fn send_crafting_result(
        &self,
        mode: crate::client::GameMode,
        grid: &crate::client::crafting::ReceivedCrafting,
        destination: crate::client::crafting::CraftingResultDestination,
    ) -> Result<crate::client::inventory::InventorySendRecord> {
        Ok(self
            .start_crafting_take(mode, grid, destination, None, true)
            .await?
            .1)
    }

    async fn send_inventory_click(
        &self,
        mode: crate::client::GameMode,
        source: crate::client::inventory::InventorySource,
        slot: u16,
        button: crate::client::inventory::InventoryClickButton,
    ) -> Result<crate::client::inventory::InventorySendRecord> {
        Ok(self
            .start_click_inventory(mode, source, slot, button, true)
            .await?
            .1)
    }
    async fn send_inventory_transfer(
        &self,
        mode: crate::client::GameMode,
        source: crate::client::inventory::InventorySource,
        slot: u16,
    ) -> Result<crate::client::inventory::InventorySendRecord> {
        Ok(self
            .start_transfer_inventory(mode, source, slot, true)
            .await?
            .1)
    }
    async fn inventory_send_record(
        &self,
    ) -> Result<Option<crate::client::inventory::InventorySendRecord>> {
        let mut state = self.bot.session.state.lock().await;
        let record = state.inventory_executor.record();
        state.reconcile_finite();
        Ok(record)
    }
    async fn resync_inventory(&self) -> Result<crate::client::inventory::InventorySendRecord> {
        let mut state = self.bot.session.state.lock().await;
        self.ready(&state)?;
        let (session, source) = execution_scope(&state);
        let window = match source {
            crate::client::inventory::InventorySource::Player => 0,
            crate::client::inventory::InventorySource::Container { screen } => screen.window_id(),
        };
        let revision = if window == 0 {
            state.operations.inventory.screen_revision
        } else {
            state
                .operations
                .inventory
                .container
                .as_ref()
                .and_then(|s| s.revision.as_ref().map(|v| v.value))
        }
        .ok_or_else(|| unavailable("resync revision unavailable"))?;
        let execution = self.begin_inventory_execution(&mut state, session, true)?;
        let mut bytes = Vec::new();
        put_varint(&mut bytes, window);
        put_varint(&mut bytes, if revision == 0 { 1 } else { 0 });
        bytes.extend((-1i16).to_be_bytes());
        bytes.extend([0, 0, 0]);
        put_default_cursor_hash(&mut bytes, &InventorySlot::Empty)?;
        let work = state.finite_work.begin(
            &self.bot.session.ledger,
            crate::client::ledger::finite::Family::InventoryResync,
            execution.number(),
        );
        if let Err(e) = work
            .run(
                self.bot
                    .session
                    .send(ids::play_serverbound::WINDOW_CLICK, &bytes),
            )
            .await
        {
            state.inventory_executor.invalidate_owned(execution);
            return Err(e);
        }
        self.send_inventory_fence(&mut state, execution).await?;
        Ok(state.inventory_executor.record().expect("resync retained"))
    }
}

impl Inventory {
    pub(crate) fn screen(
        &self,
        session: crate::client::SessionStamp,
    ) -> Option<crate::client::container::ContainerScreen> {
        let player = self
            .slots
            .iter()
            .zip(&self.slot_sequences)
            .map(|(v, s)| {
                s.and_then(|s| {
                    super::common_slot(v)
                        .ok()
                        .map(|v| crate::client::received(v, s))
                })
            })
            .collect::<Vec<_>>();
        self.container.as_ref().map(|s| s.project(session, &player))
    }
}

pub(in crate::versions::java_1_21_11::client) fn changes_received(
    state: &mut State,
    id: i32,
    in_play: bool,
) -> Result<()> {
    use ids::play_clientbound as input;
    if in_play {
        // Every received packet: open own-action windows before any later
        // selection write can make the main-hand slot ambiguous.
        let session = state.snapshot_session();
        let selected = state.operations.selected_hotbar.as_ref().map(|s| s.slot);
        state
            .inventory_executor
            .own_actions_received(session, selected);
    }
    if !in_play
        || !matches!(
            id,
            input::WINDOW_ITEMS
                | input::SET_SLOT
                | input::SET_PLAYER_INVENTORY
                | input::SET_CURSOR_ITEM
                | input::OPEN_WINDOW
                | input::CLOSE_WINDOW
                | input::LOGIN
                | input::RESPAWN
                | input::START_CONFIGURATION
                | input::TAB_COMPLETE
        )
    {
        return Ok(());
    }
    let player = super::common_player_in_state(state, state.connection_id, false)?;
    let screen = state.operations.inventory.screen(player.session);
    let current = crate::client::inventory::InventorySnapshot::capture(&player, screen);
    let crafting_complete = state.common_crafting_take.as_ref().is_some_and(|r| {
        matches!(
            r.stage,
            crate::client::crafting::CraftingTakeStage::ObservedTaken
                | crate::client::crafting::CraftingTakeStage::ObservedTransferred
        )
    });
    state.inventory_executor.receive(current, crafting_complete);
    Ok(())
}
