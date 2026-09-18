use regex::Regex;
use std::sync::LazyLock;

/// Regex to convert decimal commas between digits (e.g. "7,5" -> "7.5", "3,14" -> "3.14")
/// to avoid JLaTeXMath parsing errors and awkward spacing in TeX math mode.
static RE_DECIMAL_COMMA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<before>\d+),(?P<after>\d+)").expect("valid static regex"));

/// Regex to detect digits glued to (or followed by) text/mathrm/mbox units, e.g.:
/// "10\text{cm}", "50\text{cm}", "44\text{cm}", "7.5\text{hari}", "100 \text{ m}", "10\,\text{cm}"
static RE_NUMBER_UNIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<num>\d+(?:\.\d+)?)(?:\\ |\s|\\\,)*(?:\\(?:text|mathrm|mbox))\{(?P<inner>[^{}]+)\}",
    )
    .expect("valid static regex")
});

/// Regex to detect standalone \text{...} or \mbox{...} commands that are not preceded by a number.
static RE_STANDALONE_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\(?:text|mbox)\{(?P<inner>[^{}]+)\}").expect("valid static regex")
});

/// Regex to ensure proper binary operator spacing when a hyphen immediately follows \mathrm{...}
/// e.g. "\mathrm{Suku\ ke}-7" -> "\mathrm{Suku\ ke} - 7"
static RE_HYPHEN_AFTER_MATHROMAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\\mathrm\{[^{}]+\})-(?P<after>\d+)").expect("valid static regex")
});

/// Normalizes internal whitespace of a math roman segment into TeX escaped backslash-spaces.
fn escape_math_roman_text(inner: &str) -> String {
    inner.split_whitespace().collect::<Vec<_>>().join(r"\ ")
}

/// Sanitizes a LaTeX mathematical expression so that it renders reliably across all
/// Telegram clients, specifically preventing `ParseException` and blank/truncated cards
/// in Telegram Android's `JLaTeXMath` engine while preserving full fidelity on iOS and Desktop.
///
/// Transformations applied:
/// 1. Converts Indonesian/European decimal commas between digits (e.g. `7,5` -> `7.5`),
///    while preserving coordinate/set pairs like `(1,2)` or `{1,2}`.
/// 2. Converts numbers with text units (`10\text{cm}`) into properly spaced math roman (`10\ \mathrm{cm}`).
/// 3. Normalizes standalone `\text{...}` and `\mbox{...}` to `\mathrm{...}` with escaped whitespace.
/// 4. Ensures binary operators following math roman words have clean spacing (`-7` -> `- 7`).
/// 5. Replaces raw non-breaking spaces `~` with TeX standard `\ `.
pub fn sanitize_latex_for_telegram(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 1. Convert decimal commas (e.g., "7,5" -> "7.5", "3,14" -> "3.14"), skipping coordinate pairs
    let commas_normalized = RE_DECIMAL_COMMA.replace_all(trimmed, |caps: &regex::Captures| {
        let m = caps.get(0).expect("full match capture exists");
        let start = m.start();
        if start > 0 {
            let prev_char = trimmed[..start].chars().last();
            if matches!(prev_char, Some('(' | '[' | '{')) {
                return m.as_str().to_string();
            }
        }
        let end = m.end();
        if end < trimmed.len() {
            let next_char = trimmed[end..].chars().next();
            if matches!(next_char, Some(')' | ']' | '}'))
                && caps["before"].len() <= 2
                && caps["after"].len() <= 2
            {
                return m.as_str().to_string();
            }
        }
        format!("{}.{}", &caps["before"], &caps["after"])
    });

    // 2. Replace raw tildes used as spaces with explicit backslash-spaces
    let tildes_spaced = commas_normalized.replace('~', r"\ ");

    // 3. Convert numbers with units to math roman with explicit space: "10\ \mathrm{cm}"
    let units_converted = RE_NUMBER_UNIT.replace_all(&tildes_spaced, |caps: &regex::Captures| {
        let num = &caps["num"];
        let clean_inner = escape_math_roman_text(&caps["inner"]);
        format!(r"{num}\ \mathrm{{{clean_inner}}}")
    });

    // 4. Convert standalone \text{...} or \mbox{...} to \mathrm{...} with escaped spaces
    let text_normalized =
        RE_STANDALONE_TEXT.replace_all(&units_converted, |caps: &regex::Captures| {
            let clean_inner = escape_math_roman_text(&caps["inner"]);
            format!(r"\mathrm{{{clean_inner}}}")
        });

    // 5. Ensure spacing between math roman and trailing hyphen/minus
    let hyphens_spaced = RE_HYPHEN_AFTER_MATHROMAN.replace_all(&text_normalized, "$1 - $after");

    hyphens_spaced.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_pythagoras_expressions() {
        let line1 = r"c = \sqrt{a^2 + b^2} = \sqrt{6^2 + 8^2}";
        assert_eq!(sanitize_latex_for_telegram(line1), line1);

        let line2 = r"= \sqrt{36 + 64} = \sqrt{100} = 10\text{cm}";
        assert_eq!(
            sanitize_latex_for_telegram(line2),
            r"= \sqrt{36 + 64} = \sqrt{100} = 10\ \mathrm{cm}"
        );
    }

    #[test]
    fn sanitizes_trapesium_expressions() {
        let line1 = r"L = \frac{1}{2} \times (a+b) \times t";
        assert_eq!(sanitize_latex_for_telegram(line1), line1);

        let line2 = r"= \frac{1}{2} \times (8+12) \times 5";
        assert_eq!(sanitize_latex_for_telegram(line2), line2);

        let line3 = r"= \frac{1}{2} \times 20 \times 5 = 50\text{cm}";
        assert_eq!(
            sanitize_latex_for_telegram(line3),
            r"= \frac{1}{2} \times 20 \times 5 = 50\ \mathrm{cm}"
        );
    }

    #[test]
    fn sanitizes_decimal_comma_and_unit() {
        let line = r"x = \frac{60}{8} = 7,5\text{hari}";
        assert_eq!(
            sanitize_latex_for_telegram(line),
            r"x = \frac{60}{8} = 7.5\ \mathrm{hari}"
        );
    }

    #[test]
    fn sanitizes_circle_circumference() {
        let line_rumus = r"K = 2\pi r \quad \text{atau} \quad K = \pi d";
        assert_eq!(
            sanitize_latex_for_telegram(line_rumus),
            r"K = 2\pi r \quad \mathrm{atau} \quad K = \pi d"
        );

        let line_contoh = r"K = 2 \times \frac{22}{7} \times 7 = 2 \times 22 = 44\text{cm}";
        assert_eq!(
            sanitize_latex_for_telegram(line_contoh),
            r"K = 2 \times \frac{22}{7} \times 7 = 2 \times 22 = 44\ \mathrm{cm}"
        );
    }

    #[test]
    fn sanitizes_number_pattern_with_spaces() {
        let line = r"\text{Suku ke}-7 = 56";
        assert_eq!(
            sanitize_latex_for_telegram(line),
            r"\mathrm{Suku\ ke} - 7 = 56"
        );
    }

    #[test]
    fn sanitizes_superscript_units() {
        let line = r"V = 100\text{cm}^3";
        assert_eq!(sanitize_latex_for_telegram(line), r"V = 100\ \mathrm{cm}^3");
    }

    #[test]
    fn preserves_already_clean_latex() {
        let line = r"E = mc^2";
        assert_eq!(sanitize_latex_for_telegram(line), line);

        let coord = r"(1, 2, 3)";
        assert_eq!(sanitize_latex_for_telegram(coord), coord);

        let pair_no_space = r"(1,2)";
        assert_eq!(sanitize_latex_for_telegram(pair_no_space), pair_no_space);
    }

    #[test]
    fn handles_empty_or_whitespace() {
        assert_eq!(sanitize_latex_for_telegram(""), "");
        assert_eq!(sanitize_latex_for_telegram("   "), "");
    }
}
