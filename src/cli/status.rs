use std::env;

use crate::ai::service::{
    load_provider_store, CapabilityKind, CapabilityState, ModelRole, ModelRoute, ProviderConfig,
};
use crate::ai::AIChatService;
use crate::bot::client::TelegramBotClient;
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
    println!("\n\x1b[1;36mxiao Status\x1b[0m\n");

    let token = env::var("BOT_TOKEN")
        .ok()
        .or_else(|| crate::ai::service::load_app_setting("BOT_TOKEN"))
        .unwrap_or_default();
    let owner_id = get_configured_owner_id();

    // 1. Gateway Status
    if token.is_empty() || token == "YOUR_TELEGRAM_BOT_TOKEN_HERE" {
        println!("  Gateway      ○ Telegram: Belum dikonfigurasi (Jalankan 'xiao gateway')");
    } else {
        let bot = TelegramBotClient::new(&token);
        match bot.get_me().await {
            Ok(resp) if resp.ok => {
                if let Some(info) = resp.result {
                    let uname = info.username.unwrap_or_else(|| "Unknown".to_string());
                    let owner_str = owner_id
                        .map(|id| format!(" · Owner: {id}"))
                        .unwrap_or_default();
                    println!("  Gateway      ● Telegram (@{uname}{owner_str})");
                } else {
                    println!("  Gateway      ✖ Telegram: Tidak ada info bot");
                }
            }
            Ok(_) | Err(_) => {
                println!("  Gateway      ✖ Telegram: Token tidak valid / Error koneksi");
            }
        }
    }

    // 2. Provider & Model Status (evidence-based)
    let store = load_provider_store();
    let active_p = if let Some(ref aid) = store.active_id {
        store.providers.iter().find(|p| &p.id == aid).cloned()
    } else {
        store.providers.first().cloned()
    };

    if let Some(p) = active_p {
        let (ok, res) = ai_service
            .fetch_models_from_endpoint(&p.endpoint, &p.api_key)
            .await;
        let provider_health = if ok {
            format!(
                "\x1b[32mHealthy\x1b[0m ({} models available)",
                res.map(|m| m.len()).unwrap_or(p.models.len())
            )
        } else {
            let err = res.err().unwrap_or_else(|| "unreachable".to_string());
            format!("\x1b[31mUnhealthy\x1b[0m ({err})")
        };

        println!("  Provider     ● {} — {}", p.name, provider_health);
        println!(
            "  Main Model   ◆ {} ({} configured models)",
            p.active_model,
            p.models.len()
        );

        let cap_record = ai_service
            .capability_record(&p.endpoint, &p.active_model)
            .await;
        println!("\n  \x1b[1;37mModel Capabilities (Evidence-Based):\x1b[0m");

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
    } else {
        println!("  Provider     ○ Belum ada AI Provider (Jalankan 'xiao provider')");
    }

    // 3. Addon Routing
    println!("\nAddon Routes:");
    let providers = ai_service.get_user_providers(0).await;
    let routing = ai_service.model_routing_config().await;
    for role in ModelRole::addon_roles() {
        let route = routing
            .route(role)
            .cloned()
            .unwrap_or(ModelRoute::MainModel);
        let route_text = addon_route_text(&route, &providers);
        let health = match ai_service.resolve_model_route(role).await {
            Ok(_) => "\x1b[32mavailable\x1b[0m",
            Err(_) => "\x1b[38;5;244munavailable\x1b[0m",
        };
        println!("  {:<12} → {} ({health})", role.display_name(), route_text);
    }

    // 4. Web Search & Tools Status
    println!("\nWeb Search & Tools:");
    let (search_engine_str, mcp_url) = crate::ai::tools::get_search_engine_status();
    println!("  Search Engine → {}", search_engine_str);
    println!("  MCP Hosted    → {}", mcp_url);
    println!("  Fetch Engine  → \x1b[32mEnabled\x1b[0m (Auto Link Reader & Extract)");
    println!();
}
