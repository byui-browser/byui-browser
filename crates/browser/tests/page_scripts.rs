use std::sync::{Arc, Mutex};

use browser::{BrowserRuntime, security_context_for_url_with_policy};
use security::{
    Origin, RendererSecurityContext, StoragePermission,
    permissions::{PermissionKind, PermissionPolicy},
};
use webapis::{AlertSink, ConsoleSink};

#[derive(Default)]
struct TestConsole {
    messages: Mutex<Vec<String>>,
}

impl ConsoleSink for TestConsole {
    fn log(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

#[derive(Default)]
struct TestAlert {
    messages: Mutex<Vec<String>>,
}

impl AlertSink for TestAlert {
    fn alert(&self, message: &str) {
        self.messages.lock().unwrap().push(message.to_owned());
    }
}

fn security_context() -> RendererSecurityContext {
    RendererSecurityContext::new(
        Origin::new("https", "example.com", None),
        StoragePermission::Allowed,
    )
}

#[test]
fn html_script_reaches_registered_web_api() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let results = runtime
        .evaluate_page_scripts(
            "<main><script>print()</script></main>",
            console.clone(),
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::Undefined]);
    assert_eq!(
        console.messages.lock().unwrap().as_slice(),
        ["JS API Team Rocks!"]
    );
    assert!(alert.messages.lock().unwrap().is_empty());
}

#[test]
fn window_onload_runs_after_page_scripts_finish() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let session = BrowserRuntime::in_memory()
        .open_page_session(
            "<script>\
                var scriptState = 'running';\
                function onLoaded(event) {\
                    window.onloadRan = this === window && event.type === 'load' &&\
                        event.target === document && scriptState === 'finished';\
                }\
                window.onload = onLoaded;\
                scriptState = 'finished';\
             </script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page session should dispatch its load event");

    assert_eq!(
        session.evaluate_script("window.onloadRan === true"),
        Ok(js::Value::Boolean(true))
    );
}

#[test]
fn inline_scripts_continue_after_errors_and_keep_diagnostics_in_order() {
    let session = BrowserRuntime::in_memory().open_page_session(
        "<script>var state = 'before'; throw 'first'; state = 'unreachable';</script>\
         <script>var invalid = ;</script>\
         <script>state += '|after'; window.onload = function () { state += '|load'; }; throw 'second';</script>",
        Arc::new(TestConsole::default()),
        Arc::new(TestAlert::default()),
        browser::request_controller(None),
        security_context(),
    ).expect("script errors must not discard the session");

    assert_eq!(
        session.evaluate_script("state"),
        Ok(js::Value::String("before|after|load".into()))
    );
    let errors = session.initial_script_errors();
    assert_eq!(errors.len(), 3);
    assert_eq!(errors[0].message, "Uncaught first");
    assert_eq!(errors[1].category, js::JsErrorCategory::Syntax);
    assert_eq!(errors[2].message, "Uncaught second");
}

#[test]
fn evaluating_scripts_reports_the_first_error_after_attempting_later_scripts() {
    let console = Arc::new(TestConsole::default());
    let error = BrowserRuntime::in_memory()
        .evaluate_page_scripts(
            "<script>throw 'first';</script><script>console.log('later'); throw 'second';</script>",
            console.clone(),
            Arc::new(TestAlert::default()),
            browser::request_controller(None),
            security_context(),
        )
        .expect_err("the evaluation helper still reports errors to its caller");

    assert_eq!(error.message, "Uncaught first");
    assert_eq!(console.messages.lock().unwrap().as_slice(), ["later"]);
}

#[test]
fn policy_context_controls_runtime_local_storage() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let mut policy = PermissionPolicy::new();
    policy.deny(
        Origin::new("https", "example.com", None),
        PermissionKind::Storage,
    );
    let context = security_context_for_url_with_policy("https://example.com/", &policy);

    let error = BrowserRuntime::in_memory()
        .evaluate_page_scripts(
            "<script>localStorage.setItem('key', 'value')</script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            context,
        )
        .expect_err("explicit storage denial should reach localStorage");

    assert_eq!(
        error.to_string(),
        "Uncaught SecurityError: localStorage write access denied"
    );
}

#[test]
fn runtime_policy_is_shared_by_cloned_handles_and_can_be_revoked() {
    let runtime = BrowserRuntime::in_memory();
    let cloned_runtime = runtime.clone();
    let example = Origin::new("https", "example.com", None);
    cloned_runtime.deny_permission(example.clone(), PermissionKind::Storage);

    let denied = runtime.security_context_for_url("https://example.com/page");
    assert!(!denied.can_read_storage());
    assert!(!denied.can_write_storage());

    runtime.revoke_permission(&example, PermissionKind::Storage);

    let restored = runtime.security_context_for_url("https://example.com/other");
    assert!(restored.can_read_storage());
    assert!(restored.can_write_storage());
}

#[test]
fn runtime_policy_is_scoped_by_origin() {
    let runtime = BrowserRuntime::in_memory();
    runtime.deny_permission(
        Origin::new("https", "example.com", None),
        PermissionKind::Storage,
    );

    let denied = runtime.security_context_for_url("https://example.com/");
    let allowed = runtime.security_context_for_url("https://other.example/");

    assert!(!denied.can_read_storage());
    assert!(allowed.can_read_storage());
}

#[test]
fn page_script_runs_queued_microtasks_before_the_next_script() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let results = BrowserRuntime::in_memory()
        .evaluate_page_scripts(
            "<script>var order = ''; function callback() { order = order + 'a'; } queueMicrotask(callback)</script><script>order</script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page script should evaluate");
    assert_eq!(results[1], js::Value::String("a".into()));
}

#[test]
fn page_session_script_evaluates_through_a_microtask_checkpoint() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();
    let session = runtime
        .open_page(
            &browser::Page {
                html: "<main></main>".into(),
                url: "https://example.com/".into(),
                scripts: Vec::new(),
                stylesheets: Vec::new(),
                resources: Default::default(),
            },
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page session should open");
    session
        .evaluate_script(
            "var order = ''; function callback() { order = order + 'a'; } queueMicrotask(callback)",
        )
        .expect("script should evaluate");
    assert_eq!(
        session.evaluate_script("order"),
        Ok(js::Value::String("a".into()))
    );
}

#[test]
fn page_session_reports_uncaught_errors_to_window_onerror() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let session = BrowserRuntime::in_memory()
        .open_page(
            &browser::Page {
                html: "<main></main>".into(),
                url: "https://example.com/".into(),
                scripts: Vec::new(),
                stylesheets: Vec::new(),
                resources: Default::default(),
            },
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page session should open");

    session
        .evaluate_script(
            "var report = ''; window.onerror = (message, source, line, column, error) => { \
             report = message + '|' + source + '|' + error; }; throw 'boom';",
        )
        .expect_err("the original uncaught script error remains observable");
    assert_eq!(
        session.evaluate_script("report"),
        Ok(js::Value::String(
            "Uncaught boom|https://example.com/|boom".into()
        ))
    );
}

#[test]
fn html_script_history_survives_document_registration() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let result = BrowserRuntime::in_memory()
        .evaluate_page_scripts(
            "<script>window.history.pushState('ready', ''); window.history.state</script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("history should remain available after document registration");

    assert_eq!(result, vec![js::Value::String("ready".into())]);
}

#[test]
fn html_script_reaches_registered_alert_api() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let results = runtime
        .evaluate_page_scripts(
            "<main><script>alert('team handshake')</script></main>",
            console,
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::Undefined]);
    assert_eq!(
        alert.messages.lock().unwrap().as_slice(),
        ["team handshake"]
    );
}

#[test]
fn html_script_reads_dom_attributes_through_document() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let results = runtime
        .evaluate_page_scripts(
        "<main id='app' data-state='ready'><script>document.getElementById('app').getAttribute('DATA-STATE')</script></main>",
        console,
        alert,
        controller,
        security_context(),
    )
    .expect("page script should evaluate");

    assert_eq!(results, vec![js::Value::String("ready".to_owned())]);
}

#[test]
fn linked_page_script_reads_its_navigation_url_from_window_location() {
    let console = Arc::new(TestConsole::default());
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let captured_navigations = navigations.clone();
    let page = browser::Page {
        url: "https://example.com/path?q=term#section".into(),
        html: String::new(),
        stylesheets: Vec::new(),
        scripts: vec!["if (document.location === window.location && window.location.href === 'https://example.com/path?q=term#section') window.location.href = window.location.href".into()],
        resources: Default::default(),
    };
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    BrowserRuntime::in_memory()
        .open_page_with_navigation(
            &page,
            console.clone(),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
            Arc::new(move |url, _behavior| {
                captured_navigations.lock().unwrap().push(url.to_string());
            }),
        )
        .expect("page script should evaluate");

    assert_eq!(navigations.lock().unwrap().as_slice(), [page.url]);
}

#[test]
fn window_location_shares_the_window_with_viewport_and_device_scale() {
    let navigations = Arc::new(Mutex::new(Vec::new()));
    let captured_navigations = navigations.clone();
    let page = browser::Page {
        url: "https://example.com/start".into(),
        html: String::new(),
        stylesheets: Vec::new(),
        scripts: vec![
            "if (window.devicePixelRatio === 2 && window.innerWidth === 800 \
             && window.innerHeight === 600 \
             && window.location.href === 'https://example.com/start') location.assign('/next')"
                .into(),
        ],
        resources: Default::default(),
    };
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );

    BrowserRuntime::in_memory()
        .open_page_with_viewport_and_navigation(
            &page,
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
            2.0,
            Some(webapis::ViewportSize {
                width: 800,
                height: 600,
            }),
            Arc::new(move |url, _behavior| {
                captured_navigations.lock().unwrap().push(url.to_string());
            }),
        )
        .expect("page script should evaluate");

    assert_eq!(
        navigations.lock().unwrap().as_slice(),
        ["https://example.com/next"]
    );
}

#[test]
fn viewport_resize_updates_inner_dimensions_and_dispatches_onresize() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let mut session = BrowserRuntime::in_memory()
        .open_page_with_viewport(
            &browser::Page {
                url: "https://example.com/resize".into(),
                html: String::new(),
                stylesheets: Vec::new(),
                scripts: vec![
                    "var resizeCalls = 0; var resizeSeen = null; \
                     function resizeHandler(event) { resizeCalls++; resizeSeen = \
                     [this === window, event.type, event.target === window, \
                      event.currentTarget === window, event.bubbles, event.cancelable, \
                      window.innerWidth, window.innerHeight]; } \
                     window.onresize = resizeHandler"
                        .into(),
                ],
                resources: Default::default(),
            },
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
            1.0,
            webapis::ViewportSize {
                width: 800,
                height: 600,
            },
        )
        .expect("page script should evaluate");

    session.set_viewport(webapis::ViewportSize {
        width: 1024,
        height: 768,
    });
    assert_eq!(
        session.evaluate_script(
            "resizeCalls === 1 && resizeSeen[0] && resizeSeen[1] === 'resize' && \
             resizeSeen[2] && resizeSeen[3] && !resizeSeen[4] && !resizeSeen[5] && \
             resizeSeen[6] === 1024 && resizeSeen[7] === 768 && \
             window.innerWidth === 1024 && window.innerHeight === 768",
        ),
        Ok(js::Value::Boolean(true))
    );

    session.set_viewport(webapis::ViewportSize {
        width: 1024,
        height: 768,
    });
    assert_eq!(
        session.evaluate_script("resizeCalls === 1"),
        Ok(js::Value::Boolean(true))
    );
}

#[test]
fn document_click_listener_runs_after_the_page_loads() {
    let console = Arc::new(TestConsole::default());
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let runtime = BrowserRuntime::in_memory();

    let session = runtime
        .open_page_session(
            "<script>\
                function callback() { alert('Clicked') }\
                document.addEventListener('click', callback)\
             </script>",
            console,
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("page session should load");

    assert!(alert.messages.lock().unwrap().is_empty());
    session
        .dispatch_click()
        .expect("document click should dispatch");
    assert_eq!(alert.messages.lock().unwrap().as_slice(), ["Clicked"]);
}

#[test]
fn linked_script_finds_page_elements_and_handles_their_clicks() {
    let alert = Arc::new(TestAlert::default());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let page = browser::Page {
        url: String::new(),
        html: "<main><span>Hi</span><button id=\"btn\">Click Me</button>\
               <script src=\"script.js\"></script></main>"
            .into(),
        stylesheets: Vec::new(),
        scripts: vec![
            "function callback() { alert('Clicked') }\n\
             const btn = document.getElementById('btn')\n\
             btn.addEventListener('click', callback);"
                .into(),
        ],
        resources: Default::default(),
    };

    let session = BrowserRuntime::in_memory()
        .open_page(
            &page,
            Arc::new(TestConsole::default()),
            alert.clone(),
            controller,
            security_context(),
        )
        .expect("linked script should find #btn");

    let document = html::parse_raw_html(page.html.clone());
    let button = document.get_element_by_id("btn").expect("#btn");
    let main = document
        .get_node(button)
        .and_then(|node| node.get_parent())
        .expect("main");
    session
        .dispatch_click_at(main)
        .expect("click outside the button should dispatch");
    assert!(alert.messages.lock().unwrap().is_empty());

    session
        .dispatch_click_at(button)
        .expect("button click should dispatch");
    assert_eq!(alert.messages.lock().unwrap().as_slice(), ["Clicked"]);
}

#[test]
fn page_script_initializes_dom_with_declaration_lists_and_hex_numbers() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let session = BrowserRuntime::in_memory()
        .open_page_session(
            "<main id='status'></main><script>\
                var mask = 0xffff, value = 0X2a;\
                let element = document.getElementById('status'), text = 'Ready';\
                element.textContent = text + ': ' + (value & mask);\
             </script>",
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page session");
    assert!(session.initial_script_errors().is_empty());
    assert_eq!(
        session
            .evaluate_script("document.getElementById('status').textContent")
            .unwrap(),
        js::Value::String("Ready: 42".into()),
    );
}

#[test]
fn page_scripts_use_regular_expressions_to_build_and_update_the_dom() {
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .build()
            .expect("network client"),
    );
    let session = BrowserRuntime::in_memory()
        .open_page_session(
            r#"<main id="status"></main>
               <script>
                 var email = /^\S+@\S+\.\S+$/i;
                 var label = ' SIGN IN '.replace(/^\s+|\s+$/g, '');
                 if (email.test('student@example.com')) {
                   document.getElementById('status').textContent = label;
                 }
               </script>
               <script>
                 var parts = 'one,two'.split(/\s*,\s*/);
                 document.getElementById('status').textContent += ': ' + parts[1];
               </script>"#,
            Arc::new(TestConsole::default()),
            Arc::new(TestAlert::default()),
            controller,
            security_context(),
        )
        .expect("page session");
    assert!(
        session.initial_script_errors().is_empty(),
        "{:?}",
        session.initial_script_errors()
    );
    assert_eq!(
        session
            .evaluate_script("document.getElementById('status').textContent")
            .unwrap(),
        js::Value::String("SIGN IN: two".into()),
    );
}
