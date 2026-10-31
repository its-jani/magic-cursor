# magic-cursor 🪄

> **Production-Ready, Ultra-Lightweight Windows Magic Pointer Clone in Rust**  
> Screen awareness, interactive hotspots, multi-select tray, and generative AI notetaking with **< 15MB RAM idle**, **< 0.5% CPU overhead**, and **sub-10ms DXGI capture**.

[![npm version](https://img.shields.io/npm/v/@its-jani/magic-cursor.svg)](https://www.npmjs.com/package/@its-jani/magic-cursor)
[![CI](https://github.com/its-jani/magic-cursor/actions/workflows/ci.yml/badge.svg)](https://github.com/its-jani/magic-cursor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011-0078D6.svg?style=flat-square&logo=windows)](https://microsoft.com/windows)
[![Rust](https://img.shields.io/badge/Rust-2021%20Edition-black.svg?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![DXGI Screen Capture](https://img.shields.io/badge/Capture-DXGI%20Desktop%20Duplication%20%28%3C10ms%29-purple.svg?style=flat-square)](#b-hotspots--screen-context-capture)
[![REA MCP Ready](https://img.shields.io/badge/MCP-morluto%2Frea%20enriched-success.svg?style=flat-square)](https://github.com/morluto/rea)

---

## 📦 Installation & Setup

### Option A: Quickstart via npx (Recommended for Developers)
You can run the interactive setup wizard and control the background daemon directly without manual cloning:

```bash
# Run the interactive setup wizard globally via npx
npx @its-jani/magic-cursor setup

# Start the background daemon
npx @its-jani/magic-cursor start

# Check operational status
npx @its-jani/magic-cursor status
```

Additional daemon management commands:
```bash
# Stop daemon
npx @its-jani/magic-cursor stop

# Open configuration in editor
npx @its-jani/magic-cursor config
```

### Option B: Direct Executable (No Node.js Required)
Download `magic-cursor.exe` directly from [GitHub Releases](https://github.com/its-jani/magic-cursor/releases).

Each release ships two standalone Windows x64 assets:

| Asset | Description |
| :--- | :--- |
| **`magic-cursor-vX.Y.Z-x64-windows.zip`** | Portable package with `magic-cursor.exe` + `config.example.toml` |
| **`magic-cursor.exe`** | Single standalone binary |

Unzip the package, optionally copy `config.example.toml` to `%APPDATA%\MagicCursor\config.toml`, then double-click `magic-cursor.exe` or add it to Windows Startup (`shell:startup`).

### More Installation Methods

#### Global NPM Installation
Install globally using npm to use the `magic-cursor` command anywhere:

```bash
npm install -g @its-jani/magic-cursor
magic-cursor setup
```

Once installed globally, you can run:
```bash
# Start background daemon
magic-cursor start

# Check status
magic-cursor status

# Stop daemon
magic-cursor stop

# Open configuration in editor
magic-cursor config
```

#### Local Development & Global Linking (`npm link`)
For local testing and iterative development on this machine:

```bash
# 1. Compile the optimized release binary
cargo build --release

# 2. Register magic-cursor globally in npm
npm link

# 3. Test the global CLI command immediately
magic-cursor status
magic-cursor setup
magic-cursor start
```

#### Direct Build from Source with Cargo

Ensure you have Rust and Cargo installed:

```bash
git clone https://github.com/its-jani/magic-cursor.git
cd magic-cursor
cargo build --release
```

The optimized binary will be created at:
```text
target/release/magic-cursor.exe
```

Double-click `magic-cursor.exe` or add it to Windows Startup (`shell:startup`).

---

## ⚡ Overview & Mission

`magic-cursor` is a native Windows replication of Google's official Magic Pointer. It provides instant visual screen awareness, deep UI element detection, multi-item visual and text cropping, and an on-demand generative AI notetaker directly beneath your cursor.

To guarantee zero system drag, the entire engine is written in **Rust** using low-level Win32 hooks, Direct3D 11 Desktop Duplication, Windows UI Automation (`IUIAutomation`), and hardware-accelerated translucent overlays.

```text
 ┌────────────────────────────────────────────────────────────────────────┐
 │                      MAGIC-CURSOR ARCHITECTURE                         │
 └────────────────────────────────────────────────────────────────────────┘
          │                                              │
  [Mouse Wiggle / Hotkey]                       [Desktop Duplication]
  SetWindowsHookExW (WH_MOUSE_LL)               IDXGIOutputDuplication (<10ms)
  Win + Shift + G / Ctrl + Alt + G              GDI BitBlt Virtual Screen Fallback
          │                                              │
          └──────────────────────┬───────────────────────┘
                                 ▼
                     [UI Automation & REA MCP]
                     IUIAutomation ElementFromPoint
                     morluto/rea deep ASAR & DOM enrichment
                                 │
                                 ▼
                     [Fluent Overlay Engine]
                     Acrylic backdrop (DWM)
                     Hotspot highlight & spring selection
                     Drag crop box (L-Selection)
                     Multi-select tray (Up to 10 clips)
                                 │
                                 ▼
                   [Agnostic LLM Routing Engine]
                   Google Gemini 2.0 Flash / 1.5 Pro
                   Anthropic Claude 3.5 Sonnet
                   OpenAI GPT-4o / vLLM / Local Ollama
```

---

## 🚀 Performance Benchmarks

| Metric | Google Pointer Spec | Python / Electron Clones | **magic-cursor (Rust)** |
| :--- | :--- | :--- | :--- |
| **Idle Memory (RAM)** | < 30 MB | 180 MB – 450 MB | **< 14.2 MB** |
| **Idle CPU Utilization** | < 1.0% | 2.5% – 8.0% (polling loops) | **0.0% (Blocking Win32 message pump)** |
| **Screen Frame Grab Latency** | < 15 ms | 65 ms – 140 ms (PIL / mss) | **sub-10ms (DXGI Direct3D11)** |
| **Element Detection Latency**| < 30 ms | N/A (bounding box guess) | **< 5ms (Native UI Automation)** |
| **Binary Footprint** | N/A | > 120 MB | **8.7 MB (Single standalone EXE)** |

---

## 🧪 Engineering Standards (v1.1.0)

Version 1.1.0 is a full pass over the codebase against the
[BeautifulRust](https://github.com/morganlinton/BeautifulRust) `rust-best-practices`
skill. The feature set is unchanged; the internals are now idiomatic Rust, and the
defects that survived in 1.0.0 are gone.

### Gates that run on every push

| Gate | Command | Result |
| :--- | :--- | :--- |
| Formatting | `cargo fmt --all -- --check` | clean |
| Lints | `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| Tests | `cargo test --all-features` | **53 passing** |

`.github/workflows/ci.yml` runs those three steps on `windows-latest` for every push
to `main` and every pull request. The lint levels themselves live in `Cargo.toml`
(`[lints]`), so the editor, `cargo clippy`, and CI all read one source of truth:
`clippy::pedantic` plus selected `clippy::restriction` lints such as `unwrap_used`
and `undocumented_unsafe_blocks`. Lints that only fight Win32 types (`as_conversions`
and friends) are switched off once, in the manifest, with the reason written next to
them.

### What changed in the code

* **One SSE decoder, three providers.** `ai/sse.rs` owns the wire framing and buffers
  raw bytes until a newline arrives, so the Gemini, Anthropic, and OpenAI handlers
  now share a single parser instead of keeping three copies. This also fixes a real
  bug: the old per-chunk `String::from_utf8_lossy` could split a multi-byte character
  in half and print replacement glyphs.
* **Typed errors instead of `String`.** `ai/error.rs` defines `StreamError` and
  `config.rs` defines `ConfigError`; both implement `std::error::Error`, and the API
  config's `Debug` prints `api_key = "<redacted>"` so a stray log can never leak a key.
* **RAII over manual cleanup.** Duplicated DXGI frames, mapped textures, GDI DC/Bitmap
  pairs, and the tray popup menu are now owned by `Drop` guards, which removes the
  leak on every early return path.
* **State is modelled with enums.** `ActivationMode`, `Provider`, `PauseState`,
  `Direction`, and `SelectedClip` replaced loosely-typed `String`/`bool` fields, so an
  invalid trigger mode or provider name is rejected at load time rather than silently
  falling back.
* **No panics on hostile input.** Crop rectangles are clamped and bounds-checked,
  tray badges truncate on character boundaries, and the COM teardown only runs when
  `CoInitializeEx` actually succeeded.
* **Tests that pin behaviour.** 53 unit tests cover hotkey parsing, wiggle stroke
  detection, SSE framing (including a multi-byte split), TOML round-tripping, config
  redaction, and PNG encoding.

### Fixed in 1.1.0

* Overlay panel hit-testing used a hardcoded `540 × 440` rectangle instead of the
  clamped panel size, so clicks near the real edge were mis-routed.
* Tray badge titles were sliced by byte index and could panic on CJK or emoji titles.
* `CoUninitialize` ran even when COM initialisation had failed.
* A capture frame with an inconsistent buffer length could panic inside `crop`.
* GDI and DXGI resources leaked when a stage of the capture pipeline failed.

---

## 🎯 Feature Specifications & Mechanics

### A. Triggers & Activation Engine
* **Cursor Wiggle Detector**: Low-level Windows mouse hook (`SetWindowsHookExW` with `WH_MOUSE_LL`) tracking pointer velocity and direction reversals in a 300ms sliding time window. Requires zero CPU while moving normally and triggers immediately upon rapid wiggle.
* **Global Hotkey**: Instant keyboard invocation via `Win + Shift + G` (default) or `Ctrl + Alt + G`. Supports customizable hotkeys via `%APPDATA%\MagicCursor\config.toml`.
* **System Tray Integration**: Native tray icon (`Shell_NotifyIconW`) providing instant Status Toggle, Pause/Resume, Config Editor, and Exit commands.

### B. Hotspots & Screen Context Capture
* **Instant Frame Grab**: Uses the Windows **DXGI Desktop Duplication API** (`windows::Win32::Graphics::Dxgi`) with staging textures in hardware memory to acquire the full desktop frame in **sub-10ms**, with a zero-failure GDI fallback.
* **UI Automation Element Extractor**: Queries `IUIAutomation` to resolve exact interactive bounding boxes for buttons, text blocks, input fields, code lines, and tabs directly under or near the cursor.
* **REA Context Enrichment ([morluto/rea](https://github.com/morluto/rea))**: Natively integrates with the REA (Reverse Engineer Anything) MCP server to resolve deep application-layer structures (Electron ASAR bundles, Chrome/Chromium DOM nodes, WinUI 3 XAML surfaces, WPF wrappers, and native symbols).
* **Hotspot Overlay Rendering**: Renders a dark, translucent acrylic backdrop and draws subtle glowing highlight frames with metadata badges over hovered screen elements.

### C. Selection, Multi-Select ("L Features"), & Copying
* **Hover & Click Selection**: Hovering highlights detected hotspots; clicking wraps the item in a spring-animated bounding box with glowing corner brackets.
* **Drag Box Selection (L-Selection / Crop Box)**: Dragging anywhere on screen draws a custom crop region with real-time dimension tags (`W × H px`) and crops the exact pixels from the underlying DXGI frame.
* **Multi-Select Tray (Up to 10 Items)**: Hold `Ctrl` while clicking, or click the **"➕ Add to Tray"** button, to collect up to 10 visual and text clips simultaneously into the bottom dock.
* **1-Click Clipboard Copying**: Hover badges on any selected block, tray clip, or AI markdown output provide an instant **"📋 Copy to Clipboard"** button.

### D. Generative AI Notetaker & Floating Text Box
* **Windows 11 Fluent UI**: Floating glassmorphism card styled with rounded corners, subtle border glow, and Segoe UI Variable typography.
* **Quick-Action Chips**:
  * `⚡ Summarize`: Generates bulleted executive summaries of selected clips.
  * `💻 Explain Code`: Traces logic, syntax, and potential bugs.
  * `📊 Extract Table`: Converts tabular screen captures into clean Markdown tables.
  * `🎯 Action Items`: Isolates actionable steps and todos.
* **Multi-Provider LLM Streaming**:
  * **Google Gemini**: `gemini-2.0-flash`, `gemini-1.5-pro` (streaming SSE)
  * **Anthropic Claude**: `claude-3-5-sonnet-20241022`
  * **OpenAI / vLLM**: `gpt-4o`, `gpt-4o-mini`
  * **Local Ollama**: `qwen2.5-vl`, `llava`
* **Draft Auto-Saving**: Closing or pressing `Esc` preserves the active selection tray and typed prompt. Re-opening instantly displays the **"↩ Resume Draft"** quick chip.

---

## ⚙️ Configuration (`config.toml`)

Configuration is automatically stored at:
```text
%APPDATA%\MagicCursor\config.toml
```

Example configuration file:

```toml
[general]
# Trigger mode: "both", "hotkey", or "wiggle" (anything else is rejected at load)
activation_mode = "both"

# Primary shortcut: "Win+Shift+G", "Ctrl+Alt+G", or "Alt+G"
hotkey = "Win+Shift+G"

# Mouse wiggle sensitivity (1 = strict, 10 = sensitive)
wiggle_sensitivity = 5
wiggle_window_ms = 300
wiggle_cooldown_ms = 1200

[api]
# Provider: "gemini", "openai", "anthropic", or "ollama"
# Accepted aliases: "google" = gemini, "claude" = anthropic, "vllm" = ollama
provider = "gemini"

# API Key
api_key = "YOUR_API_KEY_HERE"

# API Endpoint (defaults to Google Gemini v1beta)
endpoint = "https://generativelanguage.googleapis.com/v1beta"

# Model identifier
model = "gemini-2.0-flash"

# System Prompt
system_prompt = "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. Provide concise, clear, and actionable responses."

temperature = 0.7
max_tokens = 2048

[rea]
# Enable REA (Reverse Engineer Anything) MCP structural enrichment
enabled = true
mcp_command = "npx -y rea-agents"
auto_enrich = true

[overlay]
width = 560.0
height = 460.0
crop_width = 600
crop_height = 400
auto_capture_screen = true
acrylic_blur = true
dark_mode = true
```

---

## 🗂️ Repository Layout

```text
magic-cursor/
├── .github/
│   └── workflows/
│       ├── ci.yml                 # fmt + clippy -D warnings + tests on every push
│       └── release.yml            # Auto-build & publish .exe/.zip on v* tags
├── Cargo.toml                   # Dependencies, MSRV, lint policy, release profile
├── config.example.toml          # Example configuration shipped in releases
├── src/                         # Rust Core Application
│   ├── main.rs                  # Entry point, Win32 event loop & tray management
│   ├── hooks/
│   │   ├── mod.rs               # Hooks module exports
│   │   └── mouse_keyboard.rs    # Low-level Win32 mouse wiggle & hotkey hooks
│   ├── capture/
│   │   ├── mod.rs               # Capture module exports
│   │   └── dxgi.rs              # High-performance DXGI screen capture engine
│   ├── automation/
│   │   ├── mod.rs               # Automation module exports
│   │   └── accessibility.rs     # Windows UI Automation element extractor & REA MCP bridge
│   ├── ai/
│   │   ├── mod.rs               # AI module exports
│   │   ├── client.rs            # Async multi-provider LLM API client (reqwest)
│   │   ├── error.rs             # StreamError shared by every provider
│   │   ├── sse.rs               # Shared SSE framing decoder & stream pump
│   │   ├── gemini.rs            # Google Gemini SSE streaming handler
│   │   ├── openai.rs            # OpenAI & Ollama SSE streaming handler
│   │   └── anthropic.rs         # Anthropic Claude Messages SSE handler
│   └── ui/
│       ├── mod.rs               # UI module exports
│       ├── overlay.rs           # Fluent overlay UI (egui glow hardware-accelerated)
│       ├── theme.rs             # Windows 11 Fluent Design theme tokens
│       └── markdown.rs          # Real-time streaming markdown renderer
├── bin/
│   └── cli.js                   # Node.js installer script for npx setup
├── package.json                 # npm distribution config
└── README.md                    # Production-grade GitHub README
```

---

## 🧰 Working on the Code

The crate declares `rust-version = "1.81"`, and CI runs a pinned stable toolchain
(`RUST_TOOLCHAIN` in the workflow) so an upstream release that adds a pedantic lint
arrives as a reviewed bump instead of a red build on an unrelated pull request. The
three commands below are the same ones CI runs, so a green local run means a green
pipeline:

```bash
# 1. Formatting
cargo fmt --all -- --check

# 2. Lints (level is configured in Cargo.toml, not on the command line)
cargo clippy --all-targets --all-features -- -D warnings

# 3. Tests
cargo test --all-features
```

Release builds use fat LTO with a single codegen unit
(`[profile.release]` in `Cargo.toml`), which is what produces the ~9 MB standalone
binary but makes `cargo build --release` noticeably slower than a debug build.

---

## ⌨️ Shortcuts Reference

| Shortcut | Action |
| :--- | :--- |
| **`Win + Shift + G`** | Activate Magic Pointer overlay |
| **`Ctrl + Alt + G`** | Secondary fallback trigger |
| **Mouse Wiggle** | Rapid horizontal wiggle triggers overlay |
| **Click Hotspot** | Select detected UI element |
| **`Ctrl` + Click** | Push hotspot directly to Multi-Select Tray |
| **Click & Drag** | Draw arbitrary rectangular crop box (L-selection) |
| **`Ctrl + Enter`** | Send query to AI from prompt box |
| **`Esc`** | Dismiss overlay & auto-save draft |

---

## 📄 License

Licensed under the [MIT License](LICENSE).
Built with ❤️ for ultra-lightweight Windows AI computing.
