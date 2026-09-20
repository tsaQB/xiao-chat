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
        println!("\n\x1b[33m⚠ No AI Provider configured yet.\x1b[0m");
        println!("\x1b[38;5;244mLaunching Setup Wizard for initial configuration...\x1b[0m\n");
        let _ = run_cli_quickstart_wizard(ai_service).await;
        if !ai_service.has_configured_provider(0).await {
            println!("\n\x1b[31m✖ Setup cancelled. No active provider for chat.\x1b[0m\n");
            return;
        }
    }

    let main_route = match ai_service.resolve_model_route(ModelRole::Main).await {
        Ok(r) => r,
        Err(e) => {
            println!("\n\x1b[31m✖ Error: Main Model not available: {e}\x1b[0m\n");
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
    let pkg_ver = env!("CARGO_PKG_VERSION");
    let title_left = "  \x1b[48;2;15;23;42m\x1b[38;2;16;185;129m 「 小 」 \x1b[0m  \x1b[1;37mxiao › Terminal Chat REPL\x1b[0m";
    let title_left_vis = 2 + 7 + 2 + 25;
    let ver_str = format!("v{pkg_ver}");
    let ver_vis = crate::cli::tui::visible_width(&ver_str);
    let pad = bar_width.saturating_sub(title_left_vis + ver_vis + 2);
    println!(
        "\r\n{title_left}{}\x1b[38;5;244m{ver_str}\x1b[0m\r\n  \x1b[38;5;238m{}\x1b[0m\r\n",
        " ".repeat(pad),
        "─".repeat(bar_width.saturating_sub(4))
    );

    let active_session = ai_service.get_active_session(user_id).await;
    let sess_str = if let Some(sess) = &active_session {
        format!(
            "● #{} — {} ({} messages)",
            sess.id,
            sess.name,
            sess.messages.len()
        )
    } else {
        "● #1 — Default Chat (0 messages)".to_string()
    };
    let model_str = format!("● {model_name} ({provider_name})");
    let cmd_str = "/help · /new · /sessions · /switch · /clear · /exit";

    let hud_rows = [
        ("MAIN MODEL", model_str.as_str()),
        ("CHAT SESSION", sess_str.as_str()),
        ("REPL COMMANDS", cmd_str),
    ];
    let hud = crate::cli::tui::render_hud_box("ACTIVE CHAT SESSION", &hud_rows, bar_width);
    println!("{hud}\r\n");

    let stdin = io::stdin();
    loop {
        print!("  \x1b[1;38;5;45mYou ▸ \x1b[0m");
        let _ = io::stdout().flush();

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(0) => {
                println!("\n\x1b[38;5;244mChat finished.\x1b[0m\n");
                break;
            }
            Err(_) => {
                println!("\n\x1b[38;5;244mChat finished.\x1b[0m\n");
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
                    println!("\x1b[38;5;244mGoodbye!\x1b[0m\n");
                    break;
                }
                ChatCliCommand::Clear => {
                    if ai_service.clear_history(user_id).await {
                        println!(
                            "\x1b[1;32m✔ Conversation history for this session has been cleared.\x1b[0m\n"
                        );
                    } else {
                        println!("\x1b[31m✖ Failed to clear session history.\x1b[0m\n");
                    }
                    continue;
                }
                ChatCliCommand::Sessions => {
                    let sessions = ai_service.get_sessions(user_id).await;
                    let active_id = ai_service.get_active_session_id(user_id).await.unwrap_or(0);
                    println!("\n\x1b[1;37mConversation Sessions:\x1b[0m");
                    for s in &sessions {
                        let marker = if s.id == active_id {
                            "\x1b[1;32m[x]\x1b[0m"
                        } else {
                            "\x1b[38;5;244m[ ]\x1b[0m"
                        };
                        let act_label = if s.id == active_id {
                            " \x1b[1;32m(Active)\x1b[0m"
                        } else {
                            ""
                        };
                        println!(
                            "  {} #{:<2} — {:<24} \x1b[38;5;244m({} messages, {}){}\x1b[0m",
                            marker,
                            s.id,
                            s.name,
                            s.messages.len(),
                            s.created_at,
                            act_label
                        );
                    }
                    println!("\x1b[38;5;244mUse '/switch <id>' to switch sessions, '/rm <id>' to remove, or '/new [name]' to create a new one.\x1b[0m\n");
                    continue;
                }
                ChatCliCommand::Switch(target_id) => {
                    if ai_service.switch_session_by_id(user_id, target_id).await {
                        if let Some(s) = ai_service.get_active_session(user_id).await {
                            println!(
                                "\x1b[1;32m✔ Switched to session #{}: {}\x1b[0m\n",
                                s.id, s.name
                            );
                        } else {
                            println!("\x1b[1;32m✔ Switched to session #{}\x1b[0m\n", target_id);
                        }
                    } else {
                        println!("\x1b[31m✖ Session #{} not found.\x1b[0m\n", target_id);
                    }
                    continue;
                }
                ChatCliCommand::Remove(target_id) => {
                    if ai_service.remove_session_by_id(user_id, target_id).await {
                        println!(
                            "\x1b[1;32m✔ Session #{} successfully deleted.\x1b[0m\n",
                            target_id
                        );
                    } else {
                        println!(
                            "\x1b[31m✖ Session #{} not found or failed to delete.\x1b[0m\n",
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
                            "\x1b[1;32m✔ New session created: #{} — {}\x1b[0m\n",
                            new_sess.id, new_sess.name
                        );
                    } else {
                        println!("\x1b[31m✖ Failed to create new session.\x1b[0m\n");
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
                    println!("\x1b[1;37mAvailable Chat Commands:\x1b[0m");
                    println!("  \x1b[36m/clear\x1b[0m          - Clear active session history");
                    println!(
                        "  \x1b[36m/new [name]\x1b[0m     - Create and activate a new session"
                    );
                    println!("  \x1b[36m/sessions\x1b[0m       - List all conversation sessions");
                    println!(
                        "  \x1b[36m/switch [id]\x1b[0m    - Switch session (interactive if no id)"
                    );
                    println!(
                        "  \x1b[36m/rm <id>\x1b[0m        - Delete conversation session by ID"
                    );
                    println!(
                        "  \x1b[36m/model\x1b[0m          - Show active model and provider info"
                    );
                    println!("  \x1b[36m/help\x1b[0m           - Show command help");
                    println!(
                        "  \x1b[36m/exit\x1b[0m           - Exit chat mode (or Ctrl+C / Ctrl+D)\n"
                    );
                    continue;
                }
                ChatCliCommand::Unknown(cmd_str) => {
                    let lower_cmd = cmd_str.to_lowercase();
                    if lower_cmd.trim() == "/switch" && io::stdout().is_terminal() {
                        let sessions = ai_service.get_sessions(user_id).await;
                        let active_id =
                            ai_service.get_active_session_id(user_id).await.unwrap_or(0);
                        let mut items: Vec<String> = sessions
                            .iter()
                            .map(|s| {
                                let marker = if s.id == active_id { " [ACTIVE]" } else { "" };
                                format!(
                                    "#{:<2} — {:<24} ({} msgs){}",
                                    s.id,
                                    s.name,
                                    s.messages.len(),
                                    marker
                                )
                            })
                            .collect();
                        items.push("Cancel / Back".to_string());
                        let sel = crate::cli::tui::terminal_interactive_select(
                            "Select conversation session to activate:",
                            &items,
                            0,
                            false,
                            None,
                        );
                        if let Some(idx) = sel {
                            if idx < sessions.len() {
                                let target_id = sessions[idx].id;
                                if ai_service.switch_session_by_id(user_id, target_id).await {
                                    println!(
                                        "\x1b[1;32m✔ Switched to session #{}: {}\x1b[0m\n",
                                        target_id, sessions[idx].name
                                    );
                                }
                            }
                        }
                    } else if lower_cmd.starts_with("/switch") {
                        println!(
                            "\x1b[33mUsage: /switch <session_id> (example: /switch 1)\x1b[0m\n"
                        );
                    } else if lower_cmd.starts_with("/rm") || lower_cmd.starts_with("/delete") {
                        println!("\x1b[33mUsage: /rm <session_id> (example: /rm 2)\x1b[0m\n");
                    } else {
                        println!(
                            "\x1b[33mUnknown command '{cmd_str}'. Type /help for assistance.\x1b[0m\n"
                        );
                    }
                    continue;
                }
            }
        }

        execute_cli_chat_turn(ai_service, user_id, trimmed, &model_name, true).await;
    }
}

pub fn render_reasoning_box(thinking: &str, bar_width: usize) -> String {
    let card_inner = bar_width.saturating_sub(6);
    let header = "╭─ Reasoning / Thinking ";
    let header_vis: usize = 24;
    let rem_top = card_inner.saturating_sub(header_vis.saturating_sub(1));
    let top = format!(
        "  \x1b[38;5;244m{header}{}\x1b[38;5;244m╮\x1b[0m",
        "─".repeat(rem_top)
    );

    let max_text_width = card_inner.saturating_sub(4);

    let mut lines = Vec::new();
    lines.push(top);

    for raw_line in thinking.trim().lines() {
        let trimmed_line = raw_line.trim();
        if trimmed_line.is_empty() {
            lines.push(format!(
                "  \x1b[38;5;244m│\x1b[0m  {:<w$}  \x1b[38;5;244m│\x1b[0m",
                "",
                w = max_text_width
            ));
            continue;
        }

        let words: Vec<&str> = trimmed_line.split_whitespace().collect();
        let mut cur_line = String::new();
        for word in words {
            let cur_vis = crate::cli::tui::visible_width(&cur_line);
            let word_vis = crate::cli::tui::visible_width(word);

            if cur_line.is_empty() {
                if word_vis > max_text_width {
                    let mut start = 0;
                    while start < word.len() {
                        let end = (start + max_text_width).min(word.len());
                        let slice = &word[start..end];
                        let slice_vis = crate::cli::tui::visible_width(slice);
                        let pad = max_text_width.saturating_sub(slice_vis);
                        if start + max_text_width < word.len() {
                            lines.push(format!(
                                "  \x1b[38;5;244m│\x1b[0m  \x1b[38;5;250m{}{}\x1b[0m  \x1b[38;5;244m│\x1b[0m",
                                slice, " ".repeat(pad)
                            ));
                        } else {
                            cur_line = slice.to_string();
                        }
                        start = end;
                    }
                } else {
                    cur_line = word.to_string();
                }
            } else if cur_vis + 1 + word_vis <= max_text_width {
                cur_line.push(' ');
                cur_line.push_str(word);
            } else {
                let pad = max_text_width.saturating_sub(cur_vis);
                lines.push(format!(
                    "  \x1b[38;5;244m│\x1b[0m  \x1b[38;5;250m{}{}\x1b[0m  \x1b[38;5;244m│\x1b[0m",
                    cur_line,
                    " ".repeat(pad)
                ));
                if word_vis > max_text_width {
                    let mut start = 0;
                    while start < word.len() {
                        let end = (start + max_text_width).min(word.len());
                        let slice = &word[start..end];
                        let slice_vis = crate::cli::tui::visible_width(slice);
                        let pad = max_text_width.saturating_sub(slice_vis);
                        if start + max_text_width < word.len() {
                            lines.push(format!(
                                "  \x1b[38;5;244m│\x1b[0m  \x1b[38;5;250m{}{}\x1b[0m  \x1b[38;5;244m│\x1b[0m",
                                slice, " ".repeat(pad)
                            ));
                        } else {
                            cur_line = slice.to_string();
                        }
                        start = end;
                    }
                } else {
                    cur_line = word.to_string();
                }
            }
        }
        if !cur_line.is_empty() {
            let cur_vis = crate::cli::tui::visible_width(&cur_line);
            let pad = max_text_width.saturating_sub(cur_vis);
            lines.push(format!(
                "  \x1b[38;5;244m│\x1b[0m  \x1b[38;5;250m{}{}\x1b[0m  \x1b[38;5;244m│\x1b[0m",
                cur_line,
                " ".repeat(pad)
            ));
        }
    }

    let bot = format!("  \x1b[38;5;244m╰{}╯\x1b[0m", "─".repeat(card_inner));
    lines.push(bot);

    lines.join("\r\n")
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
                    "\r\x1b[38;5;81m{}\x1b[0m \x1b[38;5;244mXiao is processing... ({:.1}s)\x1b[0m",
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
                println!("\r\x1b[33m⚠ Request cancelled.\x1b[0m\n");
                return;
            }

            if is_tty {
                if let Some(think) = thinking.filter(|t| !t.trim().is_empty()) {
                    let bar_width = get_terminal_bar_width();
                    println!("{}\n", render_reasoning_box(&think, bar_width));
                }

                let rendered = crate::parser::render_terminal_markdown(&answer);
                let rendered_trimmed = rendered.trim();

                if rendered_trimmed.contains('\n')
                    || rendered_trimmed.contains("┌─")
                    || rendered_trimmed.contains("▌")
                {
                    println!("  \x1b[1;38;2;16;185;129mXiao ▸\x1b[0m\n{}", rendered_trimmed);
                } else {
                    println!("  \x1b[1;38;2;16;185;129mXiao ▸\x1b[0m {}", rendered_trimmed);
                }

                if interactive {
                    println!("\n  \x1b[38;5;243m[{:.1}s • {}]\x1b[0m\n", elapsed, model_name);
                } else {
                    println!("\n  \x1b[38;5;243m[{:.1}s • {}]\x1b[0m", elapsed, model_name);
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
            println!("\r\x1b[33m⚠ Request cancelled by user (Ctrl+C).\x1b[0m\n");
        }
    }
}
