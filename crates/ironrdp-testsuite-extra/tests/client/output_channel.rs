use core::num::NonZeroU16;
use core::time::Duration;
use std::sync::Arc;

use ironrdp_client::output_channel::{DropPolicy, output_channel};
use ironrdp_client::rdp::RdpOutputEvent;
use ironrdp_graphics::pointer::DecodedPointer;

fn pointer_bitmap(hotspot_x: u16) -> RdpOutputEvent {
    RdpOutputEvent::PointerBitmap(Arc::new(DecodedPointer {
        width: 1,
        height: 1,
        hotspot_x,
        hotspot_y: 0,
        bitmap_data: vec![0u8; 4],
    }))
}

fn image_event(width: u16) -> RdpOutputEvent {
    RdpOutputEvent::Image {
        buffer: vec![0u32; usize::from(width)],
        width: NonZeroU16::new(width).unwrap(),
        height: NonZeroU16::new(1).unwrap(),
    }
}

#[test]
fn drop_policy_classification() {
    assert_eq!(image_event(1).drop_policy(), DropPolicy::LatestOnly);
    assert_eq!(RdpOutputEvent::PointerDefault.drop_policy(), DropPolicy::LatestOnly);
    assert_eq!(RdpOutputEvent::PointerHidden.drop_policy(), DropPolicy::LatestOnly);
    assert_eq!(
        RdpOutputEvent::PointerPosition { x: 0, y: 0 }.drop_policy(),
        DropPolicy::LatestOnly
    );
    assert_eq!(RdpOutputEvent::Connected.drop_policy(), DropPolicy::MustDeliver);
    assert_eq!(RdpOutputEvent::LoginComplete.drop_policy(), DropPolicy::MustDeliver);
    assert_eq!(RdpOutputEvent::AutoReconnected.drop_policy(), DropPolicy::MustDeliver);
}

/// A burst of `LatestOnly` sends must never block, and the receiver must see
/// only the newest value, not every intermediate one.
#[tokio::test]
async fn latest_only_send_never_blocks_and_drops_stale_values() {
    // Capacity 1 so a burst would immediately fill (and block on) a plain mpsc.
    let (sender, mut receiver) = output_channel(1);

    for width in 1..=50u16 {
        // `try_send` proves this never blocks even without an executing consumer.
        sender.try_send(image_event(width)).expect("LatestOnly never fails");
    }

    let RdpOutputEvent::Image { width, .. } = receiver.recv().await.expect("a value was sent") else {
        panic!("expected Image");
    };
    assert_eq!(width.get(), 50, "receiver must observe only the newest Image");
}

/// Different `LatestOnly` variants occupy independent slots: a burst of Image
/// updates must not clobber a pending PointerPosition update, or vice versa.
#[tokio::test]
async fn latest_only_variants_are_independent() {
    let (sender, mut receiver) = output_channel(1);

    sender
        .send(RdpOutputEvent::PointerPosition { x: 10, y: 20 })
        .await
        .unwrap();
    sender.send(image_event(7)).await.unwrap();

    let mut saw_image = false;
    let mut saw_pointer = false;
    for _ in 0..2 {
        match receiver.recv().await.expect("a value was sent") {
            RdpOutputEvent::Image { width, .. } => {
                assert_eq!(width.get(), 7);
                saw_image = true;
            }
            RdpOutputEvent::PointerPosition { x, y } => {
                assert_eq!((x, y), (10, 20));
                saw_pointer = true;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert!(
        saw_image && saw_pointer,
        "both variants must be delivered independently"
    );
}

/// `MustDeliver` events keep backpressuring a full channel exactly like a
/// plain bounded `mpsc::Sender` would.
#[tokio::test]
async fn must_deliver_backpressures_on_a_full_channel() {
    let (sender, mut receiver) = output_channel(1);

    sender.send(RdpOutputEvent::Connected).await.unwrap();
    // The channel (capacity 1) is now full; a second MustDeliver send must block
    // until the first is drained, not silently drop or reorder.
    let send_second = tokio::spawn({
        let sender = sender.clone();
        async move {
            sender.send(RdpOutputEvent::LoginComplete).await.unwrap();
        }
    });

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !send_second.is_finished(),
        "second MustDeliver send must still be blocked"
    );

    assert!(matches!(receiver.recv().await, Some(RdpOutputEvent::Connected)));
    send_second.await.unwrap();
    assert!(matches!(receiver.recv().await, Some(RdpOutputEvent::LoginComplete)));
}

/// A `LatestOnly` send must never block even while a `MustDeliver` send is
/// backpressured on the same channel. Delivery order favors the pending
/// `MustDeliver` event once it is queued: `recv()` checks for one
/// unconditionally before waiting on anything else, so it comes out first
/// regardless of what `LatestOnly` traffic arrived after it.
#[tokio::test]
async fn latest_only_send_does_not_block_behind_a_full_must_deliver_channel() {
    let (sender, mut receiver) = output_channel(1);

    sender.send(RdpOutputEvent::Connected).await.unwrap();
    // Channel is full for MustDeliver; a LatestOnly send must still succeed immediately.
    sender
        .try_send(image_event(3))
        .expect("LatestOnly must not be blocked by a full MustDeliver channel");

    assert!(matches!(receiver.recv().await, Some(RdpOutputEvent::Connected)));
    assert!(matches!(receiver.recv().await, Some(RdpOutputEvent::Image { .. })));
}

/// `PointerDefault`/`PointerHidden`/`PointerBitmap` are alternative values of
/// one logical cursor-appearance state, not independent axes. Sending an older
/// appearance and then a newer one before either is received must deliver only
/// the newer one, never let the older one arrive after it.
#[tokio::test]
async fn pointer_appearance_variants_share_one_slot_newest_wins() {
    let (sender, mut receiver) = output_channel(1);

    sender.send(pointer_bitmap(1)).await.unwrap();
    sender.send(RdpOutputEvent::PointerHidden).await.unwrap();

    assert!(matches!(receiver.recv().await, Some(RdpOutputEvent::PointerHidden)));

    // And the reverse order.
    sender.send(RdpOutputEvent::PointerHidden).await.unwrap();
    sender.send(pointer_bitmap(2)).await.unwrap();

    match receiver.recv().await {
        Some(RdpOutputEvent::PointerBitmap(pointer)) => assert_eq!(pointer.hotspot_x, 2),
        other => panic!("expected the newer PointerBitmap, got {other:?}"),
    }
}

/// A sustained burst of `LatestOnly` traffic must never starve a pending
/// `MustDeliver` event: `recv()` must return it promptly, not only once the
/// burst happens to stop.
#[tokio::test]
async fn must_deliver_is_not_starved_by_sustained_latest_only_traffic() {
    let (sender, mut receiver) = output_channel(64);

    // Keep a continuous stream of Image updates flowing from another task,
    // the exact overload shape the drop policy exists to survive.
    let flooder = tokio::spawn({
        let sender = sender.clone();
        async move {
            loop {
                let _ = sender.try_send(image_event(1));
                // Yield so this task doesn't monopolize the current-thread
                // test runtime; the point is a continuous stream of ready
                // Image traffic interleaved with the receiver actually
                // running, not starving the scheduler itself.
                tokio::task::yield_now().await;
            }
        }
    });

    // Give the flood a head start so Image is already ready before recv() is
    // ever called, then queue a MustDeliver event into the middle of it.
    tokio::time::sleep(Duration::from_millis(5)).await;
    sender.send(RdpOutputEvent::Connected).await.unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match receiver.recv().await {
                Some(RdpOutputEvent::Connected) => return,
                Some(_) => continue,
                None => panic!("channel closed before Connected was delivered"),
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "Connected must not be starved by sustained Image traffic"
    );
    flooder.abort();
}
