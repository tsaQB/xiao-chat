use rand::Rng;
use std::io::{self, BufRead, Write};

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
    println!("\n\x1b[1;36mxiao Setup Wizard\x1b[0m");
    println!(
        "\x1b[38;5;244mKonfigurasi awal AI Provider dan Gateway. Tekan Ctrl+C untuk batal.\x1b[0m"
    );
    println!("\x1b[38;5;238m────────────────────────────────────────────────────────────\x1b[0m");

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

    let default_endpoint =
        env_endpoint.unwrap_or_else(|| crate::ai::storage::DEFAULT_OPENROUTER_ENDPOINT.to_string());

    // Step 1: AI Provider & Main Model
    println!("\n\x1b[1;37m[1/2] AI Provider & Main Model\x1b[0m");
    let endpoint = loop {
        print!(
            "  \x1b[1;37mEndpoint URL\x1b[0m \x1b[38;5;244m[default: {default_endpoint}]:\x1b[0m "
        );
        let _ = io::stdout().flush();
        let mut input = String::new();
        if reader.read_line(&mut input).is_err() {
            println!("\n\x1b[38;5;244mSetup dibatalkan.\x1b[0m");
            return None;
        }
        let trimmed = input.trim();
        let clean = if trimmed.is_empty() {
            default_endpoint.clone()
        } else {
            trimmed.trim_end_matches('/').to_string()
        };
        if clean.starts_with("http://") || clean.starts_with("https://") {
            break clean;
        }
        println!("  \x1b[31m✖ Error: Endpoint harus diawali dengan http:// atau https://\x1b[0m");
    };

    if env_api_key.is_some() {
        print!("  \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(Enter untuk memakai dari .env):\x1b[0m ");
    } else if endpoint == crate::ai::storage::DEFAULT_OPENROUTER_ENDPOINT
        || endpoint.contains("openrouter.ai")
    {
        print!("  \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(dapatkan di https://openrouter.ai/keys):\x1b[0m ");
    } else {
        print!("  \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(Enter jika lokal / tanpa key):\x1b[0m ");
    }
    let _ = io::stdout().flush();
    let mut key_input = String::new();
    let _ = reader.read_line(&mut key_input);
    let trimmed_key = key_input.trim();
    let mut api_key = if trimmed_key.is_empty() {
        env_api_key.clone().unwrap_or_else(|| "none".to_string())
    } else {
        trimmed_key.to_string()
    };
    if api_key.is_empty() {
        api_key = "none".to_string();
    }

    print!("  \x1b[1;37mProvider Name\x1b[0m \x1b[38;5;244m(Enter untuk default):\x1b[0m ");
    let _ = io::stdout().flush();
    let mut alias_input = String::new();
    let _ = reader.read_line(&mut alias_input);
    let raw_alias = alias_input.trim();
    let clean_alias = if raw_alias.is_empty() {
        if endpoint == crate::ai::storage::DEFAULT_OPENROUTER_ENDPOINT
            || endpoint.contains("openrouter.ai")
        {
            "OpenRouter".to_string()
        } else if let Ok(u) = url::Url::parse(&endpoint) {
            u.host_str().unwrap_or("Custom Provider").to_string()
        } else {
            "Custom Provider".to_string()
        }
    } else {
        raw_alias.to_string()
    };

    println!("  \x1b[38;5;244mMenghubungkan ke endpoint...\x1b[0m");
    let (ok, res) = ai_service
        .fetch_models_from_endpoint(&endpoint, &api_key)
        .await;
    if !ok {
        let err = res.err().unwrap_or_else(|| "Unknown error".to_string());
        println!("  \x1b[31m✖ Error: Gagal terhubung ke provider ({err})\x1b[0m");
        return None;
    }

    let models = res.unwrap_or_else(|_| vec!["gpt-4o".to_string()]);
    println!(
        "  \x1b[1;32m✔ Terhubung! Ditemukan {} model.\x1b[0m",
        models.len()
    );

    let default_idx = env_model
        .as_ref()
        .and_then(|m| models.iter().position(|name| name == m))
        .unwrap_or(0);

    let selected_idx = terminal_interactive_select(
        "Pilih Main Model untuk Provider Ini:",
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
        println!("  \x1b[31m✖ Error: Konfigurasi provider gagal disimpan: {error}\x1b[0m");
        return None;
    }
    if !ai_service.reload_provider_store().await {
        println!("  \x1b[31m✖ Error: Gagal memuat ulang provider di memori runtime.\x1b[0m");
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
                    "  \x1b[31m✖ Error: Gagal menginisialisasi route {}: {err}\x1b[0m",
                    role.display_name()
                );
                return None;
            }
        }
    }

    println!(
        "  \x1b[1;32m✔ Main Model diset ke     : {}\x1b[0m",
        active_model
    );

    // Step 2: Gateway Setup
    println!("\n\x1b[1;37m[2/2] Gateway Setup\x1b[0m");
    print!("  \x1b[1;37mKonfigurasi Gateway sekarang? [Y/n]:\x1b[0m ");
    let _ = io::stdout().flush();
    let mut gateway_ans = String::new();
    let _ = reader.read_line(&mut gateway_ans);

    let mut bot_username_opt = None;
    let mut final_token_opt = None;
    let owner_id_opt;

    if !gateway_ans.trim().eq_ignore_ascii_case("n") {
        println!("\n  \x1b[1;36mPilih Gateway:\x1b[0m");
        println!("    \x1b[1;32m❯ • Telegram\x1b[0m\n");

        let env_token = get_configured_token();
        let (final_token, bot_username) = loop {
            if env_token.is_some() {
                print!("  \x1b[1;37mTelegram Bot Token\x1b[0m \x1b[38;5;244m(Enter untuk memakai token dari environment):\x1b[0m ");
            } else {
                print!("  \x1b[1;37mTelegram Bot Token:\x1b[0m ");
            }
            let _ = io::stdout().flush();
            let mut input = String::new();
            if reader.read_line(&mut input).is_err() {
                println!("\n\x1b[38;5;244mSetup dibatalkan.\x1b[0m");
                return None;
            }
            let trimmed = input.trim();
            let user_token = if trimmed.is_empty() {
                if let Some(ref tok) = env_token {
                    tok.clone()
                } else {
                    println!("  \x1b[31m✖ Error: Token tidak boleh kosong.\x1b[0m");
                    continue;
                }
            } else {
                trimmed.to_string()
            };

            let temp_bot = TelegramBotClient::new(&user_token);
            match temp_bot.get_me().await {
                Ok(resp) if resp.ok => {
                    let Some(bot_info) = resp.result else {
                        println!(
                            "  \x1b[31m✖ Error: Telegram tidak mengembalikan info bot.\x1b[0m"
                        );
                        continue;
                    };
                    let uname = bot_info.username.unwrap_or_else(|| "Unknown".to_string());
                    if let Err(e) = save_token_to_env(&user_token) {
                        println!("  \x1b[31m✖ Error: Gagal menyimpan token ke storage: {e}\x1b[0m");
                        return None;
                    }
                    println!(
                        "  \x1b[1;32m✔ Token valid! Terhubung ke @{} ({})\x1b[0m",
                        uname, bot_info.first_name
                    );
                    break (user_token, uname);
                }
                Ok(resp) => {
                    let desc = resp
                        .description
                        .unwrap_or_else(|| "Invalid token".to_string());
                    println!("  \x1b[31m✖ Error: Token tidak valid ({desc})\x1b[0m");
                }
                Err(e) => {
                    println!("  \x1b[31m✖ Error: Gagal terhubung ke Telegram API ({e})\x1b[0m");
                }
            }
        };

        let env_owner = get_configured_owner_id();
        let owner_user_id = loop {
            if let Some(oid) = env_owner {
                print!("  \x1b[1;37mOwner User ID\x1b[0m \x1b[38;5;244m[default: {oid}]:\x1b[0m ");
            } else {
                print!("  \x1b[1;37mOwner User ID:\x1b[0m ");
            }
            let _ = io::stdout().flush();
            let mut input = String::new();
            if reader.read_line(&mut input).is_err() {
                println!("\n\x1b[38;5;244mSetup dibatalkan.\x1b[0m");
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
                    println!("  \x1b[31m✖ Error: Owner User ID harus berupa angka positif.\x1b[0m")
                }
            }
        };
        if let Err(e) = save_env_kv("OWNER_USER_ID", &owner_user_id.to_string()) {
            println!("  \x1b[31m✖ Error: Gagal menyimpan Owner ID: {e}\x1b[0m");
            return None;
        }
        println!("  \x1b[1;32m✔ Owner ID diset ke: {}\x1b[0m", owner_user_id);

        bot_username_opt = Some(bot_username);
        final_token_opt = Some(final_token);
        owner_id_opt = Some(owner_user_id);
    } else {
        println!("  \x1b[38;5;244m○ Konfigurasi gateway dilewati.\x1b[0m");
        // Reuse existing gateway token if present
        if let Some(token) = get_configured_token() {
            final_token_opt = Some(token);
        }
        owner_id_opt = get_configured_owner_id();
    }

    println!("\n\x1b[38;5;238m────────────────────────────────────────────────────────────\x1b[0m");
    println!("\x1b[1;32mSetup Selesai!\x1b[0m\n");
    println!("  Provider   ● {} ({})", clean_alias, endpoint);
    println!("  Model      ◆ {}", active_model);
    if let (Some(uname), Some(oid)) = (bot_username_opt, owner_id_opt) {
        println!("  Gateway    ● Telegram (@{} · Owner: {})", uname, oid);
    } else if let Some(oid) = owner_id_opt {
        if final_token_opt.is_some() {
            println!("  Gateway    ● Telegram (existing · Owner: {})", oid);
        } else {
            println!("  Gateway    ○ Belum dikonfigurasi (Gunakan 'xiao gateway')");
        }
    } else {
        println!("  Gateway    ○ Belum dikonfigurasi (Gunakan 'xiao gateway')");
    }
    println!("  Addons     ○ Mengikuti konfigurasi addon (atur via 'xiao addon')");
    println!("\n\x1b[1;36mJalankan bot sekarang dengan perintah:\x1b[0m");
    println!("  \x1b[1;37mxiao start\x1b[0m\n");

    final_token_opt
}

pub(crate) async fn get_or_prompt_token(ai_service: &AIChatService) -> Option<String> {
    if let Some(token) = get_configured_token() {
        if ai_service.has_configured_provider(0).await {
            return Some(token);
        }
    }
    println!("\x1b[33mKonfigurasi belum lengkap. Membuka Setup Wizard...\x1b[0m");
    run_cli_quickstart_wizard(ai_service).await
}
