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
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowAttributes, WindowId};

use crate::toolbar::{self, ChromeState, Control};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Width in logical pixels of the invisible resize handles along the window
/// edges, used only when [`Platform::resize_borders`] is set.
const RESIZE_BORDER: f64 = 6.0;

/// How a native shell wants its window set up.
///
/// The shared shell draws its own window controls in the toolbar, so every
/// platform hides the native titlebar one way or another; the differences
/// between desktops are captured here.
#[derive(Clone, Copy, Debug)]
pub struct Platform {
    /// Adjusts the default window attributes before the window is created,
    /// for example to hide the native titlebar or decorations. The title and
    /// sizes are already set when this runs.
    pub configure_window: fn(WindowAttributes) -> WindowAttributes,
    /// Returns font files to try, in order, for toolbar text. The first one
    /// that loads is used; if none loads the toolbar omits its text.
    pub ui_font_files: fn() -> Vec<PathBuf>,
    /// Whether the shell must provide resize handles itself. Set this when
    /// hiding the decorations also removes the native resize border, as on
    /// Linux; macOS keeps native edge resizing for frameless windows.
    pub resize_borders: bool,
}

struct App {
    platform: Platform,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    font: Option<FontVec>,
    chrome: ChromeState,
    /// Pointer position in logical pixels while it is over the window.
    cursor: Option<(f64, f64)>,
    /// Window edge under the pointer when the shell draws its own resize
    /// borders; takes precedence over toolbar controls.
    resize: Option<ResizeDirection>,
    /// The rendered page and the device size it was rendered for.
    page: Option<((u32, u32), Pixmap)>,
    /// Last presented window contents in softbuffer's 0x00RRGGBB format,
    /// kept so chrome-only changes repaint just the toolbar strip.
    frame: Vec<u32>,
    /// Window size, scale-factor bits, and theme `frame` was painted for.
    frame_key: Option<(u32, u32, u64, bool)>,
    last_toolbar_click: Option<Instant>,
}

impl App {
    fn new(platform: Platform) -> Self {
        Self {
            platform,
            window: None,
            surface: None,
            font: None,
            chrome: ChromeState::default(),
            cursor: None,
            resize: None,
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

    fn update_cursor(&self) {
        let Some(window) = &self.window else {
            return;
        };
        window.set_cursor(match (self.resize, self.chrome.hovered) {
            (Some(direction), _) => resize_cursor(direction),
            (None, Some(Control::Address)) => CursorIcon::Text,
            (None, _) => CursorIcon::Default,
        });
    }

    fn set_hovered(&mut self, hovered: Option<Control>) {
        let hovered = hovered.filter(|control| control.is_enabled());
        if self.chrome.hovered != hovered {
            self.chrome.hovered = hovered;
            self.update_cursor();
            self.redraw();
        }
    }

    fn set_resize(&mut self, resize: Option<ResizeDirection>) {
        if self.resize != resize {
            self.resize = resize;
            self.update_cursor();
        }
    }

    fn activate(&mut self, control: Control, event_loop: &ActiveEventLoop) {
        let Some(window) = &self.window else {
            return;
        };
        match control {
            Control::Close => event_loop.exit(),
            Control::Minimize => window.set_minimized(true),
            Control::Zoom => window.set_maximized(!window.is_maximized()),
            Control::Reload => {
                self.page = None;
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
        let size = window.inner_size().to_logical::<f64>(scale);
        self.cursor = Some((x, y));
        let resize = if self.platform.resize_borders && !window.is_maximized() {
            resize_direction(x, y, size.width, size.height)
        } else {
            None
        };
        self.set_resize(resize);
        if resize.is_some() {
            self.set_hovered(None);
        } else {
            self.set_hovered(toolbar::hit_test(x, y, size.width));
        }
    }

    fn pressed(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        if let Some(direction) = self.resize {
            let _ = window.drag_resize_window(direction);
        } else if let Some(control) = self.chrome.hovered {
            self.chrome.pressed = Some(control);
            self.redraw();
        } else if self
            .cursor
            .is_some_and(|(_, y)| y < toolbar::TOOLBAR_HEIGHT)
        {
            let now = Instant::now();
            let double = self
                .last_toolbar_click
                .is_some_and(|last| now - last < DOUBLE_CLICK);
            if double {
                self.last_toolbar_click = None;
                window.set_maximized(!window.is_maximized());
            } else {
                self.last_toolbar_click = Some(now);
                // Empty toolbar space moves the window, like a titlebar.
                let _ = window.drag_window();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            // The toolbar spans the full window width and draws the window
            // controls itself, so the platform hides its native titlebar.
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
            self.chrome.focused = window.has_focus();
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
            | WindowEvent::ThemeChanged(_) => self.redraw(),
            WindowEvent::Focused(focused) => {
                self.chrome.focused = focused;
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
                self.set_resize(None);
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
                            self.activate(control, event_loop);
                        }
                    }
                }
            },
            WindowEvent::RedrawRequested => self.paint(),
            _ => {}
        }
    }
}

/// Returns the resize handle under a pointer at (`x`, `y`) logical pixels in
/// a window of `width` × `height` logical pixels, or `None` away from the
/// edges.
fn resize_direction(x: f64, y: f64, width: f64, height: f64) -> Option<ResizeDirection> {
    let left = x < RESIZE_BORDER;
    let right = x >= width - RESIZE_BORDER;
    let top = y < RESIZE_BORDER;
    let bottom = y >= height - RESIZE_BORDER;
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, _, _, _) => ResizeDirection::West,
        (_, true, _, _) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::East => CursorIcon::EResize,
        ResizeDirection::West => CursorIcon::WResize,
        ResizeDirection::North => CursorIcon::NResize,
        ResizeDirection::South => CursorIcon::SResize,
        ResizeDirection::NorthEast => CursorIcon::NeResize,
        ResizeDirection::NorthWest => CursorIcon::NwResize,
        ResizeDirection::SouthEast => CursorIcon::SeResize,
        ResizeDirection::SouthWest => CursorIcon::SwResize,
    }
}

/// Opens the browser window described by `platform` and runs it until the
/// user closes it.
pub fn run(platform: Platform) -> Result<(), EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut App::new(platform))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_handles_cover_edges_and_corners() {
        assert_eq!(resize_direction(100.0, 100.0, 800.0, 600.0), None);
        assert_eq!(
            resize_direction(2.0, 300.0, 800.0, 600.0),
            Some(ResizeDirection::West)
        );
        assert_eq!(
            resize_direction(799.0, 300.0, 800.0, 600.0),
            Some(ResizeDirection::East)
        );
        assert_eq!(
            resize_direction(400.0, 0.0, 800.0, 600.0),
            Some(ResizeDirection::North)
        );
        assert_eq!(
            resize_direction(400.0, 598.0, 800.0, 600.0),
            Some(ResizeDirection::South)
        );
        assert_eq!(
            resize_direction(1.0, 1.0, 800.0, 600.0),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            resize_direction(797.0, 597.0, 800.0, 600.0),
            Some(ResizeDirection::SouthEast)
        );
        assert_eq!(
            resize_direction(0.0, 599.0, 800.0, 600.0),
            Some(ResizeDirection::SouthWest)
        );
        assert_eq!(
            resize_direction(799.0, 0.0, 800.0, 600.0),
            Some(ResizeDirection::NorthEast)
        );
        // Just inside the handle width is page content.
        assert_eq!(resize_direction(6.0, 300.0, 800.0, 600.0), None);
    }
}
