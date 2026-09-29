//! Minimal macOS shell. Run with `cargo run -p browser-macos`.

#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
mod macos {
    use winit::application::ApplicationHandler;
    use winit::dpi::LogicalSize;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Window, WindowId};

    #[derive(Default)]
    struct App {
        window: Option<Window>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                let attributes = Window::default_attributes()
                    .with_title("BYUI Browser")
                    .with_inner_size(LogicalSize::new(640.0, 480.0));
                self.window = Some(
                    event_loop
                        .create_window(attributes)
                        .expect("failed to create macOS window"),
                );
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            if matches!(event, WindowEvent::CloseRequested) {
                event_loop.exit();
            }
        }
    }

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
