use crate::bot::models::{InputRichMessage, RichBlock};
use regex::Regex;
use std::sync::LazyLock;

/// Returns true if the character belongs to a Right-to-Left (RTL) script
/// (Arabic, Hebrew, Syriac, Thaana, Samaritan, Mandaic, etc.)
/// or Eastern Arabic / Hindi digits.
pub fn is_rtl_char(c: char) -> bool {
    matches!(
        c,
        '\u{0590}'..='\u{05FF}' // Hebrew
        | '\u{0600}'..='\u{06FF}' // Arabic, Arabic-Indic digits (٠..٩)
        | '\u{0700}'..='\u{074F}' // Syriac
        | '\u{0750}'..='\u{077F}' // Arabic Supplement
        | '\u{0780}'..='\u{07BF}' // Thaana
        | '\u{07C0}'..='\u{07FF}' // N'Ko
        | '\u{0800}'..='\u{083F}' // Samaritan
        | '\u{0840}'..='\u{085F}' // Mandaic
        | '\u{0860}'..='\u{086F}' // Syriac Supplement
        | '\u{0870}'..='\u{089F}' // Arabic Extended-B
        | '\u{08A0}'..='\u{08FF}' // Arabic Extended-A
        | '\u{FB1D}'..='\u{FB4F}' // Hebrew Presentation Forms
        | '\u{FB50}'..='\u{FDFF}' // Arabic Presentation Forms-A
        | '\u{FE70}'..='\u{FEFF}' // Arabic Presentation Forms-B
        | '\u{10800}'..='\u{10FFF}' // Ancient / regional RTL scripts (Aramaic, Phoenician, etc.)
    )
}

/// Returns true if the character is an Eastern Arabic-Indic (Hindi) digit (٠..٩)
/// or Extended Arabic-Indic (Persian/Urdu) digit (۰..۹).
pub fn is_eastern_arabic_digit(c: char) -> bool {
    matches!(c, '\u{0660}'..='\u{0669}' | '\u{06F0}'..='\u{06F9}')
}

/// Returns true if the character is a strong Left-to-Right (LTR) character
/// (Latin, Greek, Cyrillic, CJK, Hangul, etc.).
pub fn is_strong_ltr_char(c: char) -> bool {
    matches!(
        c,
        '\u{0041}'..='\u{005A}' // A-Z
        | '\u{0061}'..='\u{007A}' // a-z
        | '\u{00C0}'..='\u{024F}' // Latin Extended
        | '\u{0370}'..='\u{03FF}' // Greek
        | '\u{0400}'..='\u{04FF}' // Cyrillic
        | '\u{1100}'..='\u{11FF}' // Hangul Jamo
        | '\u{2E80}'..='\u{9FFF}' // CJK Radicals / Ideographs
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

/// Returns true if the string contains at least one RTL character or Eastern Arabic digit.
pub fn has_rtl_characters(text: &str) -> bool {
    text.chars()
        .any(|c| is_rtl_char(c) || is_eastern_arabic_digit(c))
}

/// Returns true if the string contains at least one Eastern Arabic-Indic (Hindi) digit.
#[cfg(test)]
pub fn has_eastern_arabic_digits(text: &str) -> bool {
    text.chars().any(is_eastern_arabic_digit)
}

/// Determines whether the given text should be rendered with RTL layout.
/// Evaluates:
/// 1. Count of strong RTL characters vs strong LTR characters.
/// 2. First strong directional character rule.
pub fn is_rtl_text(text: &str) -> bool {
    let mut rtl_count = 0usize;
    let mut ltr_count = 0usize;
    let mut first_strong_is_rtl: Option<bool> = None;

    for c in text.chars() {
        if is_rtl_char(c) {
            rtl_count += 1;
            if first_strong_is_rtl.is_none() {
                first_strong_is_rtl = Some(true);
            }
        } else if is_strong_ltr_char(c) {
            ltr_count += 1;
            if first_strong_is_rtl.is_none() {
                first_strong_is_rtl = Some(false);
            }
        }
    }

    if rtl_count == 0 {
        return false;
    }

    // Dominant RTL: RTL characters equal or outnumber LTR characters
    if rtl_count >= ltr_count {
        return true;
    }

    // First strong character is RTL and there is a meaningful amount of RTL content
    if first_strong_is_rtl == Some(true) && (rtl_count >= 5 || rtl_count * 3 >= ltr_count) {
        return true;
    }

    false
}

/// Converts ASCII digits '0'..'9' into Eastern Arabic-Indic digits '٠'..'٩'.
pub fn to_eastern_arabic_digits(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '0' => '٠',
            '1' => '١',
            '2' => '٢',
            '3' => '٣',
            '4' => '٤',
            '5' => '٥',
            '6' => '٦',
            '7' => '٧',
            '8' => '٨',
            '9' => '٩',
            other => other,
        })
        .collect()
}

/// Converts Eastern Arabic-Indic digits '٠'..'٩' into standard ASCII digits '0'..'9'.
pub fn from_eastern_arabic_digits(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '٠' => '0',
            '١' => '1',
            '٢' => '2',
            '٣' => '3',
            '٤' => '4',
            '٥' => '5',
            '٦' => '6',
            '٧' => '7',
            '٨' => '8',
            '٩' => '9',
            other => other,
        })
        .collect()
}

/// Extracts clean human-readable text from pseudo-math expressions that contain
/// natural language or RTL script erroneously wrapped in TeX/LaTeX commands
/// (such as `$$\text{...}$$` or `$$\mathrm{...}$$`).
pub fn extract_text_from_pseudo_math(input: &str) -> String {
    static RE_MATH_TEXT_WRAPPERS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\\(?:text|mathrm|mbox|textbf|mathbf|textit|mathit|underline)\{([^{}]+)\}")
            .expect("valid static regex")
    });

    let mut result = input.to_string();
    // Recursively unwrap nested wrappers like \mathbf{\text{...}}
    for _ in 0..3 {
        let unwrapped = RE_MATH_TEXT_WRAPPERS
            .replace_all(&result, "$1")
            .into_owned();
        if unwrapped == result {
            break;
        }
        result = unwrapped;
    }

    // Replace TeX escaped spaces and non-breaking spaces with normal spaces
    result = result.replace(r"\ ", " ");
    result = result.replace(r"\,", " ");
    result = result.replace(r"\quad", " ");
    result = result.replace(r"\qquad", " ");
    result = result.replace('~', " ");

    // Remove escaped TeX grouping braces: \{ and \} -> { and }
    result = result.replace(r"\{", "{").replace(r"\}", "}");

    result.trim().to_string()
}

/// Checks if a string begins with an RTL character (ignoring whitespace and leading punctuation).
pub fn is_arabic_or_rtl_leading(text: &str) -> bool {
    text.chars()
        .find(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .is_some_and(is_rtl_char)
}

/// Returns true if a text segment in an LTR message starts with an RTL script
/// but contains mixed content (both RTL characters and Latin/ASCII alphabetic text).
///
/// In this case, prepending a Left-to-Right Mark (`\u{200E}`) prevents text layout
/// engines (such as Android and iOS) from treating the entire paragraph as RTL,
/// which would otherwise flip word order, punctuation, and right-align explanations.
pub fn needs_lrm_prefix(text: &str, is_message_rtl: bool) -> bool {
    if is_message_rtl {
        return false;
    }
    if !is_arabic_or_rtl_leading(text) {
        return false;
    }
    text.chars().any(|c| c.is_ascii_alphabetic())
}

/// Prepends a Left-to-Right Mark (`\u{200E}`) if the text requires it.
pub fn ensure_lrm_if_needed(text: &str, is_message_rtl: bool) -> String {
    if needs_lrm_prefix(text, is_message_rtl) {
        format!("\u{200E}{text}")
    } else {
        text.to_string()
    }
}

/// Determines whether a table's headers are predominantly written in RTL script.
pub fn is_table_predominantly_rtl(headers: &[&str]) -> bool {
    if headers.is_empty() {
        return false;
    }
    let rtl_headers = headers
        .iter()
        .filter(|h| is_rtl_text(h) || is_arabic_or_rtl_leading(h))
        .count();
    rtl_headers * 2 >= headers.len()
}

/// Checks if any RichBlock in the list contains RTL characters across all block variants.
#[allow(dead_code)]
pub fn blocks_contain_rtl(blocks: &[RichBlock]) -> bool {
    blocks.iter().any(|b| has_rtl_characters(&b.extract_text()))
}

/// Applies RTL direction to an InputRichMessage ONLY if the content as a whole
/// is predominantly RTL (>50% Arabic/Hebrew or leading RTL).
///
/// Mixed messages (e.g. Indonesian or English explaining Arabic grammar) remain LTR (None)
/// so that Telegram's native Unicode BiDi engine positions Arabic quotes naturally
/// without right-aligning Indonesian headers, lists, or tables.
pub fn apply_rtl_direction(message: &mut InputRichMessage, text: &str) {
    if message.is_rtl.is_none() {
        let dominant_rtl = if !text.trim().is_empty() {
            is_rtl_text(text)
        } else {
            let combined: String = message.blocks.iter().map(|b| b.extract_text()).collect();
            is_rtl_text(&combined)
        };
        if dominant_rtl {
            message.is_rtl = Some(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn detects_arabic_script_as_rtl() {
        assert!(is_rtl_text("مرحبا بالعالم"));
        assert!(has_rtl_characters("مرحبا"));
        assert!(!has_eastern_arabic_digits("مرحبا"));
    }

    #[test]
    fn detects_hebrew_script_as_rtl() {
        assert!(is_rtl_text("שלום עולם"));
        assert!(has_rtl_characters("שלום"));
    }

    #[test]
    fn rejects_pure_latin_and_indonesian() {
        assert!(!is_rtl_text("Halo dunia apa kabar?"));
        assert!(!is_rtl_text("Hello world, this is English."));
        assert!(!has_rtl_characters("Hello world 12345!"));
    }

    #[test]
    fn detects_eastern_arabic_hindi_numerals() {
        assert!(has_eastern_arabic_digits("١٢٣٤٥"));
        assert!(has_eastern_arabic_digits("الرقم: ١"));
        assert!(has_rtl_characters("١٢٣٤٥"));
        assert!(!has_eastern_arabic_digits("12345"));
    }

    #[test]
    fn detects_table_with_arabic_headers_and_hindi_numerals() {
        let table_text = "| الرقم | الاسم |\n| --- | --- |\n| ١ | أحمد |\n| ٢ | فاطمة |";
        assert!(is_rtl_text(table_text));
        assert!(has_eastern_arabic_digits(table_text));
    }

    #[test]
    fn mixed_text_with_arabic_quote_remains_ltr() {
        let mixed = "The Arabic word for peace is salam (سلام). It is a common greeting.";
        assert!(!is_rtl_text(mixed));
        assert!(has_rtl_characters(mixed));
    }

    #[test]
    fn arabic_text_with_english_word_remains_rtl() {
        let arabic_dominant = "مرحبا بكم في عالم programming الحديث مع لغة Rust.";
        assert!(is_rtl_text(arabic_dominant));
    }

    #[test]
    fn blocks_contain_rtl_detects_rtl_in_quotes_and_lists() {
        let quote_block = RichBlock::BlockQuotation {
            blocks: vec![Value::String("السلام عليكم".to_string())],
        };
        assert!(blocks_contain_rtl(&[quote_block]));

        let list_block = RichBlock::List {
            items: vec![crate::bot::models::RichBlockListItem::bullet(vec![
                Value::String("عنصر أول".to_string()),
            ])],
        };
        assert!(blocks_contain_rtl(&[list_block]));

        let latin_block = RichBlock::Paragraph {
            text: Value::String("Pure English".to_string()),
        };
        assert!(!blocks_contain_rtl(&[latin_block]));
    }

    #[test]
    fn mixed_indonesian_lesson_with_arabic_example_keeps_is_rtl_none() {
        let mut msg = InputRichMessage {
            blocks: vec![
                RichBlock::SectionHeading {
                    text: Value::String("4. Contoh Analisis Kalimat Sederhana".to_string()),
                    level: 3,
                },
                RichBlock::BlockQuotation {
                    blocks: vec![serde_json::json!({
                        "type": "paragraph",
                        "text": "كَتَبَ التِّلْمِيْذُ الدَّرْسَ"
                    })],
                },
                RichBlock::Paragraph {
                    text: Value::String("Artinya: Murid itu menulis pelajaran.".to_string()),
                },
            ],
            ..Default::default()
        };
        let text =
            "Ilmu Nahwu adalah salah satu cabang tata bahasa Arab. Perhatikan kalimat: كَتَبَ التِّلْمِيذُ";
        apply_rtl_direction(&mut msg, text);
        assert_eq!(msg.is_rtl, None);
    }

    #[test]
    fn extracts_clean_text_from_arabic_pseudo_math() {
        let input = r"\text{لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ}";
        assert_eq!(
            extract_text_from_pseudo_math(input),
            "لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ"
        );

        let input_spaced = r"\text{لَا}\ \text{تَقْنَطُوا}";
        assert_eq!(extract_text_from_pseudo_math(input_spaced), "لَا تَقْنَطُوا");

        let input_nested = r"\mathbf{\text{لَا تَقْنَطُوا}}";
        assert_eq!(extract_text_from_pseudo_math(input_nested), "لَا تَقْنَطُوا");

        let raw_arabic = "لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ";
        assert_eq!(
            extract_text_from_pseudo_math(raw_arabic),
            "لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ"
        );
    }

    #[test]
    fn test_needs_lrm_prefix_for_mixed_content_in_ltr() {
        // Mixed bullet item: starts with Arabic, followed by Latin transliteration and Indonesian
        assert!(needs_lrm_prefix(
            "**يَا (Yā)**: Harf nidā' (huruf panggilan) mabni di atas sukun.",
            false
        ));
        assert!(needs_lrm_prefix(
            "**أَيُّ (Ayyu)**: Munāda mabni di atas dhammah.",
            false
        ));

        // Pure Arabic verse: NO Latin letters -> should NOT prepend LRM (remains pure RTL)
        assert!(!needs_lrm_prefix(
            "يَا أَيُّهَا الَّذِينَ آمَنُوا إِذَا تَدَايَنْتُمْ بِدَيْنٍ إِلَىٰ أَجَلٍ مُسَمًّى فَاكْتُبُوهُ",
            false
        ));

        // Pure Latin item: does NOT start with Arabic -> NO LRM
        assert!(!needs_lrm_prefix(
            "Fa (فَ): Rābiṭah li-jawāb asy-syarṭ (penghubung jawaban syarat).",
            false
        ));

        // When message is RTL: NO LRM
        assert!(!needs_lrm_prefix(
            "**يَا (Yā)**: Harf nidā' (huruf panggilan) mabni di atas sukun.",
            true
        ));
    }

    #[test]
    fn test_ensure_lrm_if_needed_prepends_character() {
        let mixed = "**يَا (Yā)**: Harf nidā'";
        assert_eq!(
            ensure_lrm_if_needed(mixed, false),
            format!("\u{200E}{mixed}")
        );

        let pure_arabic = "يَا أَيُّهَا الَّذِينَ آمَنُوا";
        assert_eq!(ensure_lrm_if_needed(pure_arabic, false), pure_arabic);
    }
}
