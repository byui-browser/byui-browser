type NodeID: int32;

/**
* {
    name: "meta",
    parent: 12,
    children: vec![],
    attributes: map!({
        "name": "Viewport",
        "content": "width=device-width, initial-scale=1.0"
    })
}
**/
struct HTMLElement {
    name: String,
    attributes: Map<String, String>,
    parent: NodeID,
    children: Vec[NodeID]
}

struct HTMLDocument {
    docType: String,
    nodes: Vec[NodeID],
    location: Location
}

struct Location {
    url: String
}

pub fn parse_raw_HTML(raw_string: String) -> HTMLDocument {
    //
}