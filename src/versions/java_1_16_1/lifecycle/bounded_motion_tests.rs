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
