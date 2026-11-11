use ironrdp_cef_server::download::{Budget, Limits, collision_name, safe_name};

// oracle: mutant accepting unsafe filenames or omitting collision suffixes.
#[test]
fn unsafe_names_are_rejected_without_rewriting() {
    for name in [
        "",
        "..",
        ".hidden",
        "../escape",
        "a\\b",
        "C:foo",
        "a/b",
        "a\nb",
        "NUL.txt",
        "COM1",
        "tail.",
        "tail ",
        "spoof\u{202e}.txt",
    ] {
        assert!(safe_name(name).is_err(), "{name:?}");
    }
    assert_eq!(safe_name("été 🌍.bin").unwrap(), "été 🌍.bin");
    assert_eq!(collision_name("a.bin", 1), "a (1).bin");
    assert_eq!(collision_name("no-extension", 2), "no-extension (2)");
}

// oracle: mutant refunding observed bytes after a failed download.
#[test]
fn limits_count_growth_once_and_never_refund_failed_downloads() {
    let mut budget = Budget::new(Limits {
        file_bytes: 4,
        session_bytes: 6,
        concurrent: 2,
    })
    .unwrap();
    budget.start(1).unwrap();
    budget.observe(1, 3).unwrap();
    budget.observe(1, 2).unwrap();
    budget.observe(1, 4).unwrap();
    assert_eq!(budget.used(), 4);
    assert!(budget.observe(1, 5).is_err());
    assert_eq!(budget.used(), 5);
    budget.finish(1);
    budget.start(2).unwrap();
    budget.observe(2, 1).unwrap();
    assert!(budget.observe(2, 2).is_err());
    assert_eq!(budget.used(), 7);
}

// oracle: mutant changing the download defaults or accepting invalid resource limits.
#[test]
fn concurrency_and_configuration_are_bounded() {
    let defaults = Limits::default();
    assert_eq!(
        (defaults.file_bytes, defaults.session_bytes, defaults.concurrent),
        (1 << 30, 4 << 30, 2)
    );
    let mut budget = Budget::new(defaults).unwrap();
    budget.start(1).unwrap();
    assert!(budget.start(1).is_err());
    budget.start(2).unwrap();
    assert!(budget.start(3).is_err());
    budget.finish(1);
    budget.start(3).unwrap();
    assert!(
        Limits {
            concurrent: 0,
            ..defaults
        }
        .validate()
        .is_err()
    );
    assert!(
        Limits {
            file_bytes: 0,
            ..defaults
        }
        .validate()
        .is_err()
    );
    assert!(
        Limits {
            file_bytes: 2,
            session_bytes: 1,
            ..defaults
        }
        .validate()
        .is_err()
    );
}

// oracle: mutant routing download controls into browser content.
#[test]
fn download_status_and_cancel_hit_region_are_owned_by_the_shell() {
    use ironrdp_cef_server::model::{Action, Model};
    let mut model = Model::new(640, 480).unwrap();
    assert_eq!(model.content_point(639, 479), Some((639, 391)));
    let generation = model.generation();
    assert!(
        model
            .set_download_status("Delivering: browser 4 B / client 0 B")
            .unwrap()
    );
    assert_eq!(model.content_dimensions(), (640, 360));
    assert!(generation < model.generation());
    assert!(
        !model
            .set_download_status("Delivering: browser 4 B / client 0 B")
            .unwrap()
    );
    assert_eq!(model.content_point(639, 479), None);
    assert_eq!(model.click(543, 479), None);
    assert_eq!(model.click(544, 448), Some(Action::CancelDownloads));
    assert_eq!(model.click(639, 479), Some(Action::CancelDownloads));
    assert_eq!(model.click(639, 480), None);
    assert!(model.set_download_status("").unwrap());
    assert_eq!(model.content_dimensions(), (640, 392));
    assert_eq!(model.content_point(639, 479), Some((639, 391)));
}

#[test]
fn download_status_keeps_new_and_resized_tab_surfaces_inside_the_viewport() {
    // oracle: mutant sizing new or resized tab surfaces over the download bar.
    use ironrdp_cef_server::model::Model;
    let mut model = Model::new(640, 480).unwrap();
    model.set_download_status("Delivering").unwrap();
    let first = model.active_id();
    let second = model.new_tab("about:blank").unwrap();
    for tab in [first, second] {
        model
            .paint_view(tab, model.generation(), 640, 360, &vec![17; 640 * 360 * 4])
            .unwrap();
        model.activate(tab).unwrap();
        assert_eq!(model.compose().len(), 640 * 480 * 4);
    }
    model.resize(320, 240).unwrap();
    assert_eq!(model.content_dimensions(), (320, 120));
    for tab in [first, second] {
        model
            .paint_view(tab, model.generation(), 320, 120, &vec![53; 320 * 120 * 4])
            .unwrap();
        model.activate(tab).unwrap();
        let frame = model.compose();
        assert_eq!(frame.len(), 320 * 240 * 4);
        assert_eq!(&frame[320 * 207 * 4..320 * 207 * 4 + 4], &[53; 4]);
    }
}

#[test]
fn popup_pixels_cannot_overwrite_the_download_bar() {
    // oracle: mutant drawing a browser popup over the download bar.
    use ironrdp_cef_server::model::Model;
    let mut model = Model::new(320, 240).unwrap();
    model.set_download_status("Delivering").unwrap();
    let tab = model.active_id();
    model
        .paint_view(tab, model.generation(), 320, 120, &vec![73; 320 * 120 * 4])
        .unwrap();
    let before = model.compose();
    model
        .paint_popup(tab, model.generation(), (0, 119), 2, 2, &[1, 2, 3, 255].repeat(4))
        .unwrap();
    let frame = model.compose();
    assert_eq!(&frame[320 * 207 * 4..320 * 207 * 4 + 4], &[1, 2, 3, 255]);
    assert_eq!(&frame[320 * 208 * 4..], &before[320 * 208 * 4..]);
    model.hide_popup(tab).unwrap();
    assert_eq!(model.compose(), before);
}
