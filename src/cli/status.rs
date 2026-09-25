use std::env;

use crate::ai::service::{
    load_provider_store, CapabilityKind, CapabilityState, ModelRole, ModelRoute, ProviderConfig,
};
use crate::ai::AIChatService;
use crate::bot::client::TelegramBotClient;
use crate::cli::tui::{get_terminal_bar_width, print_mini_header, render_hud_box};
use crate::{get_configured_owner_id, load_environment};

pub(crate) fn addon_route_text(route: &ModelRoute, providers: &[ProviderConfig]) -> String {
    match route {
        ModelRoute::MainModel => "Main Model".to_string(),
        ModelRoute::Disabled => "Disabled".to_string(),
        ModelRoute::Specific { provider_id, model } => {
            let provider = providers
                .iter()
                .find(|p| &p.id == provider_id)
                .map(|p| p.name.as_str())
                .unwrap_or(provider_id.as_str());
            format!("{} :: {}", provider, model)
        }
    }
}

pub(crate) async fn run_cli_status(ai_service: &AIChatService) {
    load_environment();
    let bar_width = get_terminal_bar_width();
    print_mini_header("System Health & Telemetry Status");

    let token = env::var("BOT_TOKEN")
        .ok()
        .or_else(|| crate::ai::service::load_app_setting("BOT_TOKEN"))
        .unwrap_or_default();
    let owner_id = get_configured_owner_id();

    // 1. Gateway Status
    let gateway_str = if token.is_empty() || token == "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
        "○ Telegram: Not configured".to_string()
    } else {
        let bot = TelegramBotClient::new(&token);
        match bot.get_me().await {
            Ok(resp) if resp.ok => {
                if let Some(info) = resp.result {
                    let uname = info.username.unwrap_or_else(|| "Unknown".to_string());
                    let owner_str = owner_id
                        .map(|id| format!(" · Owner: {id}"))
                        .unwrap_or_default();
                    format!("● Telegram (@{uname}{owner_str})")
                } else {
                    "✖ Telegram: No bot info".to_string()
                }
            }
            Ok(_) | Err(_) => "✖ Telegram: Invalid token / Connection error".to_string(),
        }
    };

    // 2. Provider & Model Status (evidence-based)
    let store = load_provider_store();
    let active_p = if let Some(ref aid) = store.active_id {
        store.providers.iter().find(|p| &p.id == aid).cloned()
    } else {
        store.providers.first().cloned()
    };

    let (provider_str, model_str, cap_record) = if let Some(ref p) = active_p {
        let (ok, res) = ai_service
            .fetch_models_from_endpoint(&p.endpoint, &p.api_key)
            .await;
        let provider_health = if ok {
            format!(
                "Healthy ({} models available)",
                res.map(|m| m.len()).unwrap_or(p.models.len())
            )
        } else {
            let err = res.err().unwrap_or_else(|| "unreachable".to_string());
            format!("Unhealthy ({err})")
        };
        let p_str = format!("● {} — {}", p.name, provider_health);
        let m_str = format!(
            "◆ {} ({} configured models)",
            p.active_model,
            p.models.len()
        );
        let cap = ai_service
            .capability_record(&p.endpoint, &p.active_model)
            .await;
        (p_str, m_str, cap)
    } else {
        (
            "○ No active AI Provider".to_string(),
            "○ None".to_string(),
            None,
        )
    };

    // 3. Web Search & Tools Status
    let (search_engine_str, mcp_url) = crate::ai::tools::get_search_engine_status();

    let hud_rows = [
        ("GATEWAY", gateway_str.as_str()),
        ("MAIN PROVIDER", provider_str.as_str()),
        ("MAIN MODEL", model_str.as_str()),
        ("SEARCH PIPELINE", search_engine_str.as_str()),
    ];
    let hud = render_hud_box("SYSTEM INFRASTRUCTURE", &hud_rows, bar_width);
    println!("\n{hud}");

    // Tagged section 1: Model Capabilities
    if active_p.is_some() {
        println!("\n  \x1b[1;37m▸ MODEL CAPABILITIES (Evidence-Based)\x1b[0m");

        let text_chat_state = cap_record
            .as_ref()
            .map(|r| r.effective_state_for(CapabilityKind::TextChat))
            .unwrap_or(CapabilityState::Unknown);
        let vision_state = cap_record
            .as_ref()
            .map(|r| r.effective_state_for(CapabilityKind::ImageInput))
            .unwrap_or(CapabilityState::Unknown);
        let video_state = cap_record
            .as_ref()
            .map(|r| r.effective_state_for(CapabilityKind::VideoInput))
            .unwrap_or(CapabilityState::Unknown);
        let audio_state = cap_record
            .as_ref()
            .map(|r| r.effective_state_for(CapabilityKind::AudioInput))
            .unwrap_or(CapabilityState::Unknown);
        let tools_state = cap_record
            .as_ref()
            .map(|r| r.effective_state_for(CapabilityKind::Tools))
            .unwrap_or(CapabilityState::Unknown);

        let format_cap_line = |name: &str, state: CapabilityState, kind: CapabilityKind| {
            let src = cap_record
                .as_ref()
                .and_then(|r| r.effective_evidence_for(kind))
                .map(|e| match e.source {
                    crate::ai::storage::CapabilityEvidenceSource::ProviderMetadata => {
                        "provider metadata"
                    }
                    crate::ai::storage::CapabilityEvidenceSource::ActiveProbe => "active probe",
                    crate::ai::storage::CapabilityEvidenceSource::KnownProviderProfile => {
                        "provider profile"
                    }
                    crate::ai::storage::CapabilityEvidenceSource::UserOverride => "user override",
                })
                .unwrap_or("no evidence");
            match state {
                CapabilityState::Supported => {
                    format!(
                        "    • {:<16}: \x1b[32m✔ Supported\x1b[0m   \x1b[38;5;244m({src})\x1b[0m",
                        name
                    )
                }
                CapabilityState::Unsupported => {
                    format!(
                        "    • {:<16}: \x1b[31m✖ Unsupported\x1b[0m \x1b[38;5;244m({src})\x1b[0m",
                        name
                    )
                }
                CapabilityState::Unknown => {
                    format!(
                        "    • {:<16}: \x1b[38;5;244m○ Unknown       ({src})\x1b[0m",
                        name
                    )
                }
            }
        };

        println!(
            "{}",
            format_cap_line("Text Chat", text_chat_state, CapabilityKind::TextChat)
        );
        println!(
            "{}",
            format_cap_line("Vision (Image)", vision_state, CapabilityKind::ImageInput)
        );
        println!(
            "{}",
            format_cap_line("Video Frames", video_state, CapabilityKind::VideoInput)
        );
        println!(
            "{}",
            format_cap_line("Audio Input", audio_state, CapabilityKind::AudioInput)
        );
        println!(
            "{}",
            format_cap_line("Tools / JSON", tools_state, CapabilityKind::Tools)
        );
        if let Some(ctx) = cap_record.as_ref().and_then(|r| r.context_window) {
            println!(
                "    • {:<16}: \x1b[36m{} tokens\x1b[0m",
                "Context Limit", ctx
            );
        }
    }

    // Tagged section 2: Addon Routing
    println!("\n  \x1b[1;37m▸ MULTIMODAL SPECIALIST ROUTES\x1b[0m");
    let providers = ai_service.get_user_providers(0).await;
    let routing = ai_service.model_routing_config().await;
    for role in ModelRole::addon_roles() {
        let route = routing
            .route(role)
            .cloned()
            .unwrap_or(if role == ModelRole::ImageGeneration {
                ModelRoute::Disabled
            } else {
                ModelRoute::MainModel
            });
        let route_text = addon_route_text(&route, &providers);
        let health = match ai_service.resolve_model_route(role).await {
            Ok(_) => "\x1b[32mavailable\x1b[0m",
            Err(_) => "\x1b[38;5;244munavailable\x1b[0m",
        };
        println!(
            "    • {:<14} → {} ({health})",
            role.display_name(),
            route_text
        );
    }

    // Tagged section 3: Web Search & Tools Status
    println!("\n  \x1b[1;37m▸ WEB SEARCH & MCP PIPELINE\x1b[0m");
    println!("    • Search Engine  → {}", search_engine_str);
    println!("    • MCP Hosted     → {}", mcp_url);
    println!("    • Fetch Engine   → \x1b[32mEnabled\x1b[0m (Auto Link Reader & Extract)\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_addon_route_text() {
        let providers = vec![ProviderConfig {
            id: "p1".to_string(),
            name: "OpenAI".to_string(),
            endpoint: "https://api.openai.com".to_string(),
            api_key: "key".to_string(),
            api_key_ref: None,
            models: vec!["gpt-4o".to_string()],
            active_model: "gpt-4o".to_string(),
        }];

        assert_eq!(
            addon_route_text(&ModelRoute::MainModel, &providers),
            "Main Model"
        );
        assert_eq!(
            addon_route_text(&ModelRoute::Disabled, &providers),
            "Disabled"
        );
        assert_eq!(
            addon_route_text(
                &ModelRoute::Specific {
                    provider_id: "p1".to_string(),
                    model: "gpt-4o".to_string()
                },
                &providers
            ),
            "OpenAI :: gpt-4o"
        );
        assert_eq!(
            addon_route_text(
                &ModelRoute::Specific {
                    provider_id: "unknown_p".to_string(),
                    model: "custom-model".to_string()
                },
                &providers
            ),
            "unknown_p :: custom-model"
        );
    }
}
