//! Public integration tests for CSS values and color parsing.

use css::{
    CssColor, parse_color,
    values::{
        Length, LengthContext, LengthUnit, MEDIUM_FONT_SIZE, compute_font_size, format_px,
        parse_length, parse_number, parse_percentage,
    },
};

#[test]
fn parses_css_numbers() {
    assert_eq!(parse_number("12"), Some(12.0));
    assert_eq!(parse_number("-0.5"), Some(-0.5));
    assert_eq!(parse_number(".25"), Some(0.25));
    assert_eq!(parse_number("1e2"), Some(100.0));
    assert_eq!(parse_number(" 2.5 "), Some(2.5));
    assert_eq!(parse_number(""), None);
    assert_eq!(parse_number("12px"), None);
    assert_eq!(parse_number("NaN"), None);
}

#[test]
fn parses_supported_css_length_units() {
    let cases = [
        ("10px", LengthUnit::Px),
        ("2em", LengthUnit::Em),
        ("2rem", LengthUnit::Rem),
        ("2ex", LengthUnit::Ex),
        ("2ch", LengthUnit::Ch),
        ("1in", LengthUnit::In),
        ("1cm", LengthUnit::Cm),
        ("1mm", LengthUnit::Mm),
        ("1q", LengthUnit::Q),
        ("12pt", LengthUnit::Pt),
        ("1pc", LengthUnit::Pc),
        ("10vw", LengthUnit::Vw),
        ("10vh", LengthUnit::Vh),
        ("10vmin", LengthUnit::Vmin),
        ("10vmax", LengthUnit::Vmax),
    ];

    for (input, expected_unit) in cases {
        let length = parse_length(input).expect("supported length");
        assert_eq!(length.unit, expected_unit, "{input}");
    }
}

#[test]
fn handles_length_case_whitespace_and_unitless_zero() {
    assert_eq!(parse_length(" 1.5EM ").unwrap().unit, LengthUnit::Em);

    assert_eq!(
        parse_length("0"),
        Some(Length {
            value: 0.0,
            unit: LengthUnit::Px,
        })
    );

    assert_eq!(parse_length("5"), None);
    assert_eq!(parse_length("calc(10px + 2px)"), None);
    assert_eq!(parse_length("10%"), None);
}

#[test]
fn converts_absolute_and_relative_lengths_to_pixels() {
    let context = LengthContext {
        font_size: 20.0,
        root_font_size: 10.0,
        viewport_width: 800.0,
        viewport_height: 600.0,
    };

    let cases = [
        ("10px", 10.0),
        ("2em", 40.0),
        ("2rem", 20.0),
        ("2ex", 20.0),
        ("2ch", 20.0),
        ("1in", 96.0),
        ("12pt", 16.0),
        ("1pc", 16.0),
        ("10vw", 80.0),
        ("10vh", 60.0),
        ("10vmin", 60.0),
        ("10vmax", 80.0),
    ];

    for (input, expected) in cases {
        let actual = parse_length(input).expect("valid length").to_px(&context);
        assert!((actual - expected).abs() < 0.001, "{input}: {actual}");
    }
}

#[test]
fn identifies_viewport_relative_lengths() {
    for input in ["1vw", "1vh", "1vmin", "1vmax"] {
        assert!(
            parse_length(input)
                .expect("valid viewport length")
                .is_viewport_relative(),
            "{input}"
        );
    }

    for input in ["1px", "1em", "1rem", "1in"] {
        assert!(
            !parse_length(input)
                .expect("valid non-viewport length")
                .is_viewport_relative(),
            "{input}"
        );
    }
}

#[test]
fn parses_css_percentages() {
    assert_eq!(parse_percentage("50%"), Some(50.0));
    assert_eq!(parse_percentage(" -12.5% "), Some(-12.5));
    assert_eq!(parse_percentage(".25%"), Some(0.25));
    assert_eq!(parse_percentage("50"), None);
    assert_eq!(parse_percentage("abc%"), None);
}

#[test]
fn computes_font_sizes_from_lengths_percentages_and_keywords() {
    assert_eq!(MEDIUM_FONT_SIZE, 16.0);
    assert_eq!(compute_font_size("2em", 16.0, 16.0), Some(32.0));
    assert_eq!(compute_font_size("150%", 20.0, 16.0), Some(30.0));
    assert_eq!(compute_font_size("2rem", 20.0, 10.0), Some(20.0));
    assert_eq!(compute_font_size("large", 16.0, 16.0), Some(19.2));
    assert_eq!(compute_font_size("larger", 20.0, 16.0), Some(24.0));
    assert_eq!(compute_font_size("smaller", 24.0, 16.0), Some(20.0));
}

#[test]
fn rejects_invalid_font_sizes_and_unresolved_viewport_units() {
    assert_eq!(compute_font_size("-1px", 16.0, 16.0), None);
    assert_eq!(compute_font_size("10vw", 16.0, 16.0), None);
    assert_eq!(compute_font_size("unknown", 16.0, 16.0), None);
}

#[test]
fn formats_css_pixel_lengths() {
    assert_eq!(format_px(16.0), "16px");
    assert_eq!(format_px(21.44), "21.44px");
    assert_eq!(format_px(0.0), "0px");
}

#[test]
fn parses_hex_rgb_and_named_colors() {
    assert_eq!(
        parse_color("#abc"),
        Some(CssColor {
            r: 170,
            g: 187,
            b: 204,
            a: 255,
        })
    );

    assert_eq!(
        parse_color("#123456"),
        Some(CssColor {
            r: 18,
            g: 52,
            b: 86,
            a: 255,
        })
    );

    assert_eq!(
        parse_color("rgb(18, 52, 86)"),
        Some(CssColor {
            r: 18,
            g: 52,
            b: 86,
            a: 255,
        })
    );

    assert_eq!(
        parse_color("RED"),
        Some(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        })
    );

    assert_eq!(
        parse_color("rebeccapurple"),
        Some(CssColor {
            r: 102,
            g: 51,
            b: 153,
            a: 255,
        })
    );
}

#[test]
fn rejects_unsupported_or_invalid_colors() {
    assert_eq!(parse_color("transparent"), None);
    assert_eq!(parse_color("#12"), None);
    assert_eq!(parse_color("#ggg"), None);
    assert_eq!(parse_color("rgb(256, 0, 0)"), None);
    assert_eq!(parse_color("rgb(1, 2)"), None);
    assert_eq!(parse_color("not-a-color"), None);
}


#[test]
fn resolves_css_length_functions() {
    assert_eq!(
        css::values::resolve_length_function("calc(20px + 30px)").as_deref(),
        Some("50px")
    );
    assert_eq!(
        css::values::resolve_length_function("min(50px, 80px)").as_deref(),
        Some("50px")
    );
    assert_eq!(
        css::values::resolve_length_function("max(50px, 80px)").as_deref(),
        Some("80px")
    );
    assert_eq!(
        css::values::resolve_length_function("clamp(10px, 50px, 90px)").as_deref(),
        Some("50px")
    );
}
