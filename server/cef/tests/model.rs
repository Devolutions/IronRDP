use ironrdp_cef_server::model::{Action, CHROME_HEIGHT, Key, KeyResult, Model, Modifiers};

fn model() -> Model {
    Model::new(320, 240).unwrap()
}

fn pixel(frame: &[u8], x: usize, y: usize) -> &[u8] {
    let offset = (y * 320 + x) * 4;
    &frame[offset..offset + 4]
}

#[test]
fn view_paints_copy_bgra_rows_without_swapping_channels() {
    // oracle: mutant swapping BGRA channels or retaining the borrowed paint buffer.
    let mut model = model();
    let id = model.active_id();
    let mut source = vec![0; 320 * 152 * 4];
    source[..8].copy_from_slice(&[255, 0, 0, 255, 0, 0, 255, 255]);
    source[1280..1288].copy_from_slice(&[0, 255, 0, 255, 255, 255, 255, 255]);
    model.paint_view(id, model.generation(), 320, 152, &source).unwrap();
    source.fill(17);
    let frame = model.compose();
    assert_eq!(pixel(&frame, 0, 88), [255, 0, 0, 255]);
    assert_eq!(pixel(&frame, 1, 88), [0, 0, 255, 255]);
    assert_eq!(pixel(&frame, 0, 89), [0, 255, 0, 255]);
    assert_eq!(pixel(&frame, 1, 89), [255, 255, 255, 255]);
}

#[test]
fn popup_clips_signed_offsets_and_hiding_restores_the_base() {
    // oracle: mutant clipping the wrong popup column or retaining a hidden popup.
    let mut model = model();
    let id = model.active_id();
    let generation = model.generation();
    let base = vec![73; 320 * 152 * 4];
    model.paint_view(id, generation, 320, 152, &base).unwrap();
    let popup = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
    model.paint_popup(id, generation, (-1, 0), 2, 2, &popup).unwrap();
    let frame = model.compose();
    assert_eq!(pixel(&frame, 0, 88), [4, 5, 6, 255]);
    assert_eq!(pixel(&frame, 0, 89), [10, 11, 12, 255]);
    assert_eq!(pixel(&frame, 1, 88), [73; 4]);
    model.hide_popup(id).unwrap();
    assert_eq!(pixel(&model.compose(), 0, 88), [73; 4]);
    model.paint_popup(id, generation, (319, 151), 2, 2, &popup).unwrap();
    assert_eq!(pixel(&model.compose(), 319, 239), [1, 2, 3, 255]);
    model
        .paint_popup(id, generation, (i32::MAX, i32::MIN), 2, 2, &popup)
        .unwrap();
    assert_eq!(pixel(&model.compose(), 319, 239), [73; 4]);
}

#[test]
fn tab_surfaces_are_isolated_and_closed_ids_never_reused() {
    // oracle: mutant sharing tab pixels or accepting paints for a closed tab.
    let mut model = model();
    let first = model.active_id();
    let generation = model.generation();
    model
        .paint_view(first, generation, 320, 152, &vec![31; 320 * 152 * 4])
        .unwrap();
    let second = model.new_tab("https://second.test").unwrap();
    model
        .paint_view(second, generation, 320, 152, &vec![92; 320 * 152 * 4])
        .unwrap();
    assert_eq!(pixel(&model.compose(), 0, 88), [92; 4]);
    model.activate(first).unwrap();
    assert_eq!(pixel(&model.compose(), 0, 88), [31; 4]);
    model.close_tab(second).unwrap();
    assert!(
        model
            .paint_view(second, generation, 320, 152, &vec![92; 320 * 152 * 4])
            .is_err()
    );
    assert!(model.paint_popup(second, generation, (0, 0), 1, 1, &[1; 4]).is_err());
    assert_ne!(model.new_tab("about:blank").unwrap(), second);
}

#[test]
fn resize_rejects_old_frames_even_after_returning_to_the_old_dimensions() {
    // oracle: mutant accepting an old frame after resizing back to its original dimensions.
    let mut model = model();
    let id = model.active_id();
    let old = model.generation();
    model.resize(640, 480).unwrap();
    assert!(model.paint_view(id, old, 320, 152, &vec![1; 320 * 152 * 4]).is_err());
    model.resize(320, 240).unwrap();
    assert!(model.paint_view(id, old, 320, 152, &vec![1; 320 * 152 * 4]).is_err());
    assert!(model.paint_view(id, model.generation(), 320, 152, &[1; 4]).is_err());
    assert!(
        model
            .paint_popup(id, model.generation(), (0, 0), 2, 2, &[1; 4])
            .is_err()
    );
    assert_eq!(model.compose().len(), 320 * 240 * 4);
}

#[test]
fn dimensions_and_tab_count_are_bounded_without_partial_mutation() {
    // oracle: mutant accepting out-of-range dimensions or more than eight tabs.
    assert!(Model::new(319, 240).is_err());
    assert!(Model::new(320, 239).is_err());
    assert!(Model::new(3841, 2160).is_err());
    assert!(Model::new(3840, 2161).is_err());
    let mut model = model();
    assert!(model.resize(u32::MAX, u32::MAX).is_err());
    assert_eq!(model.dimensions(), (320, 240));
    for _ in 1..8 {
        model.new_tab("about:blank").unwrap();
    }
    let active = model.active_id();
    assert!(model.new_tab("about:blank").is_err());
    assert_eq!(model.active_id(), active);
    assert_eq!(model.tabs().len(), 8);
}

#[test]
fn address_editing_handles_surrogate_pairs_and_unicode_boundaries() {
    // oracle: mutant splitting a UTF-16 surrogate pair or deleting part of a Unicode scalar.
    let mut model = model();
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    assert_eq!(model.key(Key::Character('l'), ctrl), KeyResult::Consumed);
    assert!(model.address_focused());
    model.text("https://example.test/");
    model.text_utf16(0xd83d);
    model.text_utf16(0xde00);
    assert_eq!(model.address(), "https://example.test/😀");
    model.key(Key::Backspace, Modifiers::default());
    assert_eq!(model.address(), "https://example.test/");
    model.text("é");
    model.key(Key::Left, Modifiers::default());
    model.key(Key::Delete, Modifiers::default());
    assert_eq!(model.address(), "https://example.test/");
    assert_eq!(
        model.key(Key::Enter, Modifiers::default()),
        KeyResult::Action(Action::Navigate {
            tab: model.active_id(),
            url: "https://example.test/".into()
        })
    );
    assert!(!model.address_focused());
    assert_eq!(model.key(Key::Backspace, Modifiers::default()), KeyResult::Unhandled);
}

#[test]
fn address_character_keys_leave_text_insertion_to_the_caller() {
    // oracle: mutant consuming ordinary character keys before the caller can insert text.
    let mut model = model();
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    assert_eq!(model.key(Key::Character('l'), ctrl), KeyResult::Consumed);
    assert!(model.address_focused());
    assert_eq!(
        model.key(Key::Character('x'), Modifiers::default()),
        KeyResult::Unhandled
    );
    assert_eq!(model.address(), "about:blank");
    assert!(model.text_utf16(u16::from(b'x')));
    assert_eq!(model.address(), "x");
    assert_eq!(model.key(Key::Character('a'), ctrl), KeyResult::Consumed);
    assert!(model.text("replacement"));
    assert_eq!(model.address(), "replacement");
}

#[test]
fn metadata_updates_do_not_overwrite_address_edits() {
    // oracle: mutant overwriting an address draft with a navigation update.
    let mut model = model();
    let id = model.active_id();
    model
        .update_tab(id, Some("A title"), Some("https://a.test"), Some(true), Some("failed"))
        .unwrap();
    assert_eq!(model.address(), "https://a.test");
    model.key(
        Key::Character('l'),
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
    model.text("https://draft.test");
    model
        .update_tab(id, None, Some("https://redirect.test"), Some(false), None)
        .unwrap();
    assert_eq!(model.address(), "https://draft.test");
    assert_eq!(model.tabs()[0].title, "A title");
    assert_eq!(model.tabs()[0].url, "https://redirect.test");
    assert!(!model.tabs()[0].loading);
    assert_eq!(model.tabs()[0].error.as_deref(), Some("failed"));
    model.key(Key::Escape, Modifiers::default());
    assert_eq!(model.address(), "https://redirect.test");
}

#[test]
fn mouse_hit_regions_and_shortcuts_route_to_the_right_tab() {
    // oracle: mutant routing shell clicks into browser content or the wrong tab.
    let mut model = model();
    let first = model.active_id();
    assert_eq!(CHROME_HEIGHT, 88);
    assert_eq!(model.click(16, 50), Some(Action::Back { id: first }));
    assert_eq!(model.click(48, 50), Some(Action::Forward { id: first }));
    assert_eq!(model.click(80, 50), Some(Action::Reload { id: first }));
    assert_eq!(model.click(190, 16), Some(Action::NewTab));
    assert_eq!(model.click(168, 16), Some(Action::CloseTab { id: first }));
    assert_eq!(model.click(120, 60), None);
    assert!(model.address_focused());
    assert_eq!(model.content_point(23, 88), Some((23, 0)));
    assert_eq!(model.content_point(320, 88), None);
    assert_eq!(model.content_point(-1, 88), None);
    assert_eq!(model.content_point(i32::MIN, i32::MIN), None);
    model.click(23, 88);
    assert!(!model.address_focused());
    let second = model.new_tab("about:blank").unwrap();
    assert_eq!(model.click(16, 16), Some(Action::Activate { id: first }));
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    assert_eq!(model.key(Key::Character('t'), ctrl), KeyResult::Action(Action::NewTab));
    assert_eq!(
        model.key(Key::Character('w'), ctrl),
        KeyResult::Action(Action::CloseTab { id: second })
    );
    assert_eq!(
        model.key(Key::Tab, ctrl),
        KeyResult::Action(Action::Activate { id: first })
    );
}

#[test]
fn popup_never_overwrites_chrome_and_restores_the_latest_view() {
    // oracle: mutant clipping the wrong popup pixels at the top-left corner.
    let mut model = model();
    let id = model.active_id();
    let generation = model.generation();
    let chrome = model.compose()[..320 * 88 * 4].to_vec();
    let mut popup = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
    model.paint_popup(id, generation, (-1, -1), 2, 2, &popup).unwrap();
    popup.fill(99);
    model
        .paint_view(id, generation, 320, 152, &vec![53; 320 * 152 * 4])
        .unwrap();
    let frame = model.compose();
    assert_eq!(&frame[..320 * 88 * 4], chrome);
    assert_eq!(pixel(&frame, 0, 88), [10, 11, 12, 255]);
    assert_eq!(pixel(&frame, 1, 88), [53; 4]);
    model.hide_popup(id).unwrap();
    assert_eq!(pixel(&model.compose(), 0, 88), [53; 4]);
}

#[test]
fn rejected_paints_preserve_the_current_view_and_popup() {
    // oracle: mutant changing accepted pixels when a malformed paint is rejected.
    let mut model = model();
    let id = model.active_id();
    let generation = model.generation();
    model
        .paint_view(id, generation, 320, 152, &vec![26; 320 * 152 * 4])
        .unwrap();
    model
        .paint_popup(id, generation, (0, 0), 1, 1, &[1, 2, 3, 255])
        .unwrap();
    assert!(
        model
            .paint_view(id, generation, 160, 304, &vec![42; 320 * 152 * 4])
            .is_err()
    );
    assert!(model.paint_view(id, generation, 320, 152, &[42; 4]).is_err());
    assert!(
        model
            .paint_popup(id, generation, (0, 0), u32::MAX, u32::MAX, &[])
            .is_err()
    );
    assert!(model.paint_popup(id, generation, (0, 0), 0, 0, &[]).is_err());
    let frame = model.compose();
    assert_eq!(pixel(&frame, 0, 88), [1, 2, 3, 255]);
    assert_eq!(pixel(&frame, 1, 88), [26; 4]);
}

#[test]
fn closing_the_active_and_last_tab_keeps_a_composable_desktop() {
    // oracle: mutant reusing a closed tab ID or losing the remaining tab's composition.
    let mut model = model();
    let first = model.active_id();
    let second = model.new_tab("about:blank").unwrap();
    model.close_tab(second).unwrap();
    assert_eq!(model.active_id(), first);
    model.close_tab(first).unwrap();
    assert!(model.tabs().is_empty());
    assert!(model.close_tab(first).is_err());
    assert!(model.activate(second).is_err());
    assert_eq!(model.compose().len(), 320 * 240 * 4);
    let third = model.new_tab("about:blank").unwrap();
    assert_ne!(third, first);
    assert_ne!(third, second);
}

#[test]
fn selected_unicode_text_can_be_replaced_without_invalidating_utf8() {
    // oracle: mutant selecting or replacing only part of an emoji.
    let mut model = model();
    model.click(120, 60);
    model.text("a😀é");
    model.key(Key::Left, Modifiers::default());
    model.key(
        Key::Left,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    model.text("Z");
    assert_eq!(model.address(), "aZé");
    model.key(Key::Home, Modifiers::default());
    model.key(Key::Delete, Modifiers::default());
    assert_eq!(model.address(), "Zé");
    model.key(Key::End, Modifiers::default());
    model.text_utf16(0xdc00);
    assert_eq!(model.address(), "Zé\u{fffd}");
    model.text_utf16(0xd800);
    model.text_utf16(u16::from(b'!'));
    assert_eq!(model.address(), "Zé\u{fffd}\u{fffd}!");
}

#[test]
fn new_tab_shortcuts_respect_the_limit_and_navigation_keys_keep_the_active_id() {
    // oracle: mutant routing navigation to an inactive tab or creating a ninth tab.
    let mut model = model();
    for _ in 1..8 {
        model.new_tab("about:blank").unwrap();
    }
    let id = model.active_id();
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    assert_eq!(model.key(Key::Character('t'), ctrl), KeyResult::Consumed);
    assert_eq!(
        model.key(Key::F5, Modifiers::default()),
        KeyResult::Action(Action::Reload { id })
    );
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    assert_eq!(model.key(Key::Left, alt), KeyResult::Action(Action::Back { id }));
    assert_eq!(model.key(Key::Right, alt), KeyResult::Action(Action::Forward { id }));
    assert_eq!(model.tabs().len(), 8);
}

#[cfg(target_os = "macos")]
#[test]
fn runtime_system_font_draws_text_only_in_chrome() {
    // oracle: mutant drawing shell text into the browser content surface.
    let mut model = model();
    model
        .update_tab(model.active_id(), Some("Browser"), None, None, None)
        .unwrap();
    let without_font = model.compose();
    let font = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").unwrap();
    model.set_font(font).unwrap();
    let with_font = model.compose();
    assert_ne!(&with_font[..320 * 88 * 4], &without_font[..320 * 88 * 4]);
    assert_eq!(&with_font[320 * 88 * 4..], &without_font[320 * 88 * 4..]);
    assert!(model.set_font(vec![0; 16]).is_err());
    assert_eq!(model.compose(), with_font);
}
