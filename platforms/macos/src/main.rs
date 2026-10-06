//! Minimal macOS shell. Run with `cargo run -p browser-macos`.

#![forbid(unsafe_code)]

#[cfg(any(target_os = "macos", test))]
mod toolbar;

#[cfg(target_os = "macos")]
mod macos {
    use std::num::NonZeroU32;
    use std::rc::Rc;

    use winit::application::ApplicationHandler;
    use winit::dpi::LogicalSize;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Window, WindowId};

    #[derive(Default)]
    struct App {
        window: Option<Rc<Window>>,
        surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                let attributes = Window::default_attributes()
                    .with_title("BYUI Browser")
                    .with_inner_size(LogicalSize::new(640.0, 480.0));
                let window = Rc::new(
                    event_loop
                        .create_window(attributes)
                        .expect("failed to create macOS window"),
                );
                let context = softbuffer::Context::new(window.clone())
                    .expect("failed to create drawing context");
                self.surface = Some(
                    softbuffer::Surface::new(&context, window.clone())
                        .expect("failed to create window surface"),
                );
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
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                WindowEvent::RedrawRequested => {
                    let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
                        return;
                    };
                    let size = window.inner_size();
                    let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    else {
                        return;
                    };
                    surface
                        .resize(width, height)
                        .expect("failed to resize window surface");
                    let mut buffer = surface.buffer_mut().expect("failed to get window buffer");
                    super::toolbar::draw(&mut buffer, size.width as usize, window.scale_factor());
                    buffer.present().expect("failed to present window buffer");
                }
                _ => {}
            }
        }
    }

    /// Runs the macOS window until the user closes it.
    pub fn run() -> Result<(), winit::error::EventLoopError> {
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop.run_app(&mut App::default())
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), winit::error::EventLoopError> {
    macos::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("browser-macos requires macOS.");
}
