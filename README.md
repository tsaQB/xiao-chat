# Xiao (小)

A hardened, single-owner personal AI assistant for Telegram built in Rust targeting **Telegram Bot API 10.3**.

Xiao combines streaming draft responses, rich structured messages (AST blocks), a durable SQLite intake queue, three-tier long-term memory, and modular multimodal routing across any OpenAI-compatible provider.

---

## Highlights

- **Telegram Bot API 10.3 Native**: Implements streaming drafts (`sendRichMessageDraft`, `sendMessageDraft`), native stop controls (`stopped_message_generation`), bidirectional RTL layout support (`is_rtl`), and rich AST layout blocks (expandable blockquotes, multi-column tables with right-aligned RTL/Hindi number support, buttons, collages, slideshows, maps, and thinking blocks).
- **Hardened Single-Owner Security**: Strictly enforces `OWNER_USER_ID`. Non-owner messages and unauthorized groups are dropped silently at the network boundary without acknowledgment or leakage.
- **Zero-Slash Gateway**: Runs with empty command menus (`set_my_commands(&[])`). Interacts naturally through conversational intent, context-aware mentions, media attachments, or dedicated workspaces.
- **Modular Multi-Role Routing**: Separates roles for `Main`, `Vision`, `Video`, `Audio STT`, `Image Generation`, and `Curator`. Specialist models receive minimal transient payloads to prevent context pollution and token exhaustion.
- **Three-Tier Long-Term Memory**:
  - **Tier 1 (Facts)**: Background extraction of persistent user profile attributes (`user_memories`).
  - **Tier 2 (Topic Summary)**: Periodic summarization of older conversational turns per chat and forum thread (`scoped_summaries`).
  - **Tier 3 (Canonical Turns)**: Full thread-scoped message history (`messages`).
- **Durable Intake Queue & Per-Scope Mailboxes**: Ingests updates directly into an ACID SQLite inbox (`telegram_inbox`) before acknowledgment. Dispatches to per-scope FIFO mailboxes (`chat_id`, `thread_id`) with task-level panic isolation, bounded retry (2 attempts), and poison-pill quarantine.
- **In-Memory Document & Archive Inspection**: Safely reads plain text, source code, DOCX, XLSX, PDF (text and scanned page rendering for Vision), and archives (ZIP, TAR, TAR.GZ, 7Z) with strict memory quotas and anti-zip-bomb limits.
- **Tool Calling**: Built-in `web_search` (with multi-provider fallbacks across Brave, Tavily, Exa, and DuckDuckGo) and `fetch_url` with HTML sanitization.
- **Secret Isolation**: Sensitive credentials (bot tokens, AI keys) are stored in isolated, permission-hardened local files (`0o600`) and referenced internally via `secret://` identifiers.

---

## Architecture Overview

```
                          ┌───────────────────┐
                          │   Telegram Bot API    │
                          └─────────┬─────────┘
                                      │ (Long Polling)
                                      ▼
                          ┌────────────────────┐
                          │   TelegramBotClient    │
                          │   (SSRF Firewall)      │
                          └─────────┬──────────┘
                                      │
                                      ▼
                          ┌────────────────────┐
                          │ Durable Intake Queue   │
                          │ (SQLite WAL Inbox)     │
                          └─────────┬──────────┘
                                      │
                                      ▼
                          ┌────────────────────┐
                          │   ChatRouteScope       │
                          │ (Single-Owner Filter)  │
                          └─────────┬──────────┘
                                      │
                                      ▼
                          ┌────────────────────┐
                          │     AIChatService      │
                          └─────┬───────┬──────┘
                                 │         │
            ┌─────────────────┘        └─────────────────────┐
            ▼                                                        ▼
┌──────────────────────────┐                    ┌────────────────────┐
│       Model Role Router       │                    │   Three-Tier Memory    │
│ ├─ Main (Canonical History)   │                    │ ├─ User Memories      │
│ ├─ Vision (Transient Media)   │                    │ ├─ Topic Summaries    │
│ ├─ Video (Bounded Frames)     │                    │ └─ Canonical Messages │
│ ├─ Audio STT (Transcription)  │                    └────────────────────┘
│ ├─ Image Gen (OpenAI/Fallback)│
│ └─ Curator (Fact Extraction)  │
└──────────────────────────┘
```

---

## Prerequisites

- **Rust**: Version `1.80` or later (tested on Rust `1.98.0`).
- **Telegram Bot Token**: Created via [@BotFather](https://t.me/BotFather).
- **Telegram User ID**: Your numerical Telegram account ID.
- **OpenAI-Compatible AI Provider**: Any local or cloud endpoint offering a `/v1` interface (e.g., Ollama, vLLM, Groq, OpenRouter, OpenAI).

---

## Quickstart

### 1. Clone & Setup Environment

```bash
git clone https://github.com/Assaqib/xiao-chat.git
cd xiao-chat

# Copy sample configuration
cp .env.example .env
```

### 2. Configure Credentials

Edit `.env` with your editor of choice:

```env
# Required Telegram Settings
BOT_TOKEN=1234567890:ABCdefGHIjklMNOpqrsTUVwxyz
OWNER_USER_ID=987654321

# Default AI Provider (OpenRouter)
AI_ENDPOINT=https://openrouter.ai/api/v1
AI_API_KEY=sk-or-v1-...
AI_MODEL=google/gemini-2.0-flash-001
```

> [!TIP]
> **Zero-Prompt Headless Startup**: Populating `.env` allows `xiao start` to immediately auto-seed your AI provider and start the daemon with zero interactive prompts (ideal for Docker, systemd, or automated scripts).
> Alternatively, you can run the interactive onboarding wizard anytime to configure the gateway and test connections visually:
> ```bash
> cargo run -- setup
> ```

### 3. Run Xiao

```bash
# Start the Telegram bot daemon
cargo run --release -- start
```

---

## CLI Usage

Xiao includes an administrative and interactive CLI:

```bash
xiao <command> [arguments]
```

| Command | Description |
| :--- | :--- |
| `start` | Run the Telegram polling daemon (default). |
| `chat [prompt]` | Terminal chat mode: run an interactive REPL or execute a one-shot query without Telegram. |
| `setup` | Interactive initial configuration wizard. |
| `status` | Display status dashboard for SQLite database, active models, and providers. |
| `context [chat] [th]` | Inspect estimated token consumption and breakdown for a chat/thread. |
| `memory` | Manage Tier-1 user memories (`xiao memory`, `xiao memory rm <key>`, `xiao memory clear`). |
| `ai` | Interactive AI hub for provider management, model switching, and diagnostics. |
| `ai use <model>` | Switch the active Main model directly. |
| `ai addon` | Configure multimodal specialist roles (`Vision`, `Video`, `Audio STT`, `Image Generation`, `Curator`). |
| `ai test [role]` | Run live diagnostic capability probes against active endpoints. |
| `mcp [action]` | Manage Model Context Protocol search endpoints (`xiao mcp`, `xiao mcp url <URL>`, `xiao mcp test`, `xiao mcp search`, `xiao mcp tools`, `xiao mcp [brave|tavily|exa]`, `xiao mcp reset`). |
| `gateway` | Manage Telegram Bot Token connectivity and verify `OWNER_USER_ID`. |
| `version` | Display version information. |
| `help` | Display command-line help. |

### Terminal Chat REPL & Commands
Xiao provides an interactive terminal REPL when invoked with `cargo run -- chat`:

```text
Xiao Interactive Chat REPL Commands:
  /sessions         List all conversation sessions with IDs and message counts
  /switch <id>      Switch active conversation session
  /rm <id>          Remove conversation session by ID
  /new [name]       Create and switch to a new conversation session
  /clear            Reset conversation history in the active session
  /model            Inspect active Main model and provider endpoint
  /help             Show available REPL commands
  /exit             Exit chat mode (or Ctrl+C / Ctrl+D)
```

You can also run one-shot queries without entering the REPL:

```bash
cargo run -- chat "Summarize recent advances in Rust async runtimes"
```

---

## Configuration Reference

Settings can be defined in `.env` (or `~/.xiao.env`, `~/xiao/.env`) or managed dynamically through the CLI and persisted in SQLite.

| Variable | Default | Description |
| :--- | :--- | :--- |
| `BOT_TOKEN` | *Required* | Telegram Bot API token. |
| `OWNER_USER_ID` | *Required* | Telegram numerical ID of the authorized owner. |
| `ALLOWED_CHAT_IDS` | *Empty* | Comma-separated list of additional group chat IDs permitted to use the bot. |
| `DEDICATED_CHAT_IDS` | *Empty* | Comma-separated list of forum supergroups configured as dedicated workspaces (answers all topics without mention). |
| `AI_ENDPOINT` | `https://openrouter.ai/api/v1` | Base URL for OpenAI-compatible completions and chat endpoints (defaults to OpenRouter). |
| `AI_API_KEY` | *Required* | Bearer authentication token for the AI endpoint. |
| `AI_MODEL` | `google/gemini-2.0-flash-001` | Default model identifier for conversation. |
| `IMAGE_FALLBACK_PROVIDER` | `none` | Fallback provider for image generation (`none` or `pollinations`). |
| `AI_PROVIDER_CONNECT_TIMEOUT_SECS` | `10` | Connect timeout for standard AI API requests. |
| `IMAGE_GENERATION_TIMEOUT_SECS` | `120` | Request timeout for image generation endpoints. |
| `IMAGE_DOWNLOAD_TIMEOUT_SECS` | `30` | Timeout for retrieving generated image payloads. |
| `BRAVE_API_KEY` | *Optional* | API key for Brave Search integration in `web_search`. |
| `TAVILY_API_KEY` | *Optional* | API key for Tavily AI search integration. |
| `EXA_API_KEY` | *Optional* | API key for Exa search integration. |
| `EXA_MCP_URL` | `https://mcp.exa.ai/` | Model Context Protocol search server endpoint (keyless Exa MCP protocol). |

---

## Chat Routing & Workspaces

Xiao enforces a multi-tier routing policy inside `ChatRouteScope`:

1. **Owner Invariant**: Messages from non-owners are discarded immediately (`RouteDecision::Ignore`).
2. **Private Chat**: 1-on-1 chats with `OWNER_USER_ID` are always active and processed.
3. **Dedicated Personal Workspaces**:
   - Configured via `DEDICATED_CHAT_IDS` or any forum supergroup where Xiao holds administrator rights.
   - Xiao responds to all messages across all forum topics without requiring bot mentions, replies, or command prefixes.
4. **Guest Groups**:
   - Chat ID must be present in `ALLOWED_CHAT_IDS`.
   - Requires an explicit mention (`@bot_username`) or a direct reply to one of Xiao's messages.
5. **Native Stop**:
   - Sending a stop command or clicking stop in Telegram dispatches a `stopped_message_generation` update.
   - Stop signals bypass the intake queue for immediate in-flight stream cancellation.

---

## Supported Media & Documents

| Input Type | Extraction & Processing Method |
| :--- | :--- |
| **Plain Text** | Direct conversation, code instructions, and tool invocations. |
| **Photos / Images** | Routed to configured `Vision` role (or `Main` if multimodal). Supports JPEG, PNG, WEBP. |
| **Voice Notes & Audio** | Extracted via `Audio STT` (Whisper-compatible `/v1/audio/transcriptions`) or native audio processing. |
| **Video & Video Notes** | Bounded frame extraction processed through the `Video` specialist role. |
| **PDF Documents** | Text extracted via `lopdf`. Scanned pages (up to 6) are rendered to images and forwarded to `Vision`. |
| **DOCX Documents** | In-memory XML text and paragraph extraction. |
| **XLSX Documents** | In-memory XML parsing of sheets and shared string tables (`<si>`) within bounded memory quotas. |
| **Archives (ZIP, TAR, 7Z)** | In-memory extraction of text files up to 30 MB uncompressed limit. Flat single-level inspection prevents zip bombs. |

---

## Security Invariants

> [!IMPORTANT]
> The security boundaries below are strictly maintained across the entire codebase:

- **SSRF Mitigation**: Outbound media downloads and `fetch_url` requests pass through `url_policy`. Requests targeting private IPv4/IPv6 ranges (RFC 1918, RFC 4193), loopback addresses (`127.0.0.0/8`, `::1`), link-local spaces, and SIIT/NAT64 mapped addresses are rejected.
- **Credential Storage**: Raw API keys and bot tokens are isolated on the filesystem under `~/.local/share/xiaoai/secrets/` with strict mode `0o600` (directory `0o700`). The SQLite database retains only opaque `secret://` references.
- **Context Privacy**: Specialist models (`Vision`, `Video`, `Audio STT`) never receive the full conversation history. They only receive the immediate media artifact and query, returning bounded observation summaries to `Main`.

---

## Durable Processing & Operational Semantics

Xiao operates with durable **at-least-once** delivery semantics backed by SQLite WAL mode:

1. **Per-Scope FIFO Mailboxes**: Updates within the same conversation scope (`chat_id`, `thread_id`) are processed strictly in FIFO sequence by a dedicated scoped worker, while independent chats are processed concurrently up to a global semaphore limit (8 permits).
2. **Panic Isolation & Bounded Retry**: Each update runs in an isolated task protected against unwinding panics:
   - Upon transient panic, the global concurrency permit is immediately released (`drop(permit)`).
   - The update status is marked for retry in SQLite and waits through a 1.5-second backoff sleep.
   - The worker retries the update immediately up to a maximum of 2 attempts (`attempts <= 2`).
   - If an update fails consecutively or exceeds 2 attempts, it is quarantined to `failed` ("poison pill") to protect the queue from infinite crash loops.
3. **Operational Trade-Off & Duplicate Side-Effect Risk**:
   - Because in-worker retries trigger rapidly (~1.5s) on *any* panic without killing the daemon process, external side effects executed before an unexpected panic (e.g. an outbound Telegram message draft/reply already dispatched or an intermediate turn committed to SQLite before final inbox checkpointing) **will repeat upon retry**.
   - This is an intentional operational trade-off of at-least-once processing semantics: Xiao guarantees zero message loss over exactly-once execution.
4. **Crash Recovery**: On daemon startup, `recover_telegram_processing_async()` resets in-flight `processing` updates back to `pending` for at-least-once replay, while pre-filtering and quarantining records with `attempts >= 2`.

---

## Production Deployment (Systemd)

For production server deployments (e.g., Ubuntu, Debian, Arch Linux), run Xiao as a managed systemd service.

### Service Unit Template (`xiao.service`)

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
WorkingDirectory=/opt/xiao-chat
ExecStart=/opt/xiao-chat/target/release/xiao start
Restart=on-failure
RestartSec=5s
LimitNOFILE=65535

# Security hardening directives
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/opt/xiao-chat /var/lib/xiaoai
PrivateTmp=true
NoNewPrivileges=true

# Environment and secrets
Environment=XIAO_DATA_DIR=/var/lib/xiaoai
EnvironmentFile=-/opt/xiao-chat/.env

[Install]
WantedBy=multi-user.target
```

### Installation & Management

```bash
# 1. Create dedicated system user and runtime data directory
sudo useradd -r -s /usr/sbin/nologin -d /var/lib/xiaoai xiao
sudo mkdir -p /var/lib/xiaoai
sudo chown -R xiao:xiao /var/lib/xiaoai
sudo chmod 700 /var/lib/xiaoai

# 2. Build optimized release binary
cargo build --release --locked

# 3. Install systemd unit
sudo cp xiao.service /etc/systemd/system/xiao.service
sudo systemctl daemon-reload

# 4. Enable and start daemon
sudo systemctl enable --now xiao

# 5. Inspect daemon logs and status
sudo systemctl status xiao
journalctl -u xiao -f
```

---

## Verification & Quality Gates

Run the automated quality suite locally:

```bash
# Format check
cargo fmt --all -- --check

# Compiler check
cargo check --locked

# Unit tests and Bot API 10.3 contract suite
cargo test --locked

# Clippy linter
cargo clippy --locked --all-targets --all-features -- -D warnings
```

---

## Cross-Compilation Targets

Xiao is regularly built and tested for:
- **Linux x86_64**: Standard server and desktop environments.
- **Linux aarch64 (Armbian / SBCs)**: Single-board computers like Raspberry Pi and Orange Pi (`aarch64-unknown-linux-gnu`).
- **Android aarch64**: Native execution in Termux or standalone deployment via `cargo-ndk` (`aarch64-linux-android`).
