//! The settings page in its own fixed window.
//!
//! The page still lives on [`DshFlowApp`] — the same fields, the same actions —
//! so a change made here is visible to the main window immediately. Only the
//! root view differs: a thin wrapper that draws the page in its own frame.

use super::*;
use gpui::{
    BoxShadow, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, hsla,
};

/// Radius shared by the window's four corners.
const SETTINGS_RADIUS: f32 = 12.;

/// Room left around the panel so its drop shadow can fade out inside the
/// window; the window is transparent here.
const SHADOW_MARGIN: f32 = 28.;

/// The visible panel, not counting the shadow margin.
const PANEL_WIDTH: f32 = 820.;
const PANEL_HEIGHT: f32 = 620.;

/// The root view of the settings window.
pub(crate) struct SettingsWindow {
    app: Entity<DshFlowApp>,
    /// Keeps the repaint subscription alive for as long as the window exists.
    ///
    /// Dropping it unsubscribes, so closing the window must not leak one more
    /// observer per open.
    _observer: gpui::Subscription,
    /// True once the close hook was registered; a repaint must not stack another.
    hooked: bool,
}

impl SettingsWindow {
    /// Build the window's root view for the app whose settings it shows.
    pub(crate) fn new(app: Entity<DshFlowApp>, cx: &mut Context<Self>) -> Self {
        // Every value lives on the app, so a change from either window has to
        // repaint this one too.
        let observer = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app,
            _observer: observer,
            hooked: false,
        }
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.hooked {
            self.hooked = true;
            let app = self.app.clone();
            // Alt+F4 and the window manager close the window without going
            // through the × button, so the app's handle is dropped here as well;
            // otherwise every later open would first try a dead window.
            window.on_window_should_close(cx, move |_, cx| {
                app.update(cx, |app, cx| {
                    app.settings_window = None;
                    cx.notify();
                });
                true
            });
        }
        self.app.update(cx, |app, cx| {
            let palette = app.palette();
            // The page is drawn from this window, so its text system is the one
            // the instruction editor folds against.
            app.window_text = Some(window.text_system().clone());
            div()
                .id("settings-window")
                .flex()
                .flex_col()
                .size_full()
                .min_w(px(0.))
                // The margin is transparent: it is where the panel's shadow
                // fades into the desktop.
                .p(px(SHADOW_MARGIN))
                .text_color(rgb(palette.text))
                // The page has one closable thing of its own: the inline colour
                // grid, and Escape is the shortcut every picker answers to.
                .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" && view.color_menu.is_some() {
                        view.color_menu = None;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.))
                        .min_w(px(0.))
                        // gpui-pre 0.3.14 clips overflow with a rectangle, so each
                        // piece rounds its own corners and the transparent
                        // background shows the desktop outside them. This panel
                        // carries the shadow for the whole shape.
                        .rounded(px(SETTINGS_RADIUS))
                        .shadow(vec![
                            BoxShadow {
                                color: hsla(0., 0., 0., 0.18),
                                offset: point(px(0.), px(6.)),
                                blur_radius: px(20.),
                                spread_radius: px(-1.),
                                inset: false,
                            },
                            BoxShadow {
                                color: hsla(0., 0., 0., 0.1),
                                offset: point(px(0.), px(2.)),
                                blur_radius: px(6.),
                                spread_radius: px(-1.),
                                inset: false,
                            },
                        ])
                        .child(settings_titlebar(palette, cx))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_h(px(0.))
                                .min_w(px(0.))
                                // The navigation colour continues behind the page
                                // sheet, so the sheet's rounded top-left corner
                                // shows it through the notch instead of a sharp
                                // seam between the two areas.
                                .bg(rgb(palette.panel))
                                .rounded_bl(px(SETTINGS_RADIUS))
                                .rounded_br(px(SETTINGS_RADIUS))
                                .child(app.render_settings(palette, cx)),
                        ),
                )
        })
    }
}

/// The window's own strip: title and drag area, then the only window button.
fn settings_titlebar(palette: Palette, cx: &mut Context<DshFlowApp>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(TITLEBAR_HEIGHT))
        .flex_shrink_0()
        .bg(rgb(palette.chrome))
        .rounded_tl(px(SETTINGS_RADIUS))
        .rounded_tr(px(SETTINGS_RADIUS))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .flex_1()
                .min_w(px(0.))
                .pl_3()
                .window_control_area(WindowControlArea::Drag)
                .child(div().text_xs().text_color(rgb(palette.text)).child("设置")),
        )
        // Outside the drag area: a click here must close, not move the window.
        .child(
            div()
                .id("settings-close")
                .flex()
                .items_center()
                .justify_center()
                .w(px(46.))
                .h_full()
                .flex_shrink_0()
                // Its hover fill reaches the window's top-right corner.
                .rounded_tr(px(SETTINGS_RADIUS))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.failed)))
                .text_color(rgb(palette.text))
                .on_click(cx.listener(|view, _, window, _| {
                    // The app must forget the window before it goes, or the next
                    // open would try to raise a handle that no longer exists.
                    view.settings_window = None;
                    window.remove_window();
                }))
                .child(div().text_size(px(19.)).child("×")),
        )
}

impl DshFlowApp {
    /// Show the settings in their own fixed window, or bring that window up.
    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.refresh_model_lists(cx);
        if self.settings_tab == SettingsTab::Instructions {
            self.load_instructions(cx);
        }
        // Opening a window draws it once before returning, and that draw reads
        // this view; doing it inside the current event would borrow it twice.
        // The deferral waits until the event (and its borrow) is over.
        let app = cx.entity();
        cx.defer(move |cx| open_settings_window(&app, cx));
    }

    /// Close the settings window, if one is open; used when the main window goes.
    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// Open the settings window, or focus the one already open.
fn open_settings_window(app: &Entity<DshFlowApp>, cx: &mut App) {
    if let Some(handle) = app.read(cx).settings_window {
        // Still open: raising it beats stacking a second copy.
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(settings_bounds(cx))),
        titlebar: Some(TitlebarOptions {
            title: Some("设置".into()),
            // No system frame: the window draws its own strip and close button.
            appears_transparent: true,
            traffic_light_position: None,
        }),
        // Transparent outside the rounded pieces; the desktop shows through the
        // four corners.
        window_background: WindowBackgroundAppearance::Transparent,
        // A dialog, not a second workspace: one fixed, non-resizable frame.
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let entity = app.clone();
    match cx.open_window(options, move |_, cx| {
        cx.new(|cx| SettingsWindow::new(entity.clone(), cx))
    }) {
        Ok(handle) => {
            // gpui only knows Normal and PopUp; a Normal window is created with
            // `WS_SYSMENU` and no caption, which Windows still gives a 1px
            // non-client frame. Strip it, then let DWM round what is left.
            let _ = handle.update(cx, |_, window, _| {
                remove_window_frame(window);
                // The style change can reset the composition attributes, so ask
                // for the transparent background again before rounding.
                window.set_background_appearance(WindowBackgroundAppearance::Transparent);
                round_settings_window(window);
            });
            app.update(cx, |app, cx| {
                app.settings_window = Some(handle);
                cx.notify();
            });
        }
        Err(error) => app.update(cx, |app, cx| {
            app.banner = Some(Banner::error(format!("打不开设置窗口：{error}")));
            cx.notify();
        }),
    }
}

/// Turn the window into a popup so Windows stops drawing its 1px frame.
///
/// `WS_EX_APPWINDOW` is left alone, so the window keeps its taskbar entry and
/// Alt+Tab slot; only the frame styles are dropped.
#[cfg(target_os = "windows")]
fn remove_window_frame(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const GWL_STYLE: i32 = -16;
    const WS_POPUP: isize = 0x8000_0000u32 as isize;
    const WS_MINIMIZEBOX: isize = 0x0002_0000;
    const WS_MAXIMIZEBOX: isize = 0x0001_0000;
    const WS_SYSMENU: isize = 0x0008_0000;
    const WS_THICKFRAME: isize = 0x0004_0000;
    const WS_CAPTION: isize = 0x00C0_0000;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        fn SetWindowPos(
            hwnd: isize,
            insert_after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get();
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let wanted = (style
            & !(WS_THICKFRAME | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_CAPTION))
            | WS_POPUP;
        SetWindowLongPtrW(hwnd, GWL_STYLE, wanted);
        // Recalculate the non-client area, without moving or resizing.
        SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn remove_window_frame(_window: &Window) {}

/// Ask the compositor to round the window and drop its 1px frame border.
///
/// gpui-pre 0.3.14 clips overflow with a rectangle only, so drawing a rounded root
/// does not work: the square children paint over the corners. The window
/// manager can clip the whole window instead. Attributes unknown to older
/// Windows are ignored, leaving the window square.
#[cfg(target_os = "windows")]
fn round_settings_window(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // `Window::window_handle` is gpui's own id wrapper; the raw handle is the
    // trait method of the same name.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };

    /// `DWMWA_WINDOW_CORNER_PREFERENCE`.
    const CORNER_PREFERENCE: u32 = 33;
    /// `DWMWA_BORDER_COLOR`.
    const BORDER_COLOR: u32 = 34;
    /// `DWMWCP_ROUND`.
    const ROUND: i32 = 2;
    /// `DWMWA_COLOR_NONE`, which removes the frame border.
    const COLOR_NONE: u32 = 0xFFFF_FFFE;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get();
    unsafe {
        let round = ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            CORNER_PREFERENCE,
            (&round as *const i32).cast(),
            core::mem::size_of::<i32>() as u32,
        );
        let none = COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            BORDER_COLOR,
            (&none as *const u32).cast(),
            core::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn round_settings_window(_window: &Window) {}

/// A fixed size, shrunk only when the display itself is smaller.
fn settings_bounds(cx: &App) -> Bounds<Pixels> {
    let wanted = size(
        px(PANEL_WIDTH + SHADOW_MARGIN * 2.),
        px(PANEL_HEIGHT + SHADOW_MARGIN * 2.),
    );
    let Some(display) = cx.primary_display() else {
        return Bounds::centered(None, wanted, cx);
    };
    let available = display.bounds().size;
    let width: f32 = available.width.into();
    let height: f32 = available.height.into();
    let size = wanted.min(&size(px(width * 0.7), px(height * 0.7)));
    Bounds::centered(None, size, cx)
}
