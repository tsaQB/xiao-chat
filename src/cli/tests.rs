use crate::ai::service::{ProviderConfig, ProviderStore};
use crate::cli::ai_hub::find_model_in_store;
use crate::cli::help::print_cli_help;

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
fn test_cli_help_smoke() {
    print_cli_help();
}
