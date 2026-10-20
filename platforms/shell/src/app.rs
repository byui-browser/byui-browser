//! The winit application shared by the desktop shells: window creation,
//! pointer handling for the painted chrome, and presenting frames through
//! `softbuffer`.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ab_glyph::FontVec;
use tiny_skia::Pixmap;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::error::EventLoopError;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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
    /// Pointer position in logical pixels while it is over the window.
    cursor: Option<(f64, f64)>,
    /// The rendered page and the device size it was rendered for.
    page: Option<((u32, u32), Pixmap)>,
    /// Last presented window contents in softbuffer's 0x00RRGGBB format,
    /// kept so chrome-only changes repaint just the toolbar strip.
    frame: Vec<u32>,
    /// Window size, scale-factor bits, theme, and popover state `frame` was
    /// painted for.
    frame_key: Option<(u32, u32, u64, bool, bool)>,
    last_toolbar_click: Option<Instant>,
}

impl App {
    fn new(platform: Platform) -> Self {
        Self {
            platform,
            window: None,
            surface: None,
            font: None,
            chrome: ChromeState {
                window_controls_inset: platform.window_controls_inset,
                ..ChromeState::default()
            },
            cursor: None,
            page: None,
            frame: Vec::new(),
            frame_key: None,
            last_toolbar_click: None,
        }
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
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
                self.page = None;
                self.redraw();
            }
            Control::Security => {
                self.chrome.security_popover = !self.chrome.security_popover;
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
            self.page = card.and_then(|card| {
                crate::content::render_page(card.width, card.height, scale)
                    .map(|page| ((card.width, card.height), page))
            });
            full = true;
        }
        let key = (
            size.width,
            size.height,
            scale.to_bits(),
            self.chrome.dark,
            self.chrome.security_popover,
        );
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
        if self.chrome.security_popover && self.chrome.hovered != Some(Control::Security) {
            self.chrome.security_popover = false;
            self.redraw();
        }
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
        }
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
                self.set_hovered(None);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => self.pressed(),
                ElementState::Released => {
                    if let Some(control) = self.chrome.pressed.take() {
                        self.redraw();
                        if self.chrome.hovered == Some(control) {
                            self.activate(control);
                        }
                    }
                }
            },
            WindowEvent::RedrawRequested => self.paint(),
            _ => {}
        }
    }
}

/// Opens the browser window described by `platform` and runs it until the
/// user closes it.
pub fn run(platform: Platform) -> Result<(), EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut App::new(platform))
}
