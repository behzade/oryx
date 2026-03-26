pub(crate) fn sanitize_path_component(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());

    for ch in input.chars() {
        let replacement = match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        };
        sanitized.push(replacement);
    }

    let mut sanitized = sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['.', ' '])
        .to_string();

    if sanitized.is_empty() {
        return "Untitled".to_string();
    }

    let stem_end = sanitized.find('.').unwrap_or(sanitized.len());
    if is_windows_reserved_name(&sanitized[..stem_end]) {
        sanitized.insert(stem_end, '_');
    }

    sanitized
}

fn is_windows_reserved_name(input: &str) -> bool {
    matches!(
        input.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(test)]
mod tests {
    use super::sanitize_path_component;

    #[test]
    fn sanitize_path_component_replaces_invalid_characters() {
        assert_eq!(sanitize_path_component("a/b:c*"), "a_b_c_");
    }

    #[test]
    fn sanitize_path_component_avoids_windows_reserved_names() {
        assert_eq!(sanitize_path_component("CON"), "CON_");
        assert_eq!(sanitize_path_component("nul.txt"), "nul_.txt");
        assert_eq!(sanitize_path_component(" Lpt1 "), "Lpt1_");
    }
}
