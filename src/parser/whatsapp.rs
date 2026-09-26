use regex::Regex;
use std::sync::LazyLock;

static RE_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^#{1,6}\s+(.+)$").expect("valid regex"));
static RE_BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*[\*\-]\s+(.+)$").expect("valid regex"));
static RE_BOLD_ITALIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*\*(.+?)\*\*\*|___(.+?)___").expect("valid regex"));
static RE_BOLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*|__(.+?)__").expect("valid regex"));
static RE_ITALIC_ASTERISK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\*([^\*\s](?:.*?[^\*\s])?)\*").expect("valid regex"));
static RE_ITALIC_UNDERSCORE: LazyLock<Regex> = LazyLock::new(|| {
    // Only match standalone words with underscores, not snake_case identifiers
    Regex::new(r"(?s)(^|\s)_([^_\s](?:.*?[^_\s])?)_($|[\s\.,!\?:;])").expect("valid regex")
});
static RE_STRIKE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"~~(.+?)~~").expect("valid regex"));

/// Mengonversi teks Markdown umum (CommonMark) menjadi format WhatsApp Markdown.
///
/// Aturan formatting WhatsApp:
/// - **Tebal**: `*teks*`
/// - *Miring*: `_teks_`
/// - ***Tebal & Miring***: `*_teks_*`
/// - ~Coret~: `~teks~`
/// - `Inline code`: `\`teks\``
/// - ```Blok kode```: ```` ```teks``` ````
pub fn format_for_whatsapp(input: &str) -> String {
    // 1. Ekstrak blok kode (fenced) dan inline code terlebih dahulu agar konten di dalamnya terlindungi
    let mut segments = Vec::new();
    let mut last_idx = 0;
    let code_pattern =
        Regex::new(r"(?s)```[a-zA-Z0-9_-]*\n?(.*?)```|`([^`\n]+)`").expect("valid regex");

    for mat in code_pattern.find_iter(input) {
        if mat.start() > last_idx {
            segments.push((false, &input[last_idx..mat.start()]));
        }
        segments.push((true, mat.as_str()));
        last_idx = mat.end();
    }
    if last_idx < input.len() {
        segments.push((false, &input[last_idx..]));
    }

    let mut output = String::new();

    for (is_code, text) in segments {
        if is_code {
            output.push_str(text);
        } else {
            let mut formatted = text.to_string();

            // 1. Header Markdown (# Header) diubah menjadi placeholder tebal
            formatted = RE_HEADER.replace_all(&formatted, "\x01$1\x01").to_string();

            // 2. Bullet list (* atau -) diubah menjadi bullet symbol (•)
            formatted = RE_BULLET.replace_all(&formatted, "• $1").to_string();

            // 3. Strikethrough (~~teks~~) diubah menjadi ~teks~
            formatted = RE_STRIKE.replace_all(&formatted, "~$1~").to_string();

            // 4. Bold + Italic (***teks***) diubah menjadi placeholder tebal+miring
            formatted = RE_BOLD_ITALIC
                .replace_all(&formatted, |caps: &regex::Captures| {
                    let m = caps
                        .get(1)
                        .or_else(|| caps.get(2))
                        .map(|v| v.as_str())
                        .unwrap_or("");
                    format!("\x01\x02{}\x02\x01", m)
                })
                .to_string();

            // 5. Bold (**teks** atau __teks__) diubah menjadi placeholder tebal
            formatted = RE_BOLD
                .replace_all(&formatted, |caps: &regex::Captures| {
                    let m = caps
                        .get(1)
                        .or_else(|| caps.get(2))
                        .map(|v| v.as_str())
                        .unwrap_or("");
                    format!("\x01{}\x01", m)
                })
                .to_string();

            // 6. Italic (*teks* atau _teks_) diubah menjadi placeholder miring
            formatted = RE_ITALIC_ASTERISK
                .replace_all(&formatted, "\x02$1\x02")
                .to_string();
            formatted = RE_ITALIC_UNDERSCORE
                .replace_all(&formatted, "$1\x02$2\x02$3")
                .to_string();

            // 7. Resolusi placeholder ke sintaks WhatsApp (* untuk tebal, _ untuk miring)
            formatted = formatted.replace('\x01', "*").replace('\x02', "_");

            output.push_str(&formatted);
        }
    }

    output
}

/// Memecah pesan yang melebihi batas karakter layar WhatsApp (default: 3.500 karakter)
/// secara cerdas berdasarkan batas baris/paragraf agar tidak terpotong di tengah kalimat.
pub fn chunk_whatsapp_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if current.len() + line.len() + 1 > max_len && !current.is_empty() {
            chunks.push(current.trim_end().to_string());
            current = String::new();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        chunks.push(current.trim_end().to_string());
    }

    if chunks.is_empty() {
        vec![text.to_string()]
    } else {
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bold_and_headers() {
        let input = "## Judul Utama\nIni **teks tebal** dan ini ~~teks coret~~.";
        let res = format_for_whatsapp(input);
        assert_eq!(res, "*Judul Utama*\nIni *teks tebal* dan ini ~teks coret~.");
    }

    #[test]
    fn test_format_italics_conversion() {
        let input = "Ini *teks miring* dan ini _miring lagi_.";
        let res = format_for_whatsapp(input);
        assert_eq!(res, "Ini _teks miring_ dan ini _miring lagi_.");
    }

    #[test]
    fn test_format_bold_and_italic_combined() {
        let input = "Ini ***teks tebal dan miring***.";
        let res = format_for_whatsapp(input);
        assert_eq!(res, "Ini *_teks tebal dan miring_*.");
    }

    #[test]
    fn test_preserve_code_blocks_and_inline() {
        let input =
            "Contoh kode:\n```rust\nlet x = **tidak_diubah**;\n```\nDan `inline *code*` di sini.";
        let res = format_for_whatsapp(input);
        assert!(res.contains("```rust\nlet x = **tidak_diubah**;\n```"));
        assert!(res.contains("`inline *code*`"));
    }

    #[test]
    fn test_preserve_snake_case() {
        let input = "Variabel user_session_id tidak boleh miring.";
        let res = format_for_whatsapp(input);
        assert_eq!(res, "Variabel user_session_id tidak boleh miring.");
    }

    #[test]
    fn test_chunking() {
        let long_text = "Baris 1\n".repeat(600);
        let chunks = chunk_whatsapp_message(&long_text, 1000);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 1000);
        }
    }
}
