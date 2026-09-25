pub mod archive;

use regex::Regex;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::process::Command;
use zip::ZipArchive;

static RE_DOCX_PARAGRAPH_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</w:p>").expect("valid regex"));
static RE_DOCX_TAB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<w:tab\s*/>").expect("valid regex"));
static RE_DOCX_BREAKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<w:(br|cr)\s*/>").expect("valid regex"));
static RE_DOCX_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").expect("valid regex"));

static RE_XLSX_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<row\b[^>]*>(.*?)</row>"#).expect("valid regex"));
static RE_XLSX_CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<c\b([^>]*)>(.*?)</c>"#).expect("valid regex"));
static RE_XLSX_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<v>(.*?)</v>").expect("valid regex"));
static RE_XLSX_INLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<t[^>]*>(.*?)</t>").expect("valid regex"));
static RE_XLSX_SI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<si\b[^>]*>(.*?)</si>"#).expect("valid regex"));
static RE_XLSX_T: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<t\b[^>]*>(.*?)</t>"#).expect("valid regex"));

const MAX_EXTRACTED_TEXT_CHARS: usize = 1_500_000;
const MAX_SCANNED_PDF_PAGES: usize = 6;
const MAX_RENDERED_PDF_BYTES: usize = 12 * 1024 * 1024;
const MAX_ZIP_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PDF_STREAM_BYTES: usize = 8 * 1024 * 1024;
const MAX_XLSX_WORKSHEETS: usize = 64;
const MAX_XLSX_XML_BYTES_TOTAL: usize = 24 * 1024 * 1024;
const PDF_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(12);
const PDF_RENDER_TOTAL_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RENDERED_PAGE_DIMENSION: usize = 1600;

#[derive(Debug, Default)]
pub struct ExtractedDocument {
    pub text: Option<String>,
    pub rendered_pages: Vec<Vec<u8>>,
    pub warning: Option<String>,
}

pub const SUPPORTED_TEXT_EXTENSIONS: &[&str] = &[
    ".txt",
    ".md",
    ".markdown",
    ".json",
    ".csv",
    ".log",
    ".rs",
    ".go",
    ".py",
    ".js",
    ".ts",
    ".tsx",
    ".jsx",
    ".toml",
    ".yaml",
    ".yml",
    ".xml",
    ".html",
    ".css",
    ".sh",
    ".sql",
];

pub const SUPPORTED_DOC_EXTENSIONS: &[&str] = &[
    ".txt",
    ".md",
    ".markdown",
    ".json",
    ".csv",
    ".log",
    ".rs",
    ".go",
    ".py",
    ".js",
    ".ts",
    ".tsx",
    ".jsx",
    ".toml",
    ".yaml",
    ".yml",
    ".xml",
    ".html",
    ".css",
    ".sh",
    ".sql",
    ".pdf",
    ".docx",
    ".xlsx",
];

pub fn is_extractable_document(mime: &str, name: &str) -> bool {
    let mime = mime.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    archive::detect_archive_kind(&mime, &name).is_some()
        || mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/pdf"
        || mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || mime == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || SUPPORTED_DOC_EXTENSIONS
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

pub async fn extract_document(
    data: Vec<u8>,
    mime: &str,
    name: &str,
) -> Result<ExtractedDocument, String> {
    let mime = mime.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();

    if mime.starts_with("text/")
        || mime == "application/json"
        || SUPPORTED_TEXT_EXTENSIONS
            .iter()
            .any(|suffix| name_lower.ends_with(suffix))
    {
        let slice = if data.starts_with(b"\xef\xbb\xbf") {
            &data[3..]
        } else {
            &data[..]
        };
        let text = match std::str::from_utf8(slice) {
            Ok(valid) => valid.trim_start_matches('\u{feff}').to_string(),
            Err(_) => String::from_utf8_lossy(slice)
                .trim_start_matches('\u{feff}')
                .to_string(),
        };
        return Ok(ExtractedDocument {
            text: Some(limit_text(text)),
            ..Default::default()
        });
    }

    if mime == "application/pdf" || name_lower.ends_with(".pdf") {
        let pdf_bytes = data.clone();
        let parse_result =
            tokio::task::spawn_blocking(move || -> Result<(String, usize), String> {
                let document = lopdf::Document::load_mem_with_options(
                    &pdf_bytes,
                    lopdf::LoadOptions::with_max_decompressed_size(MAX_PDF_STREAM_BYTES),
                )
                .map_err(|err| format!("PDF tidak dapat dibaca: {err}"))?;
                let pages: Vec<u32> = document.get_pages().keys().copied().collect();
                let text = document
                    .extract_text_with_limit(&pages, MAX_PDF_STREAM_BYTES)
                    .map_err(|err| format!("Teks PDF tidak dapat diekstrak: {err}"))?;
                Ok((text, pages.len()))
            })
            .await;

        let page_count = match parse_result {
            Ok(Ok((extracted, count))) => {
                let cleaned = normalize_extracted_text(&extracted);
                if cleaned.chars().filter(|c| !c.is_whitespace()).count() >= 24 {
                    return Ok(ExtractedDocument {
                        text: Some(limit_text(cleaned)),
                        ..Default::default()
                    });
                }
                count
            }
            _ => MAX_SCANNED_PDF_PAGES,
        };

        match render_scanned_pdf_pages(&data, page_count).await {
            Ok(pages) if !pages.is_empty() => Ok(ExtractedDocument {
                text: None,
                rendered_pages: pages,
                warning: Some("PDF tampaknya berbasis gambar; halaman dirender dan akan dianalisis lewat vision model.".to_string()),
            }),
            Ok(_) => Err("PDF tidak memiliki teks yang dapat diekstrak dan renderer tidak menghasilkan halaman.".to_string()),
            Err(err) => Err(format!(
                "PDF tampaknya berupa scan/gambar dan memerlukan OCR/vision. {err}"
            )),
        }
    } else if mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || name_lower.ends_with(".docx")
    {
        let bytes = data;
        let text = tokio::task::spawn_blocking(move || extract_docx_text(&bytes))
            .await
            .map_err(|err| format!("Task extractor DOCX gagal: {err}"))??;
        Ok(ExtractedDocument {
            text: Some(limit_text(text)),
            ..Default::default()
        })
    } else if mime == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || name_lower.ends_with(".xlsx")
    {
        let bytes = data;
        let text = tokio::task::spawn_blocking(move || extract_xlsx_text(&bytes))
            .await
            .map_err(|err| format!("Task extractor XLSX gagal: {err}"))??;
        Ok(ExtractedDocument {
            text: Some(limit_text(text)),
            ..Default::default()
        })
    } else if let Some(kind) = archive::detect_archive_kind(&mime, &name_lower) {
        let bytes = data;
        let archive_name = name.to_string();
        let text = tokio::task::spawn_blocking(move || {
            archive::extract_archive(&bytes, kind, &archive_name)
        })
        .await
        .map_err(|err| format!("Task extractor arsip gagal: {err}"))??;
        Ok(ExtractedDocument {
            text: Some(limit_text(text)),
            ..Default::default()
        })
    } else {
        Err("Format dokumen belum didukung extractor Xiao.".to_string())
    }
}

fn extract_docx_text(data: &[u8]) -> Result<String, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(data)).map_err(|err| format!("DOCX invalid: {err}"))?;
    let mut file = archive
        .by_name("word/document.xml")
        .map_err(|err| format!("DOCX tidak memiliki word/document.xml: {err}"))?;
    if file.size() > MAX_ZIP_XML_BYTES {
        return Err(
            "Entry word/document.xml melebihi batas ukuran dekompresi yang aman.".to_string(),
        );
    }
    let mut xml = String::new();
    let bytes_read = (&mut file)
        .take(MAX_ZIP_XML_BYTES + 1)
        .read_to_string(&mut xml)
        .map_err(|err| format!("Gagal membaca XML DOCX: {err}"))?;
    if bytes_read as u64 > MAX_ZIP_XML_BYTES {
        return Err(
            "Entry word/document.xml melebihi batas ukuran dekompresi yang aman.".to_string(),
        );
    }

    let xml = RE_DOCX_PARAGRAPH_END.replace_all(&xml, "\n");
    let xml = RE_DOCX_TAB.replace_all(&xml, "\t");
    let xml = RE_DOCX_BREAKS.replace_all(&xml, "\n");
    let stripped = RE_DOCX_TAGS.replace_all(&xml, "");
    Ok(normalize_extracted_text(
        html_escape::decode_html_entities(&stripped).as_ref(),
    ))
}

fn extract_xlsx_text(data: &[u8]) -> Result<String, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(data)).map_err(|err| format!("XLSX invalid: {err}"))?;
    let shared = read_zip_text_optional(&mut archive, "xl/sharedStrings.xml")?;
    let mut xml_budget = 0usize;
    if let Some(shared_xml) = shared.as_deref() {
        add_xlsx_xml_budget(&mut xml_budget, shared_xml.len())?;
    }
    let shared_strings = shared
        .as_deref()
        .map(extract_shared_strings)
        .transpose()?
        .unwrap_or_default();

    let mut worksheet_names = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|err| err.to_string())?;
        let name = entry.name().to_string();
        if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
            worksheet_names.push(name);
            if worksheet_names.len() > MAX_XLSX_WORKSHEETS {
                return Err(format!("XLSX memiliki lebih dari {MAX_XLSX_WORKSHEETS} worksheet; ditolak untuk mencegah resource exhaustion."));
            }
        }
    }
    worksheet_names.sort();

    let mut output = String::new();

    for sheet_name in worksheet_names {
        let xml = read_zip_text(&mut archive, &sheet_name)?;
        add_xlsx_xml_budget(&mut xml_budget, xml.len())?;
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!(
            "[{}]\n",
            Path::new(&sheet_name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("sheet")
        ));
        let mut sheet_rows = Vec::new();
        for row_caps in RE_XLSX_ROW.captures_iter(&xml) {
            let row_body = row_caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let mut row_values = Vec::new();
            for captures in RE_XLSX_CELL.captures_iter(row_body) {
                let attrs = captures.get(1).map(|m| m.as_str()).unwrap_or("");
                let body = captures.get(2).map(|m| m.as_str()).unwrap_or("");
                let value = if attrs.contains("t=\"s\"") {
                    RE_XLSX_VALUE
                        .captures(body)
                        .and_then(|caps| caps.get(1))
                        .and_then(|m| m.as_str().trim().parse::<usize>().ok())
                        .and_then(|idx| shared_strings.get(idx).cloned())
                        .unwrap_or_default()
                } else if attrs.contains("t=\"inlineStr\"") {
                    RE_XLSX_INLINE
                        .captures(body)
                        .and_then(|caps| caps.get(1))
                        .map(|m| html_escape::decode_html_entities(m.as_str()).to_string())
                        .unwrap_or_default()
                } else {
                    RE_XLSX_VALUE
                        .captures(body)
                        .and_then(|caps| caps.get(1))
                        .map(|m| m.as_str().trim().to_string())
                        .unwrap_or_default()
                };
                if !value.is_empty() {
                    row_values.push(value);
                }
            }
            if !row_values.is_empty() {
                sheet_rows.push(row_values.join("\t"));
            }
        }
        if !sheet_rows.is_empty() {
            output.push_str(&sheet_rows.join("\n"));
        }
    }

    let normalized = normalize_extracted_text(&output);
    if normalized.trim().is_empty() {
        Err("XLSX tidak mengandung nilai sel yang dapat diekstrak.".to_string())
    } else {
        Ok(normalized)
    }
}

fn add_xlsx_xml_budget(total: &mut usize, additional: usize) -> Result<(), String> {
    let next = total.saturating_add(additional);
    if next > MAX_XLSX_XML_BYTES_TOTAL {
        return Err(format!(
            "Total XML XLSX melebihi batas aman {} MiB.",
            MAX_XLSX_XML_BYTES_TOTAL / (1024 * 1024)
        ));
    }
    *total = next;
    Ok(())
}

fn read_zip_text_optional<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<String>, String> {
    match archive.by_name(name) {
        Ok(mut file) => {
            if file.size() > MAX_ZIP_XML_BYTES {
                return Err(format!(
                    "Entry {name} terlalu besar untuk diekstrak dengan aman."
                ));
            }
            let mut value = String::new();
            let bytes_read = (&mut file)
                .take(MAX_ZIP_XML_BYTES + 1)
                .read_to_string(&mut value)
                .map_err(|err| format!("Gagal membaca {name}: {err}"))?;
            if bytes_read as u64 > MAX_ZIP_XML_BYTES {
                return Err(format!(
                    "Entry {name} melebihi batas ukuran dekompresi yang aman."
                ));
            }
            Ok(Some(value))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(err) => Err(format!("Gagal membuka {name}: {err}")),
    }
}

fn read_zip_text<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<String, String> {
    read_zip_text_optional(archive, name)?.ok_or_else(|| format!("{name} tidak ditemukan"))
}

fn extract_shared_strings(xml: &str) -> Result<Vec<String>, String> {
    let mut strings = Vec::new();
    for si_caps in RE_XLSX_SI.captures_iter(xml) {
        let si_body = si_caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let mut entry = String::new();
        for t_caps in RE_XLSX_T.captures_iter(si_body) {
            if let Some(m) = t_caps.get(1) {
                entry.push_str(&html_escape::decode_html_entities(m.as_str()));
            }
        }
        strings.push(entry);
    }
    Ok(strings)
}

fn normalize_extracted_text(text: &str) -> String {
    let mut output = String::new();
    let mut previous_blank = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if !previous_blank && !output.is_empty() {
                output.push('\n');
            }
            previous_blank = true;
        } else {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
            previous_blank = false;
        }
    }
    output.trim().to_string()
}

fn limit_text(text: String) -> String {
    if text.chars().count() <= MAX_EXTRACTED_TEXT_CHARS {
        text
    } else {
        let mut limited: String = text.chars().take(MAX_EXTRACTED_TEXT_CHARS).collect();
        limited.push_str("\n\n[Dokumen dipotong oleh batas konteks extractor Xiao]");
        limited
    }
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_base_dir() -> PathBuf {
    let candidate = std::env::temp_dir();
    if candidate.is_dir() {
        candidate
    } else {
        crate::ai::storage::xiao_data_dir().join("tmp")
    }
}

async fn render_scanned_pdf_pages(data: &[u8], page_count: usize) -> Result<Vec<Vec<u8>>, String> {
    let suffix: u64 = rand::random();
    let temp_dir = temp_base_dir().join(format!("xiao-pdf-{suffix}"));
    let _guard = TempDirGuard(temp_dir.clone());
    let input = temp_dir.join("input.pdf");

    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|err| format!("Gagal membuat direktori PDF sementara: {err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&temp_dir, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|err| format!("Gagal mengamankan direktori PDF sementara: {err}"))?;
    }
    tokio::fs::write(&input, data)
        .await
        .map_err(|err| format!("Gagal menulis PDF sementara: {err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|err| format!("Gagal mengamankan PDF sementara: {err}"))?;
    }

    let render_result = tokio::time::timeout(PDF_RENDER_TOTAL_TIMEOUT, async {
        let mut pages = Vec::new();
        let mut total = 0usize;
        let pages_to_render = page_count.min(MAX_SCANNED_PDF_PAGES);

        for index in 1..=pages_to_render {
            let prefix = temp_dir.join(format!("page-{index}"));
            let output_path = prefix.with_extension("png");
            let mut child = Command::new("pdftoppm")
                .arg("-png")
                .arg("-q")
                .arg("-scale-to")
                .arg(MAX_RENDERED_PAGE_DIMENSION.to_string())
                .arg("-f")
                .arg(index.to_string())
                .arg("-l")
                .arg(index.to_string())
                .arg("-singlefile")
                .arg(&input)
                .arg(&prefix)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|err| {
                    format!(
                        "Renderer pdftoppm tidak tersedia ({err}). Instal 'poppler' (Termux: pkg install poppler) atau 'poppler-utils' (Linux: apt install poppler-utils) untuk OCR PDF scan."
                    )
                })?;

            let status = match tokio::time::timeout(PDF_PAGE_RENDER_TIMEOUT, child.wait()).await {
                Ok(Ok(status)) => status,
                Ok(Err(err)) => return Err(format!("pdftoppm gagal dijalankan: {err}")),
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(format!(
                        "pdftoppm melebihi timeout {} detik per halaman.",
                        PDF_PAGE_RENDER_TIMEOUT.as_secs()
                    ));
                }
            };
            if !status.success() {
                if index > 1 && !pages.is_empty() {
                    break;
                }
                return Err(format!("pdftoppm gagal saat merender halaman {index}."));
            }

            let metadata = match tokio::fs::metadata(&output_path).await {
                Ok(meta) => meta,
                Err(err) => {
                    if index > 1 && !pages.is_empty() {
                        break;
                    }
                    return Err(format!("Output pdftoppm halaman {index} tidak tersedia: {err}"));
                }
            };
            let page_size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
            if page_size > MAX_RENDERED_PDF_BYTES
                || total.saturating_add(page_size) > MAX_RENDERED_PDF_BYTES
            {
                return Err(format!(
                    "Output render PDF melebihi batas aman {} MiB.",
                    MAX_RENDERED_PDF_BYTES / (1024 * 1024)
                ));
            }

            let bytes = tokio::fs::read(&output_path)
                .await
                .map_err(|err| format!("Gagal membaca render PDF halaman {index}: {err}"))?;
            total = total.saturating_add(bytes.len());
            pages.push(bytes);
            let _ = tokio::fs::remove_file(&output_path).await;
        }

        Ok(pages)
    })
    .await
    .unwrap_or_else(|_| {
        Err(format!(
            "Render PDF melebihi timeout total {} detik.",
            PDF_RENDER_TOTAL_TIMEOUT.as_secs()
        ))
    });

    render_result
}

pub use crate::ai::tools::ArchiveFileEntry;

pub fn create_in_memory_multi_file_zip(entries: &[ArchiveFileEntry]) -> Result<Vec<u8>, String> {
    if entries.is_empty() {
        return Err("Daftar file arsip tidak boleh kosong".to_string());
    }

    let estimated_size = entries
        .iter()
        .map(|entry| entry.content.len() / 2 + 128)
        .sum::<usize>()
        + 512;
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(estimated_size));
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        for entry in entries {
            let clean_name = entry
                .filename
                .replace("../", "")
                .replace("..\\", "")
                .replace(['/', '\\'], "_")
                .trim()
                .to_string();
            let final_name = if clean_name.is_empty() {
                "file.txt"
            } else {
                &clean_name
            };

            writer
                .start_file(final_name, options)
                .map_err(|e| format!("Gagal zip start_file untuk '{final_name}': {e}"))?;
            std::io::Write::write_all(&mut writer, entry.content.as_bytes())
                .map_err(|e| format!("Gagal write ke zip untuk '{final_name}': {e}"))?;
        }
        writer
            .finish()
            .map_err(|e| format!("Gagal finish multi-file zip: {e}"))?;
    }
    Ok(cursor.into_inner())
}

pub fn create_in_memory_zip(filename: &str, content: &[u8]) -> Result<Vec<u8>, String> {
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(content.len() / 2 + 512));
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file(filename, options)
            .map_err(|e| format!("Gagal zip start_file: {e}"))?;
        std::io::Write::write_all(&mut writer, content)
            .map_err(|e| format!("Gagal write ke zip: {e}"))?;
        writer
            .finish()
            .map_err(|e| format!("Gagal finish zip: {e}"))?;
    }
    Ok(cursor.into_inner())
}

pub fn escape_pdf_text(s: &str) -> String {
    let mut normalized = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => normalized.push_str("\\\\"),
            '(' => normalized.push_str("\\("),
            ')' => normalized.push_str("\\)"),
            '\u{2014}' | '\u{2013}' => normalized.push('-'), // em-dash / en-dash
            '\u{201C}' | '\u{201D}' => normalized.push('"'), // smart double quotes
            '\u{2018}' | '\u{2019}' => normalized.push('\''), // smart single quotes
            '\u{2026}' => normalized.push_str("..."),        // ellipsis
            c if (c as u32) < 128 => normalized.push(c),
            c if (c as u32) <= 255 => normalized.push(c),
            _ => normalized.push('?'),
        }
    }
    normalized
}

pub fn create_in_memory_pdf(title: &str, content: &str) -> Result<Vec<u8>, String> {
    let mut lines = Vec::new();
    for raw_line in content.lines() {
        let trimmed = raw_line.trim_end();
        if trimmed.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let mut cur = String::new();
        for w in words {
            if !cur.is_empty() && cur.len() + w.len() + 1 > 75 {
                lines.push(cur);
                cur = w.to_string();
            } else if cur.is_empty() {
                cur = w.to_string();
            } else {
                cur.push(' ');
                cur.push_str(w);
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }

    let lines_page_0 = if !title.is_empty() { 40 } else { 45 };
    let lines_subsequent = 45;
    let mut pages: Vec<Vec<String>> = Vec::new();
    if lines.is_empty() {
        pages.push(vec![String::new()]);
    } else {
        let mut remaining = &lines[..];
        let mut is_first = true;
        while !remaining.is_empty() {
            let cap = if is_first {
                lines_page_0
            } else {
                lines_subsequent
            };
            let take = cap.min(remaining.len());
            pages.push(remaining[..take].to_vec());
            remaining = &remaining[take..];
            is_first = false;
        }
    }

    let num_pages = pages.len();
    let mut objects = Vec::new();
    // obj 1: Catalog
    objects.push("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string());

    // obj 2: Pages
    let mut kids = String::new();
    for i in 0..num_pages {
        if !kids.is_empty() {
            kids.push(' ');
        }
        kids.push_str(&format!("{} 0 R", 3 + i * 2));
    }
    objects.push(format!(
        "2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {num_pages} >>\nendobj\n"
    ));

    for (i, page_lines) in pages.iter().enumerate() {
        let page_obj_num = 3 + i * 2;
        let content_obj_num = 4 + i * 2;

        let page_str = format!(
            "{page_obj_num} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.28 841.89] /Contents {content_obj_num} 0 R /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> /F2 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >> >> >> >>\nendobj\n"
        );
        objects.push(page_str);

        let mut stream_cmds = Vec::new();
        stream_cmds.push("BT".to_string());

        if i == 0 && !title.is_empty() {
            let clean_title = escape_pdf_text(title);
            stream_cmds.push("/F2 16 Tf".to_string());
            stream_cmds.push("50 790 Td".to_string());
            stream_cmds.push(format!("({clean_title}) Tj"));
            stream_cmds.push("/F1 10 Tf".to_string());
            stream_cmds.push("0 -24 Td".to_string());
            stream_cmds.push("14 TL".to_string());
        } else {
            stream_cmds.push("/F1 10 Tf".to_string());
            stream_cmds.push("50 790 Td".to_string());
            stream_cmds.push("14 TL".to_string());
        }

        for line in page_lines {
            let safe_line = escape_pdf_text(line);
            stream_cmds.push(format!("({safe_line}) '"));
        }
        stream_cmds.push("ET\n".to_string());

        let stream_content = stream_cmds.join("\n");
        let stream_len = stream_content.len();

        let content_str = format!(
            "{content_obj_num} 0 obj\n<< /Length {stream_len} >>\nstream\n{stream_content}endstream\nendobj\n"
        );
        objects.push(content_str);
    }

    let header: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n";
    let mut offsets = Vec::new();
    let mut current_offset = header.len();
    let mut body = String::new();

    for obj in &objects {
        offsets.push(current_offset);
        current_offset += obj.len();
        body.push_str(obj);
    }

    let xref_offset = current_offset;
    let total_objs = objects.len() + 1;
    let mut xref = format!("xref\n0 {total_objs}\n0000000000 65535 f \n");
    for off in offsets {
        xref.push_str(&format!("{off:010} 00000 n \n"));
    }

    let trailer =
        format!("trailer\n<< /Size {total_objs} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n");

    let mut result = Vec::with_capacity(header.len() + body.len() + xref.len() + trailer.len());
    result.extend_from_slice(header);
    result.extend_from_slice(body.as_bytes());
    result.extend_from_slice(xref.as_bytes());
    result.extend_from_slice(trailer.as_bytes());

    Ok(result)
}

pub fn create_document_payload(
    filename: &str,
    content: &str,
    as_zip: bool,
) -> (Vec<u8>, String, String) {
    if as_zip {
        let zip_name = if !filename.to_ascii_lowercase().ends_with(".zip") {
            format!("{}.zip", filename)
        } else {
            filename.to_string()
        };
        if let Ok(z) = create_in_memory_zip(filename, content.as_bytes()) {
            (z, zip_name, "application/zip".to_string())
        } else {
            let mime = detect_mime_from_filename(filename).to_string();
            (content.as_bytes().to_vec(), filename.to_string(), mime)
        }
    } else if filename.to_ascii_lowercase().ends_with(".pdf") && !content.starts_with("%PDF-") {
        if let Ok(pdf_bytes) = create_in_memory_pdf(filename, content) {
            (
                pdf_bytes,
                filename.to_string(),
                "application/pdf".to_string(),
            )
        } else {
            let mime = detect_mime_from_filename(filename);
            let truthful_mime = if mime == "application/pdf" {
                "text/plain".to_string()
            } else {
                mime.to_string()
            };
            (
                content.as_bytes().to_vec(),
                filename.to_string(),
                truthful_mime,
            )
        }
    } else {
        let mime = detect_mime_from_filename(filename).to_string();
        (content.as_bytes().to_vec(), filename.to_string(), mime)
    }
}

pub fn detect_mime_from_filename(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        "application/zip"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".csv") {
        "text/csv"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "application/x-yaml"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else if lower.ends_with(".md") {
        "text/markdown"
    } else if lower.ends_with(".xml") {
        "application/xml"
    } else if lower.ends_with(".txt")
        || lower.ends_with(".py")
        || lower.ends_with(".rs")
        || lower.ends_with(".js")
        || lower.ends_with(".ts")
    {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn recognizes_supported_documents() {
        assert!(is_extractable_document("application/pdf", "x.bin"));
        assert!(is_extractable_document("", "notes.docx"));
        assert!(is_extractable_document("application/zip", "bundle"));
        assert!(is_extractable_document(
            "application/octet-stream",
            "archive.zip"
        ));
        assert!(is_extractable_document("", "project.tar.gz"));
        assert!(is_extractable_document("", "data.7z"));
        assert!(!is_extractable_document(
            "application/octet-stream",
            "archive.iso"
        ));
        assert!(!is_extractable_document("", "program.exe"));
    }

    #[test]
    fn xlsx_aggregate_xml_budget_is_bounded() {
        let mut total = 0usize;
        assert!(add_xlsx_xml_budget(&mut total, MAX_XLSX_XML_BYTES_TOTAL).is_ok());
        assert!(add_xlsx_xml_budget(&mut total, 1).is_err());
    }

    #[test]
    fn docx_xml_is_extracted() {
        let cursor = Cursor::new(Vec::<u8>::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("word/document.xml", options)
            .expect("start_file succeeds");
        writer
            .write_all(br#"<w:document><w:body><w:p><w:r><w:t>Hello &amp; world</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body></w:document>"#)
            .expect("write_all succeeds");
        let bytes = writer
            .finish()
            .expect("finish writer succeeds")
            .into_inner();
        let text = extract_docx_text(&bytes).expect("extract_docx_text succeeds");
        assert!(text.contains("Hello & world"));
        assert!(text.contains("Second"));
    }

    #[test]
    fn xlsx_extracts_rows_and_handles_rich_text_si() {
        let cursor = Cursor::new(Vec::<u8>::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();

        writer
            .start_file("xl/sharedStrings.xml", options)
            .expect("start_file succeeds");
        writer
            .write_all(
                br#"<sst count="2" uniqueCount="2">
                    <si><r><t>Rich </t></r><r><t>Text</t></r></si>
                    <si><t>Second String</t></si>
                </sst>"#,
            )
            .expect("write_all succeeds");

        writer
            .start_file("xl/worksheets/sheet1.xml", options)
            .expect("start_file succeeds");
        writer
            .write_all(
                br#"<worksheet>
                    <sheetData>
                        <row r="1">
                            <c r="A1" t="s"><v>0</v></c>
                            <c r="B1" t="s"><v>1</v></c>
                        </row>
                        <row r="2">
                            <c r="A2"><v>100</v></c>
                            <c r="B2"><v>200</v></c>
                        </row>
                    </sheetData>
                </worksheet>"#,
            )
            .expect("write_all succeeds");

        let bytes = writer
            .finish()
            .expect("finish writer succeeds")
            .into_inner();
        let text = extract_xlsx_text(&bytes).expect("extract_xlsx_text succeeds");
        assert!(text.contains("Rich Text\tSecond String"));
        assert!(text.contains("100\t200"));
        assert!(text.contains("Rich Text\tSecond String\n100\t200"));
    }

    #[test]
    fn zip_bomb_entry_over_decompression_limit_is_rejected() {
        let cursor = Cursor::new(Vec::<u8>::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("word/document.xml", options)
            .expect("start_file succeeds");
        // Write repeating spaces/zeroes that compress to very few bytes but decompress beyond limit
        let big_chunk = vec![b' '; 1024 * 1024]; // 1 MiB chunk
        for _ in 0..9 {
            // 9 MiB > MAX_ZIP_XML_BYTES (8 MiB)
            writer.write_all(&big_chunk).expect("write_all succeeds");
        }
        let bytes = writer
            .finish()
            .expect("finish writer succeeds")
            .into_inner();
        let err = extract_docx_text(&bytes).expect_err("zip bomb should be rejected");
        assert!(err.contains("melebihi batas ukuran dekompresi yang aman"));
    }

    #[test]
    fn zip_bomb_with_forged_header_is_rejected_by_stream_limit() {
        let cursor = Cursor::new(Vec::<u8>::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("word/document.xml", options)
            .expect("start_file succeeds");
        let big_chunk = vec![b' '; 1024 * 1024];
        for _ in 0..9 {
            writer.write_all(&big_chunk).expect("write_all succeeds");
        }
        let mut bytes = writer
            .finish()
            .expect("finish writer succeeds")
            .into_inner();
        if let Some(pos) = bytes.windows(4).position(|w| w == [0x50, 0x4b, 0x03, 0x04]) {
            bytes[pos + 22..pos + 26].copy_from_slice(&100_u32.to_le_bytes());
        }
        if let Some(pos) = bytes.windows(4).position(|w| w == [0x50, 0x4b, 0x01, 0x02]) {
            bytes[pos + 24..pos + 28].copy_from_slice(&100_u32.to_le_bytes());
        }
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).expect("open zip archive succeeds");
        let file = archive
            .by_name("word/document.xml")
            .expect("find file in zip archive succeeds");
        assert_eq!(file.size(), 100);
        drop(file);
        drop(archive);

        let err = extract_docx_text(&bytes).expect_err("forged header zip bomb should be rejected");
        assert!(err.contains("melebihi batas ukuran dekompresi yang aman"));
    }

    #[tokio::test]
    async fn text_document_strips_bom_and_tolerates_non_utf8() {
        // UTF-8 with BOM
        let mut bom_data = vec![0xEF, 0xBB, 0xBF];
        bom_data.extend_from_slice(b"Hello from UTF-8 BOM file");
        let doc = extract_document(bom_data, "text/plain", "test.txt")
            .await
            .expect("should decode BOM text");
        assert_eq!(doc.text.as_deref(), Some("Hello from UTF-8 BOM file"));

        // Non-UTF8 byte stream (e.g. Windows-1252 smart quote 0x93, 0x94)
        let windows_1252 = vec![b'H', b'i', b' ', 0x93, b'Q', b'u', b'o', b't', b'e', 0x94];
        let doc2 = extract_document(windows_1252, "text/plain", "legacy.txt")
            .await
            .expect("should decode lossy non-utf8");
        assert!(doc2.text.expect("text present").contains("Hi "));
    }

    #[test]
    fn test_create_in_memory_zip() {
        let text = b"test code";
        let zip_bytes = create_in_memory_zip("hello.txt", text).expect("zip created");
        assert!(!zip_bytes.is_empty());
        // Verify it contains standard zip headers (PK..)
        assert_eq!(&zip_bytes[0..4], &[0x50, 0x4B, 0x03, 0x04]);
    }

    #[test]
    fn test_create_in_memory_multi_file_zip() {
        use std::io::Read;

        let entries = vec![
            ArchiveFileEntry {
                filename: "script.sh".to_string(),
                content: "#!/bin/bash\necho hello".to_string(),
            },
            ArchiveFileEntry {
                filename: "subdir/config.json".to_string(),
                content: "{\"key\": \"val\"}".to_string(),
            },
            ArchiveFileEntry {
                filename: "../../../etc/passwd".to_string(),
                content: "fake".to_string(),
            },
        ];

        let zip_bytes =
            create_in_memory_multi_file_zip(&entries).expect("multi-file zip created successfully");
        assert_eq!(&zip_bytes[0..4], &[0x50, 0x4B, 0x03, 0x04]);

        let cursor = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("zip archive valid");
        assert_eq!(archive.len(), 3);

        {
            let mut file1 = archive.by_name("script.sh").expect("find script.sh");
            let mut c1 = String::new();
            file1.read_to_string(&mut c1).expect("read script.sh");
            assert_eq!(c1, "#!/bin/bash\necho hello");
        }

        // Traversal path "../../../etc/passwd" sanitized to "etc_passwd"
        assert!(archive.by_name("etc_passwd").is_ok());
    }

    #[test]
    fn test_create_in_memory_pdf() {
        let title = "Laporan Uji";
        let content = "Paragraf 1 dari dokumen uji PDF.\nBaris kedua yang cukup panjang untuk memastikan word wrapping dan stream content bekerja dengan benar.";
        let pdf_bytes = create_in_memory_pdf(title, content).expect("pdf created");
        assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
        assert!(pdf_bytes.ends_with(b"%%EOF\n") || pdf_bytes.ends_with(b"%%EOF"));
        let doc = lopdf::Document::load_mem(&pdf_bytes).expect("lopdf parses generated pdf");
        assert_eq!(doc.get_pages().len(), 1);
    }

    #[test]
    fn test_detect_mime_from_filename() {
        assert_eq!(detect_mime_from_filename("code.py"), "text/plain");
        assert_eq!(detect_mime_from_filename("data.csv"), "text/csv");
        assert_eq!(detect_mime_from_filename("doc.pdf"), "application/pdf");
        assert_eq!(detect_mime_from_filename("archive.zip"), "application/zip");
        assert_eq!(detect_mime_from_filename("vector.svg"), "image/svg+xml");
        assert_eq!(
            detect_mime_from_filename("unknown.xyz"),
            "application/octet-stream"
        );
    }
}
