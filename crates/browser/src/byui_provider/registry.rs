//! Compile-time page and resource registry for the `byui:` provider.

use net::Url;

/// One compile-time resource exposed by a [`PageDefinition`].
///
/// A resource may have multiple paths when the page needs compatibility
/// aliases, such as `/`, `/index.html`, and `/page` for its document.
#[derive(Clone, Copy)]
pub(super) struct ResourceDefinition {
    pub(super) paths: &'static [&'static str],
    pub(super) media_type: &'static str,
    pub(super) body: &'static [u8],
}

/// The route and resources belonging to one browser page.
#[derive(Clone, Copy)]
struct PageDefinition {
    host: &'static str,
    resources: &'static [ResourceDefinition],
}

static WELCOME_RESOURCES: &[ResourceDefinition] = &[
    ResourceDefinition {
        paths: &["", "/", "/index.html", "/page"],
        media_type: "text/html; charset=utf-8",
        body: include_bytes!("../../../../browser-pages/welcome/index.html"),
    },
    ResourceDefinition {
        paths: &["/style.css"],
        media_type: "text/css; charset=utf-8",
        body: include_bytes!("../../../../browser-pages/welcome/style.css"),
    },
    ResourceDefinition {
        paths: &["/script.js"],
        media_type: "text/javascript; charset=utf-8",
        body: include_bytes!("../../../../browser-pages/welcome/script.js"),
    },
];

static PAGES: &[PageDefinition] = &[PageDefinition {
    host: "welcome",
    resources: WELCOME_RESOURCES,
}];

pub(super) fn find_resource(url: &Url) -> Option<&'static ResourceDefinition> {
    let host = url.host_str()?;
    PAGES
        .iter()
        .find(|page| page.host == host)
        .and_then(|page| {
            page.resources
                .iter()
                .find(|resource| resource.paths.contains(&url.path()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_document_routes_resolve_to_html() {
        for path in ["", "/", "/index.html", "/page"] {
            let url = Url::parse(&format!("byui://welcome{path}")).unwrap();
            let resource = find_resource(&url).expect("welcome document route should resolve");

            assert_eq!(resource.media_type, "text/html; charset=utf-8");
            assert_eq!(
                resource.body,
                include_bytes!("../../../../browser-pages/welcome/index.html")
            );
        }
    }

    #[test]
    fn welcome_stylesheet_route_resolves_to_css() {
        let url = Url::parse("byui://welcome/style.css").unwrap();
        let resource = find_resource(&url).expect("welcome stylesheet route should resolve");

        assert_eq!(resource.media_type, "text/css; charset=utf-8");
        assert_eq!(
            resource.body,
            include_bytes!("../../../../browser-pages/welcome/style.css")
        );
    }

    #[test]
    fn welcome_script_route_resolves_to_javascript() {
        let url = Url::parse("byui://welcome/script.js").unwrap();
        let resource = find_resource(&url).expect("welcome script route should resolve");

        assert_eq!(resource.media_type, "text/javascript; charset=utf-8");
        assert_eq!(
            resource.body,
            include_bytes!("../../../../browser-pages/welcome/script.js")
        );
    }

    #[test]
    fn registry_rejects_unknown_pages_and_paths() {
        assert!(find_resource(&Url::parse("byui://settings/page").unwrap()).is_none());
        assert!(find_resource(&Url::parse("byui://welcome/missing").unwrap()).is_none());
    }
}
