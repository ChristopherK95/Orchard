//! OS notifications for Tabs you're not looking at. Clicking one brings the window forward and
//! opens that session's Tab (the frontend listens for `show-session`).
//!
//! This uses `notify-rust` directly because `tauri-plugin-notification` can't report clicks on
//! desktop. On Windows a body click arrives as `Default`; on Linux the notification server reports
//! it as the `default` action, which has to be offered for the click to count.
//!
//! The notification carries the app's name and icon: on Windows through the installed shortcut's
//! AppUserModelID (the bundle identifier), on Linux through the app name and the icon the package
//! installs under the binary's name.

use editor_core::SessionId;
use notify_rust::{Notification, NotificationResponse};
use tauri::{AppHandle, Emitter, Manager};

pub fn notify(app: AppHandle, session_id: SessionId, title: String, body: String) {
    #[cfg(target_os = "windows")]
    let app_id = installed().then(|| app.config().identifier.clone());
    // Waiting for the click blocks until the notification is clicked or goes away.
    std::thread::spawn(move || {
        let mut notification = Notification::new();
        notification.summary(&title).body(&body).appname("Orchard");
        #[cfg(target_os = "windows")]
        if let Some(app_id) = &app_id {
            notification.app_id(app_id);
        }
        #[cfg(target_os = "linux")]
        notification.icon("orchard").action("default", "Open");
        let Ok(handle) = notification.show() else {
            return;
        };
        let _ = handle.wait_for_response(|response: &NotificationResponse| {
            let clicked = match response {
                NotificationResponse::Default => true,
                NotificationResponse::Action(action) => action == "default",
                _ => false,
            };
            if clicked {
                bring_forward(&app, session_id);
            }
        });
    });
}

/// Whether this is the installed app, whose shortcut registers the identifier as its
/// AppUserModelID. A build run from `target/` has none, and a toast naming an unregistered one
/// isn't shown, so those fall back to the default (PowerShell's).
#[cfg(target_os = "windows")]
fn installed() -> bool {
    let Ok(exe) = tauri::utils::platform::current_exe() else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    !(dir.ends_with("target/debug") || dir.ends_with("target/release"))
}

fn bring_forward(app: &AppHandle, session_id: SessionId) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit("show-session", session_id);
}
