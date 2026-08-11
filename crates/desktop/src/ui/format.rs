//! Presentation formatting.
//!
//! The porting notes expected gpui to make digit grouping and relative-time
//! formatting moot. It doesn't — gpui is a UI framework, not an i18n library,
//! and there is no built-in for either. What gpui *did* delete is the
//! ANSI-width-aware machinery around them. These two survive, ported straight
//! across, and the CLI's rules are preserved exactly.

use jiff::Timestamp;

/// `50000` -> `"50,000"`.
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }

    out
}

/// `now` is a parameter rather than a call to the clock so this stays testable.
/// Unparseable input reads as "never", matching the CLI's behaviour on an empty
/// or malformed timestamp.
pub fn relative_time(iso: &str, now: Timestamp) -> String {
    if iso.is_empty() {
        return "never".into();
    }

    let Ok(then) = iso.parse::<Timestamp>() else {
        return "never".into();
    };

    let seconds = now.as_second() - then.as_second();
    if seconds < 60 {
        return "just now".into();
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }

    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }

    let days = hours / 24;
    if days < 30 {
        return format!("{days}d ago");
    }

    let months = days / 30;
    if months < 12 {
        return format!("{months}mo ago");
    }

    format!("{}y ago", months / 12)
}

/// GitHub's own language colours. The CLI approximated these in the 256-colour
/// cube because that was all a terminal offered; a GPU surface can just use the
/// real values.
pub fn language_color(language: &str) -> Option<u32> {
    let color = match language {
        "TypeScript" => 0x3178c6,
        "JavaScript" => 0xf1e05a,
        "Rust" => 0xdea584,
        "Go" => 0x00add8,
        "Python" => 0x3572a5,
        "Ruby" => 0x701516,
        "C" => 0x555555,
        "C++" => 0xf34b7d,
        "C#" => 0x178600,
        "Java" => 0xb07219,
        "Zig" => 0xec915c,
        "Shell" => 0x89e051,
        "HTML" => 0xe34c26,
        "CSS" => 0x663399,
        "Svelte" => 0xff3e00,
        "Vue" => 0x41b883,
        "Elixir" => 0x6e4a7e,
        "Haskell" => 0x5e5086,
        "Lua" => 0x000080,
        "Nix" => 0x7e7eff,
        "Swift" => 0xf05138,
        "Kotlin" => 0xa97bff,
        "Dart" => 0x00b4ab,
        "PHP" => 0x4f5d95,
        _ => return None,
    };

    Some(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(iso: &str) -> Timestamp {
        iso.parse().unwrap()
    }

    #[test]
    fn groups_digits_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(50_000), "50,000");
        assert_eq!(group_digits(123_456), "123,456");
        assert_eq!(group_digits(1_234_567), "1,234,567");
    }

    #[test]
    fn empty_and_malformed_timestamps_read_as_never() {
        let now = at("2024-05-01T12:00:00Z");
        assert_eq!(relative_time("", now), "never");
        assert_eq!(relative_time("not a date", now), "never");
    }

    #[test]
    fn walks_the_units_up() {
        let now = at("2024-05-01T12:00:00Z");

        assert_eq!(relative_time("2024-05-01T11:59:30Z", now), "just now");
        assert_eq!(relative_time("2024-05-01T11:30:00Z", now), "30m ago");
        assert_eq!(relative_time("2024-05-01T06:00:00Z", now), "6h ago");
        assert_eq!(relative_time("2024-04-26T12:00:00Z", now), "5d ago");
        assert_eq!(relative_time("2024-02-01T12:00:00Z", now), "3mo ago");
        // 1096 days -> 36 flat months -> 3 years, by the CLI's arithmetic.
        assert_eq!(relative_time("2021-05-01T12:00:00Z", now), "3y ago");
    }

    #[test]
    fn boundaries_match_the_cli() {
        let now = at("2024-05-01T12:00:00Z");

        // 59s is still "just now"; 60s becomes minutes.
        assert_eq!(relative_time("2024-05-01T11:59:01Z", now), "just now");
        assert_eq!(relative_time("2024-05-01T11:59:00Z", now), "1m ago");
        // The CLI's month is a flat 30 days, and its year is 12 of those.
        assert_eq!(relative_time("2024-04-02T12:00:00Z", now), "29d ago");
        assert_eq!(relative_time("2024-04-01T12:00:00Z", now), "1mo ago");
    }

    #[test]
    fn a_future_timestamp_reads_as_just_now_not_a_negative() {
        let now = at("2024-05-01T12:00:00Z");
        assert_eq!(relative_time("2025-01-01T00:00:00Z", now), "just now");
    }

    #[test]
    fn known_languages_get_a_colour_and_others_do_not() {
        assert_eq!(language_color("Rust"), Some(0xdea584));
        assert_eq!(language_color("TypeScript"), Some(0x3178c6));
        assert_eq!(language_color("Brainfuck"), None);
        assert_eq!(language_color(""), None);
    }
}
