# AGENTS.md

> Operational and architectural guide for AI coding agents working in the `xiao-chat` repository.

---

## 1. Project Overview

`xiao` is a hardened, single-owner AI assistant for Telegram built in Rust (2021 edition) targeting **Telegram Bot API 10.3**. It supports Rich Messages (AST blocks), streaming drafts with native stop controls, durable inbox queueing with at-least-once recovery, three-tier long-term memory, and modular OpenAI-compatible multimodal AI routing (Main, Vision, Video, Audio STT, Image Generation, Curator).

---

## 2. Essential Commands

### Build & Check
```bash
# Verify compilation without emitting binaries
cargo check --locked

# Build development binary
cargo build --locked

# Build optimized release binary
cargo build --release --locked
```

### Testing
```bash
# Run all unit tests and contract tests
cargo test --locked

# Run a specific test by filter
cargo test <filter_name> --locked

# Run tests with stdout output enabled
cargo test --locked -- --nocapture

# Run the Telegram Bot API 10.3 contract suite only
cargo test --test bot_api_10_3_contract --locked
```

### Formatting & Linting
```bash
# Check code formatting (CI enforced)
cargo fmt --all -- --check

# Auto-format codebase
cargo fmt --all

# Run Clippy with strict warnings (CI enforced)
cargo clippy --locked --all-targets --all-features -- -D warnings
```

### CLI Subcommands (`xiao`)
The binary supports direct execution for testing, administration, and daemon operation:
```bash
# Start Telegram daemon (default long-polling loop)
cargo run -- start

# Direct terminal chat mode (one-shot query or interactive REPL without Telegram)
cargo run -- chat "Hello, who are you?"
cargo run -- chat

# Display system, database, and provider status dashboard
cargo run -- status

# Inspect token usage and context breakdown for a chat/thread
cargo run -- context [chat_id] [thread_id]

# Manage long-term user memories
cargo run -- memory
cargo run -- memory rm <key>
cargo run -- memory clear

# AI Provider and Specialist Addon Management
cargo run -- ai
cargo run -- ai list
cargo run -- ai use <model>
cargo run -- ai addon
cargo run -- ai test [role]

# Model Context Protocol (MCP) & Search Management
cargo run -- mcp
cargo run -- mcp url <URL>
cargo run -- mcp test [query]
cargo run -- mcp search <query>
cargo run -- mcp tools
cargo run -- mcp brave [KEY|rm]
cargo run -- mcp tavily [KEY|rm]
cargo run -- mcp exa [KEY|rm]
cargo run -- mcp reset

# Telegram Gateway & Owner Configuration
cargo run -- gateway
cargo run -- gateway check
cargo run -- gateway token <BOT_TOKEN>
cargo run -- gateway owner <OWNER_USER_ID>

# Interactive initial setup wizard
cargo run -- setup
```

---

## 3. Architecture & Control Flow

### High-Level Component Map
```
┌─────────────────────────────────────────────────────────────┐
│                       xiao CLI / Daemon                     │
└──────────────┬───────────────────────────────┬──────────────┘
               │                               │
       (Telegram Polling)               (Direct CLI Chat)
               ▼                               │
┌───────────────────────────────┐              │
│      TelegramBotClient        │              │
│  (Transport, SSRF, Fallback)  │              │
└──────────────┬────────────────┘              │
               ▼                               │
┌───────────────────────────────┐              │
│    Durable Intake Queue       │              │
│   (SQLite telegram_inbox)     │              │
└──────────────┬────────────────┘              │
               ▼                               │
┌───────────────────────────────┐              │
│   ChatRouteScope Evaluator    │              │
│  (Owner & Workspace Filter)   │              │
└──────────────┬────────────────┘              │
               ▼                               ▼
┌─────────────────────────────────────────────────────────────┐
│                       AIChatService                         │
│  ┌─────────────────────────┐   ┌─────────────────────────┐  │
│  │    Model Role Router    │   │    Three-Tier Memory    │  │
│  │ (Main, Vision, STT, etc)│   │ (Facts, Summary, Turns) │  │
│  └───────────┬─────────────┘   └────────────┬────────────┘  │
│              ▼                              ▼               │
│  ┌─────────────────────────┐   ┌─────────────────────────┐  │
│  │  Streaming SSE Decoder  │   │ SQLite Storage & WAL    │  │
│  │  & Timeline Draft Sync  │   │   (~/.local/share/...)  │  │
│  └─────────────────────────┘   └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Module Responsibilities
- `src/cli/`: Modular command-line subcommands and interactive interfaces:
  - `tui.rs`: Terminal UI engine, RAII raw-mode lifecycle guard (`CleanRawMode`), and ANSI layout formatters.
  - `status.rs`: Dashboard and diagnostic system status rendering.
  - `gateway.rs`: Telegram gateway and bot token/owner management.
  - `wizard.rs`: Interactive setup and onboarding quickstart.
  - `chat.rs`: Terminal chat REPL and multi-session manager (`/sessions`, `/switch`, `/rm`, `/new`).
  - `memory.rs`: Tier-1 persistent memory management.
  - `context.rs`: Token usage and sliding-window breakdown inspector.
  - `mcp.rs`: Model Context Protocol and search engine hub.
  - `ai_hub.rs`: Provider, model catalog, and multimodal specialist routing.
  - `help.rs`: Global CLI help screen.
- `src/bot/`:
  - `client.rs` / `client/raw.rs`: Telegram API client supporting Bot API 10.3 rich message drafts, ephemeral contexts, and file downloads.
  - `models.rs` / `models/base.rs`: Type-safe Telegram API models, rich message block definitions (`RichBlock`), and validation bounds.
  - `transport_policy.rs`: Retry backoff logic, HTTP 429 rate limit parsing, and Bad Request fallback gates.
  - `url_policy.rs`: Outbound SSRF firewall preventing requests to private, loopback, link-local, and SIIT/NAT64 translated IP ranges.
- `src/ai/`:
  - `service.rs`: Primary orchestration for text chat, streaming responses, multimodal routing, tool execution, and image generation.
  - `routing.rs`: Specialist model role resolution (`ModelRole`: Main, Vision, Video, AudioStt, ImageGeneration, Curator).
  - `capability.rs` / `provider.rs`: Live model probe harness and capability verification (e.g. confirming whether an endpoint actually supports vision or tool calling).
  - `storage.rs`: SQLite persistence layer (WAL mode) for messages, sessions, user memories, scoped summaries, and encrypted/private credentials.
  - `stream.rs`: UTF-8 chunk-safe Server-Sent Events (SSE) streaming decoder.
  - `tools.rs`: Function calling engine (`web_search` with keyless Exa MCP protocol, Tavily/Brave API, and DuckDuckGo/Wikipedia fallbacks; and `fetch_url`).
- `src/document.rs` & `src/document/archive.rs`: In-memory safe extraction of text, archives (ZIP, TAR, TAR.GZ, 7Z), Office files (DOCX, XLSX), and PDF page extraction/rendering.
- `src/attachments.rs`: Content attachment persistence scoped by chat/thread.
- `src/timeline.rs`: Real-time streaming draft management with progress spinner and activity state indicators.
- `src/parser/`:
  - `markdown.rs`: Converts extended markdown to Telegram Bot API 10.3 `RichBlock` AST representations.
  - `latex.rs`: Sanitizes mathematical expressions for cross-platform Android and iOS rendering.
  - `rtl.rs`: Detects Right-to-Left (RTL) scripts (Arabic, Hebrew, Persian, Urdu, etc.) and Eastern Arabic numerals, automatically setting layout direction and right-aligned table cells.
  - `terminal.rs`: ANSI terminal rendering for CLI chat and logs.

---

## 4. Key Architectural & Security Invariants

When modifying or adding features, you **must** preserve these invariants:

### 1. Hard Single-Owner Invariant
- **Rule**: Non-owner updates are dropped silently at the gateway (`main.rs`).
- **Implementation**: `ctx.user_id != self.owner_user_id` immediately yields `RouteDecision::Ignore`. Never respond to, log, or leak bot existence to unauthorized Telegram IDs.

### 2. Pure Zero-Slash Gateway
- **Rule**: Xiao is designed as a natural conversational gateway.
- **Implementation**: `bot.set_my_commands(&[])` is executed at startup to clear Telegram slash menus. User requests (including image generation and clear requests) are detected conversationally via regex/heuristics or handled via CLI.

### 3. Outbound SSRF & Network Security
- **Rule**: Any remote fetch (e.g., in `fetch_url`, image generation fallback download, or media downloader) must be validated against `bot::url_policy::resolve_download_url` and `is_unsafe_remote_ip`.
- **Blocked**: Loopback (`127.0.0.0/8`, `::1`), RFC 1918 private subnets, link-local addresses, SIIT/NAT64-mapped IPv6, and unsafe URI schemes.

### 4. Secret Isolation
- **Rule**: Plaintext API keys and bot tokens are **never** committed, written to SQLite in plaintext, or printed in debug logs.
- **Mechanism**: Secrets are written to disk under `~/.local/share/xiaoai/secrets/` with strict `0o600` file / `0o700` directory permissions. The database only stores a `secret://` URI reference (`api_key_ref`).

### 5. Durable Intake Queue, Keyed Mailbox Isolation & Native Stop
- **Queue**: Telegram long-polling writes incoming updates directly to SQLite table `telegram_inbox` as `pending`.
- **Per-Scope Keyed Mailboxes**: Updates are dispatched into dedicated per-scope channels (`ScopeKey { chat_id, thread_id }`). This guarantees strict FIFO processing within any individual chat or topic while processing different chats concurrently up to a global semaphore limit (8 permits). Mailbox workers gracefully despawn after 30 seconds of inactivity using an atomic critical section to prevent lost messages.
- **Panic Isolation & Bounded Retry**: Each update is executed inside an isolated `tokio::spawn` task with unwinding panic protection (`JoinError::is_panic()`).
  - Upon transient panic, the global concurrency permit is immediately released (`drop(permit)`), a 1.5-second backoff sleep occurs, and the update is retried immediately in-worker up to a maximum of 2 attempts (`attempts <= 2`, seeded and tracked directly in SQLite).
  - If a task panics consecutively or exceeds 2 attempts, it is quarantined as `failed` ("poison pill") to protect the queue from infinite crash loops.
- **Operational Trade-Off & Duplicate Side-Effect Risk**:
  - Because in-worker retries trigger rapidly (~1.5s) on *any* panic without killing the daemon process, external side effects executed before an unexpected panic (e.g. an outbound Telegram message draft/reply already dispatched or an intermediate turn committed to SQLite before final inbox checkpointing) **will repeat upon retry**.
  - This is an intentional operational trade-off of at-least-once processing semantics: Xiao guarantees zero message loss over exactly-once execution.
- **Crash Recovery**: On startup, `recover_telegram_processing_async()` resets any in-flight `processing` updates back to `pending` to guarantee at-least-once recovery across process restarts, while quarantining updates with `attempts >= 2`.
- **Native Stop Priority**: `stopped_message_generation` updates bypass worker mailboxes and execute synchronously to cancel in-flight generation tokens with zero latency.

### 6. Specialist Context Isolation
- **Rule**: Canonical conversational history belongs solely to the `Main` model.
- **Specialists**:
  - `Vision` / `Video`: Receive only the current media payload and the immediate user prompt; never the full conversation history.
  - `AudioStt`: Returns transcripts to Main; never receives previous conversation history.
  - Specialist outputs are returned to Main as bounded observation turns.

---

## 5. Storage & State Layout

SQLite database location and files default to:
- **Base directory**: `~/.local/share/xiaoai/` (or `$XIAO_DATA_DIR` if configured)
- **Database**: `xiaoai.db` (configured with `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;`)
- **Secrets directory**: `<base>/secrets/` (e.g. `~/.local/share/xiaoai/secrets/`)
- **Attachments directory**: `<base>/attachments/<chat_id>/<thread_id>/`

### Database Tables:
- `settings`: Key-value application configuration.
- `sessions`, `active_sessions`, `session_counters`: Chat session tracking.
- `messages`: Canonical conversation history.
- `user_memories`: Tier-1 persistent user profile facts (Name, Tech Stack, Preferences).
- `scoped_summaries`: Tier-2 condensed summaries of older topics per chat/thread.
- `telegram_inbox`: Durable update queue for Telegram intake.
- `telegram_state`: Offset and webhook state persistence.

### Configuration Resolution Order
Configuration is loaded via `get_config_path()` in `src/main.rs`:
1. `.env` in current working directory
2. `~/.xiao.env`
3. `~/xiao/.env`
4. `~/XiaoAI/.env`

---

## 6. Testing Patterns & Guidelines

- **Unit tests**: Colocated in each source file within `#[cfg(test)] mod tests { ... }`.
- **Contract tests**: Located in `tests/bot_api_10_3_contract.rs`. This suite verifies serialization and wire compatibility against Telegram Bot API 10.3 requirements:
  - Discriminator fields (e.g., `type: "voice_note"`, `type: "button"`).
  - Rich block bounds (`RICH_MESSAGE_MAX_TEXT_CHARS = 32_768`, `RICH_MESSAGE_MAX_BLOCKS = 500`).
  - Media group constraints (albums must contain 2 to 10 homogeneous items).
- **Mocking & Isolation**:
  - Tests do not require a live Telegram bot token or active AI provider; network calls in tests use mock HTTP responses or test synthetic structs.
  - When writing tests involving `rusqlite`, use in-memory SQLite connections (`Connection::open_in_memory()`) or isolated temporary directories.

---

## 7. Development Gotchas & Conventions

1. **Telegram Rich Block Serialization**:
   - `RichBlock` types serialize directly to JSON structures required by Telegram 10.3. Ensure all variants adhere to base schemas in `src/bot/models/base.rs`.
   - When converting markdown, use `build_full_rich_message` or `parse_streaming_markdown_to_rich_blocks`.
2. **Decompression & Archive Limits**:
   - Archive extraction in `src/document/archive.rs` enforces strict resource caps to defeat zip bombs:
     - Max total uncompressed bytes: 30 MB (`MAX_ARCHIVE_TOTAL_UNCOMPRESSED_BYTES`).
     - Max single entry: 2 MB (`MAX_ARCHIVE_SINGLE_ENTRY_BYTES`).
     - Nested archives are classified as `NestedArchive` and **not** unpacked recursively.
3. **Android / Termux Platform Context**:
   - This project compiles and runs inside Termux on Android (`aarch64-linux-android`) as well as standard Linux servers (`aarch64-unknown-linux-gnu` and `x86_64`).
   - Keep build dependencies pure Rust where possible (e.g., `rustls-tls` is enabled in `reqwest`, `rusqlite` uses the `bundled` feature).
4. **Git Commits & Formatting**:
   - Always run `cargo fmt --all -- --check` and `cargo clippy --locked --all-targets --all-features -- -D warnings` before committing.
   - Commit messages must follow project conventions: clear, descriptive, under 72 characters on the first line, focusing on why the change exists.
5. **RTL & Bidirectional Layout (Bot API 10.3)**:
   - When text or rich block contents contain RTL scripts (Arabic, Hebrew, Persian, Urdu) or Eastern Arabic-Indic / Hindi numerals (`٠..٩` / `\u0660..\u0669`), `InputRichMessage.is_rtl` must be set to `Some(true)`.
   - Markdown and Unicode box tables automatically detect RTL content in headers and cells, defaulting unspecified column alignments to `"right"` so Telegram mirrors and renders them naturally from right to left.
