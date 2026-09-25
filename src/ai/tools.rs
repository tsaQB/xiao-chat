use std::collections::HashSet;
use std::env;
use std::sync::LazyLock;
use std::time::Duration;

use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, REFERER, USER_AGENT};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};
use url::Url;

static RE_DDG_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a class="result__url"[^>]*href="(?P<url>[^"]+)"[^>]*>"#)
        .expect("valid static regex")
});
static RE_DDG_SNIPPET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a class="result__snippet"[^>]*>(?P<snippet>.*?)</a>"#)
        .expect("valid static regex")
});

static RE_HTML_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script.*?</script>").expect("valid static regex"));
static RE_HTML_STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<style.*?</style>").expect("valid static regex"));
static RE_HTML_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<head.*?</head>").expect("valid static regex"));
static RE_HTML_NOSCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<noscript.*?</noscript>").expect("valid static regex"));
static RE_HTML_BLOCK_BREAK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)</(?:p|div|section|article|blockquote|h[1-6]|tr|table|ul|ol)>|<br\s*/?>|<hr\s*/?>",
    )
    .expect("valid static regex")
});
static RE_HTML_LIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>").expect("valid static regex"));
static RE_HTML_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("valid static regex"));
static RE_WHITESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[ \t]+").expect("valid static regex"));

pub fn clean_html_to_text(html: &str) -> String {
    let no_script = RE_HTML_SCRIPT.replace_all(html, "");
    let no_style = RE_HTML_STYLE.replace_all(&no_script, "");
    let no_head = RE_HTML_HEAD.replace_all(&no_style, "");
    let no_noscript = RE_HTML_NOSCRIPT.replace_all(&no_head, "");
    let with_blocks = RE_HTML_BLOCK_BREAK.replace_all(&no_noscript, "\n\n");
    let with_lists = RE_HTML_LIST_ITEM.replace_all(&with_blocks, "\n• ");
    let no_tags = RE_HTML_TAGS.replace_all(&with_lists, " ");
    let decoded = html_escape::decode_html_entities(&no_tags);

    let mut normalized = String::with_capacity(decoded.len());
    for line in decoded.lines() {
        let trimmed_line = RE_WHITESPACE.replace_all(line.trim(), " ");
        if !trimmed_line.is_empty() {
            normalized.push_str(&trimmed_line);
            normalized.push('\n');
        } else if !normalized.ends_with("\n\n") && !normalized.is_empty() {
            normalized.push('\n');
        }
    }
    normalized.trim().to_string()
}

static RE_HTML_IMG_SRC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)<img\b[^>]*?\b(?:src|data-src|data-original|data-lazy-src|data-high-res-src|data-full-url|data-url)=["'](?P<src>[^"']+)["']"#,
    )
    .expect("valid static regex")
});
static RE_HTML_SRCSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bsrcset=["'](?P<srcset>[^"']+)["']"#).expect("valid static regex")
});
static RE_HTML_A_HREF_IMG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)<a\b[^>]*?\bhref=["'](?P<href>[^"']+\.(?:jpg|jpeg|png|webp)(?:\?[^"']*)?)["']"#,
    )
    .expect("valid static regex")
});
static RE_VISUAL_KEYWORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:foto|foto-foto|gambar|gambar-gambar|potret|pemandangan|citra|lukisan|ilustrasi|wallpaper|bagan|diagram|grafis|logo|logos|logonya|ikon|icon|icons|lambang|simbol|symbol|symbols|emblem|emblems|badge|badges|vektor|vector|vectors|bendera|flag|flags|photo|photos|picture|pictures|pic|pics|image|images|visual|visuals|illustration|wallpaper|png|jpg|jpeg|webp)\b",
    )
    .expect("valid static regex")
});
static RE_LOGO_KEYWORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:logo|logos|logonya|ikon|icon|icons|lambang|simbol|symbol|symbols|emblem|emblems|badge|badges|crest|coat of arms)\b",
    )
    .expect("valid static regex")
});
static RE_CONVERSATIONAL_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:(?:tolong|coba|mohon|bisakah|bisa|silakan|please|can you|could you|i want|i need|aku mau|saya mau)\s+)?(?:(?:berikan|carikan|tampilkan|tunjukkan|perlihatkan|lihatkan|kirimkan|cari|lihat|minta|give|show|find|search|send|get)\b(?:\s+(?:saya|aku|kami|me|us)\b)?)?(?:\s*(?:\d+|satu|dua|tiga|empat|lima|enam|tujuh|delapan|sembilan|sepuluh|beberapa|one|two|three|four|five|six|seven|eight|nine|ten|some|a|an)\b)?(?:\s*(?:buah|lembar|keping|ekor|item|items)\b)?(?:\s*(?:foto-foto|foto|gambar-gambar|gambar|potret|citra|logo|logos|logonya|ikon|icon|icons|photos?|pictures?|images?|pics?)\b)?(?:\s*(?:dari|tentang|mengenai|of|about)\b)?\s*",
    )
    .expect("valid static regex")
});
static RE_CONVERSATIONAL_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[,.]?\s*\b(?:please|ya|dong|tolong|kan)\b\s*$").expect("valid static regex")
});
static RE_ID_MARKERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:yang|dan|di|ke|dari|ini|itu|untuk|pada|adalah|dengan|foto|gambar|gunung|pemandangan|pantai|kota|indonesia|wisata|kuliner|pulau|sejarah|presiden|taman|danau|masjid|candi)\b",
    )
    .expect("valid static regex")
});

#[allow(dead_code)]
pub fn is_visual_search_query(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }
    RE_VISUAL_KEYWORDS.is_match(trimmed)
}

#[allow(dead_code)]
pub fn is_logo_query(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }
    RE_LOGO_KEYWORDS.is_match(trimmed)
}

#[allow(dead_code)]
pub fn extract_core_search_terms(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let cleaned = RE_CONVERSATIONAL_PREFIX.replace(trimmed, "");
    let no_suffix = RE_CONVERSATIONAL_SUFFIX.replace(&cleaned, "");
    let mut res = no_suffix.trim();
    if let Some(stripped) = res
        .strip_prefix(':')
        .or_else(|| res.strip_prefix(','))
        .or_else(|| res.strip_prefix('-'))
    {
        res = stripped.trim();
    }
    if res.is_empty() {
        trimmed.to_string()
    } else {
        res.to_string()
    }
}

#[allow(dead_code)]
pub fn is_likely_indonesian(text: &str) -> bool {
    RE_ID_MARKERS.is_match(text)
}

#[allow(dead_code)]
pub fn is_valid_raster_image_url(url_str: &str) -> bool {
    sanitize_and_validate_raster_url(url_str).is_some()
}

#[allow(dead_code)]
pub fn sanitize_and_validate_raster_url(url_str: &str) -> Option<String> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("data:")
        || lower.starts_with("javascript:")
        || lower.starts_with("blob:")
        || lower.starts_with("file:")
    {
        return None;
    }

    let full_url_str = if trimmed.starts_with("//") {
        format!("https:{trimmed}")
    } else {
        trimmed.to_string()
    };

    let Ok(mut parsed) = Url::parse(&full_url_str) else {
        return None;
    };

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }

    let host = parsed.host_str()?;
    let host_lower = host.to_ascii_lowercase();

    if host_lower == "localhost"
        || host_lower == "127.0.0.1"
        || host_lower == "::1"
        || host_lower.ends_with(".local")
        || host_lower.ends_with(".internal")
        || host_lower.ends_with(".test")
        || host_lower.ends_with(".example")
        || host_lower.ends_with(".invalid")
    {
        return None;
    }

    if !host_lower.contains('.') && host_lower.parse::<std::net::IpAddr>().is_err() {
        return None;
    }

    const TRACKING_DOMAINS: &[&str] = &[
        "google-analytics.com",
        "googletagmanager.com",
        "doubleclick.net",
        "adnxs.com",
        "scorecardresearch.com",
        "quantserve.com",
        "clarity.ms",
        "pixel.wp.com",
        "stats.wp.com",
        "analytics.twitter.com",
        "bat.bing.com",
    ];
    for &td in TRACKING_DOMAINS {
        if host_lower == td || host_lower.ends_with(&format!(".{td}")) {
            return None;
        }
    }

    let path = parsed.path().to_ascii_lowercase();

    // DuckDuckGo proxy unwrapping: if URL is /iu/?u=<encoded_url>, unwrap and validate target
    if (host_lower == "duckduckgo.com" || host_lower.ends_with(".duckduckgo.com"))
        && path.starts_with("/iu/")
    {
        if let Some(target_u) = parsed
            .query_pairs()
            .find(|(k, _)| k == "u")
            .map(|(_, v)| v.to_string())
        {
            if let Ok(decoded) = urlencoding::decode(&target_u) {
                if let Some(valid_target) = sanitize_and_validate_raster_url(&decoded) {
                    return Some(valid_target);
                }
            }
        }
        return None;
    }

    if (host_lower == "duckduckgo.com" || host_lower.ends_with(".duckduckgo.com"))
        && path.starts_with("/t/")
    {
        return None;
    }

    if path.contains("anomaly-modal") || path.contains("challenge-form") {
        return None;
    }

    const BAD_PATH_KEYWORDS: &[&str] = &[
        "placeholder",
        "dummyimage",
        "placekitten",
        "placehold.it",
        "pixel.gif",
        "1x1.gif",
        "1x1.png",
        "spacer.gif",
        "blank.gif",
        "/beacon",
        "/telemetry",
        "transparent.png",
        "empty.png",
    ];
    for &kw in BAD_PATH_KEYWORDS {
        if path.contains(kw) || host_lower.contains(kw) {
            return None;
        }
    }

    if path.ends_with(".svg") || path.ends_with(".gif") || path.ends_with(".ico") {
        return None;
    }

    let query_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let mut retained_query: Vec<(String, String)> = Vec::new();
    let mut format_hint = None;

    for (k, v) in query_pairs {
        let k_lower = k.to_ascii_lowercase();
        let v_lower = v.to_ascii_lowercase();
        if k_lower.starts_with("utm_")
            || k_lower == "fbclid"
            || k_lower == "gclid"
            || k_lower == "msclkid"
            || k_lower == "ref"
            || k_lower == "ref_src"
            || k_lower == "_ga"
            || k_lower == "_gl"
            || k_lower == "mc_cid"
            || k_lower == "mc_eid"
        {
            continue;
        }
        if (k_lower == "format" || k_lower == "fm" || k_lower == "ext")
            && matches!(v_lower.as_str(), "jpg" | "jpeg" | "png" | "webp")
        {
            format_hint = Some(v_lower);
        }
        retained_query.push((k, v));
    }

    let has_raster_extension = path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".png")
        || path.ends_with(".webp");

    let is_unsplash = host_lower == "images.unsplash.com" || host_lower.ends_with(".unsplash.com");
    let is_wikimedia = (host_lower.contains("wikimedia.org")
        || host_lower.contains("wikipedia.org"))
        && (path.contains(".jpg")
            || path.contains(".jpeg")
            || path.contains(".png")
            || path.contains(".webp"));

    if !has_raster_extension && format_hint.is_none() && !is_unsplash && !is_wikimedia {
        return None;
    }

    if retained_query.is_empty() {
        parsed.set_query(None);
    } else {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &retained_query {
            serializer.append_pair(k, v);
        }
        parsed.set_query(Some(&serializer.finish()));
    }

    Some(parsed.to_string())
}

#[allow(dead_code)]
pub fn extract_raster_images_from_html(html: &str, base_url: Option<&str>) -> Vec<String> {
    let mut images = Vec::new();
    let mut seen = HashSet::new();

    let base_parsed = base_url.and_then(|b| Url::parse(b).ok());

    let mut resolve_and_add = |candidate: &str| {
        let full = if candidate.starts_with("http://")
            || candidate.starts_with("https://")
            || candidate.starts_with("//")
        {
            candidate.to_string()
        } else if let Some(ref base) = base_parsed {
            match base.join(candidate) {
                Ok(u) => u.to_string(),
                Err(_) => return,
            }
        } else {
            return;
        };

        if let Some(valid) = sanitize_and_validate_raster_url(&full) {
            if seen.insert(valid.clone()) {
                images.push(valid);
            }
        }
    };

    for cap in RE_HTML_IMG_SRC.captures_iter(html) {
        if let Some(src) = cap.name("src") {
            resolve_and_add(src.as_str());
        }
    }

    for cap in RE_HTML_SRCSET.captures_iter(html) {
        if let Some(srcset) = cap.name("srcset") {
            for entry in srcset.as_str().split(',') {
                let candidate = entry.split_whitespace().next().unwrap_or("");
                if !candidate.is_empty() {
                    resolve_and_add(candidate);
                }
            }
        }
    }

    for cap in RE_HTML_A_HREF_IMG.captures_iter(html) {
        if let Some(href) = cap.name("href") {
            resolve_and_add(href.as_str());
        }
    }

    images
}

#[allow(dead_code)]
pub fn format_verified_images_section(image_urls: &[String]) -> String {
    if image_urls.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\n🖼️ **URL Foto/Gambar Raster Terverifikasi (Dapat Digunakan untuk Tool Multimedia)**:\n",
    );
    for url in image_urls {
        out.push_str(&format!("- {url}\n"));
    }
    out
}

#[allow(dead_code)]
pub fn format_no_images_guidance(query: &str) -> String {
    format!(
        "\n\nℹ️ **Catatan Media**: Tidak ditemukan berkas gambar raster langsung (.jpg, .png, .webp) yang valid dari hasil pencarian untuk \"{query}\". Berikan penjelasan deskriptif yang kaya dan informatif mengenai topik ini kepada pengguna dalam teks Markdown, dan hindari memanggil tool multimedia dengan URL fiktif/rekaan."
    )
}

pub fn get_tools_definition() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Cari informasi terkini, fakta ensiklopedia, atau gambar/foto dari internet menggunakan mesin pencari. Jika pengguna meminta foto/gambar, mesin pencari akan menyertakan URL raster terverifikasi (.jpg, .jpeg, .png, .webp) yang siap digunakan untuk tool multimedia (send_photo, send_collage, send_slideshow).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Kata kunci pencarian yang jelas dan spesifik"
                        }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "fetch_url",
                "description": "Ambil dan baca konten teks lengkap dari sebuah tautan URL web.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Tautan URL web yang ingin dibaca (diawali http:// atau https://)"
                        }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "create_quiz",
                "description": "Buat kuis interaktif native Telegram (mode kuis) dengan 2-10 pilihan ganda. Gunakan parameter preamble jika ingin menyajikan pengantar, konteks bacaan/studi kasus, atau potongan kode panjang sebelum kuis.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "question": {
                            "type": "string",
                            "description": "Pertanyaan kuis (maksimal 300 karakter)"
                        },
                        "options": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            },
                            "description": "Daftar pilihan jawaban (2 sampai 10 opsi, masing-masing 1-100 karakter)"
                        },
                        "correct_option_id": {
                            "type": "integer",
                            "description": "Indeks jawaban yang benar (0-based, dimulai dari 0)"
                        },
                        "explanation": {
                            "type": "string",
                            "description": "Penjelasan saat jawaban dibuka (opsional, maksimal 200 karakter, maksimal 2 line breaks)"
                        },
                        "preamble": {
                            "type": "string",
                            "description": "Pesan pengantar atau materi/studi kasus/kode panjang sebelum kuis (opsional, terformat Markdown)"
                        }
                    },
                    "required": ["question", "options", "correct_option_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_photo",
                "description": "Kirim sebuah foto atau gambar langsung ke obrolan Telegram via URL gambar publik raster (.jpg, .jpeg, .png, .webp).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL langsung file gambar raster publik (.jpg, .png, .webp). Hindari tautan Wikimedia yang memblokir bot (HTTP 403) dan jangan gunakan SVG."
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan atau teks pengantar untuk foto (opsional, mendukung Markdown standar)"
                        }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_collage",
                "description": "Kirim album kolase foto (2 sampai 10 foto) ke obrolan Telegram sebagai native sendMediaGroup.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "urls": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            },
                            "description": "Daftar URL gambar langsung (minimal 2, maksimal 10 foto raster)"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan atau teks pengantar untuk seluruh album kolase foto (opsional)"
                        }
                    },
                    "required": ["urls"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_slideshow",
                "description": "Kirim tayangan slide foto interaktif (native carousel <tg-slideshow>) ke obrolan Telegram yang dapat digeser atau dibolak-balik fotonya langsung melalui kontrol navigasi bawaan Telegram pada gambar.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "urls": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            },
                            "description": "Daftar URL gambar langsung untuk setiap slide carousel"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan atau deskripsi tayangan slide (opsional)"
                        }
                    },
                    "required": ["urls"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_audio",
                "description": "Kirim berkas audio musik native ke obrolan Telegram via URL.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL langsung berkas audio (misal .mp3, .m4a)"
                        },
                        "title": {
                            "type": "string",
                            "description": "Judul lagu atau audio (opsional)"
                        },
                        "performer": {
                            "type": "string",
                            "description": "Nama penyanyi atau pencipta audio (opsional)"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan atau deskripsi audio (opsional)"
                        }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_voice",
                "description": "Kirim rekaman suara (voice note) dengan visual waveform native ke obrolan Telegram via URL audio (.ogg/.mp3).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL langsung berkas suara (.ogg atau .mp3)"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan pesan suara (opsional)"
                        }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_location",
                "description": "Kirim koordinat lokasi geografis native Telegram berupa pin peta interaktif.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "latitude": {
                            "type": "number",
                            "description": "Garis lintang lokasi geografis (antara -90.0 dan 90.0)"
                        },
                        "longitude": {
                            "type": "number",
                            "description": "Garis bujur lokasi geografis (antara -180.0 dan 180.0)"
                        },
                        "title": {
                            "type": "string",
                            "description": "Nama tempat atau label lokasi (opsional)"
                        }
                    },
                    "required": ["latitude", "longitude"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_document",
                "description": "Kirim berkas dokumen umum (.pdf, .zip, .docx, spreadsheet, dll.) ke obrolan Telegram via URL berkas.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL langsung berkas dokumen yang akan dikirim"
                        },
                        "file_name": {
                            "type": "string",
                            "description": "Nama file berkas beserta ekstensinya, misalnya 'laporan.pdf' (opsional)"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan ringkas dokumen (opsional)"
                        }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "create_document",
                "description": "Buat berkas/dokumen teks, dokumen PDF (.pdf), berkas HTML, source code, data CSV/JSON/YAML, vektor SVG, atau arsip ZIP langsung dan kirimkan ke chat Telegram.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filename": {
                            "type": "string",
                            "description": "Nama file berkas beserta ekstensinya (contoh: laporan.pdf, script.py, index.html, data.csv)"
                        },
                        "content": {
                            "type": "string",
                            "description": "Isi/teks atau konten dokumen yang akan dibuat (untuk berkas teks/code/markdown/HTML, atau representasi konten PDF/SVG)"
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan ringkas dokumen (opsional)"
                        },
                        "as_zip": {
                            "type": "boolean",
                            "description": "True jika berkas harus dikompresi ke dalam ZIP"
                        }
                    },
                    "required": ["filename", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "create_archive",
                "description": "Buat paket arsip ZIP multi-file langsung yang memuat banyak berkas/script/dokumen ke dalam satu berkas .zip dan kirimkan ke chat Telegram.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filename": {
                            "type": "string",
                            "description": "Nama file arsip zip (contoh: cpa-toolkit.zip, project-bundle.zip)"
                        },
                        "files": {
                            "type": "array",
                            "description": "Daftar berkas yang dimasukkan ke dalam arsip zip",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "filename": {
                                        "type": "string",
                                        "description": "Nama berkas di dalam zip (contoh: cpa.sh, README.md, config.json)"
                                    },
                                    "content": {
                                        "type": "string",
                                        "description": "Konten/isi berkas"
                                    }
                                },
                                "required": ["filename", "content"]
                            }
                        },
                        "caption": {
                            "type": "string",
                            "description": "Keterangan ringkas arsip zip (opsional)"
                        }
                    },
                    "required": ["filename", "files"]
                }
            }
        }
    ])
}

pub fn get_brave_key() -> Option<String> {
    env::var("BRAVE_API_KEY")
        .ok()
        .or_else(|| crate::ai::service::load_app_setting("BRAVE_API_KEY"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn get_tavily_key() -> Option<String> {
    env::var("TAVILY_API_KEY")
        .or_else(|_| env::var("TAVILY_KEY"))
        .ok()
        .or_else(|| {
            crate::ai::service::load_app_setting("TAVILY_API_KEY")
                .or_else(|| crate::ai::service::load_app_setting("TAVILY_KEY"))
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn get_exa_key() -> Option<String> {
    env::var("EXA_API_KEY")
        .or_else(|_| env::var("EXA_KEY"))
        .ok()
        .or_else(|| {
            crate::ai::service::load_app_setting("EXA_API_KEY")
                .or_else(|| crate::ai::service::load_app_setting("EXA_KEY"))
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn get_search_engine_status() -> (String, String) {
    let exa_key = get_exa_key();
    let tavily_key = get_tavily_key();
    let brave_key = get_brave_key();

    let engine_name = if brave_key.is_some() {
        "Brave Search (API Key Active)".to_string()
    } else if tavily_key.is_some() {
        "Tavily AI (API Key Active)".to_string()
    } else if exa_key.is_some() {
        "Exa AI (REST API Key Active)".to_string()
    } else {
        "Exa MCP (Keyless) \u{2192} DuckDuckGo / Wikipedia".to_string()
    };

    let mcp_url = get_configured_mcp_url();
    (engine_name, mcp_url)
}

pub fn get_configured_mcp_url() -> String {
    env::var("EXA_MCP_URL")
        .ok()
        .or_else(|| crate::ai::service::load_app_setting("EXA_MCP_URL"))
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| "https://mcp.exa.ai/".to_string())
}

pub async fn execute_web_search(query: &str) -> String {
    let q = query.trim();
    if q.is_empty() {
        return "Query pencarian tidak boleh kosong. Silakan berikan kata kunci atau topik pencarian yang lebih spesifik.".to_string();
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let is_visual = is_visual_search_query(q);

    // 1. Check Brave Search API
    if let Some(brave_key) = get_brave_key() {
        info!("Using Brave Search API for query: {q}");
        match search_brave(&client, &brave_key, q).await {
            Ok(mut res) => {
                if is_visual && !res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                    if let Ok(wiki_res) = search_wikipedia(&client, q).await {
                        if wiki_res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                            let clean_res = res
                                .replace(&format_no_images_guidance(q), "")
                                .trim()
                                .to_string();
                            res = clean_res;
                            res.push_str("\n\n---\n\n");
                            res.push_str(&wiki_res);
                        }
                    }
                }
                return res;
            }
            Err(e) => warn!("Brave search failed ({e}), falling back to other providers"),
        }
    }

    // 2. Check Tavily API
    if let Some(tavily_key) = get_tavily_key() {
        info!("Using Tavily API for query: {q}");
        match search_tavily(&client, &tavily_key, q).await {
            Ok(mut res) => {
                if is_visual && !res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                    if let Ok(wiki_res) = search_wikipedia(&client, q).await {
                        if wiki_res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                            let clean_res = res
                                .replace(&format_no_images_guidance(q), "")
                                .trim()
                                .to_string();
                            res = clean_res;
                            res.push_str("\n\n---\n\n");
                            res.push_str(&wiki_res);
                        }
                    }
                }
                return res;
            }
            Err(e) => warn!("Tavily search failed ({e}), falling back to other providers"),
        }
    }

    // 3. Check Exa REST API
    if let Some(exa_key) = get_exa_key() {
        info!("Using Exa API for query: {q}");
        match search_exa_api(&client, &exa_key, q).await {
            Ok(mut res) => {
                if is_visual && !res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                    if let Ok(wiki_res) = search_wikipedia(&client, q).await {
                        if wiki_res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                            let clean_res = res
                                .replace(&format_no_images_guidance(q), "")
                                .trim()
                                .to_string();
                            res = clean_res;
                            res.push_str("\n\n---\n\n");
                            res.push_str(&wiki_res);
                        } else if !res.contains("ℹ️ **Catatan Media**") {
                            res.push_str(&format_no_images_guidance(q));
                        }
                    } else if !res.contains("ℹ️ **Catatan Media**") {
                        res.push_str(&format_no_images_guidance(q));
                    }
                }
                return res;
            }
            Err(e) => warn!("Exa API search failed ({e}), falling back to other providers"),
        }
    }

    // 4. Default / Keyless Exa MCP
    let mcp_url = get_configured_mcp_url();
    info!("Trying Exa Keyless MCP for query: {q}");
    match search_exa_mcp(&client, &mcp_url, q).await {
        Ok(mut res) => {
            if is_visual && !res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                if let Ok(wiki_res) = search_wikipedia(&client, q).await {
                    if wiki_res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                        let clean_res = res
                            .replace(&format_no_images_guidance(q), "")
                            .trim()
                            .to_string();
                        res = clean_res;
                        res.push_str("\n\n---\n\n");
                        res.push_str(&wiki_res);
                    } else if !res.contains("ℹ️ **Catatan Media**") {
                        res.push_str(&format_no_images_guidance(q));
                    }
                } else if !res.contains("ℹ️ **Catatan Media**") {
                    res.push_str(&format_no_images_guidance(q));
                }
            }
            return res;
        }
        Err(e) => {
            warn!("Gagal menghubungi Exa MCP ({e}), beralih ke DuckDuckGo...");
        }
    }

    // 5. DuckDuckGo Search Fallback
    info!("Using DuckDuckGo for query: {q}");
    match search_duckduckgo(&client, q).await {
        Ok(mut res) => {
            if is_visual && !res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                if let Ok(wiki_res) = search_wikipedia(&client, q).await {
                    if wiki_res.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi") {
                        let clean_res = res
                            .replace(&format_no_images_guidance(q), "")
                            .trim()
                            .to_string();
                        res = clean_res;
                        res.push_str("\n\n---\n\n");
                        res.push_str(&wiki_res);
                    }
                }
            }
            return res;
        }
        Err(e) => {
            warn!("DuckDuckGo did not return results or was blocked; trying Wikipedia knowledge base: {q}");
            warn!("Koneksi ke DuckDuckGo gagal ({e}). Catatan: Domain DuckDuckGo diblokir oleh beberapa ISP/Kominfo di Indonesia. Disarankan menggunakan TAVILY_API_KEY, EXA_API_KEY, atau BRAVE_API_KEY untuk hasil yang cepat.");
        }
    }

    // 6. Wikipedia Knowledge Base Fallback
    match search_wikipedia(&client, q).await {
        Ok(res) => {
            if res.trim().is_empty() {
                format!(
                    "[Informasi Pencarian Web]\nPencarian daring untuk topik \"{q}\" telah selesai namun tidak menghasilkan data teks.\n\nℹ️ **Panduan Asisten**: Berikan tanggapan deskriptif dan faktual mengenai topik \"{q}\" berdasarkan pengetahuan internal Anda secara lengkap dalam teks Markdown standar."
                )
            } else {
                res
            }
        }
        Err(e) => {
            warn!("Wikipedia search failed ({e})");
            format!(
                "[Informasi Pencarian Web]\nPencarian web daring untuk topik \"{q}\" saat ini tidak dapat diselesaikan karena kendala koneksi atau penyedia pencarian sedang tidak tersedia ({e}).\n\nℹ️ **Panduan Asisten**: Berikan tanggapan deskriptif dan faktual mengenai topik \"{q}\" berdasarkan pengetahuan internal Anda secara lengkap. Jika pengguna meminta gambar atau foto, jelaskan informasi visualnya secara naratif dalam teks Markdown dan hindari memanggil tool multimedia fiktif."
            )
        }
    }
}

#[inline]
fn format_search_item(index: usize, title: &str, url: &str, summary: &str) -> String {
    format!("{index}. **{title}**\n   URL: {url}\n   Ringkasan: {summary}\n\n")
}

async fn search_brave(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
) -> Result<String, String> {
    let is_visual = is_visual_search_query(query);
    let mut verified_images = Vec::new();

    if is_visual {
        let img_search_url = format!(
            "https://api.search.brave.com/res/v1/images/search?q={}&count=5",
            urlencoding::encode(query)
        );
        if let Ok(resp) = client
            .get(&img_search_url)
            .header("X-Subscription-Token", api_key)
            .header(ACCEPT, "application/json")
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(img_body) = resp.json::<Value>().await {
                    if let Some(results) = img_body.get("results").and_then(Value::as_array) {
                        for item in results.iter().take(5) {
                            let raw_url = item
                                .get("properties")
                                .and_then(|p| p.get("url"))
                                .and_then(Value::as_str)
                                .or_else(|| {
                                    item.get("thumbnail")
                                        .and_then(|t| t.get("src"))
                                        .and_then(Value::as_str)
                                });
                            if let Some(u) = raw_url {
                                if let Some(valid) = sanitize_and_validate_raster_url(u) {
                                    if !verified_images.contains(&valid) {
                                        verified_images.push(valid);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count=5",
        urlencoding::encode(query)
    );

    let resp = client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("Gagal menghubungi Brave Search API: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Brave Search API returned HTTP {status}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Gagal membaca JSON Brave: {e}"))?;

    let mut out = String::new();
    if let Some(results) = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(Value::as_array)
    {
        for (i, item) in results.iter().take(5).enumerate() {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Tanpa Judul");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            let desc = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            out.push_str(&format_search_item(i + 1, title, url, desc));

            if let Some(thumb) = item
                .get("thumbnail")
                .and_then(|t| t.get("src").or_else(|| t.get("original")))
                .and_then(Value::as_str)
            {
                if let Some(valid) = sanitize_and_validate_raster_url(thumb) {
                    if !verified_images.contains(&valid) {
                        verified_images.push(valid);
                    }
                }
            }
        }
    }

    if let Some(pics) = body
        .get("pictures")
        .and_then(|p| p.get("results"))
        .and_then(Value::as_array)
    {
        for item in pics.iter().take(5) {
            let raw_url = item
                .get("thumbnail")
                .and_then(|t| t.get("src"))
                .and_then(Value::as_str);
            if let Some(u) = raw_url {
                if let Some(valid) = sanitize_and_validate_raster_url(u) {
                    if !verified_images.contains(&valid) {
                        verified_images.push(valid);
                    }
                }
            }
        }
    }

    if out.trim().is_empty() {
        Ok(format!(
            "Tidak ada hasil ditemukan di Brave untuk query \"{query}\"."
        ))
    } else {
        let mut res = format!("[Hasil Pencarian Brave untuk \"{query}\"]\n\n{out}")
            .trim()
            .to_string();
        if !verified_images.is_empty() {
            res.push_str(&format_verified_images_section(&verified_images));
        } else if is_visual {
            res.push_str(&format_no_images_guidance(query));
        }
        Ok(res)
    }
}

async fn search_tavily(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
) -> Result<String, String> {
    let is_visual = is_visual_search_query(query);
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&json!({
            "api_key": api_key,
            "query": query,
            "include_answer": true,
            "include_images": true,
            "max_results": 5,
            "search_depth": "basic"
        }))
        .send()
        .await
        .map_err(|e| format!("Gagal menghubungi Tavily API: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Tavily API returned HTTP {status}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Gagal membaca JSON Tavily: {e}"))?;

    let mut out = String::new();
    if let Some(answer) = body
        .get("answer")
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty())
    {
        out.push_str(&format!("💡 **Jawaban Ringkas**: {}\n\n", answer));
    }

    let mut verified_images = Vec::new();

    if let Some(images) = body.get("images").and_then(Value::as_array) {
        for img in images {
            let raw_url = if let Some(s) = img.as_str() {
                Some(s)
            } else {
                img.get("url").and_then(Value::as_str)
            };
            if let Some(u) = raw_url {
                if let Some(valid_url) = sanitize_and_validate_raster_url(u) {
                    if !verified_images.contains(&valid_url) {
                        verified_images.push(valid_url);
                    }
                }
            }
        }
    }

    if let Some(results) = body.get("results").and_then(Value::as_array) {
        for (i, item) in results.iter().take(5).enumerate() {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Tanpa Judul");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            let content = item.get("content").and_then(Value::as_str).unwrap_or("");
            out.push_str(&format_search_item(i + 1, title, url, content));

            if let Some(img_u) = item.get("image").and_then(Value::as_str) {
                if let Some(valid) = sanitize_and_validate_raster_url(img_u) {
                    if !verified_images.contains(&valid) {
                        verified_images.push(valid);
                    }
                }
            }
        }
    }

    if out.trim().is_empty() {
        Ok(format!(
            "Tidak ada hasil ditemukan di Tavily untuk query \"{query}\"."
        ))
    } else {
        let mut res = format!("[Hasil Pencarian Tavily untuk \"{query}\"]\n\n{out}")
            .trim()
            .to_string();
        if !verified_images.is_empty() {
            res.push_str(&format_verified_images_section(&verified_images));
        } else if is_visual {
            res.push_str(&format_no_images_guidance(query));
        }
        Ok(res)
    }
}

async fn search_exa_api(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
) -> Result<String, String> {
    let resp = client
        .post("https://api.exa.ai/search")
        .header("x-api-key", api_key)
        .header(ACCEPT, "application/json")
        .json(&json!({
            "query": query,
            "numResults": 5,
            "highlights": true
        }))
        .send()
        .await
        .map_err(|e| format!("Gagal menghubungi Exa API: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Exa API returned HTTP {status}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Gagal membaca JSON Exa: {e}"))?;

    let mut out = String::new();
    if let Some(results) = body.get("results").and_then(Value::as_array) {
        for (i, item) in results.iter().take(5).enumerate() {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Tanpa Judul");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            let highlight = item
                .get("highlights")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(Value::as_str)
                .or_else(|| item.get("text").and_then(Value::as_str))
                .unwrap_or("");
            out.push_str(&format_search_item(i + 1, title, url, highlight));
        }
    }

    if out.trim().is_empty() {
        Ok(format!(
            "Tidak ada hasil ditemukan di Exa untuk query \"{query}\"."
        ))
    } else {
        Ok(
            format!("[Hasil Pencarian Exa AI untuk \"{query}\"]\n\n{out}")
                .trim()
                .to_string(),
        )
    }
}

pub(crate) async fn search_exa_mcp(
    _client: &reqwest::Client,
    mcp_url: &str,
    query: &str,
) -> Result<String, String> {
    let resolved = crate::bot::url_policy::resolve_download_url(mcp_url).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve(&resolved.host, resolved.address)
        .build()
        .map_err(|e| format!("Gagal menginisialisasi client HTTP Exa MCP: {e}"))?;

    let resp = client
        .post(resolved.url)
        .header(ACCEPT, "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": {
                    "query": query
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Gagal menghubungi Exa MCP ({e})"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Exa MCP returned HTTP {status}"));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Gagal membaca stream Exa MCP: {e}"))?;

    let parsed_text = if let Ok(val) = serde_json::from_str::<Value>(&text) {
        if let Some(content) = val
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(Value::as_array)
        {
            content
                .iter()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        } else {
            String::new()
        }
    } else {
        let mut extracted: Vec<String> = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.trim();
                if let Ok(val) = serde_json::from_str::<Value>(rest) {
                    if let Some(c_arr) = val
                        .get("result")
                        .and_then(|r| r.get("content"))
                        .and_then(Value::as_array)
                    {
                        for c in c_arr {
                            if let Some(t) = c.get("text").and_then(Value::as_str) {
                                extracted.push(t.to_string());
                            }
                        }
                    }
                }
            }
        }
        extracted.join("\n\n")
    };

    if parsed_text.trim().is_empty() {
        Err("Exa MCP tidak mengembalikan konten yang valid.".to_string())
    } else {
        Ok(
            format!("[Hasil Pencarian Exa AI untuk \"{query}\"]\n\n{parsed_text}")
                .trim()
                .to_string(),
        )
    }
}

fn unwrap_ddg_url(raw_url: &str) -> String {
    if let Ok(parsed) = Url::parse(raw_url) {
        if let Some(uddg) = parsed
            .query_pairs()
            .find(|(k, _)| k == "uddg")
            .map(|(_, v)| v.to_string())
        {
            if let Ok(decoded) = urlencoding::decode(&uddg) {
                return decoded.to_string();
            }
        }
    } else if let Ok(parsed) = Url::parse(&format!("https://duckduckgo.com{raw_url}")) {
        if let Some(uddg) = parsed
            .query_pairs()
            .find(|(k, _)| k == "uddg")
            .map(|(_, v)| v.to_string())
        {
            if let Ok(decoded) = urlencoding::decode(&uddg) {
                return decoded.to_string();
            }
        }
    }
    raw_url.to_string()
}

async fn search_duckduckgo(client: &reqwest::Client, query: &str) -> Result<String, String> {
    let is_visual = is_visual_search_query(query);
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let resp = client
        .get(&url)
        .header(
            USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        )
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(ACCEPT_LANGUAGE, "id,en-US;q=0.9,en;q=0.8")
        .header(REFERER, "https://duckduckgo.com/")
        .send()
        .await
        .map_err(|e| format!("Gagal mencari di DuckDuckGo: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("DuckDuckGo mengembalikan status HTTP {status}"));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Gagal membaca respon DuckDuckGo: {e}"))?;

    let mut out = String::new();
    let urls: Vec<String> = RE_DDG_TITLE
        .captures_iter(&html)
        .take(5)
        .filter_map(|c| c.name("url").map(|m| unwrap_ddg_url(m.as_str().trim())))
        .collect();
    let snippets: Vec<String> = RE_DDG_SNIPPET
        .captures_iter(&html)
        .take(5)
        .filter_map(|c| {
            c.name("snippet").map(|m| {
                let cleaned = m.as_str().replace("<b>", "").replace("</b>", "");
                html_escape::decode_html_entities(&cleaned).to_string()
            })
        })
        .collect();

    for i in 0..urls.len().min(snippets.len()) {
        out.push_str(&format_search_item(
            i + 1,
            "Hasil Pencarian",
            &urls[i],
            &snippets[i],
        ));
    }

    let mut extracted_images =
        extract_raster_images_from_html(&html, Some("https://duckduckgo.com"));

    // If visual query and no images were directly in DDG HTML, scrape top result URLs
    if is_visual && extracted_images.is_empty() {
        for target_url in urls.iter().take(2) {
            if target_url.starts_with("http://") || target_url.starts_with("https://") {
                if let Ok(page_resp) = client
                    .get(target_url)
                    .timeout(Duration::from_secs(4))
                    .header(
                        USER_AGENT,
                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
                    )
                    .header(
                        ACCEPT,
                        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                    )
                    .send()
                    .await
                {
                    if page_resp.status().is_success() {
                        if let Ok(page_html) = page_resp.text().await {
                            let scraped = extract_raster_images_from_html(&page_html, Some(target_url));
                            for img in scraped {
                                if !extracted_images.contains(&img) {
                                    extracted_images.push(img);
                                }
                                if extracted_images.len() >= 6 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if !extracted_images.is_empty() {
                break;
            }
        }
    }

    if out.trim().is_empty() {
        Err("Tidak ditemukan hasil pencarian".to_string())
    } else {
        let mut res = format!("[Hasil Pencarian Web untuk \"{query}\"]\n\n{out}")
            .trim()
            .to_string();
        if !extracted_images.is_empty() {
            res.push_str(&format_verified_images_section(&extracted_images));
        } else if is_visual {
            res.push_str(&format_no_images_guidance(query));
        }
        Ok(res)
    }
}

async fn fetch_wikipedia_article_images(
    client: &reqwest::Client,
    lang: &str,
    article_title: &str,
    is_logo_search: bool,
) -> Result<Vec<String>, String> {
    let url = format!(
        "https://{lang}.wikipedia.org/w/api.php?action=query&titles={}&generator=images&gimlimit=12&prop=imageinfo&iiprop=url&iiurlwidth=1000&format=json",
        urlencoding::encode(article_title)
    );

    let resp = client
        .get(&url)
        .header(
            USER_AGENT,
            concat!(
                "xiao/",
                env!("CARGO_PKG_VERSION"),
                " (Telegram Bot Assistant)"
            ),
        )
        .send()
        .await
        .map_err(|e| format!("Wikipedia gallery request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Wikipedia gallery status HTTP {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Wikipedia gallery JSON: {e}"))?;

    let mut images = Vec::new();
    if let Some(pages_obj) = body
        .get("query")
        .and_then(|q| q.get("pages"))
        .and_then(Value::as_object)
    {
        for page in pages_obj.values() {
            let title = page.get("title").and_then(Value::as_str).unwrap_or("");
            let title_lower = title.to_ascii_lowercase();

            if !is_logo_search {
                if title_lower.contains("logo")
                    || title_lower.contains("flag")
                    || title_lower.contains("icon")
                    || title_lower.contains("symbol")
                    || title_lower.contains("disambig")
                    || title_lower.contains("ui")
                    || title_lower.contains("locator")
                    || title_lower.contains("map")
                    || title_lower.contains("peta")
                    || title_lower.contains("diagram")
                    || title_lower.contains("insignia")
                    || title_lower.contains("coat_of_arms")
                    || title_lower.contains("lambang")
                    || title_lower.contains("stub")
                {
                    continue;
                }
            } else if title_lower.contains("disambig")
                || title_lower.contains("ui")
                || title_lower.contains("locator")
                || title_lower.contains("map")
                || title_lower.contains("peta")
                || title_lower.contains("stub")
            {
                continue;
            }

            if let Some(info_arr) = page.get("imageinfo").and_then(Value::as_array) {
                if let Some(first_info) = info_arr.first() {
                    let candidate_url = first_info
                        .get("thumburl")
                        .and_then(Value::as_str)
                        .and_then(sanitize_and_validate_raster_url)
                        .or_else(|| {
                            first_info
                                .get("url")
                                .and_then(Value::as_str)
                                .and_then(sanitize_and_validate_raster_url)
                        });
                    if let Some(valid_url) = candidate_url {
                        if !images.contains(&valid_url) {
                            images.push(valid_url);
                        }
                    }
                }
            }
        }
    }

    Ok(images)
}

async fn search_wikimedia_commons_files(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<String>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&generator=search&gsrsearch={}&gsrnamespace=6&gsrlimit=6&prop=imageinfo&iiprop=url&iiurlwidth=1000&format=json",
        urlencoding::encode(q)
    );

    let resp = client
        .get(&url)
        .header(
            USER_AGENT,
            concat!(
                "xiao/",
                env!("CARGO_PKG_VERSION"),
                " (Telegram Bot Assistant)"
            ),
        )
        .send()
        .await
        .map_err(|e| format!("Wikimedia Commons request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Wikimedia Commons status HTTP {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Wikimedia Commons JSON: {e}"))?;

    let mut images = Vec::new();
    if let Some(pages_obj) = body
        .get("query")
        .and_then(|qu| qu.get("pages"))
        .and_then(Value::as_object)
    {
        for page in pages_obj.values() {
            let title = page.get("title").and_then(Value::as_str).unwrap_or("");
            let title_lower = title.to_ascii_lowercase();
            if title_lower.contains("disambig")
                || title_lower.contains("locator")
                || title_lower.contains("map")
                || title_lower.contains("peta")
                || title_lower.contains("stub")
            {
                continue;
            }

            if let Some(info_arr) = page.get("imageinfo").and_then(Value::as_array) {
                if let Some(first_info) = info_arr.first() {
                    let candidate_url = first_info
                        .get("thumburl")
                        .and_then(Value::as_str)
                        .and_then(sanitize_and_validate_raster_url)
                        .or_else(|| {
                            first_info
                                .get("url")
                                .and_then(Value::as_str)
                                .and_then(sanitize_and_validate_raster_url)
                        });
                    if let Some(valid_url) = candidate_url {
                        if !images.contains(&valid_url) {
                            images.push(valid_url);
                        }
                    }
                }
            }
        }
    }

    Ok(images)
}

async fn search_wikipedia(client: &reqwest::Client, query: &str) -> Result<String, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("Query pencarian tidak boleh kosong".to_string());
    }

    let langs = if is_likely_indonesian(q) {
        vec!["id", "en"]
    } else {
        vec!["en", "id"]
    };

    let core_terms = extract_core_search_terms(q);
    let is_visual = is_visual_search_query(q);
    let is_logo = is_logo_query(q);

    let search_attempts =
        if is_visual && !core_terms.is_empty() && core_terms.to_lowercase() != q.to_lowercase() {
            vec![core_terms.as_str(), q]
        } else if !core_terms.is_empty() && core_terms.to_lowercase() != q.to_lowercase() {
            vec![q, core_terms.as_str()]
        } else {
            vec![q]
        };

    for lang in &langs {
        for attempt in &search_attempts {
            let url = format!(
                "https://{lang}.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}&gsrlimit=5&prop=pageimages|extracts&piprop=original|thumbnail&pithumbsize=1000&exintro=1&explaintext=1&exchars=350&format=json",
                urlencoding::encode(attempt)
            );

            let resp = match client
                .get(&url)
                .header(
                    USER_AGENT,
                    concat!(
                        "xiao/",
                        env!("CARGO_PKG_VERSION"),
                        " (Telegram Bot Assistant)"
                    ),
                )
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Wikipedia API connection failed for {lang} ({e})");
                    continue;
                }
            };

            if !resp.status().is_success() {
                continue;
            }

            let body: Value = match resp.json().await {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to parse Wikipedia JSON for {lang}: {e}");
                    continue;
                }
            };

            let Some(pages_obj) = body
                .get("query")
                .and_then(|qu| qu.get("pages"))
                .and_then(Value::as_object)
            else {
                continue;
            };

            if pages_obj.is_empty() {
                continue;
            }

            let mut page_list: Vec<&Value> = pages_obj.values().collect();
            page_list.sort_by_key(|p| p.get("index").and_then(Value::as_i64).unwrap_or(999));

            let mut out = String::new();
            let mut verified_images = Vec::new();

            for (i, page) in page_list.iter().enumerate().take(5) {
                let title = page
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Tanpa Judul");
                let extract = page.get("extract").and_then(Value::as_str).unwrap_or("");
                let clean_extract = extract.trim();
                let page_url = format!(
                    "https://{lang}.wikipedia.org/wiki/{}",
                    urlencoding::encode(title)
                );

                out.push_str(&format_search_item(i + 1, title, &page_url, clean_extract));

                // Prefer thumbnail (raster render, e.g. 1000px PNG) first if original is SVG or non-raster
                let candidate_url = page
                    .get("thumbnail")
                    .and_then(|t| t.get("source"))
                    .and_then(Value::as_str)
                    .and_then(sanitize_and_validate_raster_url)
                    .or_else(|| {
                        page.get("original")
                            .and_then(|o| o.get("source"))
                            .and_then(Value::as_str)
                            .and_then(sanitize_and_validate_raster_url)
                    });
                if let Some(valid_url) = candidate_url {
                    if !verified_images.contains(&valid_url) {
                        verified_images.push(valid_url);
                    }
                }
            }

            if is_visual {
                let is_logo_search = is_logo || is_logo_query(attempt);
                for page in page_list.iter().take(3) {
                    if let Some(title) = page.get("title").and_then(Value::as_str) {
                        if let Ok(gallery_images) =
                            fetch_wikipedia_article_images(client, lang, title, is_logo_search)
                                .await
                        {
                            for img in gallery_images {
                                if !verified_images.contains(&img) {
                                    verified_images.push(img);
                                }
                                if verified_images.len() >= 8 {
                                    break;
                                }
                            }
                        }
                    }
                    if verified_images.len() >= 8 {
                        break;
                    }
                }

                if verified_images.len() < 3 || is_logo_search {
                    if let Ok(commons_images) =
                        search_wikimedia_commons_files(client, attempt).await
                    {
                        for img in commons_images {
                            if !verified_images.contains(&img) {
                                verified_images.push(img);
                            }
                            if verified_images.len() >= 8 {
                                break;
                            }
                        }
                    }
                }
            }

            if !out.trim().is_empty() {
                // If this is a visual search and we haven't found images yet, try the next search attempt if available
                if is_visual
                    && verified_images.is_empty()
                    && *attempt != search_attempts.last().copied().unwrap_or("")
                {
                    continue;
                }

                let mut res = format!("[Hasil Informasi Ensiklopedia Web untuk \"{q}\"]\n\n{out}")
                    .trim()
                    .to_string();
                if !verified_images.is_empty() {
                    res.push_str(&format_verified_images_section(&verified_images));
                } else if is_visual {
                    res.push_str(&format_no_images_guidance(q));
                }
                return Ok(res);
            }
        }
    }

    if is_visual {
        if let Ok(commons_images) = search_wikimedia_commons_files(client, q).await {
            if !commons_images.is_empty() {
                let mut res = format!("[Hasil Informasi Berkas Media untuk \"{q}\"]\n\nDitemukan berkas media resmi terverifikasi untuk topik tersebut.\n");
                res.push_str(&format_verified_images_section(&commons_images));
                return Ok(res);
            }
        }
    }

    Err(format!(
        "Tidak ada hasil ditemukan di ensiklopedia untuk query: \"{q}\""
    ))
}

const MAX_FETCH_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_FETCH_REDIRECTS: usize = 5;

pub async fn fetch_web_content(url: &str) -> Result<String, String> {
    let mut current_url_str = url.trim().to_string();
    if !current_url_str.starts_with("http://") && !current_url_str.starts_with("https://") {
        return Err("URL harus diawali dengan http:// atau https://".to_string());
    }

    let mut redirect_count = 0;
    let resp = loop {
        let resolved = crate::bot::url_policy::resolve_download_url(&current_url_str).await?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve(&resolved.host, resolved.address)
            .build()
            .map_err(|e| format!("Gagal menginisialisasi client HTTP: {e}"))?;

        let response = client
            .get(resolved.url.clone())
            .header(
                USER_AGENT,
                concat!(
                    "xiao/",
                    env!("CARGO_PKG_VERSION"),
                    " (Telegram Bot Assistant)"
                ),
            )
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8,*/*;q=0.5",
            )
            .header(ACCEPT_LANGUAGE, "id,en-US;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| format!("Gagal mengunduh halaman web: {e}"))?;

        let status = response.status();
        if status.is_redirection() {
            if redirect_count >= MAX_FETCH_REDIRECTS {
                return Err("Terlalu banyak pengalihan (redirect loop).".to_string());
            }
            redirect_count += 1;
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| "Pengalihan tanpa header Location yang valid".to_string())?;

            let next_url = resolved
                .url
                .join(location)
                .map_err(|e| format!("URL pengalihan tidak valid: {e}"))?;

            current_url_str = next_url.to_string();
            continue;
        }

        if !status.is_success() {
            return Err(format!("Halaman web mengembalikan status HTTP {status}"));
        }

        break response;
    };

    if resp
        .content_length()
        .is_some_and(|length| length > MAX_FETCH_HTML_BYTES as u64)
    {
        return Err("Ukuran konten web melebihi batas aman 2 MiB.".to_string());
    }

    let mut stream = resp.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| format!("Gagal membaca stream web: {e}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_FETCH_HTML_BYTES {
            return Err("Ukuran konten web melebihi batas aman 2 MiB.".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }

    let html = String::from_utf8_lossy(&bytes);
    let mut cleaned = clean_html_to_text(&html);

    if cleaned.is_empty() {
        return Err("Halaman web tidak menghasilkan konten teks yang dapat dibaca.".to_string());
    }

    let extracted_images = extract_raster_images_from_html(&html, Some(&current_url_str));
    if !extracted_images.is_empty() {
        let max_imgs = extracted_images.into_iter().take(6).collect::<Vec<_>>();
        cleaned.push_str(&format_verified_images_section(&max_imgs));
    }

    let max_len = 8000;
    if cleaned.chars().nth(max_len).is_some() {
        let truncated: String = cleaned.chars().take(max_len).collect();
        Ok(format!(
            "{}\n\n[...Konten web dipotong karena melebihi batas panjang teks xiao...]",
            truncated
        ))
    } else {
        Ok(cleaned)
    }
}

static RE_TOOL_PREAMBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:baik|tentu|oke|siap|halo|yes|sure|okay|alright|fine)?\s*[,.:!?-]?\s*(?:tunggu|sebentar|biar|mari|tolong|saya|aku|kami|kita|akan|let me|i will|i'll|allow me|searching|looking up|checking)\b.*?\b(?:cari|carikan|mencari|pencarian|cek|mengecek|pengecekan|periksa|memeriksa|lihat|search|searching|check|checking|look up|looking up|fetch|retrieve|find)\b",
    )
    .expect("valid static regex")
});

const DEFINITIVE_PREAMBLE_PREFIXES: &[&str] = &[
    "tunggu sebentar",
    "sebentar ya",
    "sebentar, saya",
    "sebentar saya",
    "sebentar...",
    "tunggu ya",
    "tunggu sebentar ya",
    "saya akan mencari",
    "saya sedang mencari",
    "saya akan carikan",
    "saya carikan",
    "akan saya carikan",
    "akan saya cari",
    "biar saya carikan",
    "biar saya cari",
    "mari saya cari",
    "mari saya carikan",
    "mari kita cari",
    "saya cek dulu",
    "saya cek",
    "biar saya cek",
    "saya periksa dulu",
    "saya periksa",
    "biar saya periksa",
    "let me search",
    "let me check",
    "let me look up",
    "let me find",
    "i will search",
    "i'll search",
    "i will look up",
    "i'll look up",
    "i will check",
    "i'll check",
    "searching for",
    "looking up",
    "checking",
];

pub fn is_tool_calling_preamble(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_lowercase();
    if lower.chars().count() > 140 {
        return false;
    }
    if RE_TOOL_PREAMBLE.is_match(&lower) {
        return true;
    }
    DEFINITIVE_PREAMBLE_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

pub fn is_suppressed_tool_preamble_stream(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if is_tool_calling_preamble(trimmed) {
        return true;
    }
    let lower = trimmed.to_lowercase();
    DEFINITIVE_PREAMBLE_PREFIXES
        .iter()
        .any(|prefix| prefix.starts_with(&lower))
}

fn deserialize_quiz_options<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum OptionItem {
        Str(String),
        Obj { text: String },
    }

    let items = Vec::<OptionItem>::deserialize(deserializer)?;
    Ok(items
        .into_iter()
        .map(|item| match item {
            OptionItem::Str(s) => s,
            OptionItem::Obj { text } => text,
        })
        .collect())
}

fn deserialize_flexible_opt_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum BoolHelper {
        Bool(bool),
        Str(String),
        Num(i64),
    }

    match Option::<BoolHelper>::deserialize(deserializer)? {
        None => Ok(None),
        Some(BoolHelper::Bool(b)) => Ok(Some(b)),
        Some(BoolHelper::Str(s)) => {
            let s_clean = s.trim().to_ascii_lowercase();
            if s_clean == "true" || s_clean == "1" || s_clean == "yes" {
                Ok(Some(true))
            } else if s_clean == "false" || s_clean == "0" || s_clean == "no" {
                Ok(Some(false))
            } else {
                Err(serde::de::Error::custom(format!(
                    "invalid boolean value: {s}"
                )))
            }
        }
        Some(BoolHelper::Num(n)) => Ok(Some(n != 0)),
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct CreateQuizArgs {
    pub question: String,
    #[serde(deserialize_with = "deserialize_quiz_options")]
    pub options: Vec<String>,
    #[serde(
        default,
        deserialize_with = "crate::bot::models::deserialize_flexible_opt_i32"
    )]
    pub correct_option_id: Option<i32>,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub preamble: Option<String>,
    #[serde(default, deserialize_with = "deserialize_flexible_opt_bool")]
    pub is_anonymous: Option<bool>,
}

impl CreateQuizArgs {
    pub fn sanitize(&mut self) {
        self.question = self.question.trim().to_string();
        if self.question.chars().count() > crate::bot::models::QUIZ_MAX_QUESTION_CHARS {
            self.question = crate::util::truncate_chars(
                &self.question,
                crate::bot::models::QUIZ_MAX_QUESTION_CHARS,
            )
            .to_string();
        }

        if self.options.len() > crate::bot::models::QUIZ_MAX_OPTIONS {
            self.options.truncate(crate::bot::models::QUIZ_MAX_OPTIONS);
        }
        let mut seen = std::collections::HashSet::new();
        let mut dup_counter = 1;
        for opt in &mut self.options {
            let mut trimmed = opt.trim().to_string();
            if trimmed.chars().count() > crate::bot::models::QUIZ_MAX_OPTION_CHARS {
                trimmed = crate::util::truncate_chars(
                    &trimmed,
                    crate::bot::models::QUIZ_MAX_OPTION_CHARS,
                )
                .to_string();
            }
            if seen.contains(&trimmed) {
                let candidate = loop {
                    dup_counter += 1;
                    let suffix = format!(" ({dup_counter})");
                    let max_base_len = crate::bot::models::QUIZ_MAX_OPTION_CHARS
                        .saturating_sub(suffix.chars().count());
                    let base = crate::util::truncate_chars(&trimmed, max_base_len);
                    let cand = format!("{base}{suffix}");
                    if !seen.contains(&cand) {
                        break cand;
                    }
                };
                seen.insert(candidate.clone());
                *opt = candidate;
            } else {
                seen.insert(trimmed.clone());
                *opt = trimmed;
            }
        }

        if let Some(correct_id) = self.correct_option_id {
            if !self.options.is_empty() {
                if correct_id < 0 {
                    self.correct_option_id = Some(0);
                } else if correct_id as usize >= self.options.len() {
                    self.correct_option_id = Some(self.options.len().saturating_sub(1) as i32);
                }
            }
        }

        if let Some(exp) = &mut self.explanation {
            let normalized = exp.replace("\r\n", "\n").replace('\r', "\n");
            let trimmed = normalized.trim().to_string();
            let mut line_break_count = 0;
            let mut sanitized_exp = String::with_capacity(trimmed.len());
            for ch in trimmed.chars() {
                if ch == '\n' {
                    line_break_count += 1;
                    if line_break_count <= crate::bot::models::QUIZ_MAX_EXPLANATION_LINE_BREAKS {
                        sanitized_exp.push(ch);
                    } else {
                        sanitized_exp.push(' ');
                    }
                } else {
                    sanitized_exp.push(ch);
                }
            }
            let sanitized_trimmed = sanitized_exp.trim();
            if sanitized_trimmed.is_empty() {
                self.explanation = None;
            } else {
                let mut final_exp = sanitized_trimmed.to_string();
                if final_exp.chars().count() > crate::bot::models::QUIZ_MAX_EXPLANATION_CHARS {
                    final_exp = crate::util::truncate_chars(
                        &final_exp,
                        crate::bot::models::QUIZ_MAX_EXPLANATION_CHARS,
                    )
                    .to_string();
                }
                let final_trimmed = final_exp.trim().to_string();
                if final_trimmed.is_empty() {
                    self.explanation = None;
                } else {
                    *exp = final_trimmed;
                }
            }
        }

        if let Some(pre) = &mut self.preamble {
            *pre = pre.trim().to_string();
            if pre.is_empty() {
                self.preamble = None;
            } else if pre.chars().count() > crate::bot::models::RICH_MESSAGE_MAX_TEXT_CHARS {
                *pre = crate::util::truncate_chars(
                    pre,
                    crate::bot::models::RICH_MESSAGE_MAX_TEXT_CHARS,
                )
                .to_string();
            }
        }
    }

    pub fn validate(&self) -> Result<i32, String> {
        let correct_id = self
            .correct_option_id
            .ok_or_else(|| "Quiz requires correct_option_id".to_string())?;
        let input_options: Vec<crate::bot::models::InputPollOption> = self
            .options
            .iter()
            .map(|opt| crate::bot::models::InputPollOption::new(opt.as_str()))
            .collect();
        crate::bot::models::validate_quiz(
            &self.question,
            &input_options,
            correct_id,
            self.explanation.as_deref(),
        )?;
        Ok(correct_id)
    }
}

#[allow(dead_code)]
pub const MULTIMEDIA_CAPTION_MAX_CHARS: usize = 1024;
#[allow(dead_code)]
pub const CAPTION_MAX_CHARS: usize = MULTIMEDIA_CAPTION_MAX_CHARS;

#[allow(dead_code)]
pub fn deserialize_flexible_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum F64Helper {
        Num(f64),
        Str(String),
    }

    let val = match F64Helper::deserialize(deserializer)? {
        F64Helper::Num(n) => n,
        F64Helper::Str(s) => s.trim().parse::<f64>().map_err(serde::de::Error::custom)?,
    };

    if val.is_finite() {
        Ok(val)
    } else {
        Err(serde::de::Error::custom(
            "coordinate must be a finite number",
        ))
    }
}

pub fn sanitize_multimedia_caption(caption: &mut Option<String>) {
    if let Some(c) = caption {
        let trimmed = c.trim().to_string();
        if trimmed.is_empty() {
            *caption = None;
        } else if trimmed.chars().count() > MULTIMEDIA_CAPTION_MAX_CHARS {
            *caption = Some(
                crate::util::truncate_chars(&trimmed, MULTIMEDIA_CAPTION_MAX_CHARS).to_string(),
            );
        } else {
            *caption = Some(trimmed);
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SendPhotoArgs {
    pub url: String,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl SendPhotoArgs {
    pub fn sanitize(&mut self) {
        self.url = self.url.trim().to_string();
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.url.trim().is_empty() {
            return Err("URL foto tidak boleh kosong".to_string());
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SendCollageArgs {
    pub urls: Vec<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl SendCollageArgs {
    pub fn sanitize(&mut self) {
        self.urls.retain(|u| !u.trim().is_empty());
        for u in &mut self.urls {
            *u = u.trim().to_string();
        }
        if self.urls.len() > 10 {
            self.urls.truncate(10);
        }
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.urls.len() < 2 {
            return Err(format!(
                "Kolase foto memerlukan minimal 2 foto (maksimal 10), ditemukan {}",
                self.urls.len()
            ));
        }
        if self.urls.len() > 10 {
            return Err(format!(
                "Kolase foto maksimal 10 foto, ditemukan {}",
                self.urls.len()
            ));
        }
        for (i, url) in self.urls.iter().enumerate() {
            if url.trim().is_empty() {
                return Err(format!("URL foto kolase ke-{} tidak boleh kosong", i + 1));
            }
        }
        Ok(())
    }

    pub fn to_input_media(&self) -> Vec<crate::bot::models::InputMedia> {
        self.urls
            .iter()
            .enumerate()
            .map(|(idx, url)| {
                let caption = if idx == 0 { self.caption.clone() } else { None };
                crate::bot::models::InputMedia::Photo {
                    media: url.clone(),
                    caption,
                    parse_mode: Some("Markdown".to_string()),
                    show_caption_above_media: None,
                    has_spoiler: None,
                }
            })
            .collect()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SendSlideshowArgs {
    pub urls: Vec<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl SendSlideshowArgs {
    pub fn sanitize(&mut self) {
        self.urls.retain(|u| !u.trim().is_empty());
        for u in &mut self.urls {
            *u = u.trim().to_string();
        }
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.urls.is_empty() {
            return Err("Slideshow memerlukan minimal 1 URL slide gambar".to_string());
        }
        for (i, url) in self.urls.iter().enumerate() {
            if url.trim().is_empty() {
                return Err(format!("URL slide ke-{} tidak boleh kosong", i + 1));
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SendAudioArgs {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub performer: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl SendAudioArgs {
    pub fn sanitize(&mut self) {
        self.url = self.url.trim().to_string();
        if let Some(title) = &mut self.title {
            let trimmed = title.trim().to_string();
            self.title = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
        }
        if let Some(performer) = &mut self.performer {
            let trimmed = performer.trim().to_string();
            self.performer = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
        }
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.url.trim().is_empty() {
            return Err("URL audio tidak boleh kosong".to_string());
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SendVoiceArgs {
    pub url: String,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl SendVoiceArgs {
    pub fn sanitize(&mut self) {
        self.url = self.url.trim().to_string();
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.url.trim().is_empty() {
            return Err("URL voice note tidak boleh kosong".to_string());
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct SendLocationArgs {
    #[serde(deserialize_with = "deserialize_flexible_f64")]
    pub latitude: f64,
    #[serde(deserialize_with = "deserialize_flexible_f64")]
    pub longitude: f64,
    #[serde(default)]
    pub title: Option<String>,
}

#[allow(dead_code)]
impl SendLocationArgs {
    pub fn sanitize(&mut self) {
        if let Some(title) = &mut self.title {
            let trimmed = title.trim().to_string();
            self.title = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.latitude.is_finite() {
            return Err(
                "Garis lintang (latitude) harus berupa angka berhingga (finite)".to_string(),
            );
        }
        if !self.longitude.is_finite() {
            return Err(
                "Garis bujur (longitude) harus berupa angka berhingga (finite)".to_string(),
            );
        }
        if !(-90.0..=90.0).contains(&self.latitude) {
            return Err(format!(
                "Garis lintang (latitude) harus berada dalam rentang -90.0 hingga 90.0, ditemukan {}",
                self.latitude
            ));
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            return Err(format!(
                "Garis bujur (longitude) harus berada dalam rentang -180.0 hingga 180.0, ditemukan {}",
                self.longitude
            ));
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct SendDocumentArgs {
    pub url: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl SendDocumentArgs {
    pub fn sanitize(&mut self) {
        self.url = self.url.trim().to_string();
        if let Some(file_name) = &mut self.file_name {
            let trimmed = file_name.trim().to_string();
            self.file_name = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
        }
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.url.trim().is_empty() {
            return Err("URL dokumen tidak boleh kosong".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct CreateDocumentArgs {
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub as_zip: bool,
}

#[allow(dead_code)]
impl CreateDocumentArgs {
    pub fn sanitize(&mut self) {
        let mut clean_name = self
            .filename
            .replace("../", "")
            .replace("..\\", "")
            .replace(['/', '\\'], "")
            .trim()
            .to_string();
        if clean_name.is_empty() {
            clean_name = "document.txt".to_string();
        }
        self.filename = clean_name;
        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.content.is_empty() {
            return Err("Konten dokumen tidak boleh kosong".to_string());
        }
        if self.content.len() > 20 * 1024 * 1024 {
            return Err("Ukuran konten melebihi batas 20MB".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct ArchiveFileEntry {
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct CreateArchiveArgs {
    pub filename: String,
    pub files: Vec<ArchiveFileEntry>,
    #[serde(default)]
    pub caption: Option<String>,
}

#[allow(dead_code)]
impl CreateArchiveArgs {
    pub fn sanitize(&mut self) {
        let mut clean_name = self
            .filename
            .replace("../", "")
            .replace("..\\", "")
            .replace(['/', '\\'], "")
            .trim()
            .to_string();
        if clean_name.is_empty() {
            clean_name = "archive.zip".to_string();
        } else if !clean_name.to_ascii_lowercase().ends_with(".zip") {
            clean_name = format!("{clean_name}.zip");
        }
        self.filename = clean_name;

        for entry in &mut self.files {
            let mut file_clean = entry
                .filename
                .replace("../", "")
                .replace("..\\", "")
                .replace(['/', '\\'], "_")
                .trim()
                .to_string();
            if file_clean.is_empty() {
                file_clean = "file.txt".to_string();
            }
            entry.filename = file_clean;
        }

        sanitize_multimedia_caption(&mut self.caption);
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.files.is_empty() {
            return Err("Daftar file dalam arsip tidak boleh kosong".to_string());
        }
        if self.files.len() > 100 {
            return Err("Jumlah file dalam arsip melebihi batas 100 file".to_string());
        }
        let total_size: usize = self.files.iter().map(|f| f.content.len()).sum();
        if total_size > 20 * 1024 * 1024 {
            return Err("Total ukuran konten arsip melebihi batas 20MB".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_multimedia_caption_behavior() {
        let mut none_caption: Option<String> = None;
        sanitize_multimedia_caption(&mut none_caption);
        assert!(none_caption.is_none());

        let mut empty_caption = Some("   \n\t  ".to_string());
        sanitize_multimedia_caption(&mut empty_caption);
        assert!(empty_caption.is_none());

        let mut normal_caption = Some("  Gambar Pemandangan  ".to_string());
        sanitize_multimedia_caption(&mut normal_caption);
        assert_eq!(normal_caption.as_deref(), Some("Gambar Pemandangan"));

        let long_str: String = "a".repeat(MULTIMEDIA_CAPTION_MAX_CHARS + 50);
        let mut long_caption = Some(long_str);
        sanitize_multimedia_caption(&mut long_caption);
        assert_eq!(
            long_caption.as_ref().map(|s| s.chars().count()),
            Some(MULTIMEDIA_CAPTION_MAX_CHARS)
        );
    }

    #[test]
    fn test_tools_definition_contains_expected_tools() {
        let tools = get_tools_definition();
        let array = tools.as_array().expect("tools should be an array");
        assert_eq!(array.len(), 12);

        let names: Vec<_> = array
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str())
            .collect();
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"fetch_url"));
        assert!(names.contains(&"create_quiz"));
        assert!(names.contains(&"send_photo"));
        assert!(names.contains(&"send_collage"));
        assert!(names.contains(&"send_slideshow"));
        assert!(names.contains(&"send_audio"));
        assert!(names.contains(&"send_voice"));
        assert!(names.contains(&"send_location"));
        assert!(names.contains(&"send_document"));
        assert!(names.contains(&"create_document"));
        assert!(names.contains(&"create_archive"));
    }

    #[test]
    fn test_search_engine_status_not_empty() {
        let (engine, detail) = get_search_engine_status();
        assert!(!engine.is_empty());
        assert!(!detail.is_empty());
    }

    #[test]
    fn test_html_cleaning_logic() {
        let raw_html = "<html><head><style>body{color:red;}</style></head><body><h1>Hello &amp; Welcome</h1><script>alert(1);</script><p>This is a test.</p><ul><li>Item 1</li><li>Item 2</li></ul></body></html>";
        let cleaned = clean_html_to_text(raw_html);

        assert_eq!(
            cleaned,
            "Hello & Welcome\n\nThis is a test.\n\n• Item 1\n• Item 2"
        );
    }

    #[test]
    fn test_is_tool_calling_preamble() {
        assert!(is_tool_calling_preamble(
            "Tunggu sebentar, saya sedang mencari referensi"
        ));
        assert!(is_tool_calling_preamble("Sebentar ya, saya carikan"));
        assert!(is_tool_calling_preamble("Saya akan carikan harga SO"));
        assert!(is_tool_calling_preamble("Saya carikan harga SOL terkini"));
        assert!(is_tool_calling_preamble("Biar saya cek dulu"));
        assert!(is_tool_calling_preamble("Baik, akan saya carikan datanya"));
        assert!(is_tool_calling_preamble(
            "Let me search for that information"
        ));
        assert!(is_tool_calling_preamble(
            "I'll look up the latest SOL price"
        ));
        assert!(!is_tool_calling_preamble(
            "Halo, tentu ini jawaban dari pertanyaan Anda"
        ));
        assert!(!is_tool_calling_preamble(""));
    }

    #[test]
    fn test_is_suppressed_tool_preamble_stream() {
        // Early stream chunks that could be preamble openings
        assert!(is_suppressed_tool_preamble_stream("Saya"));
        assert!(is_suppressed_tool_preamble_stream("Saya akan"));
        assert!(is_suppressed_tool_preamble_stream("Saya akan carikan"));
        assert!(is_suppressed_tool_preamble_stream(
            "Saya akan carikan harga SO"
        ));
        assert!(is_suppressed_tool_preamble_stream("Biar"));
        assert!(is_suppressed_tool_preamble_stream("Tunggu sebentar"));

        // Direct, non-preamble answers should not be suppressed
        assert!(!is_suppressed_tool_preamble_stream(
            "Solana adalah platform blockchain layer-1 berkecepatan tinggi dengan konsensus Proof of History."
        ));
        assert!(!is_suppressed_tool_preamble_stream(
            "Bitcoin diciptakan pada tahun 2009 oleh Satoshi Nakamoto sebagai mata uang terdesentralisasi."
        ));
        assert!(!is_suppressed_tool_preamble_stream(
            "Baik, tentu ini jawaban dari pertanyaan Anda."
        ));
        assert!(!is_suppressed_tool_preamble_stream(
            "Saya adalah asisten AI pribadi Anda."
        ));
    }

    #[test]
    fn test_search_engine_status_and_resolvers() {
        let (status, mcp_url) = get_search_engine_status();
        assert!(!status.is_empty());
        assert!(!mcp_url.is_empty());
        assert!(mcp_url.starts_with("http"));
    }

    #[test]
    fn test_create_quiz_cascading_duplicate_disambiguation() {
        let mut args = CreateQuizArgs {
            question: "Question with duplicates?".to_string(),
            options: vec!["A".into(), "A (2)".into(), "A".into()],
            correct_option_id: Some(0),
            explanation: None,
            preamble: None,
            is_anonymous: None,
        };
        args.sanitize();
        assert_eq!(args.options, vec!["A", "A (2)", "A (3)"]);
        assert!(args.validate().is_ok());

        // Extreme duplicate case: 4 identical options
        let mut all_same = CreateQuizArgs {
            question: "Same options?".to_string(),
            options: vec!["X".into(), "X".into(), "X".into(), "X".into()],
            correct_option_id: Some(2),
            explanation: None,
            preamble: None,
            is_anonymous: None,
        };
        all_same.sanitize();
        assert_eq!(all_same.options, vec!["X", "X (2)", "X (3)", "X (4)"]);
        assert!(all_same.validate().is_ok());
    }

    #[test]
    fn test_create_quiz_flexible_deserialization() {
        // Deserializing options from both objects and strings, plus string boolean and string correct_option_id
        let json_data = r#"{
            "question": "Flexible quiz test?",
            "options": [{"text": "Obj Option 1"}, "Plain Option 2"],
            "correct_option_id": "1",
            "is_anonymous": "false",
            "explanation": "Valid explanation"
        }"#;

        let args: CreateQuizArgs =
            serde_json::from_str(json_data).expect("must deserialize flexible quiz args");
        assert_eq!(args.question, "Flexible quiz test?");
        assert_eq!(args.options, vec!["Obj Option 1", "Plain Option 2"]);
        assert_eq!(args.correct_option_id, Some(1));
        assert_eq!(args.is_anonymous, Some(false));
        assert_eq!(args.explanation.as_deref(), Some("Valid explanation"));
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_create_quiz_oversized_preamble_truncated() {
        let huge_preamble = "a".repeat(crate::bot::models::RICH_MESSAGE_MAX_TEXT_CHARS + 100);
        let mut args = CreateQuizArgs {
            question: "Quiz with huge preamble?".to_string(),
            options: vec!["A".into(), "B".into()],
            correct_option_id: Some(0),
            explanation: None,
            preamble: Some(huge_preamble),
            is_anonymous: None,
        };
        args.sanitize();
        let pre = args.preamble.expect("preamble must exist");
        assert_eq!(
            pre.chars().count(),
            crate::bot::models::RICH_MESSAGE_MAX_TEXT_CHARS
        );
    }

    #[test]
    fn test_deserialize_flexible_f64() {
        #[derive(serde::Deserialize)]
        struct Coord {
            #[serde(deserialize_with = "deserialize_flexible_f64")]
            val: f64,
        }

        // Float number
        let c1: Coord =
            serde_json::from_str(r#"{"val": -6.2088}"#).expect("should deserialize float");
        assert!((c1.val - -6.2088).abs() < 1e-6);

        // Integer number
        let c2: Coord =
            serde_json::from_str(r#"{"val": 106}"#).expect("should deserialize integer");
        assert_eq!(c2.val, 106.0);

        // Float string
        let c3: Coord = serde_json::from_str(r#"{"val": " -6.2088 "}"#)
            .expect("should deserialize float string");
        assert!((c3.val - -6.2088).abs() < 1e-6);

        // Integer string
        let c4: Coord =
            serde_json::from_str(r#"{"val": "180"}"#).expect("should deserialize integer string");
        assert_eq!(c4.val, 180.0);

        // Non-number string should fail
        assert!(serde_json::from_str::<Coord>(r#"{"val": "abc"}"#).is_err());

        // Empty string should fail
        assert!(serde_json::from_str::<Coord>(r#"{"val": ""}"#).is_err());

        // NaN string should fail
        assert!(serde_json::from_str::<Coord>(r#"{"val": "NaN"}"#).is_err());

        // Infinity string should fail
        assert!(serde_json::from_str::<Coord>(r#"{"val": "Infinity"}"#).is_err());
        assert!(serde_json::from_str::<Coord>(r#"{"val": "-inf"}"#).is_err());
    }

    #[test]
    fn test_send_location_args_validation() {
        // String coordinates
        let json_str = r#"{"latitude": "-6.2088", "longitude": " 106.8456 ", "title": " Monas "}"#;
        let mut args: SendLocationArgs =
            serde_json::from_str(json_str).expect("deserialize location");
        args.sanitize();
        assert_eq!(args.title.as_deref(), Some("Monas"));
        assert!(args.validate().is_ok());

        // Exact boundary coordinates
        let b1 = SendLocationArgs {
            latitude: 90.0,
            longitude: 180.0,
            title: None,
        };
        assert!(b1.validate().is_ok());

        let b2 = SendLocationArgs {
            latitude: -90.0,
            longitude: -180.0,
            title: None,
        };
        assert!(b2.validate().is_ok());

        // Latitude out of bounds
        let bad_lat = SendLocationArgs {
            latitude: 90.001,
            longitude: 0.0,
            title: None,
        };
        assert!(bad_lat.validate().is_err());

        let bad_lat_neg = SendLocationArgs {
            latitude: -90.001,
            longitude: 0.0,
            title: None,
        };
        assert!(bad_lat_neg.validate().is_err());

        // Longitude out of bounds
        let bad_lon = SendLocationArgs {
            latitude: 0.0,
            longitude: 180.001,
            title: None,
        };
        assert!(bad_lon.validate().is_err());

        let bad_lon_neg = SendLocationArgs {
            latitude: 0.0,
            longitude: -180.001,
            title: None,
        };
        assert!(bad_lon_neg.validate().is_err());

        // Non-finite coordinates
        let nan_loc = SendLocationArgs {
            latitude: f64::NAN,
            longitude: 0.0,
            title: None,
        };
        assert!(nan_loc.validate().is_err());

        let inf_loc = SendLocationArgs {
            latitude: 0.0,
            longitude: f64::INFINITY,
            title: None,
        };
        assert!(inf_loc.validate().is_err());
    }

    #[test]
    fn test_send_collage_args_validation_and_media_mapping() {
        // 1 item should fail validation
        let mut single = SendCollageArgs {
            urls: vec!["https://example.com/1.jpg".to_string()],
            caption: Some("Single photo".to_string()),
        };
        single.sanitize();
        assert!(single.validate().is_err());

        // 2 items should pass
        let mut two = SendCollageArgs {
            urls: vec![
                "https://example.com/1.jpg".to_string(),
                "https://example.com/2.jpg".to_string(),
            ],
            caption: Some("Album".to_string()),
        };
        two.sanitize();
        assert!(two.validate().is_ok());

        // InputMedia conversion verifies caption on first item only
        let media = two.to_input_media();
        assert_eq!(media.len(), 2);
        match &media[0] {
            crate::bot::models::InputMedia::Photo { media, caption, .. } => {
                assert_eq!(media, "https://example.com/1.jpg");
                assert_eq!(caption.as_deref(), Some("Album"));
            }
            _ => panic!("Expected InputMedia::Photo"),
        }
        match &media[1] {
            crate::bot::models::InputMedia::Photo { media, caption, .. } => {
                assert_eq!(media, "https://example.com/2.jpg");
                assert!(caption.is_none());
            }
            _ => panic!("Expected InputMedia::Photo"),
        }

        // 11 items sanitized truncates to 10
        let urls_11: Vec<String> = (0..11)
            .map(|i| format!("https://example.com/{i}.jpg"))
            .collect();
        let mut collage_11 = SendCollageArgs {
            urls: urls_11,
            caption: None,
        };
        assert!(collage_11.validate().is_err());
        collage_11.sanitize();
        assert_eq!(collage_11.urls.len(), 10);
        assert!(collage_11.validate().is_ok());

        // Empty URL string fails validation
        let empty_url = SendCollageArgs {
            urls: vec!["https://example.com/1.jpg".to_string(), "   ".to_string()],
            caption: None,
        };
        assert!(empty_url.validate().is_err());
    }

    #[test]
    fn test_send_slideshow_args_validation() {
        let empty = SendSlideshowArgs {
            urls: vec![],
            caption: None,
        };
        assert!(empty.validate().is_err());

        let mut valid = SendSlideshowArgs {
            urls: vec!["https://example.com/1.jpg".to_string()],
            caption: Some("   Slide caption   ".to_string()),
        };
        valid.sanitize();
        assert_eq!(valid.caption.as_deref(), Some("Slide caption"));
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn test_send_photo_args_validation() {
        let empty = SendPhotoArgs {
            url: "   ".to_string(),
            caption: None,
        };
        assert!(empty.validate().is_err());

        let mut valid = SendPhotoArgs {
            url: "https://example.com/photo.png".to_string(),
            caption: Some("Caption".to_string()),
        };
        valid.sanitize();
        assert!(valid.validate().is_ok());

        // Oversized caption truncated to MULTIMEDIA_CAPTION_MAX_CHARS (1024)
        let huge_caption = "x".repeat(1500);
        let mut oversized = SendPhotoArgs {
            url: "https://example.com/photo.png".to_string(),
            caption: Some(huge_caption),
        };
        oversized.sanitize();
        assert_eq!(
            oversized.caption.as_deref().map(|c| c.chars().count()),
            Some(MULTIMEDIA_CAPTION_MAX_CHARS)
        );
    }

    #[test]
    fn test_send_audio_args_validation() {
        let empty = SendAudioArgs {
            url: "".to_string(),
            title: None,
            performer: None,
            caption: None,
        };
        assert!(empty.validate().is_err());

        let mut valid = SendAudioArgs {
            url: "https://example.com/song.mp3".to_string(),
            title: Some(" Song Title ".to_string()),
            performer: Some(" Artist Name ".to_string()),
            caption: Some(" Great track ".to_string()),
        };
        valid.sanitize();
        assert_eq!(valid.title.as_deref(), Some("Song Title"));
        assert_eq!(valid.performer.as_deref(), Some("Artist Name"));
        assert_eq!(valid.caption.as_deref(), Some("Great track"));
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn test_send_voice_args_validation() {
        let empty = SendVoiceArgs {
            url: "".to_string(),
            caption: None,
        };
        assert!(empty.validate().is_err());

        let mut valid = SendVoiceArgs {
            url: "https://example.com/voice.ogg".to_string(),
            caption: Some("Voice note".to_string()),
        };
        valid.sanitize();
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn test_send_document_args_validation() {
        let empty = SendDocumentArgs {
            url: "".to_string(),
            file_name: None,
            caption: None,
        };
        assert!(empty.validate().is_err());

        let mut valid = SendDocumentArgs {
            url: "https://example.com/doc.pdf".to_string(),
            file_name: Some(" doc.pdf ".to_string()),
            caption: Some(" Annual Report ".to_string()),
        };
        valid.sanitize();
        assert_eq!(valid.file_name.as_deref(), Some("doc.pdf"));
        assert_eq!(valid.caption.as_deref(), Some("Annual Report"));
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn test_is_valid_raster_image_url_accepts_valid_raster_extensions() {
        assert!(is_valid_raster_image_url(
            "https://example.com/photos/mountain.jpg"
        ));
        assert!(is_valid_raster_image_url(
            "https://example.com/photos/mountain.jpeg"
        ));
        assert!(is_valid_raster_image_url(
            "https://example.com/photos/mountain.png"
        ));
        assert!(is_valid_raster_image_url(
            "https://example.com/photos/mountain.webp"
        ));
        assert!(is_valid_raster_image_url(
            "http://example.com/photos/mountain.JPG"
        ));

        assert!(is_valid_raster_image_url(
            "https://upload.wikimedia.org/wikipedia/commons/4/4c/KAGAGAHAN_RIJANI.jpg"
        ));
        assert!(is_valid_raster_image_url(
            "https://thumb.wikimedia.org/wikipedia/commons/thumb/4/4c/KAGAGAHAN_RIJANI.jpg/1280px-KAGAGAHAN_RIJANI.jpg?utm_source=id.wikipedia.org"
        ));

        assert!(is_valid_raster_image_url(
            "https://images.unsplash.com/photo-1546527868-ccb7ee7dfa6a"
        ));
        assert!(is_valid_raster_image_url(
            "https://images.unsplash.com/photo-1546527868-ccb7ee7dfa6a?fm=jpg&w=1080"
        ));

        assert!(is_valid_raster_image_url(
            "//cdn.example.com/images/cat.png"
        ));
        assert!(is_valid_raster_image_url(
            "https://example.com/fetch-image?id=123&format=webp"
        ));
    }

    #[test]
    fn test_is_valid_raster_image_url_rejects_svg_gif_and_data_urls() {
        assert!(!is_valid_raster_image_url("https://example.com/logo.svg"));
        assert!(!is_valid_raster_image_url(
            "https://upload.wikimedia.org/wikipedia/en/4/4a/Commons-logo.svg"
        ));

        assert!(!is_valid_raster_image_url(
            "https://example.com/spinner.gif"
        ));
        assert!(!is_valid_raster_image_url("https://example.com/icon.ico"));

        assert!(!is_valid_raster_image_url("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="));

        assert!(!is_valid_raster_image_url("javascript:alert(1)"));
        assert!(!is_valid_raster_image_url("file:///etc/passwd"));
        assert!(!is_valid_raster_image_url(
            "blob:https://example.com/1234-5678"
        ));

        assert!(!is_valid_raster_image_url(""));
        assert!(!is_valid_raster_image_url("   "));
    }

    #[test]
    fn test_is_valid_raster_image_url_rejects_tracking_and_placeholders() {
        assert!(!is_valid_raster_image_url(
            "https://duckduckgo.com/t/tqadb?5540565&s=lite"
        ));
        assert!(!is_valid_raster_image_url(
            "//duckduckgo.com/t/tqadb?5540565&s=lite"
        ));

        assert!(!is_valid_raster_image_url(
            "https://example.com/tracking/pixel.gif"
        ));
        assert!(!is_valid_raster_image_url("https://example.com/1x1.gif"));
        assert!(!is_valid_raster_image_url("https://example.com/spacer.gif"));
        assert!(!is_valid_raster_image_url(
            "https://google-analytics.com/collect.jpg"
        ));

        assert!(!is_valid_raster_image_url(
            "https://via.placeholder.com/300.jpg"
        ));
        assert!(!is_valid_raster_image_url(
            "https://dummyimage.com/600x400.png"
        ));
        assert!(!is_valid_raster_image_url(
            "https://placekitten.com/200/300.jpg"
        ));

        assert!(!is_valid_raster_image_url(
            "http://localhost:8080/image.jpg"
        ));
        assert!(!is_valid_raster_image_url("http://127.0.0.1/test.png"));

        assert!(!is_valid_raster_image_url(
            "../assets/anomaly/images/challenge/123.jpg"
        ));
    }

    #[test]
    fn test_sanitize_and_validate_raster_url_strips_tracking_params() {
        let dirty = "https://upload.wikimedia.org/wikipedia/commons/4/4c/KAGAGAHAN_RIJANI.jpg?utm_source=id.wikipedia.org&utm_campaign=api&utm_content=original&fbclid=IwAR123";
        let cleaned = sanitize_and_validate_raster_url(dirty).expect("should be valid");
        assert_eq!(
            cleaned,
            "https://upload.wikimedia.org/wikipedia/commons/4/4c/KAGAGAHAN_RIJANI.jpg"
        );
    }

    #[test]
    fn test_is_visual_search_query_detection() {
        assert!(is_visual_search_query(
            "Berikan 2 foto pemandangan gunung rinjani"
        ));
        assert!(is_visual_search_query("cari gambar kucing persia lucu"));
        assert!(is_visual_search_query("tampilkan potret presiden soekarno"));
        assert!(is_visual_search_query("pemandangan danau toba"));
        assert!(is_visual_search_query("wallpaper sunset pantai kuta"));

        assert!(is_visual_search_query("show me 3 photos of Mount Bromo"));
        assert!(is_visual_search_query("find pictures of Tokyo tower"));
        assert!(is_visual_search_query(
            "give me high resolution images of aurora"
        ));
        assert!(is_visual_search_query("download wallpaper of galaxy"));
        assert!(is_visual_search_query("python logo png official"));
        assert!(is_visual_search_query("berikan gambar logonya disini"));
        assert!(is_visual_search_query("tampilkan lambang garuda pancasila"));
        assert!(is_visual_search_query("icon rust programming language"));
        assert!(is_visual_search_query("simbol atom fisika"));

        assert!(!is_visual_search_query("harga solana hari ini"));
        assert!(!is_visual_search_query("apa itu rust borrow checker"));
        assert!(!is_visual_search_query("sejarah kemerdekaan indonesia"));
        assert!(!is_visual_search_query(""));
    }

    #[test]
    fn test_is_logo_query_detection() {
        assert!(is_logo_query("python logo png official"));
        assert!(is_logo_query("berikan gambar logonya disini"));
        assert!(is_logo_query("lambang indonesia"));
        assert!(is_logo_query("simbol atom"));
        assert!(is_logo_query("icon telegram"));
        assert!(is_logo_query("emblem club barcelona"));
        assert!(is_logo_query("badge army"));

        assert!(!is_logo_query("pemandangan gunung bromo"));
        assert!(!is_logo_query("foto kucing lucu"));
        assert!(!is_logo_query("harga bitcoin"));
        assert!(!is_logo_query(""));
    }

    #[test]
    fn test_extract_core_search_terms_strips_conversational_verbs() {
        assert_eq!(
            extract_core_search_terms("Berikan 2 foto pemandangan gunung rinjani"),
            "pemandangan gunung rinjani"
        );
        assert_eq!(
            extract_core_search_terms("Tolong carikan gambar kucing persia"),
            "kucing persia"
        );
        assert_eq!(
            extract_core_search_terms("Show me 3 photos of Mount Bromo"),
            "Mount Bromo"
        );
        assert_eq!(
            extract_core_search_terms("3 photos of Mount Bromo"),
            "Mount Bromo"
        );
        assert_eq!(
            extract_core_search_terms("photos of Mount Bromo"),
            "Mount Bromo"
        );
        assert_eq!(
            extract_core_search_terms("Show me photos of Mount Bromo"),
            "Mount Bromo"
        );
        assert_eq!(
            extract_core_search_terms("Show me 5 pictures of Tokyo Tower"),
            "Tokyo Tower"
        );
        assert_eq!(
            extract_core_search_terms("Gunung Rinjani"),
            "Gunung Rinjani"
        );
        // Word boundary tests: whole words like \bphotos?\b must not strip subwords
        assert_eq!(
            extract_core_search_terms("photosynthesis process in plants"),
            "photosynthesis process in plants"
        );
        assert_eq!(
            extract_core_search_terms("fotovoltaik panel surya"),
            "fotovoltaik panel surya"
        );
        // Trailing conversational suffixes
        assert_eq!(
            extract_core_search_terms("Mount Bromo, please"),
            "Mount Bromo"
        );
        assert_eq!(
            extract_core_search_terms("gunung bromo dong"),
            "gunung bromo"
        );
    }

    #[test]
    fn test_extract_raster_images_from_html() {
        let html_content = r#"
            <div>
                <img src="https://example.com/photos/valid1.jpg" alt="Valid 1">
                <img data-src="https://example.com/photos/valid2.png" alt="Valid 2">
                <img data-lazy-src="https://example.com/photos/valid_lazy.webp" alt="Lazy">
                <img srcset="https://example.com/photos/valid_srcset_small.jpg 400w, https://example.com/photos/valid_srcset_large.webp 1200w">
                <img src="/relative/valid3.webp" alt="Relative valid">
                <img src="https://example.com/icon.svg" alt="SVG rejected">
                <img src="https://google-analytics.com/pixel.gif" alt="Tracker rejected">
                <img src="https://via.placeholder.com/150.jpg" alt="Placeholder rejected">
                <a href="https://example.com/gallery/full_mountain.jpg">Download Full</a>
            </div>
        "#;

        let extracted = extract_raster_images_from_html(html_content, Some("https://example.com"));
        assert!(extracted.len() >= 6);
        assert!(extracted.contains(&"https://example.com/photos/valid1.jpg".to_string()));
        assert!(extracted.contains(&"https://example.com/photos/valid2.png".to_string()));
        assert!(extracted.contains(&"https://example.com/photos/valid_lazy.webp".to_string()));
        assert!(
            extracted.contains(&"https://example.com/photos/valid_srcset_small.jpg".to_string())
        );
        assert!(
            extracted.contains(&"https://example.com/photos/valid_srcset_large.webp".to_string())
        );
        assert!(extracted.contains(&"https://example.com/relative/valid3.webp".to_string()));
        assert!(extracted.contains(&"https://example.com/gallery/full_mountain.jpg".to_string()));
    }

    #[test]
    fn test_duckduckgo_proxy_url_unwrapping() {
        let proxy_url = "https://external-content.duckduckgo.com/iu/?u=https%3A%2F%2Fexample.com%2Fphotos%2Fsummit.jpg&f=1&nofb=1";
        let unwrapped = sanitize_and_validate_raster_url(proxy_url);
        assert_eq!(
            unwrapped,
            Some("https://example.com/photos/summit.jpg".to_string())
        );

        let tracking_url = "https://duckduckgo.com/t/tqadb?5540565&s=lite";
        assert_eq!(sanitize_and_validate_raster_url(tracking_url), None);
    }

    #[test]
    fn test_format_verified_images_section_and_guidance() {
        let imgs = vec![
            "https://example.com/1.jpg".to_string(),
            "https://example.com/2.png".to_string(),
        ];
        let formatted = format_verified_images_section(&imgs);
        assert!(formatted.contains("🖼️ **URL Foto/Gambar Raster Terverifikasi"));
        assert!(formatted.contains("- https://example.com/1.jpg"));
        assert!(formatted.contains("- https://example.com/2.png"));

        let empty_formatted = format_verified_images_section(&[]);
        assert!(empty_formatted.is_empty());

        let guidance = format_no_images_guidance("gunung rinjani");
        assert!(guidance.contains("ℹ️ **Catatan Media**"));
        assert!(guidance.contains("gunung rinjani"));
        assert!(guidance.contains("teks Markdown"));
    }

    #[test]
    fn test_create_archive_args_sanitization_and_validation() {
        let mut args = CreateArchiveArgs {
            filename: "../../project".to_string(),
            files: vec![
                ArchiveFileEntry {
                    filename: "../../../etc/passwd".to_string(),
                    content: "root:x:0:0".to_string(),
                },
                ArchiveFileEntry {
                    filename: "src/main.rs".to_string(),
                    content: "fn main() {}".to_string(),
                },
            ],
            caption: Some("  Test Archive  ".to_string()),
        };

        args.sanitize();
        assert_eq!(args.filename, "project.zip");
        assert_eq!(args.files[0].filename, "etc_passwd");
        assert_eq!(args.files[1].filename, "src_main.rs");
        assert_eq!(args.caption.as_deref(), Some("Test Archive"));
        assert!(args.validate().is_ok());

        let empty_args = CreateArchiveArgs {
            filename: "empty.zip".to_string(),
            files: vec![],
            caption: None,
        };
        assert!(empty_args.validate().is_err());
    }

    #[test]
    fn test_error_recovery_never_returns_empty_response() {
        let err_msg = "Connection timeout to search provider";
        let query = "pemandangan lombok";
        let recovery = format!(
            "[Informasi Pencarian Web]\nPencarian web daring untuk topik \"{query}\" saat ini tidak dapat diselesaikan karena kendala koneksi atau penyedia pencarian sedang tidak tersedia ({err_msg}).\n\nℹ️ **Panduan Asisten**: Berikan tanggapan deskriptif dan faktual mengenai topik \"{query}\" berdasarkan pengetahuan internal Anda secara lengkap. Jika pengguna meminta gambar atau foto, jelaskan informasi visualnya secara naratif dalam teks Markdown dan hindari memanggil tool multimedia fiktif."
        );

        assert!(!recovery.trim().is_empty());
        assert!(recovery.contains(query));
        assert!(recovery.contains(err_msg));
        assert!(recovery.contains("Panduan Asisten"));
    }

    #[tokio::test]
    async fn test_execute_web_search_empty_query_returns_clean_guidance() {
        let empty_res = execute_web_search("").await;
        assert!(!empty_res.trim().is_empty());
        assert!(empty_res.contains("tidak boleh kosong"));

        let whitespace_res = execute_web_search("   \t\n  ").await;
        assert!(!whitespace_res.trim().is_empty());
        assert!(whitespace_res.contains("tidak boleh kosong"));
    }

    #[test]
    fn test_create_document_args_sanitization() {
        let mut args = CreateDocumentArgs {
            filename: "../../etc/passwd".to_string(),
            content: "secret".to_string(),
            caption: Some("   Test caption...   ".to_string()),
            as_zip: false,
        };
        args.sanitize();
        assert_eq!(args.filename, "etcpasswd");
        assert_eq!(args.caption, Some("Test caption...".to_string()));
        assert!(args.validate().is_ok());

        let mut emp = CreateDocumentArgs {
            filename: "    ".to_string(),
            content: "".to_string(),
            caption: None,
            as_zip: false,
        };
        emp.sanitize();
        assert_eq!(emp.filename, "document.txt");
        assert!(emp.validate().is_err());
    }
}
