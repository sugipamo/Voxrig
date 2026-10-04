fn swap_comparison() -> crate::versions::java_1_16_1::ItemStack { crate::versions::java_1_16_1::ItemStack { item_id:1,count:3,nbt:None } }
#[tokio::test]
async fn bounded_motion_excludes_normal_dispatch_but_preserves_protocol_and_cleanup() {
    let (actor, mut peer) = actor_fixture().await;
    actor.mark_ready().await;
    let context = OperationContext { generation: actor.generation(), source_observation_sequence: 0 };
    let revision = actor.motion_admission_revision().await.unwrap();
    actor.begin_bounded_motion(1, revision).await.unwrap();
    assert_eq!(actor.dispatch_operation(context, OperationClass::Normal, 0x13, &[0;33]).await, Err(OperationAdmissionError::BoundedMotionInProgress));
    assert_eq!(actor.replace_control(context, OperationClass::Normal, crate::ControlState { forward: true, ..Default::default() }).await, Err(OperationAdmissionError::BoundedMotionInProgress));
    assert_eq!(actor.admit(context, OperationClass::Normal).await, Err(OperationAdmissionError::BoundedMotionInProgress));
    no_packet(&mut peer).await;
    actor.dispatch_protocol(0x10, &[9]).await.unwrap();
    assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x10,vec![9]));
    assert!(actor.bounded_position(2,vec![0;33]).await.is_err());
    actor.bounded_position(1,vec![0;33]).await.unwrap();
    assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x13,vec![0;33]));
    actor.finish_bounded_motion(1).await.unwrap();
    actor.dispatch_operation(context,OperationClass::Normal,0x13,&[0;33]).await.unwrap();
    assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x13,vec![0;33]));
    let revision = actor.motion_admission_revision().await.unwrap();
    actor.begin_bounded_motion(2,revision).await.unwrap();
    actor.begin_disconnect().await.unwrap();
    assert!(actor.bounded_position(2,vec![0;33]).await.is_err());
    actor.dispatch(context,OperationClass::Cleanup,0x1b,&[1]).await.unwrap();
    assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x1b,vec![1]));
}

#[tokio::test]
async fn inventory_swap_actor_owns_click_identity_and_exact_release_without_replay(){
    let (actor,mut peer)=actor_fixture().await;actor.mark_ready().await;let revision=actor.motion_admission_revision().await.unwrap();
    let action=actor.begin_inventory_swap(1,revision).await.unwrap();assert_eq!(action,1);
    let context=OperationContext{generation:actor.generation(),source_observation_sequence:0};
    assert_eq!(actor.replace_control(context,OperationClass::Normal,crate::ControlState::default()).await,Err(OperationAdmissionError::BoundedInventorySwapInProgress));
    assert!(actor.finish_bounded_motion(1).await.is_err());assert!(actor.finish_bounded_placement(1).await.is_err());assert!(actor.finish_inventory_swap(1).await.is_err());assert!(actor.bounded_inventory_swap(2,9,0,swap_comparison()).await.is_err());no_packet(&mut peer).await;
    actor.bounded_inventory_swap(1,9,0,swap_comparison()).await.unwrap();assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x09,vec![0,0,9,0,0,1,2,1,1,3,0]));
    assert!(actor.bounded_inventory_swap(1,9,0,swap_comparison()).await.is_err());actor.finish_inventory_swap(1).await.unwrap();
    let revision=actor.motion_admission_revision().await.unwrap();assert_eq!(actor.begin_inventory_swap(2,revision).await.unwrap(),2);
    actor.bounded_inventory_swap(2,35,8,swap_comparison()).await.unwrap();assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x09,vec![0,0,35,8,0,2,2,1,1,3,0]));actor.finish_inventory_swap(2).await.unwrap();
}
#[tokio::test]
async fn storage_swap_actor_uses_reserved_window_and_independent_native_action_pool(){
    let (actor,mut peer)=actor_fixture().await;actor.mark_ready().await;
    for (run,window,slot,action) in [(1,3,0,1),(2,0,9,1),(3,3,26,2)] {
        let revision=actor.motion_admission_revision().await.unwrap();
        assert_eq!(actor.begin_window_swap(run,revision,window).await.unwrap(),action);
        actor.bounded_inventory_swap(run,slot,0,swap_comparison()).await.unwrap();
        let (id,packet)=read_packet(&mut peer,None).await.unwrap();assert_eq!(id,0x09);
        assert_eq!(packet[0],window as u8);assert_eq!(i16::from_be_bytes([packet[1],packet[2]]),slot as i16);
        assert_eq!(i16::from_be_bytes([packet[4],packet[5]]),action);
        assert!(actor.bounded_inventory_swap(run,slot,0,swap_comparison()).await.is_err());
        actor.finish_inventory_swap(run).await.unwrap();
    }
}
#[tokio::test]
async fn inventory_swap_actor_cancelled_waiter_does_not_cancel_owned_write(){
    let (actor,mut peer,writer)=actor_fixture_with_writer().await;actor.mark_ready().await;actor.begin_inventory_swap(1,actor.motion_admission_revision().await.unwrap()).await.unwrap();
    let guard=writer.lock().await;let mut send=Box::pin(actor.bounded_inventory_swap(1,9,0,swap_comparison()));std::future::poll_fn(|cx|{assert!(send.as_mut().poll(cx).is_pending());std::task::Poll::Ready(())}).await;drop(send);drop(guard);
    assert_eq!(timeout(Duration::from_secs(1),read_packet(&mut peer,None)).await.unwrap().unwrap().0,0x09);assert!(actor.bounded_inventory_swap(1,9,0,swap_comparison()).await.is_err());actor.finish_inventory_swap(1).await.unwrap();
}
#[tokio::test]
async fn inventory_swap_actor_write_failure_retains_terminal_uncertainty(){
    let (actor,_peer)=actor_fixture().await;actor.mark_ready().await;actor.begin_inventory_swap(1,actor.motion_admission_revision().await.unwrap()).await.unwrap();actor.shutdown_writer().await.unwrap();assert!(actor.bounded_inventory_swap(1,9,0,swap_comparison()).await.is_err());assert_eq!(actor.wait_for_terminal().await,ConnectionState::ConnectionStateUnknown);assert!(actor.finish_inventory_swap(1).await.is_err());
}
#[tokio::test]
async fn bounded_motion_rejects_intervening_native_gameplay_before_acquisition() {
    let (actor,mut peer)=actor_fixture().await;
    actor.mark_ready().await;
    let revision=actor.motion_admission_revision().await.unwrap();
    let context=OperationContext { generation:actor.generation(),source_observation_sequence:0 };
    actor.dispatch(context,OperationClass::Normal,0x13,&[0;33]).await.unwrap();
    read_packet(&mut peer,None).await.unwrap();
    assert_eq!(actor.begin_bounded_motion(1,revision).await,Err(OperationAdmissionError::InvalidOperation));
    assert!(actor.bounded_position(1,vec![0;33]).await.is_err());
    no_packet(&mut peer).await;
}
#[tokio::test]
async fn bounded_motion_packet_waiter_cancellation_does_not_cancel_actor_write() {
    let (actor,mut peer,writer)=actor_fixture_with_writer().await;
    actor.mark_ready().await;
    actor.begin_bounded_motion(1,actor.motion_admission_revision().await.unwrap()).await.unwrap();
    let writer_guard=writer.lock().await;
    let mut dispatch=Box::pin(actor.bounded_position(1,vec![0;33]));
    std::future::poll_fn(|cx| { assert!(dispatch.as_mut().poll(cx).is_pending()); std::task::Poll::Ready(()) }).await;
    drop(dispatch);
    drop(writer_guard);
    assert_eq!(timeout(Duration::from_secs(1),read_packet(&mut peer,None)).await.unwrap().unwrap(),(0x13,vec![0;33]));
    actor.finish_bounded_motion(1).await.unwrap();
}
#[tokio::test]
async fn bounded_motion_failed_write_terminates_actor_without_replay() {
    let (actor,_peer)=actor_fixture().await;
    actor.mark_ready().await;
    actor.begin_bounded_motion(1,actor.motion_admission_revision().await.unwrap()).await.unwrap();
    actor.shutdown_writer().await.unwrap();
    assert!(actor.bounded_position(1,vec![0;33]).await.is_err());
    assert_eq!(actor.wait_for_terminal().await,ConnectionState::ConnectionStateUnknown);
    assert!(actor.bounded_position(1,vec![0;33]).await.is_err());
}

#[tokio::test]
async fn bounded_mining_owns_exact_target_and_native_stages_without_motion_aliases() {
    use crate::client::survival::MiningAction::{Start,Finish,Abort};
    let (actor,mut peer)=actor_fixture().await;
    actor.mark_ready().await;
    let target=crate::BlockPos {x:8,y:66,z:11};
    actor.begin_bounded_mining(1,actor.motion_admission_revision().await.unwrap(),target,2).await.unwrap();
    let context=OperationContext {generation:actor.generation(),source_observation_sequence:0};
    assert_eq!(actor.replace_control(context,OperationClass::Normal,crate::ControlState::default()).await,Err(OperationAdmissionError::BoundedMiningInProgress));
    assert!(actor.bounded_position(1,vec![0;33]).await.is_err());
    assert!(actor.finish_bounded_motion(1).await.is_err());
    assert!(actor.bounded_mining(1,Finish).await.is_err());
    assert!(actor.bounded_mining(2,Start).await.is_err());
    no_packet(&mut peer).await;
    for action in [Start,Finish,Abort] {
        actor.bounded_mining(1,action).await.unwrap();
        let (id,bytes)=read_packet(&mut peer,None).await.unwrap();
        assert_eq!(id,0x1b);
        let mut expected=vec![action as u8];
        expected.extend(target.packed().to_be_bytes()); expected.push(2);
        assert_eq!(bytes,expected); assert_eq!(bytes.len(),10);
        assert!(actor.bounded_mining(1,action).await.is_err());
        no_packet(&mut peer).await;
    }
    assert_eq!(actor.dispatch_operation(context,OperationClass::Normal,0x24,&[0;2]).await,Err(OperationAdmissionError::BoundedMiningInProgress));
    assert!(actor.finish_bounded_motion(1).await.is_err());
    actor.dispatch_protocol(0x10,&[9]).await.unwrap();
    assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x10,vec![9]));
    actor.begin_disconnect().await.unwrap();
    assert!(actor.bounded_mining(1,Abort).await.is_err());
    actor.dispatch(context,OperationClass::Cleanup,0x1b,&[1]).await.unwrap();
    assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x1b,vec![1]));
}
#[tokio::test]
async fn bounded_mining_cancelled_waiter_cannot_repeat_start_or_release_owner() {
    use crate::client::survival::MiningAction::Start;
    let (actor,mut peer,writer)=actor_fixture_with_writer().await;
    actor.mark_ready().await;
    actor.begin_bounded_mining(1,actor.motion_admission_revision().await.unwrap(),crate::BlockPos{x:8,y:66,z:11},2).await.unwrap();
    let guard=writer.lock().await;
    let mut send=Box::pin(actor.bounded_mining(1,Start));
    std::future::poll_fn(|cx|{assert!(send.as_mut().poll(cx).is_pending());std::task::Poll::Ready(())}).await;
    drop(send);drop(guard);
    let (id,bytes)=timeout(Duration::from_secs(1),read_packet(&mut peer,None)).await.unwrap().unwrap();
    assert_eq!(id,0x1b);assert_eq!(bytes.len(),10);assert_eq!(bytes[0],0);
    assert!(actor.bounded_mining(1,Start).await.is_err());
    no_packet(&mut peer).await;
    assert_eq!(actor.motion_admission_revision().await,Err(OperationAdmissionError::BoundedMiningInProgress));
}
#[tokio::test]
async fn bounded_mining_failed_write_ends_actor_and_cannot_replay() {
    use crate::client::survival::MiningAction::Start;
    let (actor,_peer)=actor_fixture().await;
    actor.mark_ready().await;
    actor.begin_bounded_mining(1,actor.motion_admission_revision().await.unwrap(),crate::BlockPos{x:8,y:66,z:11},2).await.unwrap();
    actor.shutdown_writer().await.unwrap();
    assert!(actor.bounded_mining(1,Start).await.is_err());
    assert_eq!(actor.wait_for_terminal().await,ConnectionState::ConnectionStateUnknown);
    assert!(actor.bounded_mining(1,Start).await.is_err());
}

#[tokio::test]
async fn bounded_placement_excludes_other_owners_and_only_exact_completion_releases() {
    let (actor,mut peer)=actor_fixture().await;actor.mark_ready().await;
    let revision=actor.motion_admission_revision().await.unwrap();
    actor.bounded_placement(1,revision,crate::BlockPos{x:10,y:66,z:8},4,[0.0,0.5,0.5]).await.unwrap();
    let (id,p)=read_packet(&mut peer,None).await.unwrap();assert_eq!((id,p.len()),(0x2d,23));
    let context=OperationContext{generation:actor.generation(),source_observation_sequence:0};
    assert_eq!(actor.admit(context,OperationClass::Normal).await,Err(OperationAdmissionError::BoundedPlacementInProgress));
    assert!(actor.bounded_mining(1,crate::client::survival::MiningAction::Finish).await.is_err());
    assert!(actor.finish_bounded_motion(1).await.is_err());assert!(actor.finish_bounded_placement(2).await.is_err());
    assert!(actor.bounded_placement(1,revision,crate::BlockPos{x:10,y:66,z:8},4,[0.0,0.5,0.5]).await.is_err());
    actor.dispatch_protocol(0x10,&[9]).await.unwrap();assert_eq!(read_packet(&mut peer,None).await.unwrap(),(0x10,vec![9]));
    actor.finish_bounded_placement(1).await.unwrap();actor.admit(context,OperationClass::Normal).await.unwrap();no_packet(&mut peer).await;
}
#[tokio::test]
async fn bounded_placement_cancelled_waiter_cannot_replay_or_clear_owner() {
    let (actor,mut peer,writer)=actor_fixture_with_writer().await;actor.mark_ready().await;
    let revision=actor.motion_admission_revision().await.unwrap();let held=writer.lock().await;
    let mut send=Box::pin(actor.bounded_placement(1,revision,crate::BlockPos{x:10,y:66,z:8},4,[0.0,0.5,0.5]));
    std::future::poll_fn(|cx| {assert!(send.as_mut().poll(cx).is_pending());std::task::Poll::Ready(())}).await;drop(send);drop(held);
    assert_eq!(timeout(Duration::from_secs(1),read_packet(&mut peer,None)).await.unwrap().unwrap().0,0x2d);
    assert_eq!(actor.motion_admission_revision().await,Err(OperationAdmissionError::BoundedPlacementInProgress));
    actor.finish_bounded_placement(1).await.unwrap();no_packet(&mut peer).await;
}
#[tokio::test]
async fn bounded_placement_failed_write_terminates_without_retry() {
    let (actor,_peer)=actor_fixture().await;actor.mark_ready().await;
    let revision=actor.motion_admission_revision().await.unwrap();actor.shutdown_writer().await.unwrap();
    assert!(actor.bounded_placement(1,revision,crate::BlockPos{x:10,y:66,z:8},4,[0.0,0.5,0.5]).await.is_err());
    assert_eq!(actor.wait_for_terminal().await,ConnectionState::ConnectionStateUnknown);
    assert!(actor.bounded_placement(1,revision,crate::BlockPos{x:10,y:66,z:8},4,[0.0,0.5,0.5]).await.is_err());
}
