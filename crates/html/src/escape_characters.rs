#[path = "html5_entities.rs"]
mod html5_entities;

/// Decodes HTML named and numeric character references in text or attribute values.
pub(crate) fn decode_html_entities(input: &str, in_attribute: bool) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_amp) = input[cursor..].find('&') {
        let amp = cursor + relative_amp;
        output.push_str(&input[cursor..amp]);
        let tail = &input[amp + 1..];
        if let Some((decoded, consumed)) = decode_reference(tail, in_attribute) {
            output.push_str(&decoded);
            cursor = amp + 1 + consumed;
        } else {
            output.push('&');
            cursor = amp + 1;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn decode_reference(input: &str, in_attribute: bool) -> Option<(String, usize)> {
    if let Some(rest) = input.strip_prefix('#') {
        let (radix, digits) = if let Some(hex) = rest.strip_prefix(['x', 'X']) {
            (16, hex)
        } else {
            (10, rest)
        };
        let digit_len = digits
            .bytes()
            .take_while(|byte| match radix {
                16 => byte.is_ascii_hexdigit(),
                _ => byte.is_ascii_digit(),
            })
            .count();
        if digit_len == 0 {
            return None;
        }
        // HTML replaces values beyond Unicode's range with U+FFFD. Saturating
        // here lets arbitrarily long digit sequences follow that rule too.
        let number = u32::from_str_radix(&digits[..digit_len], radix).unwrap_or(u32::MAX);
        let semicolon = digits.as_bytes().get(digit_len) == Some(&b';');
        let character = char::from_u32(numeric_character(number))?;
        let consumed = 1 + usize::from(radix == 16) + digit_len + usize::from(semicolon);
        return Some((character.to_string(), consumed));
    }

    let max_len = input.len().min(32);
    for end in (1..=max_len).rev() {
        if !input.is_char_boundary(end) {
            continue;
        }
        let key = &input[..end];
        if let Ok(index) =
            html5_entities::NAMED_REFERENCES.binary_search_by_key(&key, |(name, _)| name)
        {
            if key.ends_with(';') {
                return Some((html5_entities::NAMED_REFERENCES[index].1.to_owned(), end));
            }
            let next = input.as_bytes().get(end).copied();
            if in_attribute && next.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'=')
            {
                continue;
            }
            return Some((html5_entities::NAMED_REFERENCES[index].1.to_owned(), end));
        }
    }
    None
}

fn numeric_character(value: u32) -> u32 {
    const WINDOWS_1252: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x192, 0x201E, 0x2026, 0x2020, 0x2021, 0x2C6, 0x2030, 0x160, 0x2039,
        0x152, 0x8D, 0x17D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
        0x2DC, 0x2122, 0x161, 0x203A, 0x153, 0x9D, 0x17E, 0x178,
    ];
    let value = if (0x80..=0x9F).contains(&value) {
        WINDOWS_1252[(value - 0x80) as usize]
    } else {
        value
    };
    if value == 0 || value > 0x10FFFF || (0xD800..=0xDFFF).contains(&value) {
        0xFFFD
    } else {
        value
    }
}
