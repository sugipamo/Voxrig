async fn acknowledge_storage_clicks(bot: &Bot, work: &tokio::task::JoinHandle<Result<()>>, accept: bool) {
    timeout(Duration::from_secs(3), async {
        while !work.is_finished() {
            let pending = bot.inventory.read().await.pending_clicks.keys().next().copied();
            if let Some((window, action)) = pending {
                let mut packet = vec![window as u8];
                packet.extend(action.to_be_bytes());
                packet.push(u8::from(accept));
                bot.apply_packet(0x12, packet).await.unwrap();
            } else {
                tokio::task::yield_now().await;
            }
        }
    }).await.unwrap();
}

fn storage_test_stack(name: &str, count: i8) -> ItemStack {
    ItemStack { item_id:(0..2000).find(|id|crate::registry::item_name(*id)==Some(name)).unwrap(),count,nbt:None }
}

#[tokio::test]
async fn crafting_output_merges_and_never_uses_offhand_or_picks_without_capacity() {
    for merge in [true,false] {
        let (bot, server, release)=ready_test_bot(ConnectionOptions::default()).await;
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        let result=storage_test_stack("spruce_planks",4);
        let mut slots=vec![None;46];
        slots[0]=Some(result.clone());
        for slot in &mut slots[9..45] { *slot=Some(storage_test_stack("cobblestone",64)); }
        if merge {slots[9]=Some(storage_test_stack("spruce_planks",60));}
        bot.inventory.write().await.windows.insert(0,slots);
        let worker=bot.clone_internal();
        let work=tokio::spawn(async move {worker.take_crafting_result(0,4).await});
        acknowledge_storage_clicks(&bot,&work,true).await;
        let outcome=work.await.unwrap();
        let inventory=bot.inventory().await;
        assert!(inventory.cursor.is_none());
        assert!(inventory.windows[&0][45].is_none());
        if merge {
            outcome.unwrap();
            assert_eq!(inventory.windows[&0][9],Some(storage_test_stack("spruce_planks",64)));
        } else {
            assert!(outcome.unwrap_err().to_string().contains("result not taken"));
            assert_eq!(inventory.windows[&0][0],Some(result));
        }
        release.send(()).unwrap(); drop(bot); server.await.unwrap();
    }
}

#[test]
fn storage_capacity_respects_nbt_maximum_and_window_ranges() {
    let item=storage_test_stack("spruce_planks",4);
    let mut other=item.clone(); other.nbt=Some(vec![10,0,0,0]);
    assert_eq!(merge_room(Some(&other),&item),0);
    let tool=storage_test_stack("iron_pickaxe",1);
    assert_eq!(merge_room(Some(&tool),&tool),0);
    assert_eq!(storage_range(0,46).unwrap(),9..45);
    assert_eq!(storage_range(1,46).unwrap(),10..46);
    assert_eq!(storage_range(2,63).unwrap(),27..63);
    assert!(storage_range(0,44).is_err());
}

#[tokio::test]
async fn compaction_preserves_items_recovers_cursor_and_offhand_and_stops_on_rejection() {
    for accept in [true,false] {
        let (bot, server, release)=ready_test_bot(ConnectionOptions::default()).await;
        bot.teleport_barrier_ticks.store(u8::MAX, Ordering::Release);
        let mut slots=vec![None;46];
        for slot in &mut slots[9..45] { *slot=Some(storage_test_stack("spruce_planks",4)); }
        slots[45]=Some(storage_test_stack("spruce_planks",4));
        bot.inventory.write().await.windows.insert(0,slots);
        bot.inventory.write().await.cursor=Some(storage_test_stack("spruce_planks",4));
        let worker=bot.clone_internal();
        let work=tokio::spawn(async move {worker.compact_player_inventory(0).await});
        acknowledge_storage_clicks(&bot,&work,accept).await;
        let outcome=work.await.unwrap();
        let inventory=bot.inventory().await;
        let total:i32=inventory.windows[&0][9..].iter().flatten().map(|s|i32::from(s.count)).sum::<i32>()
            +inventory.cursor.as_ref().map_or(0,|s|i32::from(s.count));
        assert_eq!(total,152);
        if accept {
            outcome.unwrap();
            assert!(inventory.cursor.is_none());
            assert!(inventory.windows[&0][45].is_none());
            assert_eq!(inventory.windows[&0][9..45].iter().flatten().count(),3);
        } else { assert!(outcome.is_err()); }
        release.send(()).unwrap(); drop(bot); server.await.unwrap();
    }
}
