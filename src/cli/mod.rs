pub mod ai_hub;
pub mod chat;
pub mod context;
pub mod gateway;
pub mod help;
pub mod launcher;
pub mod mcp;
pub mod memory;
pub mod search;
pub mod status;
pub mod tui;
pub mod wizard;

#[cfg(test)]
mod tests;

pub(crate) use ai_hub::run_cli_ai_hub;
pub(crate) use chat::run_cli_chat;
pub(crate) use context::run_cli_context;
pub(crate) use gateway::run_cli_gateway_hub;
pub(crate) use help::print_cli_help;
pub(crate) use launcher::run_cli_launcher;
pub(crate) use mcp::run_cli_mcp_hub;
pub(crate) use memory::run_cli_memory;
pub(crate) use search::run_cli_search_hub;
pub(crate) use status::run_cli_status;
pub(crate) use wizard::{get_or_prompt_token, run_cli_quickstart_wizard};
