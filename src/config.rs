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
    pub rea: ReaConfig,
    #[serde(default)]
    pub overlay: OverlayConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Activation mode: "hotkey", "wiggle", or "both"
    #[serde(default = "default_activation_mode")]
    pub activation_mode: String,

    /// Global hotkey string, e.g. "Win+Shift+G", "Ctrl+Alt+G", "Alt+G"
    #[serde(default = "default_hotkey")]
    pub hotkey: String,

    /// Wiggle sensitivity: 1 (least sensitive) to 10 (most sensitive). Default is 5.
    #[serde(default = "default_wiggle_sensitivity")]
    pub wiggle_sensitivity: u32,

    /// Sliding window time in milliseconds for wiggle detection (default: 300ms)
    #[serde(default = "default_wiggle_window_ms")]
    pub wiggle_window_ms: u64,

    /// Cooldown between wiggle triggers in milliseconds (default: 1200ms)
    #[serde(default = "default_wiggle_cooldown_ms")]
    pub wiggle_cooldown_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Provider: "gemini", "openai", "anthropic", or "ollama"
    #[serde(default = "default_provider")]
    pub provider: String,

    /// API key
    #[serde(default)]
    pub api_key: String,

    /// REST endpoint URL
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// Model identifier (e.g. "gemini-2.0-flash", "gpt-4o", "claude-3-5-sonnet")
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
pub struct ReaConfig {
    /// Enable REA (Reverse Engineer Anything) MCP enrichment
    #[serde(default = "default_rea_enabled")]
    pub enabled: bool,

    /// Command to execute REA MCP or CLI
    #[serde(default = "default_rea_command")]
    pub mcp_command: String,

    /// Auto enrich detected window context with deep ASAR/DOM/binary metadata
    #[serde(default = "default_rea_auto_enrich")]
    pub auto_enrich: bool,
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

    /// Whether to auto capture screen region
    #[serde(default = "default_auto_crop")]
    pub auto_capture_screen: bool,

    /// Acrylic backdrop blur effect
    #[serde(default = "default_acrylic_blur")]
    pub acrylic_blur: bool,

    /// Dark mode UI
    #[serde(default = "default_dark_mode")]
    pub dark_mode: bool,
}

fn default_activation_mode() -> String {
    "both".to_string()
}
fn default_hotkey() -> String {
    "Win+Shift+G".to_string()
}
fn default_wiggle_sensitivity() -> u32 {
    5
}
fn default_wiggle_window_ms() -> u64 {
    300
}
fn default_wiggle_cooldown_ms() -> u64 {
    1200
}

fn default_provider() -> String {
    "gemini".to_string()
}
fn default_endpoint() -> String {
    "https://generativelanguage.googleapis.com/v1beta".to_string()
}
fn default_model() -> String {
    "gemini-2.0-flash".to_string()
}
fn default_system_prompt() -> String {
    "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. Provide concise, clear, and actionable responses. If code or text is provided, focus on the user's explicit question directly.".to_string()
}
fn default_temperature() -> f32 {
    0.7
}
fn default_max_tokens() -> u32 {
    2048
}

fn default_rea_enabled() -> bool {
    true
}
fn default_rea_command() -> String {
    "npx -y rea-agents".to_string()
}
fn default_rea_auto_enrich() -> bool {
    true
}

fn default_overlay_width() -> f32 {
    560.0
}
fn default_overlay_height() -> f32 {
    460.0
}
fn default_crop_width() -> i32 {
    600
}
fn default_crop_height() -> i32 {
    400
}
fn default_auto_crop() -> bool {
    true
}
fn default_acrylic_blur() -> bool {
    true
}
fn default_dark_mode() -> bool {
    true
}

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

impl Default for ReaConfig {
    fn default() -> Self {
        Self {
            enabled: default_rea_enabled(),
            mcp_command: default_rea_command(),
            auto_enrich: default_rea_auto_enrich(),
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
            acrylic_blur: default_acrylic_blur(),
            dark_mode: default_dark_mode(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            api: ApiConfig::default(),
            rea: ReaConfig::default(),
            overlay: OverlayConfig::default(),
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        if let Some(app_data) = dirs::data_dir() {
            app_data.join("MagicCursor")
        } else {
            PathBuf::from(".magic-cursor")
        }
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn load_or_create() -> (Self, PathBuf) {
        let path = Self::config_path();

        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str::<Config>(&content) {
                    return (cfg, path);
                }
            }
        }

        let default_cfg = Config::default();
        let _ = default_cfg.save_to(&path);
        (default_cfg, path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let toml_string = toml::to_string_pretty(self).unwrap_or_default();
        fs::write(path, toml_string)?;
        Ok(())
    }
}
