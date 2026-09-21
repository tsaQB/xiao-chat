use std::env;
use std::sync::LazyLock;
use std::time::Duration;

use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, REFERER, USER_AGENT};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};

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

pub fn get_tools_definition() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Cari informasi terkini atau referensi dari internet menggunakan mesin pencari.",
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
        return "Query pencarian tidak boleh kosong.".to_string();
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    // 1. Check Brave Search API
    if let Some(brave_key) = get_brave_key() {
        info!("Using Brave Search API for query: {q}");
        match search_brave(&client, &brave_key, q).await {
            Ok(res) => return res,
            Err(e) => warn!("Brave search failed ({e}), falling back to other providers"),
        }
    }

    // 2. Check Tavily API
    if let Some(tavily_key) = get_tavily_key() {
        info!("Using Tavily API for query: {q}");
        match search_tavily(&client, &tavily_key, q).await {
            Ok(res) => return res,
            Err(e) => warn!("Tavily search failed ({e}), falling back to other providers"),
        }
    }

    // 3. Check Exa REST API
    if let Some(exa_key) = get_exa_key() {
        info!("Using Exa API for query: {q}");
        match search_exa_api(&client, &exa_key, q).await {
            Ok(res) => return res,
            Err(e) => warn!("Exa API search failed ({e}), falling back to other providers"),
        }
    }

    // 4. Default / Keyless Exa MCP
    let mcp_url = get_configured_mcp_url();
    info!("Trying Exa Keyless MCP for query: {q}");
    match search_exa_mcp(&client, &mcp_url, q).await {
        Ok(res) => return res,
        Err(e) => {
            warn!("Gagal menghubungi Exa MCP ({e}), beralih ke DuckDuckGo...");
        }
    }

    // 5. DuckDuckGo Search Fallback
    info!("Using DuckDuckGo for query: {q}");
    match search_duckduckgo(&client, q).await {
        Ok(res) => return res,
        Err(e) => {
            warn!("DuckDuckGo did not return results or was blocked; trying Wikipedia knowledge base: {q}");
            warn!("Koneksi ke DuckDuckGo gagal ({e}). Catatan: Domain DuckDuckGo diblokir oleh beberapa ISP/Kominfo di Indonesia. Disarankan menggunakan TAVILY_API_KEY, EXA_API_KEY, atau BRAVE_API_KEY untuk hasil yang cepat.");
        }
    }

    // 6. Wikipedia Knowledge Base Fallback
    match search_wikipedia(&client, q).await {
        Ok(res) => res,
        Err(e) => format!("Tidak ditemukan hasil pencarian untuk \"{q}\": {e}"),
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
        }
    }

    if out.trim().is_empty() {
        Ok(format!(
            "Tidak ada hasil ditemukan di Brave untuk query \"{query}\"."
        ))
    } else {
        Ok(
            format!("[Hasil Pencarian Brave untuk \"{query}\"]\n\n{out}")
                .trim()
                .to_string(),
        )
    }
}

async fn search_tavily(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
) -> Result<String, String> {
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&json!({
            "api_key": api_key,
            "query": query,
            "include_answer": true,
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

    if let Some(results) = body.get("results").and_then(Value::as_array) {
        for (i, item) in results.iter().take(5).enumerate() {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Tanpa Judul");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            let content = item.get("content").and_then(Value::as_str).unwrap_or("");
            out.push_str(&format_search_item(i + 1, title, url, content));
        }
    }

    if out.trim().is_empty() {
        Ok(format!(
            "Tidak ada hasil ditemukan di Tavily untuk query \"{query}\"."
        ))
    } else {
        Ok(
            format!("[Hasil Pencarian Tavily untuk \"{query}\"]\n\n{out}")
                .trim()
                .to_string(),
        )
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

    // Exa MCP may return JSON or SSE with event/data
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
        // Parse SSE data: {...}
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

async fn search_duckduckgo(client: &reqwest::Client, query: &str) -> Result<String, String> {
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
        .filter_map(|c| c.name("url").map(|m| m.as_str().trim().to_string()))
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

    if out.trim().is_empty() {
        Err("Tidak ditemukan hasil pencarian".to_string())
    } else {
        Ok(format!("[Hasil Pencarian Web untuk \"{query}\"]\n\n{out}")
            .trim()
            .to_string())
    }
}

async fn search_wikipedia(client: &reqwest::Client, query: &str) -> Result<String, String> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&utf8=1&format=json",
        urlencoding::encode(query)
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
        .map_err(|e| format!("Gagal menghubungi Wikipedia: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Wikipedia API returned HTTP {status}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Gagal membaca JSON Wikipedia: {e}"))?;

    let mut out = String::new();
    if let Some(results) = body
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(Value::as_array)
    {
        for (i, item) in results.iter().take(5).enumerate() {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let snippet = item.get("snippet").and_then(Value::as_str).unwrap_or("");
            let clean_snippet = snippet
                .replace("<span class=\"searchmatch\">", "")
                .replace("</span>", "");
            let decoded_snippet = html_escape::decode_html_entities(&clean_snippet);
            let page_url = format!(
                "https://en.wikipedia.org/wiki/{}",
                urlencoding::encode(title)
            );
            out.push_str(&format_search_item(
                i + 1,
                title,
                &page_url,
                &decoded_snippet,
            ));
        }
    }

    if out.trim().is_empty() {
        Err(format!(
            "Tidak ada hasil ditemukan di ensiklopedia untuk query: \"{query}\""
        ))
    } else {
        Ok(
            format!("[Hasil Informasi Ensiklopedia Web untuk \"{query}\"]\n\n{out}")
                .trim()
                .to_string(),
        )
    }
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
    let cleaned = clean_html_to_text(&html);

    if cleaned.is_empty() {
        return Err("Halaman web tidak menghasilkan konten teks yang dapat dibaca.".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_definition_contains_expected_tools() {
        let tools = get_tools_definition();
        let array = tools.as_array().expect("tools should be an array");
        assert_eq!(array.len(), 3);

        let names: Vec<_> = array
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str())
            .collect();
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"fetch_url"));
        assert!(names.contains(&"create_quiz"));
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
}
