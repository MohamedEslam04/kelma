//! The sign-in splash (the animated logo, the app name and "Signing in…") covers the window at
//! start and is gone once sign-in completes. The animation itself is judged by eye.

use super::*;
use std::time::Duration;

fn frames(ms: u64) {
    testing::mock_elapsed_time(Duration::from_millis(ms));
    testing::mock_elapsed_time(Duration::from_millis(16));
}

#[test]
fn the_splash_shows_at_start_and_hides_after_sign_in() {
    let app = new_window();
    app.window().set_size(LogicalSize::new(1200., 800.));
    app.show().unwrap();
    frames(600);
    assert_eq!(app.get_login_state(), "checking");
    assert!(app.get_splash_shown(), "shown while signing in");

    // The device-code step keeps the splash up too.
    app.set_user_code("ABCD1234".into());
    app.set_verify_url("https://example.invalid/device".into());
    app.set_login_state("code".into());
    frames(600);
    assert!(app.get_splash_shown(), "shown while waiting for the code");

    app.set_login_state("ready".into());
    frames(600);
    assert!(!app.get_splash_shown(), "gone once the main UI is up");
}
