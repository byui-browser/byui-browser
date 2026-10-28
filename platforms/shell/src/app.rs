//! The winit application shared by the desktop shells: window creation,
//! pointer handling for the painted chrome, and presenting frames through
//! `softbuffer`.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ab_glyph::FontVec;
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use tiny_skia::Pixmap;
use webapis::AlertSink;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::error::EventLoopError;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Theme, Window, WindowAttributes, WindowId};

use crate::toolbar::{self, ChromeState, Control};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How a native shell wants its window set up.
///
/// Every shell keeps the operating system's own window decorations (title
/// bar buttons, borders, resize handles); the differences between desktops
/// are captured here.
#[derive(Clone, Copy, Debug)]
pub struct Platform {
    /// Adjusts the default window attributes before the window is created,
    /// for example to make the titlebar transparent so the toolbar extends
    /// under it. The title and sizes are already set when this runs.
    pub configure_window: fn(WindowAttributes) -> WindowAttributes,
    /// Returns font files to try, in order, for toolbar text. The first one
    /// that loads is used; if none loads the toolbar omits its text.
    pub ui_font_files: fn() -> Vec<PathBuf>,
    /// Logical width at the toolbar's left that the system's window buttons
    /// cover when they overlap the toolbar, as macOS's traffic lights do on a
    /// transparent titlebar. Zero when the system draws its buttons in a
    /// separate titlebar above the window contents.
    pub window_controls_inset: f32,
}

struct App {
    platform: Platform,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    font: Option<FontVec>,
    chrome: ChromeState,
    /// Session history shared by all desktop platforms.
    browser_state: chrome::BrowserState,
    refresh: Option<browser::refresh::RefreshClient>,
    /// Loaded document, retained independently of the size-dependent pixmap.
    loaded_page: Option<browser::Page>,
    /// Pointer position in logical pixels while it is over the window.
    cursor: Option<(f64, f64)>,
    /// The rendered page and the device size it was rendered for.
    page: Option<((u32, u32), Pixmap)>,
    /// Last presented window contents in softbuffer's 0x00RRGGBB format,
    /// kept so chrome-only changes repaint just the toolbar strip.
    frame: Vec<u32>,
    /// Window size, scale-factor bits, and theme `frame` was painted for.
    frame_key: Option<(u32, u32, u64, bool)>,
    last_toolbar_click: Option<Instant>,
    /// Shared browser runtime and persistent storage backend.
    runtime: browser::BrowserRuntime,
    alerts: Arc<PageAlerts>,
    /// Live document listeners. Clicks are delivered here, off the UI thread.
    page_events: crate::content::PageEvents,
    /// The pointer went down on the page and has not been released yet.
    page_press: bool,
    scripts_evaluated: bool,
}

/// Delivers `alert` on the UI thread, attached to the browser window.
///
/// Page scripts run on the JS engine thread. Showing the dialog from that
/// thread deadlocks: the UI thread is waiting for the script, and the dialog
/// waits for the UI thread. A dialog with no parent window also ends the
/// process event loop, so the browser quits as the alert closes.
struct PageAlerts {
    proxy: Mutex<Option<EventLoopProxy<()>>>,
    pending: Mutex<Vec<(String, mpsc::Sender<()>)>>,
}

impl PageAlerts {
    fn new() -> Self {
        Self {
            proxy: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
        }
    }

    fn set_proxy(&self, proxy: EventLoopProxy<()>) {
        *self.proxy.lock().expect("alert proxy lock") = Some(proxy);
    }

    /// Shows every queued alert on `parent` and releases the waiting script.
    fn present(&self, parent: &Window) {
        let pending = std::mem::take(&mut *self.pending.lock().expect("alert queue lock"));
        for (message, done) in pending {
            MessageDialog::new()
                .set_title("BYUI Browser")
                .set_description(message)
                .set_level(MessageLevel::Info)
                .set_buttons(MessageButtons::Ok)
                .set_parent(parent)
                .show();
            let _ = done.send(());
        }
    }
}

impl AlertSink for PageAlerts {
    fn alert(&self, message: &str) {
        let proxy = self.proxy.lock().expect("alert proxy lock").clone();
        let Some(proxy) = proxy else {
            return;
        };
        let (done_tx, done_rx) = mpsc::channel();
        self.pending
            .lock()
            .expect("alert queue lock")
            .push((message.to_owned(), done_tx));
        if proxy.send_event(()).is_ok() {
            let _ = done_rx.recv();
        }
    }
}

impl App {
    fn new(platform: Platform) -> Self {
        let root = browser::application_data_directory()
            .expect("application data directory should be available");
        let runtime =
            browser::BrowserRuntime::open(root).expect("browser storage should initialize");
        Self::with_runtime(platform, runtime)
    }

    fn with_runtime(platform: Platform, runtime: browser::BrowserRuntime) -> Self {
        let refresh = browser::refresh::RefreshClient::new()
            .map_err(|error| {
                eprintln!("Refresh initialization failed: {error}");
            })
            .ok();
        Self {
            refresh,
            platform,
            window: None,
            surface: None,
            font: None,
            chrome: ChromeState {
                window_controls_inset: platform.window_controls_inset,
                ..ChromeState::default()
            },
            cursor: None,
            browser_state: chrome::BrowserState::new(),
            loaded_page: None,
            page: None,
            frame: Vec::new(),
            frame_key: None,
            last_toolbar_click: None,
            runtime,
            alerts: Arc::new(PageAlerts::new()),
            page_events: crate::content::PageEvents::new(),
            page_press: false,
            scripts_evaluated: false,
        }
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Accepts a successfully loaded document and records one visit.
    ///
    /// Future navigation should call this only after loading succeeds.
    /// Repaints and Reload must not call it. The current loader and renderer
    /// support only the welcome page; this method does not implement navigation.
    fn page_loaded(&mut self, url: &str, page: browser::Page) {
        self.loaded_page = Some(page);
        self.browser_state.record_visit(url);
        self.page = None;
        self.redraw();
    }

    fn ensure_welcome_loaded(&mut self) {
        if self.loaded_page.is_none() {
            // Record only after the existing loader returns successfully.
            let page = browser::welcome_page();
            self.page_loaded("byui://welcome", page);
        }
    }

    fn set_hovered(&mut self, hovered: Option<Control>) {
        let hovered = hovered.filter(|control| control.is_enabled());
        if self.chrome.hovered != hovered {
            self.chrome.hovered = hovered;
            if let Some(window) = &self.window {
                window.set_cursor(if hovered == Some(Control::Address) {
                    CursorIcon::Text
                } else {
                    CursorIcon::Default
                });
            }
            self.redraw();
        }
    }

    fn refresh_hover(&mut self) {
        let (Some(window), Some((x, y))) = (&self.window, self.cursor) else {
            return;
        };
        let width = window
            .inner_size()
            .to_logical::<f64>(window.scale_factor())
            .width;
        self.set_hovered(toolbar::hit_test(
            x,
            y,
            width,
            self.platform.window_controls_inset,
        ));
    }

    fn activate(&mut self, control: Control) {
        match control {
            Control::Reload => {
                if let Some(refresh) = &self.refresh {
                    // Dropping the handle detaches the task; fetching stays off the UI thread.
                    drop(refresh.refresh(&self.browser_state));
                } else {
                    eprintln!("Refresh unavailable: initialization failed.");
                }
                self.page = None;
                self.scripts_evaluated = false;
                self.redraw();
            }
            // Navigation, tabs, address entry, and the menu are not
            // implemented yet; these controls only give visual feedback.
            Control::Back
            | Control::Forward
            | Control::Address
            | Control::NewTab
            | Control::Menu => {}
        }
    }

    fn paint(&mut self) {
        if self.window.is_none() || self.surface.is_none() {
            return;
        }
        self.ensure_welcome_loaded();
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        let scale = window.scale_factor();
        self.chrome.dark = window.theme() == Some(Theme::Dark);
        let card = toolbar::page_rect(size.width, size.height, scale);
        let card_size = card.map(|card| (card.width, card.height));
        let mut full = self.page.is_none();
        if self.page.as_ref().map(|(size, _)| *size) != card_size {
            // Paint this frame before the page script runs. Evaluation joins
            // the JS engine thread; doing that inside this draw leaves `alert`
            // unable to finish and the window never appears.
            let mut scripts_already_ran = true;
            self.page = card.and_then(|card| {
                crate::content::render_page(
                    self.loaded_page.as_ref().expect("page was loaded"),
                    card.width,
                    card.height,
                    scale,
                    Arc::clone(&self.alerts) as Arc<dyn AlertSink>,
                    &self.runtime,
                    &mut scripts_already_ran,
                )
                .map(|page| ((card.width, card.height), page))
            });
            full = true;
        }
        let key = (size.width, size.height, scale.to_bits(), self.chrome.dark);
        if self.frame_key != Some(key) {
            full = true;
        }
        // Hover, press, and focus only change the toolbar strip; repainting
        // just that strip keeps pointer feedback and window drags smooth.
        let composed = if full {
            toolbar::compose(
                size.width,
                size.height,
                scale,
                &self.chrome,
                self.page.as_ref().map(|(_, page)| page),
                self.font.as_ref(),
            )
        } else {
            toolbar::compose_controls(
                size.width,
                size.height,
                scale,
                &self.chrome,
                self.font.as_ref(),
            )
        };
        let Some(composed) = composed else {
            return;
        };
        if full {
            self.frame
                .resize(size.width as usize * size.height as usize, 0);
            self.frame_key = Some(key);
        }
        // Softbuffer uses 0x00RRGGBB; the composed frame is fully opaque.
        for (target, source) in self.frame.iter_mut().zip(composed.pixels()) {
            *target = (u32::from(source.red()) << 16)
                | (u32::from(source.green()) << 8)
                | u32::from(source.blue());
        }
        surface
            .resize(width, height)
            .expect("failed to resize window surface");
        let mut buffer = surface.buffer_mut().expect("failed to get window buffer");
        buffer.copy_from_slice(&self.frame);
        buffer.present().expect("failed to present window buffer");
    }

    fn cursor_moved(&mut self, x: f64, y: f64) {
        let Some(window) = &self.window else {
            return;
        };
        let scale = window.scale_factor();
        let width = window.inner_size().to_logical::<f64>(scale).width;
        self.cursor = Some((x, y));
        let inset = self.platform.window_controls_inset;
        self.set_hovered(toolbar::hit_test(x, y, width, inset));
    }

    fn pressed(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        if let Some(control) = self.chrome.hovered {
            self.chrome.pressed = Some(control);
            self.redraw();
        } else if self
            .cursor
            .is_some_and(|(_, y)| y < toolbar::TOOLBAR_HEIGHT)
        {
            let cursor_control = self.cursor.and_then(|(x, y)| {
                let width = window
                    .inner_size()
                    .to_logical::<f64>(window.scale_factor())
                    .width;
                toolbar::hit_test(x, y, width, self.platform.window_controls_inset)
            });
            // Disabled controls still occupy toolbar space. Do not let a
            // click on Back or Forward fall through to dragging or zooming.
            if cursor_control.is_some() {
                return;
            }
            let now = Instant::now();
            let double = self
                .last_toolbar_click
                .is_some_and(|last| now - last < DOUBLE_CLICK);
            if double {
                self.last_toolbar_click = None;
                window.set_maximized(!window.is_maximized());
            } else {
                self.last_toolbar_click = Some(now);
                // Empty toolbar space moves the window, like a titlebar;
                // the system's own titlebar and frame keep working too.
                let _ = window.drag_window();
            }
        } else if self
            .cursor
            .is_some_and(|(x, y)| self.pointer_is_on_page(x, y))
        {
            self.page_press = true;
        }
    }

    /// Returns the page node under a logical-pixel window point, matching the
    /// geometry `paint` uses to render the page card.
    fn page_node_at(&self, x: f64, y: f64) -> Option<html::NodeID> {
        let window = self.window.as_ref()?;
        let page = self.loaded_page.as_ref()?;
        let size = window.inner_size();
        let scale = window.scale_factor();
        let card = toolbar::page_rect(size.width, size.height, scale)?;
        crate::content::hit_test_page(
            page,
            (card.width as f64 / scale).ceil() as u32,
            (card.height as f64 / scale).ceil() as u32,
            (x - card.x as f64 / scale) as f32,
            (y - card.y as f64 / scale) as f32,
        )
    }

    /// Whether `(x, y)` is over the document rather than the toolbar.
    ///
    /// Coordinates are logical pixels. A point on a toolbar control, including
    /// a disabled one, is not part of the page.
    fn pointer_is_on_page(&self, x: f64, y: f64) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        let width = window
            .inner_size()
            .to_logical::<f64>(window.scale_factor())
            .width;
        y >= toolbar::TOOLBAR_HEIGHT
            && toolbar::hit_test(x, y, width, self.platform.window_controls_inset).is_none()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attributes = (self.platform.configure_window)(
                Window::default_attributes()
                    .with_title("BYUI Browser")
                    .with_inner_size(LogicalSize::new(1120.0, 720.0))
                    .with_min_inner_size(LogicalSize::new(420.0, 280.0)),
            );
            let window = Rc::new(
                event_loop
                    .create_window(attributes)
                    .expect("failed to create window"),
            );
            let context =
                softbuffer::Context::new(window.clone()).expect("failed to create drawing context");
            self.surface = Some(
                softbuffer::Surface::new(&context, window.clone())
                    .expect("failed to create window surface"),
            );
            self.font = toolbar::load_ui_font((self.platform.ui_font_files)());
            window.request_redraw();
            self.window = Some(window);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_) => {
                self.refresh_hover();
                self.redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(window) = &self.window else {
                    return;
                };
                let position = position.to_logical::<f64>(window.scale_factor());
                self.cursor_moved(position.x, position.y);
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.page_press = false;
                self.set_hovered(None);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => self.pressed(),
                ElementState::Released => {
                    let page_press = self.page_press;
                    self.page_press = false;
                    if let Some(control) = self.chrome.pressed.take() {
                        self.redraw();
                        if self.chrome.hovered == Some(control) {
                            self.activate(control);
                        }
                    } else if page_press
                        && let Some((x, y)) = self.cursor
                        && self.pointer_is_on_page(x, y)
                    {
                        let target = self.page_node_at(x, y);
                        self.page_events.click(target);
                    }
                }
            },
            WindowEvent::RedrawRequested => self.paint(),
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(window) = &self.window {
            let window = Rc::clone(window);
            self.alerts.present(&window);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // `frame_key` is set only after a frame has been presented, so the
        // window is on screen before the page script can request an alert.
        if self.scripts_evaluated || self.frame_key.is_none() {
            return;
        }
        let Some(page) = self.loaded_page.clone() else {
            return;
        };
        self.scripts_evaluated = true;
        // Leave this thread in the event loop. The script runs on the page
        // session thread and asks the loop to present an alert; joining that
        // thread here would deadlock. The session stays alive so a later
        // click can run element and `document` listeners.
        let alerts = Arc::clone(&self.alerts) as Arc<dyn AlertSink>;
        self.page_events.load(&page, alerts, self.runtime.clone());
    }
}

/// Opens the browser window described by `platform` and runs it until the
/// user closes it.
pub fn run(platform: Platform) -> Result<(), EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(platform);
    app.alerts.set_proxy(event_loop.create_proxy());
    event_loop.run_app(&mut app)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SilentAlert;

    impl AlertSink for SilentAlert {
        fn alert(&self, _message: &str) {}
    }

    fn app() -> App {
        App::with_runtime(
            Platform {
                configure_window: |attributes| attributes,
                ui_font_files: Vec::new,
                window_controls_inset: 0.0,
            },
            browser::BrowserRuntime::in_memory(),
        )
    }

    #[test]
    fn welcome_is_recorded_only_after_loading() {
        let mut app = app();
        assert!(app.loaded_page.is_none());
        assert!(app.browser_state.visited_urls().is_empty());

        app.ensure_welcome_loaded();

        assert!(app.loaded_page.is_some());
        assert_eq!(app.browser_state.current_url(), Some("byui://welcome"));
        assert_eq!(app.browser_state.visited_urls(), &["byui://welcome"]);
    }

    #[test]
    fn repaint_resize_and_reload_do_not_record_visits() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let original = app.loaded_page.clone();

        // Exercise the same load guard and renderer used by paint, including
        // an empty content area, changed dimensions, and a changed scale.
        // A silent sink stays off the native dialog if the page script calls
        // `alert`.
        let alert_sink: Arc<dyn AlertSink> = Arc::new(SilentAlert);
        for (width, height, scale) in [(0, 0, 1.0), (160, 120, 1.0), (320, 240, 2.0)] {
            app.ensure_welcome_loaded();
            let rendered = crate::content::render_page(
                app.loaded_page.as_ref().unwrap(),
                width,
                height,
                scale,
                Arc::clone(&alert_sink),
                &app.runtime,
                &mut app.scripts_evaluated,
            );
            assert_eq!(rendered.is_some(), width > 0 && height > 0);
            app.page = rendered.map(|page| ((width, height), page));
            app.scripts_evaluated = true;
            app.activate(Control::Reload);
            assert!(!app.scripts_evaluated);
            assert!(app.page.is_none());
            app.ensure_welcome_loaded();
        }

        assert_eq!(app.loaded_page, original);
        assert_eq!(app.browser_state.current_url(), Some("byui://welcome"));
        assert_eq!(app.browser_state.visited_urls(), &["byui://welcome"]);
    }

    #[test]
    fn successful_page_completions_record_repeated_visits() {
        let mut app = app();
        app.ensure_welcome_loaded();
        // A distinct successful visit to the same page is retained; unlike
        // Reload it explicitly passes through the page-completion boundary.
        app.page_loaded("byui://welcome", browser::welcome_page());
        assert_eq!(app.browser_state.current_url(), Some("byui://welcome"));
        assert_eq!(
            app.browser_state.visited_urls(),
            &["byui://welcome", "byui://welcome"]
        );
    }
}
