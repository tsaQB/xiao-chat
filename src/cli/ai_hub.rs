use std::env;
use std::io::{self, BufRead, Write};

use rand::Rng;

use crate::ai::service::{
    load_provider_store, save_provider_store, ModelRole, ModelRoute, ProviderConfig, ProviderStore,
};
use crate::ai::storage::{CapabilityKind, CapabilityState, ProbeEvent, ProbeOutcome};
use crate::ai::AIChatService;
use crate::cli::status::addon_route_text;
use crate::cli::tui::terminal_interactive_select;
use crate::load_environment;

#[derive(Debug, PartialEq, Eq)]
pub enum AiCliAction<'a> {
    Menu,
    Use(Option<&'a str>),
    List,
    Add,
    Remove,
    Provider(Option<&'a str>),
    Addon,
    Test(Option<&'a str>),
    Help,
    Unknown(&'a str),
}

pub fn parse_ai_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
) -> AiCliAction<'a> {
    match action {
        None => AiCliAction::Menu,
        Some("use") => AiCliAction::Use(target),
        Some("list") => AiCliAction::List,
        Some("add") => AiCliAction::Add,
        Some("rm") | Some("remove") => AiCliAction::Remove,
        Some("provider") | Some("providers") => AiCliAction::Provider(target),
        Some("addon") | Some("addons") => AiCliAction::Addon,
        Some("test") | Some("probe") => AiCliAction::Test(target),
        Some("help") | Some("--help") | Some("-h") => AiCliAction::Help,
        Some(unknown) => AiCliAction::Unknown(unknown),
    }
}

pub(crate) async fn run_cli_provider_menu(ai_service: &AIChatService, action: Option<&str>) {
    load_environment();
    if action == Some("add") {
        run_cli_provider_add(ai_service).await;
        return;
    }
    if action == Some("rm") || action == Some("remove") {
        run_cli_provider_remove(ai_service).await;
        return;
    }

    loop {
        let store = load_provider_store();
        if store.providers.is_empty() {
            println!("\n\x1b[33mNo AI Providers registered yet.\x1b[0m");
            print!("Add a provider now? [Y/n]: ");
            let _ = io::stdout().flush();
            let mut ans = String::new();
            let _ = io::stdin().read_line(&mut ans);
            if ans.trim().eq_ignore_ascii_case("n") {
                return;
            }
            run_cli_provider_add(ai_service).await;
            return;
        }

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › AI Center › Provider Management\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 38;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let active_p = store
            .providers
            .iter()
            .find(|p| store.active_id.as_deref() == Some(&p.id));
        let active_str = active_p
            .map(|p| {
                format!(
                    "\x1b[1;32m● {}\x1b[0m \x1b[38;5;244m(active: {})\x1b[0m",
                    p.name, p.active_model
                )
            })
            .unwrap_or_else(|| "\x1b[38;5;244m○ None\x1b[0m".to_string());
        let standby_str = {
            let names: Vec<&str> = store
                .providers
                .iter()
                .filter(|p| store.active_id.as_deref() != Some(&p.id))
                .map(|p| p.name.as_str())
                .collect();
            if names.is_empty() {
                "\x1b[38;5;244m(none)\x1b[0m".to_string()
            } else {
                format!("\x1b[38;5;252m○ {}\x1b[0m", names.join(" · "))
            }
        };
        let total_str = format!(
            "\x1b[1;37m{} registered providers\x1b[0m",
            store.providers.len()
        );

        let hud_rows = [
            ("ACTIVE PROVIDER", active_str.as_str()),
            ("STANDBY PROVIDERS", standby_str.as_str()),
            ("TOTAL REGISTERED", total_str.as_str()),
        ];
        let hud = crate::cli::tui::render_hud_box("REGISTERED AI PROVIDERS", &hud_rows, bar_width);

        let title =
            format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mManage AI Providers:\x1b[0m");

        let mut menu_items: Vec<String> = store
            .providers
            .iter()
            .map(|p| {
                let is_act = store.active_id.as_deref() == Some(p.id.as_str());
                if is_act {
                    format!("{} [ACTIVE] ({})", p.name, p.active_model)
                } else {
                    format!("{} ({})", p.name, p.active_model)
                }
            })
            .collect();

        menu_items.push(
            "Add New Provider                (Preset: OpenRouter, Groq, Ollama...)".to_string(),
        );
        menu_items
            .push("Remove Provider                 (Delete provider configuration)".to_string());
        menu_items.push("Back to AI Hub                  (Return to Xiao AI Hub)".to_string());

        let sel = terminal_interactive_select(&title, &menu_items, 0, false, None);

        let Some(idx) = sel else {
            break;
        };

        if idx < store.providers.len() {
            let target_prov = &store.providers[idx];
            let is_act = store.active_id.as_deref() == Some(target_prov.id.as_str());

            let card_header = format!("PROVIDER PROFILE: {}", target_prov.name.to_uppercase());
            let state_str = if is_act {
                "\x1b[1;32m● ACTIVE PROVIDER\x1b[0m"
            } else {
                "\x1b[38;5;244m○ STANDBY\x1b[0m"
            };
            let total_models_str = format!(
                "{} models available (auto-discovered)",
                target_prov.models.len()
            );
            let card_rows = [
                ("BASE ENDPOINT", target_prov.endpoint.as_str()),
                ("ACTIVE MODEL", target_prov.active_model.as_str()),
                ("TOTAL MODELS", total_models_str.as_str()),
                ("PROVIDER STATE", state_str),
            ];
            let card_hud = crate::cli::tui::render_hud_box(&card_header, &card_rows, bar_width);
            let title_summary = format!("{card_hud}\r\n\r\n  \x1b[1;37mProvider Actions:\x1b[0m");

            let mut sub_actions = Vec::new();
            if !is_act {
                sub_actions.push(format!("Set as Active Provider ({})", target_prov.name));
            }
            sub_actions.push("Select / Change Model for this Provider".to_string());
            sub_actions.push(format!("Remove Provider ({})", target_prov.name));
            sub_actions.push("Back".to_string());

            let sub_sel = terminal_interactive_select(&title_summary, &sub_actions, 0, false, None);
            let Some(action_idx) = sub_sel else {
                continue;
            };

            let chosen_action = if is_act { action_idx + 1 } else { action_idx };

            match chosen_action {
                0 => {
                    let mut updated_store = load_provider_store();
                    updated_store.active_id = Some(target_prov.id.clone());
                    if let Err(e) = save_provider_store(&updated_store) {
                        println!("\n\x1b[31m✖ Error: Failed to save active provider: {e}\x1b[0m\n");
                        continue;
                    }
                    if !ai_service.reload_provider_store().await {
                        println!(
                            "\n\x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m\n"
                        );
                    }
                }
                1 => {
                    let (ok, res) = ai_service
                        .fetch_models_from_endpoint(&target_prov.endpoint, &target_prov.api_key)
                        .await;
                    let models = if ok {
                        res.unwrap_or_default()
                    } else {
                        target_prov.models.clone()
                    };
                    if !models.is_empty() {
                        let curr_idx = models
                            .iter()
                            .position(|m| m == &target_prov.active_model)
                            .unwrap_or(0);
                        if let Some(m_idx) = terminal_interactive_select(
                            &format!("Select Model for '{}':", target_prov.name),
                            &models,
                            curr_idx,
                            true,
                            None,
                        ) {
                            let chosen_model = models[m_idx].clone();
                            let mut updated_store = load_provider_store();
                            if let Some(p) = updated_store
                                .providers
                                .iter_mut()
                                .find(|p| p.id == target_prov.id)
                            {
                                p.active_model = chosen_model;
                                p.models = models;
                            }
                            if let Err(e) = save_provider_store(&updated_store) {
                                println!("\n\x1b[31m✖ Error: Failed to save provider model: {e}\x1b[0m\n");
                                continue;
                            }
                            if !ai_service.reload_provider_store().await {
                                println!("\n\x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m\n");
                            }
                        }
                    }
                }
                2 => {
                    let dependencies = ai_service
                        .provider_route_dependencies(&target_prov.id)
                        .await;
                    if !dependencies.is_empty() {
                        println!(
                            "\n\x1b[31m✖ Provider '{}' is still used by specific Addons.\x1b[0m",
                            target_prov.name
                        );
                        print!("\x1b[38;5;244mPress Enter to return...\x1b[0m");
                        let _ = io::stdout().flush();
                        let mut tmp = String::new();
                        let _ = io::stdin().read_line(&mut tmp);
                        continue;
                    }
                    let mut updated_store = load_provider_store();
                    if let Some(pos) = updated_store
                        .providers
                        .iter()
                        .position(|p| p.id == target_prov.id)
                    {
                        let removed = updated_store.providers.remove(pos);
                        if updated_store.active_id.as_deref() == Some(removed.id.as_str()) {
                            updated_store.active_id =
                                updated_store.providers.first().map(|p| p.id.clone());
                        }
                        if let Err(e) = save_provider_store(&updated_store) {
                            println!("\n\x1b[31m✖ Error: Failed to remove provider: {e}\x1b[0m\n");
                            continue;
                        }
                        if !ai_service.reload_provider_store().await {
                            println!(
                                "\n\x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m\n"
                            );
                        }
                    }
                }
                _ => {}
            }
        } else if idx == store.providers.len() {
            run_cli_provider_add(ai_service).await;
        } else if idx == store.providers.len() + 1 {
            run_cli_provider_remove(ai_service).await;
        } else {
            break;
        }
    }
}

pub(crate) async fn run_cli_provider_add(ai_service: &AIChatService) {
    println!("\n\x1b[1;36mAdd New AI Provider\x1b[0m");
    println!("\x1b[38;5;238m────────────────────────────────────────────────────────────\x1b[0m");

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    let default_ep = crate::ai::storage::DEFAULT_OPENROUTER_ENDPOINT;
    print!("  \x1b[1;37mEndpoint URL\x1b[0m \x1b[38;5;244m[default: {default_ep}]:\x1b[0m ");
    let _ = io::stdout().flush();
    let mut endpoint_input = String::new();
    if reader.read_line(&mut endpoint_input).is_err() {
        return;
    }
    let trimmed_ep = endpoint_input.trim();
    let endpoint = if trimmed_ep.is_empty() {
        default_ep.to_string()
    } else {
        match crate::cli::wizard::normalize_endpoint_url(trimmed_ep) {
            Ok(normalized) => {
                if normalized != trimmed_ep {
                    println!(
                        "  \x1b[38;2;16;185;129m✔\x1b[0m \x1b[38;5;244mAdjusted endpoint:\x1b[0m \x1b[1;37m{normalized}\x1b[0m"
                    );
                }
                normalized
            }
            Err(err) => {
                println!("  \x1b[31m✖ Error: {err}\x1b[0m\n");
                return;
            }
        }
    };

    if endpoint == crate::ai::storage::DEFAULT_OPENROUTER_ENDPOINT
        || endpoint.contains("openrouter.ai")
    {
        print!("  \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(obtain at https://openrouter.ai/keys):\x1b[0m ");
    } else {
        print!("  \x1b[1;37mAPI Key\x1b[0m \x1b[38;5;244m(Enter if keyless):\x1b[0m ");
    }
    let _ = io::stdout().flush();
    let mut key_input = String::new();
    if reader.read_line(&mut key_input).is_err() {
        return;
    }
    let mut api_key = key_input.trim().trim_matches(['"', '\'', '`']).to_string();
    if api_key.is_empty() {
        api_key = "none".to_string();
    }

    print!("  \x1b[1;37mProvider Name / Alias:\x1b[0m ");
    let _ = io::stdout().flush();
    let mut alias_input = String::new();
    if reader.read_line(&mut alias_input).is_err() {
        return;
    }
    let raw_alias = alias_input.trim();
    let alias = if raw_alias.is_empty() {
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

    println!("  \x1b[38;5;244mConnecting to endpoint...\x1b[0m");
    let (ok, res) = ai_service
        .fetch_models_from_endpoint(&endpoint, &api_key)
        .await;
    if !ok {
        let err = res.err().unwrap_or_else(|| "Unknown error".to_string());
        println!("  \x1b[31m✖ Error: Failed to connect to provider ({err})\x1b[0m\n");
        return;
    }

    let models = res.unwrap_or_else(|_| vec!["gpt-4o".to_string()]);
    println!(
        "  \x1b[1;32m✔ Connected! Found {} models.\x1b[0m",
        models.len()
    );

    let selected_idx = terminal_interactive_select(
        "Select Active Model for this Provider:",
        &models,
        0,
        true,
        None,
    );

    let active_model = if let Some(idx) = selected_idx {
        models[idx].clone()
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
        name: alias.clone(),
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
        println!("  \x1b[31m✖ Error: Failed to save provider configuration: {error}\x1b[0m\n");
        return;
    }
    if !ai_service.reload_provider_store().await {
        println!("  \x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m\n");
        return;
    }

    println!(
        "\n  \x1b[1;32m✔ Provider '{}' successfully added and activated!\x1b[0m",
        alias
    );
    println!("    Active Model: \x1b[1;36m{}\x1b[0m\n", active_model);
}

pub(crate) async fn run_cli_provider_remove(ai_service: &AIChatService) {
    let mut store = load_provider_store();
    if store.providers.is_empty() {
        println!("\n\x1b[33mNo providers saved yet.\x1b[0m\n");
        return;
    }

    let items: Vec<String> = store
        .providers
        .iter()
        .map(|p| {
            let is_act = store.active_id.as_deref() == Some(p.id.as_str());
            if is_act {
                format!("{} \x1b[1;32m[ACTIVE]\x1b[0m", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect();

    let selected =
        terminal_interactive_select("Select Provider to Remove:", &items, 0, false, None);

    if let Some(idx) = selected {
        let target = store.providers[idx].clone();
        let dependencies = ai_service.provider_route_dependencies(&target.id).await;
        if !dependencies.is_empty() {
            println!(
                "\n\x1b[31m✖ Provider '{}' is still used by specific Addons.\x1b[0m\n",
                target.name
            );
            return;
        }
        let removed = store.providers.remove(idx);
        if store.active_id.as_deref() == Some(removed.id.as_str()) {
            store.active_id = store.providers.first().map(|p| p.id.clone());
        }
        if let Err(e) = save_provider_store(&store) {
            println!("\n\x1b[31m✖ Error: Failed to save provider changes: {e}\x1b[0m\n");
            return;
        }
        if !ai_service.reload_provider_store().await {
            println!("\n\x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m\n");
            return;
        }
        println!(
            "\n\x1b[1;32m✔ Provider '{}' successfully removed.\x1b[0m\n",
            removed.name
        );
    }
}

pub(crate) async fn run_cli_model_picker(ai_service: &AIChatService, initial_filter: Option<&str>) {
    load_environment();
    let mut store = load_provider_store();

    if store.providers.is_empty() {
        println!("\n\x1b[33mNo AI Providers registered yet. Run 'xiao provider'.\x1b[0m\n");
        return;
    }

    for prov in store.providers.iter_mut() {
        if prov.models.len() <= 1 && !prov.endpoint.is_empty() {
            if let (true, Ok(fetched)) = ai_service
                .fetch_models_from_endpoint(&prov.endpoint, &prov.api_key)
                .await
            {
                if !fetched.is_empty() {
                    prov.models = fetched;
                }
            }
        }
    }
    if let Err(e) = save_provider_store(&store) {
        println!("\n\x1b[31m✖ Error: Failed to save provider model catalog: {e}\x1b[0m\n");
        return;
    }

    let active_prov_id = store.active_id.clone().unwrap_or_default();
    let current_model = env::var("AI_MODEL").unwrap_or_default();

    let mut catalog: Vec<(String, String, String, bool)> = Vec::new();
    for prov in &store.providers {
        let is_prov_active = prov.id == active_prov_id;
        for m in &prov.models {
            let is_model_active =
                is_prov_active && (m == &prov.active_model || m == &current_model);
            catalog.push((
                prov.id.clone(),
                prov.name.clone(),
                m.clone(),
                is_model_active,
            ));
        }
    }

    if catalog.is_empty() {
        println!("\n\x1b[33mNo models found from registered providers.\x1b[0m\n");
        return;
    }

    let is_multi = store.providers.len() > 1;
    let items: Vec<String> = catalog
        .iter()
        .map(|(_, prov_name, model_name, is_act)| {
            let act_tag = if *is_act {
                " \x1b[1;32m[ACTIVE]\x1b[0m"
            } else {
                ""
            };
            if is_multi {
                format!(
                    "{} \x1b[38;5;244m({})\x1b[0m{}",
                    model_name, prov_name, act_tag
                )
            } else {
                format!("{}{}", model_name, act_tag)
            }
        })
        .collect();

    let curr_idx = catalog
        .iter()
        .position(|(_, _, _, is_act)| *is_act)
        .unwrap_or(0);

    let bar_width = crate::cli::tui::get_terminal_bar_width();
    let pkg_ver = env!("CARGO_PKG_VERSION");
    let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › AI Center › Model Quick-Picker\x1b[0m";
    let title_left_vis = 2 + 7 + 2 + 37;
    let ver_str = format!("v{pkg_ver}");
    let ver_vis = crate::cli::tui::visible_width(&ver_str);
    let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
    let mini_header = format!(
        "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
        " ".repeat(pad),
        "─".repeat(bar_width.saturating_sub(4))
    );

    let active_model_str = format!(
        "\x1b[1;32m● {}\x1b[0m \x1b[38;5;244m({})\x1b[0m",
        if current_model.is_empty() {
            "None"
        } else {
            &current_model
        },
        store
            .providers
            .iter()
            .find(|p| p.id == active_prov_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown")
    );
    let catalog_str = format!(
        "\x1b[1;37m{} models registered\x1b[0m \x1b[38;5;244m· {} providers\x1b[0m",
        catalog.len(),
        store.providers.len()
    );
    let filter_hint = "\x1b[38;5;244m(type keyword to filter / Enter to pick)\x1b[0m";

    let hud_rows = [
        ("ACTIVE MODEL", active_model_str.as_str()),
        ("TOTAL CATALOG", catalog_str.as_str()),
        ("SEARCH FILTER", filter_hint),
    ];
    let hud =
        crate::cli::tui::render_hud_box("CURRENT SELECTION & CONTEXT LIMIT", &hud_rows, bar_width);

    let title =
        format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Model to Activate:\x1b[0m");

    let selected_idx = terminal_interactive_select(&title, &items, curr_idx, true, initial_filter);

    if let Some(idx) = selected_idx {
        if let Some((prov_id, prov_name, chosen_model, _)) = catalog.get(idx) {
            let mut updated_store = load_provider_store();
            updated_store.active_id = Some(prov_id.clone());
            if let Some(p) = updated_store
                .providers
                .iter_mut()
                .find(|p| &p.id == prov_id)
            {
                p.active_model = chosen_model.clone();
            }
            if let Err(e) = save_provider_store(&updated_store) {
                println!("\n\x1b[31m✖ Error: Failed to save active model: {e}\x1b[0m\n");
                return;
            }
            if !ai_service.reload_provider_store().await {
                println!("\n\x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m\n");
                return;
            }
            println!(
                "\n\x1b[1;32m✔ Main Model set to: {}\x1b[0m ({})\n",
                chosen_model, prov_name
            );
        }
    }
}


pub(crate) async fn run_cli_addon_menu(ai_service: &AIChatService) {
    load_environment();
    loop {
        let providers = ai_service.get_user_providers(0).await;
        let routing = ai_service.model_routing_config().await;

        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › AI Center › Specialist Addon Routing\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 43;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let vision_route = routing
            .route(ModelRole::Vision)
            .cloned()
            .unwrap_or(ModelRoute::MainModel);
        let audio_route = routing
            .route(ModelRole::AudioStt)
            .cloned()
            .unwrap_or(ModelRoute::MainModel);
        let video_route = routing
            .route(ModelRole::Video)
            .cloned()
            .unwrap_or(ModelRoute::MainModel);
        let image_route = routing
            .route(ModelRole::ImageGeneration)
            .cloned()
            .unwrap_or(ModelRoute::Disabled);

        let vision_str = format!("→ {}", addon_route_text(&vision_route, &providers));
        let audio_str = format!("→ {}", addon_route_text(&audio_route, &providers));
        let video_str = format!("→ {}", addon_route_text(&video_route, &providers));
        let image_str = format!("→ {}", addon_route_text(&image_route, &providers));

        let hud_rows = [
            ("VISION ROUTE", vision_str.as_str()),
            ("AUDIO STT", audio_str.as_str()),
            ("VIDEO FRAMES", video_str.as_str()),
            ("IMAGE GEN", image_str.as_str()),
        ];
        let hud =
            crate::cli::tui::render_hud_box("CURRENT ADDON ROUTING MATRIX", &hud_rows, bar_width);

        let title = format!(
            "{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Addon Role to Configure:\x1b[0m"
        );

        let mut menu_items = Vec::new();
        menu_items.push(format!(
            "Vision (Image Understanding)   [{}]",
            addon_route_text(&vision_route, &providers)
        ));
        menu_items.push(format!(
            "Audio STT (Voice Notes)        [{}]",
            addon_route_text(&audio_route, &providers)
        ));
        menu_items.push(format!(
            "Video Frames (Video Analysis)  [{}]",
            addon_route_text(&video_route, &providers)
        ));
        menu_items.push(format!(
            "Image Generation               [{}]",
            addon_route_text(&image_route, &providers)
        ));
        menu_items.push("Test Capabilities of All Active Addon Models".to_string());
        menu_items.push("Reset All Addons to Main Model (Restore default)".to_string());
        menu_items.push("Back to AI Hub                 (Return to Xiao AI Hub)".to_string());

        let sel = terminal_interactive_select(&title, &menu_items, 0, false, None);

        let Some(idx) = sel else {
            break;
        };

        let roles = ModelRole::addon_roles();
        if idx < roles.len() {
            let role = roles[idx];
            run_cli_addon_role_submenu(ai_service, role).await;
        } else if idx == roles.len() {
            run_cli_addon_test_all_routes(ai_service).await;
        } else if idx == roles.len() + 1 {
            let mut failed = false;
            for r in ModelRole::addon_roles() {
                if let Err(e) = ai_service.set_model_route(r, ModelRoute::MainModel).await {
                    println!(
                        "\n\x1b[31m✖ Error: Failed to reset addon {}: {e}\x1b[0m\n",
                        r.display_name()
                    );
                    failed = true;
                    break;
                }
            }
            if !failed {
                println!("\n\x1b[1;32m✔ All addon roles reset to Main Model.\x1b[0m\n");
            }
        } else {
            break;
        }
    }
}

async fn run_cli_addon_role_submenu(ai_service: &AIChatService, role: ModelRole) {
    let providers = ai_service.get_user_providers(0).await;
    let routing = ai_service.model_routing_config().await;
    let curr_route = routing
        .route(role)
        .cloned()
        .unwrap_or(ModelRoute::MainModel);
    let route_label = addon_route_text(&curr_route, &providers);

    let summary = format!(
        "== Addon Role: {} ==\r\n\
         • Current Route: \x1b[1;36m{}\x1b[0m",
        role.display_name(),
        route_label
    );

    let options = vec![
        "Use Main Model (Inherited / Default)".to_string(),
        "Disable This Role (Disabled)".to_string(),
        "Select Specific Model from Provider...".to_string(),
        "Back".to_string(),
    ];

    let sel = terminal_interactive_select(&summary, &options, 0, false, None);
    let Some(choice) = sel else {
        return;
    };

    match choice {
        0 => {
            if let Err(e) = ai_service
                .set_model_route(role, ModelRoute::MainModel)
                .await
            {
                println!(
                    "\n\x1b[31m✖ Error: Failed to save route {}: {e}\x1b[0m\n",
                    role.display_name()
                );
            }
        }
        1 => {
            if let Err(e) = ai_service.set_model_route(role, ModelRoute::Disabled).await {
                println!(
                    "\n\x1b[31m✖ Error: Failed to disable route {}: {e}\x1b[0m\n",
                    role.display_name()
                );
            }
        }
        2 => {
            let mut choices = Vec::new();
            let mut routes = Vec::new();
            for prov in &providers {
                for m in &prov.models {
                    choices.push(format!("{} :: {}", prov.name, m));
                    routes.push(ModelRoute::Specific {
                        provider_id: prov.id.clone(),
                        model: m.clone(),
                    });
                }
            }
            if choices.is_empty() {
                println!("\n\x1b[31m✖ No provider models available.\x1b[0m\n");
                return;
            }
            if let Some(m_idx) = terminal_interactive_select(
                &format!("Select Specific Model for {}:", role.display_name()),
                &choices,
                0,
                true,
                None,
            ) {
                let chosen_route = routes[m_idx].clone();
                let chosen_label = choices[m_idx].clone();
                if let Err(e) = ai_service.set_model_route(role, chosen_route).await {
                    println!(
                        "\n\x1b[31m✖ Error: Failed to save route {}: {e}\x1b[0m\n",
                        role.display_name()
                    );
                } else {
                    println!(
                        "\n\x1b[1;32m✔ Route {} routed to: {}\x1b[0m",
                        role.display_name(),
                        chosen_label
                    );
                }
            }
        }
        _ => {}
    }
}

async fn run_cli_addon_test_all_routes(ai_service: &AIChatService) {
    println!(
        "\n\x1b[1;36mDiagnostics & Capability Testing for All Active Addon Routes...\x1b[0m\n"
    );
    let providers = ai_service.get_user_providers(0).await;
    let routing = ai_service.model_routing_config().await;

    for role in ModelRole::addon_roles() {
        let route = routing
            .route(role)
            .cloned()
            .unwrap_or(ModelRoute::MainModel);
        let route_str = addon_route_text(&route, &providers);

        if route == ModelRoute::Disabled {
            println!(
                "  ● {:<22} : \x1b[38;5;244m✖ Disabled (Skipped)\x1b[0m\n",
                role.display_name()
            );
            continue;
        }

        println!("  ● \x1b[1m{}\x1b[0m → {}", role.display_name(), route_str);

        if role == ModelRole::ImageGeneration {
            print!("    Test Image Generation? (may consume API quota) [y/N]: ");
            let _ = io::stdout().flush();
            let mut ans = String::new();
            let _ = io::stdin().read_line(&mut ans);
            if !ans.trim().eq_ignore_ascii_case("y") {
                println!("    \x1b[38;5;244m○ Image Generation test skipped.\x1b[0m\n");
                continue;
            }
            match ai_service
                .probe_image_generation_active_with_observer(
                    ModelRole::ImageGeneration,
                    print_probe_event,
                )
                .await
            {
                Ok((_rec, ProbeOutcome::Supported)) => {
                    println!(
                        "    \x1b[1;32m✔ Success: Image Generation verified & working normally.\x1b[0m"
                    );
                }
                Ok((rec, outcome)) => {
                    println!(
                        "    \x1b[31m✖ Failed: Probe result {:?}, saved status {:?}.\x1b[0m",
                        outcome,
                        rec.effective_state_for(CapabilityKind::ImageGeneration)
                    );
                }
                Err(e) => {
                    println!("    \x1b[31m✖ Error: {e}\x1b[0m");
                }
            }
        } else {
            match ai_service
                .probe_addon_role_with_observer(role, print_probe_event)
                .await
            {
                Ok((record, status)) => match status {
                    ProbeOutcome::Supported => {
                        println!(
                            "    \x1b[32m✔ Capability verified & saved to SQLite ({})\x1b[0m",
                            record.checked_at
                        );
                    }
                    ProbeOutcome::Unsupported => {
                        println!(
                            "    \x1b[31m✖ Model rejected this capability (Unsupported).\x1b[0m"
                        );
                    }
                    _ => {
                        println!("    \x1b[33m○ Capability status unverified ({status:?}).\x1b[0m");
                    }
                },
                Err(e) => {
                    println!("    \x1b[31m✖ Error probe: {e}\x1b[0m");
                }
            }
        }
        println!();
    }

    println!("\x1b[1;32m✔ Finished checking all addon routes.\x1b[0m");
    print_press_enter();
}

pub(crate) async fn run_cli_probe_menu(ai_service: &AIChatService) {
    load_environment();
    loop {
        let bar_width = crate::cli::tui::get_terminal_bar_width();
        let pkg_ver = env!("CARGO_PKG_VERSION");
        let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › AI Center › Live Diagnostic Probes\x1b[0m";
        let title_left_vis = 2 + 7 + 2 + 41;
        let ver_str = format!("v{pkg_ver}");
        let ver_vis = crate::cli::tui::visible_width(&ver_str);
        let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
        let mini_header = format!(
            "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
            " ".repeat(pad),
            "─".repeat(bar_width.saturating_sub(4))
        );

        let main_route = ai_service.resolve_model_route(ModelRole::Main).await;
        let cap_record = match &main_route {
            Ok(r) => {
                ai_service
                    .capability_record(&r.provider.endpoint, &r.model)
                    .await
            }
            Err(_) => None,
        };

        let format_probe_status = |kind: CapabilityKind| -> String {
            match cap_record.as_ref().map(|r| r.effective_state_for(kind)) {
                Some(CapabilityState::Supported) => "\x1b[1;32m● Supported\x1b[0m".to_string(),
                Some(CapabilityState::Unsupported) => "\x1b[31m✖ Unsupported\x1b[0m".to_string(),
                _ => "\x1b[38;5;244m○ Unknown\x1b[0m".to_string(),
            }
        };

        let text_status = format_probe_status(CapabilityKind::TextChat);
        let vision_status = format_probe_status(CapabilityKind::ImageInput);
        let video_status = format_probe_status(CapabilityKind::VideoInput);
        let audio_status = format_probe_status(CapabilityKind::AudioInput);

        let hud_rows = [
            ("TEXT CHAT", text_status.as_str()),
            ("VISION", vision_status.as_str()),
            ("VIDEO FRAMES", video_status.as_str()),
            ("AUDIO STT", audio_status.as_str()),
        ];
        let hud = crate::cli::tui::render_hud_box(
            "EVIDENCE-BASED CAPABILITY MATRIX",
            &hud_rows,
            bar_width,
        );

        let title = format!(
            "{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect Diagnostic Action:\x1b[0m"
        );

        let menu_items = vec![
            "Audit & Refresh All Active Models (Run full diagnostic probe suite)".to_string(),
            "Test Vision Specialist           (Send test image to active model)".to_string(),
            "Test Video Specialist            (Verify video frame extraction)".to_string(),
            "Test Audio STT Specialist        (Verify audio transcription)".to_string(),
            "Test Image Gen Specialist        (Live image generation test)".to_string(),
            "Test Memory Curator Specialist   (Test Tier-1 memory summarization)".to_string(),
            "View SQLite Capability Cache     (Inspect raw evidence table)".to_string(),
            "Back to AI Hub                   (Return to Xiao AI Hub)".to_string(),
        ];

        let sel = terminal_interactive_select(&title, &menu_items, 0, false, None);

        let Some(idx) = sel else {
            break;
        };

        match idx {
            0 => {
                run_cli_probe_all_active(ai_service).await;
                print_press_enter();
            }
            1 => {
                run_cli_probe_test_role(ai_service, ModelRole::Vision).await;
                print_press_enter();
            }
            2 => {
                run_cli_probe_test_role(ai_service, ModelRole::Video).await;
                print_press_enter();
            }
            3 => {
                run_cli_probe_test_role(ai_service, ModelRole::AudioStt).await;
                print_press_enter();
            }
            4 => {
                run_cli_probe_test_image_gen(ai_service).await;
                print_press_enter();
            }
            5 => {
                run_cli_probe_test_role(ai_service, ModelRole::Curator).await;
                print_press_enter();
            }
            6 => {
                run_cli_probe_show_registry().await;
                print_press_enter();
            }
            _ => break,
        }
    }
}

fn print_press_enter() {
    print!("\n\x1b[38;5;244mPress Enter to return...\x1b[0m");
    let _ = io::stdout().flush();
    let mut tmp = String::new();
    let _ = io::stdin().read_line(&mut tmp);
}

fn capability_display_label(cap: CapabilityKind) -> &'static str {
    match cap {
        CapabilityKind::TextChat => "Text Chat",
        CapabilityKind::ImageInput => "Vision (Image)",
        CapabilityKind::ImageGeneration => "Image Generation",
        CapabilityKind::ImageEditing => "Image Editing",
        CapabilityKind::AudioInput => "Audio Native",
        CapabilityKind::AudioTranscription => "Audio STT",
        CapabilityKind::VideoInput => "Video Frames",
        CapabilityKind::NativeFileInput => "Native File",
        CapabilityKind::Tools => "Tools / Function",
        CapabilityKind::StructuredOutput => "Structured JSON",
        CapabilityKind::Reasoning => "Reasoning",
    }
}

fn format_probe_outcome_badge(outcome: ProbeOutcome) -> String {
    match outcome {
        ProbeOutcome::Supported => "\x1b[1;32m✔ Supported\x1b[0m".to_string(),
        ProbeOutcome::Unsupported => "\x1b[1;31m✖ Unsupported\x1b[0m".to_string(),
        ProbeOutcome::Inconclusive => "\x1b[33m○ Inconclusive\x1b[0m".to_string(),
        ProbeOutcome::Timeout => "\x1b[33m! Timeout\x1b[0m".to_string(),
        ProbeOutcome::NetworkError => "\x1b[31m✖ NetworkError\x1b[0m".to_string(),
        ProbeOutcome::ProtocolMismatch => "\x1b[33m! ProtocolMismatch\x1b[0m".to_string(),
        ProbeOutcome::AuthFailed => "\x1b[31m✖ AuthFailed\x1b[0m".to_string(),
        ProbeOutcome::RateLimited => "\x1b[33m! RateLimited\x1b[0m".to_string(),
        ProbeOutcome::ProviderError => "\x1b[31m✖ ProviderError\x1b[0m".to_string(),
    }
}

fn format_cap_bool_badge(val: Option<bool>) -> &'static str {
    match val {
        Some(true) => "\x1b[32m✔ Supported\x1b[0m",
        Some(false) => "\x1b[31m✖ Unsupported\x1b[0m",
        None => "\x1b[38;5;244m○ Unknown\x1b[0m",
    }
}

pub(crate) async fn run_cli_probe_all_active(ai_service: &AIChatService) {
    println!("\n\x1b[1;36mChecking Active Model Capabilities...\x1b[0m\n");
    let providers = ai_service.get_user_providers(0).await;
    if providers.is_empty() {
        println!("  \x1b[33m✖ No AI providers registered yet.\x1b[0m");
        return;
    }

    for prov in &providers {
        let model = &prov.active_model;
        if model.is_empty() {
            continue;
        }
        println!("  ● Provider: \x1b[1m{}\x1b[0m ({})", prov.name, model);
        if let Some(record) = run_persisted_capability_probe(ai_service, prov, model).await {
            println!("    \x1b[1;37mCapability Diagnostic Summary:\x1b[0m");
            println!(
                "      • Text Chat        : {}",
                format_cap_bool_badge(record.supports_text_chat)
            );
            println!(
                "      • Vision (Image)   : {}",
                format_cap_bool_badge(record.supports_image_input)
            );
            println!(
                "      • Structured JSON  : {}",
                format_cap_bool_badge(record.supports_structured_output)
            );
            println!(
                "      • Tools / Function : {}",
                format_cap_bool_badge(record.supports_tools)
            );
            println!(
                "      • Audio Native     : {}",
                format_cap_bool_badge(record.supports_audio_input)
            );
            println!(
                "      • Audio STT        : {}",
                format_cap_bool_badge(record.supports_audio_transcription)
            );
            println!(
                "      • Video Frames     : {}",
                format_cap_bool_badge(record.supports_video_input)
            );
            if let Some(ctx) = record.context_window {
                println!("      • Context Limit    : \x1b[36m{} tokens\x1b[0m", ctx);
            }
        } else {
            println!(
                "    \x1b[31m✖ Capability verification failed / endpoint did not respond.\x1b[0m"
            );
        }
        println!();
    }
    println!("\x1b[1;32m✔ Diagnostics completed. Results do not restrict route usage.\x1b[0m");
}

async fn run_cli_probe_test_role(ai_service: &AIChatService, role: ModelRole) {
    println!(
        "\n\x1b[1;36mDiagnostics & Live Test: {}\x1b[0m",
        role.display_name()
    );
    println!("  Optional route diagnostics, not a requirement for usage...");
    match ai_service
        .probe_addon_role_with_observer(role, print_probe_event)
        .await
    {
        Ok((record, status)) => match status {
            ProbeOutcome::Supported => {
                println!(
                    "  ✔ Observed Supported (checked: {}); see persistence result above",
                    record.checked_at
                );
            }
            ProbeOutcome::Unsupported => {
                println!("  ✖ Probe executed: Model explicitly rejected capability.");
            }
            ProbeOutcome::Inconclusive
            | ProbeOutcome::AuthFailed
            | ProbeOutcome::RateLimited
            | ProbeOutcome::Timeout
            | ProbeOutcome::NetworkError
            | ProbeOutcome::ProtocolMismatch
            | ProbeOutcome::ProviderError => {
                println!("  ⚪ Completed but not verified: Result is inconclusive/stale.");
            }
        },
        Err(e) => {
            println!("  ✖ Error / PersistenceFailed: {e}");
        }
    }
}

async fn run_cli_probe_test_image_gen(ai_service: &AIChatService) {
    println!("\n\x1b[1;36mLive Test: Image Generation\x1b[0m");
    println!(
        "\x1b[33mWarning: This test will generate a test image and may consume API credits.\x1b[0m"
    );
    print!("Proceed with test? [y/N]: ");
    let _ = io::stdout().flush();
    let mut ans = String::new();
    let _ = io::stdin().read_line(&mut ans);
    if !ans.trim().eq_ignore_ascii_case("y") {
        println!("○ Test cancelled.");
        return;
    }

    println!("Generating test image...");
    match ai_service
        .probe_image_generation_active_with_observer(ModelRole::ImageGeneration, print_probe_event)
        .await
    {
        Ok((_rec, ProbeOutcome::Supported)) => {
            println!(
                "  \x1b[1;32m✔ Success: Image generated successfully and passed runtime validation.\x1b[0m"
            );
        }
        Ok((rec, outcome)) => {
            println!(
                "  \x1b[31m✖ Failed: Probe result {:?}, saved status {:?}.\x1b[0m",
                outcome,
                rec.effective_state_for(CapabilityKind::ImageGeneration)
            );
        }
        Err(e) => {
            println!("  \x1b[31m✖ Error: {e}\x1b[0m");
        }
    }
}

async fn run_cli_probe_show_registry() {
    let registry = crate::ai::service::load_capability_registry();
    println!(
        "\n\x1b[1;36mCapability Registry (Total {} models):\x1b[0m\n",
        registry.models.len()
    );
    if registry.models.is_empty() {
        println!("  \x1b[38;5;244mNo model capabilities saved in registry yet.\x1b[0m\n");
        return;
    }
    for r in &registry.models {
        let vision = if r.supports_image_input == Some(true) {
            "\x1b[32m✔ Vision\x1b[0m"
        } else {
            "\x1b[38;5;244m○ Vision\x1b[0m"
        };
        let audio = if r.supports_audio_input == Some(true)
            || r.supports_audio_transcription == Some(true)
        {
            "\x1b[32m✔ Audio\x1b[0m"
        } else {
            "\x1b[38;5;244m○ Audio\x1b[0m"
        };
        let tools = if r.supports_tools == Some(true) {
            "\x1b[32m✔ Tools\x1b[0m"
        } else {
            "\x1b[38;5;244m○ Tools\x1b[0m"
        };
        let ctx_str = r
            .context_window
            .map(|c| format!(" · Ctx: {c}"))
            .unwrap_or_default();
        println!(
            "  ● \x1b[1m{}\x1b[0m ({})\n    [{vision} · {audio} · {tools}{ctx_str}] \x1b[38;5;244m· {}\x1b[0m",
            r.model, r.provider_name, r.checked_at
        );
    }
}

fn print_probe_event(event: ProbeEvent) {
    match event {
        ProbeEvent::Started { .. } => {}
        ProbeEvent::Progress {
            capability,
            message,
        } => {
            if message.starts_with("Vision 1/2") || message.starts_with("Vision 2/2") {
                println!(
                    "    ├─ {:<20} : \x1b[38;5;244m{}\x1b[0m",
                    capability_display_label(capability),
                    message
                );
            }
        }
        ProbeEvent::Completed {
            capability,
            outcome,
        } => {
            println!(
                "    ├─ {:<20} : {}",
                capability_display_label(capability),
                format_probe_outcome_badge(outcome)
            );
        }
        ProbeEvent::Skipped { capability, reason } => {
            println!(
                "    ├─ {:<20} : \x1b[38;5;244m○ Skipped ({})\x1b[0m",
                capability_display_label(capability),
                reason
            );
        }
        ProbeEvent::Persistence { saved } => {
            if saved {
                println!("    └─ Persist Registry     : \x1b[32m✔ Saved to SQLite\x1b[0m");
            } else {
                println!("    └─ Persist Registry     : \x1b[31m✖ Persistence Failed\x1b[0m");
            }
        }
        ProbeEvent::Finished => {}
    }
}

async fn run_persisted_capability_probe(
    ai_service: &AIChatService,
    provider: &ProviderConfig,
    model: &str,
) -> Option<crate::ai::service::CapabilityRecord> {
    let candidate = ai_service
        .probe_model_capabilities_with_observer(provider, model, print_probe_event)
        .await;
    let persisted = ai_service
        .capability_record(&provider.endpoint, model)
        .await;
    match persisted {
        Some(record) if record.checked_at == candidate.checked_at => Some(record),
        _ => None,
    }
}

fn find_matching_model_in_provider(prov: &ProviderConfig, query: &str) -> Option<String> {
    if let Some(m) = prov.models.iter().find(|m| *m == query) {
        return Some(m.clone());
    }
    if let Some(m) = prov.models.iter().find(|m| m.eq_ignore_ascii_case(query)) {
        return Some(m.clone());
    }
    if prov.active_model.eq_ignore_ascii_case(query) && !prov.active_model.trim().is_empty() {
        return Some(prov.active_model.clone());
    }
    None
}

pub(crate) fn find_model_in_store<'a>(
    store: &'a ProviderStore,
    target: &str,
) -> Option<(&'a ProviderConfig, String)> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }

    if let Some((prov_query, model_query)) = target.split_once('/') {
        let prov_query = prov_query.trim();
        let model_query = model_query.trim();

        let matched_prov = store
            .providers
            .iter()
            .find(|p| p.id == prov_query || p.name == prov_query)
            .or_else(|| {
                store.providers.iter().find(|p| {
                    p.id.eq_ignore_ascii_case(prov_query) || p.name.eq_ignore_ascii_case(prov_query)
                })
            });

        if let Some(prov) = matched_prov {
            if let Some(m) = find_matching_model_in_provider(prov, model_query) {
                return Some((prov, m));
            }
        }
    }

    let active_prov = store
        .active_id
        .as_deref()
        .and_then(|aid| store.providers.iter().find(|p| p.id == aid))
        .or_else(|| store.providers.first());

    if let Some(prov) = active_prov {
        if let Some(m) = find_matching_model_in_provider(prov, target) {
            return Some((prov, m));
        }
    }

    let active_prov_id = active_prov.map(|p| p.id.as_str()).unwrap_or("");
    for prov in &store.providers {
        if prov.id == active_prov_id {
            continue;
        }
        if let Some(m) = find_matching_model_in_provider(prov, target) {
            return Some((prov, m));
        }
    }

    None
}

pub(crate) fn print_ai_models_list() {
    let store = load_provider_store();
    if store.providers.is_empty() {
        println!("\n\x1b[33mNo AI Providers registered yet.\x1b[0m");
        println!("  Run 'xiao ai add' to register a new provider.\n");
        return;
    }

    let active_id = store.active_id.as_deref().unwrap_or("");
    let rows: Vec<(String, String, String, String, bool)> = store
        .providers
        .iter()
        .map(|p| {
            let is_active = if active_id.is_empty() {
                store
                    .providers
                    .first()
                    .map(|fp| fp.id == p.id)
                    .unwrap_or(false)
            } else {
                p.id == active_id
            };
            let status = if is_active {
                "[ACTIVE]".to_string()
            } else {
                "INACTIVE".to_string()
            };
            let active_model = if p.active_model.trim().is_empty() {
                "-".to_string()
            } else {
                p.active_model.clone()
            };
            (
                p.name.clone(),
                active_model,
                p.models.len().to_string(),
                status,
                is_active,
            )
        })
        .collect();

    let col_prov = rows
        .iter()
        .map(|r| r.0.len())
        .max()
        .unwrap_or(8)
        .max("PROVIDER".len());
    let col_model = rows
        .iter()
        .map(|r| r.1.len())
        .max()
        .unwrap_or(12)
        .max("ACTIVE MODEL".len());
    let col_total = rows
        .iter()
        .map(|r| r.2.len())
        .max()
        .unwrap_or(12)
        .max("TOTAL MODELS".len());
    let col_status = "STATUS".len().max(8);

    println!(
        "\n\x1b[1;37m{:<w_prov$}  {:<w_model$}  {:>w_total$}  {:<w_status$}\x1b[0m",
        "PROVIDER",
        "ACTIVE MODEL",
        "TOTAL MODELS",
        "STATUS",
        w_prov = col_prov,
        w_model = col_model,
        w_total = col_total,
        w_status = col_status,
    );
    let total_width = col_prov + col_model + col_total + col_status + 6;
    println!("\x1b[38;5;238m{}\x1b[0m", "─".repeat(total_width));

    for (prov, model, total, status, is_active) in rows {
        let status_styled = if is_active {
            format!(
                "\x1b[1;32m{:<w_status$}\x1b[0m",
                status,
                w_status = col_status
            )
        } else {
            format!(
                "\x1b[38;5;244m{:<w_status$}\x1b[0m",
                status,
                w_status = col_status
            )
        };
        println!(
            "{:<w_prov$}  {:<w_model$}  {:>w_total$}  {}",
            prov,
            model,
            total,
            status_styled,
            w_prov = col_prov,
            w_model = col_model,
            w_total = col_total,
        );
    }

    println!(
        "\nUse 'xiao ai use <model>' to switch models. Use 'xiao ai add' to add a provider.\n"
    );
}

pub(crate) async fn run_cli_ai_hub(
    ai_service: &AIChatService,
    action: Option<&str>,
    target: Option<&str>,
) {
    load_environment();
    match parse_ai_cli_action(action, target) {
        AiCliAction::Menu => loop {
            let store = load_provider_store();
            let routing = ai_service.model_routing_config().await;

            let active_provider = if let Some(ref aid) = store.active_id {
                store.providers.iter().find(|p| &p.id == aid)
            } else {
                store.providers.first()
            };

            let (active_prov_name, active_model_name) = match active_provider {
                Some(p) => (
                    p.name.as_str(),
                    if p.active_model.trim().is_empty() {
                        "Not set"
                    } else {
                        p.active_model.as_str()
                    },
                ),
                None => ("None", "Not set"),
            };

            let bar_width = crate::cli::tui::get_terminal_bar_width();
            let pkg_ver = env!("CARGO_PKG_VERSION");
            let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › AI Management Hub\x1b[0m";
            let title_left_vis = 2 + 7 + 2 + 24;
            let ver_str = format!("v{pkg_ver}");
            let ver_vis = crate::cli::tui::visible_width(&ver_str);
            let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
            let mini_header = format!(
                "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m",
                " ".repeat(pad),
                "─".repeat(bar_width.saturating_sub(4))
            );

            let val_active = format!(
                "\x1b[1;32m●\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;244m({})\x1b[0m",
                active_model_name, active_prov_name
            );
            let val_prov = format!(
                "\x1b[38;2;16;185;129m●\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;244m· {} models · Healthy\x1b[0m",
                active_prov_name,
                store
                    .providers
                    .iter()
                    .find(|p| p.name == active_prov_name || p.id == active_prov_name)
                    .map(|p| p.models.len())
                    .unwrap_or(0)
            );
            let val_addons = {
                let vis = match routing.route(ModelRole::Vision) {
                    Some(ModelRoute::MainModel) | None => "\x1b[32mMain\x1b[0m",
                    Some(ModelRoute::Disabled) => "\x1b[38;5;244mDisabled\x1b[0m",
                    Some(ModelRoute::Specific { .. }) => "\x1b[38;5;75mCustom\x1b[0m",
                };
                let aud = match routing.route(ModelRole::AudioStt) {
                    Some(ModelRoute::MainModel) | None => "\x1b[32mMain\x1b[0m",
                    Some(ModelRoute::Disabled) => "\x1b[38;5;244mDisabled\x1b[0m",
                    Some(ModelRoute::Specific { .. }) => "\x1b[38;5;75mCustom\x1b[0m",
                };
                let img = match routing.route(ModelRole::ImageGeneration) {
                    Some(ModelRoute::MainModel) => "\x1b[32mMain\x1b[0m",
                    Some(ModelRoute::Disabled) | None => "\x1b[38;5;244mDisabled\x1b[0m",
                    Some(ModelRoute::Specific { .. }) => "\x1b[38;5;75mCustom\x1b[0m",
                };
                format!(
                    "\x1b[38;5;252mVision: {}\x1b[0m · \x1b[38;5;252mAudio: {}\x1b[0m · \x1b[38;5;252mImage: {}\x1b[0m",
                    vis, aud, img
                )
            };

            let hud_rows = [
                ("MAIN MODEL", val_active.as_str()),
                ("PROVIDER", val_prov.as_str()),
                ("ADDON ROUTES", val_addons.as_str()),
            ];
            let hud =
                crate::cli::tui::render_hud_box("ACTIVE AI CONFIGURATION", &hud_rows, bar_width);

            let title =
                format!("{mini_header}\r\n\r\n{hud}\r\n\r\n  \x1b[1;37mSelect AI Action:\x1b[0m");

            let menu_items = vec![
                "Switch Main Model             (Quick picker with filter search)".to_string(),
                "Manage AI Providers           (Add, remove, probe, switch endpoint)".to_string(),
                "Multimodal Addon Routing      (Vision, Audio STT, Video, Image Gen)".to_string(),
                "Live Diagnostic Probes        (Verify latency & multimodal capability)"
                    .to_string(),
                "View Registered Model Matrix  (Full table of models & context limits)".to_string(),
                "Back to Main Menu             (Exit to Xiao Control Center)".to_string(),
            ];

            let sel = terminal_interactive_select(&title, &menu_items, 0, false, None);
            let Some(idx) = sel else {
                break;
            };

            match idx {
                0 => {
                    run_cli_model_picker(ai_service, None).await;
                }
                1 => {
                    run_cli_provider_menu(ai_service, None).await;
                }
                2 => {
                    run_cli_addon_menu(ai_service).await;
                }
                3 => {
                    run_cli_probe_menu(ai_service).await;
                }
                4 => {
                    print_ai_models_list();
                    print!("\n\x1b[38;5;244mPress Enter to return...\x1b[0m");
                    let _ = std::io::stdout().flush();
                    let mut tmp = String::new();
                    let _ = std::io::stdin().read_line(&mut tmp);
                }
                _ => break,
            }
        },
        AiCliAction::Use(target_opt) => {
            let target = match target_opt {
                Some(t) if !t.trim().is_empty() => t.trim(),
                _ => {
                    run_cli_model_picker(ai_service, None).await;
                    return;
                }
            };

            let mut store = load_provider_store();
            if let Some((matched_provider, matched_model)) = find_model_in_store(&store, target) {
                let prov_id = matched_provider.id.clone();
                let prov_name = matched_provider.name.clone();
                let prov_endpoint = matched_provider.endpoint.clone();
                let model_name = matched_model;

                store.active_id = Some(prov_id.clone());
                if let Some(p) = store.providers.iter_mut().find(|p| p.id == prov_id) {
                    p.active_model = model_name.clone();
                }

                if let Err(e) = save_provider_store(&store) {
                    println!("\x1b[31m✖ Error: Failed to save provider configuration: {e}\x1b[0m");
                    return;
                }

                if !ai_service.reload_provider_store().await {
                    println!("\x1b[31m✖ Error: Failed to reload provider at runtime.\x1b[0m");
                    return;
                }

                println!(
                    "\x1b[32m✔\x1b[0m Main Model successfully switched to: \x1b[1m{}\x1b[0m",
                    model_name
                );
                println!("  • Provider : {} ({})", prov_name, prov_endpoint);
            } else {
                println!(
                    "\x1b[31m✖\x1b[0m Model '{}' not found in any registered provider.",
                    target
                );
                println!("  Run 'xiao ai list' to see available models or 'xiao ai add' to register a new provider.");
            }
        }
        AiCliAction::List => {
            print_ai_models_list();
        }
        AiCliAction::Add => {
            run_cli_provider_add(ai_service).await;
        }
        AiCliAction::Remove => {
            run_cli_provider_remove(ai_service).await;
        }
        AiCliAction::Provider(tgt) => {
            run_cli_provider_menu(ai_service, tgt).await;
        }
        AiCliAction::Addon => {
            run_cli_addon_menu(ai_service).await;
        }
        AiCliAction::Test(tgt) => match tgt {
            Some("all") => {
                run_cli_probe_all_active(ai_service).await;
                print_press_enter();
            }
            Some(role_str) => {
                if let Some(role) = ModelRole::parse(role_str) {
                    if role == ModelRole::ImageGeneration {
                        run_cli_probe_test_image_gen(ai_service).await;
                    } else if role == ModelRole::Main {
                        println!("\x1b[33mMain Model is tested via specialist roles or direct chat.\x1b[0m");
                    } else {
                        run_cli_probe_test_role(ai_service, role).await;
                    }
                    print_press_enter();
                } else {
                    println!("\x1b[31mModel role '{role_str}' is unknown.\x1b[0m");
                    println!("Options: vision, video, stt, image, curator, all");
                    print_press_enter();
                }
            }
            None => {
                run_cli_probe_menu(ai_service).await;
            }
        },
        AiCliAction::Help => {
            println!("\n\x1b[1;36mxiao ai — Unified AI Management Hub\x1b[0m\n");
            println!("\x1b[1;37mUsage:\x1b[0m");
            println!("  xiao ai [action]\n");
            println!("\x1b[1;37mSubcommands for 'ai':\x1b[0m");
            println!("     \x1b[36mxiao ai\x1b[0m             Open Interactive AI Center Hub");
            println!("     \x1b[36mxiao ai use <model>\x1b[0m Switch Main Model directly");
            println!(
                "     \x1b[36mxiao ai list\x1b[0m        Print table of registered providers and models"
            );
            println!("     \x1b[36mxiao ai [add|rm]\x1b[0m    Add or remove an OpenAI-compatible AI provider");
            println!("     \x1b[36mxiao ai addon\x1b[0m       Configure multimodal specialist routes (Vision, STT, Video, Image, Curator)");
            println!("     \x1b[36mxiao ai test [role]\x1b[0m Open live diagnostic probe center (or test: vision, stt, video, image, curator, all)\n");
        }
        AiCliAction::Unknown(unknown) => {
            println!("\x1b[31m✖ Error: Subcommand 'ai {unknown}' is unknown.\x1b[0m");
            println!("  Run 'xiao ai help' or 'xiao help' for assistance.");
        }
    }
}
