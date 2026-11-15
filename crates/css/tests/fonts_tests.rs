use css::{FontSource, font_face_rules, parse_font_family_list, parse_stylesheet};

#[test]
fn font_face_rules_parse_public_descriptors() {
    let sheet = parse_stylesheet(
        r#"@font-face {
            font-family: "Open Sans";
            src: local("Open Sans"), url("fonts/open.woff2") format("woff2");
            font-weight: 300 800;
            font-style: italic;
        }"#,
    );
    let rules = font_face_rules(&sheet);

    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].family, "Open Sans");
    assert_eq!(rules[0].weight, (300, 800));
    assert!(rules[0].italic);
    assert_eq!(rules[0].sources[0], FontSource::Local("Open Sans".into()));
    assert!(rules[0].sources[1].is_loadable_url());
}

#[test]
fn public_font_family_parser_normalizes_names() {
    assert_eq!(
        parse_font_family_list(r#" "My Font", Times   New Roman, serif"#),
        ["My Font", "Times New Roman", "serif"]
    );
    assert!(parse_font_family_list("inherit").is_empty());
}
