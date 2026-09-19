use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

use crate::bot::models::{
    InputRichMessage, Location, RichBlock, RichBlockCaption, RichBlockListItem, RichBlockTableCell,
};
use crate::parser::latex::sanitize_latex_for_telegram;
use crate::parser::rtl;

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

    // <tg-map lat="..." lon="..."/>
    if let Some(rest) = s.strip_prefix("<tg-map") {
        let trimmed = rest.trim().trim_end_matches('>').trim_end_matches('/');
        let lat_s = trimmed
            .split("lat=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let lon_s = trimmed
            .split("lon=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let zoom_s = trimmed
            .split("zoom=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let lat = lat_s.parse::<f64>().ok()?;
        let lon = lon_s.parse::<f64>().ok()?;
        let zoom = zoom_s.parse::<i32>().ok();
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
    let needle_double = format!("{attr}=\"");
    let needle_single = format!("{attr}='");
    if let Some(rest) = tag.split(&needle_double).nth(1) {
        return rest.split('"').next().map(str::trim);
    }
    if let Some(rest) = tag.split(&needle_single).nth(1) {
        return rest.split('\'').next().map(str::trim);
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

    // 3. Telegram native HTML media tags: <tg-photo ...>, <tg-video ...>, <tg-audio ...>, <img ...>
    if s.starts_with("<tg-photo")
        || s.starts_with("<tg-video")
        || s.starts_with("<tg-audio")
        || s.starts_with("<img")
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
    let cap_attr = extract_html_attribute(s, "caption")
        .or_else(|| extract_html_attribute(s, "alt"))
        .or_else(|| extract_html_attribute(s, "title"));
    let inner_text = s
        .split('>')
        .nth(1)
        .and_then(|t| t.split("</").next())
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let caption_text = cap_attr.or(inner_text).unwrap_or("");
    let caption =
        (!caption_text.is_empty()).then(|| RichBlockCaption::new(parse_inline(caption_text)));

    if s.starts_with("<tg-photo") || s.starts_with("<img") {
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

    if s.starts_with("<tg-audio") {
        if is_streaming_web_audio(src) {
            return Some(format_media_fallback_paragraph(
                caption_text,
                src,
                "Dengarkan Audio",
                "🎵",
            ));
        }
        return Some(RichBlock::Audio {
            audio: json!({"type": "audio", "media": src}),
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

    let mut sub_blocks = Vec::new();
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
        {
            let lower = clean_url.to_lowercase();
            if lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mov") {
                sub_blocks
                    .push(json!({"type": "video", "video": {"type": "video", "media": clean_url}}));
            } else if !lower.ends_with(".html")
                && !lower.ends_with(".htm")
                && !is_streaming_web_video(clean_url)
                && !is_streaming_web_audio(clean_url)
                && !is_unsupported_image_format(clean_url)
            {
                sub_blocks
                    .push(json!({"type": "photo", "photo": {"type": "photo", "media": clean_url}}));
            }
        }
    }

    if sub_blocks.len() >= 2 {
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
        r#"(?i)(!?\[(?:photo|foto|image|img|gambar|picture|pic|video|vid|audio|musik|music|lagu|song|voice|voicenote|voice_note|suara|rekaman|vn|animation|animasi|gif|collage|kolase|gallery|galeri|album|slideshow|slide|document|dokumen|doc|file|berkas|map|location|lokasi|peta|geo)\s*:[^\]]+\](?:\s*\([^\)]+\))?[.,;:]?|!\[[^\]]*\]\s*\([^\)]+\)[.,;:]?|<tg-(?:photo|video|audio|document|map|collage|slideshow)[^>]*>|</tg-(?:photo|video|audio|document|map|collage|slideshow)>|<img[^>]*>)"#
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
static RE_BLOCK_DIVIDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\-{3,}|\*{3,}|_{3,}|─{3,}|—{2,})$").expect("valid static regex")
});
static RE_BLOCK_BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-*•]\s+").expect("valid static regex"));
static RE_BLOCK_NUMBERED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+|[\u0660-\u0669]+)[\.)]\s+").expect("valid static regex"));

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

        // 4. Horizontal Divider (---, ***, ___, ───)
        if RE_BLOCK_DIVIDER.is_match(stripped) {
            blocks.push(RichBlock::Divider {});
            i += 1;
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
                blocks.push(RichBlock::BlockQuotation {
                    blocks: vec![json!({
                        "type": "paragraph",
                        "text": parse_inline(inner)
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
            blocks.push(RichBlock::BlockQuotation {
                blocks: vec![json!({
                    "type": "paragraph",
                    "text": parse_inline(&quote_lines.join("\n"))
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
            blocks.push(RichBlock::BlockQuotation {
                blocks: vec![json!({
                    "type": "paragraph",
                    "text": parse_inline(&quote_lines.join("\n"))
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
                    list_items.push(RichBlockListItem::ordered(
                        vec![json!({
                            "type": "paragraph",
                            "text": parse_inline(&item_text)
                        })],
                        value,
                    ));
                    i += 1;
                } else if !is_ordered && RE_BLOCK_BULLET.is_match(curr) {
                    let item_text = RE_BLOCK_BULLET.replace(curr, "").trim().to_string();
                    list_items.push(RichBlockListItem::bullet(vec![json!({
                        "type": "paragraph",
                        "text": parse_inline(&item_text)
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
                || RE_BLOCK_HEADING.is_match(s_curr)
                || s_curr.starts_with("**>")
                || s_curr.starts_with("<blockquote")
                || s_curr.starts_with('>')
                || s_curr.starts_with(">>>")
                || s_curr.starts_with("[table:")
                || s_curr.starts_with("[caption:")
                || s_curr.starts_with("<tg-map")
                || s_curr.starts_with("<tg-document")
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
            blocks.push(RichBlock::Paragraph {
                text: parse_inline(&para_lines.join("\n")),
            });
        }
    }

    blocks
}

pub fn build_full_rich_message(answer_text: &str, footer_text: Option<&str>) -> InputRichMessage {
    let mut blocks = parse_markdown_to_rich_blocks(answer_text);
    if blocks.is_empty() {
        blocks.push(RichBlock::Paragraph {
            text: parse_inline(answer_text.trim()),
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
}
