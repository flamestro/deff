pub(crate) fn normalized_char_count(value: &str) -> usize {
    value.chars().count()
}

pub(crate) fn slice_chars(value: &str, start: usize, len: usize) -> String {
    if len == 0 {
        return String::new();
    }

    value.chars().skip(start).take(len).collect()
}

pub(crate) fn truncate_to_width(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    if normalized_char_count(value) <= width {
        return value.to_string();
    }

    if width <= 3 {
        return value.chars().take(width).collect();
    }

    let mut truncated: String = value.chars().take(width - 3).collect();
    truncated.push_str("...");
    truncated
}

pub(crate) fn pad_to_width(value: String, width: usize) -> String {
    let len = normalized_char_count(&value);
    if len >= width {
        value.chars().take(width).collect()
    } else {
        format!("{value}{}", " ".repeat(width - len))
    }
}

pub(crate) fn fit_line(value: &str, width: usize) -> String {
    let truncated = truncate_to_width(value, width);
    pad_to_width(truncated, width)
}

pub(crate) fn normalize_content(value: &str) -> String {
    value.replace('\t', "  ").replace('\r', "")
}

pub(crate) fn get_max_normalized_line_length(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| normalized_char_count(&normalize_content(line)))
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{fit_line, normalize_content, truncate_to_width};

    #[test]
    fn truncate_adds_ellipsis_for_long_values() {
        assert_eq!(truncate_to_width("abcdefgh", 6), "abc...");
    }

    #[test]
    fn fit_line_pads_short_values() {
        assert_eq!(fit_line("abc", 5), "abc  ");
    }

    #[test]
    fn normalize_content_expands_tabs_and_cr() {
        assert_eq!(normalize_content("a\tb\r"), "a  b");
    }
}
