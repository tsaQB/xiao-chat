pub(crate) fn print_cli_help() {
    let bar_width = crate::cli::tui::get_terminal_bar_width();
    crate::cli::tui::print_mini_header("Help & Command Reference");

    println!("\n  \x1b[1;37mUsage:\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao\x1b[0m                                \x1b[38;5;250mStart terminal chat (default)\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao\x1b[0m \x1b[38;5;245m<question...>\x1b[0m                   \x1b[38;5;250mAsk a quick one-shot question\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao\x1b[0m \x1b[38;5;245m<command>\x1b[0m \x1b[38;5;245m[arguments...]\x1b[0m\n");

    println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mCORE\x1b[0m");
    println!("    \x1b[1;38;5;45mchat\x1b[0m \x1b[38;5;245m[prompt]\x1b[0m             \x1b[38;5;250mTerminal chat or one-shot prompt\x1b[0m");
    println!("    \x1b[1;38;5;45mmenu\x1b[0m                      \x1b[38;5;250mOpen interactive Control Center (TUI)\x1b[0m");
    println!("    \x1b[1;38;5;45msetup\x1b[0m                     \x1b[38;5;250mInteractive first-time setup wizard\x1b[0m");
    println!("    \x1b[1;38;5;45mstart\x1b[0m                     \x1b[38;5;250mRun Telegram bot daemon\x1b[0m\n");

    println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mAI & INTELLIGENCE\x1b[0m");
    println!("    \x1b[1;38;5;45mai\x1b[0m \x1b[38;5;245m[use|list|add|test]\x1b[0m    \x1b[38;5;250mManage models, providers, & multimodal routes\x1b[0m");
    println!("    \x1b[1;38;5;45msearch\x1b[0m \x1b[38;5;245m[test|brave|exa]\x1b[0m   \x1b[38;5;250mWeb search engine hub & provider API keys\x1b[0m");
    println!("    \x1b[1;38;5;45mmcp\x1b[0m \x1b[38;5;245m[list|add|tools]\x1b[0m      \x1b[38;5;250mModel Context Protocol (MCP) server & tool hub\x1b[0m");
    println!("    \x1b[1;38;5;45mcontext\x1b[0m \x1b[38;5;245m[chat] [thread]\x1b[0m   \x1b[38;5;250mInspect token consumption & sliding window budget\x1b[0m");
    println!("    \x1b[1;38;5;45mmemory\x1b[0m \x1b[38;5;245m[list|rm|clear]\x1b[0m    \x1b[38;5;250mManage Tier-1 long-term remembered facts\x1b[0m\n");

    println!("  \x1b[1;38;2;6;182;212m▸ \x1b[1;37mGATEWAY & SYSTEM\x1b[0m");
    println!("    \x1b[1;38;5;45mgateway\x1b[0m \x1b[38;5;245m[check|token|id]\x1b[0m  \x1b[38;5;250mManage Telegram bot token & owner authorization\x1b[0m");
    println!("    \x1b[1;38;5;45mstatus\x1b[0m                    \x1b[38;5;250mDisplay telemetry, health, & provider dashboard\x1b[0m");
    println!("    \x1b[1;38;5;45mversion\x1b[0m, \x1b[1;38;5;45m-v\x1b[0m               \x1b[38;5;250mDisplay binary version\x1b[0m");
    println!("    \x1b[1;38;5;45mhelp\x1b[0m, \x1b[1;38;5;45m-h\x1b[0m                  \x1b[38;5;250mShow this help reference\x1b[0m\n");

    println!(
        "  \x1b[38;5;238m{}\x1b[0m\n",
        "─".repeat(bar_width.saturating_sub(4))
    );

    println!("  \x1b[1;37mQuick Examples:\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao\x1b[0m                             \x1b[38;5;242m# Start chatting immediately\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao \"Explain Rust async\"\x1b[0m        \x1b[38;5;242m# Direct one-shot terminal query\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao menu\x1b[0m                        \x1b[38;5;242m# Open Control Center TUI\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao search test \"Rust 2024\"\x1b[0m     \x1b[38;5;242m# Test active search engine\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao search brave\x1b[0m                \x1b[38;5;242m# Interactively set Brave Search key\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao mcp list\x1b[0m                    \x1b[38;5;242m# List connected MCP servers\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao ai use\x1b[0m                      \x1b[38;5;242m# Interactive model picker\x1b[0m");
    println!("    \x1b[1;38;5;45mxiao gateway token\x1b[0m               \x1b[38;5;242m# Interactively bind Telegram bot token\x1b[0m\n");

    println!("  \x1b[1;37mTips:\x1b[0m");
    println!("    \x1b[38;5;244mRun '\x1b[1;37mxiao <command> help\x1b[0m\x1b[38;5;244m' for subcommand details and interactive menus.\x1b[0m\n");
}
