//! Shared helpers for running the data fixtures in `tests/fixtures/`.
//!
//! Each fixture is a folder with `input.*` files and an `expected.txt`. A test
//! turns the input into "console output" (mostly Rust `{:?}` of the real
//! types), and compares it with `expected.txt`.
//!
//! - `BLESS=1 cargo test -p fixture-runner` rewrites `expected.txt` from the
//!   current output. Review the diff against the spec before committing.
//! - A case with a `KNOWN_FAIL` file is expected to mismatch (the code does
//!   not meet the spec yet). It is reported but does not fail the run, and it
//!   is never rewritten by `BLESS`. Delete the file once the case passes.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    thread,
};

use html::{Attribute, ElementData, HTMLDocument, Namespace, NodeId, NodeKind};

/// Returns the fixture case folders under `tests/fixtures/<group>`, sorted.
///
/// `group` is a team name (`html`) or an integration join
/// (`integration/parse-style`).
pub fn cases(group: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(group);
    let mut cases: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", root.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    cases
}

/// Reads a UTF-8 input file from a case folder.
pub fn read(case: &Path, file: &str) -> String {
    let path = case.join(file);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Runs `produce` on every case in `group` and checks it against
/// `expected.txt`. Panics listing every unexpected mismatch.
pub fn run_group(group: &str, produce: impl Fn(&Path) -> String) {
    let bless = std::env::var_os("BLESS").is_some();
    let mut failures = Vec::new();
    for case in cases(group) {
        let actual = produce(&case);
        let expected_path = case.join("expected.txt");
        let known_fail = case.join("KNOWN_FAIL").exists();
        if bless && !known_fail {
            fs::write(&expected_path, &actual).expect("write expected.txt");
            continue;
        }
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        match (expected == actual, known_fail) {
            (true, false) => {}
            (true, true) => failures.push(format!(
                "{group}/{name} now passes: delete its KNOWN_FAIL file"
            )),
            (false, true) => eprintln!("known failure: {group}/{name}"),
            (false, false) => failures.push(format!(
                "{group}/{name} mismatch\n--- expected\n{expected}\n--- actual\n{actual}"
            )),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Prints a DOM as an indented tree, one `{:?}` of each node's kind per line.
pub fn dump_dom(document: &HTMLDocument) -> String {
    fn walk(document: &HTMLDocument, id: NodeId, depth: usize, out: &mut String) {
        let Some(node) = document.node(id) else {
            return;
        };
        out.push_str(&"  ".repeat(depth));
        out.push_str(&format!("{:?}\n", node.kind));
        for child in &node.children {
            walk(document, *child, depth + 1, out);
        }
    }
    let mut out = String::new();
    walk(document, document.root, 0, &mut out);
    out
}

/// Prints computed styles as `NodeId(n) <tag>: [declarations]` lines.
pub fn dump_styles(document: &HTMLDocument, styles: &css::ComputedStyles) -> String {
    let mut out = String::new();
    for (id, declarations) in &styles.styles {
        let tag = match document
            .node(NodeId(id.index() as usize))
            .map(|node| &node.kind)
        {
            Some(NodeKind::Element(element)) => element.name.as_str(),
            _ => "?",
        };
        let list: Vec<String> = declarations
            .iter()
            .map(|d| format!("{}: {}", d.property, d.value))
            .collect();
        out.push_str(&format!("{id:?} <{tag}>: [{}]\n", list.join("; ")));
    }
    out
}

/// Builds a DOM from the hand-written `input.dom` format, without the HTML
/// parser.
///
/// One node per line; two spaces of indent per level. A line is either
/// `"text"` (a text node) or `tag name=value name="value with spaces"`.
pub fn parse_dom_data(source: &str) -> HTMLDocument {
    let mut document = HTMLDocument::new();
    let mut stack: Vec<NodeId> = vec![document.root];
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let indent = line.len() - line.trim_start().len();
        let depth = indent / 2 + 1;
        stack.truncate(depth);
        let parent = *stack.last().expect("indent jumps more than one level");
        let body = line.trim();
        let kind = if let Some(text) = body.strip_prefix('"') {
            NodeKind::Text(text.trim_end_matches('"').to_owned())
        } else {
            let (name, attributes) = parse_tag(body);
            NodeKind::Element(ElementData {
                name,
                namespace: Namespace::Html,
                attributes,
            })
        };
        let id = document.push_node(kind, None);
        document.append_child(parent, id);
        stack.push(id);
    }
    document
}

fn parse_tag(body: &str) -> (String, Vec<Attribute>) {
    let (name, mut rest) = body.split_once(' ').unwrap_or((body, ""));
    let mut attributes = Vec::new();
    while let Some((key, after)) = rest.trim_start().split_once('=') {
        let (value, tail) = if let Some(quoted) = after.strip_prefix('"') {
            let end = quoted.find('"').expect("unclosed attribute quote");
            (&quoted[..end], &quoted[end + 1..])
        } else {
            after.split_once(' ').unwrap_or((after, ""))
        };
        attributes.push(Attribute {
            name: key.to_owned(),
            value: value.to_owned(),
        });
        rest = tail;
    }
    (name.to_owned(), attributes)
}

/// Serves the raw bytes of a recorded HTTP response once on `127.0.0.1` and
/// returns the URL to fetch.
pub fn serve_once(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/", listener.local_addr().unwrap());
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = [0; 4096];
        let mut seen = Vec::new();
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).expect("read");
            if count == 0 {
                break;
            }
            seen.extend_from_slice(&buffer[..count]);
        }
        stream.write_all(&response).expect("write");
    });
    url
}

/// Reads a recorded response file and normalizes its line endings to CRLF,
/// so fixtures can be edited as normal text files. A `Content-Length` header
/// is added when the file has none.
pub fn read_http(case: &Path) -> Vec<u8> {
    let raw = read(case, "input.http");
    let (head, body) = raw.split_once("\n\n").unwrap_or((&raw, ""));
    let mut head = head.to_owned();
    if !head.to_ascii_lowercase().contains("content-length:") {
        head.push_str(&format!("\nContent-Length: {}", body.len()));
    }
    let mut bytes = head.replace('\n', "\r\n").into_bytes();
    bytes.extend_from_slice(b"\r\n\r\n");
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// Fetches `url` with a default controller and returns the response and body.
pub fn fetch(url: &str) -> Result<(net::Response, Vec<u8>), net::RequestError> {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let controller = net::RequestController::builder(net::Config::default()).build()?;
        let mut response = controller.fetch(net::Request::get(url)).await?;
        let body = std::mem::replace(&mut response.body, net::Body::from_bytes(Vec::new()));
        let bytes = body.bytes().await?;
        Ok((response, bytes))
    })
}
