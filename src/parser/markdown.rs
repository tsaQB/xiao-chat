use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

use crate::bot::models::{
    InputMedia, InputRichMessage, InputRichMessageMedia, Location, RichBlock, RichBlockCaption,
    RichBlockListItem, RichBlockTableCell,
};
use crate::parser::latex::sanitize_latex_for_telegram;
use crate::parser::rtl;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum ParserError {
    InvalidCoordinate(String),
    InvalidTag(String),
    MediaValidation(String),
    MalformedHtml(String),
}

impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCoordinate(msg) => write!(f, "Invalid coordinate: {msg}"),
            Self::InvalidTag(msg) => write!(f, "Invalid tag: {msg}"),
            Self::MediaValidation(msg) => write!(f, "Media validation error: {msg}"),
            Self::MalformedHtml(msg) => write!(f, "Malformed HTML: {msg}"),
        }
    }
}

impl std::error::Error for ParserError {}

impl From<String> for ParserError {
    fn from(s: String) -> Self {
        Self::MediaValidation(s)
    }
}

static RE_HTML_SPOILER_TG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<tg-spoiler(?:\s+[^>]*)?>(.*?)</tg-spoiler>").expect("valid static regex")
});
static RE_HTML_SPOILER_SPAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<span\s+class=["']?(?:tg-)?spoiler["']?>(.*?)</span>"#)
        .expect("valid static regex")
});
static RE_HTML_STRIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:s|strike|del)(?:\s+[^>]*)?>(.*?)</(?:s|strike|del)>")
        .expect("valid static regex")
});
static RE_HTML_UNDERLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:u|ins)(?:\s+[^>]*)?>(.*?)</(?:u|ins)>").expect("valid static regex")
});
static RE_HTML_BOLD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:b|strong)(?:\s+[^>]*)?>(.*?)</(?:b|strong)>").expect("valid static regex")
});
static RE_HTML_ITALIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:i|em)(?:\s+[^>]*)?>(.*?)</(?:i|em)>").expect("valid static regex")
});
static RE_HTML_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<code(?:\s+[^>]*)?>(.*?)</code>").expect("valid static regex")
});
static RE_HTML_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\s+[^>]*href=["']([^"']+)["'][^>]*>(.*?)</a>"#)
        .expect("valid static regex")
});
static RE_HTML_BR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("valid static regex"));
static RE_HTML_LEAKED_TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:b|strong|i|em|s|strike|del|u|ins|code|pre|blockquote|a|tg-spoiler|span|p|div|mark|kbd)(?:\s+[^>]*)?>").expect("valid static regex")
});

fn try_format_standalone_logic_symbol(inner: &str) -> Option<&'static str> {
    let clean_cmd = inner.trim_end_matches(r"\ ").trim();
    match clean_cmd {
        r"\therefore" => Some("∴"),
        r"\because" => Some("∵"),
        _ => None,
    }
}

pub fn parse_inline(input_str: &str) -> Value {
    if input_str.is_empty() {
        return Value::String(String::new());
    }

    // Normalize well-formed inline HTML tags before cleaning leaked residual HTML
    let mut normalized = input_str.to_string();
    if normalized.contains("spoiler") || normalized.contains("<tg-spoiler") {
        normalized = RE_HTML_SPOILER_TG
            .replace_all(&normalized, "||$1||")
            .into_owned();
        normalized = RE_HTML_SPOILER_SPAN
            .replace_all(&normalized, "||$1||")
            .into_owned();
    }
    if normalized.contains("<s") || normalized.contains("<strike") || normalized.contains("<del") {
        normalized = RE_HTML_STRIKE
            .replace_all(&normalized, "~~$1~~")
            .into_owned();
    }
    if normalized.contains("<u") || normalized.contains("<ins") {
        normalized = RE_HTML_UNDERLINE
            .replace_all(&normalized, "++${1}++")
            .into_owned();
    }
    if normalized.contains("<b") || normalized.contains("<strong") {
        normalized = RE_HTML_BOLD.replace_all(&normalized, "**$1**").into_owned();
    }
    if normalized.contains("<i") || normalized.contains("<em") {
        normalized = RE_HTML_ITALIC.replace_all(&normalized, "*$1*").into_owned();
    }
    if normalized.contains("<code") {
        normalized = RE_HTML_CODE.replace_all(&normalized, "`$1`").into_owned();
    }
    if normalized.contains("<a ") || normalized.contains("<a>") {
        normalized = RE_HTML_LINK
            .replace_all(&normalized, "[$2]($1)")
            .into_owned();
    }
    if normalized.contains("<br") {
        normalized = RE_HTML_BR.replace_all(&normalized, "\n").into_owned();
    }

    // Clean leaked HTML tags
    let cleaned = RE_HTML_LEAKED_TAGS
        .replace_all(&normalized, "")
        .into_owned();
    let unescaped = html_escape::decode_html_entities(&cleaned).to_string();

    let mut out: Vec<Value> = Vec::new();
    let mut rest = unescaped.as_str();

    while !rest.is_empty() {
        // 1. Bold **text**
        if rest.starts_with("**") {
            if let Some(end) = rest[2..].find("**") {
                let inner = &rest[2..2 + end];
                out.push(json!({
                    "type": "bold",
                    "text": parse_inline(inner)
                }));
                rest = &rest[2 + end + 2..];
                continue;
            }
        }

        // 2c. Underline <u>text</u> (++text++)
        if rest.starts_with("++") {
            if let Some(end) = rest[2..].find("++") {
                let inner = &rest[2..2 + end];
                out.push(json!({
                    "type": "underline",
                    "text": parse_inline(inner)
                }));
                rest = &rest[2 + end + 2..];
                continue;
            }
        }

        // 2. Bold __text__
        if rest.starts_with("__") {
            if let Some(end) = rest[2..].find("__") {
                let inner = &rest[2..2 + end];
                out.push(json!({
                    "type": "bold",
                    "text": parse_inline(inner)
                }));
                rest = &rest[2 + end + 2..];
                continue;
            }
        }

        // 2a. Spoiler ||text||
        if rest.starts_with("||") {
            if let Some(end) = rest[2..].find("||") {
                let inner = &rest[2..2 + end];
                out.push(json!({
                    "type": "spoiler",
                    "text": parse_inline(inner)
                }));
                rest = &rest[2 + end + 2..];
                continue;
            }
        }

        // 2b. Strikethrough ~~text~~
        if rest.starts_with("~~") {
            if let Some(end) = rest[2..].find("~~") {
                let inner = &rest[2..2 + end];
                out.push(json!({
                    "type": "strikethrough",
                    "text": parse_inline(inner)
                }));
                rest = &rest[2 + end + 2..];
                continue;
            }
        }

        // 3. Inline code `code`
        if rest.starts_with('`') {
            if let Some(end) = rest[1..].find('`') {
                let inner = &rest[1..1 + end];
                out.push(json!({
                    "type": "code",
                    "text": inner
                }));
                rest = &rest[1 + end + 1..];
                continue;
            }
        }

        // 4. Italic *text*
        if rest.starts_with('*') && !rest.starts_with("**") {
            if let Some(end) = rest[1..].find('*') {
                if end > 0 && !rest[1..].starts_with('*') {
                    let inner = &rest[1..1 + end];
                    out.push(json!({
                        "type": "italic",
                        "text": parse_inline(inner)
                    }));
                    rest = &rest[1 + end + 1..];
                    continue;
                }
            }
        }

        // 5. Italic _text_
        if rest.starts_with('_') && !rest.starts_with("__") {
            if let Some(end) = rest[1..].find('_') {
                if end > 0 {
                    let inner = &rest[1..1 + end];
                    out.push(json!({
                        "type": "italic",
                        "text": parse_inline(inner)
                    }));
                    rest = &rest[1 + end + 1..];
                    continue;
                }
            }
        }

        // 6a. Markdown image ![alt](url)
        if rest.starts_with("![") {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') {
                    let raw_url = &rest[close + 2..close + 2 + end];
                    let url = raw_url
                        .trim()
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .trim();
                    if url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with("tg://")
                    {
                        let alt = &rest[2..close];
                        let alt_trimmed = alt.trim();
                        let display_label = if alt_trimmed.is_empty() {
                            "📷 Foto".to_string()
                        } else {
                            format!("📷 {alt_trimmed}")
                        };
                        out.push(json!({
                            "type": "url",
                            "text": parse_inline(&display_label),
                            "url": url
                        }));
                        rest = &rest[close + 3 + end..];
                        continue;
                    }
                }
            }
        }

        // 6b. Links [text](url)
        if rest.starts_with('[') {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') {
                    let raw_url = &rest[close + 2..close + 2 + end];
                    let url = raw_url
                        .trim()
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .trim();
                    if url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with("tg://")
                    {
                        let inner = &rest[1..close];
                        let display_label = normalize_inline_media_label(inner);
                        out.push(json!({
                            "type": "url",
                            "text": parse_inline(&display_label),
                            "url": url
                        }));
                        rest = &rest[close + 3 + end..];
                        continue;
                    }
                }
            }
        }

        // 7. Inline math $...$
        if rest.starts_with('$') && !rest.starts_with("$$") {
            if let Some(end) = rest[1..].find('$') {
                if end > 0 && !rest[1..].starts_with('$') {
                    let inner = rest[1..1 + end].trim();
                    if !inner.is_empty() {
                        if let Some(sym) = try_format_standalone_logic_symbol(inner) {
                            out.push(Value::String(sym.to_string()));
                            rest = &rest[1 + end + 1..];
                            continue;
                        }
                        if rtl::has_rtl_characters(inner) {
                            let clean_text = rtl::extract_text_from_pseudo_math(inner);
                            let parsed = parse_inline(&clean_text);
                            match parsed {
                                Value::Array(arr) => out.extend(arr),
                                other => out.push(other),
                            }
                            rest = &rest[1 + end + 1..];
                            continue;
                        }
                        let sanitized = sanitize_latex_for_telegram(inner);
                        out.push(json!({
                            "type": "mathematical_expression",
                            "expression": sanitized
                        }));
                        rest = &rest[1 + end + 1..];
                        continue;
                    }
                }
            }
        }

        // 8. Inline math \( ... \)
        if rest.starts_with(r"\(") {
            if let Some(end) = rest[2..].find(r"\)") {
                let inner = rest[2..2 + end].trim();
                if !inner.is_empty() {
                    if let Some(sym) = try_format_standalone_logic_symbol(inner) {
                        out.push(Value::String(sym.to_string()));
                        rest = &rest[2 + end + 2..];
                        continue;
                    }
                    if rtl::has_rtl_characters(inner) {
                        let clean_text = rtl::extract_text_from_pseudo_math(inner);
                        let parsed = parse_inline(&clean_text);
                        match parsed {
                            Value::Array(arr) => out.extend(arr),
                            other => out.push(other),
                        }
                        rest = &rest[2 + end + 2..];
                        continue;
                    }
                    let sanitized = sanitize_latex_for_telegram(inner);
                    out.push(json!({
                        "type": "mathematical_expression",
                        "expression": sanitized
                    }));
                    rest = &rest[2 + end + 2..];
                    continue;
                }
            }
        }

        // 9. Plain text chunk until next token
        let mut next_pos = rest.len();
        for delim in &[
            "**", "__", "||", "~~", "++", "`", "*", "_", "![", "[", "$", r"\(",
        ] {
            if let Some(idx) = rest.find(delim) {
                if idx > 0 && idx < next_pos {
                    next_pos = idx;
                }
            }
        }

        if next_pos == rest.len() {
            out.push(Value::String(rest.to_string()));
            break;
        } else {
            out.push(Value::String(rest[..next_pos].to_string()));
            rest = &rest[next_pos..];
        }
    }

    // Merge adjacent strings
    let mut merged: Vec<Value> = Vec::new();
    for item in out {
        if let Value::String(s) = item {
            if let Some(Value::String(prev)) = merged.last_mut() {
                prev.push_str(&s);
            } else if !s.is_empty() {
                merged.push(Value::String(s));
            }
        } else {
            merged.push(item);
        }
    }

    if merged.is_empty() {
        Value::String(String::new())
    } else if merged.len() == 1 {
        merged.pop().unwrap_or_else(|| Value::String(String::new()))
    } else {
        Value::Array(merged)
    }
}

fn normalize_inline_media_label(label: &str) -> String {
    let t = label.trim();
    if let Some((tag, rest)) = t.split_once(':') {
        let tag_clean = tag.trim().to_lowercase();
        let rest_clean = rest.trim();
        let name = if rest_clean.is_empty() {
            tag.trim()
        } else {
            rest_clean
        };
        match tag_clean.as_str() {
            "photo" | "foto" | "image" | "img" | "gambar" | "picture" | "pic" => {
                return format!("📷 {name}");
            }
            "video" | "vid" => {
                return format!("🎬 {name}");
            }
            "audio" | "musik" | "music" | "lagu" | "song" => {
                return format!("🎵 {name}");
            }
            "voice" | "voicenote" | "voice_note" | "suara" | "rekaman" | "vn" => {
                return format!("🎙️ {name}");
            }
            "animation" | "animasi" | "gif" => {
                return format!("🎞️ {name}");
            }
            "document" | "dokumen" | "doc" | "file" | "berkas" => {
                return format!("📄 {name}");
            }
            "map" | "location" | "lokasi" | "peta" | "geo" => {
                return format!("📍 {name}");
            }
            _ => {}
        }
    }
    t.to_string()
}

fn is_border_line(line: &str) -> bool {
    let s = line.trim();
    if s.is_empty() {
        return true;
    }
    s.chars()
        .all(|c| "┌╔┏┬┰├┝┼╂└╚┗┴┸┤┥─━═+-=_ \t┐┘┒┙╗╝┚┖┓┛│|║┃".contains(c))
}

fn parse_coords_pair(text: &str) -> Option<(f64, f64, Option<i32>)> {
    let clean = text.trim().trim_matches(['(', ')', '[', ']']);
    let clean = clean.strip_prefix("geo:").unwrap_or(clean);
    let (coords_part, zoom_part) = if let Some((c, z)) = clean.split_once("?z=") {
        (c, z.parse::<i32>().ok())
    } else if let Some((c, z)) = clean.split_once("zoom=") {
        (c.trim_end_matches([',', ' ']), z.parse::<i32>().ok())
    } else {
        (clean, None)
    };
    let parts: Vec<&str> = coords_part.split(',').map(str::trim).collect();
    if parts.len() >= 2 {
        let lat = parts[0].parse::<f64>().ok()?;
        let lon = parts[1].parse::<f64>().ok()?;
        let zoom = zoom_part.or_else(|| {
            parts.get(2).and_then(|z| {
                z.strip_prefix("zoom=")
                    .unwrap_or(z)
                    .trim()
                    .parse::<i32>()
                    .ok()
            })
        });
        return Some((lat, lon, zoom));
    }
    None
}

fn try_parse_map_block(line: &str) -> Option<RichBlock> {
    let s = line.trim();
    let s_clean = s.trim_end_matches(['.', ',', ';', ':']);

    // Tag based: [map: ...], [location: ...], [lokasi: ...], [peta: ...], [geo: ...]
    let candidate = s_clean.strip_prefix('!').unwrap_or(s_clean);
    if candidate.starts_with('[') {
        if let Some(bracket_end) = candidate.find(']') {
            let tag_part = &candidate[1..bracket_end];
            if let Some((tag_name, label)) = tag_part.split_once(':') {
                let t = tag_name.trim().to_lowercase();
                if matches!(t.as_str(), "map" | "location" | "lokasi" | "peta" | "geo") {
                    let right = candidate[bracket_end + 1..].trim();
                    let right_clean = right.trim_end_matches(['.', ',', ';', ':', ' ']);
                    let coords_source = if let Some(inner) = right_clean
                        .strip_prefix('(')
                        .and_then(|r| r.strip_suffix(')'))
                    {
                        let link = inner
                            .trim()
                            .trim_start_matches('<')
                            .trim_end_matches('>')
                            .trim();
                        if link.starts_with("http") {
                            link.split("?q=").nth(1).unwrap_or(link)
                        } else {
                            link
                        }
                    } else {
                        label.trim()
                    };

                    if let Some((lat, lon, zoom)) = parse_coords_pair(coords_source) {
                        return RichBlock::map_coords(lat, lon, zoom).ok();
                    }
                }
            }
        }
    }

    // ![map](geo:...) or ![location](geo:...)
    if let Some(geo) = s_clean
        .strip_prefix("![map](geo:")
        .or_else(|| s_clean.strip_prefix("![location](geo:"))
        .or_else(|| s_clean.strip_prefix("![lokasi](geo:"))
        .and_then(|r| r.strip_suffix(')'))
    {
        if let Some((lat, lon, zoom)) = parse_coords_pair(geo) {
            return RichBlock::map_coords(lat, lon, zoom).ok();
        }
    }

    // <tg-map lat="..." lon="..." zoom="..."/>
    if let Some(rest) = s.strip_prefix("<tg-map") {
        let lat_s = extract_html_attribute(rest, "lat").unwrap_or("");
        let lon_s = extract_html_attribute(rest, "lon").unwrap_or("");
        let zoom_s = extract_html_attribute(rest, "zoom");
        let lat = lat_s.parse::<f64>().ok()?;
        let lon = lon_s.parse::<f64>().ok()?;
        let zoom = match zoom_s {
            Some(z) => Some(z.parse::<i32>().ok()?),
            None => None,
        };
        return RichBlock::map_coords(lat, lon, zoom).ok();
    }

    None
}

fn split_bracket_and_parenthesis(text: &str) -> Option<(&str, &str)> {
    let (left, right) = text.split_once(']')?;
    let right = right.trim();
    let right_cleaned = right.trim_end_matches(['.', ',', ';', ':', ' ']);
    let inner_right = right_cleaned.strip_prefix('(')?.strip_suffix(')')?.trim();
    let inner_right = inner_right
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    Some((left.trim(), inner_right))
}

fn classify_media_tag(tag: &str) -> Option<&'static str> {
    let t = tag.trim().to_lowercase();
    match t.as_str() {
        "photo" | "foto" | "image" | "img" | "gambar" | "picture" | "pic" => Some("photo"),
        "video" | "vid" => Some("video"),
        "audio" | "musik" | "music" | "lagu" | "song" => Some("audio"),
        "voice" | "voicenote" | "voice_note" | "suara" | "rekaman" | "vn" => Some("voice"),
        "animation" | "animasi" | "gif" => Some("animation"),
        "collage" | "kolase" | "gallery" | "galeri" | "album" => Some("collage"),
        "slideshow" | "slide" => Some("slideshow"),
        "document" | "dokumen" | "doc" | "file" | "berkas" => Some("document"),
        "map" | "location" | "lokasi" | "peta" | "geo" => Some("map"),
        _ => None,
    }
}

pub fn is_streaming_web_video(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("youtube.com/")
        || lower.contains("youtu.be/")
        || lower.contains("vimeo.com/")
        || lower.contains("dailymotion.com/")
        || lower.contains("twitch.tv/")
        || lower.contains("tiktok.com/")
        || lower.contains("bilibili.com/")
        || lower.contains("instagram.com/reel/")
        || lower.contains("facebook.com/watch")
        || lower.contains("streamable.com/")
        || lower.contains("loom.com/")
}

pub fn is_streaming_web_audio(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("spotify.com/")
        || lower.contains("soundcloud.com/")
        || lower.contains("music.apple.com/")
        || lower.contains("podcasts.apple.com/")
        || lower.contains("podbean.com/")
        || lower.contains("anchor.fm/")
        || lower.contains("mixcloud.com/")
        || lower.contains("bandcamp.com/")
        || lower.contains("audiomack.com/")
}

pub fn is_unsupported_image_format(url: &str) -> bool {
    let clean = url.split('?').next().unwrap_or(url).to_lowercase();
    clean.ends_with(".svg")
        || clean.ends_with(".bmp")
        || clean.ends_with(".tiff")
        || clean.ends_with(".tif")
        || clean.ends_with(".ico")
        || clean.ends_with(".heic")
        || clean.ends_with(".avif")
        || clean.ends_with(".html")
        || clean.ends_with(".htm")
        || clean.ends_with(".php")
        || clean.contains("imgur.com/a/")
        || clean.contains("imgur.com/gallery/")
        || clean.contains("flickr.com/photos/")
        || clean.contains("pinterest.com/pin/")
}

fn extract_html_attribute<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    let mut cursor = 0;
    let attr_lower = attr.to_lowercase();
    let tag_lower = tag.to_lowercase();
    while let Some(idx) = tag_lower[cursor..].find(&attr_lower) {
        let pos = cursor + idx;
        let after_attr = &tag[pos + attr.len()..];
        // Ensure word boundary before attr
        if pos > 0 {
            let prev = tag.as_bytes()[pos - 1];
            if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
                cursor = pos + attr.len();
                continue;
            }
        }
        let trimmed_after = after_attr.trim_start();
        if let Some(rest) = trimmed_after.strip_prefix('=') {
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix('"') {
                if let Some(end) = val.find('"') {
                    return Some(&val[..end]);
                }
            } else if let Some(val) = rest.strip_prefix('\'') {
                if let Some(end) = val.find('\'') {
                    return Some(&val[..end]);
                }
            } else {
                let end = rest.find([' ', '>', '/', '\t', '\n']).unwrap_or(rest.len());
                if end > 0 {
                    return Some(&rest[..end]);
                }
            }
        }
        cursor = pos + attr.len();
    }
    None
}

fn format_media_fallback_paragraph(
    label: &str,
    link: &str,
    default_label: &str,
    emoji: &str,
) -> RichBlock {
    let cap_text = if label.is_empty() {
        default_label
    } else {
        label
    };
    RichBlock::Paragraph {
        text: parse_inline(&format!("{emoji} [{cap_text}]({link})")),
    }
}

fn parse_multi_media_list_block(
    link: &str,
    label: &str,
    caption: Option<RichBlockCaption>,
    is_slideshow: bool,
) -> Option<RichBlock> {
    let urls: Vec<&str> = link
        .split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .collect();
    let mut valid_blocks = Vec::new();
    let mut fallback_urls = Vec::new();
    for u in urls {
        if is_unsupported_image_format(u) || is_streaming_web_video(u) || is_streaming_web_audio(u)
        {
            fallback_urls.push(u);
        } else {
            valid_blocks.push(json!({"type": "photo", "photo": {"type": "photo", "media": u}}));
        }
    }
    if valid_blocks.len() >= 2 {
        if is_slideshow {
            Some(RichBlock::Slideshow {
                blocks: valid_blocks,
                caption,
            })
        } else {
            Some(RichBlock::Collage {
                blocks: valid_blocks,
                caption,
            })
        }
    } else if valid_blocks.len() == 1 {
        let first = valid_blocks
            .pop()
            .expect("guaranteed single element in valid_blocks");
        let photo_val = first.get("photo").cloned().unwrap_or(first);
        Some(RichBlock::Photo {
            photo: photo_val,
            caption,
        })
    } else {
        let default_title = if is_slideshow {
            "Slideshow"
        } else {
            "Galeri Foto"
        };
        let cap_text = if label.is_empty() {
            default_title
        } else {
            label
        };
        let item_prefix = if is_slideshow { "Slide" } else { "Foto" };
        let mut text_parts = format!("🖼️ [{cap_text}]: ");
        for (idx, u) in fallback_urls.into_iter().enumerate() {
            if idx > 0 {
                text_parts.push_str(" • ");
            }
            text_parts.push_str(&format!("[{item_prefix} #{}]({u})", idx + 1));
        }
        Some(RichBlock::Paragraph {
            text: parse_inline(&text_parts),
        })
    }
}

fn try_parse_doc_block(line: &str) -> Option<RichBlock> {
    let s = line.trim();
    let s_clean = s.trim_end_matches(['.', ',', ';', ':']);

    if s_clean.starts_with('[') || s_clean.starts_with("![") {
        let candidate = s_clean.strip_prefix('!').unwrap_or(s_clean);
        if let Some(bracket_end) = candidate.find(']') {
            let tag_part = &candidate[1..bracket_end];
            if let Some((tag_name, name)) = tag_part.split_once(':') {
                let t = tag_name.trim().to_lowercase();
                if matches!(
                    t.as_str(),
                    "document" | "dokumen" | "doc" | "file" | "berkas"
                ) {
                    let right = candidate[bracket_end + 1..].trim();
                    let right_clean = right.trim_end_matches(['.', ',', ';', ':', ' ']);
                    if let Some(inner) = right_clean
                        .strip_prefix('(')
                        .and_then(|r| r.strip_suffix(')'))
                    {
                        let link = inner
                            .trim()
                            .trim_start_matches('<')
                            .trim_end_matches('>')
                            .trim();
                        let name = name.trim();
                        return Some(RichBlock::Document {
                            document: json!({"type": "document", "media": link}),
                            caption: (!name.is_empty())
                                .then(|| RichBlockCaption::new(parse_inline(name))),
                        });
                    }
                }
            }
        }
    }

    if let Some(rest) = s.strip_prefix("<tg-document") {
        let trimmed = rest.trim().trim_end_matches('>').trim_end_matches('/');
        let link = trimmed
            .split("src=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .or_else(|| {
                trimmed
                    .split("src='")
                    .nth(1)
                    .and_then(|s| s.split('\'').next())
            })
            .unwrap_or("");
        let name = trimmed
            .split("name=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .or_else(|| {
                trimmed
                    .split("name='")
                    .nth(1)
                    .and_then(|s| s.split('\'').next())
            })
            .unwrap_or("");
        if !link.is_empty() {
            return Some(RichBlock::Document {
                document: json!({"type": "document", "media": link}),
                caption: (!name.is_empty()).then(|| RichBlockCaption::new(parse_inline(name))),
            });
        }
    }
    None
}

fn try_parse_media_block(line: &str) -> Option<RichBlock> {
    let s = line.trim();
    let s_clean = s.trim_end_matches(['.', ',', ';', ':']);

    // 1. Bracketed media tags [photo: ...] or ![photo: ...]
    let candidate = s_clean.strip_prefix('!').unwrap_or(s_clean).trim_start();
    if candidate.starts_with('[') {
        if let Some(bracket_end) = candidate.find(']') {
            let tag_part = &candidate[1..bracket_end];
            if let Some((tag_name, label)) = tag_part.split_once(':') {
                if let Some(kind) = classify_media_tag(tag_name) {
                    let label = label.trim();
                    let right = candidate[bracket_end + 1..].trim();
                    let right_clean = right.trim_end_matches(['.', ',', ';', ':', ' ']);

                    // Map without parenthesis: [map: -6.2, 106.8]
                    if kind == "map" && right_clean.is_empty() {
                        if let Some((lat, lon, zoom)) = parse_coords_pair(label) {
                            return Some(RichBlock::Map {
                                location: Location {
                                    latitude: lat,
                                    longitude: lon,
                                    horizontal_accuracy: None,
                                },
                                zoom,
                                width: None,
                                height: None,
                            });
                        }
                    }

                    if let Some(inner_right) = right_clean
                        .strip_prefix('(')
                        .and_then(|r| r.strip_suffix(')'))
                    {
                        let link = inner_right
                            .trim()
                            .trim_start_matches('<')
                            .trim_end_matches('>')
                            .trim();
                        let caption =
                            (!label.is_empty()).then(|| RichBlockCaption::new(parse_inline(label)));

                        match kind {
                            "photo" => {
                                if is_streaming_web_video(link) {
                                    return Some(format_media_fallback_paragraph(
                                        label,
                                        link,
                                        "Tonton Video",
                                        "🎬",
                                    ));
                                }
                                if is_streaming_web_audio(link) {
                                    return Some(format_media_fallback_paragraph(
                                        label,
                                        link,
                                        "Dengarkan Audio",
                                        "🎵",
                                    ));
                                }
                                if is_unsupported_image_format(link) {
                                    return Some(format_media_fallback_paragraph(
                                        label,
                                        link,
                                        "Lihat Foto",
                                        "🖼️",
                                    ));
                                }
                                return Some(RichBlock::Photo {
                                    photo: json!({"type": "photo", "media": link}),
                                    caption,
                                });
                            }
                            "video" => {
                                if is_streaming_web_video(link) {
                                    return Some(format_media_fallback_paragraph(
                                        label,
                                        link,
                                        "Tonton Video",
                                        "🎬",
                                    ));
                                }
                                return Some(RichBlock::Video {
                                    video: json!({"type": "video", "media": link}),
                                    caption,
                                });
                            }
                            "audio" => {
                                if is_streaming_web_audio(link) {
                                    return Some(format_media_fallback_paragraph(
                                        label,
                                        link,
                                        "Dengarkan Audio",
                                        "🎵",
                                    ));
                                }
                                return Some(RichBlock::Audio {
                                    audio: json!({"type": "audio", "media": link}),
                                    caption,
                                });
                            }
                            "voice" => {
                                return Some(RichBlock::VoiceNote {
                                    voice_note: json!({"type": "voice_note", "media": link}),
                                    caption,
                                });
                            }
                            "animation" => {
                                return Some(RichBlock::Animation {
                                    animation: json!({"type": "animation", "media": link}),
                                    caption,
                                });
                            }
                            "document" => {
                                return Some(RichBlock::Document {
                                    document: json!({"type": "document", "media": link}),
                                    caption,
                                });
                            }
                            "collage" => {
                                return parse_multi_media_list_block(link, label, caption, false);
                            }
                            "slideshow" => {
                                return parse_multi_media_list_block(link, label, caption, true);
                            }
                            "map" => {
                                let coords_str = if link.starts_with("http") {
                                    link.split("?q=").nth(1).unwrap_or(link)
                                } else {
                                    link
                                };
                                if let Some((lat, lon, zoom)) = parse_coords_pair(coords_str) {
                                    return Some(RichBlock::Map {
                                        location: Location {
                                            latitude: lat,
                                            longitude: lon,
                                            horizontal_accuracy: None,
                                        },
                                        zoom,
                                        width: None,
                                        height: None,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // 2. Plain Markdown image syntax ![alt](url)
    if let Some(rest) = s_clean.strip_prefix("![") {
        let lower = s_clean.to_lowercase();
        if !lower.starts_with("![map")
            && !lower.starts_with("![location")
            && !lower.starts_with("![lokasi")
            && !lower.starts_with("![peta")
        {
            if let Some((alt, link)) = split_bracket_and_parenthesis(rest) {
                if link.starts_with("http://")
                    || link.starts_with("https://")
                    || link.starts_with("tg://")
                {
                    if is_streaming_web_video(link) {
                        return Some(format_media_fallback_paragraph(
                            alt,
                            link,
                            "Tonton Video",
                            "🎬",
                        ));
                    }
                    if is_streaming_web_audio(link) {
                        return Some(format_media_fallback_paragraph(
                            alt,
                            link,
                            "Dengarkan Audio",
                            "🎵",
                        ));
                    }
                    if is_unsupported_image_format(link) {
                        return Some(format_media_fallback_paragraph(
                            alt,
                            link,
                            "Lihat Foto",
                            "🖼️",
                        ));
                    }

                    let clean_url = link.split('?').next().unwrap_or(link);
                    let lower_link = clean_url.to_lowercase();
                    let caption =
                        (!alt.is_empty()).then(|| RichBlockCaption::new(parse_inline(alt)));
                    if lower_link.ends_with(".mp4")
                        || lower_link.ends_with(".webm")
                        || lower_link.ends_with(".mov")
                    {
                        return Some(RichBlock::Video {
                            video: json!({"type": "video", "media": link}),
                            caption,
                        });
                    } else if lower_link.ends_with(".mp3")
                        || lower_link.ends_with(".ogg")
                        || lower_link.ends_with(".wav")
                        || lower_link.ends_with(".m4a")
                    {
                        return Some(RichBlock::Audio {
                            audio: json!({"type": "audio", "media": link}),
                            caption,
                        });
                    } else if lower_link.ends_with(".gif") {
                        return Some(RichBlock::Animation {
                            animation: json!({"type": "animation", "media": link}),
                            caption,
                        });
                    } else {
                        return Some(RichBlock::Photo {
                            photo: json!({"type": "photo", "media": link}),
                            caption,
                        });
                    }
                }
            }
        }
    }

    // 3. Telegram native HTML media tags: <tg-photo ...>, <tg-video ...>, <tg-audio ...>, <img ...>, <audio ...>
    if s.starts_with("<tg-photo")
        || s.starts_with("<tg-video")
        || s.starts_with("<tg-audio")
        || s.starts_with("<img")
        || s.starts_with("<audio")
    {
        if let Some(block) = try_parse_html_media_tag(s) {
            return Some(block);
        }
    }

    None
}

fn try_parse_html_media_tag(tag: &str) -> Option<RichBlock> {
    let s = tag.trim();
    let src = extract_html_attribute(s, "src").unwrap_or("");
    if src.is_empty() {
        return None;
    }
    let inner_text = s
        .split('>')
        .nth(1)
        .and_then(|t| t.split("</").next())
        .map(str::trim)
        .filter(|t| !t.is_empty());

    if s.starts_with("<tg-photo") || s.starts_with("<img") {
        let cap_attr = extract_html_attribute(s, "caption")
            .or_else(|| extract_html_attribute(s, "alt"))
            .or_else(|| extract_html_attribute(s, "title"));
        let caption_text = cap_attr.or(inner_text).unwrap_or("");
        let caption =
            (!caption_text.is_empty()).then(|| RichBlockCaption::new(parse_inline(caption_text)));

        if is_streaming_web_video(src) {
            return Some(format_media_fallback_paragraph(
                caption_text,
                src,
                "Tonton Video",
                "🎬",
            ));
        }
        if is_streaming_web_audio(src) {
            return Some(format_media_fallback_paragraph(
                caption_text,
                src,
                "Dengarkan Audio",
                "🎵",
            ));
        }
        if is_unsupported_image_format(src) {
            return Some(format_media_fallback_paragraph(
                caption_text,
                src,
                "Lihat Foto",
                "🖼️",
            ));
        }
        return Some(RichBlock::Photo {
            photo: json!({"type": "photo", "media": src}),
            caption,
        });
    }

    if s.starts_with("<tg-video") {
        let cap_attr =
            extract_html_attribute(s, "caption").or_else(|| extract_html_attribute(s, "title"));
        let caption_text = cap_attr.or(inner_text).unwrap_or("");
        let caption =
            (!caption_text.is_empty()).then(|| RichBlockCaption::new(parse_inline(caption_text)));

        if is_streaming_web_video(src) {
            return Some(format_media_fallback_paragraph(
                caption_text,
                src,
                "Tonton Video",
                "🎬",
            ));
        }
        return Some(RichBlock::Video {
            video: json!({"type": "video", "media": src}),
            caption,
        });
    }

    if s.starts_with("<tg-audio") || s.starts_with("<audio") {
        let cap_attr = extract_html_attribute(s, "caption");
        let caption_text = cap_attr.or(inner_text).unwrap_or("");
        let caption =
            (!caption_text.is_empty()).then(|| RichBlockCaption::new(parse_inline(caption_text)));

        if is_streaming_web_audio(src) {
            return Some(format_media_fallback_paragraph(
                caption_text,
                src,
                "Dengarkan Audio",
                "🎵",
            ));
        }
        let title = extract_html_attribute(s, "title");
        let performer = extract_html_attribute(s, "performer");
        let mut audio_obj = json!({"type": "audio", "media": src});
        if let Some(t) = title {
            audio_obj["title"] = Value::String(t.to_string());
        }
        if let Some(p) = performer {
            audio_obj["performer"] = Value::String(p.to_string());
        }
        return Some(RichBlock::Audio {
            audio: audio_obj,
            caption,
        });
    }

    None
}

fn try_parse_container_media_block(
    lines: &[String],
    start_idx: usize,
) -> Option<(RichBlock, usize)> {
    let first_line = lines[start_idx].trim();
    let is_html = first_line.starts_with('<');
    let is_slideshow = first_line.to_lowercase().contains("slideshow");

    let close_tag = if is_html {
        if is_slideshow {
            "</tg-slideshow>"
        } else {
            "</tg-collage>"
        }
    } else if is_slideshow {
        "[/slideshow]"
    } else if first_line.to_lowercase().contains("kolase") {
        "[/kolase]"
    } else {
        "[/collage]"
    };

    let mut collected = Vec::new();
    let mut i = start_idx;
    let n = lines.len();

    if is_html && first_line.contains(close_tag) {
        collected.push(first_line.to_string());
        i += 1;
    } else {
        while i < n {
            let line = lines[i].trim();
            collected.push(line.to_string());
            i += 1;
            if line.contains(close_tag) {
                break;
            }
        }
    }

    let full_content = collected.join("\n");
    let caption_text = if is_html {
        extract_html_attribute(first_line, "caption")
            .or_else(|| extract_html_attribute(first_line, "title"))
            .or_else(|| extract_html_attribute(first_line, "alt"))
    } else {
        first_line
            .strip_prefix('[')
            .and_then(|s| s.split_once(']'))
            .map(|(tag_part, _)| tag_part)
            .and_then(|t| t.split_once(':'))
            .map(|(_, cap)| cap.trim())
            .filter(|c| !c.is_empty())
    };

    let caption = caption_text.map(|c| RichBlockCaption::new(parse_inline(c)));

    static RE_MEDIA_SRC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:src=["']([^"']+)["']|!?\[[^\]]*\]\(([^)]+)\)|https?://[^\s"'<>()]+)"#)
            .expect("valid static regex")
    });
    static RE_CHILD_TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<(?:img|tg-photo|video|tg-video)\s+[^>]*?/?>"#)
            .expect("valid static regex")
    });

    let mut sub_blocks = Vec::new();
    // First try tag-based extraction to preserve per-image alt/caption
    for tag_mat in RE_CHILD_TAG.find_iter(&full_content) {
        let tag_str = tag_mat.as_str();
        let src = extract_html_attribute(tag_str, "src").unwrap_or("");
        let clean_url = src.trim_matches(['"', '\'', '<', '>']);
        if clean_url.starts_with("http://")
            || clean_url.starts_with("https://")
            || clean_url.starts_with("tg://")
            || clean_url.starts_with("attach://")
        {
            let lower = clean_url.to_lowercase();
            let is_vid = lower.ends_with(".mp4")
                || lower.ends_with(".webm")
                || lower.ends_with(".mov")
                || tag_str.to_lowercase().starts_with("<video")
                || tag_str.to_lowercase().starts_with("<tg-video");
            let child_cap = extract_html_attribute(tag_str, "caption")
                .or_else(|| extract_html_attribute(tag_str, "alt"))
                .or_else(|| extract_html_attribute(tag_str, "title"));

            if is_vid {
                let mut vid_obj = json!({"type": "video", "media": clean_url});
                if let Some(c) = child_cap {
                    vid_obj["caption"] = Value::String(c.to_string());
                }
                sub_blocks.push(json!({"type": "video", "video": vid_obj}));
            } else if !lower.ends_with(".html")
                && !lower.ends_with(".htm")
                && !is_streaming_web_video(clean_url)
                && !is_streaming_web_audio(clean_url)
                && !is_unsupported_image_format(clean_url)
            {
                let mut photo_obj = json!({"type": "photo", "media": clean_url});
                if let Some(c) = child_cap {
                    photo_obj["caption"] = Value::String(c.to_string());
                }
                sub_blocks.push(json!({"type": "photo", "photo": photo_obj}));
            }
        }
    }

    // If no HTML child tags found, fall back to RE_MEDIA_SRC
    if sub_blocks.is_empty() {
        for caps in RE_MEDIA_SRC.captures_iter(&full_content) {
            let url = caps
                .get(1)
                .or_else(|| caps.get(2))
                .or_else(|| caps.get(0))
                .map(|m| m.as_str().trim())
                .unwrap_or("");
            let clean_url = url.trim_matches(['"', '\'', '<', '>']);
            if clean_url.starts_with("http://")
                || clean_url.starts_with("https://")
                || clean_url.starts_with("tg://")
                || clean_url.starts_with("attach://")
            {
                let lower = clean_url.to_lowercase();
                if lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mov") {
                    sub_blocks.push(
                        json!({"type": "video", "video": {"type": "video", "media": clean_url}}),
                    );
                } else if !lower.ends_with(".html")
                    && !lower.ends_with(".htm")
                    && !is_streaming_web_video(clean_url)
                    && !is_streaming_web_audio(clean_url)
                    && !is_unsupported_image_format(clean_url)
                {
                    sub_blocks.push(
                        json!({"type": "photo", "photo": {"type": "photo", "media": clean_url}}),
                    );
                }
            }
        }
    }

    if sub_blocks.len() >= 2 {
        if !is_slideshow && sub_blocks.len() > 10 {
            sub_blocks.truncate(10);
        }
        let block = if is_slideshow {
            RichBlock::Slideshow {
                blocks: sub_blocks,
                caption,
            }
        } else {
            RichBlock::Collage {
                blocks: sub_blocks,
                caption,
            }
        };
        Some((block, i))
    } else if sub_blocks.len() == 1 {
        let first = sub_blocks
            .pop()
            .expect("guaranteed single element in sub_blocks");
        let block = if first["type"] == "video" {
            RichBlock::Video {
                video: first["video"].clone(),
                caption,
            }
        } else {
            RichBlock::Photo {
                photo: first["photo"].clone(),
                caption,
            }
        };
        Some((block, i))
    } else {
        let cap = caption_text.unwrap_or("Galeri Media");
        let mut text = format!("🖼️ [{cap}]: ");
        let mut count = 0;
        for caps in RE_MEDIA_SRC.captures_iter(&full_content) {
            let url = caps
                .get(1)
                .or_else(|| caps.get(2))
                .or_else(|| caps.get(0))
                .map(|m| m.as_str().trim())
                .unwrap_or("");
            let clean_url = url.trim_matches(['"', '\'', '<', '>']);
            if clean_url.starts_with("http://") || clean_url.starts_with("https://") {
                if count > 0 {
                    text.push_str(" • ");
                }
                count += 1;
                text.push_str(&format!("[Tautan #{count}]({clean_url})"));
            }
        }
        if count > 0 {
            Some((
                RichBlock::Paragraph {
                    text: parse_inline(&text),
                },
                i,
            ))
        } else {
            None
        }
    }
}

static RE_EMBEDDED_MEDIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(!?\[(?:photo|foto|image|img|gambar|picture|pic|video|vid|audio|musik|music|lagu|song|voice|voicenote|voice_note|suara|rekaman|vn|animation|animasi|gif|collage|kolase|gallery|galeri|album|slideshow|slide|document|dokumen|doc|file|berkas|map|location|lokasi|peta|geo)\s*:[^\]]+\](?:\s*\([^\)]+\))?[.,;:]?|!\[[^\]]*\]\s*\([^\)]+\)[.,;:]?|<tg-(?:photo|video|audio|document|map|collage|slideshow)[^>]*>|</tg-(?:photo|video|audio|document|map|collage|slideshow)>|<img[^>]*>|<audio[^>]*>|</audio>)"#
    ).expect("valid static regex")
});

pub fn isolate_embedded_media_blocks(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(text.len() + 64);
    let mut in_code_block = false;

    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
            continue;
        }

        if in_code_block || trimmed.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
            continue;
        }

        if !RE_EMBEDDED_MEDIA.is_match(line) {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
            continue;
        }

        // Split line around matches, putting each media block on its own line
        let mut last_end = 0;
        for mat in RE_EMBEDDED_MEDIA.find_iter(line) {
            let start = mat.start();
            let end = mat.end();

            let before = line[last_end..start].trim();
            if !before.is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(before);
            }

            let matched_tag = line[start..end].trim();
            if !matched_tag.is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(matched_tag);
            }

            last_end = end;
        }

        let after = line[last_end..].trim();
        if !after.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(after);
        }
    }

    output
}

static RE_THINK_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:think|thought|reasoning|reflection)\b.*?</(?:think|thought|reasoning|reflection)>").expect("valid static regex")
});
static RE_TOOL_CALL_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:tool_call|function_calls?)\b.*?</(?:tool_call|function_calls?)>")
        .expect("valid static regex")
});
static RE_SQUARE_THINK_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\[(?:think|thinking|thought|reasoning|reflection)\].*?\[/(?:think|thinking|thought|reasoning|reflection)\]")
        .expect("valid static regex")
});
static RE_UNCLOSED_THINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:think|thought|reasoning|reflection)\b.*$").expect("valid static regex")
});
static RE_UNCLOSED_SQUARE_THINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\[(?:think|thinking|thought|reasoning|reflection)\](?:[^(].*|$)")
        .expect("valid static regex")
});
static RE_TRAILING_INCOMPLETE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*(?:t(?:h(?:i(?:n(?:k)?)?)?)?|t(?:h(?:o(?:u(?:g(?:h(?:t)?)?)?)?)?)?|r(?:e(?:a(?:s(?:o(?:n(?:i(?:n(?:g)?)?)?)?)?)?)?)?|r(?:e(?:f(?:l(?:e(?:c(?:t(?:i(?:o(?:n)?)?)?)?)?)?)?)?)?)?$").expect("valid static regex")
});
static RE_TRAILING_INCOMPLETE_SQUARE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\[\s*(?:t(?:h(?:i(?:n(?:k(?:i(?:n(?:g)?)?)?)?)?)?)?|t(?:h(?:o(?:u(?:g(?:h(?:t)?)?)?)?)?)?|r(?:e(?:a(?:s(?:o(?:n(?:i(?:n(?:g)?)?)?)?)?)?)?)?|r(?:e(?:f(?:l(?:e(?:c(?:t(?:i(?:o(?:n)?)?)?)?)?)?)?)?)?)?$").expect("valid static regex")
});
static RE_LEAKED_CONTROL_TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:think|thought|reasoning|reflection|tool_call|function_calls?)\b[^>]*>")
        .expect("valid static regex")
});

pub fn sanitize_leaked_llm_artifacts(text: &str) -> String {
    // 1. Strip closed thinking / reflection / tool blocks
    let step1 = RE_THINK_BLOCK.replace_all(text, "");
    let step2 = RE_TOOL_CALL_BLOCK.replace_all(&step1, "");
    let step3 = RE_SQUARE_THINK_BLOCK.replace_all(&step2, "").into_owned();

    // 2. Strip unclosed thinking blocks to EOF
    let step4 = RE_UNCLOSED_THINK.replace_all(&step3, "");
    let step5 = RE_UNCLOSED_SQUARE_THINK
        .replace_all(&step4, "")
        .into_owned();

    // 3. Strip trailing partial opening tags (e.g. "<", "<th", "[th")
    let step6 = RE_TRAILING_INCOMPLETE_TAG
        .replace_all(&step5, "")
        .into_owned();
    let step7 = RE_TRAILING_INCOMPLETE_SQUARE_TAG
        .replace_all(&step6, "")
        .into_owned();

    // 4. Strip any residual leaked tags or control tokens
    let mut cleaned = RE_LEAKED_CONTROL_TAGS.replace_all(&step7, "").into_owned();
    cleaned = cleaned
        .replace("<|im_start|>", "")
        .replace("<|im_end|>", "")
        .replace("<|endoftext|>", "");

    cleaned
}

pub fn extract_thinking_and_answer(raw: &str) -> (Option<String>, String) {
    static RE_EXTRACT_THINK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<(?:think|thought|reasoning|reflection)\b[^>]*>(.*?)</(?:think|thought|reasoning|reflection)>").expect("valid static regex")
    });
    static RE_EXTRACT_SQUARE_THINK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)\[(?:think|thinking|thought|reasoning|reflection)\](.*?)(?:\[/(?:think|thinking|thought|reasoning|reflection)\]|$)").expect("valid static regex")
    });

    let thinking = RE_EXTRACT_THINK
        .captures(raw)
        .or_else(|| RE_EXTRACT_SQUARE_THINK.captures(raw))
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty());

    let answer = sanitize_leaked_llm_artifacts(raw).trim().to_string();
    (thinking, answer)
}

fn compute_column_rtl_flags(rows: &[Vec<&str>]) -> Vec<bool> {
    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_is_rtl = vec![false; col_count];
    for row in rows {
        for (col_idx, cell) in row.iter().enumerate() {
            if col_idx < col_is_rtl.len() && rtl::has_rtl_characters(cell) {
                col_is_rtl[col_idx] = true;
            }
        }
    }
    col_is_rtl
}

fn resolve_table_cell_align<'a>(
    explicit: Option<&'a str>,
    col_is_rtl: bool,
    cell_text: &str,
) -> &'a str {
    if let Some(align) = explicit {
        return align;
    }
    if col_is_rtl || rtl::has_rtl_characters(cell_text) {
        "right"
    } else {
        "left"
    }
}

fn is_ascii_numeric_cell(c: &str) -> bool {
    c.chars()
        .all(|ch| ch.is_ascii_digit() || ch.is_whitespace() || ch == '.' || ch == ',')
        && c.chars().any(|ch| ch.is_ascii_digit())
}

/// Splits a table row into cell strings, respecting escaping, code spans, and math blocks
/// so that pipes `|` inside `$ ... $`, `$$ ... $$`, `\( ... \)`, `\[ ... \]`, or ` `...` `
/// (e.g. absolute value `|x|`, norm `|v|_p`, or set builder `{x | x > 0}`) are preserved
/// inside the cell content rather than splitting the table columns prematurely.
fn split_table_row_cells(row_str: &str, is_box_table: bool) -> Vec<String> {
    let trimmed = row_str.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let is_delim = |c: char| -> bool {
        if is_box_table {
            "│|║┃".contains(c)
        } else {
            c == '|'
        }
    };

    let chars: Vec<char> = trimmed.chars().collect();
    let n = chars.len();

    // Determine if there is a leading outer border delimiter
    let start_idx = if n > 0 && is_delim(chars[0]) { 1 } else { 0 };

    // Determine if there is a trailing outer border delimiter (not escaped)
    let end_idx = if n > start_idx && is_delim(chars[n - 1]) {
        if n >= 2 && chars[n - 2] == '\\' {
            n
        } else {
            n - 1
        }
    } else {
        n
    };

    let mut cells: Vec<String> = Vec::new();
    let mut current_cell = String::new();

    let mut in_code = false;
    let mut in_inline_math = false;
    let mut in_display_math = false;
    let mut in_paren_math = false;
    let mut in_bracket_math = false;

    let mut i = start_idx;
    while i < end_idx {
        let ch = chars[i];

        // Check for escaping
        if ch == '\\' {
            if i + 1 < end_idx {
                let next = chars[i + 1];
                if next == '|' {
                    // Escaped pipe \| -> keep pipe in cell!
                    current_cell.push('|');
                    i += 2;
                    continue;
                } else if next == '(' && !in_code {
                    in_paren_math = true;
                    current_cell.push(ch);
                    current_cell.push(next);
                    i += 2;
                    continue;
                } else if next == ')' && !in_code {
                    in_paren_math = false;
                    current_cell.push(ch);
                    current_cell.push(next);
                    i += 2;
                    continue;
                } else if next == '[' && !in_code {
                    in_bracket_math = true;
                    current_cell.push(ch);
                    current_cell.push(next);
                    i += 2;
                    continue;
                } else if next == ']' && !in_code {
                    in_bracket_math = false;
                    current_cell.push(ch);
                    current_cell.push(next);
                    i += 2;
                    continue;
                }
            }
            current_cell.push(ch);
            i += 1;
            continue;
        }

        // Code span
        if ch == '`' {
            in_code = !in_code;
            current_cell.push(ch);
            i += 1;
            continue;
        }

        // Math handling when not in code
        if !in_code && ch == '$' {
            if i + 1 < end_idx && chars[i + 1] == '$' {
                in_display_math = !in_display_math;
                current_cell.push('$');
                current_cell.push('$');
                i += 2;
                continue;
            } else if !in_display_math {
                in_inline_math = !in_inline_math;
                current_cell.push('$');
                i += 1;
                continue;
            }
        }

        let in_math = in_inline_math || in_display_math || in_paren_math || in_bracket_math;

        if is_delim(ch) && !in_code && !in_math {
            cells.push(current_cell.trim().to_string());
            current_cell.clear();
        } else {
            current_cell.push(ch);
        }

        i += 1;
    }

    cells.push(current_cell.trim().to_string());
    cells
}

fn try_parse_table(
    lines: &[String],
    i: usize,
    is_message_rtl: bool,
) -> (Option<Vec<Vec<RichBlockTableCell>>>, bool, usize) {
    let n = lines.len();
    let line = lines[i].trim();

    // 1. Standard Markdown Table (| Col 1 | Col 2 |\n| --- | --- |)
    if line.contains('|') && i + 1 < n {
        let next_line = lines[i + 1].trim();
        let sep_cells = split_table_row_cells(next_line, false);

        let is_sep = !sep_cells.is_empty()
            && sep_cells.iter().any(|c| {
                !c.is_empty() && c.trim_matches(':').chars().all(|ch| ch == '-' || ch == '=')
            })
            && sep_cells.iter().all(|c| {
                if c.is_empty() {
                    return true;
                }
                let trimmed = c.trim_matches(':');
                !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '-' || ch == '=')
            });

        if is_sep {
            let mut explicit_aligns: Vec<Option<&str>> = Vec::new();
            for c in &sep_cells {
                if c.starts_with(':') && c.ends_with(':') {
                    explicit_aligns.push(Some("center"));
                } else if c.ends_with(':') {
                    explicit_aligns.push(Some("right"));
                } else if c.starts_with(':') {
                    explicit_aligns.push(Some("left"));
                } else {
                    explicit_aligns.push(None);
                }
            }

            let header_raw = split_table_row_cells(line, false);
            let mut raw_rows: Vec<Vec<String>> = Vec::new();
            let mut idx_line = i + 2;

            while idx_line < n {
                let row_str = lines[idx_line].trim();
                if row_str.is_empty() || !row_str.contains('|') {
                    break;
                }
                let row_raw = split_table_row_cells(row_str, false);
                raw_rows.push(row_raw);
                idx_line += 1;
            }

            let headers_str: Vec<&str> = header_raw.iter().map(|s| s.as_str()).collect();
            let table_is_rtl = rtl::is_table_predominantly_rtl(&headers_str);

            let col_is_rtl = {
                let mut all_rows: Vec<Vec<&str>> = Vec::with_capacity(raw_rows.len() + 1);
                all_rows.push(headers_str.clone());
                for r in &raw_rows {
                    all_rows.push(r.iter().map(|s| s.as_str()).collect());
                }
                compute_column_rtl_flags(&all_rows)
            };

            let mut header_row: Vec<RichBlockTableCell> = header_raw
                .into_iter()
                .enumerate()
                .map(|(idx, h)| {
                    let explicit = explicit_aligns.get(idx).copied().flatten();
                    let is_rtl = col_is_rtl.get(idx).copied().unwrap_or(false);
                    let align = resolve_table_cell_align(explicit, is_rtl, &h);
                    RichBlockTableCell::new(parse_inline(&h), true, Some(align))
                })
                .collect();

            let mut data_rows: Vec<Vec<RichBlockTableCell>> = Vec::with_capacity(raw_rows.len());
            for row_raw in raw_rows {
                let data_row: Vec<RichBlockTableCell> = row_raw
                    .into_iter()
                    .enumerate()
                    .map(|(idx, c)| {
                        let explicit = explicit_aligns.get(idx).copied().flatten();
                        let is_rtl = col_is_rtl.get(idx).copied().unwrap_or(false);
                        let align = resolve_table_cell_align(explicit, is_rtl, &c);
                        let is_numeric = is_ascii_numeric_cell(&c);
                        let formatted_cell = if (table_is_rtl || is_rtl) && is_numeric {
                            rtl::to_eastern_arabic_digits(&c)
                        } else {
                            c
                        };
                        RichBlockTableCell::new(parse_inline(&formatted_cell), false, Some(align))
                    })
                    .collect();
                data_rows.push(data_row);
            }

            if table_is_rtl && !is_message_rtl {
                header_row.reverse();
                for r in &mut data_rows {
                    r.reverse();
                }
            }

            let mut table_cells = Vec::with_capacity(data_rows.len() + 1);
            table_cells.push(header_row);
            table_cells.extend(data_rows);

            return (Some(table_cells), true, idx_line);
        }
    }

    // 2. Unicode Box or ASCII Grid Table (┌─┬─┐ or +---+---+)
    let is_unicode_box = line.chars().any(|c| "┌╔┏┬┰├┝┼╂".contains(c))
        || line
            .strip_prefix('│')
            .is_some_and(|rest| rest.contains('│'));
    let is_ascii_grid = line
        .strip_prefix('+')
        .is_some_and(|rest| rest.contains('+'))
        && (line.contains('-') || line.contains('='));

    if is_unicode_box || is_ascii_grid {
        let mut table_lines = Vec::new();
        let mut curr_i = i;

        while curr_i < n {
            let curr = lines[curr_i].trim();
            if curr.is_empty() {
                break;
            }
            if curr
                .chars()
                .any(|c| "┌╔┏┬┰├┝┼╂└╚┗┴┸┤┥│║┃|┐┘┒┙╗╝┚┖┓┛".contains(c))
                || curr
                    .strip_prefix('+')
                    .is_some_and(|rest| rest.contains('+'))
            {
                table_lines.push(curr);
                curr_i += 1;
            } else {
                break;
            }
        }

        if table_lines.len() >= 2 {
            let mut raw_rows = Vec::new();
            let mut has_header = false;
            let mut first_row_done = false;

            for l in &table_lines {
                if is_border_line(l) {
                    if first_row_done {
                        has_header = true;
                    }
                    continue;
                }
                let cols: Vec<String> = split_table_row_cells(l, true);

                if !cols.is_empty() && cols.iter().any(|c| !c.is_empty()) {
                    raw_rows.push(cols);
                    first_row_done = true;
                }
            }

            if !raw_rows.is_empty() {
                let headers_str: Vec<&str> = raw_rows[0].iter().map(|s| s.as_str()).collect();
                let table_is_rtl = if has_header {
                    rtl::is_table_predominantly_rtl(&headers_str)
                } else {
                    false
                };

                let col_is_rtl = {
                    let mut all_rows: Vec<Vec<&str>> = Vec::with_capacity(raw_rows.len());
                    for r in &raw_rows {
                        all_rows.push(r.iter().map(|s| s.as_str()).collect());
                    }
                    compute_column_rtl_flags(&all_rows)
                };
                let mut table_cells = Vec::with_capacity(raw_rows.len());

                for (r_idx, row_cols) in raw_rows.into_iter().enumerate() {
                    let is_hdr = r_idx == 0 && has_header;
                    let mut row: Vec<RichBlockTableCell> = row_cols
                        .into_iter()
                        .enumerate()
                        .map(|(idx, c)| {
                            let is_rtl = col_is_rtl.get(idx).copied().unwrap_or(false);
                            let align = resolve_table_cell_align(None, is_rtl, &c);
                            let is_numeric = is_ascii_numeric_cell(&c);
                            let formatted_c = if (table_is_rtl || is_rtl) && is_numeric {
                                rtl::to_eastern_arabic_digits(&c)
                            } else {
                                c
                            };
                            RichBlockTableCell::new(parse_inline(&formatted_c), is_hdr, Some(align))
                        })
                        .collect();
                    if table_is_rtl && !is_message_rtl {
                        row.reverse();
                    }
                    table_cells.push(row);
                }

                if table_cells.len() >= 2 || (!table_cells.is_empty() && has_header) {
                    return (Some(table_cells), has_header, curr_i);
                }
            }
        }
    }

    static RE_TABLE_UNDERLINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^-{3,}$").expect("valid static regex"));
    static RE_SPACE_SPLIT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s{2,}|\t+").expect("valid static regex"));

    // 3. Plain Underline Table: Header \n ---------------- \n Data
    let underline_match = i + 1 < n && RE_TABLE_UNDERLINE.is_match(lines[i + 1].trim());
    if underline_match {
        let cols_hdr: Vec<&str> = RE_SPACE_SPLIT
            .split(line)
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();

        if cols_hdr.len() >= 2 {
            let mut raw_rows: Vec<Vec<&str>> = vec![cols_hdr];
            let mut curr_i = i + 2;

            while curr_i < n {
                let curr = lines[curr_i].trim();
                if curr.is_empty() {
                    break;
                }
                if RE_TABLE_UNDERLINE.is_match(curr) {
                    curr_i += 1;
                    continue;
                }
                let data_cols: Vec<&str> = RE_SPACE_SPLIT
                    .split(curr)
                    .map(|c| c.trim())
                    .filter(|c| !c.is_empty())
                    .collect();
                if !data_cols.is_empty() {
                    raw_rows.push(data_cols);
                }
                curr_i += 1;
            }

            if raw_rows.len() >= 2 {
                let table_is_rtl = rtl::is_table_predominantly_rtl(&raw_rows[0]);

                let col_is_rtl = compute_column_rtl_flags(&raw_rows);
                let mut table_cells = Vec::with_capacity(raw_rows.len());

                for (r_idx, row_cols) in raw_rows.into_iter().enumerate() {
                    let is_hdr = r_idx == 0;
                    let mut row: Vec<RichBlockTableCell> = row_cols
                        .into_iter()
                        .enumerate()
                        .map(|(idx, c)| {
                            let is_rtl = col_is_rtl.get(idx).copied().unwrap_or(false);
                            let align = resolve_table_cell_align(None, is_rtl, c);
                            let is_numeric = is_ascii_numeric_cell(c);
                            let formatted_c = if (table_is_rtl || is_rtl) && is_numeric {
                                rtl::to_eastern_arabic_digits(c)
                            } else {
                                c.to_string()
                            };
                            RichBlockTableCell::new(parse_inline(&formatted_c), is_hdr, Some(align))
                        })
                        .collect();
                    if table_is_rtl && !is_message_rtl {
                        row.reverse();
                    }
                    table_cells.push(row);
                }

                return (Some(table_cells), true, curr_i);
            }
        }
    }

    (None, false, i)
}

fn extract_html_cite(text: &str) -> (String, Option<String>) {
    if let Some(start) = text.find("<cite>") {
        if let Some(end) = text[start + 6..].find("</cite>") {
            let credit = text[start + 6..start + 6 + end].trim().to_string();
            let mut body = text[..start].to_string();
            body.push_str(&text[start + 6 + end + 7..]);
            let credit_opt = if credit.is_empty() {
                None
            } else {
                Some(credit)
            };
            return (body.trim().to_string(), credit_opt);
        }
    }
    (text.to_string(), None)
}

fn extract_quote_credit(lines: &[String]) -> (String, Option<String>) {
    let combined = lines.join("\n");
    let (body, cite) = extract_html_cite(&combined);
    if cite.is_some() {
        return (body, cite);
    }
    if lines.len() > 1 {
        if let Some(last) = lines.last() {
            let trimmed = last.trim();
            for prefix in &["— ", "– ", "-- "] {
                if let Some(credit) = trimmed.strip_prefix(prefix) {
                    let credit = credit.trim();
                    if !credit.is_empty() {
                        let text = lines[..lines.len() - 1].join("\n");
                        return (text, Some(credit.to_string()));
                    }
                }
            }
        }
    }
    (combined, None)
}

/// Parse an accumulated streaming Markdown buffer without exposing syntax that
/// is still provisional. Completed syntax is rendered through the canonical
/// Rich Message parser; an incomplete tail is reduced to safe semantic text.
/// This lets a draft converge naturally without a second completion repaint.
pub fn parse_streaming_markdown_to_rich_blocks(text: &str) -> Vec<RichBlock> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let sanitized = sanitize_leaked_llm_artifacts(text);
    if sanitized.trim().is_empty() {
        return Vec::new();
    }

    let unstable_at = provisional_markdown_start(&sanitized).unwrap_or(sanitized.len());
    let mut blocks = parse_markdown_to_rich_blocks(&sanitized[..unstable_at]);
    if unstable_at < sanitized.len() {
        let provisional = sanitize_provisional_markdown(&sanitized[unstable_at..]);
        if !provisional.trim().is_empty() {
            blocks.push(RichBlock::Paragraph {
                text: Value::String(provisional),
            });
        }
    }
    blocks
}

fn provisional_markdown_start(text: &str) -> Option<usize> {
    let mut openings = Vec::new();

    // Fenced code dominates all inline syntax until the matching fence.
    let mut fence_open: Option<usize> = None;
    let mut offset = 0usize;
    for segment in text.split_inclusive('\n') {
        let trimmed = segment.trim_start();
        if trimmed.starts_with("```") {
            let marker = offset + (segment.len() - trimmed.len());
            if fence_open.is_some() {
                fence_open = None;
            } else {
                fence_open = Some(marker);
            }
        }
        offset += segment.len();
    }
    if let Some(index) = fence_open {
        openings.push(index);
    }

    // Inline code and emphasis are deliberately conservative: if a delimiter
    // is unmatched, the entire construct remains provisional rather than
    // flashing the raw opener to Telegram.
    for marker in ["**", "__", "`", "||", "~~", "++"] {
        let mut open: Option<usize> = None;
        let mut cursor = 0usize;
        while let Some(relative) = text[cursor..].find(marker) {
            let index = cursor + relative;
            if marker == "`" && text[index..].starts_with("```") {
                cursor = index + 3;
                continue;
            }
            open = if open.is_some() { None } else { Some(index) };
            cursor = index + marker.len();
        }
        if let Some(index) = open {
            openings.push(index);
        }
    }

    // A single underscore used as an emphasis opener is provisional. Limit
    // detection to word-boundary-ish positions so identifiers such as foo_bar
    // are not unnecessarily hidden.
    let mut underscore_open: Option<usize> = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (position, (index, ch)) in chars.iter().enumerate() {
        if *ch != '_' {
            continue;
        }
        let prev = position
            .checked_sub(1)
            .and_then(|p| chars.get(p))
            .map(|(_, c)| *c);
        let next = chars.get(position + 1).map(|(_, c)| *c);
        let delimiter_like = prev.is_none_or(|c| c.is_whitespace() || "([{>".contains(c))
            || next.is_none_or(|c| c.is_whitespace() || ".,!?;:)]}".contains(c));
        if delimiter_like {
            underscore_open = if underscore_open.is_some() {
                None
            } else {
                Some(*index)
            };
        }
    }
    if let Some(index) = underscore_open {
        openings.push(index);
    }

    // Line-oriented Markdown markers can themselves arrive split across chunks.
    // Keep an otherwise marker-only current line provisional until it becomes
    // a valid heading/divider/list item or ordinary text.
    let line_start = text.rfind('\n').map_or(0, |index| index + 1);
    let current_line = &text[line_start..];
    let leading_ws = current_line.len() - current_line.trim_start().len();
    let marker_start = line_start + leading_ws;
    let marker = current_line.trim();
    let incomplete_heading =
        !marker.is_empty() && marker.chars().all(|ch| ch == '#') && marker.chars().count() <= 6;
    let incomplete_divider = matches!(
        marker,
        "-" | "--" | "*" | "**" | "_" | "__" | "|" | "||" | "~" | "~~"
    );
    let incomplete_quote = matches!(marker, ">" | "**>" | ">>" | ">>>");
    let numeric_list_prefix = marker
        .strip_suffix('.')
        .or_else(|| marker.strip_suffix(')'));
    let incomplete_list = marker == "-"
        || marker == "*"
        || numeric_list_prefix.is_some_and(|prefix| {
            !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit())
        });
    if incomplete_heading || incomplete_divider || incomplete_list || incomplete_quote {
        openings.push(marker_start);
    }

    // Incomplete links: keep from `[` provisional until both `](` and `)` are
    // available. Nested link destinations are intentionally treated
    // conservatively rather than attempting a full Markdown grammar here.
    let mut search = 0usize;
    while let Some(rel) = text[search..].find('[') {
        let start = search + rel;
        let rest = &text[start + 1..];
        match rest.find(']') {
            None => {
                openings.push(start);
                break;
            }
            Some(close_rel) => {
                let after_close = start + 1 + close_rel + 1;
                if text[after_close..].starts_with('(') {
                    if let Some(dest_close) = text[after_close + 1..].find(')') {
                        search = after_close + 1 + dest_close + 1;
                    } else {
                        openings.push(start);
                        break;
                    }
                } else {
                    search = after_close;
                }
            }
        }
    }

    openings.into_iter().min()
}

static RE_UNCLOSED_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^\)]*$").expect("valid static regex"));
static RE_DRAFT_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*#{1,6}\s*").expect("valid static regex"));
static RE_DRAFT_LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:[-*•]|\d+[.)])\s+").expect("valid static regex"));
static RE_DRAFT_DIVIDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:-{1,}|\*{3,}|_{3,})\s*$").expect("valid static regex")
});
static RE_DRAFT_QUOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:\*\*>|>>>|>)\s*").expect("valid static regex"));

fn sanitize_provisional_markdown(tail: &str) -> String {
    let mut safe = tail
        .replace("```", "")
        .replace("**", "")
        .replace("__", "")
        .replace("||", "")
        .replace("~~", "")
        .replace("++", "");
    safe = safe.replace('`', "");

    safe = RE_UNCLOSED_LINK.replace_all(&safe, "$1").into_owned();
    safe = RE_DRAFT_HEADING.replace_all(&safe, "").into_owned();
    safe = RE_DRAFT_LIST.replace_all(&safe, "").into_owned();
    safe = RE_DRAFT_DIVIDER.replace_all(&safe, "").into_owned();
    safe = RE_DRAFT_QUOTE.replace_all(&safe, "").into_owned();

    // Remove only obvious unmatched edge delimiters; do not blanket-delete
    // underscores from identifiers or ordinary punctuation.
    let trimmed = safe
        .trim_start_matches(['_', '*', '[', '|', '~', '>'])
        .trim_end_matches(['_', '*', '[', ']', '|', '~', '>']);
    trimmed.to_string()
}

static RE_BLOCK_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s*([^\s#].*)$").expect("valid static regex"));
static RE_HTML_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)^<h([1-6])(?:\s+[^>]*)?>(.*?)</h[1-6]>$"#).expect("valid static regex")
});
static RE_HTML_HEADING_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)^<h([1-6])(?:\s+[^>]*)?>"#).expect("valid static regex"));
static RE_BLOCK_DIVIDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\-{3,}|\*{3,}|_{3,}|─{3,}|—{2,})$").expect("valid static regex")
});
static RE_BLOCK_BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-*•]\s+").expect("valid static regex"));
static RE_BLOCK_NUMBERED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+|[\u0660-\u0669]+)[\.)]\s+").expect("valid static regex"));

fn emit_math_or_quote_blocks(blocks: &mut Vec<RichBlock>, math_lines: &[String]) {
    if math_lines.iter().any(|l| rtl::has_rtl_characters(l)) {
        let clean_lines: Vec<String> = math_lines
            .iter()
            .map(|l| rtl::extract_text_from_pseudo_math(l.trim()))
            .filter(|l| !l.is_empty())
            .collect();
        if !clean_lines.is_empty() {
            let joined = clean_lines.join("\n");
            let lrm_text = rtl::ensure_lrm_if_needed(&joined, false);
            blocks.push(RichBlock::BlockQuotation {
                blocks: vec![json!({
                    "type": "paragraph",
                    "text": parse_inline(&lrm_text)
                })],
            });
        }
    } else {
        for line in math_lines {
            let trimmed_line = line.trim();
            if !trimmed_line.is_empty() {
                let sanitized = sanitize_latex_for_telegram(trimmed_line);
                if !sanitized.is_empty() {
                    blocks.push(RichBlock::MathematicalExpression {
                        expression: sanitized,
                    });
                }
            }
        }
    }
}

pub fn parse_markdown_to_rich_blocks(text: &str) -> Vec<RichBlock> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let sanitized = sanitize_leaked_llm_artifacts(text);
    if sanitized.trim().is_empty() {
        return Vec::new();
    }

    let is_message_rtl = rtl::is_rtl_text(&sanitized);
    let isolated = isolate_embedded_media_blocks(&sanitized);
    let lines: Vec<String> = isolated
        .replace("\r\n", "\n")
        .split('\n')
        .map(|s| s.to_string())
        .collect();
    let mut blocks: Vec<RichBlock> = Vec::new();

    let mut i = 0;
    let n = lines.len();

    while i < n {
        let line = &lines[i];
        let stripped = line.trim();

        // 1. Skip blank lines
        if stripped.is_empty() {
            i += 1;
            continue;
        }

        // 2. Fenced Code Block (```lang ... ```)
        if let Some(after_fence) = stripped.strip_prefix("```") {
            let lang = after_fence.trim();
            let language = if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            };
            let mut code_lines = Vec::new();
            i += 1;
            while i < n && !lines[i].trim().starts_with("```") {
                code_lines.push(lines[i].clone());
                i += 1;
            }
            if i < n && lines[i].trim().starts_with("```") {
                i += 1;
            }
            blocks.push(RichBlock::Preformatted {
                text: code_lines.join("\n"),
                language,
            });
            continue;
        }

        // 3. Math Block ($$...$$ or \[...\])
        if stripped.starts_with("$$") || stripped.starts_with(r"\[") {
            let is_bracket = stripped.starts_with(r"\[");
            let closing_token = if is_bracket { r"\]" } else { "$$" };
            let start_len = 2;
            let mut math_lines = Vec::new();

            if stripped.ends_with(closing_token) && stripped.len() > (start_len * 2) {
                math_lines.push(
                    stripped[start_len..stripped.len() - closing_token.len()]
                        .trim()
                        .to_string(),
                );
                i += 1;
            } else {
                if stripped.len() > start_len {
                    math_lines.push(stripped[start_len..].trim().to_string());
                }
                i += 1;
                while i < n && !lines[i].trim().ends_with(closing_token) {
                    math_lines.push(lines[i].clone());
                    i += 1;
                }
                if i < n && lines[i].trim().ends_with(closing_token) {
                    let end_line = lines[i].trim();
                    if end_line.len() > closing_token.len() {
                        math_lines.push(
                            end_line[..end_line.len() - closing_token.len()]
                                .trim()
                                .to_string(),
                        );
                    }
                    i += 1;
                }
            }

            emit_math_or_quote_blocks(&mut blocks, &math_lines);
            continue;
        }

        // Standalone LaTeX math formula line (\text{...} or \frac{...})
        if (stripped.starts_with(r"\text{")
            || stripped.starts_with(r"\frac")
            || stripped.starts_with(r"\sqrt"))
            && (stripped.contains(r"\frac")
                || stripped.contains('=')
                || stripped.contains(r"\times"))
        {
            let mut math_lines = vec![stripped.to_string()];
            i += 1;
            while i < n {
                let curr_s = lines[i].trim();
                if curr_s.is_empty()
                    || ![
                        r"\frac", r"\text", "=", r"\times", r"\sqrt", "^", "_", "+", "-", "{", "}",
                    ]
                    .iter()
                    .any(|k| curr_s.contains(k))
                {
                    break;
                }
                math_lines.push(curr_s.to_string());
                i += 1;
            }
            emit_math_or_quote_blocks(&mut blocks, &math_lines);
            continue;
        }

        // Map Block ([map: lat, lon] or <tg-map .../>)
        if stripped.starts_with("[map:")
            || stripped.starts_with("[location:")
            || stripped.starts_with("![map]")
            || stripped.starts_with("![location]")
            || stripped.starts_with("<tg-map")
        {
            if let Some(map_block) = try_parse_map_block(stripped) {
                blocks.push(map_block);
                i += 1;
                continue;
            }
        }

        // Document Block ([document: name](tg://...) or <tg-document .../>)
        if stripped.starts_with("[document:") || stripped.starts_with("<tg-document") {
            if let Some(doc_block) = try_parse_doc_block(stripped) {
                blocks.push(doc_block);
                i += 1;
                continue;
            }
        }

        // Multi-line or container Media Block (<tg-collage>...</tg-collage>, etc.)
        if stripped.starts_with("<tg-collage")
            || stripped.starts_with("<tg-slideshow")
            || (stripped.starts_with("[collage") && !stripped.contains('('))
            || (stripped.starts_with("[kolase") && !stripped.contains('('))
            || (stripped.starts_with("[slideshow") && !stripped.contains('('))
        {
            if let Some((container_block, next_i)) = try_parse_container_media_block(&lines, i) {
                blocks.push(container_block);
                i = next_i;
                continue;
            }
        }

        // Media Block (Photo, Video, Audio, VoiceNote, Animation, Collage, Slideshow)
        if let Some(media_block) = try_parse_media_block(stripped) {
            blocks.push(media_block);
            i += 1;
            continue;
        }

        // 4. Horizontal Divider (---, ***, ___, ───, or <hr>, <hr/>)
        if RE_BLOCK_DIVIDER.is_match(stripped)
            || stripped.eq_ignore_ascii_case("<hr>")
            || stripped.eq_ignore_ascii_case("<hr/>")
            || stripped.eq_ignore_ascii_case("<hr />")
        {
            blocks.push(RichBlock::Divider {});
            i += 1;
            continue;
        }

        // 5a. HTML Heading (<h1>...</h1> to <h6>...</h6>)
        if let Some(caps) = RE_HTML_HEADING_START.captures(stripped) {
            let level = caps
                .get(1)
                .and_then(|m| m.as_str().parse::<usize>().ok())
                .unwrap_or(1);
            let after_open = &stripped[caps.get(0).map_or(0, |m| m.end())..];
            let close_tag = format!("</h{level}>");

            if let Some(end) = after_open.to_lowercase().rfind(&close_tag) {
                let inner = after_open[..end].trim();
                blocks.push(RichBlock::SectionHeading {
                    text: parse_inline(inner),
                    level: level.min(6),
                });
                i += 1;
                continue;
            }
            if let Some(end) = after_open.to_lowercase().rfind("</h") {
                if let Some(_gt) = after_open[end..].find('>') {
                    let inner = after_open[..end].trim();
                    blocks.push(RichBlock::SectionHeading {
                        text: parse_inline(inner),
                        level: level.min(6),
                    });
                    i += 1;
                    continue;
                }
            }

            // Multi-line heading
            let mut h_lines = Vec::new();
            if !after_open.trim().is_empty() {
                h_lines.push(after_open.trim().to_string());
            }
            i += 1;
            while i < n {
                let line_str = lines[i].trim();
                let lower = line_str.to_lowercase();
                if let Some(end) = lower.rfind(&close_tag) {
                    let before = line_str[..end].trim();
                    if !before.is_empty() {
                        h_lines.push(before.to_string());
                    }
                    i += 1;
                    break;
                } else if let Some(end) = lower.rfind("</h") {
                    if let Some(_gt) = lower[end..].find('>') {
                        let before = line_str[..end].trim();
                        if !before.is_empty() {
                            h_lines.push(before.to_string());
                        }
                        i += 1;
                        break;
                    }
                }
                if !line_str.is_empty() {
                    h_lines.push(line_str.to_string());
                }
                i += 1;
            }
            let joined = h_lines.join(" ");
            blocks.push(RichBlock::SectionHeading {
                text: parse_inline(&joined),
                level: level.min(6),
            });
            continue;
        }

        // 5b. HTML Paragraph (<p>...</p>)
        if stripped.starts_with("<p>") || stripped.starts_with("<p ") {
            let mut p_lines = Vec::new();
            let mut curr = stripped.to_string();
            if let Some(pos) = curr.find('>') {
                curr = curr[pos + 1..].to_string();
            }
            if let Some(end) = curr.rfind("</p>") {
                let inner = curr[..end].trim();
                if !inner.is_empty() {
                    let lrm_text = rtl::ensure_lrm_if_needed(inner, is_message_rtl);
                    blocks.push(RichBlock::Paragraph {
                        text: parse_inline(&lrm_text),
                    });
                }
                i += 1;
                continue;
            }
            if !curr.trim().is_empty() {
                p_lines.push(curr.trim().to_string());
            }
            i += 1;
            while i < n {
                let line_str = lines[i].trim();
                if let Some(end) = line_str.rfind("</p>") {
                    let before = line_str[..end].trim();
                    if !before.is_empty() {
                        p_lines.push(before.to_string());
                    }
                    i += 1;
                    break;
                }
                if !line_str.is_empty() {
                    p_lines.push(line_str.to_string());
                }
                i += 1;
            }
            if !p_lines.is_empty() {
                let joined = p_lines.join(" ");
                let lrm_text = rtl::ensure_lrm_if_needed(&joined, is_message_rtl);
                blocks.push(RichBlock::Paragraph {
                    text: parse_inline(&lrm_text),
                });
            }
            continue;
        }

        // 5. Section Heading (# Heading, ## Subheading, etc.)
        if let Some(caps) = RE_BLOCK_HEADING.captures(stripped) {
            let level = caps.get(1).map(|m| m.as_str().len()).unwrap_or(1);
            let heading_text = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            blocks.push(RichBlock::SectionHeading {
                text: parse_inline(heading_text),
                level: level.min(6),
            });
            i += 1;
            continue;
        }

        // 6a. Pullquote (>>> quote)
        if stripped.starts_with(">>>") {
            let mut quote_lines = Vec::new();
            while i < n && lines[i].trim().starts_with(">>>") {
                let q = lines[i].trim();
                let stripped_q = q.strip_prefix(">>>").unwrap_or(q).trim_start();
                quote_lines.push(stripped_q.to_string());
                i += 1;
            }
            let (quote_text, credit) = extract_quote_credit(&quote_lines);
            blocks.push(RichBlock::PullQuotation {
                text: parse_inline(&quote_text),
                credit: credit.map(|c| parse_inline(&c)),
            });
            continue;
        }

        // 6b. Expandable Blockquote (**> quote or <blockquote expandable>)
        if stripped.starts_with("**>") {
            let mut quote_lines = Vec::new();
            while i < n {
                let curr_stripped = lines[i].trim();
                if curr_stripped.starts_with("**>") {
                    quote_lines.push(
                        curr_stripped
                            .strip_prefix("**>")
                            .unwrap_or(curr_stripped)
                            .trim_start()
                            .to_string(),
                    );
                    i += 1;
                } else if curr_stripped.starts_with('>') && !curr_stripped.starts_with(">>>") {
                    quote_lines.push(
                        curr_stripped
                            .strip_prefix('>')
                            .unwrap_or(curr_stripped)
                            .trim_start()
                            .to_string(),
                    );
                    i += 1;
                } else {
                    break;
                }
            }
            let (quote_text, credit) = extract_quote_credit(&quote_lines);
            blocks.push(RichBlock::ExpandableBlockQuotation {
                text: parse_inline(&quote_text),
                credit: credit.map(|c| parse_inline(&c)),
            });
            continue;
        }

        if stripped.starts_with("<blockquote") && stripped.contains("expandable") {
            let mut quote_lines = Vec::new();
            let mut first_line = stripped.to_string();
            if let Some(pos) = first_line.find('>') {
                first_line = first_line[pos + 1..].to_string();
            }
            if let Some(end) = first_line.find("</blockquote>") {
                let inner = first_line[..end].trim();
                let (quote_text, credit) = extract_html_cite(inner);
                blocks.push(RichBlock::ExpandableBlockQuotation {
                    text: parse_inline(&quote_text),
                    credit: credit.map(|c| parse_inline(&c)),
                });
                i += 1;
                continue;
            }
            if !first_line.trim().is_empty() {
                quote_lines.push(first_line.trim().to_string());
            }
            i += 1;
            while i < n {
                let curr = lines[i].trim();
                if let Some(end) = curr.find("</blockquote>") {
                    let before = curr[..end].trim();
                    if !before.is_empty() {
                        quote_lines.push(before.to_string());
                    }
                    i += 1;
                    break;
                }
                quote_lines.push(curr.to_string());
                i += 1;
            }
            let (quote_text, credit) = extract_quote_credit(&quote_lines);
            blocks.push(RichBlock::ExpandableBlockQuotation {
                text: parse_inline(&quote_text),
                credit: credit.map(|c| parse_inline(&c)),
            });
            continue;
        }

        // 6c. HTML Blockquote (<blockquote> ... </blockquote>)
        if stripped.starts_with("<blockquote") && !stripped.contains("expandable") {
            let mut quote_lines = Vec::new();
            let mut first_line = stripped.to_string();
            if let Some(pos) = first_line.find('>') {
                first_line = first_line[pos + 1..].to_string();
            }
            if let Some(end) = first_line.find("</blockquote>") {
                let inner = first_line[..end].trim();
                let lrm_text = rtl::ensure_lrm_if_needed(inner, is_message_rtl);
                blocks.push(RichBlock::BlockQuotation {
                    blocks: vec![json!({
                        "type": "paragraph",
                        "text": parse_inline(&lrm_text)
                    })],
                });
                i += 1;
                continue;
            }
            if !first_line.trim().is_empty() {
                quote_lines.push(first_line.trim().to_string());
            }
            i += 1;
            while i < n {
                let curr = lines[i].trim();
                if let Some(end) = curr.find("</blockquote>") {
                    let before = curr[..end].trim();
                    if !before.is_empty() {
                        quote_lines.push(before.to_string());
                    }
                    i += 1;
                    break;
                }
                quote_lines.push(curr.to_string());
                i += 1;
            }
            let joined = quote_lines.join("\n");
            let lrm_text = rtl::ensure_lrm_if_needed(&joined, is_message_rtl);
            blocks.push(RichBlock::BlockQuotation {
                blocks: vec![json!({
                    "type": "paragraph",
                    "text": parse_inline(&lrm_text)
                })],
            });
            continue;
        }

        // 6. Blockquote (> quote)
        if stripped.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < n && lines[i].trim().starts_with('>') && !lines[i].trim().starts_with(">>>") {
                let q = lines[i].trim();
                let stripped_q = q.strip_prefix('>').unwrap_or(q).trim_start();
                quote_lines.push(stripped_q.to_string());
                i += 1;
            }
            if let Some(first) = quote_lines.first_mut() {
                let alerts = [
                    ("[!NOTE]", "ℹ️ **Catatan:**"),
                    ("[!note]", "ℹ️ **Catatan:**"),
                    ("[!TIP]", "💡 **Tips:**"),
                    ("[!tip]", "💡 **Tips:**"),
                    ("[!IMPORTANT]", "📌 **Penting:**"),
                    ("[!important]", "📌 **Penting:**"),
                    ("[!WARNING]", "⚠️ **Peringatan:**"),
                    ("[!warning]", "⚠️ **Peringatan:**"),
                    ("[!CAUTION]", "🚨 **Perhatian:**"),
                    ("[!caution]", "🚨 **Perhatian:**"),
                ];
                for (marker, replacement) in alerts {
                    if first.starts_with(marker) {
                        let rest = first[marker.len()..].trim();
                        if rest.is_empty() {
                            *first = replacement.to_string();
                        } else {
                            *first = format!("{replacement} {rest}");
                        }
                        break;
                    }
                }
            }
            let joined = quote_lines.join("\n");
            let lrm_text = rtl::ensure_lrm_if_needed(&joined, is_message_rtl);
            blocks.push(RichBlock::BlockQuotation {
                blocks: vec![json!({
                    "type": "paragraph",
                    "text": parse_inline(&lrm_text)
                })],
            });
            continue;
        }

        // 7. Table (Markdown, Unicode, ASCII, Underline)
        if let Some(inner) = stripped
            .strip_prefix("[table:")
            .or_else(|| stripped.strip_prefix("[caption:"))
            .and_then(|r| r.strip_suffix(']'))
        {
            let cap = inner.trim();
            if !cap.is_empty() && i + 1 < n {
                let (t_cells, has_hdr, next_i) = try_parse_table(&lines, i + 1, is_message_rtl);
                if let Some(cells) = t_cells {
                    blocks.push(RichBlock::Table {
                        cells,
                        has_header: has_hdr,
                        is_bordered: false,
                        is_striped: false,
                        is_compact: true,
                        caption: Some(cap.to_string()),
                    });
                    i = next_i;
                    continue;
                }
            }
        }

        let (t_cells, has_hdr, next_i) = try_parse_table(&lines, i, is_message_rtl);
        if let Some(cells) = t_cells {
            let mut caption: Option<String> = None;
            let mut final_next_i = next_i;
            if final_next_i < n {
                let next_line = lines[final_next_i].trim();
                if let Some(inner) = next_line
                    .strip_prefix("[table:")
                    .or_else(|| next_line.strip_prefix("[caption:"))
                    .and_then(|r| r.strip_suffix(']'))
                {
                    let cap = inner.trim();
                    if !cap.is_empty() {
                        caption = Some(cap.to_string());
                        final_next_i += 1;
                    }
                }
            }
            blocks.push(RichBlock::Table {
                cells,
                has_header: has_hdr,
                is_bordered: false,
                is_striped: false,
                is_compact: true,
                caption,
            });
            i = final_next_i;
            continue;
        }

        // 8. List Items (- item, * item, 1. item)
        let is_bullet = RE_BLOCK_BULLET.is_match(stripped);
        let is_numbered = RE_BLOCK_NUMBERED.is_match(stripped);

        if is_bullet || is_numbered {
            let mut list_items = Vec::new();
            let is_ordered = is_numbered;

            while i < n {
                let curr = lines[i].trim();
                if curr.is_empty() {
                    break;
                }
                if is_ordered && RE_BLOCK_NUMBERED.is_match(curr) {
                    let item_text = RE_BLOCK_NUMBERED.replace(curr, "").trim().to_string();
                    let value = curr.split_once(['.', ')']).and_then(|(prefix, _)| {
                        let prefix_clean = prefix.trim();
                        let ascii = rtl::from_eastern_arabic_digits(prefix_clean);
                        ascii
                            .parse::<i64>()
                            .ok()
                            .or_else(|| prefix_clean.parse::<i64>().ok())
                    });
                    let lrm_text = rtl::ensure_lrm_if_needed(&item_text, is_message_rtl);
                    list_items.push(RichBlockListItem::ordered(
                        vec![json!({
                            "type": "paragraph",
                            "text": parse_inline(&lrm_text)
                        })],
                        value,
                    ));
                    i += 1;
                } else if !is_ordered && RE_BLOCK_BULLET.is_match(curr) {
                    let item_text = RE_BLOCK_BULLET.replace(curr, "").trim().to_string();
                    let lrm_text = rtl::ensure_lrm_if_needed(&item_text, is_message_rtl);
                    list_items.push(RichBlockListItem::bullet(vec![json!({
                        "type": "paragraph",
                        "text": parse_inline(&lrm_text)
                    })]));
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(RichBlock::List { items: list_items });
            continue;
        }

        // 9. Regular Paragraph
        let mut para_lines = Vec::new();
        while i < n {
            let curr = &lines[i];
            let s_curr = curr.trim();
            if s_curr.is_empty()
                || s_curr.starts_with("```")
                || s_curr.starts_with("$$")
                || s_curr.starts_with(r"\[")
                || RE_BLOCK_HEADING.is_match(s_curr)
                || RE_HTML_HEADING.is_match(s_curr)
                || s_curr.starts_with("<p>")
                || s_curr.starts_with("<p ")
                || s_curr.eq_ignore_ascii_case("<hr>")
                || s_curr.eq_ignore_ascii_case("<hr/>")
                || s_curr.eq_ignore_ascii_case("<hr />")
                || s_curr.starts_with("**>")
                || s_curr.starts_with("<blockquote")
                || s_curr.starts_with('>')
                || s_curr.starts_with(">>>")
                || s_curr.starts_with("[table:")
                || s_curr.starts_with("[caption:")
                || s_curr.starts_with("<tg-map")
                || s_curr.starts_with("<tg-document")
                || s_curr.starts_with("<tg-collage")
                || s_curr.starts_with("<tg-slideshow")
                || s_curr.starts_with("<audio")
                || s_curr.starts_with("<tg-audio")
                || s_curr.starts_with("<img")
                || s_curr.starts_with("<tg-photo")
                || try_parse_media_block(s_curr).is_some()
                || try_parse_doc_block(s_curr).is_some()
                || try_parse_map_block(s_curr).is_some()
                || RE_BLOCK_BULLET.is_match(s_curr)
                || RE_BLOCK_NUMBERED.is_match(s_curr)
                || RE_BLOCK_DIVIDER.is_match(s_curr)
                || try_parse_table(&lines, i, is_message_rtl).0.is_some()
            {
                break;
            }
            para_lines.push(curr.clone());
            i += 1;
        }

        if !para_lines.is_empty() {
            let joined = para_lines.join("\n");
            let lrm_text = rtl::ensure_lrm_if_needed(&joined, is_message_rtl);
            blocks.push(RichBlock::Paragraph {
                text: parse_inline(&lrm_text),
            });
        }
    }

    blocks
}

pub fn build_full_rich_message(answer_text: &str, footer_text: Option<&str>) -> InputRichMessage {
    let mut blocks = parse_markdown_to_rich_blocks(answer_text);
    if blocks.is_empty() {
        let is_msg_rtl = rtl::is_rtl_text(answer_text);
        let lrm_text = rtl::ensure_lrm_if_needed(answer_text.trim(), is_msg_rtl);
        blocks.push(RichBlock::Paragraph {
            text: parse_inline(&lrm_text),
        });
    }
    if let Some(footer) = footer_text.map(str::trim).filter(|m| !m.is_empty()) {
        blocks.push(RichBlock::Footer {
            text: parse_inline(footer),
        });
    }
    let mut message = InputRichMessage::new(blocks);
    rtl::apply_rtl_direction(&mut message, answer_text);
    message
}

/// Validates container tags and geographic map parameters within rich HTML.
#[allow(dead_code)]
fn validate_rich_html_containers(html: &str) -> Result<(), ParserError> {
    static RE_COLLAGE_CONTAINER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<tg-collage(?:\s+[^>]*)?>(.*?)</tg-collage>"#)
            .expect("valid static regex")
    });

    for caps in RE_COLLAGE_CONTAINER.captures_iter(html) {
        let content = caps.get(1).map_or("", |m| m.as_str());
        let lower = content.to_lowercase();
        if lower.contains("<audio")
            || lower.contains("<tg-audio")
            || lower.contains("<tg-document")
            || lower.contains("<document")
        {
            return Err(ParserError::InvalidTag(
                "Collage album cannot mix audio or documents with visual items".to_string(),
            ));
        }
    }

    static RE_MAP_TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<tg-map(?:\s+[^>]*)?/?>"#).expect("valid static regex")
    });

    for caps in RE_MAP_TAG.captures_iter(html) {
        let tag = caps.get(0).map_or("", |m| m.as_str());
        let lat_s = extract_html_attribute(tag, "lat").ok_or_else(|| {
            ParserError::InvalidCoordinate("Missing 'lat' attribute on <tg-map>".to_string())
        })?;
        let lon_s = extract_html_attribute(tag, "lon").ok_or_else(|| {
            ParserError::InvalidCoordinate("Missing 'lon' attribute on <tg-map>".to_string())
        })?;
        let lat = lat_s.parse::<f64>().map_err(|_| {
            ParserError::InvalidCoordinate(format!("Invalid latitude float value: '{lat_s}'"))
        })?;
        let lon = lon_s.parse::<f64>().map_err(|_| {
            ParserError::InvalidCoordinate(format!("Invalid longitude float value: '{lon_s}'"))
        })?;

        if !lat.is_finite() || !lon.is_finite() {
            return Err(ParserError::InvalidCoordinate(
                "Geo coordinates must be finite numbers".to_string(),
            ));
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(ParserError::InvalidCoordinate(format!(
                "Latitude {lat} out of range [-90.0, 90.0]"
            )));
        }
        if !(-180.0..=180.0).contains(&lon) {
            return Err(ParserError::InvalidCoordinate(format!(
                "Longitude {lon} out of range [-180.0, 180.0]"
            )));
        }

        if let Some(zoom_s) = extract_html_attribute(tag, "zoom") {
            let zoom = zoom_s.parse::<i32>().map_err(|_| {
                ParserError::InvalidCoordinate(format!("Invalid zoom integer value: '{zoom_s}'"))
            })?;
            if !(1..=20).contains(&zoom) {
                return Err(ParserError::InvalidCoordinate(format!(
                    "Zoom level {zoom} out of range [1, 20]"
                )));
            }
        }
    }

    Ok(())
}

/// Extracts and validates media items referenced in HTML tags, resolving their IDs and metadata.
#[allow(dead_code)]
pub fn extract_rich_html_media(html: &str) -> Result<Vec<InputRichMessageMedia>, ParserError> {
    static RE_HTML_MEDIA_TAGS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<(img|tg-photo|audio|tg-audio|video|tg-video|tg-document|document)(?:\s+[^>]*?)(?:/>|>.*?</(?:img|tg-photo|audio|tg-audio|video|tg-video|tg-document|document)>|>)"#)
            .expect("valid static regex")
    });

    let mut media_items: Vec<InputRichMessageMedia> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut counter: usize = 1;

    for caps in RE_HTML_MEDIA_TAGS.captures_iter(html) {
        let tag_match = caps.get(0).map_or("", |m| m.as_str());
        let tag_name = caps.get(1).map_or("", |m| m.as_str()).to_lowercase();

        let src = match extract_html_attribute(tag_match, "src") {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => continue,
        };

        let is_photo = matches!(tag_name.as_str(), "img" | "tg-photo");
        let is_audio = matches!(tag_name.as_str(), "audio" | "tg-audio");
        let is_video = matches!(tag_name.as_str(), "video" | "tg-video");
        let is_doc = matches!(tag_name.as_str(), "tg-document" | "document");

        let id = if let Some(stripped) = src.strip_prefix("tg://") {
            let (scheme_type, query) = stripped.split_once('?').unwrap_or((stripped, ""));
            let extracted_id = if let Some(id_val) = query.strip_prefix("id=") {
                id_val.split('&').next().unwrap_or(id_val)
            } else if let Some((_, val)) = query
                .split('&')
                .filter_map(|p| p.split_once('='))
                .find(|(k, _)| *k == "id")
            {
                val
            } else {
                ""
            };

            if extracted_id.is_empty() {
                return Err(ParserError::InvalidTag(format!(
                    "Missing ID in tg:// scheme: '{src}'"
                )));
            }

            if (is_photo && scheme_type != "photo")
                || (is_audio && scheme_type != "audio")
                || (is_video && scheme_type != "video")
                || (is_doc && scheme_type != "document")
            {
                return Err(ParserError::InvalidTag(format!(
                    "Tag <{tag_name}> cannot reference scheme 'tg://{scheme_type}'"
                )));
            }

            InputRichMessageMedia::validate_id(extracted_id)
                .map_err(ParserError::MediaValidation)?;
            extracted_id.to_string()
        } else if let Some(key) = src.strip_prefix("attach://") {
            let extracted_id = extract_html_attribute(tag_match, "id").unwrap_or(key);
            InputRichMessageMedia::validate_id(extracted_id)
                .map_err(ParserError::MediaValidation)?;
            extracted_id.to_string()
        } else {
            if let Some(explicit_id) = extract_html_attribute(tag_match, "id") {
                InputRichMessageMedia::validate_id(explicit_id)
                    .map_err(ParserError::MediaValidation)?;
                explicit_id.to_string()
            } else {
                let prefix = if is_photo {
                    "photo"
                } else if is_audio {
                    "audio"
                } else if is_video {
                    "video"
                } else {
                    "doc"
                };
                let generated = format!("{prefix}_{counter}");
                counter += 1;
                generated
            }
        };

        let caption = extract_html_attribute(tag_match, "caption")
            .or_else(|| {
                if is_photo {
                    extract_html_attribute(tag_match, "alt")
                } else {
                    None
                }
            })
            .map(str::to_string);

        let input_media = if is_photo {
            InputMedia::photo(src, caption, None)
        } else if is_audio {
            let title = extract_html_attribute(tag_match, "title").map(str::to_string);
            let performer = extract_html_attribute(tag_match, "performer").map(str::to_string);
            InputMedia::audio(src, caption, None, title, performer)
        } else if is_video {
            InputMedia::video(src, caption, None)
        } else if is_doc {
            InputMedia::document(src, caption, None)
        } else {
            continue;
        };

        let media_item = InputRichMessageMedia {
            id: id.clone(),
            media: input_media,
        };
        media_item
            .validate()
            .map_err(ParserError::MediaValidation)?;

        if seen_ids.insert(id.clone()) {
            if media_items.len() >= 50 {
                return Err(ParserError::MediaValidation(
                    "Rich Message media count exceeds Telegram limit of 50".to_string(),
                ));
            }
            media_items.push(media_item);
        } else if let Some(existing) = media_items.iter().find(|m| m.id == id) {
            if existing.media.media_url() != src {
                return Err(ParserError::MediaValidation(format!(
                    "Duplicate media ID found with conflicting URL: '{id}'"
                )));
            }
        }
    }

    Ok(media_items)
}

/// Normalizes HTML blocks by ensuring block-level tags reside on separate lines.
#[allow(dead_code)]
fn normalize_html_blocks(html: &str) -> String {
    static RE_NORM_BLOCKS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)(</?(?:p|h[1-6]|blockquote|pre|table|hr|tg-collage|tg-slideshow|tg-map|audio|tg-audio|img|tg-photo|video|tg-video|tg-document|document)(?:\s+[^>]*)?/?>)"#)
            .expect("valid static regex")
    });

    let mut result = String::with_capacity(html.len() + 128);
    let mut last_end = 0;

    for mat in RE_NORM_BLOCKS.find_iter(html) {
        let text_before = &html[last_end..mat.start()];
        let tag = mat.as_str().trim();

        if !text_before.trim().is_empty() {
            result.push_str(text_before);
        }

        let is_closing = tag.starts_with("</");
        let is_self_closing = tag.ends_with("/>")
            || tag.eq_ignore_ascii_case("<hr>")
            || tag.to_lowercase().starts_with("<hr ")
            || tag.to_lowercase().starts_with("<img")
            || tag.to_lowercase().starts_with("<tg-photo")
            || tag.to_lowercase().starts_with("<tg-map")
            || (tag.to_lowercase().starts_with("<audio")
                && !html.to_lowercase().contains("</audio>"))
            || (tag.to_lowercase().starts_with("<tg-audio")
                && !html.to_lowercase().contains("</tg-audio>"));

        if is_closing {
            result.push_str(tag);
            result.push_str("\n\n");
        } else if is_self_closing {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(tag);
            result.push_str("\n\n");
        } else {
            // Opening block tag
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(tag);
        }

        last_end = mat.end();
    }

    let remaining = &html[last_end..];
    if !remaining.trim().is_empty() {
        result.push_str(remaining);
    }

    result
}

/// Helper function to parse HTML rich message representation with media resolution.
/// Parses `<img>`, `<audio>`, `<tg-collage>`, `<tg-slideshow>`, `<tg-map>` as well as
/// HTML typography tags into `Vec<RichBlock>` and resolves/extracts `Vec<InputRichMessageMedia>`.
#[allow(dead_code)]
pub fn parse_rich_html(
    html: &str,
) -> Result<(Vec<RichBlock>, Vec<InputRichMessageMedia>), ParserError> {
    let trimmed = html.trim();
    if trimmed.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    if trimmed.chars().count() > 32_768 {
        return Err(ParserError::MalformedHtml(
            "Rich Message HTML text exceeds Telegram limit of 32768 characters".to_string(),
        ));
    }

    // 1. Validate container integrity and geographic map parameters
    validate_rich_html_containers(trimmed)?;

    // 2. Extract and resolve media objects
    let media = extract_rich_html_media(trimmed)?;

    // 3. Normalize HTML blocks and parse to RichBlocks
    let normalized = normalize_html_blocks(trimmed);
    let mut blocks = parse_markdown_to_rich_blocks(&normalized);

    // Normalize voice_note wire discriminators
    for block in &mut blocks {
        if let RichBlock::VoiceNote { voice_note, .. } = block {
            if let Some(object) = voice_note.as_object_mut() {
                object.insert("type".to_string(), Value::String("voice_note".to_string()));
            }
        }
    }

    if blocks.len() > 500 {
        return Err(ParserError::MalformedHtml(format!(
            "Rich Message contains {} blocks; Telegram limit is 500",
            blocks.len()
        )));
    }

    Ok((blocks, media))
}

/// Resolves media references in `blocks` against `media_items`, replacing `tg://` scheme URIs
/// with their actual target media URLs.
#[allow(dead_code)]
pub fn resolve_media_references(blocks: &mut [RichBlock], media_items: &[InputRichMessageMedia]) {
    let map: std::collections::HashMap<&str, &str> = media_items
        .iter()
        .map(|item| (item.id.as_str(), item.media.media_url()))
        .collect();

    for block in blocks {
        block.replace_media_urls(&|url| {
            for prefix in &[
                "tg://photo?id=",
                "tg://audio?id=",
                "tg://video?id=",
                "tg://document?id=",
            ] {
                if let Some(id) = url.strip_prefix(prefix) {
                    if let Some(target_url) = map.get(id) {
                        return Some(target_url.to_string());
                    }
                }
            }
            None
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_box_table_parses_without_byte_boundary_slicing() {
        let input =
            "┌──────┬──────┐\n│ Nama │ Ikon │\n├──────┼──────┤\n│ 世界 │ 😊   │\n└──────┴──────┘";
        let blocks = parse_markdown_to_rich_blocks(input);
        assert!(blocks
            .iter()
            .any(|block| matches!(block, RichBlock::Table { .. })));
    }

    #[test]
    fn ordered_list_preserves_native_ordering_metadata() {
        let blocks = parse_markdown_to_rich_blocks("5. lima\n6. enam");
        let RichBlock::List { items } = &blocks[0] else {
            panic!("expected list");
        };
        assert_eq!(items[0].kind.as_deref(), Some("1"));
        assert_eq!(items[0].value, Some(5));
        assert_eq!(items[1].value, Some(6));
    }

    #[test]
    fn emoji_and_multibyte_inline_text_survive_parser() {
        let value = parse_inline("Halo █ 😊 世界 **tebal**");
        let serialized = serde_json::to_string(&value).expect("serialize value succeeds");
        assert!(serialized.contains("世界"));
        assert!(serialized.contains("😊"));
    }

    #[test]
    fn streaming_markdown_never_exposes_provisional_serialization_markers() {
        let cases = [
            "Ini **gaya gravitasi** selesai",
            "Ini _italic_ selesai",
            "Gunakan `kode` sekarang",
            "```rust\nfn main() {}\n```",
            "### Heading tumbuh",
            "---",
            "[OpenAI](https://example.com/path)",
            "1. pertama\n2. kedua",
            "- satu\n- dua",
            "Emoji 😊 世界 **tebal**",
        ];

        for source in cases {
            let mut boundaries: Vec<usize> =
                source.char_indices().map(|(index, _)| index).collect();
            boundaries.push(source.len());
            boundaries.sort_unstable();
            boundaries.dedup();
            for end in boundaries.into_iter().filter(|end| *end > 0) {
                let prefix = &source[..end];
                let blocks = parse_streaming_markdown_to_rich_blocks(prefix);
                let wire = serde_json::to_string(&blocks).expect("serialize blocks succeeds");
                assert!(
                    !wire.contains("**"),
                    "bold marker leaked for {prefix:?}: {wire}"
                );
                assert!(
                    !wire.contains("__"),
                    "emphasis marker leaked for {prefix:?}: {wire}"
                );
                assert!(
                    !wire.contains("```"),
                    "fence marker leaked for {prefix:?}: {wire}"
                );
                assert!(
                    !wire.contains("]("),
                    "link serialization leaked for {prefix:?}: {wire}"
                );
                if prefix.trim().chars().all(|ch| ch == '#') {
                    assert!(
                        !wire.contains('#'),
                        "heading marker leaked for {prefix:?}: {wire}"
                    );
                }
                if matches!(prefix.trim(), "-" | "--") {
                    assert!(
                        !wire.contains(prefix.trim()),
                        "divider marker leaked for {prefix:?}: {wire}"
                    );
                }
            }
        }
    }

    #[test]
    fn collage_slideshow_audio_voice_parse_correctly() {
        let text = "[audio: Judul Musik](https://example.com/song.mp3)

[voice: Rekaman Suara](tg://audio?id=rec1)

[collage: Galeri](url1, url2)

[slideshow: Slide](url3, url4)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert!(blocks.iter().any(|b| matches!(b, RichBlock::Audio { .. })));
        assert!(blocks
            .iter()
            .any(|b| matches!(b, RichBlock::VoiceNote { .. })));
        assert!(blocks
            .iter()
            .any(|b| matches!(b, RichBlock::Collage { .. })));
        assert!(blocks
            .iter()
            .any(|b| matches!(b, RichBlock::Slideshow { .. })));
    }

    #[test]
    fn media_blocks_tolerate_whitespace_between_bracket_and_parenthesis() {
        let text = "[audio: Suara Contoh] (https://upload.wikimedia.org/wikipedia/commons/c/c8/Example.ogg)\n\n[photo: Foto Indah]  (https://example.com/pic.jpg)\n\n[collage: Galeri] (url1, url2)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], RichBlock::Audio { .. }));
        assert!(matches!(blocks[1], RichBlock::Photo { .. }));
        assert!(matches!(blocks[2], RichBlock::Collage { .. }));
    }

    #[test]
    fn map_and_document_blocks_parse_correctly() {
        let text = "[map: -6.175392, 106.827153, zoom=15]

[document: Laporan.pdf](tg://document?id=laporan_1)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert!(blocks.iter().any(|b| matches!(b, RichBlock::Map { .. })));
        assert!(blocks
            .iter()
            .any(|b| matches!(b, RichBlock::Document { .. })));
    }

    #[test]
    fn pullquote_and_footer_parse_correctly() {
        let text = ">>> Ini adalah kutipan penting

Paragraf normal";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert!(blocks
            .iter()
            .any(|b| matches!(b, RichBlock::PullQuotation { .. })));

        let full = build_full_rich_message("Jawaban AI", Some("`⚡ 3.0s`"));
        let footer = full
            .blocks
            .iter()
            .find_map(|b| match b {
                RichBlock::Footer { text } => Some(text),
                _ => None,
            })
            .expect("footer block should exist");
        let serialized = serde_json::to_string(footer).expect("serialize footer succeeds");
        assert!(serialized.contains("3.0s"));
        assert!(serialized.contains("⚡"));
        assert!(serialized.contains("code"));
    }

    #[test]
    fn markdown_image_and_media_is_media_check() {
        let text = "Penjelasan aurora:\n\n![Cahaya Aurora](https://picsum.photos/1000/600)\n\n[photo: Tromso](https://picsum.photos/800/600)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[0].is_media());
        assert!(blocks[1].is_media());
        assert!(blocks[2].is_media());
        assert!(matches!(blocks[1], RichBlock::Photo { .. }));
        assert!(matches!(blocks[2], RichBlock::Photo { .. }));
    }

    #[test]
    fn completed_streaming_markdown_converges_to_canonical_parser() {
        let source = "## Judul\n\n**tebal** dan _miring_\n\n---\n\n1. satu\n2. dua";
        let streaming = parse_streaming_markdown_to_rich_blocks(source);
        let canonical = parse_markdown_to_rich_blocks(source);
        assert_eq!(
            serde_json::to_value(streaming).expect("serialize streaming succeeds"),
            serde_json::to_value(canonical).expect("serialize canonical succeeds")
        );
    }

    #[test]
    fn spoiler_and_strikethrough_parse_correctly() {
        let markdown = "Info: ||rahasia besar|| dan ~~harga lama~~";
        let parsed = parse_inline(markdown);
        let serialized = serde_json::to_string(&parsed).expect("serialize parsed succeeds");
        assert!(serialized.contains(r#""type":"spoiler""#));
        assert!(serialized.contains("rahasia besar"));
        assert!(serialized.contains(r#""type":"strikethrough""#));
        assert!(serialized.contains("harga lama"));

        let html = "Tag: <tg-spoiler>kunci rahasia</tg-spoiler> dan <s>coret html</s>";
        let parsed_html = parse_inline(html);
        let serialized_html =
            serde_json::to_string(&parsed_html).expect("serialize parsed_html succeeds");
        assert!(serialized_html.contains(r#""type":"spoiler""#));
        assert!(serialized_html.contains("kunci rahasia"));
        assert!(serialized_html.contains(r#""type":"strikethrough""#));
        assert!(serialized_html.contains("coret html"));
    }

    #[test]
    fn expandable_blockquote_parses_correctly() {
        let markdown = "**> Baris penalaran pertama\n**> Baris penalaran kedua\n**> — As-tsaqib";
        let blocks = parse_markdown_to_rich_blocks(markdown);
        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::ExpandableBlockQuotation { text, credit }) = blocks.first() else {
            panic!("expected expandable blockquote");
        };
        let text_str = serde_json::to_string(text).expect("serialize text succeeds");
        assert!(text_str.contains("Baris penalaran pertama"));
        assert!(text_str.contains("Baris penalaran kedua"));
        assert!(credit.is_some());
        let credit_str = serde_json::to_string(&credit).expect("serialize credit succeeds");
        assert!(credit_str.contains("As-tsaqib"));

        let html =
            "<blockquote expandable>Catatan terlipat penting<cite>Dokumentasi</cite></blockquote>";
        let blocks_html = parse_markdown_to_rich_blocks(html);
        assert_eq!(blocks_html.len(), 1);
        let Some(RichBlock::ExpandableBlockQuotation {
            text: h_text,
            credit: h_credit,
        }) = blocks_html.first()
        else {
            panic!("expected HTML expandable blockquote");
        };
        assert!(serde_json::to_string(h_text)
            .expect("serialize h_text succeeds")
            .contains("Catatan terlipat penting"));
        assert!(serde_json::to_string(h_credit)
            .expect("serialize h_credit succeeds")
            .contains("Dokumentasi"));
    }

    #[test]
    fn table_compact_and_caption_parse_correctly() {
        let text = "[table: Perbandingan Spesifikasi]\n| Model | Konteks |\n| :--- | :---: |\n| GPT-4o | 128k |\n| Claude | 200k |";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Table {
            cells,
            is_compact,
            is_bordered,
            is_striped,
            caption,
            has_header,
            ..
        }) = blocks.first()
        else {
            panic!("expected rich table block");
        };
        assert!(*is_compact);
        assert!(!*is_bordered);
        assert!(!*is_striped);
        assert!(*has_header);
        assert_eq!(caption.as_deref(), Some("Perbandingan Spesifikasi"));
        assert_eq!(cells.len(), 3);
    }

    #[test]
    fn tg_document_links_and_underline_parse_correctly() {
        let text = "Tautan: [Buka File](tg://document?id=doc_abc123) dan <u>garis bawah</u> serta ++format ins++";
        let parsed = parse_inline(text);
        let serialized = serde_json::to_string(&parsed).expect("serialize parsed succeeds");
        assert!(serialized.contains(r#""type":"url""#));
        assert!(serialized.contains("tg://document?id=doc_abc123"));
        assert!(serialized.contains("Buka File"));
        assert!(serialized.contains(r#""type":"underline""#));
        assert!(serialized.contains("garis bawah"));
        assert!(serialized.contains("format ins"));
    }

    #[test]
    fn indonesian_and_case_insensitive_media_tags_parse_correctly() {
        let text = "[foto: Kucing Anggora](https://example.com/cat.jpg)\n\n[Foto : Kucing Lucu]  ( https://example.com/cat2.jpg ).\n\n[gambar: Pantai](https://example.com/beach.jpg)\n\n[dokumen: Laporan Keuangan](https://example.com/laporan.pdf)\n\n[file: Data Excel](https://example.com/data.xlsx)\n\n[musik: Suara Hujan](https://example.com/rain.mp3)\n\n[rekaman: Catatan Suara](https://example.com/voice.ogg)\n\n[lokasi: Monas, Jakarta](-6.175392, 106.827153)\n\n[kolase: Liburan](url1, url2)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 9);
        assert!(matches!(blocks[0], RichBlock::Photo { .. }));
        assert!(matches!(blocks[1], RichBlock::Photo { .. }));
        assert!(matches!(blocks[2], RichBlock::Photo { .. }));
        assert!(matches!(blocks[3], RichBlock::Document { .. }));
        assert!(matches!(blocks[4], RichBlock::Document { .. }));
        assert!(matches!(blocks[5], RichBlock::Audio { .. }));
        assert!(matches!(blocks[6], RichBlock::VoiceNote { .. }));
        assert!(matches!(blocks[7], RichBlock::Map { .. }));
        assert!(matches!(blocks[8], RichBlock::Collage { .. }));
    }

    #[test]
    fn embedded_media_blocks_in_paragraphs_are_isolated_and_parsed() {
        let text = "Ini fotonya: [photo: Kucing](https://example.com/cat.jpg) Kucing ini lucu.";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[1], RichBlock::Photo { .. }));
        assert!(matches!(blocks[2], RichBlock::Paragraph { .. }));

        let text_doc = "Silakan unduh dokumen [dokumen: Panduan](https://example.com/doc.pdf) yang telah kami siapkan.";
        let blocks_doc = parse_markdown_to_rich_blocks(text_doc);
        assert_eq!(blocks_doc.len(), 3);
        assert!(matches!(blocks_doc[0], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks_doc[1], RichBlock::Document { .. }));
        assert!(matches!(blocks_doc[2], RichBlock::Paragraph { .. }));
    }

    #[test]
    fn html_tags_convert_to_rich_formatting() {
        let input = "Teks <b>tebal</b> dan <strong>kuat</strong> serta <i>miring</i> dan <code>kode()</code> serta <a href=\"https://example.com\">Tautan</a>";
        let value = parse_inline(input);
        let serialized = serde_json::to_string(&value).expect("serialize value succeeds");
        assert!(serialized.contains(r#""type":"bold""#));
        assert!(serialized.contains("tebal"));
        assert!(serialized.contains("kuat"));
        assert!(serialized.contains(r#""type":"italic""#));
        assert!(serialized.contains("miring"));
        assert!(serialized.contains(r#""type":"code""#));
        assert!(serialized.contains("kode()"));
        assert!(serialized.contains(r#""type":"url""#));
        assert!(serialized.contains("https://example.com"));
    }

    #[test]
    fn github_alert_callouts_parse_correctly() {
        let text = "> [!NOTE]\n> Ini catatan penting sistem.";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 1);
        let serialized = serde_json::to_string(&blocks[0]).expect("serialize callout succeeds");
        assert!(serialized.contains("Catatan:"));
        assert!(serialized.contains("Ini catatan penting sistem."));
    }

    #[test]
    fn headings_without_space_parse_correctly() {
        let text = "###Fitur Baru\n\nPenjelasan fitur.";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            blocks[0],
            RichBlock::SectionHeading { level: 3, .. }
        ));
    }

    #[test]
    fn leaked_thinking_and_tool_calls_are_stripped() {
        let text = "<think>\nInternal secret reasoning\n</think>\n<tool_call>\n{\"name\": \"search\"}\n</tool_call>\nHalo! Ada yang bisa dibantu?";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 1);
        let serialized =
            serde_json::to_string(&blocks[0]).expect("serialize stripped block succeeds");
        assert!(!serialized.contains("Internal secret reasoning"));
        assert!(!serialized.contains("tool_call"));
        assert!(serialized.contains("Halo! Ada yang bisa dibantu?"));
    }

    #[test]
    fn streaming_markdown_never_leaks_unclosed_thinking_or_provisional_artifacts() {
        for text in [
            "<think>\ntunggu sebentar, saya sedang mencari - referensi",
            "<thought>\ntunggu sebentar, saya sedang - mencari",
            "<reasoning>\nsedang memikirkan - langkah",
            "<think>proses awal</think>\n<thought>proses kedua - lanjutan",
            "[thinking]\nproses bracket - pemikiran",
            "[think]\nproses bracket - singkat",
            "<",
            "<th",
            "<think",
            "<thought",
            "[",
            "[th",
            "[think",
            "[thinking",
        ] {
            let blocks = parse_streaming_markdown_to_rich_blocks(text);
            assert!(
                blocks.is_empty(),
                "expected empty blocks for thinking draft '{text}', but got {blocks:?}"
            );
        }

        // Ensure normal markdown link with [thinking] text is not wiped
        let normal_link = "[thinking](https://example.com) adalah link normal";
        let blocks = parse_streaming_markdown_to_rich_blocks(normal_link);
        assert!(!blocks.is_empty(), "expected markdown link to be preserved");
    }

    #[test]
    fn streaming_video_urls_do_not_produce_raw_video_blocks() {
        let text = "[video: Belajar Rust](https://www.youtube.com/watch?v=5C_HPTJg5ek)\n\n![Tutorial](https://youtu.be/abc12345)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 2);
        // Should parse as Paragraphs with styled links so Telegram link preview works without API 400 rejection
        assert!(matches!(blocks[0], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[1], RichBlock::Paragraph { .. }));
        let s0 = serde_json::to_string(&blocks[0]).expect("serialize block 0 succeeds");
        let s1 = serde_json::to_string(&blocks[1]).expect("serialize block 1 succeeds");
        assert!(s0.contains("Belajar Rust") && s0.contains("youtube.com"));
        assert!(s1.contains("Tutorial") && s1.contains("youtu.be"));
    }

    #[test]
    fn direct_video_files_produce_native_video_blocks() {
        let text = "[video: Animasi Robot](https://example.com/demo.mp4)\n\n![Clip](https://example.com/sample.webm)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], RichBlock::Video { .. }));
        assert!(matches!(blocks[1], RichBlock::Video { .. }));
    }

    #[test]
    fn telegram_html_media_tags_parse_into_rich_blocks() {
        let text = "<tg-photo src=\"https://example.com/cat.jpg\" caption=\"Kucing Manis\"/>\n\n<tg-audio src=\"https://example.com/audio.mp3\" caption=\"Lagu Pengantar\"/>\n\n<img src=\"https://example.com/pic.png\" alt=\"Foto Profil\">";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], RichBlock::Photo { .. }));
        assert!(matches!(blocks[1], RichBlock::Audio { .. }));
        assert!(matches!(blocks[2], RichBlock::Photo { .. }));
        let cap = blocks[0].caption_text().expect("caption present");
        assert_eq!(cap, "Kucing Manis");
    }

    #[test]
    fn multi_line_tg_collage_and_slideshow_parse_correctly() {
        let collage_html = r#"<tg-collage caption="Koleksi Logo">
<tg-photo src="https://example.com/logo1.png"/>
<tg-photo src="https://example.com/logo2.png"/>
</tg-collage>"#;
        let blocks = parse_markdown_to_rich_blocks(collage_html);
        assert_eq!(blocks.len(), 1);
        let RichBlock::Collage {
            blocks: items,
            caption: _,
        } = &blocks[0]
        else {
            panic!("expected collage block");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(blocks[0].caption_text().as_deref(), Some("Koleksi Logo"));

        let slideshow_html = r#"<tg-slideshow caption="Alur Slide">
<tg-photo src="https://example.com/s1.jpg"/>
<tg-photo src="https://example.com/s2.jpg"/>
</tg-slideshow>"#;
        let s_blocks = parse_markdown_to_rich_blocks(slideshow_html);
        assert_eq!(s_blocks.len(), 1);
        assert!(matches!(s_blocks[0], RichBlock::Slideshow { .. }));
    }

    #[test]
    fn multiple_consecutive_photos_parse_into_separate_rich_blocks() {
        let text = "Berikut logonya:\n\n[photo: Logo Rust](https://example.com/rust.png)\n[photo: Logo Go](https://example.com/go.png)\n[photo: Logo Python](https://example.com/py.png)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 4);
        assert!(matches!(blocks[0], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[1], RichBlock::Photo { .. }));
        assert!(matches!(blocks[2], RichBlock::Photo { .. }));
        assert!(matches!(blocks[3], RichBlock::Photo { .. }));
    }

    #[test]
    fn unsupported_image_formats_and_streaming_audio_produce_emoji_links() {
        let text = "[photo: Vektor SVG](https://example.com/logo.svg)\n\n![Audio](https://open.spotify.com/track/12345)\n\n<tg-photo src=\"https://example.com/art.bmp\" caption=\"Gambar Bitmap\"/>\n\n![](https://example.com/vector.svg)";
        let blocks = parse_markdown_to_rich_blocks(text);
        assert_eq!(blocks.len(), 4);
        assert!(matches!(blocks[0], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[1], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[2], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[3], RichBlock::Paragraph { .. }));
        let s0 = serde_json::to_string(&blocks[0]).expect("serialize block 0 succeeds");
        let s1 = serde_json::to_string(&blocks[1]).expect("serialize block 1 succeeds");
        let s2 = serde_json::to_string(&blocks[2]).expect("serialize block 2 succeeds");
        let s3 = serde_json::to_string(&blocks[3]).expect("serialize block 3 succeeds");
        assert!(s0.contains("🖼️") && s0.contains("logo.svg") && s0.contains("Vektor SVG"));
        assert!(s1.contains("🎵") && s1.contains("spotify.com") && s1.contains("Audio"));
        assert!(s2.contains("🖼️") && s2.contains("art.bmp") && s2.contains("Gambar Bitmap"));
        assert!(s3.contains("🖼️") && s3.contains("vector.svg") && s3.contains("Lihat Foto"));
    }

    #[test]
    fn math_blocks_and_inline_math_are_sanitized_for_cross_platform_rendering() {
        let md = r#"2. Teorema Pythagoras
$$c = \sqrt{a^2 + b^2} = \sqrt{6^2 + 8^2}$$
$$= \sqrt{36 + 64} = \sqrt{100} = 10\text{cm}$$

Contoh inline: $44\text{cm}$ dan $7,5\text{hari}$."#;

        let blocks = parse_markdown_to_rich_blocks(md);
        let math_blocks: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                RichBlock::MathematicalExpression { expression } => Some(expression.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(math_blocks.len(), 2);
        assert_eq!(math_blocks[0], r"c = \sqrt{a^2 + b^2} = \sqrt{6^2 + 8^2}");
        assert_eq!(
            math_blocks[1],
            r"= \sqrt{36 + 64} = \sqrt{100} = 10\ \mathrm{cm}"
        );

        // Verify inline math serialization inside paragraph
        let paragraph = blocks
            .iter()
            .find(|b| matches!(b, RichBlock::Paragraph { .. }))
            .expect("paragraph block present");
        let serialized = serde_json::to_string(paragraph).expect("serialize paragraph succeeds");
        assert!(serialized.contains(r"44\\ \\mathrm{cm}"));
        assert!(serialized.contains(r"7.5\\ \\mathrm{hari}"));
    }

    #[test]
    fn multiline_fenced_math_emits_individual_rich_blocks_per_line() {
        let md = "$$\nc = \\sqrt{a^2 + b^2}\n= \\sqrt{36 + 64}\n= 10\\text{cm}\n$$";
        let blocks = parse_markdown_to_rich_blocks(md);
        let math_blocks: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                RichBlock::MathematicalExpression { expression } => Some(expression.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(math_blocks.len(), 3);
        assert_eq!(math_blocks[0], r"c = \sqrt{a^2 + b^2}");
        assert_eq!(math_blocks[1], r"= \sqrt{36 + 64}");
        assert_eq!(math_blocks[2], r"= 10\ \mathrm{cm}");
    }

    #[test]
    fn rtl_markdown_table_with_hindi_numerals_defaults_to_right_alignment_and_sets_is_rtl() {
        let md = "| الرقم | الاسم |\n| --- | --- |\n| ١ | أحمد |\n| ٢ | فاطمة |";
        let message = build_full_rich_message(md, None);
        assert_eq!(message.is_rtl, Some(true));

        let Some(RichBlock::Table { cells, .. }) = message.blocks.first() else {
            panic!("expected table block");
        };

        // All cells in RTL table with unspecified separator default to "right"
        assert_eq!(cells[0][0].align.as_deref(), Some("right"));
        assert_eq!(cells[0][1].align.as_deref(), Some("right"));
        assert_eq!(cells[1][0].align.as_deref(), Some("right"));
        assert_eq!(cells[1][1].align.as_deref(), Some("right"));
    }

    #[test]
    fn rtl_table_honors_explicit_column_alignment() {
        let md = "| الرقم | الاسم | النتيجة |\n| :--- | :---: | ---: |\n| ١ | أحمد | ممتاز |";
        let blocks = parse_markdown_to_rich_blocks(md);
        let Some(RichBlock::Table { cells, .. }) = blocks.first() else {
            panic!("expected table block");
        };

        assert_eq!(cells[0][0].align.as_deref(), Some("left"));
        assert_eq!(cells[0][1].align.as_deref(), Some("center"));
        assert_eq!(cells[0][2].align.as_deref(), Some("right"));

        assert_eq!(cells[1][0].align.as_deref(), Some("left"));
        assert_eq!(cells[1][1].align.as_deref(), Some("center"));
        assert_eq!(cells[1][2].align.as_deref(), Some("right"));
    }

    #[test]
    fn unicode_box_table_with_rtl_header_defaults_column_to_right_alignment() {
        let input =
            "┌──────┬──────┐\n│ الرقم │ Score│\n├──────┼──────┤\n│ 123  │ 98   │\n└──────┴──────┘";
        let blocks = parse_markdown_to_rich_blocks(input);
        let Some(RichBlock::Table { cells, .. }) = blocks.first() else {
            panic!("expected table block");
        };

        // Col 0 has RTL header "الرقم", so even though cell is ASCII "123", it defaults to "right"
        assert_eq!(cells[0][0].align.as_deref(), Some("right"));
        assert_eq!(cells[1][0].align.as_deref(), Some("right"));

        // Col 1 is pure Latin "Score" / "98", so it remains "left"
        assert_eq!(cells[0][1].align.as_deref(), Some("left"));
        assert_eq!(cells[1][1].align.as_deref(), Some("left"));
    }

    #[test]
    fn test_split_table_row_cells_preserves_math_pipes() {
        let row = r"| $|v|_p$ | Norma- $p$ | $\left( \sum |v_i|^p \right)^{1/p}$ |";
        let cells = split_table_row_cells(row, false);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0], r"$|v|_p$");
        assert_eq!(cells[1], r"Norma- $p$");
        assert_eq!(cells[2], r"$\left( \sum |v_i|^p \right)^{1/p}$");
    }

    #[test]
    fn test_norm_table_with_math_pipes_parses_three_columns() {
        let md = "| Simbol | Nama | Definisi |\n| :---: | :--- | :--- |\n| $|v|_p$ | Norma- $p$ | $\\left( \\sum |v_i|^p \\right)^{1/p}$ |\n";
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Table { cells, .. }) = blocks.first() else {
            panic!("expected table block");
        };
        assert_eq!(cells.len(), 2); // 1 header row, 1 data row
        assert_eq!(cells[0].len(), 3); // 3 header columns
        assert_eq!(cells[1].len(), 3); // 3 data columns!

        // Check third cell of second row contains single mathematical_expression
        let json = serde_json::to_string(&cells[1][2]).expect("cell serializes");
        assert!(json.contains("mathematical_expression"));
        assert!(json.contains(r"\\left( \\sum |v_i|^p \\right)^{1/p}"));
    }

    #[test]
    fn test_standalone_therefore_and_because_render_as_unicode() {
        let md = "| Simbol | Arti / Nama | Penjelasan |\n| :---: | :--- | :--- |\n| $\\therefore$ | Oleh karena itu | Kesimpulan logis |\n| $\\because$ | Karena | Alasan/Premis |\n| $\\implies$ | Implikasi | Jika... maka... |\n| $\\impliedby$ | Implikasi balik | ...jika... |\n";
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Table { cells, .. }) = blocks.first() else {
            panic!("expected table block");
        };
        assert_eq!(cells.len(), 5);

        // Row 1: \therefore -> text "∴" (must be plain string, not unsupported plain_text entity)
        let cell_therefore = serde_json::to_string(&cells[1][0]).expect("cell serializes");
        assert!(cell_therefore.contains(r#""text":"∴""#));
        assert!(!cell_therefore.contains(r#""type":"plain_text""#));

        // Row 2: \because -> text "∵" (must be plain string, not unsupported plain_text entity)
        let cell_because = serde_json::to_string(&cells[2][0]).expect("cell serializes");
        assert!(cell_because.contains(r#""text":"∵""#));
        assert!(!cell_because.contains(r#""type":"plain_text""#));

        // Row 3: \implies -> mathematical_expression \implies
        let cell_implies = serde_json::to_string(&cells[3][0]).expect("cell serializes");
        assert!(cell_implies.contains(r#""expression":"\\implies""#));

        // Row 4: \impliedby -> normalized to \Longleftarrow for SwiftMath
        let cell_impliedby = serde_json::to_string(&cells[4][0]).expect("cell serializes");
        assert!(cell_impliedby.contains(r#""expression":"\\Longleftarrow""#));
    }

    #[test]
    fn test_reproduce_math_logic_table_no_unsupported_plain_text() {
        let md = r#"### 4. Logika Matematika & Pembuktian

| Simbol | Nama / Arti | Makna / Contoh |
| :---: | :--- | :--- |
| $\neg$ / $\sim$ | Negasi / Ingkaran | Menyangkal pernyataan ("bukan" / $\neg P$) |
| $\land$ | Konjungsi | Logika "dan" ($P \land Q$) |
| $\lor$ | Disjungsi | Logika "atau" ($P \lor Q$) |
| $\oplus$ | *Exclusive OR* (XOR) | Benar jika salah satu benar, tapi tidak keduanya |
| $\implies$ / $\to$ | Implikasi | "Jika $P$ maka $Q$" ($P \implies Q$) |
| $\iff$ / $\leftrightarrow$ | Biimplikasi | "Jika dan hanya jika" ($P \iff Q$) |
| $\forall$ | Kuantor Universal | "Untuk setiap / untuk semua" ($\forall x \in \mathbb{R}$) |
| $\exists$ | Kuantor Eksistensial | "Ada / terdapat setidaknya satu" ($\exists x$) |
| $\nexists$ | Negasi Eksistensial | "Tidak ada" |
| $\exists!$ | Keunikan | "Ada tepat satu" |
| $\therefore$ | Maka / Oleh karena itu | Penarikan kesimpulan (*Therefore*) |
| $\because$ | Karena | Memberikan alasan (*Because*) |
| $\blacksquare$ / Q.E.D. | Akhir pembuktian | *Quod Erat Demonstrandum* (telah terbukti) |
"#;
        let blocks = parse_markdown_to_rich_blocks(md);
        let msg = crate::bot::models::InputRichMessage::new(blocks);
        let val_res = msg.validate();
        assert!(val_res.is_ok(), "Validation failed: {:?}", val_res);

        let json_str = serde_json::to_string(&msg).expect("serialize rich message");
        assert!(
            !json_str.contains(r#""type":"plain_text""#),
            "Telegram Bot API rejects 'plain_text' as an unsupported rich text type"
        );
        assert!(
            !json_str.contains(r#""plain_text""#),
            "No plain_text discriminator should ever appear in rich message entities"
        );
    }

    #[test]
    fn indonesian_nahwu_lesson_preserves_ltr_canvas_and_correct_table_order() {
        let md = r#"### 4. Contoh Analisis Kalimat Sederhana

Mari kita bedah kalimat ini:
> **كَتَبَ التِّلْمِيْذُ الدَّرْسَ** (*Kataba at-tilmiidzu ad-darsa*)
Artinya: *Murid itu telah menulis pelajaran.*

1. **كَتَبَ** (*Kataba*): Fi'il Madhi (Kata kerja lampau).
2. **التِّلْمِيْذُ** (*At-tilmiidzu*): Fa'il (Pelaku), wajib berstatus *Rofa'*.
3. **الدَّرْسَ** (*Ad-darsa*): Maf'ul Bih (Objek), wajib berstatus *Nashab*.

| Nama I'rab | Tanda Asli (Harakat) | Biasanya Dipakai Untuk | Contoh |
| :--- | :---: | :--- | ---: |
| **Rofa'** | Dhammah (ـُ) | Subjek / Pelaku (*Fa'il*) | جَاءَ رَجُلٌ |
| **Nashab** | Fathah (ـَ) | Objek penderita (*Maf'ul Bih*) | رَأَيْتُ رَجُلاً |

### Ringkasan untuk Pemula:
1. Kenali dulu apakah suatu kata itu Benda (Isim), Kerja (Fi'il), atau Huruf.
2. Perhatikan awal kalimatnya: dimulai Isim atau Fi'il.
"#;
        let msg = build_full_rich_message(md, None);
        // The message is predominantly Indonesian, so is_rtl MUST be None
        assert_eq!(
            msg.is_rtl, None,
            "Mixed Indonesian lesson must not trigger global is_rtl"
        );

        // Verify the table block
        let table_block = msg
            .blocks
            .iter()
            .find(|b| matches!(b, RichBlock::Table { .. }))
            .expect("must contain a table block");

        let RichBlock::Table { cells, .. } = table_block else {
            panic!("expected table");
        };

        // Table column 0 must remain "Nama I'rab" (LTR column order preserved)
        let col0_header_text = &cells[0][0].text;
        assert!(
            serde_json::to_string(col0_header_text)
                .expect("serialize")
                .contains("Nama I'rab"),
            "Column 0 must remain 'Nama I'rab' on the left"
        );

        // Column 3 must be "Contoh" with right alignment
        let col3_header_text = &cells[0][3].text;
        assert!(
            serde_json::to_string(col3_header_text)
                .expect("serialize")
                .contains("Contoh"),
            "Column 3 must be 'Contoh'"
        );
        assert_eq!(cells[0][3].align.as_deref(), Some("right"));
        assert_eq!(cells[1][3].align.as_deref(), Some("right"));

        // Verify lists
        let list_blocks: Vec<_> = msg
            .blocks
            .iter()
            .filter(|b| matches!(b, RichBlock::List { .. }))
            .collect();
        assert_eq!(list_blocks.len(), 2, "Must contain 2 lists");
    }

    #[test]
    fn arabic_table_inside_ltr_message_is_reversed_with_eastern_arabic_digits() {
        let md = r#"Berikut adalah daftar santri teladan:

| الرقم | الاسم |
| :---: | :---: |
| 1 | أحمد |
| 2 | فاطمة |

Semoga bermanfaat untuk kita semua.
"#;
        let msg = build_full_rich_message(md, None);
        // Surrounding text is Indonesian -> is_rtl is None
        assert_eq!(msg.is_rtl, None);

        let table_block = msg
            .blocks
            .iter()
            .find(|b| matches!(b, RichBlock::Table { .. }))
            .expect("must contain a table block");

        let RichBlock::Table { cells, .. } = table_block else {
            panic!("expected table");
        };

        // Because header is pure Arabic (| الرقم | الاسم |) in an LTR message,
        // columns are reversed so that Column 0 (الرقم) appears visually on the right
        let col0_text = serde_json::to_string(&cells[0][0].text).expect("serialize");
        let col1_text = serde_json::to_string(&cells[0][1].text).expect("serialize");
        assert!(
            col0_text.contains("الاسم"),
            "Reversed: 'الاسم' should be at index 0"
        );
        assert!(
            col1_text.contains("الرقم"),
            "Reversed: 'الرقم' should be at index 1 (right edge)"
        );

        // Digits in the number column are converted to Eastern Arabic numerals
        let row1_num_cell = serde_json::to_string(&cells[1][1].text).expect("serialize");
        assert!(
            row1_num_cell.contains('١'),
            "Row 1 number should be Eastern Arabic '١'"
        );

        let row2_num_cell = serde_json::to_string(&cells[2][1].text).expect("serialize");
        assert!(
            row2_num_cell.contains('٢'),
            "Row 2 number should be Eastern Arabic '٢'"
        );
    }

    #[test]
    fn eastern_arabic_ordered_list_parses_value_correctly() {
        let md = r#"١. كتب الطالب الدرس
٢. قرأ زيد الكتاب
٣. جلس المعلم في الفصل
"#;
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 1);
        let RichBlock::List { items } = &blocks[0] else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].value, Some(1));
        assert_eq!(items[1].value, Some(2));
        assert_eq!(items[2].value, Some(3));
    }

    #[test]
    fn test_arabic_in_display_math_becomes_block_quotation() {
        let md = "Jika ditinjau:\n\n$$\\text{لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ}$$\n\n* **لَا (Lā)**";
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], RichBlock::Paragraph { .. }));
        let RichBlock::BlockQuotation {
            blocks: quote_blocks,
        } = &blocks[1]
        else {
            panic!("expected BlockQuotation, got {:?}", blocks[1]);
        };
        let quote_json = serde_json::to_string(&quote_blocks[0]).expect("serialize");
        assert!(quote_json.contains("لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ"));
        assert!(!quote_json.contains(r"\text"));
        assert!(!quote_json.contains(r"\mathrm"));

        // Verify that NO MathematicalExpression contains RTL
        let has_math_arabic = blocks.iter().any(|b| match b {
            RichBlock::MathematicalExpression { expression } => {
                crate::parser::rtl::has_rtl_characters(expression)
            }
            _ => false,
        });
        assert!(!has_math_arabic);
    }

    #[test]
    fn test_arabic_in_bracket_math_becomes_block_quotation() {
        let md = r#"Kutipan:
\[ \text{إِنَّ مَعَ الْعُسْرِ يُسْرًا} \]
Penjelasan berikutnya."#;
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 3);
        let RichBlock::BlockQuotation {
            blocks: quote_blocks,
        } = &blocks[1]
        else {
            panic!("expected BlockQuotation, got {:?}", blocks[1]);
        };
        let quote_json = serde_json::to_string(&quote_blocks[0]).expect("serialize");
        assert!(quote_json.contains("إِنَّ مَعَ الْعُسْرِ يُسْرًا"));
        assert!(!quote_json.contains(r"\text"));
    }

    #[test]
    fn test_arabic_in_inline_math_becomes_native_inline_text() {
        let md = "Perhatikan kata $\\text{لَا}$ di dalam kalimat.";
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 1);
        let RichBlock::Paragraph { text } = &blocks[0] else {
            panic!("expected Paragraph");
        };
        let para_json = serde_json::to_string(text).expect("serialize");
        assert!(para_json.contains("لَا"));
        assert!(!para_json.contains("mathematical_expression"));
    }

    #[test]
    fn test_genuine_math_formulas_still_produce_mathematical_expression() {
        let md = "$$c = \\sqrt{a^2 + b^2}$$\n\nInline: $E = mc^2$";
        let blocks = parse_markdown_to_rich_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            blocks[0],
            RichBlock::MathematicalExpression { .. }
        ));
        let RichBlock::Paragraph { text } = &blocks[1] else {
            panic!("expected Paragraph");
        };
        let para_json = serde_json::to_string(text).expect("serialize");
        assert!(para_json.contains("mathematical_expression"));
    }

    #[test]
    fn test_turn_34_exact_nahwu_snippet_parses_without_rtl_in_math() {
        let md = r#"### **Sentuhan Nahwu & Kebahasaan**

Jika ditinjau dari kaidah tata bahasa Arab (*nahwu*), penggalan kalimat tersebut mengandung uslub larangan (*an-nahyu*):

$$\text{لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ}$$

* **لَا (Lā)**: Disebut **لَا النَّاهِيَةُ** (*Lā an-Nāhiyah*), yaitu huruf yang bermakna larangan ("janganlah") dan bersifat menjazamkan kata kerja mudhari' (*tajzumu al-fi'l al-mudhāri'*).
* **تَقْنَطُوا (Taqnathū)**: Adalah **فِعْلٌ مُضَارِعٌ مَجْزُومٌ** (*fi'il mudhāri' majzūm*) dengan tanda jazam **حَذْفُ النُّونِ** (dibuangnya huruf nun) karena termasuk ke dalam kelompok **الْأَفْعَالُ الْخَمْسَةُ** (*al-af'āl al-khamsah* — bentuk asalnya sebelum kemasukan *lā* adalah *taqnathūna* / تَقْنَطُونَ).
  * Huruf **Wawu** (و) di dalamnya berposisi sebagai dhamir fail (*fā'il* / subjek).
* **مِنْ (Min)**: Huruf jar (*harf jarr*).
* **رَحْمَةِ (Rahmati)**: Isim majrur tanda kasrah, sekaligus berposisi sebagai **mudhaf** (مُضَاف).
* **اللَّهِ (Allāh)**: Lafaz jalalah sebagai **mudhaf ilaih** (مُضَاف إِلَيْهِ) yang majrur dengan kasrah di akhirnya.
"#;
        let rich = build_full_rich_message(md, None);
        // Ensure no block is a MathematicalExpression with Arabic
        for b in &rich.blocks {
            if let RichBlock::MathematicalExpression { expression } = b {
                assert!(
                    !crate::parser::rtl::has_rtl_characters(expression),
                    "MathematicalExpression should not contain Arabic: {expression}"
                );
            }
        }
        // Ensure the Arabic verse appears in a BlockQuotation
        let has_quote = rich.blocks.iter().any(|b| {
            if let RichBlock::BlockQuotation { blocks } = b {
                let s = serde_json::to_string(blocks).expect("serialize");
                s.contains("لَا تَقْنَطُوا مِنْ رَحْمَةِ اللَّهِ")
            } else {
                false
            }
        });
        assert!(has_quote, "Arabic phrase should be inside a BlockQuotation");
    }

    #[test]
    fn test_mixed_arabic_indonesian_list_items_get_lrm_prefix() {
        let md = r#"
Berikut adalah uraian I'rab:

> **وَلْيَكْتُبْ بَيْنَكُمْ كَاتِبٌ بِالْعَدْلِ ۚ**

* **يَا (Yā)**: *Harf nidā'* (huruf panggilan) mabni di atas sukun.
* **أَيُّ (Ayyu)**: *Munāda* mabni di atas dhammah.
* Fa (فَ): Rābiṭah li-jawāb asy-syarṭ (penghubung jawaban syarat).

**اللَّهِ**: Lafaz jalalah sebagai mudhaf ilaih.
"#;
        let blocks = parse_markdown_to_rich_blocks(md);

        // 1. Pure Arabic quote box should NOT have LRM prefix
        let quote = blocks
            .iter()
            .find(|b| matches!(b, RichBlock::BlockQuotation { .. }))
            .expect("find quote block");
        let quote_json = serde_json::to_string(quote).expect("serialize quote");
        assert!(
            !quote_json.contains('\u{200E}'),
            "Pure Arabic quote should NOT contain LRM"
        );

        // 2. List items
        let list_block = blocks
            .iter()
            .find(|b| matches!(b, RichBlock::List { .. }))
            .expect("find list block");
        let RichBlock::List { items } = list_block else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 3);

        // Item 0 starts with Arabic `يَا` and has Indonesian text -> MUST have LRM `\u{200E}`
        let item0_json = serde_json::to_string(&items[0]).expect("serialize item 0");
        assert!(
            item0_json.contains('\u{200E}'),
            "Mixed item 0 starting with Arabic must have LRM: {item0_json}"
        );

        // Item 1 starts with Arabic `أَيُّ` and has Indonesian text -> MUST have LRM `\u{200E}`
        let item1_json = serde_json::to_string(&items[1]).expect("serialize item 1");
        assert!(
            item1_json.contains('\u{200E}'),
            "Mixed item 1 starting with Arabic must have LRM: {item1_json}"
        );

        // Item 2 starts with Latin `Fa` -> MUST NOT have LRM `\u{200E}`
        let item2_json = serde_json::to_string(&items[2]).expect("serialize item 2");
        assert!(
            !item2_json.contains('\u{200E}'),
            "Item 2 starting with Latin should NOT have LRM: {item2_json}"
        );

        // 3. Mixed paragraph starting with Arabic **اللَّهِ**: Lafaz jalalah... -> MUST have LRM
        let mixed_para = blocks
            .iter()
            .find(|b| {
                if let RichBlock::Paragraph { text } = b {
                    serde_json::to_string(text)
                        .unwrap_or_default()
                        .contains("mudhaf ilaih")
                } else {
                    false
                }
            })
            .expect("find mixed paragraph");
        let para_json = serde_json::to_string(mixed_para).expect("serialize mixed para");
        assert!(
            para_json.contains('\u{200E}'),
            "Mixed paragraph starting with Arabic must have LRM: {para_json}"
        );
    }

    // =========================================================================
    // Milestone M2: Rich Tag HTML Parser & Media Resolution Tests
    // =========================================================================

    #[test]
    fn test_parse_rich_html_single_img_with_tg_scheme() {
        let html = r#"<img src="tg://photo?id=pic_summit" alt="Puncak Rinjani"/>"#;
        let res = parse_rich_html(html);
        assert!(res.is_ok(), "Expected Ok, got: {res:?}");
        let (blocks, media) = res.expect("valid result");

        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Photo { photo, caption }) = blocks.first() else {
            panic!("Expected RichBlock::Photo, got: {:?}", blocks.first());
        };
        assert_eq!(photo["type"], "photo");
        assert_eq!(photo["media"], "tg://photo?id=pic_summit");
        assert_eq!(
            caption
                .as_ref()
                .map(|c| serde_json::to_string(&c.text).unwrap_or_default()),
            Some("\"Puncak Rinjani\"".to_string())
        );

        assert_eq!(media.len(), 1);
        assert_eq!(media[0].id, "pic_summit");
        assert_eq!(media[0].media.media_url(), "tg://photo?id=pic_summit");
        assert_eq!(media[0].media.caption_text(), Some("Puncak Rinjani"));
    }

    #[test]
    fn test_parse_rich_html_single_img_with_direct_url() {
        let html =
            r#"<img src="https://example.com/rinjani.jpg" caption="Puncak Matahari Terbit"/>"#;
        let (blocks, media) = parse_rich_html(html).expect("valid direct url img");

        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Photo { photo, caption }) = blocks.first() else {
            panic!("Expected RichBlock::Photo");
        };
        assert_eq!(photo["type"], "photo");
        assert_eq!(photo["media"], "https://example.com/rinjani.jpg");
        assert!(caption.is_some());

        assert_eq!(media.len(), 1);
        assert_eq!(media[0].id, "photo_1");
        assert_eq!(
            media[0].media.media_url(),
            "https://example.com/rinjani.jpg"
        );
        assert_eq!(
            media[0].media.caption_text(),
            Some("Puncak Matahari Terbit")
        );
    }

    #[test]
    fn test_parse_rich_html_audio_tag_attributes() {
        let html = r#"<audio src="tg://audio?id=aud1" title="Angin Sembalun" performer="Lombok Sounds" caption="Suara Alam"/>"#;
        let (blocks, media) = parse_rich_html(html).expect("valid audio tag");

        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Audio { audio, caption }) = blocks.first() else {
            panic!("Expected RichBlock::Audio");
        };
        assert_eq!(audio["type"], "audio");
        assert_eq!(audio["media"], "tg://audio?id=aud1");
        assert_eq!(audio["title"], "Angin Sembalun");
        assert_eq!(audio["performer"], "Lombok Sounds");
        assert_eq!(
            caption
                .as_ref()
                .map(|c| serde_json::to_string(&c.text).unwrap_or_default()),
            Some("\"Suara Alam\"".to_string())
        );

        assert_eq!(media.len(), 1);
        assert_eq!(media[0].id, "aud1");
        assert_eq!(media[0].media.media_url(), "tg://audio?id=aud1");
        if let InputMedia::Audio {
            title,
            performer,
            caption,
            ..
        } = &media[0].media
        {
            assert_eq!(title.as_deref(), Some("Angin Sembalun"));
            assert_eq!(performer.as_deref(), Some("Lombok Sounds"));
            assert_eq!(caption.as_deref(), Some("Suara Alam"));
        } else {
            panic!("Expected InputMedia::Audio");
        }
    }

    #[test]
    fn test_parse_rich_html_collage_container() {
        let html = r#"<tg-collage caption="Album Kawah"><img src="tg://photo?id=p1" alt="Danau"/><img src="tg://photo?id=p2" alt="Puncak"/></tg-collage>"#;
        let (blocks, media) = parse_rich_html(html).expect("valid collage");

        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Collage {
            blocks: child_blocks,
            caption,
        }) = blocks.first()
        else {
            panic!("Expected RichBlock::Collage");
        };
        assert_eq!(child_blocks.len(), 2);
        assert_eq!(child_blocks[0]["type"], "photo");
        assert_eq!(child_blocks[0]["photo"]["media"], "tg://photo?id=p1");
        assert_eq!(child_blocks[0]["photo"]["caption"], "Danau");
        assert_eq!(child_blocks[1]["photo"]["media"], "tg://photo?id=p2");
        assert_eq!(child_blocks[1]["photo"]["caption"], "Puncak");
        assert_eq!(
            caption
                .as_ref()
                .map(|c| serde_json::to_string(&c.text).unwrap_or_default()),
            Some("\"Album Kawah\"".to_string())
        );

        assert_eq!(media.len(), 2);
        assert_eq!(media[0].id, "p1");
        assert_eq!(media[0].media.caption_text(), Some("Danau"));
        assert_eq!(media[1].id, "p2");
        assert_eq!(media[1].media.caption_text(), Some("Puncak"));
    }

    #[test]
    fn test_parse_rich_html_slideshow_container() {
        let html = r#"<tg-slideshow caption="Slideshow Pendakian"><img src="tg://photo?id=s1"/><img src="tg://photo?id=s2"/><img src="tg://photo?id=s3"/></tg-slideshow>"#;
        let (blocks, media) = parse_rich_html(html).expect("valid slideshow");

        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Slideshow {
            blocks: child_blocks,
            caption,
        }) = blocks.first()
        else {
            panic!("Expected RichBlock::Slideshow");
        };
        assert_eq!(child_blocks.len(), 3);
        assert!(caption.is_some());
        assert_eq!(media.len(), 3);
        assert_eq!(media[0].id, "s1");
        assert_eq!(media[1].id, "s2");
        assert_eq!(media[2].id, "s3");
    }

    #[test]
    fn test_parse_rich_html_tg_map_valid_coordinates_and_zoom() {
        let html =
            r#"<tg-map lat="-8.4113" lon="116.4573" zoom="13" title="Puncak Rinjani 3.726 mdpl"/>"#;
        let (blocks, media) = parse_rich_html(html).expect("valid map tag");

        assert_eq!(blocks.len(), 1);
        let Some(RichBlock::Map { location, zoom, .. }) = blocks.first() else {
            panic!("Expected RichBlock::Map");
        };
        assert_eq!(location.latitude, -8.4113);
        assert_eq!(location.longitude, 116.4573);
        assert_eq!(*zoom, Some(13));
        assert!(media.is_empty(), "Map produces no media upload items");
    }

    #[test]
    fn test_parse_rich_html_tg_map_rejects_out_of_bounds_and_non_finite() {
        // Latitude out of bounds [-90, 90]
        assert!(parse_rich_html(r#"<tg-map lat="91.0" lon="0.0"/>"#).is_err());
        assert!(parse_rich_html(r#"<tg-map lat="-91.0" lon="0.0"/>"#).is_err());

        // Longitude out of bounds [-180, 180]
        assert!(parse_rich_html(r#"<tg-map lat="0.0" lon="181.0"/>"#).is_err());
        assert!(parse_rich_html(r#"<tg-map lat="0.0" lon="-181.0"/>"#).is_err());

        // Non-finite coordinates
        assert!(parse_rich_html(r#"<tg-map lat="NaN" lon="0.0"/>"#).is_err());
        assert!(parse_rich_html(r#"<tg-map lat="0.0" lon="Infinity"/>"#).is_err());

        // Zoom out of bounds [1, 20]
        assert!(parse_rich_html(r#"<tg-map lat="0.0" lon="0.0" zoom="0"/>"#).is_err());
        assert!(parse_rich_html(r#"<tg-map lat="0.0" lon="0.0" zoom="21"/>"#).is_err());
    }

    #[test]
    fn test_parse_rich_html_rejects_collage_with_audio_or_docs() {
        let mixed = r#"<tg-collage><img src="tg://photo?id=p1"/><audio src="tg://audio?id=a1"/></tg-collage>"#;
        assert!(parse_rich_html(mixed).is_err());

        let mixed_doc = r#"<tg-collage><img src="tg://photo?id=p1"/><tg-document src="tg://document?id=d1"/></tg-collage>"#;
        assert!(parse_rich_html(mixed_doc).is_err());
    }

    #[test]
    fn test_parse_rich_html_media_deduplication() {
        let html = r#"<p>Dua kali foto sama:</p><img src="tg://photo?id=pic1"/><img src="tg://photo?id=pic1"/>"#;
        let (blocks, media) = parse_rich_html(html).expect("dedup html");
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            media.len(),
            1,
            "Duplicate ID must be deduplicated in media array"
        );
        assert_eq!(media[0].id, "pic1");
    }

    #[test]
    fn test_parse_rich_html_conflicting_duplicate_id_rejected() {
        let html = r#"<img src="tg://photo?id=pic1"/><img id="pic1" src="https://example.com/different.jpg"/>"#;
        assert!(parse_rich_html(html).is_err());
    }

    #[test]
    fn test_parse_rich_html_composite_single_unified_bubble() {
        let commentary_html = r#"<h3>Eksplorasi Gunung Rinjani</h3><p>Gunung Rinjani di Pulau Lombok adalah gunung berapi kedua tertinggi di Indonesia (3.726 mdpl) yang terkenal dengan kaldera megah dan danau kawah Segara Anak.</p><tg-collage caption="Pemandangan Kaldera & Segara Anak"><img src="tg://photo?id=pic_rinjani_1"/><img src="tg://photo?id=pic_rinjani_2"/></tg-collage><p>Berikut lokasi geografis puncak Rinjani pada peta satelit:</p><tg-map lat="-8.4113" lon="116.4573" zoom="13" title="Puncak Rinjani 3.726 mdpl"/>"#;

        let (blocks, media) =
            parse_rich_html(commentary_html).expect("rinjani composite rich html");

        assert_eq!(blocks.len(), 5);
        assert!(matches!(
            blocks[0],
            RichBlock::SectionHeading { level: 3, .. }
        ));
        assert!(matches!(blocks[1], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[2], RichBlock::Collage { .. }));
        assert!(matches!(blocks[3], RichBlock::Paragraph { .. }));
        assert!(matches!(blocks[4], RichBlock::Map { .. }));

        assert_eq!(media.len(), 2);
        assert_eq!(media[0].id, "pic_rinjani_1");
        assert_eq!(media[1].id, "pic_rinjani_2");
    }

    #[test]
    fn test_resolve_media_references() {
        let html = r#"<img src="tg://photo?id=pic1" alt="Rinjani"/>"#;
        let (mut blocks, _media) = parse_rich_html(html).expect("parse rich html");

        // External resolution: media item target is remote URL
        let resolved_media = vec![InputRichMessageMedia {
            id: "pic1".to_string(),
            media: InputMedia::photo("https://example.com/resolved_rinjani.jpg", None, None),
        }];

        resolve_media_references(&mut blocks, &resolved_media);

        let Some(RichBlock::Photo { photo, .. }) = blocks.first() else {
            panic!("Expected RichBlock::Photo");
        };
        assert_eq!(photo["media"], "https://example.com/resolved_rinjani.jpg");
    }

    #[test]
    fn test_parse_rich_html_container_degradation() {
        let collage_one = r#"<tg-collage><img src="tg://photo?id=single1"/></tg-collage>"#;
        let (blocks, _) = parse_rich_html(collage_one).expect("degrade collage 1");
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(blocks[0], RichBlock::Photo { .. }),
            "Collage with 1 item degrades to Photo"
        );

        let slideshow_one = r#"<tg-slideshow><img src="tg://photo?id=single2"/></tg-slideshow>"#;
        let (blocks, _) = parse_rich_html(slideshow_one).expect("degrade slideshow 1");
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(blocks[0], RichBlock::Photo { .. }),
            "Slideshow with 1 item degrades to Photo"
        );
    }

    #[test]
    fn test_markdown_parser_converts_document_tag_to_rich_block() {
        let md = "Berikut laporannya:\n\n[document: test.pdf](attach://doc_0)";
        let blocks = parse_streaming_markdown_to_rich_blocks(md);

        assert_eq!(blocks.len(), 2);

        // First block is a paragraph
        if let RichBlock::Paragraph { text, .. } = &blocks[0] {
            assert_eq!(text, "Berikut laporannya:");
        } else {
            panic!("Expected Paragraph, got {:?}", blocks[0]);
        }

        // Second block is the document
        if let RichBlock::Document {
            document, caption, ..
        } = &blocks[1]
        {
            assert_eq!(document["type"], "document");
            assert_eq!(document["media"], "attach://doc_0");
            assert_eq!(caption.text, "test.pdf");
        } else {
            panic!("Expected Document, got {:?}", blocks[1]);
        }
    }
}
