//! User configuration loaded from `%APPDATA%\MagicCursor\config.toml`.
//!
//! Every field has a `serde` default, so a partial or older config file keeps
//! working. [`Config::load_or_create`] writes a full example file on first run.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The directory name created under `%APPDATA%` to hold user configuration.
const APP_DIR_NAME: &str = "MagicCursor";

/// File name of the configuration inside the application directory.
const CONFIG_FILE_NAME: &str = "config.toml";

/// Which inputs open the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivationMode {
    /// Only the global hotkey opens the overlay.
    Hotkey,
    /// Only the cursor wiggle gesture opens the overlay.
    Wiggle,
    /// Both the hotkey and the wiggle gesture are active.
    #[default]
    Both,
}

impl ActivationMode {
    /// Whether the global hotkey should be registered.
    pub fn uses_hotkey(self) -> bool {
        matches!(self, Self::Hotkey | Self::Both)
    }

    /// Whether the low-level mouse hook should be installed.
    pub fn uses_wiggle(self) -> bool {
        matches!(self, Self::Wiggle | Self::Both)
    }
}

/// The LLM backend a completion stream is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Google Gemini `streamGenerateContent`.
    #[default]
    #[serde(alias = "google")]
    Gemini,
    /// Anthropic Messages API.
    #[serde(alias = "claude")]
    Anthropic,
    /// `OpenAI` chat completions.
    Openai,
    /// A local OpenAI-compatible server such as Ollama or vLLM.
    #[serde(alias = "vllm")]
    Ollama,
}

/// Everything a user can tune about activation, providers, and the overlay.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Activation triggers and their timing.
    #[serde(default)]
    pub general: GeneralConfig,
    /// Provider, credentials, and sampling parameters.
    #[serde(default)]
    pub api: ApiConfig,
    /// Reverse-engineering context enrichment.
    #[serde(default)]
    pub rea: ReaConfig,
    /// Overlay geometry and visual effects.
    #[serde(default)]
    pub overlay: OverlayConfig,
}

/// Activation triggers and their timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Which inputs open the overlay.
    #[serde(default)]
    pub activation_mode: ActivationMode,

    /// Global hotkey string, such as `Win+Shift+G`, `Ctrl+Alt+G`, or `Alt+G`.
    #[serde(default = "default_hotkey")]
    pub hotkey: String,

    /// Wiggle sensitivity from 1 (least sensitive) to 10 (most sensitive).
    #[serde(default = "default_wiggle_sensitivity")]
    pub wiggle_sensitivity: u32,

    /// Sliding window, in milliseconds, that the wiggle detector scores.
    #[serde(default = "default_wiggle_window_ms")]
    pub wiggle_window_ms: u64,

    /// Minimum time, in milliseconds, between two wiggle triggers.
    #[serde(default = "default_wiggle_cooldown_ms")]
    pub wiggle_cooldown_ms: u64,
}

/// Provider, credentials, and sampling parameters.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Which provider receives the request.
    #[serde(default)]
    pub provider: Provider,

    /// Bearer/API key. Never rendered by the derived output of this type.
    #[serde(default)]
    pub api_key: String,

    /// Base URL or full chat-completions URL, depending on the provider.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// Model identifier, such as `gemini-2.0-flash` or `gpt-4o`.
    #[serde(default = "default_model")]
    pub model: String,

    /// System prompt sent ahead of the selected clips.
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,

    /// Sampling temperature.
    #[serde(default = "default_temperature")]
    pub temperature: f32,

    /// Upper bound on generated tokens.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl fmt::Debug for ApiConfig {
    /// Renders the configuration with the API key redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A leaked key in a log line is worse than a slightly less useful Debug.
        let api_key = if self.api_key.is_empty() {
            ""
        } else {
            "<redacted>"
        };

        f.debug_struct("ApiConfig")
            .field("provider", &self.provider)
            .field("api_key", &api_key)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("system_prompt", &self.system_prompt)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

/// Reverse-engineering context enrichment (REA) settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReaConfig {
    /// Whether REA metadata is collected at all.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Command that starts the REA MCP server or CLI.
    #[serde(default = "default_rea_command")]
    pub mcp_command: String,

    /// Whether detected windows are enriched automatically.
    #[serde(default = "default_true")]
    pub auto_enrich: bool,
}

/// Overlay geometry and visual effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayConfig {
    /// Floating panel width in logical pixels.
    #[serde(default = "default_overlay_width")]
    pub width: f32,

    /// Floating panel height in logical pixels.
    #[serde(default = "default_overlay_height")]
    pub height: f32,

    /// Localized screen capture crop width.
    #[serde(default = "default_crop_width")]
    pub crop_width: i32,

    /// Localized screen capture crop height.
    #[serde(default = "default_crop_height")]
    pub crop_height: i32,

    /// Whether the screen is captured as soon as the overlay opens.
    #[serde(default = "default_true")]
    pub auto_capture_screen: bool,

    /// Whether the acrylic backdrop blur is drawn.
    #[serde(default = "default_true")]
    pub acrylic_blur: bool,

    /// Whether the dark Fluent palette is used.
    #[serde(default = "default_true")]
    pub dark_mode: bool,
}

/// A configuration file that could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// The file system rejected the read or write.
    Io(io::Error),
    /// The file exists but is not valid TOML for this schema.
    Parse(toml::de::Error),
    /// The in-memory configuration could not be serialized.
    Serialize(toml::ser::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "config file access failed: {err}"),
            Self::Parse(err) => write!(f, "config file is not valid TOML: {err}"),
            Self::Serialize(err) => write!(f, "config could not be serialized: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Parse(err) => Some(err),
            Self::Serialize(err) => Some(err),
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> Self {
        Self::Parse(err)
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(err: toml::ser::Error) -> Self {
        Self::Serialize(err)
    }
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

fn default_endpoint() -> String {
    "https://generativelanguage.googleapis.com/v1beta".to_string()
}

fn default_model() -> String {
    "gemini-2.0-flash".to_string()
}

fn default_system_prompt() -> String {
    "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. \
     Provide concise, clear, and actionable responses. If code or text is provided, focus on the \
     user's explicit question directly."
        .to_string()
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tokens() -> u32 {
    2048
}

fn default_rea_command() -> String {
    "npx -y rea-agents".to_string()
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

fn default_true() -> bool {
    true
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            activation_mode: ActivationMode::default(),
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
            provider: Provider::default(),
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
            enabled: default_true(),
            mcp_command: default_rea_command(),
            auto_enrich: default_true(),
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
            auto_capture_screen: default_true(),
            acrylic_blur: default_true(),
            dark_mode: default_true(),
        }
    }
}

impl Config {
    /// Returns `%APPDATA%\MagicCursor`, or a working-directory folder when the
    /// platform data directory is unavailable.
    pub fn config_dir() -> PathBuf {
        dirs::data_dir().map_or_else(
            || PathBuf::from(".magic-cursor"),
            |app_data| app_data.join(APP_DIR_NAME),
        )
    }

    /// Returns the full path of `config.toml` inside [`Config::config_dir`].
    pub fn config_path() -> PathBuf {
        Self::config_dir().join(CONFIG_FILE_NAME)
    }

    /// Loads the config file, falling back to defaults when it is missing or invalid.
    ///
    /// A missing file is created so the user has something to edit. Parse and IO
    /// failures are reported on stderr and replaced with defaults rather than
    /// aborting the tray application.
    pub fn load_or_create() -> (Self, PathBuf) {
        let path = Self::config_path();

        match Self::load_from(&path) {
            Ok(config) => (config, path),
            Err(err) => {
                eprintln!("[magic-cursor] {err}; using defaults");
                let config = Self::default();
                if let Err(save_err) = config.save_to(&path) {
                    eprintln!("[magic-cursor] {save_err}");
                }
                (config, path)
            }
        }
    }

    /// Reads and parses a configuration file.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    /// Writes this configuration as pretty-printed TOML, creating the parent directory.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[general]
activation_mode = "wiggle"
hotkey = "Ctrl+Alt+G"

[api]
provider = "anthropic"
api_key = "secret-value"
model = "claude-3-5-sonnet-20241022"

[rea]
enabled = false

[overlay]
width = 620.0
"#;

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let config: Config = toml::from_str("[general]\nhotkey = \"Alt+G\"\n").expect("valid toml");

        assert_eq!(config.general.hotkey, "Alt+G");
        assert_eq!(config.general.activation_mode, ActivationMode::Both);
        assert_eq!(config.general.wiggle_window_ms, 300);
        assert_eq!(config.api.max_tokens, 2048);
        assert!(config.rea.enabled);
    }

    #[test]
    fn parses_every_example_value() {
        let config: Config = toml::from_str(EXAMPLE).expect("valid toml");

        assert_eq!(config.general.activation_mode, ActivationMode::Wiggle);
        assert!(!config.general.activation_mode.uses_hotkey());
        assert_eq!(config.api.provider, Provider::Anthropic);
        assert!(!config.rea.enabled);
        assert!((config.overlay.width - 620.0).abs() < f32::EPSILON);
    }

    #[test]
    fn provider_accepts_vendor_aliases() {
        let google: Config = toml::from_str("[api]\nprovider = \"google\"\n").expect("valid toml");
        let claude: Config = toml::from_str("[api]\nprovider = \"claude\"\n").expect("valid toml");

        assert_eq!(google.api.provider, Provider::Gemini);
        assert_eq!(claude.api.provider, Provider::Anthropic);
    }

    #[test]
    fn unknown_activation_mode_is_rejected() {
        let result = toml::from_str::<Config>("[general]\nactivation_mode = \"telepathy\"\n");

        assert!(result.is_err(), "unknown modes must not silently pass");
    }

    #[test]
    fn debug_output_never_contains_the_api_key() {
        let config = Config {
            api: ApiConfig {
                api_key: "sk-super-secret".to_string(),
                ..ApiConfig::default()
            },
            ..Config::default()
        };

        let rendered = format!("{config:?}");

        assert!(rendered.contains("<redacted>"), "got {rendered}");
        assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).expect("serializable");
        let parsed: Config = toml::from_str(&text).expect("parsable");

        assert_eq!(parsed, config);
    }

    #[test]
    fn save_creates_missing_directories() {
        let dir = std::env::temp_dir().join("magic-cursor-config-test");
        let path = dir.join("nested").join("config.toml");
        let config = Config::default();

        config.save_to(&path).expect("write succeeds");
        let loaded = Config::load_from(&path).expect("read succeeds");

        assert_eq!(loaded, config);
        let _ = fs::remove_dir_all(&dir);
    }
}
