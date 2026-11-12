# Adding a `byui:` page

`browser::byui_provider` serves browser-owned resources through the Fetch
engine's `byui:` scheme. Resources are embedded at compile time, so the
provider does not depend on the process working directory or a runtime asset
directory.

## Registry structure

The provider is split across three Rust source files and this guide:

- `crates/browser/src/byui_provider/mod.rs` exposes the module and its public
  provider API.
- `crates/browser/src/byui_provider/provider.rs` implements `PageProvider` and
  converts registry entries into `SchemeResource` values.
- `crates/browser/src/byui_provider/registry.rs` owns page and resource
  definitions.
- `crates/browser/src/byui_provider/byui-provider.md` is the page-authoring
  guide and lives beside the implementation so it stays discoverable.

The registry has two static layers:

1. A `PageDefinition` maps a `byui:` host to its resources.
2. Each `ResourceDefinition` maps one or more URL paths to embedded bytes and
   a media type.

The provider selects a page from the URL host and then selects a resource from
the URL path. Query strings and fragments do not affect resource selection.
Every successful result is converted into a `SchemeResource`, allowing `net`
to perform metadata validation and public response filtering consistently.

## Adding a page

For a new page named `settings`:

1. Add its assets under `browser-pages/settings/`.
2. Add a resource list beside `WELCOME_RESOURCES` in
   `src/byui_provider/registry.rs`:

   ```rust
   static SETTINGS_RESOURCES: &[ResourceDefinition] = &[
       ResourceDefinition {
           paths: &["", "/", "/index.html", "/page"],
           media_type: "text/html; charset=utf-8",
           body: include_bytes!("../../../../browser-pages/settings/index.html"),
       },
       ResourceDefinition {
           paths: &["/style.css"],
           media_type: "text/css; charset=utf-8",
           body: include_bytes!("../../../../browser-pages/settings/style.css"),
       },
   ];
   ```

3. Register the page in `PAGES` in `src/byui_provider/registry.rs`:

   ```rust
   static PAGES: &[PageDefinition] = &[
       PageDefinition {
           host: "welcome",
           resources: WELCOME_RESOURCES,
       },
       PageDefinition {
           host: "settings",
           resources: SETTINGS_RESOURCES,
       },
   ];
   ```

The resulting routes are, for example, `byui://settings/page` and
`byui://settings/style.css`.

## Conventions

- Use a lowercase, stable host name for each page.
- Include `/page` as the canonical document route; `/`, the empty path, and
  `/index.html` are compatibility aliases when appropriate.
- Set the exact media type, including charset when the asset is text.
- Keep all page assets compile-time embedded with `include_bytes!`.
- Add route-lookup tests for the document and each subresource.
- Return `ResourceMissing` for unknown hosts or paths; do not fall back to
  HTTP, filesystem access, or another page.

The browser page loader may choose which resources to request, but routing and
resource ownership belong here. This keeps future page additions declarative
and prevents the provider's dispatch method from becoming a growing list of
page-specific conditionals.
