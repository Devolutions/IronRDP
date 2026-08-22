use ironrdp_client::rdp::{RdpInputEvent, RdpInputSender};

#[test]
fn input_sender_reservation_prevents_state_changes_without_queue_capacity() {
    let (sender, mut receiver) = RdpInputSender::channel(1);
    let permit = sender.try_reserve().expect("the empty queue has capacity");
    assert!(sender.try_reserve().is_err());

    permit.send(RdpInputEvent::Resize {
        width: 1024,
        height: 768,
        scale_factor: 100,
        physical_size: None,
    });

    assert!(matches!(receiver.try_recv(), Ok(RdpInputEvent::Resize { .. })));
}
