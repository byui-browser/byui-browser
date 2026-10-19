//! Minimal native Windows shell. Run with `cargo run -p byui-browser-windows`.

#[cfg(any(target_os = "windows", test))]
mod toolbar;

#[cfg(any(target_os = "windows", test))]
mod content;

#[cfg(target_os = "windows")]
mod windows {
    use std::num::NonZeroU32;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use ab_glyph::FontVec;
    use tiny_skia::Pixmap;
    use winit::application::ApplicationHandler;
    use winit::dpi::LogicalSize;
    use winit::event::{ElementState, MouseButton, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{CursorIcon, Theme, Window, WindowId};

    use super::toolbar::{self, ChromeState, Control};

    const DOUBLE_CLICK: Duration = Duration::from_millis(400);

    #[derive(Default)]
    struct App {
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
        /// Window size, scale-factor bits, and theme `frame` was painted for.
        frame_key: Option<(u32, u32, u64, bool)>,
        last_toolbar_click: Option<Instant>,
        history: Vec<u8>,
        history_index: usize,
    }

    impl App {
        fn redraw(&self) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }

        fn set_hovered(&mut self, hovered: Option<Control>) {
            let hovered = hovered.filter(|control| self.chrome.is_enabled(*control));
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
            self.set_hovered(toolbar::hit_test(x, y, width));
        }

        fn activate(&mut self, control: Control, event_loop: &ActiveEventLoop) {
            let Some(window) = &self.window else {
                return;
            };
            self.chrome.can_go_back = self.history_index > 0;
            self.chrome.can_go_forward = self.history_index + 1 < self.history.len();
            match control {
                Control::Close => event_loop.exit(),
                Control::Minimize => window.set_minimized(true),
                Control::Zoom => window.set_maximized(!window.is_maximized()),
                Control::Reload => {
                    self.page = None;
                    self.redraw();
                }
                Control::Back if self.history_index > 0 => {
                    self.history_index -= 1;
                    self.page = None;
                    self.redraw();
                }
                Control::Forward if self.history_index + 1 < self.history.len() => {
                    self.history_index += 1;
                    self.page = None;
                    self.redraw();
                }
                // New Tab creates a second local page so Back and Forward have
                // real history until URL navigation is connected.
                Control::NewTab => {
                    self.history.truncate(self.history_index + 1);
                    self.history.push(self.history.len() as u8);
                    self.history_index += 1;
                    self.page = None;
                    self.redraw();
                }
                Control::Address | Control::Menu | Control::Back | Control::Forward => {}
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
            self.chrome.can_go_back = self.history_index > 0;
            self.chrome.can_go_forward = self.history_index + 1 < self.history.len();
            let card = toolbar::page_rect(size.width, size.height, scale);
            let card_size = card.map(|card| (card.width, card.height));
            let mut full = self.page.is_none();
            if self.page.as_ref().map(|(size, _)| *size) != card_size {
                self.page = card.and_then(|card| {
                    super::content::render_page(
                        card.width,
                        card.height,
                        scale,
                        self.history[self.history_index],
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
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                // Keep the native Windows titlebar and system window controls.
                // The browser toolbar is rendered below it.
                let attributes = Window::default_attributes()
                    .with_title("BYUI Browser")
                    .with_inner_size(LogicalSize::new(1120.0, 720.0))
                    .with_min_inner_size(LogicalSize::new(420.0, 280.0));
                let window = Rc::new(
                    event_loop
                        .create_window(attributes)
                        .expect("failed to create Windows window"),
                );
                let context = softbuffer::Context::new(window.clone())
                    .expect("failed to create drawing context");
                self.surface = Some(
                    softbuffer::Surface::new(&context, window.clone())
                        .expect("failed to create window surface"),
                );
                self.font = toolbar::load_ui_font();
                self.history = vec![0];
                self.history_index = 0;
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
                WindowEvent::Resized(_) => {
                    self.refresh_hover();
                    self.redraw();
                }
                WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ThemeChanged(_) => {
                    self.refresh_hover();
                    self.redraw();
                }
                WindowEvent::Focused(focused) => {
                    self.chrome.focused = focused;
                    self.redraw();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let Some(window) = &self.window else {
                        return;
                    };
                    let scale = window.scale_factor();
                    let position = position.to_logical::<f64>(scale);
                    let width = window.inner_size().to_logical::<f64>(scale).width;
                    self.cursor = Some((position.x, position.y));
                    self.set_hovered(toolbar::hit_test(position.x, position.y, width));
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
                    ElementState::Pressed => {
                        if let Some(control) = self.chrome.hovered {
                            self.chrome.pressed = Some(control);
                            self.redraw();
                        } else if self
                            .cursor
                            .is_some_and(|(_, y)| y < toolbar::TOOLBAR_HEIGHT)
                        {
                            let Some(window) = &self.window else {
                                return;
                            };
                            let cursor_control = self.cursor.and_then(|(x, y)| {
                                let width = window
                                    .inner_size()
                                    .to_logical::<f64>(window.scale_factor())
                                    .width;
                                toolbar::hit_test(x, y, width)
                            });
                            // Disabled controls are still controls: clicking
                            // them must not fall through to window dragging.
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
                                // Empty toolbar space moves the window, like a titlebar.
                                let _ = window.drag_window();
                            }
                        }
                    }
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

    /// Runs the Windows window until the user closes it.
    pub fn run() -> Result<(), winit::error::EventLoopError> {
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop.run_app(&mut App::default())
    }
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), winit::error::EventLoopError> {
    windows::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("browser-windows requires Windows.");
}
