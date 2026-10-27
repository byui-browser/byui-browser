//! In-process rendering demo using the layout team's HTML -> paint -> frame API.
//! The demo evaluates its page script once per document load before painting.

use std::sync::Arc;

use browser::{BrowserRuntime, Page};
use layout::{Size, StyledDom, layout_tree};
use render::{Compositor, Frame};
use security::{Origin, RendererSecurityContext, StoragePermission};
use tiny_skia::Pixmap;
use webapis::{AlertSink, ConsoleSink};

struct NoopConsole;

impl ConsoleSink for NoopConsole {
    fn log(&self, _message: &str) {}
}

fn welcome_security_context() -> RendererSecurityContext {
    RendererSecurityContext::new(
        Origin::new("byui", "welcome", None),
        StoragePermission::Allowed,
    )
}

fn render_demo(
    page: &browser::Page,
    width: u32,
    height: u32,
    alert_sink: Arc<dyn AlertSink>,
    runtime: &BrowserRuntime,
    scripts_evaluated: &mut bool,
) -> Frame {
    let stylesheet = css::parse_stylesheet(&page.css);
    assert!(
        !stylesheet.rules.is_empty(),
        "the welcome page should have a stylesheet"
    );
    if !*scripts_evaluated {
        // The completion value is not part of painting. `alert` returns
        // `undefined`, so requiring the welcome string aborts the first draw
        // and the window never opens.
        evaluate_page_script(page, Arc::clone(&alert_sink), runtime.clone());
        *scripts_evaluated = true;
    }
    render_document(page, width, height, alert_sink, runtime, scripts_evaluated)
}

fn render_document(
    page: &Page,
    width: u32,
    height: u32,
    alert_sink: Arc<dyn AlertSink>,
    runtime: &BrowserRuntime,
    scripts_evaluated: &mut bool,
) -> Frame {
    let document = html::parse_raw_html(page.html.to_owned());
    if !*scripts_evaluated {
        evaluate_page_script(page, alert_sink, runtime.clone());
        *scripts_evaluated = true;
    }
    let tree = layout_tree(
        &StyledDom::from_html_document(&document),
        Size {
            width: width as f32,
            height: height as f32,
        },
    );
    let display_list = paint::paint_document(&tree, &document);
    Compositor::new(width, height).compose_paint(&display_list)
}

/// Evaluates the page's script for its side effects, including `alert`.
///
/// Call this after the window has presented a frame. Running it inside the
/// draw callback joins the JS engine thread while the UI thread cannot
/// service a dialog.
pub(crate) fn evaluate_page_script(
    page: &Page,
    alert_sink: Arc<dyn AlertSink>,
    runtime: BrowserRuntime,
) -> Vec<js::Value> {
    let controller = browser::request_controller(None);
    let source = format!("<script>{}</script>", page.js);
    runtime
        .evaluate_page_scripts(
            &source,
            Arc::new(NoopConsole),
            alert_sink,
            controller,
            welcome_security_context(),
        )
        .expect("the page script should execute")
}

/// Renders the demo into a device-pixel pixmap sized for the page card.
/// The current compositor rasterizes at 1x; nearest-neighbor scaling keeps its
/// bitmap font at a consistent logical size on Retina. Returns `None` for an
/// empty area.
pub(crate) fn render_page(
    page: &browser::Page,
    width: u32,
    height: u32,
    scale_factor: f64,
    alert_sink: Arc<dyn AlertSink>,
    runtime: &BrowserRuntime,
    scripts_evaluated: &mut bool,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let frame = render_demo(
        page,
        (width as f64 / scale_factor).ceil() as u32,
        (height as f64 / scale_factor).ceil() as u32,
        alert_sink,
        runtime,
        scripts_evaluated,
    );
    for (y, row) in pixmap
        .data_mut()
        .chunks_exact_mut(width as usize * 4)
        .enumerate()
    {
        let source_y = ((y as f64 / scale_factor) as u32).min(frame.height - 1);
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let source_x = ((x as f64 / scale_factor) as u32).min(frame.width - 1);
            let offset = (source_y * frame.width + source_x) as usize * 4;
            let rgba = &frame.pixels[offset..offset + 4];
            // Composite RGBA onto the white page so the card stays opaque.
            let alpha = u32::from(rgba[3]);
            let channel =
                |value: u8| ((u32::from(value) * alpha + 255 * (255 - alpha)) / 255) as u8;
            pixel.copy_from_slice(&[channel(rgba[0]), channel(rgba[1]), channel(rgba[2]), 255]);
        }
    }
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SilentAlert;

    impl AlertSink for SilentAlert {
        fn alert(&self, _message: &str) {}
    }

    fn test_alert_sink() -> Arc<dyn AlertSink> {
        Arc::new(SilentAlert)
    }

    fn test_runtime() -> BrowserRuntime {
        BrowserRuntime::in_memory()
    }

    #[test]
    fn demo_produces_layout_background_and_renderer_text() {
        let page = browser::welcome_page();
        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        let frame = render_demo(
            &page,
            160,
            120,
            test_alert_sink(),
            &runtime,
            &mut scripts_evaluated,
        );
        assert_eq!((frame.width, frame.height), (160, 120));
        assert_eq!(&frame.pixels[..4], &[242, 245, 250, 255]);
        assert!(frame.pixels.chunks_exact(4).any(|p| p == [30, 43, 62, 255]));
        assert_eq!(&frame.pixels[160 * 110 * 4..160 * 110 * 4 + 4], &[255; 4]);
    }

    #[test]
    fn page_is_scaled_to_device_pixels() {
        let page_source = browser::welcome_page();
        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        for scale in [1.0, 1.5, 2.0] {
            let (width, height) = ((160.0 * scale) as u32, (120.0 * scale) as u32);
            let page = render_page(
                &page_source,
                width,
                height,
                scale,
                test_alert_sink(),
                &runtime,
                &mut scripts_evaluated,
            )
            .unwrap();
            assert_eq!((page.width(), page.height()), (width, height));
            let data = page.data();
            assert_eq!(&data[..4], &[242, 245, 250, 255]);
            assert!(data.chunks_exact(4).any(|p| p == [30, 43, 62, 255]));
            let bottom = ((110.0 * scale) as u32 * width) as usize * 4;
            assert_eq!(&data[bottom..bottom + 4], &[255; 4]);
        }
    }

    #[test]
    fn tiny_and_empty_pages_are_safe() {
        let page = browser::welcome_page();
        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        assert!(
            render_page(
                &page,
                0,
                10,
                1.0,
                test_alert_sink(),
                &runtime,
                &mut scripts_evaluated,
            )
            .is_none()
        );
        for (width, height) in [(1, 1), (3, 48), (17, 70)] {
            assert!(
                render_page(
                    &page,
                    width,
                    height,
                    2.0,
                    test_alert_sink(),
                    &runtime,
                    &mut scripts_evaluated,
                )
                .is_some()
            );
        }
    }

    #[test]
    fn rendered_page_scripts_use_the_alert_sink() {
        #[derive(Default)]
        struct TestAlert(std::sync::Mutex<Vec<String>>);

        impl AlertSink for TestAlert {
            fn alert(&self, message: &str) {
                self.0.lock().unwrap().push(message.to_owned());
            }
        }

        let alert = Arc::new(TestAlert::default());
        let page = Page {
            html: "<main></main>".into(),
            css: String::new(),
            js: "alert('rendered page')".into(),
        };

        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        render_document(
            &page,
            160,
            120,
            alert.clone(),
            &runtime,
            &mut scripts_evaluated,
        );

        assert_eq!(alert.0.lock().unwrap().as_slice(), ["rendered page"]);
    }

    #[test]
    fn welcome_page_alert_still_paints() {
        #[derive(Default)]
        struct Recording(std::sync::Mutex<Vec<String>>);

        impl AlertSink for Recording {
            fn alert(&self, message: &str) {
                self.0.lock().unwrap().push(message.to_owned());
            }
        }

        let welcome = browser::welcome_page();
        let page = Page {
            html: welcome.html,
            css: welcome.css,
            js: "alert('Hello from script.js')".into(),
        };
        let alert = Arc::new(Recording::default());
        let runtime = test_runtime();
        let mut scripts_evaluated = false;
        let frame = render_demo(
            &page,
            160,
            120,
            alert.clone(),
            &runtime,
            &mut scripts_evaluated,
        );

        assert!(scripts_evaluated);
        assert_eq!(alert.0.lock().unwrap().as_slice(), ["Hello from script.js"]);
        assert_eq!((frame.width, frame.height), (160, 120));
        assert_eq!(&frame.pixels[..4], &[242, 245, 250, 255]);
    }
}
