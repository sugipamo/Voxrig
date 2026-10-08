//! Planned top-level cursor return. Every executed step retains actual predecessors.
use super::*;
use crate::client::{
    PlayerObservation, ValueSource,
    inventory::{
        self as inventory, InventoryClickButton, InventoryClickSource, InventoryClickStage, click,
        slot_policy,
    },
};

pub(crate) const RECEIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// One before-I/O cursor-return destination. Predictions are never receipts.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CursorReturnPlanStep {
    /// Original native screen slot, from its verified constructor mapping.
    pub screen_slot: u16,
    /// Canonical player main/hotbar slot.
    pub player_slot: usize,
    /// Actual predecessor captured before the whole close.
    pub source_before: ObservedValue<SlotKnowledge>,
    /// Planned destination after the step, always Predicted.
    pub source_prediction: ObservedValue<SlotKnowledge>,
    /// Planned carried predecessor, always Predicted; execution re-captures actual cursor.
    pub cursor_prediction_before: ObservedValue<SlotKnowledge>,
    /// Planned carried result, always Predicted.
    pub cursor_prediction_after: ObservedValue<SlotKnowledge>,
}
fn predicted(value: SlotKnowledge) -> ObservedValue<SlotKnowledge> {
    ObservedValue {
        value,
        source: ValueSource::Predicted,
    }
}
fn known(value: Option<&ObservedValue<SlotKnowledge>>) -> Option<&ObservedValue<SlotKnowledge>> {
    value.filter(|v| {
        matches!(v.source, ValueSource::Received { .. }) && v.value != SlotKnowledge::Unavailable
    })
}
pub(super) fn plan(
    initial: &PlayerObservation,
    screen: &ContainerScreen,
    context: Option<&inventory::ItemContext>,
) -> crate::Result<Vec<CursorReturnPlanStep>> {
    let cursor = known(initial.inventory.cursor.as_ref())
        .ok_or_else(|| inventory::unavailable("close requires actual known cursor"))?;
    if cursor.value == SlotKnowledge::Empty {
        return Ok(Vec::new());
    }
    if !initial
        .health
        .as_ref()
        .is_some_and(|h| matches!(h.source, ValueSource::Received { .. }) && h.value.health > 0.0)
    {
        return Err(inventory::unavailable(
            "cursor return requires received living player",
        ));
    }
    let layout = screen.layout.as_ref().ok_or_else(|| {
        inventory::unavailable("cursor return requires constructor-verified player mapping")
    })?;
    let menu = screen
        .menu_name
        .as_deref()
        .filter(|m| {
            inventory::storage_menu(m)
                || *m == "minecraft:crafting"
                || crate::client::furnace::native_menu(initial.session.version, m).is_some()
        })
        .ok_or_else(|| inventory::unavailable("cursor return requires audited container menu"))?;
    if screen.full_contents_sequence.is_none() || screen.slots.len() != layout.total_slots {
        return Err(inventory::unavailable(
            "cursor return requires actual full screen",
        ));
    }
    let mut mappings = layout.player_slots.clone();
    mappings.sort_by_key(|m| m.player_slot);
    if mappings.len() != 36 || !mappings.iter().map(|m| m.player_slot).eq(9..45) {
        return Err(inventory::unavailable(
            "cursor return native main/hotbar mapping incomplete",
        ));
    }
    let mut remaining = cursor.value.clone();
    let mut output = Vec::new();
    // Match exact stack identity/data first, then known empty top-level slots.
    // Missing or unavailable knowledge is never treated as free capacity.
    for empty in [false, true] {
        for mapping in &mappings {
            if remaining == SlotKnowledge::Empty {
                return Ok(output);
            }
            let Some(source) = known(
                screen
                    .slots
                    .get(mapping.screen_slot)
                    .and_then(Option::as_ref),
            ) else {
                continue;
            };
            if screen.slots.get(mapping.screen_slot)
                != initial.inventory.slots.get(mapping.player_slot)
            {
                return Err(inventory::unavailable(
                    "cursor return screen/player receipt projection disagrees",
                ));
            }
            let SlotKnowledge::Item { item: carried } = &remaining else {
                unreachable!()
            };
            let suitable = match &source.value {
                SlotKnowledge::Empty => empty,
                SlotKnowledge::Item { item } => {
                    let same = if let Some(context) = context {
                        context.same_data(item, carried)?
                    } else {
                        item.id == carried.id
                            && item.name == carried.name
                            && item.data == carried.data
                    };
                    let capacity = if context.is_some() {
                        u32::try_from(item.properties()?.max_stack_size).ok()
                    } else {
                        slot_policy::default_item_capacity(
                            initial.session.version,
                            item.id.value(),
                            &item.name,
                        )
                    };
                    !empty && same && capacity.is_some_and(|max| item.count < max)
                }
                _ => false,
            };
            if !suitable {
                continue;
            }
            let (after, cursor_after) = if let Some(context) = context {
                slot_policy::pickup_with_data(
                    initial.session.version,
                    menu,
                    mapping.screen_slot,
                    InventoryClickButton::Left,
                    (&source.value, &remaining),
                    context,
                )?
            } else {
                slot_policy::pickup(
                    initial.session.version,
                    menu,
                    mapping.screen_slot,
                    InventoryClickButton::Left,
                    &source.value,
                    &remaining,
                )?
            };
            if cursor_after == remaining {
                continue;
            }
            output.push(CursorReturnPlanStep {
                screen_slot: u16::try_from(mapping.screen_slot).map_err(|_| {
                    inventory::unavailable("cursor return screen slot out of range")
                })?,
                player_slot: mapping.player_slot,
                source_before: source.clone(),
                source_prediction: predicted(after),
                cursor_prediction_before: predicted(remaining),
                cursor_prediction_after: predicted(cursor_after.clone()),
            });
            remaining = cursor_after;
        }
    }
    if remaining != SlotKnowledge::Empty {
        return Err(crate::client::registry::invalid(
            "no known sufficient top-level player inventory capacity; close not submitted",
        ));
    }
    Ok(output)
}
impl ContainerCloseRecord {
    pub(crate) fn return_complete(&self) -> bool {
        self.return_steps.len() == self.return_plan.len()
            && self
                .return_steps
                .iter()
                .all(|s| s.stage == InventoryClickStage::ObservedClicked)
    }
    #[cfg(test)]
    pub(crate) fn begin_return_step(
        &mut self,
        current: PlayerObservation,
        screen: ContainerScreen,
    ) -> crate::Result<()> {
        self.begin_return_step_inner(current, screen, None)
    }
    pub(crate) fn begin_return_step_received(
        &mut self,
        current: PlayerObservation,
        screen: ContainerScreen,
        registries: crate::client::registry::ServerRegistryObservation,
    ) -> crate::Result<()> {
        self.begin_return_step_inner(current, screen, Some(registries))
    }
    fn begin_return_step_inner(
        &mut self,
        mut current: PlayerObservation,
        screen: ContainerScreen,
        registries: Option<crate::client::registry::ServerRegistryObservation>,
    ) -> crate::Result<()> {
        self.return_received_inner(&current, Some(&screen), registries.as_ref());
        if self.requires_inspection.is_some()
            || self
                .return_steps
                .last()
                .is_some_and(|s| s.stage != InventoryClickStage::ObservedClicked)
        {
            return Err(inventory::unavailable(
                "close return predecessor/previous step unresolved",
            ));
        }
        let index = self.return_steps.len();
        let plan = self
            .return_plan
            .get(index)
            .ok_or_else(|| inventory::unavailable("no further close return step"))?;
        // This internal preparation excludes only its own retained close marker;
        // outer adapters hold the exclusive parent and reject other mutations.
        current.pending_dispatch = false;
        let mut step = if let Some(registries) = registries {
            click::prepare_received(
                (current, registries),
                self.mode,
                InventoryClickSource::Container {
                    screen: self.id.screen(),
                },
                plan.screen_slot,
                InventoryClickButton::Left,
                index as u64 + 1,
                Some(screen),
            )?
        } else {
            click::prepare(
                current,
                self.mode,
                InventoryClickSource::Container {
                    screen: self.id.screen(),
                },
                plan.screen_slot,
                InventoryClickButton::Left,
                index as u64 + 1,
                Some(screen),
            )?
        };
        step.bind_close(self.id);
        let matches = if let Some(context) = &self.item_context {
            context.equivalent_values(
                &step.source_before.value,
                &plan.source_before.value,
                step.source_before.source == plan.source_before.source,
            )? && context.equivalent_values(
                &step.cursor_before.value,
                &plan.cursor_prediction_before.value,
                false,
            )? && context.equivalent_values(
                &step.prediction.source.value,
                &plan.source_prediction.value,
                false,
            )? && context.equivalent_values(
                &step.prediction.cursor.value,
                &plan.cursor_prediction_after.value,
                false,
            )?
        } else {
            step.source_before.value == plan.source_before.value
                && step.cursor_before.value == plan.cursor_prediction_before.value
                && step.prediction.source.value == plan.source_prediction.value
                && step.prediction.cursor.value == plan.cursor_prediction_after.value
        };
        if !matches {
            return Err(inventory::unavailable(
                "close return actual predecessor no longer matches retained plan",
            ));
        }
        self.return_steps.push(step);
        self.stage = ContainerCloseStage::ReturningCursor;
        Ok(())
    }
    pub(crate) fn return_reply(&mut self, reply: inventory::InventoryTransactionReply) {
        let Some(step) = self.return_steps.last_mut().filter(|s| s.unresolved()) else {
            return;
        };
        if step.send.legacy_action == Some(reply.action)
            && step.window_id() == i32::from(reply.window_id)
            && reply.receive_sequence > step.send.after_sequence
        {
            step.legacy_reply.get_or_insert(reply);
        }
    }
    #[cfg(test)]
    pub(crate) fn return_received(
        &mut self,
        current: &PlayerObservation,
        screen: Option<&ContainerScreen>,
    ) {
        self.return_received_inner(current, screen, None)
    }
    pub(crate) fn return_received_with_registries(
        &mut self,
        current: &PlayerObservation,
        screen: Option<&ContainerScreen>,
        registries: &crate::client::registry::ServerRegistryObservation,
    ) {
        self.return_received_inner(current, screen, Some(registries))
    }
    fn return_received_inner(
        &mut self,
        current: &PlayerObservation,
        screen: Option<&ContainerScreen>,
        registries: Option<&crate::client::registry::ServerRegistryObservation>,
    ) {
        if !matches!(
            self.stage,
            ContainerCloseStage::Pending | ContainerCloseStage::ReturningCursor
        ) {
            return;
        }
        if self.item_context.is_some()
            && registries.is_none_or(|r| {
                r.session() != current.session || r.receive_sequence() != current.receive_sequence
            })
        {
            self.inspection("close return registry capture disagrees with actual inventory");
            return;
        }
        self.context_received(
            current.session,
            current.game_mode,
            screen.map(|s| s.id),
            current.inventory.cursor.as_ref(),
            registries,
        );
        if self.requires_inspection.is_some() || self.return_plan.is_empty() {
            return;
        }
        let Some(screen) = screen.filter(|s| {
            s.layout == self.initial_screen.layout
                && s.menu_name == self.initial_screen.menu_name
                && s.full_contents_sequence.is_some()
                && s.slots.len() == self.initial_screen.slots.len()
        }) else {
            self.inspection("close return original menu/layout/full context changed");
            return;
        };
        if current.selected_hotbar.as_ref().map(|s| s.value)
            != self.initial.selected_hotbar.as_ref().map(|s| s.value)
            || !current.health.as_ref().is_some_and(|h| {
                matches!(h.source, ValueSource::Received { .. }) && h.value.health > 0.0
            })
            || self.initial.received_pose.as_ref().is_some_and(|p| {
                current
                    .received_pose
                    .as_ref()
                    .is_none_or(|q| p.position != q.position || p.rotation != q.rotation)
            })
        {
            self.inspection("close return selected hand/health/received pose changed");
            return;
        }
        for (index, before) in self.initial_screen.slots.iter().enumerate() {
            let Some(before) = known(before.as_ref()) else {
                continue;
            };
            let latest = self
                .return_steps
                .iter()
                .rev()
                .find(|s| usize::from(s.source_slot) == index);
            let expected = latest.map_or(&before.value, |s| {
                if s.stage == InventoryClickStage::ObservedClicked {
                    &s.prediction.source.value
                } else {
                    &s.source_before.value
                }
            });
            let actual = known(screen.slots.get(index).and_then(Option::as_ref));
            let active_after = latest
                .filter(|s| s.unresolved())
                .map(|s| &s.prediction.source.value);
            let matches = actual.is_some_and(|v| {
                if let Some(context) = &self.item_context {
                    registries
                        .filter(|r| {
                            r.session() == current.session
                                && r.receive_sequence() == current.receive_sequence
                        })
                        .and_then(|r| context.classify(v, before, expected, r).ok())
                        .is_some_and(|(same, after)| after || (expected == &before.value && same))
                        || active_after.is_some_and(|after| {
                            registries
                                .and_then(|r| context.classify(v, before, after, r).ok())
                                .is_some_and(|(_, after)| after)
                        })
                } else {
                    &v.value == expected || active_after.is_some_and(|after| &v.value == after)
                }
            });
            if !matches {
                self.inspection("close return known source/destination/unaffected slot changed");
                return;
            }
        }
        // Equipment/crafting/offhand are outside this storage menu's appended
        // main/hotbar projection, but remain actual unaffected parent context.
        for (index, before) in self.initial.inventory.slots.iter().enumerate() {
            if (9..45).contains(&index) {
                continue;
            }
            if let Some(before) = known(before.as_ref()) {
                let same = known(current.inventory.slots.get(index).and_then(Option::as_ref))
                    .is_some_and(|actual| {
                        if let Some(context) = &self.item_context {
                            registries
                                .and_then(|r| {
                                    context.classify(actual, before, &before.value, r).ok()
                                })
                                .is_some_and(|(same, _)| same)
                        } else {
                            actual.value == before.value
                        }
                    });
                if !same {
                    self.inspection("close return unaffected player equipment/offhand changed");
                    return;
                }
            }
        }
        if let Some(layout) = screen.layout.as_ref() {
            for mapping in &layout.player_slots {
                if screen.slots.get(mapping.screen_slot)
                    != current.inventory.slots.get(mapping.player_slot)
                {
                    self.inspection("close return actual player projection changed");
                    return;
                }
            }
        }
        if let Some(step) = self.return_steps.last_mut().filter(|s| s.unresolved()) {
            if let Some(registries) = registries {
                click::receive_with_registries(step, current, Some(screen), registries);
            } else {
                click::receive(step, current, Some(screen));
            }
            if let Some(reason) = step.requires_inspection.clone() {
                self.inspection(reason);
            } else if step.ready() {
                step.stage = InventoryClickStage::ObservedClicked;
            }
        }
    }
    pub(super) fn valid_cursor(
        &self,
        cursor: Option<&ObservedValue<SlotKnowledge>>,
        registries: Option<&crate::client::registry::ServerRegistryObservation>,
    ) -> bool {
        let Some(cursor) = known(cursor) else {
            return false;
        };
        if let Some(context) = &self.item_context {
            let Some(registries) = registries else {
                return false;
            };
            if let Some(step) = self.return_steps.last() {
                return context
                    .classify(
                        cursor,
                        &step.cursor_before,
                        &step.prediction.cursor.value,
                        registries,
                    )
                    .is_ok_and(|(before, after)| after || (step.unresolved() && before));
            }
            return self
                .initial
                .inventory
                .cursor
                .as_ref()
                .is_some_and(|before| {
                    context
                        .classify(cursor, before, &before.value, registries)
                        .is_ok_and(|(same, _)| same)
                });
        }
        if let Some(step) = self.return_steps.last() {
            cursor.value == step.prediction.cursor.value
                || (step.unresolved() && cursor.value == step.cursor_before.value)
        } else {
            self.initial
                .inventory
                .cursor
                .as_ref()
                .is_some_and(|before| cursor.value == before.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        GameMode, Health, InventoryObservation, ItemData, ItemStack, registry::Registry,
    };
    fn item(version: MinecraftVersion, name: &str, count: u32) -> SlotKnowledge {
        let definition = Registry::for_version(version)
            .item(&format!("minecraft:{name}"))
            .unwrap();
        SlotKnowledge::Item {
            item: ItemStack {
                id: definition.id,
                name: definition.name,
                count,
                data: ItemData::Default,
            },
        }
    }
    fn fixture(version: MinecraftVersion, mode: GameMode) -> (PlayerObservation, ContainerScreen) {
        let session = SessionStamp {
            version,
            connection_id: 42,
            world_generation: 1,
        };
        let mut screen = ScreenReceipts::open(version, 3, Some(2), ScreenTitle::Unavailable, 8);
        screen
            .full_items(
                vec![Some(received(SlotKnowledge::Empty, 10)); 63],
                (version == MinecraftVersion::Java1_21_11).then_some(7),
                10,
            )
            .unwrap();
        let mut screen = screen.capture(session);
        screen.slots[27] = Some(received(item(version, "stone", 63), 10));
        let mut slots = vec![Some(received(SlotKnowledge::Empty, 10)); 46];
        slots[9] = screen.slots[27].clone();
        let initial = PlayerObservation {
            using_item: None,
            entity_id: None,
            attributes: Default::default(),
            effects: Default::default(),
            air_supply: None,
            session,
            receive_sequence: 10,
            pending_dispatch: false,
            dimension: None,
            position: None,
            received_pose: None,
            rotation: [0., 0.],
            game_mode: Some(mode),
            may_fly: None,
            health: Some(received(
                Health {
                    health: 20.,
                    food: 20,
                    saturation: 5.,
                },
                10,
            )),
            selected_hotbar: Some(received(0, 10)),
            inventory: InventoryObservation {
                slots,
                cursor: Some(received(item(version, "stone", 5), 10)),
                window_id: Some(3),
                player_screen: None,
                screen_revision: Some(7),
                player_screen_revision: None,
                local_cache: None,
            },
        };
        (initial, screen)
    }
    fn record(version: MinecraftVersion, mode: GameMode) -> ContainerCloseRecord {
        let (initial, screen) = fixture(version, mode);
        prepare_close(initial, screen.clone(), screen.id, mode, None).unwrap()
    }
    #[test]
    fn multi_step_return_retains_actual_predecessors_and_requires_full_write_and_fresh_receipts() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                let mut record = record(version, mode);
                assert_eq!(record.return_plan.len(), 2);
                assert_eq!(record.return_plan[0].player_slot, 9);
                assert_eq!(record.return_plan[1].player_slot, 10);
                let mut current = record.initial.clone();
                let mut screen = record.initial_screen.clone();
                for index in 0..2 {
                    record
                        .begin_return_step(current.clone(), screen.clone())
                        .unwrap();
                    let step = record.return_steps.last_mut().unwrap();
                    assert_eq!(step.id.close(), Some(record.id));
                    assert!(matches!(
                        step.source_before.source,
                        ValueSource::Received { .. }
                    ));
                    assert!(matches!(
                        step.cursor_before.source,
                        ValueSource::Received { .. }
                    ));
                    assert!(matches!(
                        step.prediction.cursor.source,
                        ValueSource::Predicted
                    ));
                    step.send.legacy_action =
                        (version == MinecraftVersion::Java1_16_1).then_some(index as i16 + 1);
                    let slot = usize::from(step.source_slot);
                    let player_slot = record.return_plan[index].player_slot;
                    let sequence = current.receive_sequence + 1;
                    let source_after = step.prediction.source.value.clone();
                    let cursor_after = step.prediction.cursor.value.clone();
                    record.return_received(&current, Some(&screen));
                    assert!(!record.return_complete());
                    record.return_steps[index].send.dispatched = true;
                    screen.slots[slot] = Some(received(source_after, sequence));
                    current.inventory.slots[player_slot] = screen.slots[slot].clone();
                    current.inventory.cursor = Some(received(cursor_after, sequence));
                    current.receive_sequence = sequence;
                    if version == MinecraftVersion::Java1_16_1 {
                        record.return_reply(inventory::InventoryTransactionReply {
                            window_id: 3,
                            action: index as i16 + 1,
                            accepted: false,
                            receive_sequence: sequence,
                        });
                    }
                    record.return_received(&current, Some(&screen));
                    assert_eq!(
                        record.return_steps[index].stage,
                        InventoryClickStage::ObservedClicked
                    );
                }
                assert!(record.return_complete());
                assert!(!record.dispatched);
                assert_eq!(
                    current.inventory.cursor.as_ref().unwrap().value,
                    SlotKnowledge::Empty
                );
                record.sent();
                assert_eq!(record.stage, ContainerCloseStage::Dispatched);
                assert!(record.server_close_sequence.is_none());
            }
        }
    }
    #[test]
    fn unknown_capacity_never_means_empty_and_full_bundle_never_means_insert() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let (mut current, mut screen) = fixture(version, GameMode::Survival);
            for mapping in screen.layout.as_ref().unwrap().player_slots.clone() {
                screen.slots[mapping.screen_slot] = None;
                current.inventory.slots[mapping.player_slot] = None;
            }
            assert!(plan(&current, &screen, None).is_err());
            screen.slots[28] = Some(received(SlotKnowledge::Empty, 10));
            current.inventory.slots[10] = screen.slots[28].clone();
            assert_eq!(plan(&current, &screen, None).unwrap().len(), 1);
        }
        let version = MinecraftVersion::Java1_21_11;
        let (mut current, mut screen) = fixture(version, GameMode::Creative);
        current.inventory.cursor = Some(received(item(version, "bundle", 1), 10));
        screen.slots[27] = Some(received(item(version, "bundle", 1), 10));
        current.inventory.slots[9] = screen.slots[27].clone();
        let plan = plan(&current, &screen, None).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].screen_slot, 28);
    }
    #[test]
    fn transient_parent_changes_latch_before_restore_and_do_not_create_steps() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            for conflict in 0..5 {
                let mut record = record(version, GameMode::Survival);
                let mut current = record.initial.clone();
                let mut screen = record.initial_screen.clone();
                match conflict {
                    0 => current.health.as_mut().unwrap().value.health = 0.,
                    1 => current.game_mode = Some(GameMode::Creative),
                    2 => current.inventory.slots[45] = Some(received(item(version, "dirt", 1), 11)),
                    3 => {
                        screen.slots[0] = Some(received(item(version, "dirt", 1), 11));
                    }
                    4 => current.session.world_generation += 1,
                    _ => unreachable!(),
                }
                record.return_received(&current, Some(&screen));
                let reason = record.requires_inspection.clone();
                assert!(reason.is_some());
                let initial = record.initial.clone();
                let original_screen = record.initial_screen.clone();
                record.return_received(&initial, Some(&original_screen));
                assert_eq!(record.requires_inspection, reason);
                assert!(record.begin_return_step(initial, original_screen).is_err());
                assert!(record.return_steps.is_empty());
                assert!(!record.dispatched);
            }
        }
    }
}
