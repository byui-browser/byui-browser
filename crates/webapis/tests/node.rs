use html::parse_raw_html;
use js::{Realm, Value};
use webapis::{Document, register_document};

fn realm(markup: &str) -> Realm {
    let mut realm = Realm::new();
    register_document(
        &mut realm,
        Document::from_html_document(&parse_raw_html(markup.into())),
    )
    .unwrap();
    realm
}

fn assert_script(realm: &mut Realm, script: &str) {
    assert_eq!(
        realm.evaluate_script(script),
        Ok(Value::Boolean(true)),
        "{script}"
    );
}

#[test]
fn document_and_all_tree_nodes_inherit_node_and_event_target() {
    let mut realm = realm("<!doctype html><main id='p'>hello<!--comment--><b></b></main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p');
        document instanceof Node && document instanceof Document &&
        p instanceof Node && p instanceof Element &&
        p.firstChild instanceof Node && document.firstChild instanceof Node &&
        document instanceof EventTarget && p instanceof EventTarget &&
        document.appendChild === p.appendChild &&
        document.addEventListener === p.addEventListener &&
        document.nodeType === Node.DOCUMENT_NODE &&
        document.firstChild.nodeType === Node.DOCUMENT_TYPE_NODE &&
        p.nodeType === Node.ELEMENT_NODE && p.nodeName === 'MAIN' &&
        p.firstChild.nodeName === '#text' && p.firstChild.nextSibling.nodeName === '#comment'
    ",
    );
}

#[test]
fn node_is_abstract_and_receiver_checks_reject_forged_nodes() {
    let mut realm = realm("<main id='p'></main>");
    assert_script(&mut realm, "
        var abstract = false; var bad = false;
        try { new Node(); } catch (error) { abstract = error.name === 'TypeError'; }
        try { document.appendChild({__nodeId: 0}); } catch (error) { bad = error.name === 'TypeError'; }
        abstract && bad
    ");
}

#[test]
fn child_nodes_is_a_stable_live_list_and_preserves_wrapper_identity() {
    let mut realm = realm("<main id='p'>hi<b id='b'></b></main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p'); var list = p.childNodes;
        var text = list[0]; var b = document.getElementById('b');
        var initial = list === p.childNodes && list.length === 2 &&
            list.item(0) === text && list[1] === b && list.item(99) === null;
        p.removeChild(text);
        var live = list.length === 1 && list[0] === b && list[1] === undefined;
        p.insertBefore(text, b);
        initial && live && list[0] === text && text.parentNode === p &&
            text.parentElement === p && text.nextSibling === b &&
            b.previousSibling === text && p.lastChild === b &&
            document.ownerDocument === null && text.ownerDocument === document &&
            text.getRootNode() === document && document.isConnected
    ",
    );
}

#[test]
fn node_value_and_text_content_follow_node_kind_rules() {
    let mut realm = realm("<main id='p'>one<!--omit--><b>two</b></main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p'); var text = p.firstChild;
        var comment = text.nextSibling;
        var initial = p.textContent === 'onetwo' && p.nodeValue === null &&
            document.textContent === null && document.nodeValue === null;
        text.nodeValue = 'changed'; comment.textContent = 'new';
        var changed = text.data === 'changed' && comment.data === 'new' &&
            p.textContent === 'changedtwo';
        p.nodeValue = 'ignored'; document.textContent = 'ignored';
        p.textContent = 'replacement';
        initial && changed && p.childNodes.length === 1 &&
            p.firstChild.nodeValue === 'replacement' && text.parentNode === null
    ",
    );
}

#[test]
fn normalize_merges_text_without_changing_first_text_identity() {
    let mut realm = realm("<main id='p'></main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p');
        var a = document.createTextNode('a'); var empty = document.createTextNode('');
        var b = document.createTextNode('b'); var comment = document.createComment('x');
        var c = document.createTextNode('c');
        p.appendChild(a); p.appendChild(empty); p.appendChild(b);
        p.appendChild(comment); p.appendChild(c); p.normalize();
        p.childNodes.length === 3 && p.firstChild === a &&
            a.nodeValue === 'ab' && b.parentNode === null &&
            empty.parentNode === null && c.previousSibling === comment
    ",
    );
}

#[test]
fn clone_and_equality_copy_subtrees_without_copying_listeners() {
    let mut realm = realm("<main id='p'><b>hello</b></main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p'); var calls = 0;
        function onCheck(event) { calls = calls + 1; }
        p.addEventListener('check', onCheck);
        var shallow = p.cloneNode(); var deep = p.cloneNode(true);
        deep.dispatchEvent(Event('check'));
        shallow.childNodes.length === 0 && deep !== p && p.isEqualNode(deep) &&
            !p.isSameNode(deep) && p.isSameNode(p) && !p.isEqualNode(null) &&
            deep.ownerDocument === document && deep.parentNode === null &&
            deep.firstChild !== p.firstChild && calls === 0
    ",
    );
}

#[test]
fn document_clone_owns_its_nodes_and_keeps_shared_node_methods() {
    let mut realm = realm("<!doctype html><main id='p'>hello</main>");
    assert_script(
        &mut realm,
        "
        var copy = document.cloneNode(true);
        copy !== document && copy instanceof Node && document.isEqualNode(copy) &&
            copy.ownerDocument === null && copy.childNodes.length === 2 &&
            copy.lastChild.ownerDocument === copy && copy.appendChild === document.appendChild &&
            copy.lastChild.textContent === 'hello'
    ",
    );
}

#[test]
fn node_mutations_move_nodes_and_fail_without_changing_the_tree() {
    let mut realm = realm("<main id='p'><a id='a'></a><b id='b'></b></main>");
    assert_script(&mut realm, "
        var p = document.getElementById('p'); var a = document.getElementById('a');
        var b = document.getElementById('b'); var hierarchy = false; var notfound = false;
        try { a.appendChild(p); } catch(error) { hierarchy = error.name === 'HierarchyRequestError'; }
        try { document.removeChild(a); } catch(error) { notfound = error.name === 'NotFoundError'; }
        var unchanged = p.firstChild === a && p.lastChild === b;
        p.insertBefore(b, a);
        var moved = p.firstChild === b && p.lastChild === a;
        var replaced = p.replaceChild(b, a) === a;
        hierarchy && notfound && unchanged && moved && replaced &&
            p.childNodes.length === 1 && a.parentNode === null &&
            p.replaceChild(b, b) === b && p.insertBefore(b, b) === b &&
            b.contains(b) && p.contains(b) && !b.contains(p) &&
            document.compareDocumentPosition(b) === 20
    ");
}

#[test]
fn document_mutation_enforces_doctype_order_and_disallows_text() {
    let mut realm = realm("<!doctype html><main id='p'></main>");
    assert_script(&mut realm, "
        var doctype = document.firstChild; var main = document.lastChild;
        var order = false; var text = false;
        try { document.appendChild(doctype); } catch(error) { order = error.name === 'HierarchyRequestError'; }
        try { document.appendChild(document.createTextNode('bad')); } catch(error) { text = error.name === 'HierarchyRequestError'; }
        order && text && document.firstChild === doctype && document.lastChild === main
    ");
}

#[test]
fn namespace_and_base_uri_queries_use_the_associated_document() {
    let mut realm = realm("<main id='p' xmlns:example='urn:example'>text</main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p');
        p.lookupNamespaceURI(null) === 'http://www.w3.org/1999/xhtml' &&
            p.isDefaultNamespace('http://www.w3.org/1999/xhtml') &&
            p.lookupNamespaceURI('example') === 'urn:example' &&
            p.firstChild.lookupPrefix('urn:example') === 'example' &&
            document.baseURI === 'about:blank' && p.baseURI === document.baseURI &&
            p.lookupNamespaceURI('missing') === null && p.lookupPrefix('urn:missing') === null
    ",
    );
}

#[test]
fn cross_document_checks_do_not_confuse_equal_arena_indices() {
    let mut realm = realm("<main id='p'>hello</main>");
    assert_script(&mut realm, "
        var copy = document.cloneNode(true); var p = document.getElementById('p');
        var other = copy.lastChild; var rejected = false;
        try { p.appendChild(other); } catch(error) { rejected = error.name === 'NotSupportedError'; }
        rejected && p !== other && p.isEqualNode(other) && !p.contains(other) &&
            (p.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_DISCONNECTED) !== 0
    ");
}

#[test]
fn node_constants_are_read_only_and_real_prototype_methods_accept_other_nodes() {
    let mut realm = realm("<main id='p'><b id='b'></b></main>");
    assert_script(
        &mut realm,
        "
        var p = document.getElementById('p'); var b = document.getElementById('b');
        Node.ELEMENT_NODE = 99; Node.prototype.TEXT_NODE = 99;
        p.ELEMENT_NODE = 99;
        p.borrowed = Node.prototype.contains;
        var borrowed = p.borrowed(b);
        var invalid = false;
        var fake = {contains: Node.prototype.contains};
        try { fake.contains(b); } catch(error) { invalid = error.name === 'TypeError'; }
        borrowed && invalid && Node.ELEMENT_NODE === 1 &&
            Node.prototype.TEXT_NODE === 3 && p.ELEMENT_NODE === 1
    ",
    );
}

#[test]
fn document_and_event_target_constructors_preserve_inheritance() {
    let mut realm = realm("<main></main>");
    assert_script(
        &mut realm,
        "
        var doc = new Document(); var target = new EventTarget();
        var calls = 0;
        function listener(event) { calls = calls + 1; }
        target.addEventListener('check', listener);
        target.dispatchEvent(Event('check'));
        target.removeEventListener('check', listener);
        target.dispatchEvent(Event('check'));
        doc instanceof Document && doc instanceof Node && doc.ownerDocument === null &&
            doc.childNodes.length === 0 && doc.baseURI === 'about:blank' &&
            target instanceof EventTarget && !(target instanceof Node) && calls === 1
    ",
    );
}

#[test]
fn text_node_events_use_live_wrappers_and_detached_trees_do_not_reach_document() {
    let mut realm = realm("<main id='p'>hello</main>");
    assert_script(&mut realm, "
        var p = document.getElementById('p'); var text = p.firstChild;
        var parentSeen = false; var documentSeen = false; var docCalls = 0;
        function parentListener(event) { parentSeen = event.currentTarget === p && event.target === text; }
        function documentListener(event) { documentSeen = event.currentTarget === document; docCalls = docCalls + 1; }
        p.addEventListener('check', parentListener);
        document.addEventListener('check', documentListener);
        text.dispatchEvent(Event('check', {bubbles: true}));
        p.removeChild(text);
        text.dispatchEvent(Event('check', {bubbles: true}));
        parentSeen && documentSeen && docCalls === 1 && window.Node === Node &&
            p.lookupNamespaceURI('xml') === 'http://www.w3.org/XML/1998/namespace'
    ");
}

#[test]
fn registering_another_document_preserves_realm_node_interfaces() {
    let mut realm = realm("<main id='old'></main>");
    realm
        .evaluate_script("var OldNode = Node; var old = document.getElementById('old');")
        .unwrap();
    register_document(&mut realm, Document::new()).unwrap();
    assert_script(
        &mut realm,
        "OldNode === Node && old instanceof Node && document instanceof Node && old.ownerDocument !== document",
    );
}

#[test]
fn required_nullable_node_arguments_must_still_be_present() {
    let mut realm = realm("<main></main>");
    assert_script(&mut realm, "
        var containsError = false; var namespaceError = false; var equalityError = false;
        try { document.contains(); } catch(error) { containsError = error.name === 'TypeError'; }
        try { document.lookupNamespaceURI(); } catch(error) { namespaceError = error.name === 'TypeError'; }
        try { document.isEqualNode(); } catch(error) { equalityError = error.name === 'TypeError'; }
        containsError && namespaceError && equalityError && !document.contains(undefined)
    ");
}
