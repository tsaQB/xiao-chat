use super::*;
use crate::ai::service::{ModelRoute, ProviderConfig, ProviderStore};
use crate::cli::ai_hub::{find_model_in_store, parse_ai_cli_action, AiCliAction};
use crate::cli::chat::{parse_chat_cli_command, ChatCliCommand};
use crate::cli::context::{format_context_gauge, parse_context_cli_args, ContextCliArgs};
use crate::cli::gateway::{parse_gateway_cli_action, GatewayCliAction};
use crate::cli::launcher::build_telemetry_hud;
use crate::cli::mcp::{parse_mcp_cli_action, McpCliAction};
use crate::cli::memory::{parse_memory_cli_action, MemoryCliAction};
use crate::cli::search::{mask_api_key, parse_search_cli_action, SearchCliAction};
use crate::cli::status::addon_route_text;
use crate::cli::tui::{
    cycle_next, cycle_prev, format_tui_title, get_terminal_bar_width, truncate_visible,
    visible_width, MENU_BAR_WIDTH,
};

#[test]
fn find_model_in_store_exact_and_cross_provider() {
    let store = ProviderStore {
        active_id: Some("groq-1".to_string()),
        providers: vec![
            ProviderConfig {
                id: "groq-1".to_string(),
                name: "Groq".to_string(),
                endpoint: "https://api.groq.com/openai/v1".to_string(),
                api_key: "".to_string(),
                api_key_ref: None,
                models: vec![
                    "llama-3.3-70b-versatile".to_string(),
                    "llama-3.1-8b-instant".to_string(),
                ],
                active_model: "llama-3.3-70b-versatile".to_string(),
            },
            ProviderConfig {
                id: "openai-1".to_string(),
                name: "OpenAI".to_string(),
                endpoint: "https://api.openai.com/v1".to_string(),
                api_key: "".to_string(),
                api_key_ref: None,
                models: vec![
                    "gpt-4o".to_string(),
                    "gpt-4o-mini".to_string(),
                    "meta-llama/llama-3.1-8b-instruct".to_string(),
                ],
                active_model: "gpt-4o".to_string(),
            },
        ],
    };

    // Match in active provider
    let res = find_model_in_store(&store, "llama-3.1-8b-instant");
    assert!(res.is_some());
    let (p, m) = res.expect("model found in store");
    assert_eq!(p.id, "groq-1");
    assert_eq!(m, "llama-3.1-8b-instant");

    // Model name containing a slash when prefix is not a provider
    let res = find_model_in_store(&store, "meta-llama/llama-3.1-8b-instruct");
    assert!(res.is_some());
    let (p, m) = res.expect("model found in store");
    assert_eq!(p.id, "openai-1");
    assert_eq!(m, "meta-llama/llama-3.1-8b-instruct");

    // Match across other provider
    let res = find_model_in_store(&store, "gpt-4o-mini");
    assert!(res.is_some());
    let (p, m) = res.expect("model found in store");
    assert_eq!(p.id, "openai-1");
    assert_eq!(m, "gpt-4o-mini");

    // Case-insensitive match across provider
    let res = find_model_in_store(&store, "GPT-4O-MINI");
    assert!(res.is_some());
    let (p, m) = res.expect("model found in store");
    assert_eq!(p.id, "openai-1");
    assert_eq!(m, "gpt-4o-mini");

    // Delimited provider/model match with name
    let res = find_model_in_store(&store, "openai/gpt-4o");
    assert!(res.is_some());
    let (p, m) = res.expect("model found in store");
    assert_eq!(p.id, "openai-1");
    assert_eq!(m, "gpt-4o");

    // Delimited provider/model match with ID
    let res = find_model_in_store(&store, "groq-1/llama-3.1-8b-instant");
    assert!(res.is_some());
    let (p, m) = res.expect("model found in store");
    assert_eq!(p.id, "groq-1");
    assert_eq!(m, "llama-3.1-8b-instant");

    // Delimited with non-existent model in that provider
    let res = find_model_in_store(&store, "groq/gpt-4o");
    assert!(res.is_none());

    // Model not found anywhere
    let res = find_model_in_store(&store, "claude-3-5-sonnet");
    assert!(res.is_none());
}

#[test]
fn print_cli_help_executes() {
    print_cli_help();
}

#[test]
fn test_interactive_cursor_wrap_around() {
    assert_eq!(cycle_prev(0, 5), 4);
    assert_eq!(cycle_prev(1, 5), 0);
    assert_eq!(cycle_prev(4, 5), 3);
    assert_eq!(cycle_next(0, 5), 1);
    assert_eq!(cycle_next(3, 5), 4);
    assert_eq!(cycle_next(4, 5), 0);

    // Edge cases
    assert_eq!(cycle_prev(0, 0), 0);
    assert_eq!(cycle_next(0, 0), 0);
    assert_eq!(cycle_prev(0, 1), 0);
    assert_eq!(cycle_next(0, 1), 0);
    assert_eq!(cycle_prev(10, 5), 4);
    assert_eq!(cycle_next(10, 5), 0);
}

#[test]
fn test_visible_width() {
    assert_eq!(visible_width(""), 0);
    assert_eq!(visible_width("Hello World"), 11);
    assert_eq!(visible_width("\x1b[1;32m[ACTIVE]\x1b[0m"), 8);
    assert_eq!(visible_width("\x1b[38;5;81m ▸ \x1b[0m"), 3);
    assert_eq!(
        visible_width("\x1b[48;5;237m\x1b[1;38;5;81m ▸ \x1b[1;37m 1. Model\x1b[0m"),
        12
    );
    assert_eq!(
        visible_width("OpenAI \x1b[1;32m[ACTIVE]\x1b[0m (gpt-4o)"),
        24
    );
}

#[test]
fn test_truncate_visible() {
    assert_eq!(truncate_visible("Hello World", 20), "Hello World");
    assert_eq!(truncate_visible("Hello World", 11), "Hello World");
    assert_eq!(truncate_visible("Hello World", 8), "Hello W…\x1b[0m");
    assert_eq!(
        truncate_visible("\x1b[1;32mHello World\x1b[0m", 8),
        "\x1b[1;32mHello W…\x1b[0m"
    );
}

#[test]
fn test_format_tui_title() {
    // Single line title
    let single = format_tui_title("Select Main Model:");
    assert_eq!(single.len(), 1);
    assert!(single[0].contains("Select Main Model:"));
    assert!(single[0].contains("\x1b[1;38;5;45m"));

    // Single line already containing ANSI
    let colored = format_tui_title("\x1b[1;36mTitle\x1b[0m");
    assert_eq!(colored.len(), 1);
    assert_eq!(colored[0], "\x1b[1;36mTitle\x1b[0m");

    // Multi-line title
    let multi = "== Xiao AI Management Hub ==\r\n • Active Model   : gpt-4o\r\n • Addon Routes:\r\n     Vision   : \x1b[38;5;37mMain Model\x1b[0m";
    let res = format_tui_title(multi);
    assert_eq!(res.len(), 4);
    assert!(res[0].contains("\x1b[1;38;5;45m== Xiao AI Management Hub ==\x1b[0m"));
    assert!(res[1].contains("\x1b[38;5;245m • Active Model   :\x1b[0m"));
    assert!(res[1].contains("\x1b[1;37m gpt-4o\x1b[0m"));
    assert!(res[2].contains("\x1b[38;5;245m • Addon Routes:\x1b[0m"));
    assert!(res[3].contains("\x1b[38;5;245m     Vision   :\x1b[0m"));
    assert!(res[3].contains("\x1b[38;5;37mMain Model\x1b[0m"));
}

#[test]
fn test_menu_bar_width_bounds() {
    let width = get_terminal_bar_width();
    assert!((40..=MENU_BAR_WIDTH).contains(&width));
}

#[test]
fn test_build_telemetry_hud() {
    let hud = build_telemetry_hud("○ Not connected", "○ No active Provider", "Exa MCP", 76);
    let lines: Vec<&str> = hud.lines().collect();
    assert_eq!(lines.len(), 5);
    assert!(lines[0].contains("STATUS TELEMETRY"));
    assert!(lines[1].contains("GATEWAY"));
    assert!(lines[2].contains("MAIN AI"));
    assert!(lines[3].contains("SEARCH"));
    assert!(lines[4].contains('╰'));

    // Check visible widths of all lines match
    let w0 = visible_width(lines[0]);
    let w1 = visible_width(lines[1]);
    let w2 = visible_width(lines[2]);
    let w3 = visible_width(lines[3]);
    let w4 = visible_width(lines[4]);
    assert_eq!(w0, 74);
    assert_eq!(w1, 74);
    assert_eq!(w2, 74);
    assert_eq!(w3, 74);
    assert_eq!(w4, 74);
}

#[test]
fn test_cli_help_includes_memory_and_context() {
    print_cli_help();
}

#[test]
fn test_parse_chat_cli_command() {
    assert_eq!(parse_chat_cli_command("/exit"), Some(ChatCliCommand::Exit));
    assert_eq!(parse_chat_cli_command("/quit"), Some(ChatCliCommand::Exit));
    assert_eq!(
        parse_chat_cli_command("/clear"),
        Some(ChatCliCommand::Clear)
    );
    assert_eq!(
        parse_chat_cli_command("/reset"),
        Some(ChatCliCommand::Clear)
    );
    assert_eq!(
        parse_chat_cli_command("/sessions"),
        Some(ChatCliCommand::Sessions)
    );
    assert_eq!(
        parse_chat_cli_command("/switch 2"),
        Some(ChatCliCommand::Switch(2))
    );
    assert_eq!(
        parse_chat_cli_command("/switch"),
        Some(ChatCliCommand::Unknown("/switch"))
    );
    assert_eq!(
        parse_chat_cli_command("/new Research Session"),
        Some(ChatCliCommand::New(Some("Research Session")))
    );
    assert_eq!(
        parse_chat_cli_command("/new"),
        Some(ChatCliCommand::New(None))
    );
    assert_eq!(
        parse_chat_cli_command("/rm 3"),
        Some(ChatCliCommand::Remove(3))
    );
    assert_eq!(
        parse_chat_cli_command("/model"),
        Some(ChatCliCommand::Model)
    );
    assert_eq!(
        parse_chat_cli_command("/models"),
        Some(ChatCliCommand::Model)
    );
    assert_eq!(parse_chat_cli_command("/help"), Some(ChatCliCommand::Help));
    assert_eq!(
        parse_chat_cli_command("/foobar"),
        Some(ChatCliCommand::Unknown("/foobar"))
    );
    assert_eq!(parse_chat_cli_command("Hello there"), None);
}

#[test]
fn test_parse_gateway_cli_action() {
    assert_eq!(parse_gateway_cli_action(None, None), GatewayCliAction::Menu);
    assert_eq!(
        parse_gateway_cli_action(Some("check"), None),
        GatewayCliAction::Check
    );
    assert_eq!(
        parse_gateway_cli_action(Some("status"), None),
        GatewayCliAction::Check
    );
    assert_eq!(
        parse_gateway_cli_action(Some("token"), Some("123:ABC")),
        GatewayCliAction::BindToken(Some("123:ABC"))
    );
    assert_eq!(
        parse_gateway_cli_action(Some("owner"), Some("42")),
        GatewayCliAction::SetOwner(Some("42"))
    );
    assert_eq!(
        parse_gateway_cli_action(Some("help"), None),
        GatewayCliAction::Help
    );
    assert_eq!(
        parse_gateway_cli_action(Some("unknown"), None),
        GatewayCliAction::Unknown("unknown")
    );
}

#[test]
fn test_parse_memory_cli_action() {
    assert_eq!(parse_memory_cli_action(None, None), MemoryCliAction::List);
    assert_eq!(
        parse_memory_cli_action(Some("list"), None),
        MemoryCliAction::List
    );
    assert_eq!(
        parse_memory_cli_action(Some("clear"), None),
        MemoryCliAction::Clear
    );
    assert_eq!(
        parse_memory_cli_action(Some("rm"), Some("preferred_language")),
        MemoryCliAction::Remove(Some("preferred_language"))
    );
    assert_eq!(
        parse_memory_cli_action(Some("help"), None),
        MemoryCliAction::Help
    );
    assert_eq!(
        parse_memory_cli_action(Some("what"), None),
        MemoryCliAction::Unknown("what")
    );
}

#[test]
fn test_parse_context_cli_args() {
    assert_eq!(
        parse_context_cli_args(Some("help"), None),
        ContextCliArgs::Help
    );
    assert_eq!(
        parse_context_cli_args(Some("--help"), None),
        ContextCliArgs::Help
    );
    assert_eq!(
        parse_context_cli_args(Some("12345"), Some("678")),
        ContextCliArgs::Inspect {
            chat_id: Some(12345),
            thread_id: Some(678)
        }
    );
    assert_eq!(
        parse_context_cli_args(None, None),
        ContextCliArgs::Inspect {
            chat_id: None,
            thread_id: None
        }
    );
    assert_eq!(
        parse_context_cli_args(Some("abc"), None),
        ContextCliArgs::InvalidChatId("abc")
    );
    assert_eq!(
        parse_context_cli_args(Some("123"), Some("def")),
        ContextCliArgs::InvalidThreadId("def")
    );
}

#[test]
fn test_format_context_gauge() {
    let (bar20, col20) = format_context_gauge(20.0, 10);
    assert_eq!(bar20, "██░░░░░░░░");
    assert_eq!(col20, "\x1b[1;32m");

    let (bar70, col70) = format_context_gauge(70.0, 10);
    assert_eq!(bar70, "███████░░░");
    assert_eq!(col70, "\x1b[1;33m");

    let (bar95, col95) = format_context_gauge(95.0, 10);
    assert_eq!(bar95, "██████████");
    assert_eq!(col95, "\x1b[1;31m");
}

#[test]
fn test_mask_api_key() {
    assert_eq!(mask_api_key(""), "(not set)");
    assert_eq!(mask_api_key("   "), "(not set)");
    assert_eq!(mask_api_key("short"), "••••••••");
    assert_eq!(mask_api_key("12345678"), "••••••••");
    assert_eq!(mask_api_key("sk-ant-api03-abcdef123456"), "sk-a••••3456");
}

#[test]
fn test_parse_mcp_cli_action() {
    assert_eq!(parse_mcp_cli_action(None, None, None), McpCliAction::Status);
    assert_eq!(
        parse_mcp_cli_action(Some("status"), None, None),
        McpCliAction::Status
    );
    assert_eq!(
        parse_mcp_cli_action(Some("list"), None, None),
        McpCliAction::List
    );
    assert_eq!(
        parse_mcp_cli_action(Some("help"), None, None),
        McpCliAction::Help
    );
    assert_eq!(
        parse_mcp_cli_action(Some("tools"), None, None),
        McpCliAction::Tools
    );
    assert_eq!(
        parse_mcp_cli_action(Some("url"), Some("https://mcp.local"), None),
        McpCliAction::Url(Some("https://mcp.local"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("add"), Some("custom"), Some("https://mcp.local")),
        McpCliAction::Add(Some("custom"), Some("https://mcp.local"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("rm"), Some("custom"), None),
        McpCliAction::Remove(Some("custom"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("test"), Some("query"), None),
        McpCliAction::Test(Some("query"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("search"), Some("query"), None),
        McpCliAction::Search(Some("query"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("brave"), Some("key"), None),
        McpCliAction::Brave(Some("key"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("tavily"), Some("key"), None),
        McpCliAction::Tavily(Some("key"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("exa"), Some("key"), None),
        McpCliAction::Exa(Some("key"))
    );
    assert_eq!(
        parse_mcp_cli_action(Some("reset"), None, None),
        McpCliAction::Reset
    );
    assert_eq!(
        parse_mcp_cli_action(Some("bogus"), None, None),
        McpCliAction::Unknown("bogus")
    );
}

#[test]
fn test_parse_search_cli_action() {
    assert_eq!(parse_search_cli_action(None, None), SearchCliAction::Status);
    assert_eq!(
        parse_search_cli_action(Some("status"), None),
        SearchCliAction::Status
    );
    assert_eq!(
        parse_search_cli_action(Some("help"), None),
        SearchCliAction::Help
    );
    assert_eq!(
        parse_search_cli_action(Some("test"), Some("query")),
        SearchCliAction::Test(Some("query"))
    );
    assert_eq!(
        parse_search_cli_action(Some("brave"), Some("key")),
        SearchCliAction::Brave(Some("key"))
    );
    assert_eq!(
        parse_search_cli_action(Some("tavily"), Some("key")),
        SearchCliAction::Tavily(Some("key"))
    );
    assert_eq!(
        parse_search_cli_action(Some("exa"), Some("key")),
        SearchCliAction::Exa(Some("key"))
    );
    assert_eq!(
        parse_search_cli_action(Some("engine"), Some("brave")),
        SearchCliAction::Engine(Some("brave"))
    );
    assert_eq!(
        parse_search_cli_action(Some("bogus"), None),
        SearchCliAction::Unknown("bogus")
    );
}

#[test]
fn test_parse_ai_cli_action() {
    assert_eq!(parse_ai_cli_action(None, None), AiCliAction::Menu);
    assert_eq!(
        parse_ai_cli_action(Some("use"), Some("gpt-4o")),
        AiCliAction::Use(Some("gpt-4o"))
    );
    assert_eq!(parse_ai_cli_action(Some("list"), None), AiCliAction::List);
    assert_eq!(parse_ai_cli_action(Some("add"), None), AiCliAction::Add);
    assert_eq!(parse_ai_cli_action(Some("rm"), None), AiCliAction::Remove);
    assert_eq!(
        parse_ai_cli_action(Some("provider"), Some("groq")),
        AiCliAction::Provider(Some("groq"))
    );
    assert_eq!(parse_ai_cli_action(Some("addon"), None), AiCliAction::Addon);
    assert_eq!(
        parse_ai_cli_action(Some("test"), Some("vision")),
        AiCliAction::Test(Some("vision"))
    );
    assert_eq!(parse_ai_cli_action(Some("help"), None), AiCliAction::Help);
    assert_eq!(
        parse_ai_cli_action(Some("invalid"), None),
        AiCliAction::Unknown("invalid")
    );
}

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

#[test]
fn test_normalize_endpoint_url() {
    use crate::cli::wizard::normalize_endpoint_url;

    // 1. Standard HTTPS
    assert_eq!(
        normalize_endpoint_url("https://cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 2. Trailing slashes
    assert_eq!(
        normalize_endpoint_url("https://cpa.oxygen.web.id/v1/").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 3. Domain without scheme (auto-infers https)
    assert_eq!(
        normalize_endpoint_url("cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 4. Domain without /v1 path (auto-appends /v1)
    assert_eq!(
        normalize_endpoint_url("cpa.oxygen.web.id").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 5. Localhost and 127.0.0.1 (auto-infers http and appends /v1 if missing)
    assert_eq!(
        normalize_endpoint_url("127.0.0.1:8317/v1").as_deref(),
        Ok("http://127.0.0.1:8317/v1")
    );
    assert_eq!(
        normalize_endpoint_url("127.0.0.1:8317").as_deref(),
        Ok("http://127.0.0.1:8317/v1")
    );
    assert_eq!(
        normalize_endpoint_url("localhost:11434").as_deref(),
        Ok("http://localhost:11434/v1")
    );

    // 6. Typo variations for https scheme
    assert_eq!(
        normalize_endpoint_url("https:cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );
    assert_eq!(
        normalize_endpoint_url("https//cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );
    assert_eq!(
        normalize_endpoint_url("https/cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );
    assert_eq!(
        normalize_endpoint_url("https cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );
    assert_eq!(
        normalize_endpoint_url("https: //cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );
    assert_eq!(
        normalize_endpoint_url("https:// cpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 7. Directly attached without delimiter
    assert_eq!(
        normalize_endpoint_url("httpscpa.oxygen.web.id/v1").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 8. Single-word remote domains (preserves host name)
    assert_eq!(
        normalize_endpoint_url("httpserver.com/v1").as_deref(),
        Ok("https://httpserver.com/v1")
    );

    // 9. Surrounding quotes & angle brackets
    assert_eq!(
        normalize_endpoint_url("<https://cpa.oxygen.web.id/v1>").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );
    assert_eq!(
        normalize_endpoint_url("\"https://cpa.oxygen.web.id/v1\"").as_deref(),
        Ok("https://cpa.oxygen.web.id/v1")
    );

    // 10. Errors
    assert!(normalize_endpoint_url("   ").is_err());
}
