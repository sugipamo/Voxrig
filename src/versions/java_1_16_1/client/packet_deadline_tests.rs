mod packet_deadline_tests {
    use super::*;
    use std::{future::{Future, poll_fn}, task::Poll};

    #[tokio::test]
    async fn packet_deadline_is_not_extended_by_capture_processing() {
        let (mut bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
        // The fixture's network reader keeps its original, longer timeout.
        bot.connection_options.play_packet_timeout = Duration::from_millis(100);
        tokio::time::pause();
        let (_writer, reader) = tokio::io::duplex(64);
        let (capture_tx, capture_rx) = mpsc::channel(2);
        let (_movement_tx, movement_rx) = mpsc::channel(2);
        {
            let read = bot.read_loop(reader, capture_rx, movement_rx);
            tokio::pin!(read);
            poll_fn(|cx| {
                assert!(read.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            }).await;
            tokio::time::advance(Duration::from_millis(60)).await;
            let (reply, response) = oneshot::channel();
            capture_tx.send(crate::observation::CaptureCommand {
                request: CoherentObservationRequest::default(), reply,
            }).await.unwrap();
            tokio::select! {
                result = &mut read => panic!("reader ended during capture: {result:?}"),
                result = response => { result.unwrap().unwrap(); }
            }
            tokio::time::advance(Duration::from_millis(41)).await;
            // The original deadline has elapsed. A restarted timeout would
            // still be pending for another 59 ms.
            let result = poll_fn(|cx| match read.as_mut().poll(cx) {
                Poll::Ready(result) => Poll::Ready(result),
                Poll::Pending => panic!("capture restarted the packet deadline"),
            }).await;
            assert_eq!(result.unwrap_err().to_string(), "play packet timed out");
        }
        tokio::time::resume();
        bot.cancel.notify_waiters();
        drop(release);
        server.abort();
    }

    #[tokio::test]
    async fn pending_packet_exits_on_cancel_or_request_channel_close() {
        for exit_kind in 0..3 {
            let (bot, server, release) = ready_test_bot(ConnectionOptions::default()).await;
            let (_writer, reader) = tokio::io::duplex(64);
            let (capture_tx, capture_rx) = mpsc::channel(2);
            let (movement_tx, movement_rx) = mpsc::channel(2);
            {
                let read = bot.read_loop(reader, capture_rx, movement_rx);
                tokio::pin!(read);
                poll_fn(|cx| {
                    assert!(read.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                }).await;
                match exit_kind {
                    0 => bot.cancel.notify_waiters(),
                    1 => drop(capture_tx),
                    2 => drop(movement_tx),
                    _ => unreachable!(),
                }
                timeout(Duration::from_secs(1), read).await.unwrap().unwrap();
            }
            bot.cancel.notify_waiters();
            drop(release);
            server.abort();
        }
    }
}
