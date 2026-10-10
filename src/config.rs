use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub overlay: OverlayConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Activation mode: "hotkey", "wiggle", or "both"
    #[serde(default = "default_activation_mode")]
    pub activation_mode: String,
    
    /// Global hotkey string, e.g. "Alt+G", "Win+Alt+Space", "Ctrl+Shift+Space"
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    
    /// Wiggle sensitivity: 1 (least sensitive) to 10 (most sensitive). Default is 5.
    #[serde(default = "default_wiggle_sensitivity")]
    pub wiggle_sensitivity: u32,
    
    /// Sliding window time in milliseconds for wiggle detection (default: 250ms)
    #[serde(default = "default_wiggle_window_ms")]
    pub wiggle_window_ms: u64,

    /// Cooldown between wiggle triggers in milliseconds (default: 1200ms)
    #[serde(default = "default_wiggle_cooldown_ms")]
    pub wiggle_cooldown_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Provider: "openai", "gemini", "anthropic", or "custom"
    #[serde(default = "default_provider")]
    pub provider: String,
    
    /// API key
    #[serde(default)]
    pub api_key: String,
    
    /// REST endpoint URL
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    
    /// Model identifier
    #[serde(default = "default_model")]
    pub model: String,
    
    /// System prompt
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    
    /// Temperature for LLM response
    #[serde(default = "default_temperature")]
    pub temperature: f32,

    /// Max tokens for generation
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    /// Overlay window width in pixels
    #[serde(default = "default_overlay_width")]
    pub width: f32,
    
    /// Overlay window height in pixels
    #[serde(default = "default_overlay_height")]
    pub height: f32,
    
    /// Localized screen capture crop width
    #[serde(default = "default_crop_width")]
    pub crop_width: i32,
    
    /// Localized screen capture crop height
    #[serde(default = "default_crop_height")]
    pub crop_height: i32,

    /// Whether to auto capture screen region if clipboard is empty
    #[serde(default = "default_auto_crop")]
    pub auto_capture_screen: bool,
}

fn default_activation_mode() -> String { "both".to_string() }
fn default_hotkey() -> String { "Alt+G".to_string() }
fn default_wiggle_sensitivity() -> u32 { 5 }
fn default_wiggle_window_ms() -> u64 { 250 }
fn default_wiggle_cooldown_ms() -> u64 { 1200 }

fn default_provider() -> String { "openai".to_string() }
fn default_endpoint() -> String { "https://api.openai.com/v1/chat/completions".to_string() }
fn default_model() -> String { "gpt-4o-mini".to_string() }
fn default_system_prompt() -> String {
    "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. Provide concise, clear, and actionable responses. If code or text is provided, focus on the user's explicit question directly.".to_string()
}
fn default_temperature() -> f32 { 0.7 }
fn default_max_tokens() -> u32 { 2048 }

fn default_overlay_width() -> f32 { 520.0 }
fn default_overlay_height() -> f32 { 420.0 }
fn default_crop_width() -> i32 { 600 }
fn default_crop_height() -> i32 { 400 }
fn default_auto_crop() -> bool { true }

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            activation_mode: default_activation_mode(),
            hotkey: default_hotkey(),
            wiggle_sensitivity: default_wiggle_sensitivity(),
            wiggle_window_ms: default_wiggle_window_ms(),
            wiggle_cooldown_ms: default_wiggle_cooldown_ms(),
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            api_key: String::new(),
            endpoint: default_endpoint(),
            model: default_model(),
            system_prompt: default_system_prompt(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
        }
    }
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            width: default_overlay_width(),
            height: default_overlay_height(),
            crop_width: default_crop_width(),
            crop_height: default_crop_height(),
            auto_capture_screen: default_auto_crop(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            api: ApiConfig::default(),
            overlay: OverlayConfig::default(),
        }
    }
}

impl Config {
    /// Returns the standard config directory: %APPDATA%\MagicCursor
    pub fn config_dir() -> PathBuf {
        if let Some(appdata) = dirs::config_dir() {
            appdata.join("MagicCursor")
        } else {
            PathBuf::from("config")
        }
    }

    /// Returns the standard config file path: %APPDATA%\MagicCursor\config.toml
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Loads config from disk, or creates default with template if missing
    pub fn load_or_create() -> (Self, PathBuf) {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(config) = toml::from_str::<Config>(&content) {
                    return (config, path);
                } else {
                    eprintln!("[MagicCursor] Warning: Failed to parse config file at {:?}, using defaults.", path);
                }
            }
        }

        let default_config = Config::default();
        let _ = default_config.save_to(&path);
        (default_config, path)
    }

    /// Saves config to given path with helpful comments
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let template = format!(
r#"# ==============================================================================
# MagicCursor-Win Configuration
# Hyper-lightweight, High-Performance AI Cursor Utility for Windows
# ==============================================================================

[general]
# Activation trigger mode: "hotkey", "wiggle", or "both"
activation_mode = "{activation_mode}"

# Global hotkey shortcut. Supported modifiers: Alt, Ctrl, Shift, Win. E.g. "Alt+G", "Win+Alt+Space"
hotkey = "{hotkey}"

# Mouse wiggle sensitivity (1 = hard to trigger, 10 = very easy, 5 = balanced)
wiggle_sensitivity = {wiggle_sensitivity}

# Wiggle time window in milliseconds (default: 250)
wiggle_window_ms = {wiggle_window_ms}

# Cooldown between wiggle triggers in milliseconds
wiggle_cooldown_ms = {wiggle_cooldown_ms}

[api]
# Provider type: "openai", "gemini", "anthropic", or "custom"
provider = "{provider}"

# API Key for your LLM provider
api_key = "{api_key}"

# API Endpoint URL:
#   - OpenAI: "https://api.openai.com/v1/chat/completions"
#   - Gemini: "https://generativelanguage.googleapis.com/v1beta"
#   - Anthropic: "https://api.anthropic.com/v1/messages"
#   - Local / Ollama: "http://localhost:11434/v1/chat/completions"
endpoint = "{endpoint}"

# Model name:
#   - OpenAI: "gpt-4o-mini", "gpt-4o", "o3-mini"
#   - Gemini: "gemini-1.5-flash", "gemini-2.0-flash", "gemini-1.5-pro"
#   - Anthropic: "claude-3-5-sonnet-20241022", "claude-3-5-haiku-20241022"
#   - Ollama: "llama3.2", "mistral", "deepseek-r1:8b"
model = "{model}"

# System Prompt
system_prompt = "{system_prompt}"

# Temperature (0.0 to 1.0)
temperature = {temperature}

# Max tokens
max_tokens = {max_tokens}

[overlay]
# HUD Overlay Dimensions
width = {overlay_width}
height = {overlay_height}

# Localized Screen Crop Dimensions (when screen capture is triggered)
crop_width = {crop_width}
crop_height = {crop_height}

# Automatically take localized screen crop when clipboard is empty
auto_capture_screen = {auto_crop}
"#,
            activation_mode = self.general.activation_mode,
            hotkey = self.general.hotkey,
            wiggle_sensitivity = self.general.wiggle_sensitivity,
            wiggle_window_ms = self.general.wiggle_window_ms,
            wiggle_cooldown_ms = self.general.wiggle_cooldown_ms,
            provider = self.api.provider,
            api_key = self.api.api_key,
            endpoint = self.api.endpoint,
            model = self.api.model,
            system_prompt = self.api.system_prompt.replace('"', "\\\""),
            temperature = self.api.temperature,
            max_tokens = self.api.max_tokens,
            overlay_width = self.overlay.width,
            overlay_height = self.overlay.height,
            crop_width = self.overlay.crop_width,
            crop_height = self.overlay.crop_height,
            auto_crop = self.overlay.auto_capture_screen,
        );

        fs::write(path, template)
    }
}
