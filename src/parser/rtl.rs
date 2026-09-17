use crate::bot::models::{InputRichMessage, RichBlock};

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
#[allow(dead_code)]
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

/// Checks if any RichBlock in the list contains RTL characters across all block variants.
pub fn blocks_contain_rtl(blocks: &[RichBlock]) -> bool {
    blocks.iter().any(|b| has_rtl_characters(&b.extract_text()))
}

/// Applies RTL direction to an InputRichMessage if either the provided text
/// or any of its contained rich blocks contain RTL content.
pub fn apply_rtl_direction(message: &mut InputRichMessage, text: &str) {
    if message.is_rtl.is_none() && (is_rtl_text(text) || blocks_contain_rtl(&message.blocks)) {
        message.is_rtl = Some(true);
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
}
