pub(crate) fn print_cli_help() {
    println!(
        "\n\x1b[1;36mxiao v{} — AI Assistant Bot\x1b[0m\n",
        env!("CARGO_PKG_VERSION")
    );
    println!("\x1b[1;37mUsage:\x1b[0m");
    println!("  xiao <command>\n");
    println!("\x1b[1;37mCommands:\x1b[0m");
    println!("  \x1b[36mstart\x1b[0m               Run bot daemon (default)");
    println!("  \x1b[36mchat [prompt]\x1b[0m       Direct terminal chat mode (Interactive REPL or one-shot prompt)");
    println!(
        "  \x1b[36msetup\x1b[0m               Interactive initial setup wizard (AI -> Telegram)"
    );
    println!("  \x1b[36mstatus\x1b[0m              Display system, database, and provider status dashboard");
    println!("  \x1b[36mcontext [chat] [th]\x1b[0m  Display token consumption and context gauge (default: owner private chat)");
    println!("  \x1b[36mmemory [action]\x1b[0m     Manage long-term user memories (list, rm <key>, clear)");
    println!("  \x1b[36mai [action]\x1b[0m         Unified AI management hub (Model, Provider, Addon) [Interactive/One-Liner]");
    println!("  \x1b[36mmcp [action]\x1b[0m        Manage Model Context Protocol search endpoints [status, url, test, reset]");
    println!("  \x1b[36mgateway [action]\x1b[0m    Manage Telegram messaging gateway (Token & Owner ID) [Interactive/One-Liner]");
    println!("  \x1b[36mversion, -v\x1b[0m         Display binary version");
    println!("  \x1b[36mhelp\x1b[0m                Show this help message\n");
    println!("\x1b[1;37mUsage for 'chat':\x1b[0m");
    println!("     \x1b[36mxiao chat\x1b[0m               Start interactive terminal chat session");
    println!(
        "     \x1b[36mxiao chat <prompt>\x1b[0m      Send one-shot query and print response\n"
    );
    println!("\x1b[1;37mSubcommands for 'memory':\x1b[0m");
    println!(
        "     \x1b[36mxiao memory\x1b[0m                 List remembered long-term facts (default)"
    );
    println!("     \x1b[36mxiao memory rm <key>\x1b[0m        Remove a specific remembered fact");
    println!(
        "     \x1b[36mxiao memory clear\x1b[0m           Wipe all remembered facts for the owner\n"
    );
    println!("\x1b[1;37mSubcommands for 'ai':\x1b[0m");
    println!("     \x1b[36mxiao ai\x1b[0m             Open Interactive AI Center Hub");
    println!("     \x1b[36mxiao ai use <model>\x1b[0m Switch Main Model directly");
    println!(
        "     \x1b[36mxiao ai list\x1b[0m        Print table of registered providers and models"
    );
    println!(
        "     \x1b[36mxiao ai [add|rm]\x1b[0m    Add or remove an OpenAI-compatible AI provider"
    );
    println!("     \x1b[36mxiao ai addon\x1b[0m       Configure multimodal specialist routes (Vision, STT, Video, Image)");
    println!("     \x1b[36mxiao ai test [role]\x1b[0m Open live diagnostic probe center (or test: vision, stt, video, image, all)\n");
    println!("\x1b[1;37mSubcommands for 'mcp':\x1b[0m");
    println!("     \x1b[36mxiao mcp\x1b[0m                    Display active search engine status and provider keys");
    println!("     \x1b[36mxiao mcp url <URL>\x1b[0m          Set custom MCP server endpoint (SSRF protected)");
    println!("     \x1b[36mxiao mcp test [query]\x1b[0m       Probe Exa MCP server with a live test query");
    println!("     \x1b[36mxiao mcp search <query>\x1b[0m     Test end-to-end web search pipeline with active engine");
    println!("     \x1b[36mxiao mcp tools\x1b[0m              List registered function calling tools and schemas");
    println!(
        "     \x1b[36mxiao mcp brave [KEY|rm]\x1b[0m     Configure or remove Brave Search API key"
    );
    println!(
        "     \x1b[36mxiao mcp tavily [KEY|rm]\x1b[0m    Configure or remove Tavily Search API key"
    );
    println!(
        "     \x1b[36mxiao mcp exa [KEY|rm]\x1b[0m       Configure or remove Exa REST API key"
    );
    println!("     \x1b[36mxiao mcp reset\x1b[0m              Reset MCP endpoint to default (https://mcp.exa.ai/)\n");
    println!("\x1b[1;37mSubcommands for 'gateway':\x1b[0m");
    println!("     \x1b[36mxiao gateway\x1b[0m                Open Interactive Gateway Manager");
    println!(
        "     \x1b[36mxiao gateway check\x1b[0m          Verify bot token connectivity (getMe)"
    );
    println!("     \x1b[36mxiao gateway token <TOKEN>\x1b[0m  Bind and verify Telegram Bot Token");
    println!("     \x1b[36mxiao gateway owner <ID>\x1b[0m     Set Telegram Owner User ID\n");
}
