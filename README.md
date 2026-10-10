# MagicCursor-Win 🪄

> **Hyper-Lightweight, High-Performance AI Cursor Utility for Windows 10 & 11**  
> Built with Rust, Windows Win32 APIs, Tokio, and egui.

---

## ⚡ Key Highlights & Architecture

- **Ultra-Resource Efficient**: Idles at **0.0% CPU** and **< 20MB RAM** using a native Win32 message pump with blocking message retrieval (`GetMessageW`).
- **Flexible Trigger Engine**:
  - **Global Hotkey**: System-wide shortcut (`Alt+G`, `Win+Alt+Space`, or configurable) registered with `RegisterHotKey`.
  - **Mouse "Wiggle" Gesture (Opt-in)**: High-speed low-level hook (`WH_MOUSE_LL`) tracking horizontal direction reversals with a 250ms sliding window and 0ms hook processing overhead.
- **Context Capture Engine**:
  - Automatically captures active clipboard text or images (`arboard`).
  - Fallback to silent, localized screen capture centered at the mouse cursor (`600x400` px) using Win32 GDI `BitBlt` and in-memory PNG base64 encoding.
- **Multi-Provider AI Streaming Client**:
  - **OpenAI Compatible**: OpenAI, OpenRouter, Groq, DeepSeek, Ollama, LM Studio, vLLM.
  - **Google Gemini**: Direct Gemini REST API with SSE streaming.
  - **Anthropic Claude**: Claude Messages API with SSE streaming.
- **HUD UI Overlay**:
  - Topmost, frameless, borderless glassmorphism card spawned beside cursor coordinates `(Mouse_X + 15, Mouse_Y + 15)` clamped to virtual desktop boundaries.
  - Auto-focused input, quick-action chips (Summarize, Explain, Fix Grammar, Translate, Refactor).
  - Real-time streaming markdown renderer, copy button, Esc to close.
- **System Tray Integration**:
  - Native Win32 Tray icon (`Shell_NotifyIconW`) with context menu: Trigger, Pause/Resume, Open Config, Open Folder, Exit.

---

## 🚀 Getting Started

### 1. Build the Binary
```bash
cargo build --release
```
The optimized executable will be generated at:
```
target/release/magic-cursor-win.exe
```

### 2. Configuration (`config.toml`)
The configuration file is automatically generated on first launch at:
```
%APPDATA%\MagicCursor\config.toml
```

Example configuration:
```toml
[general]
# Activation trigger mode: "hotkey", "wiggle", or "both"
activation_mode = "both"

# Global hotkey shortcut (e.g. "Alt+G", "Win+Alt+Space", "Ctrl+Shift+Space")
hotkey = "Alt+G"

# Mouse wiggle sensitivity (1 = strict, 10 = sensitive, default = 5)
wiggle_sensitivity = 5

# Wiggle time window in milliseconds (default = 250)
wiggle_window_ms = 250

# Cooldown between wiggle triggers in milliseconds
wiggle_cooldown_ms = 1200

[api]
# Provider: "openai", "gemini", "anthropic", or "custom"
provider = "openai"

# API Key
api_key = "YOUR_API_KEY_HERE"

# API Endpoint URL
endpoint = "https://api.openai.com/v1/chat/completions"

# Model name (e.g. "gpt-4o-mini", "gemini-1.5-flash", "claude-3-5-sonnet-20241022", "llama3.2")
model = "gpt-4o-mini"

# System Prompt
system_prompt = "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. Provide concise, clear, and actionable responses."

temperature = 0.7
max_tokens = 2048

[overlay]
width = 520.0
height = 420.0
crop_width = 600
crop_height = 400
auto_capture_screen = true
```

---

## 🎯 Usage & Workflows

1. **Highlight & Ask**: Select any text in any application (browser, IDE, document), copy it or press `Alt+G` (or wiggle your mouse rapidly). The overlay appears instantly with the text attached.
2. **Visual Inspection**: Hover over any diagram, UI element, or code snippet and press `Alt+G`. MagicCursor takes a silent 600x400 screen crop and attaches it as vision context.
3. **Quick Action Pills**: Click `⚡ Summarize`, `🔍 Explain`, `✏️ Fix Grammar`, `🌐 Translate`, or `🛠️ Refactor` to run preset prompts.
4. **Instant Dismiss**: Press `Esc` or click the close button to dismiss the overlay.
