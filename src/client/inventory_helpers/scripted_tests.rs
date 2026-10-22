//! One deterministic consumer script, driven through both native packet fixtures.
use super::*;
use crate::client::{Client, inventory::InventoryClickStage as Stage};
use crate::client::{crafting::CraftingTakeId, inventory::InventoryClickId};
use std::{cell::Cell, future::Future, pin::Pin, task::Poll};

#[derive(Clone, Copy, PartialEq)]
enum Waiting {
    Click(InventoryClickId),
    Take(CraftingTakeId),
}
tokio::task_local! { static WAITING: Cell<Option<Waiting>>; }

// Signal the actual polling boundary, after admission has returned its identity.
// Task-local storage keeps parallel adapter scenarios independent.
pub(super) fn click_waiting(id: InventoryClickId) {
    let _ = WAITING.try_with(|waiting| waiting.set(Some(Waiting::Click(id))));
}
pub(super) fn take_waiting(id: CraftingTakeId) {
    let _ = WAITING.try_with(|waiting| waiting.set(Some(Waiting::Take(id))));
}

pub(crate) trait Fixture {
    fn client(&self) -> Client;
    async fn inventory(&mut self, slots: &[SlotKnowledge], cursor: &SlotKnowledge);
    async fn furnace_inventory(&mut self, slots: &[SlotKnowledge]);
    async fn slot(&mut self, slot: u16, value: &SlotKnowledge);
    async fn cursor(&mut self, value: &SlotKnowledge);
    async fn reply(&mut self, action: Option<i16>);
    async fn click_packet(&mut self, record: &InventoryClickRecord);
    async fn take_packet(&mut self, record: &CraftingTakeRecord);
    async fn replace_screen(&mut self);
    async fn forget_cursor(&mut self);
    async fn no_packets(&mut self);
}

// Keep the executor runnable while waiting for loopback I/O. Paused Tokio time
// advances only at the explicit helper polling/deadline boundaries below.
pub(crate) async fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    for _ in 0..100_000 {
        if let Poll::Ready(value) = poll(future.as_mut()).await {
            return value;
        }
        tokio::task::yield_now().await;
    }
    panic!("script did not reach its next I/O boundary");
}

async fn poll<T>(mut future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

async fn click<T: std::fmt::Debug>(
    fixture: &mut impl Fixture,
    mut helper: Pin<&mut impl Future<Output = T>>,
    previous: Option<super::super::inventory::InventoryClickId>,
    slot: u16,
    button: InventoryClickButton,
) -> InventoryClickRecord {
    let client = fixture.client();
    let record = ready(async {
        loop {
            assert!(
                poll(helper.as_mut()).await.is_pending(),
                "helper finished before click"
            );
            if let Some(record) = client.survival().inventory_click_record().await.unwrap() {
                if Some(record.id) != previous
                    && record.send.dispatched
                    && WAITING.with(|waiting| waiting.get() == Some(Waiting::Click(record.id)))
                {
                    break record;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(
        (record.source, record.source_slot, record.button),
        (InventoryClickSource::Player, slot, button)
    );
    assert_eq!(record.stage, Stage::Pending);
    assert_eq!(record.id.session(), record.initial.session);
    if let Some(previous) = previous {
        assert_eq!(record.id.attempt(), previous.attempt() + 1);
        assert_eq!(record.id.session(), previous.session());
    }
    fixture.click_packet(&record).await;
    record
}

async fn complete(
    fixture: &mut impl Fixture,
    record: &InventoryClickRecord,
    source: &SlotKnowledge,
    cursor: &SlotKnowledge,
) {
    fixture.slot(record.source_slot, source).await;
    fixture.cursor(cursor).await;
    fixture.reply(record.send.legacy_action).await;
    let latest = fixture
        .client()
        .survival()
        .inventory_click_record()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, record.id);
    assert_eq!(latest.stage, Stage::ObservedClicked, "{latest:?}");
    assert!(latest.requires_inspection.is_none());
    for (receipt, expected) in [
        (latest.source_receipt.unwrap(), source),
        (latest.cursor_receipt.unwrap(), cursor),
    ] {
        assert_eq!(&receipt.value, expected);
        assert!(
            matches!(receipt.source, crate::client::ValueSource::Received { sequence }
            if sequence > record.send.after_sequence)
        );
    }
    tokio::time::advance(Duration::from_millis(21)).await;
}

fn item(client: &Client, name: &str, count: u32) -> SlotKnowledge {
    let definition = crate::client::registry::Registry::for_version(
        crate::client::adapter::version_of(&client.adapter),
    )
    .item(&format!("minecraft:{name}"))
    .unwrap();
    SlotKnowledge::Item {
        item: ItemStack {
            id: definition.id,
            name: definition.name,
            count,
            data: crate::client::ItemData::Default,
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Case {
    Move(MoveCase),
    Compact,
    Fill,
    Craft,
    CraftTimeout,
    CraftReplacement,
    CompactTimeout,
    FillRejection,
    CraftFillReplacement,
    MissingSource,
    MissingCursor,
    Rejection,
    Replacement,
    Full,
    UnavailableCursor,
}

pub(crate) const CASES: &[Case] = &[
    Case::Move(MoveCase::FuelTagsUnavailable),
    Case::Move(MoveCase::Empty),
    Case::Move(MoveCase::Merge),
    Case::Move(MoveCase::Partial),
    Case::Move(MoveCase::Armor),
    Case::Move(MoveCase::Data),
    Case::Move(MoveCase::Offhand),
    Case::Move(MoveCase::SecondRejected),
    Case::Move(MoveCase::ThirdRejected),
    Case::Move(MoveCase::Timeout),
    Case::Move(MoveCase::ScreenChanged),
    Case::Move(MoveCase::Cancelled),
    Case::Move(MoveCase::HeldCursor),
    Case::Move(MoveCase::Binding),
    Case::Move(MoveCase::WrongEquipment),
    Case::Move(MoveCase::SameSlot),
    Case::Compact,
    Case::Fill,
    Case::Craft,
    Case::CraftTimeout,
    Case::CraftReplacement,
    Case::CompactTimeout,
    Case::FillRejection,
    Case::CraftFillReplacement,
    Case::MissingSource,
    Case::MissingCursor,
    Case::Rejection,
    Case::Replacement,
    Case::Full,
    Case::UnavailableCursor,
];

pub(crate) async fn run(fixture: &mut impl Fixture, case: Case) {
    WAITING
        .scope(Cell::new(None), run_case(fixture, case))
        .await;
}

async fn run_case(fixture: &mut impl Fixture, case: Case) {
    eprintln!("inventory helper script: {case:?}");
    if let Case::Move(case) = case {
        return Box::pin(run_move_case(fixture, case)).await;
    }
    let client = fixture.client();
    let ops = client.survival();
    let empty = SlotKnowledge::Empty;
    let stone = |count| item(&client, "stone", count);
    let plank = |count| item(&client, "oak_planks", count);
    let dirt = item(&client, "dirt", 1);
    let button = item(&client, "oak_button", 1);
    let mut slots = vec![empty.clone(); 46];
    match case {
        Case::Move(_) => unreachable!(),
        Case::Compact => {
            slots[9] = stone(60);
            slots[44] = stone(6);
            fixture.inventory(&slots, &stone(2)).await;
            let mut helper = std::pin::pin!(ops.compact_inventory());
            let mut expected = Vec::new();
            for (slot, source, cursor) in [
                (9, stone(62), empty.clone()),
                (44, empty.clone(), stone(6)),
                (9, stone(64), stone(4)),
                (10, stone(4), empty.clone()),
            ] {
                let record = click(
                    fixture,
                    helper.as_mut(),
                    expected.last().copied(),
                    slot,
                    InventoryClickButton::Left,
                )
                .await;
                expected.push(record.id);
                complete(fixture, &record, &source, &cursor).await;
            }
            let records = ready(helper).await.unwrap();
            assert_eq!(records.iter().map(|r| r.id).collect::<Vec<_>>(), expected);
            assert!(records.iter().all(|r| r.stage == Stage::ObservedClicked));
            let inventory = client.received_inventory().await.unwrap();
            assert_eq!(inventory.slot(9).unwrap().unwrap().value(), &stone(64));
            assert_eq!(inventory.slot(10).unwrap().unwrap().value(), &stone(4));
            assert_eq!(inventory.slot(44).unwrap().unwrap().value(), &empty);
            assert_eq!(inventory.cursor().unwrap().value(), &empty);
        }
        Case::Fill | Case::Craft | Case::CraftTimeout | Case::CraftReplacement => {
            slots[1] = dirt.clone();
            slots[9] = plank(3);
            fixture.inventory(&slots, &empty).await;
            let ingredients = [Some("oak_planks"), None, None, None];
            let mut helper = std::pin::pin!(async {
                if matches!(case, Case::Fill) {
                    ops.fill_crafting_grid(&ingredients)
                        .await
                        .map(|fill| (fill, None))
                } else {
                    ops.craft_once(&ingredients)
                        .await
                        .map(|r| (r.fill, Some(r.take)))
                }
            });
            let mut expected = Vec::new();
            for (slot, key, source, cursor) in [
                (1, InventoryClickButton::Left, empty.clone(), dirt.clone()),
                (10, InventoryClickButton::Left, dirt.clone(), empty.clone()),
                (9, InventoryClickButton::Left, empty.clone(), plank(3)),
                (1, InventoryClickButton::Right, plank(1), plank(2)),
                (9, InventoryClickButton::Left, plank(2), empty.clone()),
            ] {
                let record = click(
                    fixture,
                    helper.as_mut(),
                    expected.last().copied(),
                    slot,
                    key,
                )
                .await;
                expected.push(record.id);
                complete(fixture, &record, &source, &cursor).await;
            }
            fixture.slot(0, &button).await;
            let mut sent = None;
            if !matches!(case, Case::Fill) {
                let take = ready(async {
                    loop {
                        assert!(poll(helper.as_mut()).await.is_pending());
                        if let Some(take) = ops.crafting_take_record().await.unwrap() {
                            if take.send.dispatched
                                && WAITING
                                    .with(|waiting| waiting.get() == Some(Waiting::Take(take.id)))
                            {
                                break take;
                            }
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await;
                fixture.take_packet(&take).await;
                assert_eq!(take.stage, crate::client::CraftingTakeStage::Pending);
                match case {
                    Case::Craft => {
                        slots[0] = empty.clone();
                        slots[1] = empty.clone();
                        slots[9] = plank(2);
                        slots[10] = dirt.clone();
                        slots[44] = button.clone();
                        fixture.inventory(&slots, &empty).await;
                        fixture.reply(take.send.legacy_action).await;
                    }
                    Case::CraftReplacement => fixture.replace_screen().await,
                    Case::CraftTimeout => {}
                    _ => unreachable!(),
                }
                sent = Some(take);
                tokio::time::advance(Duration::from_secs(3)).await;
            }
            let (fill, take) = ready(helper).await.unwrap();
            assert_eq!(
                fill.result,
                match &button {
                    SlotKnowledge::Item { item } => Some(item.clone()),
                    _ => unreachable!(),
                }
            );
            assert_eq!(
                fill.clicks.iter().map(|r| r.id).collect::<Vec<_>>(),
                expected
            );
            assert!(
                fill.clicks
                    .iter()
                    .all(|r| r.stage == Stage::ObservedClicked)
            );
            if let Some(sent) = sent {
                let take = take.unwrap();
                assert_eq!(take.id, sent.id);
                assert_eq!(take.id.session(), sent.initial.session);
                let latest = ops.crafting_take_record().await.unwrap().unwrap();
                assert_eq!(latest.id, sent.id);
                assert_eq!(take.stage, latest.stage);
                assert_eq!(take.requires_inspection, latest.requires_inspection);
                match case {
                    Case::Craft => assert_eq!(
                        take.stage,
                        crate::client::CraftingTakeStage::ObservedTransferred
                    ),
                    Case::CraftTimeout => {
                        // Existing craft_once contract: timeout returns the pending identity.
                        assert_eq!(take.stage, crate::client::CraftingTakeStage::Pending);
                        assert!(take.after.is_none());
                    }
                    Case::CraftReplacement => {
                        assert_eq!(
                            take.stage,
                            crate::client::CraftingTakeStage::RequiresInspection
                        );
                        assert!(take.requires_inspection.is_some());
                    }
                    _ => unreachable!(),
                }
            }
        }
        Case::Full => {
            slots[9..45].fill(stone(64));
            fixture.inventory(&slots, &stone(1)).await;
            let error = ops.compact_inventory().await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ResourceLimit);
            assert_eq!(error.to_string(), "no storage room for the cursor");
            assert!(ops.inventory_click_record().await.unwrap().is_none());
        }
        Case::UnavailableCursor => {
            slots[9] = plank(3);
            fixture.inventory(&slots, &empty).await;
            fixture.forget_cursor().await;
            let error = ops
                .fill_crafting_grid(&[Some("oak_planks"), None, None, None])
                .await
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::State);
            assert!(ops.inventory_click_record().await.unwrap().is_none());
        }
        _ => {
            slots[9] = stone(3);
            let source_slot = match case {
                Case::CompactTimeout => {
                    slots[44] = stone(6);
                    44
                }
                Case::FillRejection | Case::CraftFillReplacement => {
                    slots[1] = dirt.clone();
                    1
                }
                _ => 9,
            };
            fixture.inventory(&slots, &empty).await;
            let mut records = Vec::new();
            let direct = !matches!(
                case,
                Case::CompactTimeout | Case::FillRejection | Case::CraftFillReplacement
            );
            let mut helper = Box::pin(async {
                match case {
                    Case::CompactTimeout => ops.compact_inventory().await.map(|_| ()),
                    Case::FillRejection => ops
                        .fill_crafting_grid(&[Some("oak_planks"), None, None, None])
                        .await
                        .map(|_| ()),
                    Case::CraftFillReplacement => ops
                        .craft_once(&[Some("oak_planks"), None, None, None])
                        .await
                        .map(|_| ()),
                    _ => {
                        ops.click_and_wait(
                            InventoryClickSource::Player,
                            9,
                            InventoryClickButton::Left,
                            &mut records,
                        )
                        .await
                    }
                }
            });
            let sent = click(
                fixture,
                helper.as_mut(),
                None,
                source_slot,
                InventoryClickButton::Left,
            )
            .await;
            match case {
                Case::MissingSource => {
                    fixture.cursor(&stone(3)).await;
                    fixture.reply(sent.send.legacy_action).await;
                }
                Case::MissingCursor => {
                    fixture.slot(9, &empty).await;
                    fixture.reply(sent.send.legacy_action).await;
                }
                Case::CompactTimeout => {}
                Case::Rejection | Case::FillRejection => fixture.slot(source_slot, &stone(1)).await,
                Case::Replacement | Case::CraftFillReplacement => fixture.replace_screen().await,
                _ => unreachable!(),
            }
            let missing = matches!(
                case,
                Case::MissingSource | Case::MissingCursor | Case::CompactTimeout
            );
            if missing {
                let latest = ops.inventory_click_record().await.unwrap().unwrap();
                assert_eq!(latest.id, sent.id);
                assert_eq!(latest.stage, Stage::Pending);
                assert_eq!(
                    latest.source_receipt.is_none(),
                    !matches!(case, Case::MissingCursor)
                );
                assert_eq!(
                    latest.cursor_receipt.is_none(),
                    !matches!(case, Case::MissingSource)
                );
                assert!(poll(helper.as_mut()).await.is_pending());
                tokio::time::advance(CLICK_TIMEOUT + Duration::from_millis(21)).await;
            } else {
                tokio::time::advance(Duration::from_millis(21)).await;
            }
            let error = ready(helper.as_mut()).await.unwrap_err();
            drop(helper);
            assert_eq!(
                error.kind(),
                if missing {
                    ErrorKind::Timeout
                } else {
                    ErrorKind::Rejected
                }
            );
            if direct {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].id, sent.id);
                assert_eq!(
                    error.to_string(),
                    if missing {
                        format!("click on slot {source_slot} not received")
                    } else {
                        format!(
                            "click on slot {source_slot}: {}",
                            records[0].requires_inspection.as_ref().unwrap()
                        )
                    }
                );
                assert_eq!(
                    records[0].stage,
                    if missing {
                        Stage::Pending
                    } else {
                        Stage::RequiresInspection
                    }
                );
            }
            assert!(ops.crafting_take_record().await.unwrap().is_none());
            let latest = ops.inventory_click_record().await.unwrap().unwrap();
            assert_eq!(latest.id, sent.id);
            if !missing {
                let reason = latest.requires_inspection.clone();
                assert!(reason.is_some());
                if matches!(case, Case::Rejection) {
                    fixture.slot(9, &empty).await;
                    fixture.cursor(&stone(3)).await;
                    fixture.reply(sent.send.legacy_action).await;
                }
                let latest = ops.inventory_click_record().await.unwrap().unwrap();
                assert_eq!(latest.id, sent.id);
                assert_eq!(latest.stage, Stage::RequiresInspection);
                assert_eq!(latest.requires_inspection, reason);
            }
        }
    }
    fixture.no_packets().await;
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MoveCase {
    FuelTagsUnavailable,
    Empty,
    Merge,
    Partial,
    Armor,
    Data,
    Offhand,
    SecondRejected,
    ThirdRejected,
    Timeout,
    ScreenChanged,
    Cancelled,
    HeldCursor,
    Binding,
    WrongEquipment,
    SameSlot,
}

async fn run_move_case(fixture: &mut impl Fixture, case: MoveCase) {
    let client = fixture.client();
    let ops = client.survival();
    let version = crate::client::adapter::version_of(&client.adapter);
    let empty = SlotKnowledge::Empty;
    if matches!(case, MoveCase::FuelTagsUnavailable) {
        let mut slots = vec![empty.clone(); 39];
        slots[3] = item(&client, "coal", 7);
        fixture.furnace_inventory(&slots).await;
        let screen = client.screen_state().await.unwrap().screen.unwrap().id;
        let mut parent = InventoryMoveRecord::new(InventoryClickSource::Container { screen }, 3, 1);
        let mut helper = Box::pin(ops.move_inventory(&mut parent));
        let result = ready(async {
            loop {
                if let Poll::Ready(result) = poll(helper.as_mut()).await {
                    break result;
                }
                assert!(
                    ops.inventory_click_record().await.unwrap().is_none(),
                    "fuel tag preflight must reject before picking up the source"
                );
                tokio::task::yield_now().await;
            }
        })
        .await;
        drop(helper);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("fuel tags unavailable")
        );
        assert_eq!(parent.stage(), InventoryMoveStage::Stopped);
        assert_eq!(parent.completed_steps(), 0);
        assert!(parent.clicks().is_empty());
        let state = client.screen_state().await.unwrap();
        assert_eq!(state.cursor.unwrap().value, empty);
        assert_eq!(
            state.screen.unwrap().slots[3].as_ref().unwrap().value,
            slots[3]
        );
        fixture.no_packets().await;
        return;
    }
    let mut slots = vec![empty.clone(); 46];
    slots[9] = item(&client, "stone", 7);
    let destination = match case {
        MoveCase::Armor | MoveCase::Data | MoveCase::Binding | MoveCase::WrongEquipment => 5,
        MoveCase::Offhand => 45,
        MoveCase::SameSlot => 9,
        _ => 10,
    };
    match case {
        MoveCase::Merge => slots[10] = item(&client, "stone", 5),
        MoveCase::Partial => slots[10] = item(&client, "stone", 62),
        MoveCase::Armor | MoveCase::Data | MoveCase::Binding => {
            slots[9] = if matches!(case, MoveCase::Data) {
                super::super::inventory::armor_tests::fixture(version, false)
            } else {
                item(&client, "iron_helmet", 1)
            };
            slots[5] = if matches!(case, MoveCase::Binding) {
                super::super::inventory::armor_tests::fixture(version, true)
            } else {
                item(&client, "golden_helmet", 1)
            };
        }
        MoveCase::Offhand => slots[45] = item(&client, "shield", 1),
        MoveCase::ThirdRejected => slots[10] = item(&client, "dirt", 1),
        _ => {}
    }
    let original = slots.clone();
    fixture
        .inventory(
            &slots,
            &if matches!(case, MoveCase::HeldCursor) {
                item(&client, "dirt", 1)
            } else {
                empty.clone()
            },
        )
        .await;
    let mut parent = InventoryMoveRecord::new(InventoryClickSource::Player, 9, destination);
    let mut helper = Box::pin(ops.move_inventory(&mut parent));
    if matches!(
        case,
        MoveCase::HeldCursor | MoveCase::Binding | MoveCase::WrongEquipment | MoveCase::SameSlot
    ) {
        assert!(ready(helper.as_mut()).await.is_err());
        drop(helper);
        assert_eq!(parent.stage(), InventoryMoveStage::Stopped);
        assert!(parent.clicks().is_empty());
        fixture.no_packets().await;
        return;
    }
    let first = click(
        fixture,
        helper.as_mut(),
        None,
        9,
        InventoryClickButton::Left,
    )
    .await;
    // A single fresh slot receipt never permits the next click.
    fixture.slot(9, &first.prediction.source.value).await;
    assert!(poll(helper.as_mut()).await.is_pending());
    fixture.no_packets().await;
    fixture.cursor(&first.prediction.cursor.value).await;
    fixture.reply(first.send.legacy_action).await;
    tokio::time::advance(Duration::from_millis(21)).await;
    if matches!(case, MoveCase::Cancelled) {
        drop(helper);
        ops.refresh_inventory_move(&mut parent).await.unwrap();
        assert_eq!(parent.stage(), InventoryMoveStage::Stopped);
        assert_eq!(parent.completed_steps(), 1);
        assert_eq!(parent.clicks()[0].id, first.id);
        assert!(parent.stop_reason().unwrap().contains("cancelled"));
        assert_eq!(
            client
                .player_state()
                .await
                .unwrap()
                .inventory
                .cursor
                .unwrap()
                .value,
            original[9]
        );
        assert!(ops.move_inventory(&mut parent).await.is_err());
        fixture.no_packets().await;
        return;
    }
    let second = click(
        fixture,
        helper.as_mut(),
        Some(first.id),
        destination,
        InventoryClickButton::Left,
    )
    .await;
    match case {
        MoveCase::SecondRejected => fixture.cursor(&item(&client, "dirt", 2)).await,
        MoveCase::ScreenChanged => fixture.replace_screen().await,
        MoveCase::Timeout => tokio::time::advance(CLICK_TIMEOUT + Duration::from_millis(21)).await,
        _ => {
            complete(
                fixture,
                &second,
                &second.prediction.source.value,
                &second.prediction.cursor.value,
            )
            .await
        }
    }
    let stopped = matches!(
        case,
        MoveCase::SecondRejected | MoveCase::ScreenChanged | MoveCase::Timeout
    );
    let mut ids = vec![first.id, second.id];
    if !stopped && second.prediction.cursor.value != empty {
        let third = click(
            fixture,
            helper.as_mut(),
            Some(second.id),
            9,
            InventoryClickButton::Left,
        )
        .await;
        ids.push(third.id);
        if matches!(case, MoveCase::ThirdRejected) {
            fixture.cursor(&item(&client, "dirt", 2)).await;
            tokio::time::advance(Duration::from_millis(21)).await;
        } else {
            complete(
                fixture,
                &third,
                &third.prediction.source.value,
                &third.prediction.cursor.value,
            )
            .await;
        }
    }
    tokio::time::advance(Duration::from_millis(21)).await;
    let result = ready(helper.as_mut()).await;
    drop(helper);
    let stopped = stopped || matches!(case, MoveCase::ThirdRejected);
    assert_eq!(result.is_err(), stopped, "{case:?} {result:?}");
    assert_eq!(
        parent.stage(),
        if stopped {
            InventoryMoveStage::Stopped
        } else {
            InventoryMoveStage::ObservedMoved
        }
    );
    assert_eq!(
        parent.clicks().iter().map(|r| r.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(parent.completed_steps(), ids.len() - usize::from(stopped));
    assert_eq!(parent.clicks()[0].stage, Stage::ObservedClicked);
    if matches!(case, MoveCase::Timeout) {
        assert_eq!(result.unwrap_err().kind(), ErrorKind::Timeout);
        assert_eq!(parent.clicks()[1].stage, Stage::Pending);
        assert_eq!(
            client
                .player_state()
                .await
                .unwrap()
                .inventory
                .cursor
                .unwrap()
                .value,
            original[9]
        );
        // A late receipt can update only the original child; no third click is sent.
        complete(
            fixture,
            &second,
            &second.prediction.source.value,
            &second.prediction.cursor.value,
        )
        .await;
        ops.refresh_inventory_move(&mut parent).await.unwrap();
        assert_eq!(parent.completed_steps(), 2);
        assert_eq!(parent.stage(), InventoryMoveStage::Stopped);
    }
    if !stopped {
        let state = client.player_state().await.unwrap();
        assert_eq!(state.inventory.cursor.unwrap().value, empty);
        let dest = &state.inventory.slots[usize::from(destination)]
            .as_ref()
            .unwrap()
            .value;
        if matches!(case, MoveCase::Armor | MoveCase::Data | MoveCase::Offhand) {
            assert_eq!(dest, &original[9]);
            assert_eq!(
                state.inventory.slots[9].as_ref().unwrap().value,
                original[usize::from(destination)]
            );
        }
    }
    fixture.no_packets().await;
}
