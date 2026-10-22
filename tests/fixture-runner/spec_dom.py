# Prints the spec DOM of an HTML file (via html5lib) in the fixture-runner dump format.
# Use it to write a spec-correct expected.txt:
#   pip install html5lib && python3 tests/fixture-runner/spec_dom.py input.html > expected.txt
import sys, html5lib
from xml.dom import Node
def rs(s): return '"' + s.replace('\\','\\\\').replace('"','\\"').replace('\n','\\n') + '"'
def opt(v): return 'None' if v is None or v=='' else f'Some({rs(v)})'
def walk(n, d, out):
    p='  '*d
    t=n.nodeType
    if t==Node.DOCUMENT_NODE: out.append(p+'Document')
    elif t==Node.DOCUMENT_TYPE_NODE: out.append(p+f'Doctype {{ name: {opt(n.name)}, public_id: {opt(n.publicId)}, system_id: {opt(n.systemId)} }}')
    elif t==Node.ELEMENT_NODE:
        attrs=', '.join(f'Attribute {{ name: {rs(a.name)}, value: {rs(a.value)} }}' for a in [n.attributes.item(i) for i in range(n.attributes.length)])
        out.append(p+f'Element(ElementData {{ name: {rs(n.tagName)}, namespace: Html, attributes: [{attrs}] }})')
    elif t==Node.TEXT_NODE: out.append(p+f'Text({rs(n.data)})')
    elif t==Node.COMMENT_NODE: out.append(p+f'Comment({rs(n.data)})')
    for c in n.childNodes: walk(c,d+1,out)
doc=html5lib.parse(open(sys.argv[1]).read(), treebuilder='dom', namespaceHTMLElements=False)
doc.normalize(); out=[]; walk(doc,0,out); print('\n'.join(out))
