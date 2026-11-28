//! dsh-flow-ui — a GPUI front end for the `dsh-flow` engine.
//!
//! The window opens a conversation with the assistant, lists the workflows the
//! engine can run, and streams run state back into the same app. It is a viewer:
//! the engine owns workflow and model behaviour, the window only drives it.

// A window app, not a console app: starting the binary shows no terminal.
#![cfg_attr(not(test), windows_subsystem = "windows")]
// GPUI element trees are tuples (a row, its state handle, a child) and the
// transcript / markdown renderers take the pieces they need instead of a
// params struct: both lints would only add aliases and indirection to code
// that is read at the call site. Everything else stays at `-D warnings`.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod ai;
mod app;
mod assets;
mod backup;
mod chats;
mod clipboard_image;
mod config;
mod engine;
mod folder;
mod foreground;
mod json;
mod logging;
mod manifest;
mod markdown;
mod mcp;
mod memory;
mod run_store;
mod secrets;
mod select_text;
mod sidebar_store;
mod single_instance;
mod skill;
mod snippet;
mod stats_store;
mod store;
mod text_input;
mod theme;
mod workflow_store;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Pixels, Size, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

/// The size the window asks for when the screen has room.
fn preferred_size() -> Size<Pixels> {
    size(px(1140.), px(720.))
}

/// The bounds to open with.
///
/// The window is created at the size it asks for in device pixels and the
/// display's scale factor is only applied afterwards, which multiplies both its
/// size and where it sits. Asking for about two thirds of the screen and opening
/// near the top left keeps it fully visible on a scaled display, and still gives
/// a generous window on one that needs no scaling.
fn initial_bounds(cx: &App) -> Bounds<Pixels> {
    let wanted = preferred_size();
    let Some(display) = cx.primary_display() else {
        return Bounds::centered(None, wanted, cx);
    };
    let available = display.bounds().size;
    let width: f32 = available.width.into();
    let height: f32 = available.height.into();
    let size = wanted.min(&size(px(width * 0.65), px(height * 0.65)));
    Bounds::new(point(px(width * 0.05), px(height * 0.05)), size)
}

fn main() {
    // Two windows on one data folder would overwrite each other's transcripts,
    // so the first one claims it; a second one wakes the window that already
    // owns it and leaves the files alone.
    let _instance = match single_instance::acquire(&config::AiSettings::home()) {
        Some(guard) => guard,
        None => {
            single_instance::focus_existing();
            return;
        }
    };
    logging::init();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            // Closing every window must not leave a `node` request running in the
            // background. The settings window is separate, so only the last window
            // closing ends the session.
            std::mem::forget(cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    ai::kill_children();
                }
            }));
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(initial_bounds(cx))),
                        window_min_size: Some(size(px(860.), px(560.))),
                        // The title bar is drawn by the app so it can match the window.
                        titlebar: Some(TitlebarOptions {
                            title: Some("PAgent".into()),
                            appears_transparent: true,
                            traffic_light_position: None,
                        }),
                        ..Default::default()
                    },
                    |_, cx| cx.new(app::build),
                )
                .expect("failed to open the main window");

            window
                .update(cx, |view, window, cx| {
                    // Keep whatever the assistants had written before the window
                    // goes away; the request itself is killed afterwards.
                    let entity = cx.entity();
                    window.on_window_should_close(cx, move |_, cx| {
                        entity.update(cx, |view, cx| {
                            view.stop_all_answers(cx);
                            // A workflow run is a separate engine process; it must
                            // not keep writing files with the window gone.
                            view.stop_active_run();
                            // Keep the unsent text and attached files too.
                            view.save_before_close();
                            // The settings window must not outlive the main one.
                            view.close_settings(cx);
                        });
                        true
                    });
                    window.focus(view.composer_focus(), cx);
                })
                .ok();
            cx.activate(true);
        });
    // The window is gone; no engine process should outlive it.
    ai::kill_children();
    // The browser tool leaves its Chromium running so a page survives into the
    // next question; that also means the window has to end it on the way out.
    ai::kill_browsers();
}
