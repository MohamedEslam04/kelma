//! The camera preview, the camera picker and the Video effects panel, through the real window and
//! engine with the fake call driver: its preview takes the pictures a test hands it through the
//! same effects and mirror as the live camera's.

use super::*;
use crate::backend::call::{CallCmd, Event, FakeDriver};
use crate::{CallStore, VideoFxStore};
use calling::video::I420Frame;
use calling::video::effects::{Background, Filter};
use i_slint_backend_testing::AccessibleRole;

const THREAD: &str = "19:meeting_fx@thread.v2";

struct Fx {
    h: Harness,
    fake: FakeDriver,
    log: Log,
}

impl Fx {
    fn new() -> Self {
        let h = Harness::new();
        let fake = FakeDriver::manual();
        h.engine.calls.set_driver(Arc::new(fake.clone()));
        crate::ui::call::wire(&h.app);
        // A build with video.
        h.app.global::<CallStore>().set_camera_available(true);
        let log = Log::default();
        let (l, run) = (log.clone(), h.runner());
        h.app.global::<CallStore>().on_command(move |name, arg| {
            l.borrow_mut()
                .push(format!("{name} {arg}").trim().to_owned());
            if let Some(c) = CallCmd::parse(&name, &arg) {
                run(Cmd::Call(c));
            }
        });
        Fx { h, fake, log }
    }

    fn cs(&self) -> CallStore<'_> {
        self.h.app.global::<CallStore>()
    }

    fn fx(&self) -> VideoFxStore<'_> {
        self.h.app.global::<VideoFxStore>()
    }

    fn all(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.h.app, label).collect()
    }

    fn with_role(&self, label: &str, role: AccessibleRole) -> ElementHandle {
        self.all(label)
            .into_iter()
            .find(|e| e.accessible_role() == Some(role))
            .unwrap_or_else(|| panic!("no {role:?} {label:?}"))
    }

    fn button(&self, label: &str) -> ElementHandle {
        self.with_role(label, AccessibleRole::Button)
    }

    fn shows(&self, label: &str) -> bool {
        !self.all(label).is_empty()
    }

    fn prejoin(&self) {
        self.h.app.set_current_id(THREAD.into());
        click(&self.button("Join meeting"));
        assert_eq!(self.cs().get_phase(), "prejoin");
    }

    fn camera_on(&self) {
        click(&self.button("Camera"));
        assert!(self.cs().get_prejoin_camera());
    }

    fn sent(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    /// The preview's left-most pixel's red.
    fn preview_left_red(&self) -> u8 {
        let px = self.fx().get_preview().to_rgb8().expect("an RGB picture");
        px.as_slice()[0].r
    }
}

/// An 8x4 camera picture: white on the left half, black on the right.
fn half_white() -> I420Frame {
    let (w, h) = (8, 4);
    let mut data = vec![128u8; I420Frame::len_for(w, h)];
    for y in 0..h {
        for x in 0..w {
            data[y * w + x] = if x < w / 2 { 235 } else { 16 };
        }
    }
    I420Frame::new(w, h, data).unwrap()
}

#[test]
fn the_camera_switch_starts_the_preview_and_the_prejoin_picks_a_camera() {
    let f = Fx::new();
    f.prejoin();
    assert!(f.shows("Your camera is off"));
    assert!(f.fake.previews().is_empty(), "no camera before the switch");

    f.camera_on();
    assert_eq!(f.fake.previews(), ["on default"]);
    assert!(
        f.shows("Your camera turns on when you join"),
        "no picture yet"
    );
    assert!(f.fake.preview_frame(&half_white()));
    assert!(f.fx().get_has_preview());
    let preview = f.with_role("Your camera preview", AccessibleRole::Image);
    assert!(preview.size().width > 300.0, "the preview fills its box");

    // The camera picker sits with the audio ones and restarts the preview on the new camera.
    let picker = f.with_role("Camera", AccessibleRole::Combobox);
    assert_eq!(picker.accessible_value().unwrap(), "Default camera");
    click(&picker);
    click(&f.with_role("USB Webcam", AccessibleRole::ListItem));
    assert_eq!(f.fx().get_camera(), 2);
    assert!(f.sent().contains(&"camera-device 2".to_owned()));
    assert_eq!(f.fake.previews(), ["on default", "off", "on cam-usb"]);
    assert!(
        !f.fx().get_has_preview(),
        "the old camera's picture is gone"
    );

    // Off: the camera is let go and the picture cleared.
    click(&f.button("Camera"));
    assert_eq!(f.fake.previews().last().unwrap(), "off");
    assert!(!f.fx().get_has_preview());
    assert!(f.shows("Your camera is off"));
    assert!(
        !f.fake.preview_frame(&half_white()),
        "nothing listens any more"
    );
}

#[test]
fn leaving_the_prejoin_lets_the_camera_go() {
    let f = Fx::new();
    f.prejoin();
    f.camera_on();
    click(&f.button("Join now"));
    assert_eq!(f.fake.previews(), ["on default", "off"]);
    f.fake.emit(Event::Connected);
    assert_eq!(f.cs().get_phase(), "connected");
    assert_eq!(f.fake.previews().len(), 2, "no preview in the call");

    let g = Fx::new();
    g.prejoin();
    g.camera_on();
    click(&g.button("Cancel"));
    assert_eq!(g.fake.previews(), ["on default", "off"]);
}

#[test]
fn the_effects_panel_opens_and_blur_applies_to_what_is_sent() {
    let f = Fx::new();
    f.prejoin();
    f.camera_on();
    assert!(!f.fx().get_open());
    click(&f.button("Video effects"));
    assert!(f.fx().get_open());
    assert!(f.shows("Background"));
    for tile in ["None", "Blur", "Strong blur", "Bookshelf", "Bright office"] {
        assert!(f.shows(tile), "{tile}");
    }
    let none = f.button("None");
    assert_eq!(none.accessible_checked(), Some(true));

    click(&f.button("Blur"));
    assert!(f.sent().contains(&"fx-background blur".to_owned()));
    assert_eq!(f.fx().get_background(), "blur");
    assert_eq!(f.button("Blur").accessible_checked(), Some(true));
    assert_eq!(f.h.engine.calls.effects().background, Background::Blur);

    click(&f.button("Black and white"));
    assert_eq!(f.fx().get_filter(), "mono");
    assert_eq!(f.h.engine.calls.effects().filter, Filter::Mono);

    let bright = f.with_role("Adjust brightness", AccessibleRole::Switch);
    click(&bright);
    assert!(f.fx().get_brighten());
    assert!(f.h.engine.calls.effects().brighten);

    click(&f.button("Close video effects"));
    assert!(!f.fx().get_open());
    assert_eq!(f.fx().get_background(), "blur", "closing keeps the choice");
}

#[test]
fn mirror_changes_only_the_self_view() {
    let f = Fx::new();
    f.prejoin();
    f.camera_on();
    assert!(f.fx().get_mirror(), "mirrored by default, as Teams does");
    f.fake.preview_frame(&half_white());
    assert!(
        f.preview_left_red() < 40,
        "mirrored: the right (black) half shows on the left"
    );
    let sent = f.h.engine.calls.effects();

    click(&f.button("Video effects"));
    click(&f.with_role("Mirror my video", AccessibleRole::Switch));
    assert!(f.sent().contains(&"fx-mirror off".to_owned()));
    assert!(!f.fx().get_mirror());
    f.fake.preview_frame(&half_white());
    assert!(f.preview_left_red() > 200, "not mirrored now");
    assert_eq!(
        f.h.engine.calls.effects(),
        sent,
        "what the others see is untouched"
    );
}

#[test]
fn the_call_s_more_panel_opens_the_effects() {
    let f = Fx::new();
    f.prejoin();
    click(&f.button("Join now"));
    f.fake.emit(Event::Connected);
    click(&f.button("More"));
    assert!(
        f.all("Camera")
            .iter()
            .any(|e| e.accessible_role() == Some(AccessibleRole::Combobox)),
        "More lists the camera with the audio devices"
    );
    click(&f.button("Video effects"));
    assert!(f.fx().get_open());
    click(&f.button("Strong blur"));
    assert_eq!(
        f.h.engine.calls.effects().background,
        Background::StrongBlur
    );
    click(&f.button("Close video effects"));
    assert!(f.shows("Device settings"), "back to More");
}
