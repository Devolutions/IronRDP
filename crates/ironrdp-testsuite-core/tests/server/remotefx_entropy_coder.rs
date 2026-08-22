use ironrdp_pdu::rdp::capability_sets::EntropyBits;
use ironrdp_server::pick_remotefx_entropy_coder;

#[test]
fn no_preference_picks_whichever_coder_is_offered_first() {
    let offered = [EntropyBits::Rlgr1, EntropyBits::Rlgr3].into_iter();
    assert_eq!(pick_remotefx_entropy_coder(None, offered), Some(EntropyBits::Rlgr1));

    let offered = [EntropyBits::Rlgr3, EntropyBits::Rlgr1].into_iter();
    assert_eq!(pick_remotefx_entropy_coder(None, offered), Some(EntropyBits::Rlgr3));
}

#[test]
fn preference_wins_regardless_of_offered_order() {
    let offered = [EntropyBits::Rlgr1, EntropyBits::Rlgr3].into_iter();
    assert_eq!(
        pick_remotefx_entropy_coder(Some(EntropyBits::Rlgr3), offered),
        Some(EntropyBits::Rlgr3)
    );

    let offered = [EntropyBits::Rlgr3, EntropyBits::Rlgr1].into_iter();
    assert_eq!(
        pick_remotefx_entropy_coder(Some(EntropyBits::Rlgr1), offered),
        Some(EntropyBits::Rlgr1)
    );
}

#[test]
fn unoffered_preference_falls_back_to_whichever_coder_is_offered_first() {
    let offered = [EntropyBits::Rlgr3].into_iter();
    assert_eq!(
        pick_remotefx_entropy_coder(Some(EntropyBits::Rlgr1), offered),
        Some(EntropyBits::Rlgr3)
    );
}

#[test]
fn nothing_offered_picks_nothing() {
    assert_eq!(
        pick_remotefx_entropy_coder(Some(EntropyBits::Rlgr1), core::iter::empty()),
        None
    );
}
