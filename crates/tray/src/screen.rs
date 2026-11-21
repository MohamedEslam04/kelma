//! Whether the screen is locked, from the desktop's `org.freedesktop.ScreenSaver` service (KDE,
//! GNOME through its compatibility name, and others). Presence uses it to report Away while the
//! user is not at the computer.

/// `Some(true)` while the screen saver or lock screen is active, `None` when the desktop has no
/// such service (the caller then assumes the user is present). Always `None` on Windows and macOS.
#[cfg(not(target_os = "linux"))]
pub async fn screen_locked() -> Option<bool> {
    None
}

/// `Some(true)` while the screen saver or lock screen is active, `None` when the desktop has no
/// such service (the caller then assumes the user is present).
#[cfg(target_os = "linux")]
pub async fn screen_locked() -> Option<bool> {
    let conn = zbus::Connection::session().await.ok()?;
    let reply = conn
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "GetActive",
            &(),
        )
        .await
        .ok()?;
    reply.body().deserialize::<bool>().ok()
}
