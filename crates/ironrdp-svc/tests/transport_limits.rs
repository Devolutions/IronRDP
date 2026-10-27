use core::error::Error as _;

use bitflags as _;
use ironrdp_core::{DecodeError, DecodeErrorKind};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduError, PduErrorKind, PduResult, pdu_other_err};
use ironrdp_svc::{StaticVirtualChannel, SvcMessage, SvcProcessor};

const FIRST: u32 = 1;
const LAST: u32 = 2;
const COMPRESSED: u32 = 0x0020_0000;
const OVERSIZE: &str = "static channel message exceeds configured maximum size";

#[derive(Debug)]
struct RecordingProcessor {
    limit: Option<usize>,
    messages: Vec<Vec<u8>>,
    fail: bool,
}

ironrdp_core::impl_as_any!(RecordingProcessor);

impl SvcProcessor for RecordingProcessor {
    fn channel_name(&self) -> ChannelName {
        ChannelName::from_static(b"testcap\0")
    }

    fn max_message_size(&self) -> Option<usize> {
        self.limit
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        self.messages.push(payload.to_vec());
        if self.fail {
            return Err(pdu_other_err!("recording processor failure"));
        }
        Ok(vec![SvcMessage::from(vec![0xac])])
    }
}

#[derive(Debug)]
struct LegacyProcessor(RecordingProcessor);

ironrdp_core::impl_as_any!(LegacyProcessor);

impl SvcProcessor for LegacyProcessor {
    fn channel_name(&self) -> ChannelName {
        self.0.channel_name()
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        self.0.process(payload)
    }
}

fn channel(limit: Option<usize>) -> StaticVirtualChannel {
    StaticVirtualChannel::new(RecordingProcessor {
        limit,
        messages: Vec::new(),
        fail: false,
    })
}

fn frame(length: u32, flags: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = length.to_le_bytes().to_vec();
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(data);
    bytes
}

fn messages(channel: &StaticVirtualChannel) -> &[Vec<u8>] {
    &channel
        .channel_processor_downcast_ref::<RecordingProcessor>()
        .expect("recording processor")
        .messages
}

fn assert_reply(responses: Vec<SvcMessage>) {
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].encode_unframed_pdu().expect("encode reply"), [0xac]);
}

fn decode_source(error: &PduError) -> &DecodeError {
    assert!(matches!(error.kind(), PduErrorKind::Decode));
    error
        .source()
        .expect("nested decode error")
        .downcast_ref::<DecodeError>()
        .expect("structured decode error")
}

fn assert_invalid(error: &PduError, field: &str, reason: &str) {
    let source = decode_source(error);
    assert!(matches!(
        source.kind(),
        DecodeErrorKind::InvalidField {
            field: actual_field,
            reason: actual_reason,
            offset: Some(8),
        } if *actual_field == field && *actual_reason == reason
    ));
    assert_eq!(
        source.kind().to_string(),
        format!("invalid `{field}` at offset 8: {reason}")
    );
}

fn assert_rejected_and_recovers(frames: &[Vec<u8>], field: &str, reason: &str) {
    let mut channel = channel(Some(4));
    for pending in &frames[..frames.len() - 1] {
        assert!(channel.process(pending).expect("pending fragment").is_empty());
        assert!(messages(&channel).is_empty());
    }
    let error = channel
        .process(frames.last().expect("rejected frame"))
        .expect_err("reject");
    assert_invalid(&error, field, reason);
    assert!(messages(&channel).is_empty(), "rejected input must not dispatch");
    assert!(channel.process(&frame(3, FIRST, &[9])).expect("fresh first").is_empty());
    assert!(messages(&channel).is_empty());
    assert_reply(channel.process(&frame(3, LAST, &[8, 7])).expect("fresh last"));
    assert_eq!(messages(&channel), [vec![9, 8, 7]]);
}

#[test]
fn default_limit_preserves_unlimited_messages() {
    let processor = LegacyProcessor(RecordingProcessor {
        limit: Some(0),
        messages: Vec::new(),
        fail: false,
    });
    assert_eq!(processor.max_message_size(), None);
    let mut channel = StaticVirtualChannel::new(processor);
    let payload = vec![0x5a; 4097];
    assert_reply(channel.process(&frame(4097, 0, &payload)).expect("unlimited single"));
    assert!(
        channel
            .process(&frame(4097, FIRST, &payload[..2048]))
            .expect("unlimited first")
            .is_empty()
    );
    assert_reply(
        channel
            .process(&frame(4097, LAST, &payload[2048..]))
            .expect("unlimited last"),
    );
    assert_eq!(
        channel
            .channel_processor_downcast_ref::<LegacyProcessor>()
            .expect("legacy processor")
            .0
            .messages,
        [payload.clone(), payload]
    );
}

#[test]
fn unfragmented_below_and_at_cap_dispatch_exact_payloads() {
    for flags in [0, FIRST | LAST] {
        let mut channel = channel(Some(4));
        for payload in [vec![], vec![0x31], vec![1, 2, 3], vec![4, 5, 6, 7]] {
            assert_reply(
                channel
                    .process(&frame(
                        u32::try_from(payload.len()).expect("small length"),
                        flags,
                        &payload,
                    ))
                    .expect("within cap"),
            );
        }
        assert_eq!(
            messages(&channel),
            [vec![], vec![0x31], vec![1, 2, 3], vec![4, 5, 6, 7]]
        );
    }
}

#[test]
fn fragmented_below_and_at_cap_dispatch_only_on_last() {
    for (length, last, expected) in [(3, vec![3], vec![1, 2, 3]), (4, vec![3, 4], vec![1, 2, 3, 4])] {
        let mut channel = channel(Some(4));
        assert!(channel.process(&frame(length, FIRST, &[1])).expect("first").is_empty());
        assert!(channel.process(&frame(length, 0, &[2])).expect("middle").is_empty());
        assert!(messages(&channel).is_empty());
        assert_reply(channel.process(&frame(length, LAST, &last)).expect("last"));
        assert_eq!(messages(&channel), [expected]);
        assert_reply(channel.process(&frame(1, FIRST | LAST, &[8])).expect("next message"));
        assert_eq!(messages(&channel).last(), Some(&vec![8]));
    }
}

#[test]
fn advertised_unfragmented_over_cap_rejects_and_recovers() {
    for flags in [0, FIRST | LAST] {
        assert_rejected_and_recovers(&[frame(5, flags, &[1])], "length", OVERSIZE);
    }
}

#[test]
fn actual_unfragmented_over_cap_rejects_and_recovers() {
    for flags in [0, FIRST | LAST] {
        assert_rejected_and_recovers(&[frame(4, flags, &[1, 2, 3, 4, 5])], "length", OVERSIZE);
    }
}

#[test]
fn advertised_first_over_cap_rejects_and_recovers() {
    assert_rejected_and_recovers(&[frame(5, FIRST, &[1])], "length", OVERSIZE);
    assert_rejected_and_recovers(
        &[frame(4, FIRST, &[0xaa]), frame(5, FIRST, &[0xbb])],
        "length",
        OVERSIZE,
    );
}

#[test]
fn fragment_overruns_reject_at_each_position_and_recover() {
    for frames in [
        vec![frame(4, FIRST, &[1, 2, 3, 4, 5])],
        vec![frame(4, FIRST, &[1, 2]), frame(4, 0, &[3, 4, 5])],
        vec![frame(4, FIRST, &[1]), frame(4, 0, &[2]), frame(4, LAST, &[3, 4, 5])],
    ] {
        assert_rejected_and_recovers(&frames, "length", OVERSIZE);
    }
}

#[test]
fn compressed_advertised_over_cap_rejects_and_recovers() {
    for flags in [0, FIRST, LAST, FIRST | LAST] {
        assert_rejected_and_recovers(&[frame(5, flags | COMPRESSED, &[1])], "length", OVERSIZE);
    }
}

#[test]
fn compressed_wire_overruns_reject_at_each_position_and_recover() {
    for frames in [
        vec![frame(2, COMPRESSED, &[1, 2, 3, 4, 5])],
        vec![frame(2, FIRST | COMPRESSED, &[1, 2, 3, 4, 5])],
        vec![frame(2, FIRST | COMPRESSED, &[1, 2]), frame(2, COMPRESSED, &[3, 4, 5])],
        vec![
            frame(2, FIRST | COMPRESSED, &[1]),
            frame(2, COMPRESSED, &[2]),
            frame(2, LAST | COMPRESSED, &[3, 4, 5]),
        ],
    ] {
        assert_rejected_and_recovers(&frames, "length", OVERSIZE);
    }
}

#[test]
fn oversize_declared_continuations_clear_pending_sequence_and_recover() {
    for flags in [0, LAST, COMPRESSED, LAST | COMPRESSED] {
        assert_rejected_and_recovers(
            &[frame(4, FIRST, &[0xaa]), frame(5, flags, &[0xbb])],
            "length",
            OVERSIZE,
        );
    }
}

#[test]
fn far_over_cap_lengths_and_fragments_reject_and_recover() {
    for flags in [0, FIRST, FIRST | LAST, COMPRESSED, FIRST | COMPRESSED] {
        assert_rejected_and_recovers(&[frame(u32::MAX, flags, &[1])], "length", OVERSIZE);
        assert_rejected_and_recovers(&[frame(4, flags, &[1; 8])], "length", OVERSIZE);
    }
    assert_rejected_and_recovers(
        &[
            frame(2, FIRST | COMPRESSED, &[1; 4]),
            frame(2, LAST | COMPRESSED, &[2; 4]),
        ],
        "length",
        OVERSIZE,
    );
}

#[test]
fn compressed_wire_and_plaintext_lengths_are_independent_within_cap() {
    for (declared, payload) in [(4, vec![1, 2]), (2, vec![1, 2, 3, 4]), (0, vec![1]), (4, vec![])] {
        for flags in [0, FIRST | LAST] {
            let mut channel = channel(Some(4));
            assert_reply(
                channel
                    .process(&frame(declared, flags | COMPRESSED, &payload))
                    .expect("compressed"),
            );
            assert_eq!(messages(&channel), core::slice::from_ref(&payload));
        }
        let mut channel = channel(Some(4));
        let split = payload.len() / 2;
        assert!(
            channel
                .process(&frame(declared, FIRST | COMPRESSED, &payload[..split]))
                .expect("compressed first")
                .is_empty()
        );
        assert!(messages(&channel).is_empty());
        assert_reply(
            channel
                .process(&frame(declared, LAST | COMPRESSED, &payload[split..]))
                .expect("compressed last"),
        );
        assert_eq!(messages(&channel), [payload]);
    }
}

#[test]
fn zero_cap_accepts_only_empty_messages() {
    for flags in [0, FIRST | LAST, COMPRESSED, FIRST | LAST | COMPRESSED] {
        let mut channel = channel(Some(0));
        assert_reply(channel.process(&frame(0, flags, &[])).expect("empty message"));
        assert_eq!(messages(&channel), [Vec::<u8>::new()]);
    }
    for (length, data) in [(1, vec![]), (0, vec![1])] {
        for flags in [0, FIRST, LAST, FIRST | LAST, COMPRESSED, FIRST | COMPRESSED] {
            let mut channel = channel(Some(0));
            let error = channel.process(&frame(length, flags, &data)).expect_err("zero cap");
            assert_invalid(&error, "length", OVERSIZE);
            assert!(messages(&channel).is_empty());
            assert!(channel.process(&frame(0, FIRST, &[])).expect("empty first").is_empty());
            assert_reply(channel.process(&frame(0, LAST, &[])).expect("empty last"));
            assert_eq!(messages(&channel), [Vec::<u8>::new()]);
        }
    }
}

#[test]
fn malformed_sequences_preserve_errors_and_recover() {
    for (frames, field, reason) in [
        (
            vec![frame(3, LAST, &[1])],
            "length",
            "received a channel fragment without a matching initial fragment",
        ),
        (
            vec![frame(3, FIRST, &[1]), frame(3, FIRST, &[2])],
            "flags",
            "received an initial channel fragment before completing the previous sequence",
        ),
        (
            vec![frame(3, FIRST, &[1]), frame(4, 0, &[2])],
            "length",
            "received a channel fragment without a matching initial fragment",
        ),
        (
            vec![frame(3, FIRST, &[1]), frame(4, LAST, &[2])],
            "length",
            "received a channel fragment without a matching initial fragment",
        ),
        (
            vec![frame(1, FIRST, &[1, 2])],
            "length",
            "channel fragment sequence exceeds its declared length",
        ),
        (
            vec![frame(3, FIRST, &[1, 2]), frame(3, 0, &[3, 4])],
            "length",
            "channel fragment sequence exceeds its declared length",
        ),
        (
            vec![frame(3, FIRST, &[1]), frame(3, LAST, &[2, 3, 4])],
            "length",
            "channel fragment sequence exceeds its declared length",
        ),
        (
            vec![frame(3, FIRST, &[1]), frame(3, LAST, &[2])],
            "length",
            "terminal channel fragment does not match its declared length",
        ),
        (
            vec![frame(3, FIRST | LAST, &[1])],
            "length",
            "terminal channel fragment does not match its declared length",
        ),
    ] {
        assert_rejected_and_recovers(&frames, field, reason);
    }
}

#[test]
fn unfragmented_length_mismatches_preserve_errors_and_recover() {
    for (declared, data) in [(3, vec![1, 2]), (1, vec![1, 2])] {
        assert_rejected_and_recovers(
            &[frame(declared, 0, &data)],
            "length",
            "unfragmented channel data does not match its declared length",
        );
    }
}

#[test]
fn truncated_headers_preserve_errors_and_buffered_sequence() {
    for buffered in [false, true] {
        for end in 0..8 {
            let mut channel = channel(Some(4));
            if buffered {
                assert!(channel.process(&frame(3, FIRST, &[9])).expect("first").is_empty());
            }
            let header = frame(3, LAST, &[8, 7]);
            let error = channel.process(&header[..end]).expect_err("truncated header");
            match decode_source(&error).kind() {
                DecodeErrorKind::NotEnoughBytes {
                    received,
                    expected,
                    offset,
                } => {
                    assert_eq!(*received, end);
                    assert_eq!(*expected, 8);
                    assert_eq!(*offset, Some(0));
                }
                other => panic!("expected short header, got {other:?}"),
            }
            assert!(messages(&channel).is_empty());
            if !buffered {
                assert!(channel.process(&frame(3, FIRST, &[9])).expect("fresh first").is_empty());
            }
            assert_reply(channel.process(&frame(3, LAST, &[8, 7])).expect("valid last"));
            assert_eq!(messages(&channel), [vec![9, 8, 7]]);
        }
    }
}

#[test]
fn construction_snapshots_limit_in_both_directions() {
    for later_limit in [Some(0), Some(8), None] {
        let mut channel = channel(Some(4));
        channel
            .channel_processor_downcast_mut::<RecordingProcessor>()
            .expect("recording processor")
            .limit = later_limit;
        assert_reply(
            channel
                .process(&frame(4, FIRST | LAST, &[1, 2, 3, 4]))
                .expect("captured cap"),
        );
        let error = channel.process(&frame(5, FIRST, &[5])).expect_err("captured rejection");
        assert_invalid(&error, "length", OVERSIZE);
        assert_eq!(messages(&channel), [vec![1, 2, 3, 4]]);
        assert!(channel.process(&frame(3, FIRST, &[9])).expect("fresh first").is_empty());
        assert_reply(channel.process(&frame(3, LAST, &[8, 7])).expect("fresh last"));
        assert_eq!(messages(&channel), [vec![1, 2, 3, 4], vec![9, 8, 7]]);
    }
    let mut channel = channel(None);
    channel
        .channel_processor_downcast_mut::<RecordingProcessor>()
        .expect("recording processor")
        .limit = Some(0);
    assert_reply(
        channel
            .process(&frame(5, FIRST | LAST, &[1, 2, 3, 4, 5]))
            .expect("captured unlimited"),
    );
    assert_eq!(messages(&channel), [vec![1, 2, 3, 4, 5]]);
}

#[test]
fn empty_fragments_and_singleton_sequences_do_not_dispatch_early() {
    for (length, first, last, expected) in [
        (0, vec![], vec![], vec![]),
        (1, vec![], vec![6], vec![6]),
        (1, vec![6], vec![], vec![6]),
        (4, vec![1, 2, 3, 4], vec![], vec![1, 2, 3, 4]),
    ] {
        let mut channel = channel(Some(4));
        assert!(
            channel
                .process(&frame(length, FIRST, &first))
                .expect("first")
                .is_empty()
        );
        assert!(
            channel
                .process(&frame(length, 0, &[]))
                .expect("empty middle")
                .is_empty()
        );
        assert!(messages(&channel).is_empty());
        assert_reply(channel.process(&frame(length, LAST, &last)).expect("last"));
        assert_eq!(messages(&channel), [expected]);
    }
}

#[test]
fn processor_error_propagates_without_poisoning_completed_sequence() {
    let mut channel = channel(Some(4));
    channel
        .channel_processor_downcast_mut::<RecordingProcessor>()
        .expect("recording processor")
        .fail = true;
    assert!(channel.process(&frame(2, FIRST, &[1])).expect("first").is_empty());
    let error = channel.process(&frame(2, LAST, &[2])).expect_err("processor failure");
    assert!(matches!(
        error.kind(),
        PduErrorKind::Other {
            description: "recording processor failure"
        }
    ));
    assert_eq!(messages(&channel), [vec![1, 2]]);
    channel
        .channel_processor_downcast_mut::<RecordingProcessor>()
        .expect("recording processor")
        .fail = false;
    assert!(channel.process(&frame(3, FIRST, &[9])).expect("fresh first").is_empty());
    assert_reply(channel.process(&frame(3, LAST, &[8, 7])).expect("fresh last"));
    assert_eq!(messages(&channel), [vec![1, 2], vec![9, 8, 7]]);
}
