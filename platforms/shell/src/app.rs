//! The winit application shared by the desktop shells: window creation,
//! pointer handling for the painted chrome, and presenting frames through
//! `softbuffer`.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ab_glyph::FontVec;
use net::{NetworkEvent, NetworkPhase, Url};
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
use crate::find_in_page::{Action as FindAction, FindInPage};
use crate::page_source::PageSource;
use crate::toolbar::{self, ChromeState, Control};
use devtools::performance::{EventCategory, PerformanceLog, TimelineEvent};

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

/// Time between CSS animation frames: about 60 frames per second. Frames
/// that take longer to style, lay out, and paint simply arrive later.
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

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

/// A press in the address field that has not been released yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AddressPress {
    /// Whether the field already had keyboard focus before the press. A
    /// click without a drag on an unfocused field selects the whole address,
    /// as in other browsers; on a focused field it places the caret.
    was_focused: bool,
    /// Whether the pointer has moved the caret away from where it went down.
    dragged: bool,
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
    /// Finishes setting up the main browser window once it exists, for
    /// changes the window attributes cannot express. macOS uses it to give
    /// the titlebar the toolbar's height so the traffic lights sit centered
    /// beside the toolbar controls. Runs once, before the first redraw.
    pub window_created: fn(&Window),
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
    devtools_window: Option<Rc<Window>>,
    devtools_surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    devtools_panel: Option<ElementsPanel>,
    devtools_cursor: Option<(f64, f64)>,
    network_log: devtools::network::NetworkLog,
    console: Arc<Mutex<devtools::Console>>,
    /// Timeline events for the Performance tab. Shared with the page session
    /// thread, which records script work.
    performance: Arc<Mutex<PerformanceLog>>,
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
    /// When `loaded_page`'s animation timeline started: its first layout.
    /// `None` until then; cleared when another page is loaded. Rebuilding the
    /// layout (on resize, zoom, or find) keeps the timeline running.
    animation_clock: Option<Instant>,
    /// When the next CSS animation frame is due, while animations run.
    next_animation_frame: Option<Instant>,
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
    /// The pointer went down in the address field and has not been released
    /// yet; dragging extends the address selection.
    address_press: Option<AddressPress>,
    scripts_evaluated: bool,
    find: Option<FindInPage>,
    find_pressed: Option<FindAction>,
    find_reveal: bool,
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
    /// Previous history state while a Back/Forward load is pending.
    history_rollback: Option<chrome::BrowserState>,
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
    /// Opens or focuses the native DevTools window.
    OpenDevtools,
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
            devtools_window: None,
            devtools_surface: None,
            devtools_panel: None,
            devtools_cursor: None,
            network_log: devtools::network::NetworkLog::new(),
            console: Arc::new(Mutex::new(devtools::Console::new())),
            performance: Arc::new(Mutex::new(PerformanceLog::new())),
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
            animation_clock: None,
            next_animation_frame: None,
            scroll_y: 0.0,
            page_zoom: 1.0,
            frame: Vec::new(),
            frame_key: None,
            last_toolbar_click: None,
            runtime,
            alerts: Arc::new(PageAlerts::new()),
            page_events: crate::content::PageEvents::new(),
            page_press: false,
            address_press: None,
            scripts_evaluated: false,
            find: None,
            find_pressed: None,
            find_reveal: false,
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
            history_rollback: None,
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

    fn open_find(&mut self) {
        self.dismiss_context_menu();
        self.close_page_source();
        self.chrome.address_focused = false;
        let find = self.find.get_or_insert_with(FindInPage::default);
        find.focus();
        self.chrome.find_open = true;
        self.find_reveal = true;
        self.page = None;
        self.page_layout = None;
        self.frame_key = None;
        self.redraw();
    }

    fn close_find(&mut self) {
        self.find = None;
        self.find_pressed = None;
        self.chrome.find_open = false;
        self.find_reveal = false;
        self.page = None;
        self.page_layout = None;
        self.frame_key = None;
        self.redraw();
    }

    fn find_step(&mut self, previous: bool) {
        if let Some(find) = &mut self.find {
            find.step(previous);
            find.focused = true;
            self.chrome.address_focused = false;
        }
        self.find_reveal = true;
        self.page = None;
        self.redraw();
    }

    /// Consumes Find input before either address editing or page shortcuts.
    fn find_key(&mut self, key: &Key) -> bool {
        let shortcut = self.modifiers.control_key()
            || (cfg!(target_os = "macos") && self.modifiers.super_key());
        if shortcut && matches!(key,Key::Character(c) if c.eq_ignore_ascii_case("f")) {
            self.open_find();
            return true;
        }
        if self.page_source.is_some() || self.context_menu.is_some() {
            return false;
        }
        if self.elements.is_some()
            && self.find.as_ref().is_some_and(|f| !f.focused)
            && *key == Key::Named(NamedKey::Escape)
        {
            return false;
        }
        if self.find.is_some() && *key == Key::Named(NamedKey::Escape) {
            self.close_find();
            return true;
        }
        let Some(find) = &mut self.find else {
            return false;
        };
        if !find.focused {
            return false;
        }
        if shortcut {
            match key {
                Key::Character(c) if c.eq_ignore_ascii_case("a") => find.select_all(),
                Key::Character(c) if c.eq_ignore_ascii_case("v") => {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(text) => find.insert(&text),
                        Err(error) => log::warn!("Find paste failed: {error}"),
                    }
                }
                // Let existing browser-level shortcuts (address, zoom, Reload) run.
                Key::Character(c)
                    if ["l", "r", "+", "=", "-", "_", "0"]
                        .contains(&c.to_ascii_lowercase().as_str()) =>
                {
                    return false;
                }
                _ => {}
            }
        } else {
            match key {
                Key::Named(NamedKey::Enter) => {
                    self.find_step(self.modifiers.shift_key());
                    return true;
                }
                // Winit reports Space as a named key on macOS, not character text.
                Key::Named(NamedKey::Space) if !self.modifiers.alt_key() => find.insert(" "),
                Key::Named(NamedKey::Backspace) => find.delete(false),
                Key::Named(NamedKey::Delete) => find.delete(true),
                Key::Named(NamedKey::ArrowLeft) => find.move_caret(-1),
                Key::Named(NamedKey::ArrowRight) => find.move_caret(1),
                Key::Named(NamedKey::Home) => find.end(false),
                Key::Named(NamedKey::End) => find.end(true),
                Key::Named(NamedKey::Tab) => {
                    find.focused = false;
                }
                Key::Character(c) if !self.modifiers.alt_key() => find.insert(c),
                _ => {}
            }
        }
        self.find_reveal = true;
        self.page = None;
        self.redraw();
        true
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        if let Some(window) = &self.devtools_window {
            window.request_redraw();
        }
    }

    fn request_devtools(&self) {
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(UserEvent::OpenDevtools);
        }
    }

    fn open_devtools_window(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.devtools_window {
            window.request_redraw();
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("BYUI Browser DevTools")
            .with_inner_size(LogicalSize::new(900.0, 620.0))
            .with_min_inner_size(LogicalSize::new(560.0, 360.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Rc::new(window),
            Err(error) => {
                log::error!("failed to create DevTools window: {error}");
                return;
            }
        };
        let context = softbuffer::Context::new(window.clone()).unwrap_or_else(|error| {
            log::error!("failed to create DevTools drawing context: {error}");
            panic!("failed to create DevTools drawing context: {error}");
        });
        let surface = softbuffer::Surface::new(&context, window.clone()).unwrap_or_else(|error| {
            log::error!("failed to create DevTools window surface: {error}");
            panic!("failed to create DevTools window surface: {error}");
        });
        let live_html = self.page_events.snapshot_html();
        let html = live_html
            .as_deref()
            .filter(|html| !html.is_empty())
            .or_else(|| self.loaded_page.as_ref().map(|page| page.html.as_str()));
        let mut panel = ElementsPanel::new(html);
        sync_devtools_panel(
            &mut panel,
            &self.network_log,
            &self.console,
            &self.performance,
        );
        self.devtools_panel = Some(panel);
        self.devtools_cursor = None;
        self.devtools_surface = Some(surface);
        self.devtools_window = Some(window.clone());
        log::info!("DevTools window opened");
        window.request_redraw();
    }

    fn close_devtools_window(&mut self) {
        if self.devtools_window.take().is_some() {
            self.devtools_surface = None;
            self.devtools_panel = None;
            self.devtools_cursor = None;
            log::info!("DevTools window closed");
        }
    }

    fn paint_devtools(&mut self) {
        let Some(window) = &self.devtools_window else {
            return;
        };
        let size = window.inner_size();
        let scale = window.scale_factor();
        let Some(mut pixmap) = Pixmap::new(size.width.max(1), size.height.max(1)) else {
            return;
        };
        let dark = window.theme() != Some(Theme::Light);
        let background = if dark {
            tiny_skia::Color::from_rgba8(17, 24, 39, 255)
        } else {
            tiny_skia::Color::from_rgba8(249, 250, 251, 255)
        };
        pixmap.fill(background);
        if let Some(panel) = &mut self.devtools_panel {
            sync_devtools_panel(panel, &self.network_log, &self.console, &self.performance);
            panel.draw(
                &mut pixmap,
                scale,
                dark,
                self.devtools_cursor,
                self.font.as_ref(),
                self.source_font.as_ref(),
            );
        }
        let Some(surface) = &mut self.devtools_surface else {
            return;
        };
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        for (destination, source) in buffer.iter_mut().zip(pixmap.pixels()) {
            *destination = (u32::from(source.red()) << 16)
                | (u32::from(source.green()) << 8)
                | u32::from(source.blue());
        }
        if let Err(error) = buffer.present() {
            log::warn!("failed to present DevTools window: {error}");
        }
    }

    /// Accepts a successfully loaded document and records one visit.
    ///
    /// Navigation calls this only after loading succeeds. Repaints, Reload,
    /// and the error page for a failed load must not call it.
    fn page_loaded(&mut self, url: &str, page: browser::Page) {
        self.page_loaded_with_behavior(url, page, NavigationBehavior::Auto);
    }

    fn record_page_resources(&mut self, url: &str, page: &browser::Page) {
        let Ok(base) = Url::parse(url) else {
            return;
        };
        let document = html::parse_raw_html(page.html.clone());
        let mut stylesheet_index = 0;
        let mut script_index = 0;
        let nodes = (0..)
            .map(html::NodeID::new)
            .map_while(|id| document.node(id).map(|node| (id, node)));
        for (_id, node) in nodes {
            let html::NodeKind::Element(element) = node.get_kind() else {
                continue;
            };
            let attributes = element.get_attributes();
            let resource = match element.get_name().as_str() {
                "link"
                    if attributes.iter().any(|attribute| {
                        attribute.get_name().eq_ignore_ascii_case("rel")
                            && attribute
                                .get_value()
                                .split_ascii_whitespace()
                                .any(|value| value.eq_ignore_ascii_case("stylesheet"))
                    }) =>
                {
                    let reference = attributes
                        .iter()
                        .find(|attribute| attribute.get_name().eq_ignore_ascii_case("href"))
                        .map(|attribute| attribute.get_value().as_str());
                    let loaded = stylesheet_index < page.stylesheets.len();
                    stylesheet_index += 1;
                    reference.map(|reference| {
                        (
                            reference,
                            devtools::network::ResourceType::Stylesheet,
                            loaded,
                        )
                    })
                }
                "style" => {
                    let inline = format!("{url}#inline-style-{stylesheet_index}");
                    stylesheet_index += 1;
                    self.network_log
                        .record(devtools::network::RequestEntry::new(
                            inline,
                            Some(200),
                            devtools::network::ResourceType::Stylesheet,
                            0,
                        ));
                    None
                }
                "script" => {
                    let reference = attributes
                        .iter()
                        .find(|attribute| attribute.get_name().eq_ignore_ascii_case("src"))
                        .map(|attribute| attribute.get_value().as_str());
                    let loaded = script_index < page.scripts.len();
                    script_index += 1;
                    reference.map(|reference| {
                        (reference, devtools::network::ResourceType::Script, loaded)
                    })
                }
                _ => None,
            };
            let Some((reference, kind, loaded)) = resource else {
                if element.get_name().as_str() == "script"
                    && !attributes
                        .iter()
                        .any(|attribute| attribute.get_name().eq_ignore_ascii_case("src"))
                {
                    let inline = format!("{url}#inline-script-{}", script_index.saturating_sub(1));
                    self.network_log
                        .record(devtools::network::RequestEntry::new(
                            inline,
                            Some(200),
                            devtools::network::ResourceType::Script,
                            0,
                        ));
                }
                continue;
            };
            let resolved = base
                .join(reference)
                .map(|resolved| resolved.to_string())
                .unwrap_or_else(|_| reference.to_owned());
            self.network_log
                .record(devtools::network::RequestEntry::new(
                    resolved,
                    loaded.then_some(200),
                    kind,
                    0,
                ));
        }
    }

    fn page_loaded_with_behavior(
        &mut self,
        url: &str,
        page: browser::Page,
        behavior: NavigationBehavior,
    ) {
        log::debug!("page displayed: {url}");
        self.loaded_page = Some(page);
        self.animation_clock = None;
        self.refresh_devtools_document();
        if let Ok(mut console) = self.console.lock() {
            console.clear();
        }
        self.clear_performance();
        self.network_log
            .record(devtools::network::RequestEntry::new(
                url,
                Some(200),
                devtools::network::ResourceType::Document,
                0,
            ));
        if let Some(page) = self.loaded_page.clone() {
            self.record_page_resources(url, &page);
        }
        if let Some(panel) = &mut self.devtools_panel {
            sync_devtools_panel(panel, &self.network_log, &self.console, &self.performance);
        }
        self.find_reveal = self.find.is_some();
        // Scripts of the new document run after its first frame is presented.
        self.scripts_evaluated = false;
        self.frame_key = None;
        match behavior {
            NavigationBehavior::Auto => self.browser_state.record_visit(url),
            NavigationBehavior::Replace => self.browser_state.replace_current(url),
            NavigationBehavior::Reload | NavigationBehavior::History => {}
        }
        if behavior == NavigationBehavior::History {
            self.history_rollback = None;
        }
        self.address = url.to_owned();
        self.address_before_edit = self.address.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_anchor = None;
        self.chrome.address_focused = false;
        self.page = None;
        self.page_layout = None;
        self.scroll_y = 0.0;
        self.redraw();
    }

    /// Replaces synthetic resource rows with completed controller diagnostics.
    fn record_network_diagnostics(&mut self, events: &[NetworkEvent]) {
        #[derive(Default)]
        struct RequestTiming {
            url: String,
            started: Option<Instant>,
            queue_started: Option<Instant>,
            queue_ms: Option<u64>,
            transport_started: Option<Instant>,
            headers_ms: Option<u64>,
            status: Option<u16>,
        }

        let mut requests: HashMap<u64, RequestTiming> = HashMap::new();
        let mut completed = Vec::new();
        for event in events {
            let request = requests.entry(event.request_id).or_default();
            request.url = event.url.clone();
            request.status = event.status.or(request.status);
            match event.phase {
                NetworkPhase::Started => request.started = Some(event.at),
                NetworkPhase::QueueStarted => request.queue_started = Some(event.at),
                NetworkPhase::QueueAdmitted => {
                    request.queue_ms = request.queue_started.map(|started| {
                        event.at.saturating_duration_since(started).as_millis() as u64
                    });
                }
                NetworkPhase::TransportStarted => request.transport_started = Some(event.at),
                NetworkPhase::ResponseHeaders => {
                    request.headers_ms = request.transport_started.map(|started| {
                        event.at.saturating_duration_since(started).as_millis() as u64
                    });
                }
                NetworkPhase::Completed | NetworkPhase::Failed | NetworkPhase::Aborted => {
                    completed.push((event.request_id, event.at, event.status));
                }
                _ => {}
            }
        }
        if completed.is_empty() {
            return;
        }
        completed.sort_by_key(|(_, ended, _)| *ended);
        self.network_log.clear();
        for (index, (id, ended, terminal_status)) in completed.into_iter().enumerate() {
            let Some(request) = requests.remove(&id) else {
                continue;
            };
            let duration_ms = request
                .started
                .map(|started| ended.saturating_duration_since(started).as_millis() as u64)
                .unwrap_or_default();
            let resource_type = if index == 0 {
                devtools::network::ResourceType::Document
            } else {
                resource_type_for_url(&request.url)
            };
            self.network_log.record(
                devtools::network::RequestEntry::new(
                    request.url,
                    terminal_status.or(request.status),
                    resource_type,
                    duration_ms,
                )
                .with_network_timing(request.queue_ms, request.headers_ms),
            );
        }
    }

    fn begin_address_edit(&mut self) {
        if let Some(find) = &mut self.find {
            find.focused = false;
        }
        self.address_before_edit = self.address.clone();
        self.select_all_address();
        self.chrome.address_focused = true;
        self.redraw();
    }

    fn cancel_address_edit(&mut self) {
        self.address = self.address_before_edit.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_anchor = None;
        self.chrome.address_focused = false;
        self.redraw();
    }

    fn submit_address(&mut self) {
        let address = self.address.trim().to_owned();
        self.address = address.clone();
        self.chrome.address_caret = self.address.len();
        self.chrome.address_anchor = None;
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
        if behavior != NavigationBehavior::History {
            self.history_rollback = None;
        }
        self.navigation_id += 1;
        let id = self.navigation_id;
        let loading = self.begin_loading();
        let (Some(loader), Some(proxy)) = (&self.loader, self.proxy.clone()) else {
            log::warn!("navigation to {address} unavailable: page loader is not running");
            self.finish_loading(loading);
            self.navigation_loading = None;
            if behavior == NavigationBehavior::History
                && let Some(previous) = self.history_rollback.take()
            {
                self.browser_state = previous;
            }
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
                self.loaded_page = Some(browser::error_page(&navigation.address, &error));
                self.animation_clock = None;
                self.refresh_devtools_document();
                self.clear_performance();
                self.network_log
                    .record(devtools::network::RequestEntry::new(
                        &navigation.address,
                        None,
                        devtools::network::ResourceType::Document,
                        0,
                    ));
                if let Some(panel) = &mut self.devtools_panel {
                    sync_devtools_panel(panel, &self.network_log, &self.console, &self.performance);
                }
                self.find_reveal = self.find.is_some();
                self.scripts_evaluated = false;
                self.frame_key = None;
                self.page = None;
                self.page_layout = None;
                self.scroll_y = 0.0;
                if behavior == NavigationBehavior::History
                    && let Some(previous) = self.history_rollback.take()
                {
                    self.browser_state = previous;
                }
                self.redraw();
            }
        }
        self.record_network_diagnostics(&navigation.network_events);
        if let Some(panel) = &mut self.devtools_panel {
            sync_devtools_panel(panel, &self.network_log, &self.console, &self.performance);
        }
        // Replace coarse loader download spans with phases emitted by `net`.
        // Parse, style, layout, paint, and script events remain unchanged.
        let mut timeline = navigation.timeline;
        timeline.retain(|event| event.category != EventCategory::Network);
        timeline.extend(network_phase_timeline(&navigation.network_events));
        self.record_performance(timeline);
    }

    /// Starts a fresh Performance timeline for a newly shown page.
    fn clear_performance(&mut self) {
        if let Ok(mut performance) = self.performance.lock() {
            performance.clear();
        }
    }

    /// Adds `events` to the Performance tab's timeline.
    fn record_performance(&self, events: impl IntoIterator<Item = TimelineEvent>) {
        record_events(&self.performance, events);
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
                    self.select_all_address();
                    self.redraw();
                    return;
                }
                "c" if self.chrome.address_focused => {
                    // Copy the selection, or the whole address when nothing
                    // is selected.
                    let text = self
                        .chrome
                        .address_selection(&self.address)
                        .map_or(self.address.as_str(), |(start, end)| {
                            &self.address[start..end]
                        })
                        .to_owned();
                    if let Err(error) =
                        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text))
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

    /// Selects the whole address, leaving the caret at its end.
    fn select_all_address(&mut self) {
        self.chrome.address_anchor = Some(0);
        self.chrome.address_caret = self.address.len();
    }

    /// Deletes the selected address text, if any, leaving the caret where it
    /// was, and clears the selection. Returns whether text was selected.
    fn delete_address_selection(&mut self) -> bool {
        let selection = self.chrome.address_selection(&self.address);
        self.chrome.address_anchor = None;
        let Some((start, end)) = selection else {
            return false;
        };
        self.address.replace_range(start..end, "");
        self.chrome.address_caret = start;
        true
    }

    /// Types `text` at the caret, replacing the selection if there is one.
    ///
    /// Control characters such as the newlines of a multi-line paste are
    /// dropped, since an address is a single line.
    fn insert_address_text(&mut self, text: &str) {
        self.delete_address_selection();
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
    /// before it; selected text is deleted instead.
    fn delete_address_text(&mut self, forward: bool) {
        if !self.delete_address_selection() {
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
        let selection = self.chrome.address_selection(&self.address);
        self.chrome.address_anchor = None;
        self.chrome.address_caret = match (movement, selection) {
            (CaretMove::Start, _) => 0,
            (CaretMove::End, _) => self.address.len(),
            (CaretMove::Left, Some((start, _))) => start,
            (CaretMove::Right, Some((_, end))) => end,
            (CaretMove::Left, None) => self.address[..caret]
                .chars()
                .next_back()
                .map_or(caret, |previous| caret - previous.len_utf8()),
            (CaretMove::Right, None) => self.address[caret..]
                .chars()
                .next()
                .map_or(caret, |next| caret + next.len_utf8()),
        };
        log::trace!("address caret moved to {}", self.chrome.address_caret);
        self.redraw();
    }

    /// Returns the address byte offset under the pointer, for placing the
    /// caret where the user presses or drags in the address field.
    fn address_offset_under_pointer(&self) -> Option<usize> {
        let window = self.window.as_ref()?;
        let (x, _) = self.cursor?;
        let scale = window.scale_factor();
        let width = window.inner_size().to_logical::<f64>(scale).width;
        Some(toolbar::address_offset_at(
            x,
            width,
            self.platform.window_controls_inset,
            scale as f32,
            self.font.as_ref(),
            &self.address,
        ))
    }

    /// Starts a press in the address field at byte `offset`: focuses the
    /// field and puts the caret there, ready for a drag to extend the
    /// selection.
    fn press_address_at(&mut self, offset: usize) {
        let was_focused = self.chrome.address_focused;
        if !was_focused {
            if let Some(find) = &mut self.find {
                find.focused = false;
            }
            self.address_before_edit = self.address.clone();
            self.chrome.address_focused = true;
        }
        let offset = toolbar::floor_char_boundary(&self.address, offset);
        self.chrome.address_anchor = Some(offset);
        self.chrome.address_caret = offset;
        self.address_press = Some(AddressPress {
            was_focused,
            dragged: false,
        });
        self.redraw();
    }

    /// Moves the dragging end of the address selection to byte `offset`
    /// while a press in the field is held.
    fn drag_address_to(&mut self, offset: usize) {
        let Some(press) = &mut self.address_press else {
            return;
        };
        let offset = toolbar::floor_char_boundary(&self.address, offset);
        if offset == self.chrome.address_caret {
            return;
        }
        press.dragged = true;
        self.chrome.address_caret = offset;
        self.redraw();
    }

    /// Ends a press in the address field. A click that did not drag selects
    /// the whole address if the field was not focused before, like other
    /// browsers; otherwise the caret or dragged selection stays.
    fn release_address(&mut self) {
        let Some(press) = self.address_press.take() else {
            return;
        };
        if !press.dragged && !press.was_focused {
            self.select_all_address();
        }
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
        let hovered = hovered.filter(|control| control.is_enabled(&self.chrome));
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
        if let Some(find) = &mut self.find {
            find.focused = false;
        }
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
        if let Some(find) = &mut self.find {
            find.focused = false;
        }
        let live_html = self.page_events.snapshot_html();
        let html = live_html
            .as_deref()
            .filter(|html| !html.is_empty())
            .or_else(|| self.loaded_page.as_ref().map(|page| page.html.as_str()));
        self.elements = Some(ElementsPanel::new(html));
        if let Some(panel) = &mut self.elements {
            sync_devtools_panel(panel, &self.network_log, &self.console, &self.performance);
        }
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
        // Read the error only once the page session has caught up, so a
        // just-loaded page's scripts have run and an earlier page's failure
        // is not reported against it.
        let script_error = self
            .page_events
            .settled_script_error(Duration::from_secs(5));
        log::debug!("compatibility audit {id} started for {}", page.url);
        let run = move || {
            let script_error = script_error();
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

    /// Points open DevTools (the dock and the DevTools window) at the newly
    /// loaded page. They stay open on the same tab; the Elements tree and
    /// Compatibility audit start over for the new document. An audit still
    /// running for the previous page is discarded; if the dock is showing
    /// Compatibility, a new one starts once the page's scripts are sent to
    /// the page session (see `about_to_wait`).
    fn refresh_devtools_document(&mut self) {
        self.audit_id += 1;
        let html = self.loaded_page.as_ref().map(|page| page.html.as_str());
        for panel in [&mut self.elements, &mut self.devtools_panel]
            .into_iter()
            .flatten()
        {
            panel.load_document(html);
        }
        self.frame_key = None;
        self.redraw();
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
        if self.modifiers.control_key()
            && self.modifiers.shift_key()
            && matches!(key, Key::Character(character) if character.eq_ignore_ascii_case("h"))
        {
            self.request_devtools();
            return;
        }
        if *key == Key::Named(NamedKey::Escape) {
            if self.context_menu.is_some() {
                self.dismiss_context_menu();
            } else if self.page_source.is_some() {
                self.close_page_source();
            } else if let Some(panel) = &mut self.elements
                && panel.dismiss_menu()
            {
                self.redraw();
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

    /// Advances running CSS animations when a frame is due: restyles and lays
    /// out the page for the time since its animation clock started, and asks
    /// the window to repaint it.
    ///
    /// Returns when the next frame is due, or `None` when no animation is
    /// running (or the page has not been laid out yet), so the event loop
    /// can sleep until the next input.
    fn pump_animations(&mut self) -> Option<Instant> {
        let (Some(layout), Some(clock)) = (&mut self.page_layout, self.animation_clock) else {
            self.next_animation_frame = None;
            return None;
        };
        if !layout.is_animating() {
            self.next_animation_frame = None;
            return None;
        }
        let now = Instant::now();
        let due = *self.next_animation_frame.get_or_insert(now);
        if now < due {
            return Some(due);
        }
        layout.animate(clock.elapsed());
        record_events(&self.performance, layout.take_timeline());
        if let Some(find) = &mut self.find {
            // Highlights follow text that the animation moved.
            find.replace_runs(layout.find_text());
        }
        // An animation can shrink the page below the current scroll position.
        let scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll());
        let still_running = layout.is_animating();
        if scroll_y != self.scroll_y {
            self.scroll_y = scroll_y;
            self.page_events.set_scroll_y(scroll_y);
        }
        // Only the page pixmap changes. The DevTools window is not repainted
        // every frame.
        self.page = None;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        let next = now + ANIMATION_FRAME_INTERVAL;
        self.next_animation_frame = still_running.then_some(next);
        still_running.then_some(next)
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
        self.page_events.set_scroll_y(self.scroll_y);
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
        self.find_reveal = self.find.is_some();
        self.page = None;
        self.page_layout = None;
        self.scroll_y = 0.0;
        self.page_events.set_scroll_y(self.scroll_y);
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
        if self.page_source.is_some() {
            return;
        }
        // The DevTools dock has its own menus, such as Network's columns.
        if let Some(panel) = &mut self.elements
            && panel.contains(self.cursor)
        {
            if panel.context_click(self.cursor) {
                self.redraw();
            }
            return;
        }
        self.chrome.pressed = None;
        self.last_toolbar_click = None;
        let Some((x, y)) = self.cursor else {
            return;
        };
        let Some(card) = toolbar::page_rect_with_find(width, height, scale, self.find.is_some())
        else {
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
        if let Some(find) = &mut self.find {
            let width = self
                .window
                .as_ref()
                .map_or(420.0, |w| w.inner_size().width as f64 / w.scale_factor())
                as f32;
            let on_bar = self.cursor.is_some_and(|(_, y)| {
                y >= toolbar::TOOLBAR_HEIGHT
                    && y < toolbar::TOOLBAR_HEIGHT + crate::find_in_page::HEIGHT as f64
            });
            if on_bar || self.find_pressed.is_some() {
                let action = FindInPage::action_at(width, self.cursor);
                match state {
                    ElementState::Pressed => {
                        self.find_pressed = action;
                        self.page_press = false;
                    }
                    ElementState::Released => {
                        let selected = self.find_pressed.take().filter(|a| Some(*a) == action);
                        match selected {
                            Some(FindAction::Close) => self.close_find(),
                            Some(FindAction::Next) => self.find_step(false),
                            Some(FindAction::Previous) => self.find_step(true),
                            Some(FindAction::Field) => {
                                if let Some(find) = &mut self.find {
                                    find.focus();
                                }
                                self.chrome.address_focused = false;
                                self.redraw();
                            }
                            None => {}
                        }
                    }
                }
                return;
            }
            if state == ElementState::Pressed {
                find.focused = false;
                self.redraw();
            }
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
                        } else if let Some(panel) = &mut self.elements
                            && panel.click(self.cursor)
                        {
                            self.redraw();
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
                    if control == Control::Address {
                        // The press already focused the field; dragging
                        // outside it still selects, so release anywhere.
                        self.release_address();
                    } else if self.chrome.hovered == Some(control) {
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
            Control::Back | Control::Forward => {
                let can_move = match control {
                    Control::Back => self.browser_state.can_go_back(),
                    Control::Forward => self.browser_state.can_go_forward(),
                    _ => false,
                };
                if !can_move {
                    return;
                }
                let previous = self.browser_state.clone();
                let address = match control {
                    Control::Back => self.browser_state.go_back(),
                    Control::Forward => self.browser_state.go_forward(),
                    _ => None,
                };
                if let Some(address) = address {
                    self.history_rollback = Some(previous);
                    self.start_navigation_with_behavior(&address, NavigationBehavior::History);
                }
            }
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
            // Tabs and the menu are not implemented yet; these controls only
            // give visual feedback.
            Control::NewTab | Control::Menu => {
                log::debug!("toolbar action: {control:?} (not implemented)");
            }
        }
    }

    fn paint(&mut self) {
        if self.window.is_none() || self.surface.is_none() {
            return;
        }
        self.ensure_welcome_loaded();
        self.chrome.can_go_back = self.browser_state.can_go_back();
        self.chrome.can_go_forward = self.browser_state.can_go_forward();
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
        let card =
            toolbar::page_rect_with_find(size.width, size.height, scale, self.find.is_some());
        let card_size = card.map(|card| (card.width, card.height));
        let mut full = self.find.is_some()
            || self.page.is_none()
            || self.context_menu.is_some()
            || self.page_source.is_some()
            || self.elements.is_some();
        let page_size_changed = self.page.as_ref().map(|(size, _)| *size) != card_size;
        if page_size_changed {
            let animation_clock = *self.animation_clock.get_or_insert_with(Instant::now);
            // Painting never runs the page script: evaluation joins the JS
            // engine thread, and doing that inside this draw leaves `alert`
            // unable to finish. `about_to_wait` starts it after this frame.
            self.page = card.and_then(|card| {
                let css_size = crate::content::css_viewport(card.width, card.height, page_scale);
                let layout_changed =
                    self.page_layout.as_ref().map(|layout| layout.css_size()) != Some(css_size);
                if layout_changed {
                    let mut layout = crate::content::PageLayout::new_at(
                        self.loaded_page.as_ref().expect("page was loaded"),
                        css_size.0,
                        css_size.1,
                        animation_clock.elapsed(),
                    );
                    record_events(&self.performance, layout.take_timeline());
                    self.page_layout = Some(layout);
                }
                let layout = self.page_layout.as_ref()?;
                if let Some(find) = &mut self.find {
                    if layout_changed {
                        find.replace_runs(layout.find_text());
                    }
                    if self.find_reveal || layout_changed {
                        if let Some(target) = find.scroll_target(layout.max_scroll()) {
                            self.scroll_y = target;
                            self.page_events.set_scroll_y(self.scroll_y);
                        }
                        self.find_reveal = false;
                    }
                }
                // A resize or reload can leave less to scroll than before.
                self.scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll());
                self.page_events.set_scroll_y(self.scroll_y);
                let (page, paint) = TimelineEvent::measure(EventCategory::Paint, "Paint", || {
                    crate::content::render_page_with_find(
                        layout,
                        card.width,
                        card.height,
                        page_scale,
                        self.scroll_y,
                        self.find.as_ref(),
                        self.chrome.dark,
                    )
                });
                record_events(&self.performance, [paint]);
                page.map(|page| ((card.width, card.height), page))
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
        if let Some(find) = &self.find {
            find.draw(&mut composed, scale, self.chrome.dark, self.font.as_ref());
        }
        if let Some(panel) = &mut self.elements {
            sync_devtools_panel(panel, &self.network_log, &self.console, &self.performance);
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
        if self.address_press.is_some()
            && let Some(offset) = self.address_offset_under_pointer()
        {
            self.drag_address_to(offset);
        }
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
            if control == Control::Address {
                if let Some(offset) = self.address_offset_under_pointer() {
                    self.press_address_at(offset);
                }
            } else {
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
        let card =
            toolbar::page_rect_with_find(size.width, size.height, scale, self.find.is_some())?;
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
            + if self.find.is_some() {
                crate::find_in_page::HEIGHT as f64
            } else {
                0.0
            }
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
            (self.platform.window_created)(&window);
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
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .devtools_window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
        {
            match event {
                WindowEvent::CloseRequested => self.close_devtools_window(),
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed =>
                {
                    match event.logical_key {
                        Key::Named(NamedKey::Escape) => {
                            if let Some(panel) = &mut self.devtools_panel
                                && panel.dismiss_menu()
                            {
                                if let Some(window) = &self.devtools_window {
                                    window.request_redraw();
                                }
                            } else {
                                self.close_devtools_window();
                            }
                        }
                        Key::Named(NamedKey::Delete) => {
                            let tab = self.devtools_panel.as_ref().map(ElementsPanel::tab);
                            match tab {
                                Some(DockTab::Network) => self.network_log.clear(),
                                Some(DockTab::Performance) => self.clear_performance(),
                                Some(DockTab::Console) => {
                                    if let Ok(mut console) = self.console.lock() {
                                        console.clear();
                                    }
                                }
                                _ => {}
                            }
                            if let Some(panel) = &mut self.devtools_panel {
                                panel.clear_active();
                            }
                            if let Some(window) = &self.devtools_window {
                                window.request_redraw();
                            }
                        }
                        Key::Named(key) => {
                            if let Some(panel) = &mut self.devtools_panel {
                                panel.key(&key);
                            }
                            if let Some(window) = &self.devtools_window {
                                window.request_redraw();
                            }
                        }
                        _ => {}
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(window) = &self.devtools_window {
                        let position = position.to_logical::<f64>(window.scale_factor());
                        self.devtools_cursor = Some((position.x, position.y));
                        window.request_redraw();
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    self.devtools_cursor = None;
                    if let Some(window) = &self.devtools_window {
                        window.request_redraw();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let (Some(panel), Some(cursor)) =
                        (&mut self.devtools_panel, self.devtools_cursor)
                    {
                        let changed = match panel.tab_at(Some(cursor)) {
                            Some(tab) => {
                                panel.set_tab(tab);
                                true
                            }
                            None => panel.click(Some(cursor)),
                        };
                        if changed && let Some(window) = &self.devtools_window {
                            window.request_redraw();
                        }
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Right,
                    ..
                } => {
                    if let Some(panel) = &mut self.devtools_panel
                        && panel.context_click(self.devtools_cursor)
                        && let Some(window) = &self.devtools_window
                    {
                        window.request_redraw();
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let rows = match delta {
                        MouseScrollDelta::LineDelta(_, y) => -f64::from(y) * 3.0,
                        MouseScrollDelta::PixelDelta(position) => {
                            let scale = self
                                .devtools_window
                                .as_ref()
                                .map_or(1.0, |window| window.scale_factor());
                            -position.y / (18.0 * scale)
                        }
                    };
                    if let (Some(panel), Some(cursor)) =
                        (&mut self.devtools_panel, self.devtools_cursor)
                        && panel.contains(Some(cursor))
                    {
                        panel.scroll(rows);
                        if let Some(window) = &self.devtools_window {
                            window.request_redraw();
                        }
                    }
                }
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    if let Some(window) = &self.devtools_window {
                        window.request_redraw();
                    }
                }
                WindowEvent::ThemeChanged(_) | WindowEvent::RedrawRequested => {
                    self.paint_devtools();
                }
                _ => {}
            }
            return;
        }
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
                if event.state == ElementState::Pressed && self.find_key(&event.logical_key) {
                    return;
                }
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
                if matches!(event, WindowEvent::ThemeChanged(_)) && self.find.is_some() {
                    self.page = None;
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
            UserEvent::OpenDevtools => self.open_devtools_window(_event_loop),
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
        let reload_poll = self
            .reload_completion
            .is_some()
            .then(|| Instant::now() + Duration::from_millis(40));
        let animation_frame = self.pump_animations();
        event_loop.set_control_flow(
            match [reload_poll, animation_frame].into_iter().flatten().min() {
                Some(deadline) => ControlFlow::WaitUntil(deadline),
                None => ControlFlow::Wait,
            },
        );

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
            Arc::new(crate::content::DevtoolsConsole(Arc::clone(&self.console))),
            alerts,
            self.runtime.clone(),
            device_pixel_ratio,
            viewport,
            navigate,
            Some(Arc::clone(&self.performance)),
        );
        // A dock left on Compatibility across navigation audits the new
        // page now that its scripts are queued ahead of the audit.
        if self.elements.as_ref().is_some_and(|panel| {
            panel.tab() == DockTab::Compatibility && *panel.compat.state() == CompatState::Idle
        }) {
            self.start_audit();
        }
    }
}

/// Adds `events` to the Performance timeline `log`. A free function so
/// closures that already borrow other `App` fields can record.
fn record_events(log: &Mutex<PerformanceLog>, events: impl IntoIterator<Item = TimelineEvent>) {
    if let Ok(mut log) = log.lock() {
        log.extend(events);
    }
}

/// Splits each completed Fetch request into the engine phases that can explain
/// a slow Network bar. Events without a paired endpoint are intentionally not
/// shown: an unfinished request has no meaningful duration yet.
fn network_phase_timeline(events: &[NetworkEvent]) -> Vec<TimelineEvent> {
    #[derive(Default)]
    struct Starts {
        url: String,
        request: Option<Instant>,
        cache: Option<Instant>,
        preflight: Option<Instant>,
        queue: Option<Instant>,
        transport: Option<Instant>,
        headers: Option<Instant>,
        finalized: Option<Instant>,
    }

    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| event.at);
    let mut starts: HashMap<u64, Starts> = HashMap::new();
    let mut timeline = Vec::new();
    for event in ordered {
        let state = starts.entry(event.request_id).or_default();
        state.url = event.url.clone();
        match event.phase {
            NetworkPhase::Started => state.request = Some(event.at),
            NetworkPhase::Prepared => {
                record_network_phase(
                    &mut timeline,
                    "Prepare",
                    state.request,
                    event.at,
                    &state.url,
                );
            }
            NetworkPhase::CacheLookup => state.cache = Some(event.at),
            NetworkPhase::CacheHit | NetworkPhase::CacheMiss => {
                record_network_phase(
                    &mut timeline,
                    "Cache lookup",
                    state.cache,
                    event.at,
                    &state.url,
                );
            }
            NetworkPhase::PreflightStarted => state.preflight = Some(event.at),
            NetworkPhase::PreflightCompleted => {
                record_network_phase(
                    &mut timeline,
                    "CORS preflight",
                    state.preflight,
                    event.at,
                    &state.url,
                );
            }
            NetworkPhase::QueueStarted => state.queue = Some(event.at),
            NetworkPhase::QueueAdmitted => {
                record_network_phase(&mut timeline, "Queue", state.queue, event.at, &state.url);
            }
            NetworkPhase::TransportStarted => state.transport = Some(event.at),
            NetworkPhase::ResponseHeaders => {
                record_network_phase(
                    &mut timeline,
                    "Transport",
                    state.transport,
                    event.at,
                    &state.url,
                );
                state.headers = Some(event.at);
            }
            NetworkPhase::ResponseFinalized => {
                record_network_phase(
                    &mut timeline,
                    "Response policy",
                    state.headers.or(state.cache),
                    event.at,
                    &state.url,
                );
                state.finalized = Some(event.at);
            }
            NetworkPhase::Completed | NetworkPhase::Failed | NetworkPhase::Aborted => {
                let label = match event.phase {
                    NetworkPhase::Completed => "Deliver response",
                    NetworkPhase::Failed => "Request failure",
                    NetworkPhase::Aborted => "Request abort",
                    _ => unreachable!(),
                };
                record_network_phase(
                    &mut timeline,
                    label,
                    state.finalized.or(state.headers).or(state.request),
                    event.at,
                    &state.url,
                );
            }
            NetworkPhase::PreflightCacheHit
            | NetworkPhase::PreflightCacheMiss
            | NetworkPhase::Redirect => {}
        }
    }
    timeline
}

/// Appends one labelled Network segment when both lifecycle boundaries exist.
fn record_network_phase(
    timeline: &mut Vec<TimelineEvent>,
    phase: &str,
    start: Option<Instant>,
    end: Instant,
    url: &str,
) {
    let Some(start) = start else {
        return;
    };
    timeline.push(TimelineEvent::new(
        EventCategory::Network,
        format!("{phase} {url}"),
        start,
        end.saturating_duration_since(start),
    ));
}

/// Infers the current resource class from a completed redacted request URL.
fn resource_type_for_url(url: &str) -> devtools::network::ResourceType {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if path.ends_with(".css") {
        devtools::network::ResourceType::Stylesheet
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        devtools::network::ResourceType::Script
    } else if matches!(
        path.rsplit('.').next(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico")
    ) {
        devtools::network::ResourceType::Image
    } else {
        devtools::network::ResourceType::Other
    }
}

/// Copies the session's Network, Console, and Performance data into a
/// DevTools `panel` so it shows the latest entries.
fn sync_devtools_panel(
    panel: &mut ElementsPanel,
    network: &devtools::network::NetworkLog,
    console: &Mutex<devtools::Console>,
    performance: &Mutex<PerformanceLog>,
) {
    panel.set_network(network.clone());
    if let Ok(console) = console.lock() {
        panel.set_console(console.clone());
    }
    if let Ok(performance) = performance.lock() {
        panel.set_performance(performance.clone());
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

    #[test]
    fn network_diagnostics_are_split_into_bottleneck_phases() {
        let origin = Instant::now();
        let event = |phase, millis| NetworkEvent {
            request_id: 7,
            phase,
            at: origin + Duration::from_millis(millis),
            url: "https://example.test/app.js".into(),
            status: (millis >= 9).then_some(200),
            from_cache: false,
            redirect_count: 0,
        };
        let timeline = network_phase_timeline(&[
            event(NetworkPhase::Started, 0),
            event(NetworkPhase::Prepared, 2),
            event(NetworkPhase::CacheLookup, 2),
            event(NetworkPhase::CacheMiss, 3),
            event(NetworkPhase::QueueStarted, 3),
            event(NetworkPhase::QueueAdmitted, 8),
            event(NetworkPhase::TransportStarted, 8),
            event(NetworkPhase::ResponseHeaders, 18),
            event(NetworkPhase::ResponseFinalized, 20),
            event(NetworkPhase::Completed, 21),
        ]);
        let phases: Vec<_> = timeline.iter().map(|event| event.label.as_str()).collect();
        assert_eq!(
            phases,
            [
                "Prepare https://example.test/app.js",
                "Cache lookup https://example.test/app.js",
                "Queue https://example.test/app.js",
                "Transport https://example.test/app.js",
                "Response policy https://example.test/app.js",
                "Deliver response https://example.test/app.js",
            ]
        );
        assert_eq!(timeline[2].duration, Duration::from_millis(5));
        assert_eq!(timeline[3].duration, Duration::from_millis(10));
    }

    fn app() -> App {
        App::with_runtime(
            Platform {
                configure_window: |attributes| attributes,
                window_created: |_| {},
                ui_font_files: Vec::new,
                page_font_files: Vec::new,
                window_controls_inset: 0.0,
            },
            browser::BrowserRuntime::in_memory(),
        )
    }

    /// Mirrors the event loop's consumed-key check before page-key handling.
    fn type_find_with_named_spaces(app: &mut App, text: &str) {
        for c in text.chars() {
            let key = if c == ' ' {
                Key::Named(NamedKey::Space)
            } else {
                Key::Character(c.to_string().into())
            };
            if !app.find_key(&key) {
                app.key_pressed(&key);
                panic!("Find did not consume {key:?}");
            }
        }
    }

    #[test]
    fn find_named_space_types_phrases_with_real_counts_and_highlight_ranges() {
        let page=browser::Page {url:"byui://test".into(),
            html:"<body><p>Start Welcome <span>to</span> BYUI <b>Browser</b> end</p><button>Click <span>Me</span></button></body>".into(),
            stylesheets:vec!["body {height:2000px}".into()],scripts:vec![], resources: Default::default() };
        for query in [
            "Welcome to",
            "BYUI Browser",
            "Click Me",
            "Welcome  to",
            " Welcome  to ",
        ] {
            let mut app = app();
            app.page_loaded("byui://test", page.clone());
            app.open_find();
            let layout = crate::content::PageLayout::new(&page, 600, 150);
            assert!(layout.max_scroll() > 100.0);
            app.find.as_mut().unwrap().replace_runs(layout.find_text());
            app.page_layout = Some(layout);
            app.scroll_y = 100.0;
            let history = app.browser_state.clone();
            let address = app.address.clone();
            type_find_with_named_spaces(&mut app, query);
            let find = app.find.as_ref().unwrap();
            assert_eq!(find.query, query);
            assert_eq!(find.count(), "1/1", "{query}");
            assert!(
                find.highlight_ranges(false).len() >= 2,
                "phrase crosses fragments: {query}"
            );
            assert_eq!(app.scroll_y, 100.0, "Space must not invoke page scrolling");
            assert_eq!(app.address, address);
            assert_eq!(app.browser_state, history);
        }
    }

    #[test]
    fn find_spaces_preserve_editing_and_shift_space_does_not_page_scroll() {
        let mut app = app();
        app.open_find();
        type_find_with_named_spaces(&mut app, "  Welcome  to  ");
        assert_eq!(app.find.as_ref().unwrap().query, "  Welcome  to  ");
        assert!(app.find_key(&Key::Named(NamedKey::Backspace)));
        assert_eq!(app.find.as_ref().unwrap().query, "  Welcome  to ");
        app.find_key(&Key::Named(NamedKey::Home));
        app.find_key(&Key::Named(NamedKey::Delete));
        assert_eq!(app.find.as_ref().unwrap().query, " Welcome  to ");
        app.find_key(&Key::Named(NamedKey::End));
        app.modifiers = ModifiersState::SHIFT;
        assert!(app.find_key(&Key::Named(NamedKey::Space)));
        assert_eq!(app.find.as_ref().unwrap().query, " Welcome  to  ");
        app.modifiers = ModifiersState::empty();
        assert!(app.find_key(&Key::Character(" ".into())));
        assert_eq!(app.find.as_ref().unwrap().query, " Welcome  to   ");
        app.find.as_mut().unwrap().select_all();
        type_find_with_named_spaces(&mut app, "   ");
        assert_eq!(app.find.as_ref().unwrap().count(), "0/0");
        assert!(
            app.find
                .as_ref()
                .unwrap()
                .highlight_ranges(false)
                .is_empty()
        );
        app.begin_address_edit();
        assert!(
            !app.find_key(&Key::Named(NamedKey::Space)),
            "Find must not intercept address input"
        );
    }

    #[test]
    fn find_shortcut_and_focus_keep_query_separate_from_address() {
        let mut app = app();
        app.ensure_welcome_loaded();
        let history = app.browser_state.clone();
        let address = app.address.clone();
        app.modifiers = ModifiersState::CONTROL;
        assert!(app.find_key(&Key::Character("f".into())));
        assert!(app.chrome.find_open);
        assert!(!app.chrome.address_focused);
        app.modifiers = ModifiersState::empty();
        assert!(app.find_key(&Key::Character("Welcome".into())));
        assert_eq!(app.find.as_ref().unwrap().query, "Welcome");
        assert_eq!(app.address, address);
        assert_eq!(app.browser_state, history);
        app.begin_address_edit();
        assert!(!app.find.as_ref().unwrap().focused);
        assert!(!app.find_key(&Key::Character("x".into())));
        #[cfg(target_os = "macos")]
        {
            app.modifiers = ModifiersState::SUPER;
            assert!(app.find_key(&Key::Character("f".into())));
        }
    }

    #[test]
    fn find_enter_shift_enter_escape_and_close_button() {
        let mut app = app();
        app.open_find();
        let page = browser::welcome_page();
        let layout = crate::content::PageLayout::new(&page, 600, 200);
        let find = app.find.as_mut().unwrap();
        find.replace_runs(layout.find_text());
        find.insert("o");
        let initial = find.count();
        app.find_key(&Key::Named(NamedKey::Enter));
        assert_ne!(app.find.as_ref().unwrap().count(), initial);
        app.modifiers = ModifiersState::SHIFT;
        app.find_key(&Key::Named(NamedKey::Enter));
        assert_eq!(app.find.as_ref().unwrap().count(), initial);
        app.modifiers = ModifiersState::empty();
        assert!(app.find_key(&Key::Named(NamedKey::Escape)));
        assert!(app.find.is_none());
        app.open_find();
        let rect = FindInPage::regions(420.0)
            .into_iter()
            .find(|(a, _)| *a == FindAction::Close)
            .unwrap()
            .1;
        app.cursor = Some(((rect.x() + 1.0) as f64, (rect.y() + 1.0) as f64));
        app.left_mouse_input(ElementState::Pressed);
        app.left_mouse_input(ElementState::Released);
        assert!(app.find.is_none());
        assert!(!app.chrome.find_open);
    }

    #[test]
    fn find_remains_independent_of_panels_navigation_and_zoom() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.open_find();
        app.find.as_mut().unwrap().insert("welcome");
        app.show_elements();
        assert!(!app.find.as_ref().unwrap().focused);
        app.key_pressed(&Key::Named(NamedKey::Escape));
        assert!(app.elements.is_none());
        app.show_page_source();
        assert!(app.page_source.is_some());
        assert!(app.find.is_some());
        app.close_page_source();
        app.page_loaded("byui://welcome", browser::welcome_page());
        assert!(app.find_reveal);
        assert!(app.page_layout.is_none());
        assert_eq!(app.find.as_ref().unwrap().query, "welcome");
        app.set_page_zoom(1.25);
        assert!(app.find_reveal);
        assert!(app.find.is_some());
        let history = app.browser_state.clone();
        app.refresh = None;
        app.activate(Control::Reload);
        assert_eq!(app.browser_state, history);
        assert!(app.find.is_some());
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
    fn devtools_stay_open_across_navigation_and_show_the_new_page() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.show_elements();
        app.select_dock_tab(DockTab::Compatibility);
        assert!(matches!(
            app.elements.as_ref().unwrap().compat.state(),
            CompatState::Ready(_)
        ));
        if let Ok(mut console) = app.console.lock() {
            console.log(devtools::Level::Log, "from the old page");
        }
        let audit_before = app.audit_id;

        let next = browser::Page {
            url: "byui://test/next".into(),
            html: "<html><body><article><h1>Next</h1></article></body></html>".into(),
            stylesheets: Vec::new(),
            scripts: Vec::new(),
            resources: Default::default(),
        };
        app.page_loaded("byui://test/next", next);
        let panel = app.elements.as_ref().expect("the dock stays open");
        assert_eq!(panel.tab(), DockTab::Compatibility, "the tab is kept");
        assert_eq!(
            *panel.compat.state(),
            CompatState::Idle,
            "the old page's audit is not shown for the new page"
        );
        assert!(app.audit_id > audit_before, "a running audit is discarded");
        assert!(app.console.lock().unwrap().entries().is_empty());
        app.select_dock_tab(DockTab::Elements);
        let mut frame = tiny_skia::Pixmap::new(800, 600).unwrap();
        let panel = app.elements.as_mut().unwrap();
        panel.draw(&mut frame, 1.0, false, None, None, None);
        assert!(
            panel.shows_element("article"),
            "Elements shows the new tree"
        );
        assert_eq!(app.browser_state.current_url(), Some("byui://test/next"));

        // A failed load keeps DevTools open too, on the error page.
        app.navigation_id += 1;
        let id = app.navigation_id;
        app.navigation_finished(
            id,
            navigation(
                "https://missing.test/",
                Err(browser::LoadError::Status {
                    status: 404,
                    status_text: "Not Found".into(),
                }),
            ),
            NavigationBehavior::Auto,
        );
        let panel = app.elements.as_ref().expect("the dock stays open");
        assert!(!panel.shows_element("article"));
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
            resources: Default::default(),
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
            resources: Default::default(),
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
        assert_eq!(app.chrome.address_selection(&app.address), Some((0, 14)));
        app.address.push_str("-draft");

        app.cancel_address_edit();

        assert_eq!(app.address, "byui://welcome");
        assert_eq!(app.chrome.address_selection(&app.address), None);
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
        assert_eq!(app.chrome.address_selection(&app.address), None);

        app.insert_address_text("?q=1");
        assert_eq!(app.address, "example.com/path?q=1");
    }

    #[test]
    fn arrow_keys_collapse_the_selection_and_move_the_caret() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.begin_address_edit();

        app.move_address_caret(CaretMove::Left);
        assert_eq!(app.chrome.address_selection(&app.address), None);
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
    fn dragging_in_the_address_selects_part_of_it() {
        let mut app = app();
        app.ensure_welcome_loaded();

        // Press and drag across "ui://" in an unfocused field.
        app.press_address_at(2);
        assert!(app.chrome.address_focused);
        app.drag_address_to(7);
        app.release_address();
        assert_eq!(app.chrome.address_selection(&app.address), Some((2, 7)));

        // Typing replaces only the selection.
        app.insert_address_text("-");
        assert_eq!(app.address, "by-welcome");
        assert_eq!(app.chrome.address_caret, 3);

        // Dragging backwards selects the same way.
        app.press_address_at(10);
        app.drag_address_to(3);
        app.release_address();
        assert_eq!(app.chrome.address_selection(&app.address), Some((3, 10)));
        app.delete_address_text(false);
        assert_eq!(app.address, "by-");
    }

    #[test]
    fn clicking_the_address_selects_all_then_places_the_caret() {
        let mut app = app();
        app.ensure_welcome_loaded();

        // The first click into an unfocused field selects everything.
        app.press_address_at(4);
        app.release_address();
        assert_eq!(app.chrome.address_selection(&app.address), Some((0, 14)));

        // A second click in the focused field places the caret.
        app.press_address_at(4);
        app.release_address();
        assert_eq!(app.chrome.address_selection(&app.address), None);
        assert_eq!(app.chrome.address_caret, 4);
    }

    #[test]
    fn arrow_keys_collapse_a_partial_selection_to_its_edge() {
        let mut app = app();
        app.ensure_welcome_loaded();
        app.press_address_at(9);
        app.drag_address_to(2);
        app.release_address();

        app.move_address_caret(CaretMove::Right);
        assert_eq!(app.chrome.address_caret, 9);
        assert_eq!(app.chrome.address_selection(&app.address), None);
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
            timeline: Vec::new(),
            network_events: Vec::new(),
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
            resources: Default::default(),
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
            resources: Default::default(),
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
