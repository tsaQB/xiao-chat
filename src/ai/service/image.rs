use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::watch;

use crate::util::truncate_chars;

use super::provider_url;
use super::read_bounded_json;
use super::read_bounded_response_bytes;
use super::timeout_from_env;
use super::AIChatService;
use crate::ai::routing::{GenerationModelSnapshot, ModelRole};

pub(crate) const MAX_GENERATED_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub(crate) const IMAGE_PROVIDER_CONNECT_TIMEOUT_ENV: &str = "IMAGE_PROVIDER_CONNECT_TIMEOUT_SECS";
pub(crate) const IMAGE_GENERATION_TIMEOUT_ENV: &str = "IMAGE_GENERATION_TIMEOUT_SECS";
pub(crate) const IMAGE_DOWNLOAD_TIMEOUT_ENV: &str = "IMAGE_DOWNLOAD_TIMEOUT_SECS";

pub(crate) fn validate_generated_image_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("generated image response was empty".to_string());
    }
    if bytes.len() > MAX_GENERATED_IMAGE_BYTES {
        return Err("generated image exceeded XiaoAI byte limits".to_string());
    }
    let supported = bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
        || (bytes.len() >= 8 && &bytes[4..8] == b"ftyp");
    if !supported {
        return Err("generated image response has an unsupported file signature".to_string());
    }
    Ok(())
}

pub fn decode_generated_image_base64(encoded: &str) -> Result<Vec<u8>, ImageGenerationError> {
    use base64::Engine;
    let max_encoded_len = MAX_GENERATED_IMAGE_BYTES
        .saturating_mul(4)
        .div_ceil(3)
        .saturating_add(8);
    if encoded.len() > max_encoded_len {
        return Err(ImageGenerationError::new(
            ImageGenerationErrorKind::InvalidImage,
            "Provider mengembalikan base64 gambar melebihi batas Xiao.",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            ImageGenerationError::new(
                ImageGenerationErrorKind::InvalidBase64,
                "Provider mengembalikan base64 gambar yang rusak.",
            )
        })?;
    validate_generated_image_bytes(&bytes).map_err(|error| {
        ImageGenerationError::new(ImageGenerationErrorKind::InvalidImage, error)
    })?;
    Ok(bytes)
}

#[cfg(test)]
pub(crate) fn parse_generated_image_url(url: &str) -> Result<url::Url, ImageGenerationError> {
    let parsed = url::Url::parse(url).map_err(|_| {
        ImageGenerationError::new(
            ImageGenerationErrorKind::UnsafeImageUrl,
            "provider returned an invalid image URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ImageGenerationError::new(
            ImageGenerationErrorKind::UnsafeImageUrl,
            "provider image URL must use http or https with a host",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ImageGenerationError::new(
            ImageGenerationErrorKind::UnsafeImageUrl,
            "provider image URL must not contain embedded credentials",
        ));
    }
    Ok(parsed)
}

pub(crate) fn timeout_image_error(label: &str, timeout: Duration) -> ImageGenerationError {
    ImageGenerationError::new(
        ImageGenerationErrorKind::Timeout,
        format!("{label} melewati batas waktu {} detik.", timeout.as_secs()),
    )
}

pub async fn download_generated_image(url: &str) -> Result<Vec<u8>, ImageGenerationError> {
    let resolved = crate::bot::url_policy::resolve_download_url(url)
        .await
        .map_err(|err| ImageGenerationError::new(ImageGenerationErrorKind::UnsafeImageUrl, err))?;

    let client = reqwest::Client::builder()
        .connect_timeout(timeout_from_env(IMAGE_PROVIDER_CONNECT_TIMEOUT_ENV, 10))
        .timeout(timeout_from_env(IMAGE_DOWNLOAD_TIMEOUT_ENV, 30))
        // Redirect targets cannot be revalidated against the DNS/IP policy
        // above without a custom resolver. Disable redirects to prevent a
        // trusted public URL from pivoting into a private network address.
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve(&resolved.host, resolved.address)
        .build()
        .map_err(|_| {
            ImageGenerationError::new(
                ImageGenerationErrorKind::Provider,
                "failed to build bounded image downloader",
            )
        })?;
    let response = client.get(resolved.url).send().await.map_err(|error| {
        if error.is_timeout() {
            ImageGenerationError::new(
                ImageGenerationErrorKind::DownloadTimeout,
                "provider image download timed out",
            )
        } else {
            ImageGenerationError::new(
                ImageGenerationErrorKind::Provider,
                "provider image download failed",
            )
        }
    })?;
    if !response.status().is_success() {
        return Err(ImageGenerationError::new(
            ImageGenerationErrorKind::HttpStatus,
            format!(
                "provider image download returned status {}",
                response.status().as_u16()
            ),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_GENERATED_IMAGE_BYTES as u64)
    {
        return Err(ImageGenerationError::new(
            ImageGenerationErrorKind::InvalidImage,
            "provider image exceeded XiaoAI byte limits",
        ));
    }
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("image/"))
    {
        return Err(ImageGenerationError::new(
            ImageGenerationErrorKind::InvalidImage,
            "provider image URL did not return an image content type",
        ));
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|_| {
            ImageGenerationError::new(
                ImageGenerationErrorKind::Provider,
                "provider image stream failed",
            )
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_GENERATED_IMAGE_BYTES {
            return Err(ImageGenerationError::new(
                ImageGenerationErrorKind::InvalidImage,
                "provider image exceeded XiaoAI byte limits",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    validate_generated_image_bytes(&bytes).map_err(|error| {
        ImageGenerationError::new(ImageGenerationErrorKind::InvalidImage, error)
    })?;
    Ok(bytes)
}

pub(crate) fn external_image_fallback_enabled(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("pollinations")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageGenerationProtocol {
    OpenAiImages,
}

impl ImageGenerationProtocol {
    pub(crate) fn endpoint(self, base: &str) -> String {
        match self {
            Self::OpenAiImages => provider_url(base, "images/generations"),
        }
    }

    pub(crate) fn payload(self, model: &str, prompt: &str, width: usize, height: usize) -> Value {
        match self {
            Self::OpenAiImages => json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "size": format!("{width}x{height}"),
                "response_format": "b64_json"
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageGenerationErrorKind {
    CapabilityUnknown,
    CapabilityUnsupported,
    RouteDisabled,
    ProviderNotFound,
    ModelNotFound,
    Timeout,
    Auth,
    RateLimited,
    HttpStatus,
    ProtocolMismatch,
    InvalidResponse,
    InvalidBase64,
    InvalidImage,
    UnsafeImageUrl,
    DownloadTimeout,
    Cancelled,
    Provider,
}

#[derive(Debug, Clone)]
pub struct ImageGenerationError {
    pub kind: ImageGenerationErrorKind,
    pub message: String,
}

impl ImageGenerationError {
    pub(crate) fn new(kind: ImageGenerationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

pub(crate) fn classify_image_route_error(message: &str) -> ImageGenerationErrorKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("disabled") {
        ImageGenerationErrorKind::RouteDisabled
    } else if lower.contains("provider") && lower.contains("not found") {
        ImageGenerationErrorKind::ProviderNotFound
    } else if lower.contains("model")
        && (lower.contains("not found") || lower.contains("no longer present"))
    {
        ImageGenerationErrorKind::ModelNotFound
    } else if lower.contains("unsupported") {
        ImageGenerationErrorKind::CapabilityUnsupported
    } else {
        ImageGenerationErrorKind::CapabilityUnknown
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub provider_name: String,
    pub model: String,
    pub used_external_fallback: bool,
    pub primary_failure: Option<String>,
}

impl AIChatService {
    pub(crate) async fn generate_image_with_snapshot(
        &self,
        _user_id: i64,
        prompt: &str,
        width: usize,
        height: usize,
        snapshot: &GenerationModelSnapshot,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> Result<GeneratedImage, ImageGenerationError> {
        if *cancel_rx.borrow() {
            return Err(ImageGenerationError::new(
                ImageGenerationErrorKind::Cancelled,
                "Pembuatan gambar dibatalkan.",
            ));
        }
        let clean_prompt = prompt.trim();
        let route = Self::resolve_model_route_from_snapshot(snapshot, ModelRole::ImageGeneration)
            .map_err(|error| {
            ImageGenerationError::new(classify_image_route_error(&error), error)
        })?;

        let generation_timeout = timeout_from_env(IMAGE_GENERATION_TIMEOUT_ENV, 120);
        let protocol = ImageGenerationProtocol::OpenAiImages;
        let gen_url = protocol.endpoint(&route.provider.endpoint);
        let payload = protocol.payload(&route.model, clean_prompt, width, height);
        let mut req = self
            .client
            .post(&gen_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .timeout(generation_timeout);

        if !route.provider.api_key.is_empty()
            && !["none", "-", "no"]
                .iter()
                .any(|key| route.provider.api_key.eq_ignore_ascii_case(key))
        {
            req = req.header(
                "Authorization",
                format!("Bearer {}", route.provider.api_key),
            );
        }

        let provider_result = tokio::select! {
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    return Err(ImageGenerationError::new(
                        ImageGenerationErrorKind::Cancelled,
                        "Pembuatan gambar dibatalkan.",
                    ));
                }
                Err(ImageGenerationError::new(
                    ImageGenerationErrorKind::Provider,
                    "Kanal pembatalan image generation ditutup.",
                ))
            }
            response = req.send() => {
                match response {
                    Err(error) if error.is_timeout() => Err(timeout_image_error(
                        "Image Generation Model",
                        generation_timeout,
                    )),
                    Err(error) => Err(ImageGenerationError::new(
                        ImageGenerationErrorKind::Provider,
                        format!("Koneksi ke Image Generation Model gagal: {}", error.without_url()),
                    )),
                    Ok(response) if !response.status().is_success() => {
                        let status = response.status();
                        let detail = read_bounded_response_bytes(response, 64 * 1024)
                            .await
                            .ok()
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                            .unwrap_or_default();
                        let kind = match status.as_u16() {
                            401 | 403 => ImageGenerationErrorKind::Auth,
                            429 => ImageGenerationErrorKind::RateLimited,
                            404 | 405 => ImageGenerationErrorKind::ProtocolMismatch,
                            400 if detail.to_ascii_lowercase().contains("not supported")
                                || detail.to_ascii_lowercase().contains("not an image model")
                                || detail.to_ascii_lowercase().contains("unsupported") => {
                                ImageGenerationErrorKind::ProtocolMismatch
                            }
                            _ => ImageGenerationErrorKind::HttpStatus,
                        };
                        Err(ImageGenerationError::new(
                            kind,
                            format!(
                                "Image Generation Model mengembalikan HTTP {}. Periksa konfigurasi endpoint/model dan batas provider.",
                                status.as_u16()
                            ),
                        ))
                    }
                    Ok(response) => {
                        let body = read_bounded_json(response).await.map_err(|error| {
                            ImageGenerationError::new(
                                ImageGenerationErrorKind::InvalidResponse,
                                format!("Respons image generation tidak valid: {error}"),
                            )
                        })?;
                        let data = body
                            .get("data")
                            .and_then(|value| value.get(0))
                            .ok_or_else(|| {
                                ImageGenerationError::new(
                                    ImageGenerationErrorKind::InvalidResponse,
                                    "Respons image generation tidak memiliki data gambar.",
                                )
                            })?;

                        let bytes = if let Some(encoded) =
                            data.get("b64_json").and_then(|value| value.as_str())
                        {
                            decode_generated_image_base64(encoded)?
                        } else if let Some(url) = data.get("url").and_then(|value| value.as_str()) {
                            download_generated_image(url).await?
                        } else {
                            return Err(ImageGenerationError::new(
                                ImageGenerationErrorKind::InvalidResponse,
                                "Provider tidak mengembalikan b64_json atau URL gambar.",
                            ));
                        };

                        Ok(GeneratedImage {
                            bytes,
                            provider_name: route.provider.name.clone(),
                            model: route.model.clone(),
                            used_external_fallback: false,
                            primary_failure: None,
                        })
                    }
                }
            }
        };

        let primary_failure = match provider_result {
            Ok(image) => return Ok(image),
            Err(error) if error.kind == ImageGenerationErrorKind::Cancelled => return Err(error),
            Err(error) if error.kind == ImageGenerationErrorKind::Timeout => return Err(error),
            Err(provider_error) => {
                let fallback =
                    std::env::var("IMAGE_FALLBACK_PROVIDER").unwrap_or_else(|_| "none".to_string());
                if !external_image_fallback_enabled(&fallback) {
                    return Err(provider_error);
                }
                truncate_chars(&provider_error.message, 240)
            }
        };

        let encoded_prompt = urlencoding::encode(clean_prompt);
        let fallback_model = "flux";
        let poll_url = format!(
            "https://image.pollinations.ai/prompt/{}?width={}&height={}&model={}&nologo=true&enhance=true",
            encoded_prompt, width, height, fallback_model
        );
        let fallback_timeout = timeout_from_env(IMAGE_GENERATION_TIMEOUT_ENV, 120);
        let request = self.client.get(&poll_url).timeout(fallback_timeout);
        let response = tokio::select! {
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    return Err(ImageGenerationError::new(
                        ImageGenerationErrorKind::Cancelled,
                        "Pembuatan gambar dibatalkan.",
                    ));
                }
                return Err(ImageGenerationError::new(
                    ImageGenerationErrorKind::Provider,
                    "Kanal pembatalan image generation ditutup.",
                ));
            }
            response = request.send() => response
        }
        .map_err(|error| {
            if error.is_timeout() {
                ImageGenerationError::new(
                    ImageGenerationErrorKind::Timeout,
                    format!(
                        "Fallback image generation melewati batas waktu {} detik.",
                        fallback_timeout.as_secs()
                    ),
                )
            } else {
                ImageGenerationError::new(
                    ImageGenerationErrorKind::Provider,
                    format!("Fallback image generation gagal: {error}"),
                )
            }
        })?;

        if !response.status().is_success() {
            return Err(ImageGenerationError::new(
                ImageGenerationErrorKind::Provider,
                format!(
                    "Fallback image generation mengembalikan HTTP {}.",
                    response.status().as_u16()
                ),
            ));
        }
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("image/"))
        {
            return Err(ImageGenerationError::new(
                ImageGenerationErrorKind::InvalidResponse,
                "Fallback tidak mengembalikan content-type gambar.",
            ));
        }
        let bytes = read_bounded_response_bytes(response, MAX_GENERATED_IMAGE_BYTES)
            .await
            .map_err(|error| {
                ImageGenerationError::new(ImageGenerationErrorKind::InvalidResponse, error)
            })?;
        validate_generated_image_bytes(&bytes).map_err(|error| {
            ImageGenerationError::new(ImageGenerationErrorKind::InvalidImage, error)
        })?;

        Ok(GeneratedImage {
            bytes,
            provider_name: "Pollinations fallback".to_string(),
            model: fallback_model.to_string(),
            used_external_fallback: true,
            primary_failure: Some(primary_failure),
        })
    }
}
