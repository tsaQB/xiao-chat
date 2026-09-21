<div align="center">

```
██╗  ██╗██╗ █████╗  ██████╗ 
╚██╗██╔╝██║██╔══██╗██╔═══██╗
 ╚███╔╝ ██║███████║██║   ██║
 ██╔██╗ ██║██╔══██║██║   ██║
██╔╝ ██╗██║██║  ██║╚██████╔╝
╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝ 
```

### Xiao (小)
**A hardened, single-owner AI assistant for Telegram built with Rust.**  
*Engineered for Telegram Bot API 10.3 • Durable SQLite Intake Queue • Three-Tier Memory • Multimodal Routing*

---

[![CI](https://github.com/tsaQB/xiao-chat/actions/workflows/build.yml/badge.svg)](https://github.com/tsaQB/xiao-chat/actions)
![Telegram Bot API](https://img.shields.io/badge/Telegram%20Bot%20API-10.3-2CA5E0?logo=telegram&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-2021%20Edition-DEA584?logo=rust&logoColor=white)
![MSRV](https://img.shields.io/badge/MSRV-1.80%2B-lightgrey)
![Platforms](https://img.shields.io/badge/Platforms-Linux%20%7C%20Armbian%20%7C%20Termux-097ABB?logo=linux&logoColor=white)
![License](https://img.shields.io/badge/License-MIT-green.svg)

[Key Features](#-key-features) • [Architecture](#-architecture) • [Quickstart](#-quickstart) • [Installation](#-multi-platform-installation) • [CLI & Terminal Chat](#-cli--terminal-chat) • [Configuration](#-configuration-reference)

---

</div>

## 🌟 Highlights

Xiao is an autonomous, single-owner AI gateway designed to run continuously on low-overhead environments—from cloud servers to single-board computers (Armbian) and edge smartphones (Android Termux). It treats Telegram not as a simple chat wrapper, but as a rich display surface powered by **Telegram Bot API 10.3**.

- **Telegram Bot API 10.3 Native**: Real-time streaming drafts (`sendRichMessageDraft`), native stop controls, AST layout blocks (tables, expandable quotes, collages, slideshows, buttons, thinking indicators), and cross-platform LaTeX rendering.
- **Hardened Single-Owner Boundary**: Zero information leakage. Non-owner updates are dropped silently at the network boundary without acknowledging bot existence.
- **Pure Zero-Slash Gateway**: Runs with empty command menus (`set_my_commands(&[])`). Interacts naturally through conversational intent, context-aware mentions, media attachments, or dedicated forum topics.
- **Durable SQLite WAL Intake Queue**: Ingests updates to an ACID SQLite inbox (`telegram_inbox`) before acknowledgment. Dispatches to per-scope FIFO mailboxes with task-level panic isolation, bounded retry (2 attempts), and poison-pill quarantine.
- **Three-Tier Long-Term Memory**: Tier 1 (Autonomous profile facts), Tier 2 (Sliding-window topic summaries), and Tier 3 (Full thread-scoped turns).
- **Specialist Context Isolation**: Routes queries across `Main`, `Vision`, `Video`, `Audio STT`, `Image Generation`, and `Curator`. Specialist models only receive transient media payloads, preventing token context exhaustion and preserving privacy.
- **In-Memory Document & Anti-Bomb Inspection**: Safe extraction of PDF, DOCX, XLSX, text, code, and archives (ZIP, TAR, 7Z) with strict memory quotas and anti-zip-bomb limits.
- **Autonomous Tool Calling**: Built-in `web_search` (keyless Exa MCP protocol, Tavily, Brave, and DuckDuckGo fallbacks) and SSRF-hardened `fetch_url`.

---

## 🏛️ Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                       xiao CLI / Daemon                     │
└──────────────┬───────────────────────────────┬──────────────┘
               │ (Telegram Long Polling)       │ (Direct Terminal REPL)
               ▼                               │
┌───────────────────────────────┐              │
│      TelegramBotClient        │              │
│   (SSRF Firewall & Fallback)  │              │
└──────────────┬────────────────┘              │
               ▼                               │
┌───────────────────────────────┐              │
│     Durable Intake Queue      │              │
│    (SQLite WAL telegram_inbox)│              │
└──────────────┬────────────────┘              │
               ▼                               │
┌───────────────────────────────┐              │
│    ChatRouteScope Evaluator   │              │
│  (Hard Single-Owner Gateway)  │              │
└──────────────┬────────────────┘              ▼
               │                  ┌───────────────────────────┐
               └─────────────────►│       AIChatService       │
                                  └─────────────┬─────────────┘
                                                │
         ┌──────────────────────────────────────┴──────────────────────────────────────┐
         ▼                                                                             ▼
┌───────────────────────────────────┐                         ┌───────────────────────────────────┐
│         Model Role Router         │                         │         Three-Tier Memory         │
│  ├─ Main (Canonical History)      │                         │  ├─ Tier 1: User Profile Facts    │
│  ├─ Vision (Transient Media)      │                         │  ├─ Tier 2: Scoped Topic Summaries│
│  ├─ Video (Bounded Frames)        │                         │  └─ Tier 3: Scoped Canonical Turns│
│  ├─ Audio STT (Whisper Stream)    │                         └───────────────────────────────────┘
│  ├─ Image Gen (OpenAI/Pollinations)                         │         SQLite WAL Storage        │
│  └─ Curator (Fact Extraction)     │                         │      (~/.local/share/xiaoai/)     │
└───────────────────────────────────┘                         └───────────────────────────────────┘
```

---

## 🚀 Key Features

### 1. 🎨 Native Telegram Bot API 10.3 Engine
- **Streaming Draft Synchronization**: Responses stream live into Telegram drafts using `sendRichMessageDraft` and `sendMessageDraft`, complete with rotating progress spinners and activity indicators.
- **Real-Time Stop Controls**: Interrupt generation at any time with native Telegram stop actions (`stopped_message_generation`). Stop signals bypass worker queues for instant in-flight cancellation.
- **Rich Message AST**: Parses extended Markdown directly into Telegram Bot API 10.3 rich blocks:
  - **Tables**: Multi-column tables with custom text alignments. Unspecified columns in Right-to-Left (Arabic/Hebrew) or Eastern Arabic numeral contexts automatically mirror to the right.
  - **Expandable Quotes & Callouts**: GitHub-style alerts (`> [!NOTE]`, `> [!WARNING]`) and expandable blockquotes.
  - **Visual Media Groups**: Native photo collages and horizontal media slideshows.
  - **Interactive Action Buttons**: Copy-text buttons, URLs, and ephemeral interactive buttons.
- **Cross-Platform LaTeX Sanitizer**: Mathematical expressions (`$...$`, `$$...$$`, `\(...\)`) are sanitized downstream to render flawlessly on both Android (`JLaTeXMath`) and iOS (`SwiftMath`), normalizing units (`44\ \mathrm{cm}`), decimal commas, and LaTeX operator symbols.

### 2. 🛡️ Hardened Security & Zero-Slash UX
- **Strict Single-Owner Boundary**: Xiao ignores all non-owner interactions at the network gate. Messages from unauthorized users or rogue groups are discarded with `RouteDecision::Ignore`, leaking zero information about the bot's presence.
- **Zero-Slash Gateway**: On startup, Xiao executes `set_my_commands(&[])` to clear Telegram slash menus. Interaction is completely natural—speak naturally, drop files, send voice messages, or mention the bot.
- **Filesystem Secret Vault**: Plaintext API keys and bot tokens are never stored in SQLite or environment dumps. They are isolated in permission-hardened local files (`~/.local/share/xiaoai/secrets/`, permissions `0o600`/`0o700`) and referenced internally via opaque `secret://` URIs.
- **Outbound SSRF Firewall**: Remote fetches (`fetch_url`, media downloads) pass through strict IP validation, blocking RFC 1918 private subnets, loopback addresses (`127.0.0.0/8`, `::1`), link-local spaces, and SIIT/NAT64 translated ranges.

### 3. 🧠 Three-Tier Memory & Multimodal Routing
- **Tier 1 (Facts)**: Autonomous background analysis extracts long-term user facts, preferences, and technical stack details into `user_memories`.
- **Tier 2 (Topic Summaries)**: Older conversation turns in busy chats or forum topics are periodically condensed into concise topic summaries (`scoped_summaries`), keeping active context windows lean.
- **Tier 3 (Canonical Turns)**: Complete scoped message logs stored in SQLite WAL tables for exact replay and reference.
- **Specialist Context Isolation**: Canonically, conversation history belongs solely to the `Main` model. Specialists (`Vision`, `Video`, `Audio STT`) only receive the immediate media artifact and user prompt, returning bounded observation turns to `Main`. This eliminates context pollution and token exhaustion.

### 4. ⚡ Durable SQLite WAL Queue & Fault Isolation
- **At-Least-Once Intake Guarantee**: Telegram long-polling commits updates directly to SQLite table `telegram_inbox` with status `pending` before Telegram acknowledgment.
- **Per-Scope Keyed Mailboxes**: Updates are dispatched into dedicated per-scope FIFO queues (`ScopeKey { chat_id, thread_id }`). Messages within the same chat/topic maintain strict sequential order, while different chats process concurrently up to a global semaphore limit (8 permits).
- **Panic Isolation & Bounded Retry**: Each processing task runs within an isolated `tokio::spawn` wrapper with unwinding panic protection. Transient panics release concurrency permits immediately and trigger a 1.5-second backoff sleep with up to 2 retry attempts. Updates exceeding retry limits are quarantined as `failed` ("poison pills") to prevent infinite daemon crash loops.
- **Crash Recovery**: On process startup, `recover_telegram_processing_async()` resets in-flight `processing` updates back to `pending`, ensuring zero message loss across reboots.

---

## 📦 Multi-Platform Installation

Xiao is distributed as a single static binary with pure Rust dependencies. Choose your deployment environment:

### Option A: Linux x86_64 Server (Systemd Service)

Ideal for dedicated servers, VPS instances (Ubuntu, Debian, Arch Linux), or home servers.

1. **Download the latest release binary**:
   ```bash
   sudo mkdir -p /usr/local/bin /var/lib/xiaoai
   # Download xiao binary from GitHub Releases / Actions to /usr/local/bin/xiao
   sudo chmod +x /usr/local/bin/xiao
   ```

2. **Create a dedicated system user**:
   ```bash
   sudo useradd -r -s /usr/sbin/nologin -d /var/lib/xiaoai xiao
   sudo chown -R xiao:xiao /var/lib/xiaoai
   sudo chmod 700 /var/lib/xiaoai
   ```

3. **Install the hardened Systemd service**:
   Create `/etc/systemd/system/xiao.service`:
   ```ini
   [Unit]
   Description=Xiao Telegram AI Assistant Daemon
   After=network.target network-online.target
   Wants=network-online.target

   [Service]
   Type=simple
   User=xiao
   Group=xiao
   WorkingDirectory=/var/lib/xiaoai
   ExecStart=/usr/local/bin/xiao start
   Restart=on-failure
   RestartSec=5s
   LimitNOFILE=65535

   # Security Sandboxing
   ProtectSystem=strict
   ProtectHome=read-only
   ReadWritePaths=/var/lib/xiaoai
   PrivateTmp=true
   NoNewPrivileges=true

   Environment=XIAO_DATA_DIR=/var/lib/xiaoai
   EnvironmentFile=-/var/lib/xiaoai/.env

   [Install]
   WantedBy=multi-user.target
   ```

4. **Configure credentials & start**:
   ```bash
   sudo cp .env.example /var/lib/xiaoai/.env
   sudo nano /var/lib/xiaoai/.env
   sudo chown xiao:xiao /var/lib/xiaoai/.env && sudo chmod 600 /var/lib/xiaoai/.env

   sudo systemctl daemon-reload
   sudo systemctl enable --now xiao
   sudo systemctl status xiao
   ```

---

### Option B: Armbian / Single-Board Computers (ARM64)

Optimized for Orange Pi, Raspberry Pi, Radxa, or any SBC running Armbian or Debian ARM64 (`aarch64-unknown-linux-gnu`).

1. **Download pre-built ARM64 binary**:
   GitHub Actions automatically compiles `xiao-linux-arm64-armbian` on every commit:
   ```bash
   sudo curl -L -o /usr/local/bin/xiao "<RELEASE_OR_ARTIFACT_URL>"
   sudo chmod +x /usr/local/bin/xiao
   ```

2. **Quick onboarding setup**:
   ```bash
   # Run the interactive onboarding wizard to configure bot token and AI provider
   xiao setup
   ```

3. **Enable systemd background service**:
   ```bash
   sudo systemctl enable --now xiao
   journalctl -u xiao -f
   ```

---

### Option C: Android Termux (Zero-Root Mobile Edge)

Run Xiao 24/7 directly on your Android device without requiring root access.

1. **Install required packages in Termux**:
   ```bash
   pkg update && pkg install -y git clang rust openssl termux-api
   ```

2. **Clone & build native Android binary**:
   ```bash
   git clone https://github.com/tsaQB/xiao-chat.git
   cd xiao-chat
   cargo build --release --locked
   cp target/release/xiao $PREFIX/bin/
   ```

3. **Acquire Termux wake-lock & start daemon**:
   ```bash
   termux-wake-lock
   xiao setup
   xiao start
   ```

---

### Option D: Build from Source

Requirements: **Rust 1.80+** (`cargo`, `rustc`).

```bash
git clone https://github.com/tsaQB/xiao-chat.git
cd xiao-chat

# Verify compilation
cargo check --locked

# Build optimized release binary
cargo build --release --locked

# Binary is available at target/release/xiao
./target/release/xiao --help
```

---

## ⚡ Quickstart

### 1. Configure Environment
Xiao reads configuration in the following order:
1. `.env` in the current working directory
2. `~/.xiao.env`
3. `~/xiao/.env`
4. `~/XiaoAI/.env`

Create `.env` based on the template:
```bash
cp .env.example .env
```

Edit the core settings:
```env
# Required Telegram Gateway Credentials
BOT_TOKEN=1234567890:ABCdefGHIjklMNOpqrsTUVwxyz
OWNER_USER_ID=5385399301

# Primary AI Provider (Defaults to OpenRouter, or any OpenAI-compatible /v1 endpoint)
AI_ENDPOINT=https://openrouter.ai/api/v1
AI_API_KEY=sk-or-v1-xxxxxxxxxxxxxxxxxxxx
AI_MODEL=google/gemini-2.0-flash-001
```

> [!TIP]
> **Zero-Prompt Headless Startup**: Populating `.env` allows `xiao start` to initialize the database, seed provider credentials, and run the Telegram long-polling loop with zero interactive prompts.
> If you prefer a visual onboarding walkthrough, simply run:
> ```bash
> cargo run -- setup
> ```

### 2. Start the Daemon
```bash
cargo run --release -- start
```

---

## 💻 CLI & Terminal Chat

Xiao includes a feature-rich terminal CLI for administration, diagnostics, and direct interactive chat without opening Telegram:

```bash
xiao <subcommand> [arguments]
```

### CLI Command Reference

| Subcommand | Description |
| :--- | :--- |
| *(none)* | Start terminal chat session (default mode; auto-launches setup on first run). |
| `<question...>` | Ask a quick one-shot question directly (e.g. `xiao "What is Rust?"`). |
| `menu` | Open interactive Control Center (TUI). |
| `chat [prompt]` | Terminal chat mode: run an interactive REPL or execute a one-shot query. |
| `setup` | Interactive initial configuration and onboarding wizard. |
| `start` | Start the Telegram polling daemon. |
| `status` | Display system status dashboard, SQLite database size, and active models. |
| `context [chat] [th]` | Inspect token usage, sliding-window consumption, and context breakdown. |
| `memory` | Inspect and manage Tier-1 persistent user profile memories. |
| `memory rm [key]` | Delete a remembered fact (interactive picker if key is omitted). |
| `memory clear` | Wipe all Tier-1 persistent memories. |
| `ai` | Interactive AI hub for model catalog, capability tests, and routing. |
| `ai use [model]` | Switch the active Main model (interactive picker if model is omitted). |
| `ai addon` | Configure multimodal specialist roles (`Vision`, `Video`, `Audio STT`, etc.). |
| `ai test [role]` | Run live diagnostic capability probes against configured endpoints. |
| `search [action]` | Web search engine hub and provider API keys (Brave, Tavily, Exa). |
| `mcp [action]` | Model Context Protocol (MCP) server registry and dynamic tool hub. |
| `gateway` | Inspect Telegram Bot Token connectivity, verify owner, and test polling. |
| `version` | Display version and target build metadata. |
| `help` | Display command-line help screen. |

### Terminal Interactive REPL
Launch direct chat mode without Telegram:
```bash
xiao
# Or with cargo:
cargo run
```

Inside the REPL, manage independent chat sessions seamlessly:
```text
Xiao Interactive Chat REPL Commands:
  /sessions         List all conversation sessions with IDs and turn counts
  /switch <id>      Switch to a different conversation session
  /new [name]       Create and switch to a new isolated session
  /rm <id>          Remove a conversation session
  /clear            Reset conversation history in active session
  /model            Inspect active Main model and provider endpoint
  /help             Show available REPL commands
  /exit             Exit chat mode (or Ctrl+C / Ctrl+D)
```

Run one-shot queries directly from shell scripts or terminal:
```bash
xiao "Analyze the concurrency guarantees of SQLite in WAL mode"
# Or:
xiao chat "Analyze the concurrency guarantees of SQLite in WAL mode"
```

---

## 📋 Document & Media Processing

Xiao inspects documents and rich media locally inside bounded memory buffers:

| Media Type | Processing & Ingestion Method |
| :--- | :--- |
| **Plain Text / Code** | UTF-8 sanitized ingestion (with UTF-8 BOM stripping and Latin-1 lossy fallback). |
| **Photos & Images** | Analyzed via `Vision` specialist role (JPEG, PNG, WEBP). |
| **Voice Notes & Audio** | Transcribed via `Audio STT` (Whisper-compatible `/v1/audio/transcriptions` endpoints). |
| **Video & Video Notes** | Bounded frame extraction processed via `Video` specialist role. |
| **PDF Documents** | Text extracted via `lopdf`. Scanned pages (up to 6) are rendered to images and routed to `Vision`. |
| **DOCX Documents** | In-memory XML text and paragraph extraction. |
| **XLSX Spreadsheets** | In-memory streaming XML parsing of sheets and shared string tables (`<si>`). |
| **Archives (ZIP, TAR, 7Z)** | In-memory text extraction with strict safety caps: max 30 MB uncompressed, max 2 MB per file, no recursive bomb unpacking. |

---

## ⚙️ Configuration Reference

Settings can be provided via `.env` or managed dynamically through the CLI:

| Environment Variable | Default | Description |
| :--- | :--- | :--- |
| `BOT_TOKEN` | *Required* | Telegram Bot API token from [@BotFather](https://t.me/BotFather). |
| `OWNER_USER_ID` | *Required* | Numerical Telegram user ID of the authorized owner. |
| `ALLOWED_CHAT_IDS` | *Empty* | Comma-separated list of guest group IDs permitted to use the bot. |
| `DEDICATED_CHAT_IDS` | *Empty* | Comma-separated list of forum supergroups acting as dedicated workspaces. |
| `AI_ENDPOINT` | `https://openrouter.ai/api/v1` | OpenAI-compatible completions API endpoint base URL. |
| `AI_API_KEY` | *Required* | Bearer authentication token for the AI endpoint. |
| `AI_MODEL` | `google/gemini-2.0-flash-001` | Default model identifier for general conversation. |
| `IMAGE_FALLBACK_PROVIDER` | `none` | Fallback provider for image generation (`none` or `pollinations`). |
| `AI_PROVIDER_CONNECT_TIMEOUT_SECS` | `10` | HTTP connect timeout for standard AI API requests. |
| `IMAGE_GENERATION_TIMEOUT_SECS` | `120` | Request timeout for image generation endpoints. |
| `IMAGE_DOWNLOAD_TIMEOUT_SECS` | `30` | Timeout for retrieving generated image payloads. |
| `BRAVE_API_KEY` | *Optional* | API key for Brave Search integration in `web_search`. |
| `TAVILY_API_KEY` | *Optional* | API key for Tavily AI search integration. |
| `EXA_API_KEY` | *Optional* | API key for Exa search integration. |
| `EXA_MCP_URL` | `https://mcp.exa.ai/` | Model Context Protocol search server endpoint. |
| `XIAO_DATA_DIR` | `~/.local/share/xiaoai` | Base filesystem directory for database, secrets, and attachments. |

---

## 🧪 Verification & Quality Gates

The codebase strictly enforces clean quality gates and zero `.unwrap()` calls across both production code and test suites:

```bash
# Code formatting check (CI enforced)
cargo fmt --all -- --check

# Compiler check without emitting binaries
cargo check --locked

# Unit tests and Telegram Bot API 10.3 contract suite (415+ tests)
cargo test --locked

# Strict Clippy lint check (CI enforced)
cargo clippy --locked --all-targets --all-features -- -D warnings
```

---

## 🔒 Security Invariants

1. **Hard Single-Owner Invariant**: Non-owner updates are dropped silently at the gateway (`src/bot/router.rs`). Never acknowledge unauthorized Telegram IDs.
2. **Pure Zero-Slash Gateway**: Menus are cleared on boot. Xiao interacts conversationally or via CLI.
3. **Outbound SSRF Firewall**: Remote downloads validate target IPs against RFC 1918, RFC 4193, loopback, and SIIT/NAT64 ranges.
4. **Secret Isolation**: Secrets are stored in `~/.local/share/xiaoai/secrets/` with mode `0o600`/`0o700`. Database only stores `secret://` URIs.
5. **Zero `.unwrap()` Policy**: Handled idiomatically with `?`, pattern matching, or `.expect()` with descriptive invariant explanations in tests.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
