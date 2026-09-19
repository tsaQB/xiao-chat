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

/// Regex to detect AMS dots commands (\dotsb, \dotsm, \dotsi, \dotsc, \dotso),
/// generic \dots, Unicode ellipsis … (\u{2026}), and ASCII consecutive dots (...).
static RE_ALL_DOTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\(?:dots[bmic]?|dotso)\b|…|\.{3,}").expect("valid static regex")
});

/// TeX binary operator and relation commands that determine centered ellipsis context (\cdots).
const BIN_REL_COMMANDS: &[&str] = &[
    r"\times", r"\cdot", r"\pm", r"\mp", r"\div", r"\oplus", r"\otimes", r"\odot", r"\wedge",
    r"\vee", r"\cap", r"\cup", r"\approx", r"\equiv", r"\sim", r"\simeq", r"\le", r"\leq", r"\ge",
    r"\geq", r"\ll", r"\gg", r"\ne", r"\neq", r"\propto",
];

/// Checks if a slice ending before dots terminates in a binary operator or relation.
fn is_binary_or_relation_preceding(s: &str) -> bool {
    let s = s.trim_end();
    if s.is_empty() {
        return false;
    }
    if let Some(ch) = s.chars().last() {
        if matches!(ch, '+' | '-' | '*' | '/' | '=' | '<' | '>' | '~') {
            return true;
        }
    }
    for &cmd in BIN_REL_COMMANDS {
        if s.ends_with(cmd) {
            return true;
        }
    }
    false
}

/// Checks if a slice starting after dots begins with a binary operator or relation.
fn is_binary_or_relation_following(s: &str) -> bool {
    let s = s.trim_start();
    if s.is_empty() {
        return false;
    }
    if let Some(ch) = s.chars().next() {
        if matches!(ch, '+' | '-' | '*' | '/' | '=' | '<' | '>' | '~') {
            return true;
        }
    }
    for &cmd in BIN_REL_COMMANDS {
        if let Some(rest) = s.strip_prefix(cmd) {
            if rest.chars().next().is_none_or(|c| !c.is_ascii_alphabetic()) {
                return true;
            }
        }
    }
    false
}

/// Normalizes dots (\dots, \dotsb, \dotsm, \dotsi, \dotsc, \dotso, …, ...) to either
/// \cdots (between binary operators / relations) or \ldots (in comma lists and sets)
/// ensuring full compatibility with Telegram iOS SwiftMath (which only supports \ldots and \cdots).
fn sanitize_dots_for_telegram(input: &str) -> String {
    RE_ALL_DOTS
        .replace_all(input, |caps: &regex::Captures| {
            let m = caps.get(0).expect("full match capture exists");
            let matched = m.as_str();

            let base_repl = match matched {
                r"\dotsb" | r"\dotsm" | r"\dotsi" => r"\cdots",
                r"\dotsc" | r"\dotso" => r"\ldots",
                _ => {
                    let before = input[..m.start()].trim_end();
                    let after = input[m.end()..].trim_start();

                    if before.ends_with(',') || after.starts_with(',') {
                        r"\ldots"
                    } else if is_binary_or_relation_preceding(before)
                        || is_binary_or_relation_following(after)
                    {
                        r"\cdots"
                    } else {
                        r"\ldots"
                    }
                }
            };

            let needs_space = input[m.end()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic());

            if needs_space {
                format!("{base_repl} ")
            } else {
                base_repl.to_string()
            }
        })
        .into_owned()
}

/// Normalizes internal whitespace of a math roman segment into TeX escaped backslash-spaces.
fn escape_math_roman_text(inner: &str) -> String {
    inner.split_whitespace().collect::<Vec<_>>().join(r"\ ")
}

/// Sanitizes a LaTeX mathematical expression so that it renders reliably across all
/// Telegram clients, specifically preventing `ParseException` and blank/truncated cards
/// in Telegram Android's `JLaTeXMath` engine and raw LaTeX fallback in Telegram iOS's
/// `SwiftMath` engine while preserving full mathematical fidelity.
///
/// Transformations applied:
/// 1. Converts Indonesian/European decimal commas between digits (e.g. `7,5` -> `7.5`),
///    while preserving coordinate/set pairs like `(1,2)` or `{1,2}`.
/// 2. Converts numbers with text units (`10\text{cm}`) into properly spaced math roman (`10\ \mathrm{cm}`).
/// 3. Normalizes standalone `\text{...}` and `\mbox{...}` to `\mathrm{...}` with escaped whitespace.
/// 4. Ensures binary operators following math roman words have clean spacing (`-7` -> `- 7`).
/// 5. Normalizes all ellipsis commands (`\dots`, `\dotsb`, `…`, `...`) to `\cdots` or `\ldots` for SwiftMath.
/// 6. Replaces raw non-breaking spaces `~` with TeX standard `\ `.
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

    // 6. Normalize dots for cross-platform iOS (SwiftMath) and Android (JLaTeXMath) rendering
    let dots_sanitized = sanitize_dots_for_telegram(&hyphens_spaced);

    dots_sanitized.trim().to_string()
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

    #[test]
    fn test_dots_sanitization_for_ios_compatibility() {
        let s1 = sanitize_latex_for_telegram(r"\sum_{i=1}^n i = 1 + 2 + \dots + n");
        assert_eq!(s1, r"\sum_{i=1}^n i = 1 + 2 + \cdots + n");

        let s2 = sanitize_latex_for_telegram(r"\prod_{i=1}^n i = 1 \times 2 \times \dots \times n");
        assert_eq!(s2, r"\prod_{i=1}^n i = 1 \times 2 \times \cdots \times n");

        let s3 = sanitize_latex_for_telegram(r"\mathbb{N} = \{1, 2, 3, 4, \dots\}");
        assert_eq!(s3, r"\mathbb{N} = \{1, 2, 3, 4, \ldots\}");

        let s4 = sanitize_latex_for_telegram(r"\mathbb{Z} = \{\dots, -2, -1, 0, 1, 2, \dots\}");
        assert_eq!(s4, r"\mathbb{Z} = \{\ldots, -2, -1, 0, 1, 2, \ldots\}");

        let s5 = sanitize_latex_for_telegram(r"\{2, 3, 5, 7, 11, \dots\}");
        assert_eq!(s5, r"\{2, 3, 5, 7, 11, \ldots\}");

        // AMS dot variants
        assert_eq!(
            sanitize_latex_for_telegram(r"1 + \dotsb + n"),
            r"1 + \cdots + n"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"1 \times \dotsm \times n"),
            r"1 \times \cdots \times n"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"a_1 \dotsm a_n"),
            r"a_1 \cdots a_n"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"\int \dotsi \int"),
            r"\int \cdots \int"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"x_1, \dotsc, x_n"),
            r"x_1, \ldots, x_n"
        );
        assert_eq!(sanitize_latex_for_telegram(r"(\dotso)"), r"(\ldots)");

        // Unicode ellipsis …
        assert_eq!(
            sanitize_latex_for_telegram(r"1 + 2 + … + n"),
            r"1 + 2 + \cdots + n"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"\{1, 2, 3, …\}"),
            r"\{1, 2, 3, \ldots\}"
        );

        // ASCII ...
        assert_eq!(
            sanitize_latex_for_telegram(r"1 + 2 + ... + n"),
            r"1 + 2 + \cdots + n"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"\{1, 2, 3, ...\}"),
            r"\{1, 2, 3, \ldots\}"
        );

        // Relations
        assert_eq!(
            sanitize_latex_for_telegram(r"x_1 = x_2 = \dots = x_n"),
            r"x_1 = x_2 = \cdots = x_n"
        );
        assert_eq!(
            sanitize_latex_for_telegram(r"x_1 < x_2 < \dots < x_n"),
            r"x_1 < x_2 < \cdots < x_n"
        );
    }

    #[test]
    fn test_markdown_table_math_sanitization_integration() {
        let md_table = r#"| Simbol | Nama / Arti | Contoh / Keterangan |
| :---: | :--- | :--- |
| $\mathbb{N}$ | Bilangan Asli (*Natural Numbers*) | $\mathbb{N} = \{1, 2, 3, 4, \dots\}$ |
| $\mathbb{Z}$ | Bilangan Bulat (*Integers*) | $\mathbb{Z} = \{\dots, -2, -1, 0, 1, 2, \dots\}$ |
| $\mathbb{P}$ | Bilangan Prima (*Prime Numbers*) | $\{2, 3, 5, 7, 11, \dots\}$ |
"#;
        let blocks = crate::parser::markdown::parse_markdown_to_rich_blocks(md_table);
        assert!(!blocks.is_empty());
        let json = serde_json::to_string(&blocks).expect("table blocks serialize to json");
        // Ensure \dots was completely eliminated and replaced by \ldots in the AST
        assert!(!json.contains(r"\\dots"));
        assert!(json.contains(r"\\ldots"));
    }
}
