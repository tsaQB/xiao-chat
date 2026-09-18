use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::ai::service::{GenerationInput, ModelRole};
use crate::ai::AIChatService;
use crate::cli::tui::get_terminal_bar_width;
use crate::cli::wizard::run_cli_quickstart_wizard;
use crate::{get_configured_owner_id, load_environment};

#[derive(Debug, PartialEq, Eq)]
pub enum ChatCliCommand<'a> {
    Exit,
    Clear,
    Sessions,
    Switch(usize),
    New(Option<&'a str>),
    Remove(usize),
    Model,
    Help,
    Unknown(&'a str),
}

pub fn parse_chat_cli_command(line: &str) -> Option<ChatCliCommand<'_>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if lower == "/exit" || lower == "/quit" || lower == "/q" {
        Some(ChatCliCommand::Exit)
    } else if lower == "/clear" || lower == "/reset" {
        Some(ChatCliCommand::Clear)
    } else if lower == "/sessions" {
        Some(ChatCliCommand::Sessions)
    } else if lower.starts_with("/switch") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(id) = parts[1].parse::<usize>() {
                Some(ChatCliCommand::Switch(id))
            } else {
                Some(ChatCliCommand::Unknown(trimmed))
            }
        } else {
            Some(ChatCliCommand::Unknown(trimmed))
        }
    } else if lower.starts_with("/rm") || lower.starts_with("/delete") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(id) = parts[1].parse::<usize>() {
                Some(ChatCliCommand::Remove(id))
            } else {
                Some(ChatCliCommand::Unknown(trimmed))
            }
        } else {
            Some(ChatCliCommand::Unknown(trimmed))
        }
    } else if lower.starts_with("/new") {
        let name = if trimmed.len() > 4 {
            let n = trimmed[4..].trim();
            if n.is_empty() {
                None
            } else {
                Some(n)
            }
        } else {
            None
        };
        Some(ChatCliCommand::New(name))
    } else if lower == "/model" || lower == "/models" {
        Some(ChatCliCommand::Model)
    } else if lower == "/help" {
        Some(ChatCliCommand::Help)
    } else {
        Some(ChatCliCommand::Unknown(trimmed))
    }
}

pub(crate) async fn run_cli_chat(ai_service: &AIChatService, initial_prompt: Option<String>) {
    load_environment();

    if !ai_service.has_configured_provider(0).await {
        println!("\n\x1b[33m⚠ Belum ada AI Provider yang terkonfigurasi.\x1b[0m");
        println!("\x1b[38;5;244mMenjalankan Setup Wizard untuk konfigurasi awal...\x1b[0m\n");
        let _ = run_cli_quickstart_wizard(ai_service).await;
        if !ai_service.has_configured_provider(0).await {
            println!("\n\x1b[31m✖ Setup dibatalkan. Tidak ada provider aktif untuk chat.\x1b[0m\n");
            return;
        }
    }

    let main_route = match ai_service.resolve_model_route(ModelRole::Main).await {
        Ok(r) => r,
        Err(e) => {
            println!("\n\x1b[31m✖ Error: Main Model tidak tersedia: {e}\x1b[0m\n");
            return;
        }
    };

    let model_name = main_route.model.clone();
    let provider_name = main_route.provider.name.clone();
    let user_id = get_configured_owner_id().unwrap_or(0);

    // Ensure session is initialized
    let _ = ai_service.get_sessions(user_id).await;

    // One-shot prompt mode
    if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
        execute_cli_chat_turn(ai_service, user_id, &prompt, &model_name, false).await;
        return;
    }

    // Interactive REPL mode
    let bar_width = get_terminal_bar_width();
    println!("\x1b[1;38;5;45m== Xiao Terminal Chat ==\x1b[0m");
    println!(
        " \x1b[38;5;245m• Model   :\x1b[0m \x1b[1;37m{} \x1b[38;5;244m({})\x1b[0m",
        model_name, provider_name
    );
    let active_session = ai_service.get_active_session(user_id).await;
    if let Some(sess) = active_session {
        println!(
            " \x1b[38;5;245m• Session :\x1b[0m \x1b[1;37m#{} — {}\x1b[0m \x1b[38;5;244m({} pesan)\x1b[0m",
            sess.id, sess.name, sess.messages.len()
        );
    }
    println!(
        " \x1b[38;5;245m• Bantuan :\x1b[0m \x1b[38;5;244mKetik pesan lalu tekan Enter. Perintah: /clear, /new, /sessions, /switch, /exit\x1b[0m"
    );
    println!("\x1b[38;5;238m{}\x1b[0m\n", "─".repeat(bar_width));

    let stdin = io::stdin();
    loop {
        print!("\x1b[1;38;5;81mYou ▸ \x1b[0m");
        let _ = io::stdout().flush();

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(0) => {
                println!("\n\x1b[38;5;244mChat selesai.\x1b[0m\n");
                break;
            }
            Err(_) => {
                println!("\n\x1b[38;5;244mChat selesai.\x1b[0m\n");
                break;
            }
            Ok(_) => {}
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(cmd) = parse_chat_cli_command(trimmed) {
            match cmd {
                ChatCliCommand::Exit => {
                    println!("\x1b[38;5;244mSampai jumpa!\x1b[0m\n");
                    break;
                }
                ChatCliCommand::Clear => {
                    if ai_service.clear_history(user_id).await {
                        println!(
                            "\x1b[1;32m✔ Riwayat percakapan sesi ini telah dibersihkan.\x1b[0m\n"
                        );
                    } else {
                        println!("\x1b[31m✖ Gagal membersihkan riwayat sesi.\x1b[0m\n");
                    }
                    continue;
                }
                ChatCliCommand::Sessions => {
                    let sessions = ai_service.get_sessions(user_id).await;
                    let active_id = ai_service.get_active_session_id(user_id).await.unwrap_or(0);
                    println!("\n\x1b[1;37mDaftar Sesi Percakapan:\x1b[0m");
                    for s in &sessions {
                        let marker = if s.id == active_id {
                            "\x1b[1;32m[x]\x1b[0m"
                        } else {
                            "\x1b[38;5;244m[ ]\x1b[0m"
                        };
                        let act_label = if s.id == active_id {
                            " \x1b[1;32m(Aktif)\x1b[0m"
                        } else {
                            ""
                        };
                        println!(
                            "  {} #{:<2} — {:<24} \x1b[38;5;244m({} pesan, {}){}\x1b[0m",
                            marker,
                            s.id,
                            s.name,
                            s.messages.len(),
                            s.created_at,
                            act_label
                        );
                    }
                    println!("\x1b[38;5;244mGunakan '/switch <id>' untuk berpindah sesi, '/rm <id>' untuk menghapus, atau '/new [nama]' untuk membuat baru.\x1b[0m\n");
                    continue;
                }
                ChatCliCommand::Switch(target_id) => {
                    if ai_service.switch_session_by_id(user_id, target_id).await {
                        if let Some(s) = ai_service.get_active_session(user_id).await {
                            println!("\x1b[1;32m✔ Beralih ke sesi #{}: {}\x1b[0m\n", s.id, s.name);
                        } else {
                            println!("\x1b[1;32m✔ Beralih ke sesi #{}\x1b[0m\n", target_id);
                        }
                    } else {
                        println!("\x1b[31m✖ Sesi #{} tidak ditemukan.\x1b[0m\n", target_id);
                    }
                    continue;
                }
                ChatCliCommand::Remove(target_id) => {
                    if ai_service.remove_session_by_id(user_id, target_id).await {
                        println!("\x1b[1;32m✔ Sesi #{} berhasil dihapus.\x1b[0m\n", target_id);
                    } else {
                        println!(
                            "\x1b[31m✖ Sesi #{} tidak ditemukan atau gagal dihapus.\x1b[0m\n",
                            target_id
                        );
                    }
                    continue;
                }
                ChatCliCommand::New(custom_name) => {
                    if let Some(new_sess) =
                        ai_service.create_new_session(user_id, custom_name).await
                    {
                        println!(
                            "\x1b[1;32m✔ Sesi baru berhasil dibuat: #{} — {}\x1b[0m\n",
                            new_sess.id, new_sess.name
                        );
                    } else {
                        println!("\x1b[31m✖ Gagal membuat sesi baru.\x1b[0m\n");
                    }
                    continue;
                }
                ChatCliCommand::Model => {
                    match ai_service.resolve_model_route(ModelRole::Main).await {
                        Ok(r) => {
                            println!(
                                "  \x1b[38;5;245mActive Model   :\x1b[0m \x1b[1;37m{}\x1b[0m",
                                r.model
                            );
                            println!(
                                "  \x1b[38;5;245mActive Provider:\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;244m({})\x1b[0m\n",
                                r.provider.name, r.provider.endpoint
                            );
                        }
                        Err(e) => {
                            println!("\x1b[31m✖ Error: {e}\x1b[0m\n");
                        }
                    }
                    continue;
                }
                ChatCliCommand::Help => {
                    println!("\x1b[1;37mPerintah Chat Tersedia:\x1b[0m");
                    println!(
                        "  \x1b[36m/clear\x1b[0m          - Bersihkan riwayat percakapan sesi aktif"
                    );
                    println!(
                        "  \x1b[36m/new [nama]\x1b[0m     - Buat dan aktifkan sesi percakapan baru"
                    );
                    println!(
                        "  \x1b[36m/sessions\x1b[0m       - Tampilkan daftar semua sesi percakapan"
                    );
                    println!(
                        "  \x1b[36m/switch <id>\x1b[0m    - Beralih ke sesi percakapan tertentu"
                    );
                    println!(
                        "  \x1b[36m/rm <id>\x1b[0m        - Hapus sesi percakapan berdasarkan ID"
                    );
                    println!("  \x1b[36m/model\x1b[0m          - Tampilkan informasi model dan provider aktif");
                    println!("  \x1b[36m/help\x1b[0m           - Tampilkan bantuan perintah");
                    println!("  \x1b[36m/exit\x1b[0m           - Keluar dari mode chat (atau Ctrl+C / Ctrl+D)\n");
                    continue;
                }
                ChatCliCommand::Unknown(cmd_str) => {
                    let lower_cmd = cmd_str.to_lowercase();
                    if lower_cmd.starts_with("/switch") {
                        println!(
                            "\x1b[33mPenggunaan: /switch <id_sesi> (contoh: /switch 1)\x1b[0m\n"
                        );
                    } else if lower_cmd.starts_with("/rm") || lower_cmd.starts_with("/delete") {
                        println!("\x1b[33mPenggunaan: /rm <id_sesi> (contoh: /rm 2)\x1b[0m\n");
                    } else {
                        println!(
                            "\x1b[33mPerintah '{cmd_str}' tidak dikenal. Ketik /help untuk bantuan.\x1b[0m\n"
                        );
                    }
                    continue;
                }
            }
        }

        execute_cli_chat_turn(ai_service, user_id, trimmed, &model_name, true).await;
    }
}

pub(crate) async fn execute_cli_chat_turn(
    ai_service: &AIChatService,
    user_id: i64,
    prompt: &str,
    model_name: &str,
    interactive: bool,
) {
    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
    let is_tty = io::stdout().is_terminal();

    let spinner_done = Arc::new(AtomicBool::new(false));
    let spinner_done_clone = spinner_done.clone();
    let spinner_handle = if is_tty {
        Some(tokio::spawn(async move {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut idx = 0;
            let start = std::time::Instant::now();
            while !spinner_done_clone.load(Ordering::Relaxed) {
                let elapsed = start.elapsed().as_secs_f64();
                print!(
                    "\r\x1b[38;5;81m{}\x1b[0m \x1b[38;5;244mXiao sedang memproses... ({:.1}s)\x1b[0m",
                    frames[idx % frames.len()],
                    elapsed
                );
                let _ = io::stdout().flush();
                idx = (idx + 1) % frames.len();
                tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;
            }
            print!("\r\x1b[2K");
            let _ = io::stdout().flush();
        }))
    } else {
        None
    };

    let generation_input = GenerationInput {
        prompt,
        canonical_prompt: None,
        media_to_main: false,
        sink: None,
        image_bytes: None,
        document_images: None,
        mime_type: None,
        doc_text: None,
        doc_name: None,
        audio_bytes: None,
        audio_mime: None,
        video_bytes: None,
        video_mime: None,
        video_duration: None,
    };

    let start = std::time::Instant::now();

    tokio::select! {
        res = ai_service.generate_response(user_id, 0, user_id, generation_input, &mut cancel_rx) => {
            spinner_done.store(true, Ordering::Relaxed);
            if let Some(handle) = spinner_handle {
                let _ = handle.await;
            }

            let (thinking, answer, cancelled) = res;
            let elapsed = start.elapsed().as_secs_f64();

            if cancelled {
                println!("\r\x1b[33m⚠ Permintaan dibatalkan.\x1b[0m\n");
                return;
            }

            if is_tty {
                if let Some(think) = thinking.filter(|t| !t.trim().is_empty()) {
                    println!("\x1b[38;5;242m┌─ Penalaran / Thinking ──────────────────────────\x1b[0m");
                    for line in think.trim().lines() {
                        println!("\x1b[38;5;242m│ {}\x1b[0m", line);
                    }
                    println!("\x1b[38;5;242m└────────────────────────────────────────────────\x1b[0m");
                }

                let rendered = crate::parser::render_terminal_markdown(&answer);
                let rendered_trimmed = rendered.trim();

                if rendered_trimmed.contains('\n')
                    || rendered_trimmed.contains("┌─")
                    || rendered_trimmed.contains("▌")
                {
                    println!("\x1b[1;38;5;81mXiao ▸\x1b[0m\n{}", rendered_trimmed);
                } else {
                    println!("\x1b[1;38;5;81mXiao ▸\x1b[0m {}", rendered_trimmed);
                }

                if interactive {
                    println!("\x1b[38;5;243m[{:.1}s • {}]\x1b[0m\n", elapsed, model_name);
                } else {
                    println!("\x1b[38;5;243m[{:.1}s • {}]\x1b[0m", elapsed, model_name);
                }
            } else {
                println!("{}", answer.trim());
            }
        }
        _ = tokio::signal::ctrl_c() => {
            spinner_done.store(true, Ordering::Relaxed);
            if let Some(handle) = spinner_handle {
                let _ = handle.await;
            }
            let _ = cancel_tx.send(true);
            println!("\r\x1b[33m⚠ Permintaan dibatalkan oleh pengguna (Ctrl+C).\x1b[0m\n");
        }
    }
}
