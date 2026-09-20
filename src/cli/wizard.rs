use rand::Rng;
use std::io::{self, BufRead, IsTerminal, Write};

use crate::ai::service::{
    load_provider_store, save_provider_store, ModelRole, ModelRoute, ProviderConfig,
};
use crate::ai::AIChatService;
use crate::bot::client::TelegramBotClient;
use crate::cli::tui::terminal_interactive_select;
use crate::{
    get_configured_owner_id, get_configured_token, load_environment, save_env_kv, save_token_to_env,
};

pub(crate) async fn run_cli_quickstart_wizard(ai_service: &AIChatService) -> Option<String> {
    crate::cli::tui::print_mini_header("Quickstart Setup Wizard");
    println!("  \x1b[38;5;245mInitial configuration for AI Provider and Telegram Gateway · \x1b[1;37m[Ctrl+C]\x1b[0m \x1b[38;5;245mCancel\x1b[0m\n");

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    load_environment();

    let env_endpoint = std::env::var("AI_ENDPOINT")
        .ok()
        .and_then(|s| crate::ai::storage::parse_auto_seed_endpoint(&s));

    let env_api_key = std::env::var("AI_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let env_model = std::env::var("AI_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Step 1: AI Provider & Main Model
    println!("  \x1b[48;2;15;23;42m\x1b[38;2;6;182;212m STEP 1/2 \x1b[0m \x1b[1;37m◈ AI Provider & Main Model\x1b[0m\n");

    let provider_options = vec![
        "◆  OpenRouter               (Default · 300+ AI models, free & paid)".to_string(),
        "◈  Custom OpenAI Endpoint   (Ollama, vLLM, DeepSeek, Local / Proxy)".to_string(),
    ];

    let prov_idx = if std::io::stdout().is_terminal() {
        terminal_interactive_select(
            "Select AI Provider Type:",
            &provider_options,
            0,
            false,
            None,
        )
    } else {
        Some(0)
    };

    let Some(prov_choice) = prov_idx else {
        println!("\n  \x1b[38;5;244mSetup cancelled.\x1b[0m\n");
        return None;
    };

    let (endpoint, clean_alias) = if prov_choice == 0 {
        let ep = env_endpoint
            .unwrap_or_else(|| crate::ai::storage::DEFAULT_OPENROUTER_ENDPOINT.to_string());
        println!("  \x1b[38;2;6;182;212m●\x1b[0m \x1b[1;37mProvider :\x1b[0m OpenRouter \x1b[38;5;244m({ep})\x1b[0m");
        (ep, "OpenRouter".to_string())
    } else {
        let default_custom = env_endpoint.unwrap_or_else(|| "http://127.0.0.1:8317/v1".to_string());
        let ep = loop {
            print!(
                "  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mEndpoint URL\x1b[0m \x1b[38;5;244m[default: {default_custom}]:\x1b[0m "
            );
            let _ = io::stdout().flush();
            let mut input = String::new();
            if reader.read_line(&mut input).is_err() {
                println!("\n  \x1b[38;5;244mSetup cancelled.\x1b[0m\n");
                return None;
            }
            let trimmed = input.trim();
            if trimmed.is_empty() {
                break default_custom.clone();
            }
            match normalize_endpoint_url(trimmed) {
                Ok(normalized) => {
                    if normalized != trimmed {
                        println!(
                            "  \x1b[38;2;16;185;129m✔\x1b[0m \x1b[38;5;244mAdjusted endpoint:\x1b[0m \x1b[1;37m{normalized}\x1b[0m"
                        );
                    }
                    break normalized;
                }
                Err(err) => {
                    println!("  \x1b[31m✖ Error: {err}\x1b[0m");
                }
            }
        };

        print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mProvider Name\x1b[0m \x1b[38;5;244m(Enter for default):\x1b[0m ");
        let _ = io::stdout().flush();
        let mut alias_input = String::new();
        let _ = reader.read_line(&mut alias_input);
        let raw_alias = alias_input.trim();
        let alias = if raw_alias.is_empty() {
            if let Ok(u) = url::Url::parse(&ep) {
                u.host_str().unwrap_or("Custom Provider").to_string()
            } else {
                "Custom Provider".to_string()
            }
        } else {
            raw_alias.to_string()
        };
        (ep, alias)
    };

    if env_api_key.is_some() {
        print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(Enter to use from .env):\x1b[0m ");
    } else if prov_choice == 0 {
        print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(obtain at https://openrouter.ai/keys):\x1b[0m ");
    } else {
        print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(Enter if local / keyless):\x1b[0m ");
    }
    let _ = io::stdout().flush();
    let mut key_input = String::new();
    let _ = reader.read_line(&mut key_input);
    let trimmed_key = key_input.trim().trim_matches(['"', '\'', '`']);
    let mut api_key = if trimmed_key.is_empty() {
        env_api_key.clone().unwrap_or_else(|| "none".to_string())
    } else {
        trimmed_key.to_string()
    };
    if api_key.is_empty() {
        api_key = "none".to_string();
    }

    println!("  \x1b[38;5;244m○ Connecting to endpoint...\x1b[0m");
    let (ok, res) = ai_service
        .fetch_models_from_endpoint(&endpoint, &api_key)
        .await;
    if !ok {
        let err = res.err().unwrap_or_else(|| "Unknown error".to_string());
        println!("  \x1b[31m✖ Error: Failed to connect to provider ({err})\x1b[0m\n");
        return None;
    }

    let models = res.unwrap_or_else(|_| vec!["gpt-4o".to_string()]);
    println!(
        "  \x1b[38;2;16;185;129m●\x1b[0m \x1b[1;32mConnected!\x1b[0m \x1b[38;5;244mFound {} models.\x1b[0m",
        models.len()
    );

    let default_idx = env_model
        .as_ref()
        .and_then(|m| models.iter().position(|name| name == m))
        .unwrap_or(0);

    let selected_idx = terminal_interactive_select(
        "Select Main Model for This Provider:",
        &models,
        default_idx,
        true,
        None,
    );

    let active_model = if let Some(idx) = selected_idx {
        models[idx].clone()
    } else if let Some(env_m) = env_model.as_ref().filter(|m| models.contains(m)) {
        env_m.clone()
    } else {
        models
            .first()
            .cloned()
            .unwrap_or_else(|| "gpt-4o".to_string())
    };

    let random_suffix: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();
    let provider_id = format!("prov_{}", random_suffix.to_lowercase());

    let provider = ProviderConfig {
        id: provider_id.clone(),
        name: clean_alias.clone(),
        endpoint: endpoint.clone(),
        api_key: api_key.clone(),
        api_key_ref: None,
        models: models.clone(),
        active_model: active_model.clone(),
    };

    let mut store = load_provider_store();
    store.providers.push(provider.clone());
    store.active_id = Some(provider_id);
    if let Err(error) = save_provider_store(&store) {
        println!("  \x1b[31m✖ Error: Failed to save provider configuration: {error}\x1b[0m");
        return None;
    }
    if !ai_service.reload_provider_store().await {
        println!("  \x1b[31m✖ Error: Failed to reload provider in runtime memory.\x1b[0m");
        return None;
    }

    // Preserving existing routes: only initialize missing routes to Main Model (additive setup)
    let existing_routing = ai_service.model_routing_config().await;
    for role in ModelRole::addon_roles() {
        if existing_routing.route(role).is_none() {
            if let Err(err) = ai_service
                .set_model_route(role, ModelRoute::MainModel)
                .await
            {
                println!(
                    "  \x1b[31m✖ Error: Failed to initialize route {}: {err}\x1b[0m",
                    role.display_name()
                );
                return None;
            }
        }
    }

    println!(
        "  \x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37mMain Model set to       :\x1b[0m \x1b[1;38;2;6;182;212m{}\x1b[0m\n",
        active_model
    );

    // Step 2: Gateway Setup
    println!("  \x1b[48;2;15;23;42m\x1b[38;2;6;182;212m STEP 2/2 \x1b[0m \x1b[1;37m◉ Gateway Setup\x1b[0m\n");
    print!(
        "  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mConfigure Telegram Gateway now? [Y/n]:\x1b[0m "
    );
    let _ = io::stdout().flush();
    let mut gateway_ans = String::new();
    let _ = reader.read_line(&mut gateway_ans);

    let mut bot_username_opt = None;
    let mut final_token_opt = None;
    let owner_id_opt;

    if !gateway_ans.trim().eq_ignore_ascii_case("n") {
        println!("  \x1b[38;2;6;182;212m●\x1b[0m \x1b[1;37mGateway Target: Telegram Bot API 10.3\x1b[0m\n");

        let env_token = get_configured_token();
        let (final_token, bot_username) = loop {
            if env_token.is_some() {
                print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mTelegram Bot Token\x1b[0m \x1b[38;5;244m(Enter to use token from environment):\x1b[0m ");
            } else {
                print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mTelegram Bot Token:\x1b[0m ");
            }
            let _ = io::stdout().flush();
            let mut input = String::new();
            if reader.read_line(&mut input).is_err() {
                println!("\n  \x1b[38;5;244mSetup cancelled.\x1b[0m\n");
                return None;
            }
            let trimmed = input.trim();
            let user_token = if trimmed.is_empty() {
                if let Some(ref tok) = env_token {
                    tok.clone()
                } else {
                    println!("  \x1b[31m✖ Error: Token cannot be empty.\x1b[0m");
                    continue;
                }
            } else {
                trimmed.to_string()
            };

            let temp_bot = TelegramBotClient::new(&user_token);
            match temp_bot.get_me().await {
                Ok(resp) if resp.ok => {
                    let Some(bot_info) = resp.result else {
                        println!("  \x1b[31m✖ Error: Telegram did not return bot info.\x1b[0m");
                        continue;
                    };
                    let uname = bot_info.username.unwrap_or_else(|| "Unknown".to_string());
                    if let Err(e) = save_token_to_env(&user_token) {
                        println!("  \x1b[31m✖ Error: Failed to save token to storage: {e}\x1b[0m");
                        return None;
                    }
                    println!(
                        "  \x1b[38;2;16;185;129m●\x1b[0m \x1b[1;32mToken valid!\x1b[0m \x1b[38;5;244mConnected to\x1b[0m \x1b[1;37m@{}\x1b[0m \x1b[38;5;244m({})\x1b[0m",
                        uname, bot_info.first_name
                    );
                    break (user_token, uname);
                }
                Ok(resp) => {
                    let desc = resp
                        .description
                        .unwrap_or_else(|| "Invalid token".to_string());
                    println!("  \x1b[31m✖ Error: Invalid token ({desc})\x1b[0m");
                }
                Err(e) => {
                    println!("  \x1b[31m✖ Error: Failed to connect to Telegram API ({e})\x1b[0m");
                }
            }
        };

        let env_owner = get_configured_owner_id();
        let owner_user_id = loop {
            if let Some(oid) = env_owner {
                print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mOwner User ID\x1b[0m \x1b[38;5;244m[default: {oid}]:\x1b[0m ");
            } else {
                print!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mOwner User ID:\x1b[0m ");
            }
            let _ = io::stdout().flush();
            let mut input = String::new();
            if reader.read_line(&mut input).is_err() {
                println!("\n  \x1b[38;5;244mSetup cancelled.\x1b[0m\n");
                return None;
            }
            let trimmed = input.trim();
            if trimmed.is_empty() {
                if let Some(oid) = env_owner {
                    break oid;
                }
            }
            match trimmed.parse::<i64>() {
                Ok(value) if value > 0 => break value,
                _ => {
                    println!("  \x1b[31m✖ Error: Owner User ID must be a positive integer.\x1b[0m")
                }
            }
        };
        if let Err(e) = save_env_kv("OWNER_USER_ID", &owner_user_id.to_string()) {
            println!("  \x1b[31m✖ Error: Failed to save Owner ID: {e}\x1b[0m");
            return None;
        }
        println!(
            "  \x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37mOwner ID set to         :\x1b[0m \x1b[1;38;2;6;182;212m{}\x1b[0m\n",
            owner_user_id
        );

        bot_username_opt = Some(bot_username);
        final_token_opt = Some(final_token);
        owner_id_opt = Some(owner_user_id);
    } else {
        println!("  \x1b[38;5;244m○ Gateway configuration skipped.\x1b[0m\n");
        // Reuse existing gateway token if present
        if let Some(token) = get_configured_token() {
            final_token_opt = Some(token);
        }
        owner_id_opt = get_configured_owner_id();
    }

    let bar_width = crate::cli::tui::get_terminal_bar_width();
    let prov_val = format!(
        "\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;244m({})\x1b[0m",
        clean_alias, endpoint
    );
    let model_val = format!(
        "\x1b[38;2;6;182;212m●\x1b[0m \x1b[1;37m{}\x1b[0m",
        active_model
    );
    let gateway_val = if let (Some(uname), Some(oid)) = (bot_username_opt, owner_id_opt) {
        format!(
            "\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37mTelegram\x1b[0m \x1b[38;5;244m(@{} · Owner: {})\x1b[0m",
            uname, oid
        )
    } else if let Some(oid) = owner_id_opt {
        if final_token_opt.is_some() {
            format!(
                "\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37mTelegram\x1b[0m \x1b[38;5;244m(existing · Owner: {})\x1b[0m",
                oid
            )
        } else {
            "\x1b[38;5;244m○ Not configured (Use 'xiao gateway')\x1b[0m".to_string()
        }
    } else {
        "\x1b[38;5;244m○ Not configured (Use 'xiao gateway')\x1b[0m".to_string()
    };
    let addons_val =
        "\x1b[38;5;244m○ Follows addon configuration (configure via 'xiao ai addon')\x1b[0m";

    let summary_rows = [
        ("PROVIDER", prov_val.as_str()),
        ("MAIN AI", model_val.as_str()),
        ("GATEWAY", gateway_val.as_str()),
        ("ADDONS", addons_val),
    ];
    let hud_box = crate::cli::tui::render_hud_box("SETUP COMPLETED", &summary_rows, bar_width);
    println!("{hud_box}");
    println!("\n  \x1b[38;5;244mLaunch bot now with command:\x1b[0m");
    println!("    \x1b[1;37mxiao start\x1b[0m\n");

    final_token_opt
}

pub(crate) async fn get_or_prompt_token(ai_service: &AIChatService) -> Option<String> {
    if let Some(token) = get_configured_token() {
        if ai_service.has_configured_provider(0).await {
            return Some(token);
        }
    }
    println!("\n  \x1b[38;5;244m○ Gateway or AI Provider is not yet configured.\x1b[0m");
    println!("  \x1b[1;38;2;6;182;212m▸\x1b[0m \x1b[1;37mLaunching Setup Wizard...\x1b[0m\n");
    run_cli_quickstart_wizard(ai_service).await
}

pub(crate) fn normalize_endpoint_url(raw: &str) -> Result<String, String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '<' || c == '>');

    if trimmed.is_empty() {
        return Err("Endpoint URL cannot be empty".to_string());
    }

    let lower = trimmed.to_ascii_lowercase();

    // 1. Identify Scheme and Target Host/Path
    let (scheme, rest) = if lower.starts_with("https://") {
        ("https", &trimmed[8..])
    } else if lower.starts_with("https:/") || lower.starts_with("https//") {
        ("https", trimmed[7..].trim_start_matches('/'))
    } else if lower.starts_with("https:") {
        ("https", trimmed[6..].trim_start_matches(['/', ' ']))
    } else if lower.starts_with("https ") || lower.starts_with("https\t") {
        ("https", trimmed[5..].trim_start_matches(['/', ' ']))
    } else if lower.starts_with("http://") {
        ("http", &trimmed[7..])
    } else if lower.starts_with("http:/") || lower.starts_with("http//") {
        ("http", trimmed[6..].trim_start_matches('/'))
    } else if lower.starts_with("http:") {
        ("http", trimmed[5..].trim_start_matches(['/', ' ']))
    } else if lower.starts_with("http ") || lower.starts_with("http\t") {
        ("http", trimmed[4..].trim_start_matches(['/', ' ']))
    } else if lower.starts_with("https")
        && trimmed.len() > 5
        && trimmed[5..].matches('.').count() >= 2
    {
        // User typed e.g. "httpscpa.oxygen.web.id/v1" without separator
        ("https", &trimmed[5..])
    } else {
        // No explicit scheme. Infer based on host.
        let host_candidate = trimmed.split('/').next().unwrap_or("").to_ascii_lowercase();
        let host_clean = host_candidate.split(':').next().unwrap_or("");
        let is_local = host_clean == "localhost"
            || host_clean == "127.0.0.1"
            || host_clean == "0.0.0.0"
            || host_clean == "::1"
            || host_clean == "[::1]"
            || host_clean.is_empty();

        let s = if is_local { "http" } else { "https" };
        (s, trimmed)
    };

    let rest = rest.trim().trim_start_matches('/');
    if rest.is_empty() {
        return Err("Host/Domain endpoint not found".to_string());
    }

    let candidate = format!("{scheme}://{rest}");
    let parsed = url::Url::parse(&candidate).map_err(|e| format!("Invalid URL: {e}"))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| "Invalid host/domain endpoint".to_string())?;

    if host.is_empty() {
        return Err("Invalid host/domain endpoint".to_string());
    }

    let port_str = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    let raw_path = parsed.path().trim_end_matches('/');

    let final_path = if raw_path.is_empty() || raw_path == "/" {
        "/v1".to_string()
    } else {
        raw_path.to_string()
    };

    Ok(format!("{scheme}://{host}{port_str}{final_path}"))
}
