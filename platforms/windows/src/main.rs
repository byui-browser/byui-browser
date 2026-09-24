// GG Browser — minimal blank browser window
//
// Uses `tao` for the native OS window and `wry` for the embedded
// web view (on Windows this is backed by Microsoft Edge WebView2).

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

fn main() -> wry::Result<()> {
    // 1. Create the event loop and the native window.
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("GG Browser")
        .with_inner_size(LogicalSize::new(1100.0, 720.0))
        .build(&event_loop)
        .expect("failed to create window");

    // 2. Attach a web view to the window.
    //    "about:blank" gives you a genuinely blank browser window.
    //    Swap this for any URL (e.g. "https://example.com") once
    //    you're ready to actually browse somewhere.
    let _webview = WebViewBuilder::new(&window)
        .with_url("about:blank")
        .build()?;

    // 3. Run the event loop until the window is closed.
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
