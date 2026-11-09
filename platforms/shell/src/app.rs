//! The winit application shared by the desktop shells: window creation,
//! pointer handling for the painted chrome, and presenting frames through
//! `softbuffer`.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ab_glyph::FontVec;
use net::Url;
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use tiny_skia::Pixmap;
use webapis::{AlertSink, NavigationBehavior};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::error::EventLoopError;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Theme, Window, WindowAttributes, WindowId};

use crate::compat_panel::{CompatState, ShellAuditor, ShellScriptEngine};
use crate::context_menu::{ContextMenu, MenuAction};
use crate::elements_panel::{DockTab, ElementsPanel};
use crate::page_source::PageSource;
use crate::toolbar::{self, ChromeState, Control};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// CSS pixels the page scrolls per arrow-key press.
const ARROW_SCROLL: f32 = 40.0;

/// CSS pixels the page scrolls per wheel line; one wheel notch is three
/// lines.
const WHEEL_LINE_SCROLL: f32 = 20.0;

/// CSS pixels of the previous view kept on screen by Page Up, Page Down, and
/// Space, so the reader keeps their place.
const PAGE_SCROLL_OVERLAP: f32 = 40.0;

/// Smallest and largest supported page zoom, expressed as a multiplier.
const PAGE_ZOOM_RANGE: (f64, f64) = (0.5, 3.0);
/// Page zoom increment for each keyboard shortcut press.
const PAGE_ZOOM_STEP: f64 = 0.1;

/// A caret movement in the address field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaretMove {
    /// One character toward the start.
    Left,
    /// One character toward the end.
    Right,
    /// Before the first character.
    Start,
    /// After the last character.
    End,
}

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
    /// Returns font files to try, in order, for page text, with the
    /// system's default serif font first to match other browsers. The first
    /// one that loads is used; if none loads, pages use the built-in bitmap
    /// font.
    pub page_font_files: fn() -> Vec<PathBuf>,
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
    /// Address text currently shown in the toolbar.
    address: String,
    /// Address text to restore when Escape cancels an edit.
    address_before_edit: String,
    modifiers: ModifiersState,
    /// Loaded document, retained independently of the size-dependent pixmap.
    loaded_page: Option<browser::Page>,
    /// Pointer position in logical pixels while it is over the window.
    cursor: Option<(f64, f64)>,
    /// The rendered page and the device size it was rendered for.
    page: Option<((u32, u32), Pixmap)>,
    /// Layout of `loaded_page` for the current viewport, kept so scrolling
    /// repaints without laying the document out again.
    page_layout: Option<crate::content::PageLayout>,
    /// How far the page is scrolled down, in CSS pixels; kept within
    /// `0..=page_layout.max_scroll()`.
    scroll_y: f32,
    /// Page zoom multiplier. OS display scaling is applied separately.
    page_zoom: f64,
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
    context_menu: Option<ContextMenu>,
    menu_pressed: Option<MenuAction>,
    page_source: Option<PageSource>,
    source_font: Option<FontVec>,
    source_close_pressed: bool,
    elements: Option<ElementsPanel>,
    elements_close_pressed: bool,
    /// Compatibility auditor, created on first use and kept for the session
    /// so later audits reuse its engine probes.
    auditor: Option<Arc<ShellAuditor>>,
    /// Id of the newest audit; results for older ids are ignored.
    audit_id: u64,
    /// Background page loader; `None` when its runtime failed to start.
    loader: Option<browser::PageLoader>,
    /// Wakes the event loop from loader threads; set by [`run`].
    proxy: Option<EventLoopProxy<UserEvent>>,
    /// Id of the newest navigation; results for older ids are ignored.
    navigation_id: u64,
    /// Independent sequence for the most recently started loading operation.
    loading_id: u64,
    navigation_loading: Option<(u64, u64)>,
    /// A completion probe owns Reload's existing task handle; dropping it detaches.
    reload_completion: Option<(u64, Box<dyn Fn() -> bool>)>,
}

/// Messages that background threads post to the UI thread's event loop.
enum UserEvent {
    /// A page script queued an `alert` dialog in [`PageAlerts`].
    AlertPending,
    /// A background page load finished.
    Navigated {
        /// The navigation this result belongs to; stale ones are dropped.
        id: u64,
        /// The loaded page or the reason it failed.
        navigation: browser::Navigation,
        /// History behavior selected by the initiating navigation.
        behavior: NavigationBehavior,
    },
    /// A page script requested navigation through its `Location` object.
    LocationNavigate {
        url: Url,
        behavior: NavigationBehavior,
    },
    /// A background compatibility audit finished.
    AuditFinished {
        /// The audit this result belongs to; stale ones are dropped.
        id: u64,
        report: Box<devtools::compat::AuditReport>,
    },
}

/// Delivers `alert` on the UI thread, attached to the browser window.
///
/// Page scripts run on the JS engine thread. Showing the dialog from that
/// thread deadlocks: the UI thread is waiting for the script, and the dialog
/// waits for the UI thread. A dialog with no parent window also ends the
/// process event loop, so the browser quits as the alert closes.
struct PageAlerts {
    proxy: Mutex<Option<EventLoopProxy<UserEvent>>>,
    pending: Mutex<Vec<(String, mpsc::Sender<()>)>>,
}

impl PageAlerts {
    fn new() -> Self {
        Self {
            proxy: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
        }
    }

    fn set_proxy(&self, proxy: EventLoopProxy<UserEvent>) {
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
        if proxy.send_event(UserEvent::AlertPending).is_ok() {
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
                log::error!("refresh initialization failed: {error}");
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
            address: String::from("byui://welcome"),
            address_before_edit: String::from("byui://welcome"),
            modifiers: ModifiersState::default(),
            loaded_page: None,
            page: None,
            page_layout: None,
            scroll_y: 0.0,
            page_zoom: 1.0,
            frame: Vec::new(),
            frame_key: None,
            last_toolbar_click: None,
            runtime,
            alerts: Arc::new(PageAlerts::new()),
            page_events: crate::content::PageEvents::new(),
            page_press: false,
            scripts_evaluated: false,
            context_menu: None,
            menu_pressed: None,
            page_source: None,
            source_font: None,
            source_close_pressed: false,
            elements: None,
            elements_close_pressed: false,
            auditor: None,
            audit_id: 0,
            loader: browser::PageLoader::new()
                .map_err(|error| log::error!("page loader initialization failed: {error}"))
                .ok(),
            proxy: None,
            navigation_id: 0,
            loading_id: 0,
            navigation_loading: None,
            reload_completion: None,
        }
    }

    fn begin_loading(&mut self) -> u64 {
        self.loading_id += 1;
        self.chrome.loading = true;
        self.redraw();
        self.loading_id
    }

    fn finish_loading(&mut self, id: u64) {
        if id == self.loading_id {
            self.chrome.loading = false;
            self.redraw();
        }
    }

    fn poll_reload(&mut self) {
        if self
            .reload_completion
            .as_ref()
            .is_some_and(|(_, done)| done())
        {
            let (id, _) = self.reload_completion.take().expect("completed reload");
            self.finish_loading(id);
        }
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Accepts a successfully loaded document and records one visit.
    ///
    /// Navigation calls this only after loading succeeds. Repaints, Reload,
    /// and the error page for a failed load must not call it.
    fn page_loaded(&mut self, url: &str, page: browser::Page) {
        self.page_loaded_with_behavior(url, page, NavigationBehavior::Auto);
    }

    fn page_loaded_with_behavior(
        &mut self,
        url: &str,
        page: browser::Page,
        behavior: NavigationBehavior,
    ) {
        log::debug!("page displayed: {url}");
        // Elements is a document snapshot; never retain it across navigation.
        self.close_elements();
        self.loaded_page = Some(page);
        // Scripts of the new document run after its first frame is presented.
        self.scripts_evaluated = false;
        self.frame_key = None;
        match behavior {
            NavigationBehavior::Auto => self.browser_state.record_visit(url),
            NavigationBehavior::Replace => self.browser_state.replace_current(url),
            NavigationBehavior::Reload => {}
        }
        self.address = url.to_owned();
        self.address_before_edit = self.address.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_selected = false;
        self.chrome.address_focused = false;
        self.page = None;
        self.page_layout = None;
        self.scroll_y = 0.0;
        self.redraw();
    }

    fn begin_address_edit(&mut self) {
        self.address_before_edit = self.address.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_selected = true;
        self.chrome.address_focused = true;
        self.redraw();
    }

    fn cancel_address_edit(&mut self) {
        self.address = self.address_before_edit.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_selected = false;
        self.chrome.address_focused = false;
        self.redraw();
    }

    fn submit_address(&mut self) {
        let address = self.address.trim().to_owned();
        self.address = address.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_selected = false;
        self.chrome.address_focused = false;
        if address == "byui://welcome" {
            // Bump the id so an older load still in flight cannot replace it.
            log::debug!("address submitted: {address}");
            log::debug!("navigating to {address}");
            self.navigation_id += 1;
            let id = self.begin_loading();
            self.finish_loading(id);
            self.navigation_loading = None;
            self.page_loaded(&address, browser::welcome_page());
        } else {
            log::debug!("address submitted: {address}");
            self.start_navigation(&address);
        }
        self.redraw();
    }

    /// Loads `address` on the background loader. The result comes back as a
    /// [`UserEvent::Navigated`]; the UI keeps showing the current page until
    /// then.
    fn start_navigation(&mut self, address: &str) {
        self.start_navigation_with_behavior(address, NavigationBehavior::Auto);
    }

    fn start_navigation_with_behavior(&mut self, address: &str, behavior: NavigationBehavior) {
        self.navigation_id += 1;
        let id = self.navigation_id;
        let loading = self.begin_loading();
        let (Some(loader), Some(proxy)) = (&self.loader, self.proxy.clone()) else {
            log::warn!("navigation to {address} unavailable: page loader is not running");
            self.finish_loading(loading);
            self.navigation_loading = None;
            return;
        };
        self.navigation_loading = Some((id, loading));
        log::debug!("navigating to {address}");
        loader.load(address, move |navigation| {
            // A closed event loop means the app is exiting; nothing to do.
            let _ = proxy.send_event(UserEvent::Navigated {
                id,
                navigation,
                behavior,
            });
        });
    }

    /// Applies a finished load. A failure shows the built-in error page and
    /// leaves the typed address in the bar; it is not recorded as a visit.
    fn navigation_finished(
        &mut self,
        id: u64,
        navigation: browser::Navigation,
        behavior: NavigationBehavior,
    ) {
        if id != self.navigation_id {
            return;
        }
        if let Some((navigation_id, loading)) = self.navigation_loading
            && navigation_id == id
        {
            self.navigation_loading = None;
            self.finish_loading(loading);
        }
        match navigation.result {
            Ok(page) => {
                log::debug!(
                    "loaded {} ({} bytes of html)",
                    navigation.address,
                    page.html.len()
                );
                self.page_loaded_with_behavior(&navigation.address, page, behavior)
            }
            Err(error) => {
                // `browser::load` already logged the failure at error level.
                log::debug!("showing error page for {}: {error}", navigation.address);
                self.close_elements();
                self.loaded_page = Some(browser::error_page(&navigation.address, &error));
                self.scripts_evaluated = false;
                self.frame_key = None;
                self.page = None;
                self.page_layout = None;
                self.scroll_y = 0.0;
                self.redraw();
            }
        }
    }

    fn key_input(&mut self, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let key = event.logical_key.as_ref();
        let modifiers = self.modifiers;
        // Command is the shortcut modifier on macOS; Control works everywhere.
        let shortcut =
            modifiers.control_key() || (cfg!(target_os = "macos") && modifiers.super_key());
        if shortcut && let Key::Character(character) = key {
            match character.to_ascii_lowercase().as_str() {
                "+" | "=" => {
                    self.set_page_zoom(self.page_zoom + PAGE_ZOOM_STEP);
                    return;
                }
                "-" | "_" => {
                    self.set_page_zoom(self.page_zoom - PAGE_ZOOM_STEP);
                    return;
                }
                "0" => {
                    self.set_page_zoom(1.0);
                    return;
                }
                "l" => {
                    self.begin_address_edit();
                    return;
                }
                "r" => {
                    self.activate(Control::Reload);
                    return;
                }
                "a" if self.chrome.address_focused => {
                    self.chrome.address_selected = true;
                    self.redraw();
                    return;
                }
                "c" if self.chrome.address_focused => {
                    if let Err(error) = arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(self.address.clone()))
                    {
                        log::warn!("failed to copy the address: {error}");
                    }
                    return;
                }
                "v" if self.chrome.address_focused => {
                    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                        Ok(text) => self.insert_address_text(&text),
                        Err(error) => log::warn!("failed to paste into the address: {error}"),
                    }
                    return;
                }
                _ => {}
            }
        }
        if !self.chrome.address_focused {
            return;
        }
        match key {
            Key::Named(NamedKey::Enter) => self.submit_address(),
            Key::Named(NamedKey::Escape) => self.cancel_address_edit(),
            Key::Named(NamedKey::Backspace) => self.delete_address_text(false),
            Key::Named(NamedKey::Delete) => self.delete_address_text(true),
            // Command+Left/Right jump to the ends on macOS, like Home/End.
            Key::Named(NamedKey::ArrowLeft)
                if cfg!(target_os = "macos") && modifiers.super_key() =>
            {
                self.move_address_caret(CaretMove::Start)
            }
            Key::Named(NamedKey::ArrowRight)
                if cfg!(target_os = "macos") && modifiers.super_key() =>
            {
                self.move_address_caret(CaretMove::End)
            }
            Key::Named(NamedKey::ArrowLeft) => self.move_address_caret(CaretMove::Left),
            Key::Named(NamedKey::ArrowRight) => self.move_address_caret(CaretMove::Right),
            Key::Named(NamedKey::Home | NamedKey::ArrowUp) => {
                self.move_address_caret(CaretMove::Start)
            }
            Key::Named(NamedKey::End | NamedKey::ArrowDown) => {
                self.move_address_caret(CaretMove::End)
            }
            Key::Character(character)
                if !modifiers.control_key() && !modifiers.alt_key() && !modifiers.super_key() =>
            {
                self.insert_address_text(character);
            }
            _ => {}
        }
    }

    /// Caret position clamped to a character boundary of the address.
    fn address_caret(&self) -> usize {
        toolbar::floor_char_boundary(&self.address, self.chrome.address_caret)
    }

    /// Types `text` at the caret, replacing the whole address when selected.
    ///
    /// Control characters such as the newlines of a multi-line paste are
    /// dropped, since an address is a single line.
    fn insert_address_text(&mut self, text: &str) {
        if self.chrome.address_selected {
            self.address.clear();
            self.chrome.address_selected = false;
        }
        let text: String = text
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        let caret = self.address_caret();
        self.address.insert_str(caret, &text);
        self.chrome.address_caret = caret + text.len();
        self.redraw();
    }

    /// Deletes the character after the caret when `forward`, else the one
    /// before it; a selected address is cleared instead.
    fn delete_address_text(&mut self, forward: bool) {
        if self.chrome.address_selected {
            self.address.clear();
            self.chrome.address_selected = false;
            self.chrome.address_caret = 0;
        } else {
            let caret = self.address_caret();
            if forward {
                if caret < self.address.len() {
                    self.address.remove(caret);
                }
                self.chrome.address_caret = caret;
            } else if let Some(previous) = self.address[..caret].chars().next_back() {
                let start = caret - previous.len_utf8();
                self.address.remove(start);
                self.chrome.address_caret = start;
            }
        }
        self.redraw();
    }

    /// Moves the caret, collapsing a selection first. Left and Right collapse
    /// a selection to its start or end without moving further, as text
    /// fields on every platform do.
    fn move_address_caret(&mut self, movement: CaretMove) {
        let caret = self.address_caret();
        let selected = std::mem::take(&mut self.chrome.address_selected);
        self.chrome.address_caret = match movement {
            CaretMove::Start => 0,
            CaretMove::End => self.address.len(),
            CaretMove::Left if selected => 0,
            CaretMove::Right if selected => self.address.len(),
            CaretMove::Left => self.address[..caret]
                .chars()
                .next_back()
                .map_or(caret, |previous| caret - previous.len_utf8()),
            CaretMove::Right => self.address[caret..]
                .chars()
                .next()
                .map_or(caret, |next| caret + next.len_utf8()),
        };
        log::trace!("address caret moved to {}", self.chrome.address_caret);
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
            log::trace!("hover changed: {hovered:?}");
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

    fn dismiss_context_menu(&mut self) {
        if self.context_menu.take().is_some() {
            log::trace!("context menu dismissed");
            // Restore the page underneath the popup, without re-evaluating scripts.
            self.frame_key = None;
            self.redraw();
        }
        self.menu_pressed = None;
    }

    fn close_page_source(&mut self) {
        self.source_close_pressed = false;
        if self.page_source.take().is_some() {
            log::debug!("page source view closed");
            self.frame_key = None;
            self.redraw();
        }
    }

    fn show_page_source(&mut self) {
        if let Some(page) = &self.loaded_page {
            log::debug!("page source view opened ({} bytes)", page.html.len());
            self.page_source = Some(PageSource::new(&page.html));
            self.frame_key = None;
            self.redraw();
        }
    }

    fn scroll_page_source(&mut self, lines: isize) {
        if let (Some(source), Some(window)) = (&mut self.page_source, &self.window) {
            let size = window.inner_size().to_logical::<f64>(window.scale_factor());
            source.scroll(lines, size.width, size.height);
            self.redraw();
        }
    }

    fn scroll_page_source_page(&mut self, direction: isize) {
        if let (Some(source), Some(window)) = (&mut self.page_source, &self.window) {
            let size = window.inner_size().to_logical::<f64>(window.scale_factor());
            source.scroll_page(direction, size.width, size.height);
            self.redraw();
        }
    }

    fn resize_elements(&mut self) {
        if let (Some(panel), Some(window)) = (&mut self.elements, &self.window) {
            let size = window.inner_size().to_logical::<f64>(window.scale_factor());
            panel.resize(size.width, size.height);
        }
    }

    fn show_elements(&mut self) {
        self.elements = Some(ElementsPanel::new(
            self.loaded_page.as_ref().map(|page| page.html.as_str()),
        ));
        self.resize_elements();
        self.chrome.address_focused = false;
        self.page_press = false;
        self.frame_key = None;
        self.redraw();
    }

    /// Opens the dock (if needed) on the Compatibility tab and audits the
    /// loaded page once per dock.
    fn show_compatibility(&mut self) {
        if self.elements.is_none() {
            self.show_elements();
        }
        self.select_dock_tab(DockTab::Compatibility);
    }

    fn select_dock_tab(&mut self, tab: DockTab) {
        let Some(panel) = &mut self.elements else {
            return;
        };
        panel.set_tab(tab);
        if tab == DockTab::Compatibility && *panel.compat.state() == CompatState::Idle {
            self.start_audit();
        }
        self.redraw();
    }

    /// Audits the loaded page off the UI thread; the result arrives as
    /// [`UserEvent::AuditFinished`]. Without an event loop (tests) the audit
    /// runs inline.
    fn start_audit(&mut self) {
        let Some(panel) = &mut self.elements else {
            return;
        };
        let Some(page) = self.loaded_page.clone() else {
            panel
                .compat
                .set_state(CompatState::Unavailable("No loaded page to audit"));
            return;
        };
        panel.compat.set_state(CompatState::Auditing);
        self.audit_id += 1;
        let id = self.audit_id;
        let auditor = Arc::clone(
            self.auditor
                .get_or_insert_with(|| Arc::new(ShellAuditor::new(ShellScriptEngine::new()))),
        );
        let script_error = self.page_events.script_error();
        log::debug!("compatibility audit {id} started for {}", page.url);
        let run = move || {
            Box::new(crate::compat_panel::audit_page(
                &auditor,
                &page,
                script_error.as_deref(),
            ))
        };
        match self.proxy.clone() {
            Some(proxy) => {
                std::thread::spawn(move || {
                    let report = run();
                    let _ = proxy.send_event(UserEvent::AuditFinished { id, report });
                });
            }
            None => {
                let report = run();
                self.audit_finished(id, *report);
            }
        }
    }

    fn audit_finished(&mut self, id: u64, report: devtools::compat::AuditReport) {
        if id != self.audit_id {
            log::debug!("dropping stale compatibility audit {id}");
            return;
        }
        log::debug!(
            "compatibility audit {id} finished: {} findings",
            report.findings.len()
        );
        if let Some(panel) = &mut self.elements {
            panel.compat.set_state(CompatState::Ready(report));
            self.redraw();
        }
    }

    fn close_elements(&mut self) {
        // Any audit still running belongs to the closed dock.
        self.audit_id += 1;
        if self.elements.take().is_some() {
            self.frame_key = None;
            self.redraw();
        }
        self.elements_close_pressed = false;
        self.page_press = false;
    }

    fn key_pressed(&mut self, key: &Key) {
        if *key == Key::Named(NamedKey::Escape) {
            if self.context_menu.is_some() {
                self.dismiss_context_menu();
            } else if self.page_source.is_some() {
                self.close_page_source();
            } else {
                self.close_elements();
            }
            return;
        }
        if self.page_source.is_none()
            && !self.chrome.address_focused
            && let (Some(panel), Key::Named(key)) = (&mut self.elements, key)
        {
            panel.key(key);
            self.redraw();
            return;
        }
        if self.page_source.is_some() {
            match key {
                Key::Named(NamedKey::ArrowDown) => self.scroll_page_source(1),
                Key::Named(NamedKey::ArrowUp) => self.scroll_page_source(-1),
                Key::Named(NamedKey::PageDown) => self.scroll_page_source_page(1),
                Key::Named(NamedKey::PageUp) => self.scroll_page_source_page(-1),
                Key::Named(NamedKey::Home) => self.scroll_page_source(isize::MIN),
                Key::Named(NamedKey::End) => self.scroll_page_source(isize::MAX),
                _ => {}
            }
            return;
        }
        if self.chrome.address_focused {
            return;
        }
        let page = (self.page_viewport_height() - PAGE_SCROLL_OVERLAP).max(ARROW_SCROLL);
        let delta = match key {
            Key::Named(NamedKey::ArrowDown) => ARROW_SCROLL,
            Key::Named(NamedKey::ArrowUp) => -ARROW_SCROLL,
            Key::Named(NamedKey::PageDown) => page,
            Key::Named(NamedKey::PageUp) => -page,
            Key::Named(NamedKey::Space) if self.modifiers.shift_key() => -page,
            Key::Named(NamedKey::Space) => page,
            Key::Named(NamedKey::Home) => f32::NEG_INFINITY,
            Key::Named(NamedKey::End) => f32::INFINITY,
            _ => return,
        };
        self.scroll_page_by(delta);
    }

    /// The page viewport's height in CSS pixels, or zero before the first
    /// layout.
    fn page_viewport_height(&self) -> f32 {
        self.page_layout
            .as_ref()
            .map_or(0.0, |layout| layout.css_size().1 as f32)
    }

    /// Scrolls the page down by `delta` CSS pixels (up when negative),
    /// stopping at the top and bottom of the document, and repaints when
    /// the position changed.
    fn scroll_page_by(&mut self, delta: f32) {
        let max_scroll = self
            .page_layout
            .as_ref()
            .map_or(0.0, crate::content::PageLayout::max_scroll);
        let target = (self.scroll_y + delta).clamp(0.0, max_scroll);
        if target == self.scroll_y {
            return;
        }
        log::trace!("page scrolled to {target:.1} of {max_scroll:.1}");
        self.dismiss_context_menu();
        self.scroll_y = target;
        // Only the page pixmap changes; the layout is reused.
        self.page = None;
        self.redraw();
    }

    /// Changes page zoom and updates the page viewport, render scale, and
    /// script-visible device pixel ratio together.
    fn set_page_zoom(&mut self, zoom: f64) {
        let zoom = zoom.clamp(PAGE_ZOOM_RANGE.0, PAGE_ZOOM_RANGE.1);
        if (zoom - self.page_zoom).abs() < f64::EPSILON {
            return;
        }
        self.page_zoom = zoom;
        self.page = None;
        self.page_layout = None;
        self.scroll_y = 0.0;
        let device_pixel_ratio = self
            .window
            .as_ref()
            .map_or(zoom, |window| window.scale_factor() * zoom);
        self.page_events.set_device_pixel_ratio(device_pixel_ratio);
        self.frame_key = None;
        self.redraw();
    }

    fn open_context_menu(&mut self, width: u32, height: u32, scale: f64) {
        self.dismiss_context_menu();
        if self.page_source.is_some()
            || self
                .elements
                .as_ref()
                .is_some_and(|panel| panel.contains(self.cursor))
        {
            return;
        }
        self.chrome.pressed = None;
        self.last_toolbar_click = None;
        let Some((x, y)) = self.cursor else {
            return;
        };
        let Some(card) = toolbar::page_rect(width, height, scale) else {
            return;
        };
        if x < f64::from(card.x) / scale
            || x >= f64::from(card.x + card.width) / scale
            || y < f64::from(card.y) / scale
            || y >= f64::from(card.y + card.height) / scale
        {
            return;
        }
        log::debug!("context menu opened at ({x:.0}, {y:.0})");
        self.context_menu = Some(ContextMenu::new(
            x,
            y,
            f64::from(width) / scale,
            f64::from(height) / scale,
        ));
        self.set_hovered(None);
        self.frame_key = None;
        self.redraw();
    }

    fn left_mouse_input(&mut self, state: ElementState) {
        if let Some(menu) = self.context_menu {
            match state {
                ElementState::Pressed => {
                    self.menu_pressed = menu.action_at(self.cursor);
                    if !menu.contains(self.cursor) {
                        self.dismiss_context_menu();
                    }
                }
                ElementState::Released => {
                    // Disabled rows and padding have no press action and leave
                    // the popup open. They never fall through to browser controls.
                    if self.menu_pressed.is_none() {
                        return;
                    }
                    let action = self
                        .menu_pressed
                        .filter(|action| Some(*action) == menu.action_at(self.cursor));
                    self.dismiss_context_menu();
                    log::debug!("context menu action: {action:?}");
                    match action {
                        Some(MenuAction::Reload) => self.activate(Control::Reload),
                        Some(MenuAction::PageSource) => self.show_page_source(),
                        Some(MenuAction::Inspect) => self.show_elements(),
                        Some(MenuAction::Compatibility) => self.show_compatibility(),
                        None => {}
                    }
                }
            }
            return;
        }
        if self.page_source.is_some() {
            let close = self
                .page_source
                .as_ref()
                .is_some_and(|source| source.close_at(self.cursor));
            match state {
                ElementState::Pressed => {
                    self.source_close_pressed = close;
                    // Preserve toolbar dismissal, consuming the click.
                    if self.cursor.is_some_and(|(_, y)| y < 60.0) {
                        self.close_page_source();
                    }
                }
                ElementState::Released => {
                    if self.source_close_pressed && close {
                        self.close_page_source();
                    }
                    self.source_close_pressed = false;
                }
            }
            return;
        }
        if let Some(panel) = &self.elements {
            let inside = panel.contains(self.cursor);
            let close = panel.close_at(self.cursor);
            if inside || self.elements_close_pressed {
                let tab = panel.tab_at(self.cursor);
                match state {
                    ElementState::Pressed => {
                        self.elements_close_pressed = close;
                        self.page_press = false;
                        self.chrome.address_focused = false;
                        if let Some(tab) = tab {
                            self.select_dock_tab(tab);
                        }
                    }
                    ElementState::Released => {
                        if self.elements_close_pressed && close {
                            self.close_elements();
                        }
                        self.elements_close_pressed = false;
                        self.page_press = false;
                    }
                }
                return;
            }
        }
        match state {
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
        }
    }

    fn activate(&mut self, control: Control) {
        match control {
            Control::Reload => {
                log::debug!("reload requested");
                log::debug!("toolbar action: reload");
                if let Some(refresh) = &self.refresh {
                    if let Some(task) = refresh.refresh(&self.browser_state) {
                        let id = self.begin_loading();
                        self.reload_completion = Some((id, Box::new(move || task.is_finished())));
                    }
                } else {
                    log::error!("refresh unavailable: initialization failed");
                }
                // Reload keeps the scroll position, clamped to the new layout.
                self.page = None;
                self.page_layout = None;
                self.scripts_evaluated = false;
                self.redraw();
            }
            Control::Address => {
                log::debug!("toolbar action: address");
                self.begin_address_edit()
            }
            // Navigation, tabs, and the menu are not
            // implemented yet; these controls only give visual feedback.
            Control::Back | Control::Forward | Control::NewTab | Control::Menu => {
                log::debug!("toolbar action: {control:?} (not implemented)");
            }
        }
    }

    fn paint(&mut self) {
        if self.window.is_none() || self.surface.is_none() {
            return;
        }
        self.ensure_welcome_loaded();
        let address = self.address.clone();
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let paint_started = Instant::now();
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        let scale = window.scale_factor();
        let page_scale = scale * self.page_zoom;
        let dark = window.theme() == Some(Theme::Dark);
        self.chrome.dark = dark;
        let card = toolbar::page_rect(size.width, size.height, scale);
        let card_size = card.map(|card| (card.width, card.height));
        let mut full = self.page.is_none()
            || self.context_menu.is_some()
            || self.page_source.is_some()
            || self.elements.is_some();
        let page_size_changed = self.page.as_ref().map(|(size, _)| *size) != card_size;
        if page_size_changed {
            // Painting never runs the page script: evaluation joins the JS
            // engine thread, and doing that inside this draw leaves `alert`
            // unable to finish. `about_to_wait` starts it after this frame.
            self.page = card.and_then(|card| {
                let css_size = crate::content::css_viewport(card.width, card.height, page_scale);
                if self.page_layout.as_ref().map(|layout| layout.css_size()) != Some(css_size) {
                    self.page_layout = Some(crate::content::PageLayout::new(
                        self.loaded_page.as_ref().expect("page was loaded"),
                        css_size.0,
                        css_size.1,
                    ));
                }
                let layout = self.page_layout.as_ref()?;
                // A resize or reload can leave less to scroll than before.
                self.scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll());
                crate::content::render_page(
                    layout,
                    card.width,
                    card.height,
                    page_scale,
                    self.scroll_y,
                )
                .map(|page| ((card.width, card.height), page))
            });
            full = true;
            if let Some(layout) = &self.page_layout {
                let (width, height) = layout.css_size();
                self.page_events
                    .resize(webapis::ViewportSize { width, height });
            }
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
                &address,
                self.page.as_ref().map(|(_, page)| page),
                self.font.as_ref(),
            )
        } else {
            toolbar::compose_controls(
                size.width,
                size.height,
                scale,
                &self.chrome,
                &address,
                self.font.as_ref(),
            )
        };
        let Some(mut composed) = composed else {
            return;
        };
        if let Some(panel) = &mut self.elements {
            panel.draw(
                &mut composed,
                scale,
                self.chrome.dark,
                self.cursor,
                self.font.as_ref(),
                self.source_font.as_ref(),
            );
        }
        if let Some(source) = &mut self.page_source {
            source.draw(
                &mut composed,
                scale,
                self.source_font.as_ref().or(self.font.as_ref()),
            );
        }
        if let Some(menu) = self.context_menu {
            menu.draw(
                &mut composed,
                scale,
                self.chrome.dark,
                menu.action_at(self.cursor),
                self.font.as_ref(),
            );
        }
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
        log::trace!(
            "frame presented: {}x{} full={full} in {:?}",
            size.width,
            size.height,
            paint_started.elapsed()
        );
    }

    fn cursor_moved(&mut self, x: f64, y: f64) {
        let Some(window) = &self.window else {
            return;
        };
        let scale = window.scale_factor();
        let width = window.inner_size().to_logical::<f64>(scale).width;
        // Leaving the inspector must repaint it once to clear row and
        // close-button hover feedback.
        let left_elements = self
            .elements
            .as_ref()
            .is_some_and(|panel| panel.contains(self.cursor) && !panel.contains(Some((x, y))));
        self.cursor = Some((x, y));
        log::trace!("cursor moved: ({x:.0}, {y:.0})");
        if left_elements {
            self.redraw();
        }
        if self.context_menu.is_some()
            || self.page_source.is_some()
            || self
                .elements
                .as_ref()
                .is_some_and(|panel| panel.contains(self.cursor))
        {
            self.set_hovered(None);
            self.redraw();
            return;
        }
        let inset = self.platform.window_controls_inset;
        self.set_hovered(toolbar::hit_test(x, y, width, inset));
    }

    fn pressed(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        if let Some(control) = self.chrome.hovered {
            if control != Control::Address {
                self.chrome.address_focused = false;
            }
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
                if let Err(error) = window.drag_window() {
                    log::debug!("window drag could not start: {error}");
                }
            }
        } else if self
            .cursor
            .is_some_and(|(x, y)| self.pointer_is_on_page(x, y))
        {
            self.page_press = true;
        }
    }

    /// Returns the page node under a logical-pixel window point, matching the
    /// geometry and scroll position `paint` used to render the page card.
    fn page_node_at(&self, x: f64, y: f64) -> Option<html::NodeID> {
        let window = self.window.as_ref()?;
        let layout = self.page_layout.as_ref()?;
        let size = window.inner_size();
        let scale = window.scale_factor();
        let card = toolbar::page_rect(size.width, size.height, scale)?;
        let page_scale = scale * self.page_zoom;
        layout.hit_test(
            ((x * scale - card.x as f64) / page_scale) as f32,
            ((y * scale - card.y as f64) / page_scale) as f32,
            self.scroll_y,
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

impl ApplicationHandler<UserEvent> for App {
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
                    .unwrap_or_else(|error| {
                        log::error!("failed to create window: {error}");
                        panic!("failed to create window: {error}")
                    }),
            );
            log::info!(
                "window created: {}x{} physical pixels, scale factor {}",
                window.inner_size().width,
                window.inner_size().height,
                window.scale_factor()
            );
            let context = softbuffer::Context::new(window.clone()).unwrap_or_else(|error| {
                log::error!("failed to create drawing context: {error}");
                panic!("failed to create drawing context: {error}")
            });
            self.surface = Some(
                softbuffer::Surface::new(&context, window.clone()).unwrap_or_else(|error| {
                    log::error!("failed to create window surface: {error}");
                    panic!("failed to create window surface: {error}")
                }),
            );
            self.font = toolbar::load_ui_font((self.platform.ui_font_files)());
            self.source_font = crate::page_source::load_font();
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
            WindowEvent::CloseRequested => {
                log::info!("window closed, exiting");
                event_loop.exit()
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                log::trace!("key event: {:?}", event.state);
                // Address editing and the context menu share one key event.
                if event.state == ElementState::Pressed {
                    self.key_pressed(&event.logical_key);
                }
                self.key_input(event);
            }
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_) => {
                if log::log_enabled!(log::Level::Debug) {
                    match &event {
                        WindowEvent::Resized(size) => {
                            log::trace!("resized: {}x{}", size.width, size.height)
                        }
                        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                            log::trace!("scale factor changed: {scale_factor}")
                        }
                        WindowEvent::ThemeChanged(theme) => {
                            log::debug!("theme changed: {theme:?}")
                        }
                        _ => {}
                    }
                }
                if matches!(event, WindowEvent::ScaleFactorChanged { .. })
                    && let Some(window) = &self.window
                {
                    self.page_events
                        .set_device_pixel_ratio(window.scale_factor() * self.page_zoom);
                }
                self.dismiss_context_menu();
                self.resize_elements();
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
                if self.context_menu.is_some() {
                    self.redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.left_mouse_input(state),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    self.open_context_menu(size.width, size.height, window.scale_factor());
                }
            }
            WindowEvent::Focused(false) => self.dismiss_context_menu(),
            WindowEvent::MouseWheel { delta, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map_or(1.0, |window| window.scale_factor());
                // Panels scroll by text rows; the page scrolls by CSS pixels.
                let (rows, page_pixels) = match delta {
                    MouseScrollDelta::LineDelta(_, y) => {
                        (-f64::from(y) * 3.0, -y * 3.0 * WHEEL_LINE_SCROLL)
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        log::trace!("scroll: {:.1} px", position.y);
                        (-position.y / (18.0 * scale), (-position.y / scale) as f32)
                    }
                };
                if let (Some(source), Some(window)) = (&mut self.page_source, &self.window) {
                    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                    source.wheel(rows, size.width, size.height);
                    self.redraw();
                } else if let Some(panel) = &mut self.elements
                    && panel.contains(self.cursor)
                {
                    panel.scroll(rows);
                    self.redraw();
                } else if self
                    .cursor
                    .is_some_and(|(x, y)| self.pointer_is_on_page(x, y))
                {
                    self.scroll_page_by(page_pixels);
                }
            }
            WindowEvent::RedrawRequested => self.paint(),
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::AlertPending => {
                if let Some(window) = &self.window {
                    let window = Rc::clone(window);
                    self.alerts.present(&window);
                }
            }
            UserEvent::Navigated {
                id,
                navigation,
                behavior,
            } => self.navigation_finished(id, navigation, behavior),
            UserEvent::LocationNavigate { url, behavior } => {
                self.start_navigation_with_behavior(url.as_str(), behavior)
            }
            UserEvent::AuditFinished { id, report } => self.audit_finished(id, *report),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_reload();
        event_loop.set_control_flow(if self.reload_completion.is_some() {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(40))
        } else {
            ControlFlow::Wait
        });

        // `frame_key` is set only after a frame has been presented, so the
        // window is on screen before the page script can request an alert.
        if self.scripts_evaluated || self.frame_key.is_none() {
            return;
        }
        let Some(page) = self.loaded_page.clone() else {
            return;
        };
        self.scripts_evaluated = true;
        log::debug!("evaluating page scripts for {}", page.url);
        // Leave this thread in the event loop. The script runs on the page
        // session thread and asks the loop to present an alert; joining that
        // thread here would deadlock. The session stays alive so a later
        // click can run element and `document` listeners.
        let alerts = Arc::clone(&self.alerts) as Arc<dyn AlertSink>;
        let device_pixel_ratio = self.window.as_ref().map_or(self.page_zoom, |window| {
            window.scale_factor() * self.page_zoom
        });
        let viewport = self.page_layout.as_ref().map(|layout| {
            let (width, height) = layout.css_size();
            webapis::ViewportSize { width, height }
        });
        let proxy = self.proxy.clone();
        let navigate = Arc::new(move |url, behavior| {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::LocationNavigate { url, behavior });
            }
        });
        self.page_events.load_with_viewport(
            &page,
            alerts,
            self.runtime.clone(),
            device_pixel_ratio,
            viewport,
            navigate,
        );
    }
}

/// Opens the browser window described by `platform` and runs it until the
/// user closes it.
///
/// Reads the logging options (`--log-level`, `-v`, `-q`, `--log-file`,
/// `--no-log-file`; see [`common::logging::USAGE`]) from the process's
/// command line before anything else, and prints usage and exits for `-h` /
/// `--help`. Unless told otherwise, the log is also written to
/// [`browser::default_log_file`]. Other arguments are reported as ignored;
/// the shell takes no others yet.
pub fn run(platform: Platform) -> Result<(), EventLoopError> {
    let default_log = browser::default_log_file().ok();
    let command_line =
        common::logging::init_from_command_line_with_default_file(default_log.clone());
    if command_line.help_requested {
        println!(
            "Usage: {} [options]\n\n{}\n\n{}",
            env!("CARGO_PKG_NAME"),
            common::logging::USAGE,
            browser::log_file_help(default_log.as_deref())
        );
        return Ok(());
    }
    for argument in &command_line.remaining {
        log::warn!("ignoring unrecognized argument `{argument}`");
    }
    log::info!(
        "starting BYUI Browser {} on {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    crate::content::load_page_font((platform.page_font_files)());
    log::debug!("creating event loop");
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(platform);
    app.alerts.set_proxy(event_loop.create_proxy());
    app.proxy = Some(event_loop.create_proxy());
    let result = event_loop.run_app(&mut app);
    log::info!("event loop exited");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::with_runtime(
            Platform {
                configure_window: |attributes| attributes,
                ui_font_files: Vec::new,
                page_font_files: Vec::new,
                window_controls_inset: 0.0,
            },
            browser::BrowserRuntime::in_memory(),
        )
    }

    #[test]
    fn loading_completions_only_clear_the_latest_operation() {
        let mut app = app();
        let old = app.begin_loading();
        let current = app.begin_loading();
        app.finish_loading(old);
        assert!(app.chrome.loading);
        app.finish_loading(current);
        assert!(!app.chrome.loading);
    }

    #[test]
    fn navigation_success_failure_and_stale_completion_update_loading_safely() {
        let mut app = app();
        for failed in [false, true] {
            app.navigation_id += 1;
            let id = app.navigation_id;
            let loading = app.begin_loading();
            app.navigation_loading = Some((id, loading));
            app.navigation_finished(
                id - 1,
                navigation("old", Ok(browser::welcome_page())),
                NavigationBehavior::Auto,
            );
            assert!(app.chrome.loading);
            let result = if failed {
                Err(browser::LoadError::Status {
                    status: 500,
                    status_text: "Error".into(),
                })
            } else {
                Ok(browser::welcome_page())
            };
            app.navigation_finished(
                id,
                navigation("byui://welcome", result),
                NavigationBehavior::Auto,
            );
            assert!(!app.chrome.loading);
        }
    }

    #[test]
    fn reload_completion_preserves_newer_navigation_and_history() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let history = app.browser_state.clone();
        let id = app.begin_loading();
        let done = Rc::new(std::cell::Cell::new(false));
        let probe = done.clone();
        app.reload_completion = Some((id, Box::new(move || probe.get())));
        app.poll_reload();
        assert!(app.chrome.loading);
        let newer = app.begin_loading();
        done.set(true);
        app.poll_reload();
        assert!(app.chrome.loading);
        assert!(app.reload_completion.is_none());
        app.finish_loading(newer);
        assert!(!app.chrome.loading);
        assert_eq!(app.browser_state, history);
    }

    #[test]
    fn real_reload_success_and_failure_clear_loading_without_changing_history() {
        let mut app = app();
        for url in ["byui://welcome", "unsupported://invalid"] {
            app.browser_state.record_visit(url);
            let history = app.browser_state.clone();
            let page = app.loaded_page.clone();
            app.activate(Control::Reload);
            assert!(app.chrome.loading);
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.chrome.loading {
                assert!(
                    Instant::now() < deadline,
                    "Reload completion was not observed"
                );
                app.poll_reload();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.reload_completion.is_none());
            assert_eq!(app.browser_state, history);
            assert_eq!(app.loaded_page, page);
        }
    }

    #[test]
    fn navigation_completion_does_not_clear_a_newer_reload_indicator() {
        let mut app = app();
        let navigation_loading = app.begin_loading();
        app.navigation_id = 1;
        app.navigation_loading = Some((1, navigation_loading));
        let reload_loading = app.begin_loading();
        app.navigation_finished(
            1,
            navigation("byui://welcome", Ok(browser::welcome_page())),
            NavigationBehavior::Auto,
        );
        assert!(app.chrome.loading);
        app.finish_loading(reload_loading);
        assert!(!app.chrome.loading);
    }

    #[test]
    fn missing_loader_or_reload_url_never_leaves_loading_active() {
        let mut app = app();
        app.loader = None;
        app.start_navigation("https://example.com");
        assert!(!app.chrome.loading);
        app.activate(Control::Reload);
        assert!(!app.chrome.loading);
        app.begin_loading();
        app.address = "byui://welcome".into();
        app.submit_address();
        assert!(!app.chrome.loading);
    }

    #[test]
    fn official_page_loading_closes_the_previous_elements_snapshot() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.show_elements();
        assert!(app.elements.is_some());
        app.page_loaded("byui://welcome", browser::welcome_page());
        assert!(app.elements.is_none());
        assert_eq!(app.browser_state.current_url(), Some("byui://welcome"));
        assert!(app.loaded_page.is_some());
    }

    #[test]
    fn inspect_and_source_are_separate_actions_with_independent_interfaces() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let before = app.browser_state.clone();
        let page = app.loaded_page.clone();
        app.refresh = None;
        app.scripts_evaluated = true;
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        app.cursor = Some(app.context_menu.unwrap().item_center(4));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert!(app.elements.is_some());
        assert!(app.page_source.is_none());
        app.elements.as_mut().unwrap().resize(800.0, 600.0);
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        app.cursor = Some(app.context_menu.unwrap().item_center(3));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert!(app.page_source.is_some());
        assert!(app.elements.is_some());
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.page_source.is_none());
        assert!(
            app.elements.is_some(),
            "closing source must not close inspector"
        );
        app.show_page_source();
        app.close_elements();
        assert!(
            app.page_source.is_some(),
            "closing inspector must not close source"
        );
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.page_source.is_none());
        assert_eq!(app.browser_state, before);
        assert_eq!(app.loaded_page, page);
        assert!(app.scripts_evaluated);
    }

    #[test]
    fn audit_compatibility_opens_the_dock_on_its_tab_with_a_report() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.scripts_evaluated = true;
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        app.cursor = Some(app.context_menu.unwrap().item_center(5));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        let panel = app.elements.as_ref().expect("the dock opens");
        assert_eq!(panel.tab(), DockTab::Compatibility);
        // Without an event loop the audit runs inline.
        let CompatState::Ready(report) = panel.compat.state() else {
            panic!("audit should have finished: {:?}", panel.compat.state());
        };
        assert!(report.features_checked > 0);
        assert!(app.page_source.is_none());
    }

    #[test]
    fn dock_tabs_switch_on_click_and_audit_only_once() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.show_elements();
        let panel = app.elements.as_mut().unwrap();
        panel.resize(800.0, 600.0);
        assert_eq!(panel.tab(), DockTab::Elements);
        assert_eq!(*panel.compat.state(), CompatState::Idle);
        // The second header tab starts after the 84px Elements tab.
        app.cursor = Some((8.0 + 6.0 + 84.0 + 40.0, 340.0));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert!(app.elements.is_some(), "tab clicks never close the dock");
        assert_eq!(app.elements.as_ref().unwrap().tab(), DockTab::Compatibility);
        let first = app.audit_id;
        app.cursor = Some((8.0 + 6.0 + 20.0, 340.0));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert_eq!(app.elements.as_ref().unwrap().tab(), DockTab::Elements);
        app.select_dock_tab(DockTab::Compatibility);
        assert_eq!(app.audit_id, first, "a finished audit is reused");
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.elements.is_none());
    }

    #[test]
    fn stale_audit_results_are_dropped() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.show_compatibility();
        let id = app.audit_id;
        let report = devtools::compat::AuditReport::default();
        app.close_elements();
        app.show_elements();
        app.audit_finished(id, report);
        assert_eq!(
            *app.elements.as_ref().unwrap().compat.state(),
            CompatState::Idle
        );
    }

    #[test]
    fn compatibility_without_a_page_explains_itself() {
        let mut app = app();
        app.show_compatibility();
        assert_eq!(
            *app.elements.as_ref().unwrap().compat.state(),
            CompatState::Unavailable("No loaded page to audit")
        );
    }

    #[test]
    fn inspector_close_button_and_escape_clear_panel_without_page_clicks() {
        let mut app = app();
        for escape in [false, true] {
            app.show_elements();
            app.elements.as_mut().unwrap().resize(800.0, 600.0);
            app.frame_key = Some((800, 600, 1.0_f64.to_bits(), false));
            app.scripts_evaluated = true;
            if escape {
                app.key_pressed(&Key::Named(NamedKey::Escape));
            } else {
                app.cursor = Some((770.0, 340.0));
                app.left_mouse_input(ElementState::Pressed);
                assert!(app.elements.is_some());
                app.left_mouse_input(ElementState::Released);
            }
            assert!(app.elements.is_none());
            assert!(app.frame_key.is_none());
            assert!(!app.page_press);
            assert!(app.scripts_evaluated);
        }
    }

    #[test]
    fn source_and_inspector_handle_unavailable_html_without_crashing() {
        let mut app = app();
        app.show_page_source();
        assert!(app.page_source.is_none());
        app.show_elements();
        assert!(app.elements.is_some());
        assert!(app.loaded_page.is_none());
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.elements.is_none());
    }

    #[test]
    fn view_page_source_shows_loaded_html_without_fetching_executing_or_changing_history() {
        let mut app = app();
        let page = browser::Page {
            url: String::new(),
            html: "<h1>Already loaded</h1>\n<script>not executed</script>".into(),
            stylesheets: Vec::new(),
            scripts: Vec::new(),
        };
        app.page_loaded("byui://source-fixture", page.clone());
        let before = app.browser_state.clone();
        // Inspect must work even with the Refresh/network client unavailable.
        app.refresh = None;
        app.scripts_evaluated = true;
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        app.cursor = Some(app.context_menu.unwrap().item_center(3));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert!(app.context_menu.is_none());
        assert_eq!(app.page_source.as_ref().unwrap().html, page.html);
        assert!(app.scripts_evaluated);
        assert_eq!(app.browser_state, before);
        assert_eq!(app.loaded_page, Some(page));
        app.frame_key = Some((800, 600, 1.0_f64.to_bits(), false));
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.page_source.is_none());
        assert!(app.frame_key.is_none());
        assert!(app.scripts_evaluated);
        assert_eq!(app.browser_state, before);
    }

    #[test]
    fn source_close_button_requires_matching_press_and_release_and_preserves_state() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let before = app.browser_state.clone();
        app.show_page_source();
        app.page_source.as_mut().unwrap().resize(800.0, 600.0);
        app.scripts_evaluated = true;
        app.frame_key = Some((800, 600, 1.0_f64.to_bits(), false));
        app.cursor = Some((730.0, 80.0));
        app.left_mouse_input(ElementState::Pressed);
        app.cursor = Some((400.0, 200.0));
        app.left_mouse_input(ElementState::Released);
        assert!(app.page_source.is_some());
        app.cursor = Some((730.0, 80.0));
        app.left_mouse_input(ElementState::Pressed);
        assert!(app.page_source.is_some());
        app.left_mouse_input(ElementState::Released);
        assert!(app.page_source.is_none());
        assert!(app.frame_key.is_none());
        assert!(app.scripts_evaluated);
        assert_eq!(app.browser_state, before);
    }

    #[test]
    fn source_viewer_handles_missing_document_and_toolbar_dismissal() {
        let mut app = app();
        app.show_page_source();
        assert!(app.page_source.is_none());
        app.ensure_welcome_loaded();
        app.show_page_source();
        app.scripts_evaluated = true;
        app.cursor = Some((162.0, 26.0));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert!(app.page_source.is_none());
        assert!(app.scripts_evaluated);
    }

    #[test]
    fn disabled_back_forward_do_nothing_even_after_multiple_recorded_visits() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.browser_state.record_visit("https://a.example/");
        app.browser_state.record_visit("https://b.example/");
        let before = app.browser_state.clone();
        let page = app.loaded_page.clone();
        app.scripts_evaluated = true;
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        for row in [0, 1] {
            app.cursor = Some(app.context_menu.unwrap().item_center(row));
            app.left_mouse_input(ElementState::Pressed);
            app.left_mouse_input(ElementState::Released);
            assert!(app.context_menu.is_some());
            assert_eq!(app.browser_state, before);
            assert_eq!(app.loaded_page, page);
            assert!(app.scripts_evaluated);
            assert!(app.page_source.is_none());
        }
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.context_menu.is_none());
        assert_eq!(app.browser_state, before);
    }

    #[test]
    fn context_menu_opens_only_over_page_content_at_each_scale() {
        let mut app = app();
        for scale in [1.0, 2.0] {
            for (point, opens) in [
                ((100.0, 100.0), true),
                ((20.0, 20.0), false),
                ((1.0, 100.0), false),
            ] {
                app.cursor = Some(point);
                app.open_context_menu((800.0 * scale) as u32, (600.0 * scale) as u32, scale);
                assert_eq!(app.context_menu.is_some(), opens);
                assert!(app.browser_state.visited_urls().is_empty());
            }
        }
    }

    #[test]
    fn context_reload_uses_existing_action_and_preserves_history() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let before = app.browser_state.clone();
        let document = app.loaded_page.clone();
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        app.cursor = Some(app.context_menu.unwrap().item_center(2));
        app.scripts_evaluated = true;
        app.left_mouse_input(ElementState::Pressed);
        assert!(app.context_menu.is_some());
        assert!(app.scripts_evaluated);
        app.left_mouse_input(ElementState::Released);
        assert!(app.context_menu.is_none());
        assert!(!app.scripts_evaluated);
        assert!(app.page.is_none());
        assert_eq!(app.loaded_page, document);
        assert_eq!(app.browser_state, before);
    }

    #[test]
    fn outside_click_and_escape_clear_popup_without_reloading_or_recording_visits() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let before = app.browser_state.clone();
        for escape in [false, true] {
            app.cursor = Some((100.0, 100.0));
            app.open_context_menu(800, 600, 1.0);
            // Simulate a frame presented with the menu on it.
            app.frame_key = Some((800, 600, 1.0_f64.to_bits(), false));
            app.scripts_evaluated = true;
            if escape {
                app.key_pressed(&Key::Named(NamedKey::Escape));
            } else {
                app.cursor = Some((50.0, 26.0));
                app.left_mouse_input(ElementState::Pressed);
                app.left_mouse_input(ElementState::Released);
            }
            assert!(app.context_menu.is_none());
            assert!(
                app.frame_key.is_none(),
                "dismissal must restore the whole frame"
            );
            assert!(app.scripts_evaluated, "dismissal must not run Reload");
            assert_eq!(app.browser_state, before);
        }
    }

    #[test]
    fn releasing_outside_a_pressed_menu_item_cancels_reload() {
        let mut app = app();
        app.cursor = Some((100.0, 100.0));
        app.open_context_menu(800, 600, 1.0);
        app.cursor = Some(app.context_menu.unwrap().item_center(2));
        app.left_mouse_input(ElementState::Pressed);
        app.cursor = Some((300.0, 300.0));
        app.scripts_evaluated = true;
        app.left_mouse_input(ElementState::Released);
        assert!(app.context_menu.is_none());
        assert!(app.scripts_evaluated);
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
        for (width, height, scale) in [(0, 0, 1.0), (160, 120, 1.0), (320, 240, 2.0)] {
            app.ensure_welcome_loaded();
            let layout = crate::content::PageLayout::for_device(
                app.loaded_page.as_ref().unwrap(),
                width,
                height,
                scale,
            );
            let rendered = crate::content::render_page(&layout, width, height, scale, app.scroll_y);
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
    fn keys_scroll_the_page_within_the_document_and_navigation_resets_it() {
        let mut app = app();
        let page = browser::Page {
            url: String::from("byui://long"),
            html: "<body><div></div></body>".into(),
            stylesheets: vec!["body { margin: 0 } div { height: 1000px }".into()],
            scripts: Vec::new(),
        };
        app.page_loaded("byui://long", page.clone());
        app.page_layout = Some(crate::content::PageLayout::new(&page, 160, 200));
        let max_scroll = 1000.0 - 200.0;

        app.key_pressed(&Key::Named(NamedKey::ArrowUp));
        assert_eq!(app.scroll_y, 0.0, "the top is a hard stop");
        app.key_pressed(&Key::Named(NamedKey::ArrowDown));
        assert_eq!(app.scroll_y, ARROW_SCROLL);
        app.key_pressed(&Key::Named(NamedKey::PageDown));
        assert_eq!(app.scroll_y, ARROW_SCROLL + 200.0 - PAGE_SCROLL_OVERLAP);
        app.key_pressed(&Key::Named(NamedKey::End));
        assert_eq!(app.scroll_y, max_scroll);
        app.key_pressed(&Key::Named(NamedKey::Space));
        assert_eq!(app.scroll_y, max_scroll, "the bottom is a hard stop");
        app.key_pressed(&Key::Named(NamedKey::Home));
        assert_eq!(app.scroll_y, 0.0);

        // Typing in the address field must not scroll the page.
        app.key_pressed(&Key::Named(NamedKey::End));
        app.begin_address_edit();
        app.key_pressed(&Key::Named(NamedKey::Home));
        assert_eq!(app.scroll_y, max_scroll);
        app.cancel_address_edit();

        app.page_loaded("byui://long", page);
        assert_eq!(app.scroll_y, 0.0);
        assert!(app.page_layout.is_none());
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

    #[test]
    fn address_edit_can_be_cancelled_without_changing_the_page() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.begin_address_edit();
        assert!(app.chrome.address_selected);
        app.address.push_str("-draft");

        app.cancel_address_edit();

        assert_eq!(app.address, "byui://welcome");
        assert!(!app.chrome.address_selected);
        assert!(!app.chrome.address_focused);
        assert_eq!(app.browser_state.visited_urls(), &["byui://welcome"]);
    }

    #[test]
    fn pasted_text_replaces_a_selected_address_on_one_line() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.begin_address_edit();

        app.insert_address_text("example.com/\r\npath\t");
        assert_eq!(app.address, "example.com/path");
        assert!(!app.chrome.address_selected);

        app.insert_address_text("?q=1");
        assert_eq!(app.address, "example.com/path?q=1");
    }

    #[test]
    fn arrow_keys_collapse_the_selection_and_move_the_caret() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.begin_address_edit();

        app.move_address_caret(CaretMove::Left);
        assert!(!app.chrome.address_selected);
        assert_eq!(app.chrome.address_caret, 0);

        app.move_address_caret(CaretMove::Right);
        app.move_address_caret(CaretMove::Right);
        app.insert_address_text("X");
        assert_eq!(app.address, "byXui://welcome");
        assert_eq!(app.chrome.address_caret, 3);

        app.delete_address_text(false);
        app.delete_address_text(true);
        assert_eq!(app.address, "byi://welcome");
        assert_eq!(app.chrome.address_caret, 2);

        app.move_address_caret(CaretMove::End);
        app.move_address_caret(CaretMove::Right);
        assert_eq!(app.chrome.address_caret, app.address.len());
    }

    #[test]
    fn caret_steps_over_multi_byte_characters() {
        let mut app = app();
        app.address = "aé".to_owned();
        app.begin_address_edit();
        app.move_address_caret(CaretMove::Right);
        app.move_address_caret(CaretMove::Left);
        assert_eq!(app.chrome.address_caret, 1);
        app.move_address_caret(CaretMove::Right);
        app.delete_address_text(false);
        assert_eq!(app.address, "a");
    }

    #[test]
    fn supported_address_submission_uses_the_existing_loader_boundary() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.begin_address_edit();
        app.address = "byui://welcome".to_owned();

        app.submit_address();

        assert_eq!(app.address, "byui://welcome");
        assert!(!app.chrome.address_focused);
        assert_eq!(
            app.browser_state.visited_urls(),
            &["byui://welcome", "byui://welcome"]
        );
    }

    fn navigation(
        address: &str,
        result: Result<browser::Page, browser::LoadError>,
    ) -> browser::Navigation {
        browser::Navigation {
            address: address.to_owned(),
            result,
        }
    }

    #[test]
    fn other_addresses_load_in_the_background_and_keep_the_current_page() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let original_page = app.loaded_page.clone();
        let before = app.navigation_id;
        app.begin_address_edit();
        app.address = " example.com ".to_owned();

        app.submit_address();

        // No loader thread result has arrived, so nothing is replaced yet.
        assert_eq!(app.navigation_id, before + 1);
        assert_eq!(app.address, "example.com");
        assert!(!app.chrome.address_focused);
        assert_eq!(app.loaded_page, original_page);
        assert_eq!(app.browser_state.visited_urls(), &["byui://welcome"]);
    }

    #[test]
    fn finished_navigation_replaces_the_page_and_records_a_visit() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.address = "example.com".to_owned();
        app.submit_address();
        let page = browser::Page {
            url: String::new(),
            html: "<p>Loaded</p>".into(),
            stylesheets: Vec::new(),
            scripts: Vec::new(),
        };

        app.navigation_finished(
            app.navigation_id,
            navigation("https://example.com/", Ok(page.clone())),
            NavigationBehavior::Auto,
        );

        assert_eq!(app.loaded_page, Some(page));
        assert_eq!(app.address, "https://example.com/");
        assert_eq!(
            app.browser_state.visited_urls(),
            &["byui://welcome", "https://example.com/"]
        );
    }

    #[test]
    fn failed_navigation_shows_an_error_page_and_keeps_the_typed_address() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.address = "example.com/gone".to_owned();
        app.submit_address();
        let error = browser::LoadError::Status {
            status: 404,
            status_text: "Not Found".into(),
        };

        app.navigation_finished(
            app.navigation_id,
            navigation("https://example.com/gone", Err(error)),
            NavigationBehavior::Auto,
        );

        let page = app.loaded_page.as_ref().unwrap();
        assert!(page.html.contains("https://example.com/gone"));
        assert!(page.html.contains("404"));
        assert_eq!(app.address, "example.com/gone");
        assert_eq!(app.browser_state.visited_urls(), &["byui://welcome"]);
    }

    #[test]
    fn stale_navigation_results_are_ignored() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let original_page = app.loaded_page.clone();
        app.address = "example.com".to_owned();
        app.submit_address();
        let stale = app.navigation_id;
        app.address = "example.org".to_owned();
        app.submit_address();
        let page = browser::Page {
            url: String::new(),
            html: "<p>Old</p>".into(),
            stylesheets: Vec::new(),
            scripts: Vec::new(),
        };

        app.navigation_finished(
            stale,
            navigation("https://example.com/", Ok(page)),
            NavigationBehavior::Auto,
        );

        assert_eq!(app.loaded_page, original_page);
        assert_eq!(app.browser_state.visited_urls(), &["byui://welcome"]);
    }
}
