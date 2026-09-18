use crate::ai::AIChatService;
use crate::cli::tui::get_terminal_bar_width;
use crate::{get_configured_owner_id, load_environment};

#[derive(Debug, PartialEq, Eq)]
pub enum MemoryCliAction<'a> {
    List,
    Clear,
    Remove(Option<&'a str>),
    Help,
    Unknown(&'a str),
}

pub fn parse_memory_cli_action<'a>(
    action: Option<&'a str>,
    target: Option<&'a str>,
) -> MemoryCliAction<'a> {
    match action {
        Some("help") | Some("--help") | Some("-h") => MemoryCliAction::Help,
        Some("clear") => MemoryCliAction::Clear,
        Some("rm") | Some("remove") | Some("delete") => MemoryCliAction::Remove(target),
        Some("list") | None => MemoryCliAction::List,
        Some(unknown) => MemoryCliAction::Unknown(unknown),
    }
}

pub(crate) async fn run_cli_memory(
    _ai_service: &AIChatService,
    action: Option<&str>,
    target: Option<&str>,
) {
    load_environment();

    let parsed = parse_memory_cli_action(action, target);
    if parsed == MemoryCliAction::Help {
        println!("\n\x1b[1;36mxiao memory — Tier-1 Long-Term Memory Management\x1b[0m\n");
        println!("\x1b[1;37mUsage:\x1b[0m");
        println!("  xiao memory [action] [target]\n");
        println!("\x1b[1;37mSubcommands for 'memory':\x1b[0m");
        println!("     \x1b[36mxiao memory\x1b[0m                 List remembered long-term facts (default)");
        println!(
            "     \x1b[36mxiao memory rm <key>\x1b[0m        Remove a specific remembered fact"
        );
        println!("     \x1b[36mxiao memory clear\x1b[0m           Wipe all remembered facts for the owner\n");
        return;
    }

    let owner_id = get_configured_owner_id().unwrap_or(0);
    if owner_id == 0 {
        println!("\n\x1b[31m✖ OWNER_USER_ID belum dikonfigurasi. Jalankan 'xiao gateway owner <ID>'.\x1b[0m\n");
        std::process::exit(1);
    }

    match parsed {
        MemoryCliAction::Clear => {
            if crate::ai::storage::clear_user_memories_async(owner_id).await {
                println!("\n\x1b[1;32m✔ Semua memori jangka panjang (Tier 1) untuk Owner ({owner_id}) berhasil dihapus.\x1b[0m\n");
            } else {
                println!("\n\x1b[31m✖ Gagal membersihkan memori pengguna.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        MemoryCliAction::Remove(Some(key)) => {
            if crate::ai::storage::delete_user_memory_async(owner_id, key.to_string()).await {
                println!("\n\x1b[1;32m✔ Memori '{key}' berhasil dihapus untuk Owner ({owner_id}).\x1b[0m\n");
            } else {
                println!("\n\x1b[31m✖ Gagal menghapus memori '{key}'.\x1b[0m\n");
                std::process::exit(1);
            }
        }
        MemoryCliAction::Remove(None) => {
            println!("\n\x1b[31m✖ Error: Parameter <key> diperlukan.\x1b[0m");
            println!("  Penggunaan: xiao memory rm <key>\n");
            std::process::exit(1);
        }
        MemoryCliAction::List => {
            let memories = crate::ai::storage::get_user_memories_async(owner_id).await;
            println!("\n\x1b[1;36m== Xiao Long-Term Memory (Tier 1 Facts) ==\x1b[0m");
            println!("  \x1b[38;5;245mOwner ID :\x1b[0m \x1b[1;37m{owner_id}\x1b[0m");
            println!(
                "  \x1b[38;5;245mTotal    :\x1b[0m \x1b[1;37m{} facts remembered\x1b[0m\n",
                memories.len()
            );

            if memories.is_empty() {
                println!("  \x1b[38;5;244m(Belum ada fakta yang tersimpan. Xiao akan mengingat fakta penting secara otomatis saat Anda mengobrol.)\x1b[0m\n");
            } else {
                let bar_width = get_terminal_bar_width();
                println!("  \x1b[1;37m{:<25} Remembered Fact\x1b[0m", "Key / Topic");
                println!(
                    "  \x1b[38;5;238m{}\x1b[0m",
                    "─".repeat(bar_width.saturating_sub(4))
                );
                for (key, fact) in memories {
                    println!(
                        "  \x1b[1;38;5;45m{:<25}\x1b[0m \x1b[38;5;252m{}\x1b[0m",
                        key, fact
                    );
                }
                println!(
                    "  \x1b[38;5;238m{}\x1b[0m",
                    "─".repeat(bar_width.saturating_sub(4))
                );
                println!("  \x1b[38;5;244mKelola: 'xiao memory rm <key>' atau 'xiao memory clear'\x1b[0m\n");
            }
        }
        MemoryCliAction::Help => unreachable!(),
        MemoryCliAction::Unknown(unknown) => {
            println!("\n\x1b[31m✖ Error: Aksi '{unknown}' tidak dikenal.\x1b[0m");
            println!("  Penggunaan: xiao memory [list|rm <key>|clear]\n");
            std::process::exit(1);
        }
    }
}
