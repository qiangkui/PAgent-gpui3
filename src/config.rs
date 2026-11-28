//! The assistant's model connections, the palette and the storage locations,
//! kept in `config.json` next to the program.
//!
//! Every provider the window supports speaks the OpenAI-compatible
//! `/chat/completions` shape, so a connection is just "which endpoint, which key,
//! which model". The window owns them and hands the active one to the engine's
//! `ai` bridge per request; nothing here talks to a model. Keys are never part of
//! the repository: they live in the Windows Credential Manager and only fall back
//! to that `config.json` when the store refuses the write. The palette (`theme` /
//! `colors`) is in the same file, so copying the program folder carries the
//! window's look with it.

use std::path::{Path, PathBuf};

use crate::secrets::{SecretStore, TARGET_PREFIX};

/// A built-in endpoint the settings page can offer.
pub struct Preset {
    /// Stable key stored with the connection.
    pub id: &'static str,
    /// Name shown on the preset chip and on the connection card.
    pub label: &'static str,
    /// Endpoint base, without `/chat/completions`.
    pub base_url: &'static str,
    /// Model filled in when the preset is added.
    pub model: &'static str,
    /// Model versions the settings page offers as one-click choices. The model
    /// field still accepts anything typed by hand.
    pub models: &'static [&'static str],
}

/// Endpoints the window ships with. All of them are OpenAI compatible.
pub const PRESETS: [Preset; 6] = [
    Preset {
        id: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com",
        model: "deepseek-flash",
        // The 2026 lineup: thinking is a mode of these two, not a separate
        // `deepseek-reasoner` model. The legacy ids still resolve for older
        // keys, but they are not offered as choices.
        models: &["deepseek-flash", "deepseek-v4-pro"],
    },
    Preset {
        id: "openai",
        label: "OpenAI",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        models: &["gpt-4o-mini", "gpt-4o", "o4-mini"],
    },
    Preset {
        id: "moonshot",
        label: "Moonshot Kimi",
        base_url: "https://api.moonshot.cn/v1",
        model: "moonshot-v1-8k",
        models: &["moonshot-v1-8k", "moonshot-v1-32k", "kimi-latest"],
    },
    Preset {
        id: "dashscope",
        label: "阿里通义",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        model: "qwen-plus",
        models: &["qwen-plus", "qwen-max", "qwen-turbo"],
    },
    Preset {
        id: "zhipu",
        label: "智谱 GLM",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        model: "glm-4-flash",
        models: &["glm-4-flash", "glm-4-plus", "glm-4-air"],
    },
    Preset {
        id: "custom",
        label: "自定义（OpenAI 兼容）",
        base_url: "",
        model: "",
        models: &[],
    },
];

/// Look a preset up by id.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

/// Model versions a provider offers, empty when the provider is not a preset.
pub fn model_versions(provider: &str) -> &'static [&'static str] {
    preset(provider).map_or(&[], |preset| preset.models)
}

/// The models worth offering for one connection: the shipped list, then
/// whatever the endpoint reported that the list does not already have.
///
/// The endpoint's answer cannot be the only source. A gateway may hide a model,
/// an older fetch predates one, and a custom entry can point at a preset host —
/// in all three cases the picker would otherwise be missing a model the provider
/// does offer. The shipped list stays authoritative for order, the fetch adds
/// the rest.
///
/// @param provider - the connection's preset id, for example `deepseek`.
/// @param base_url - the endpoint base; a DeepSeek address counts even when the
///   connection was created as a custom OpenAI-compatible one.
/// @param fetched - model ids the endpoint reported the last time it was asked.
pub fn offered_models(provider: &str, base_url: &str, fetched: &[String]) -> Vec<String> {
    let mut shipped: Vec<&str> = model_versions(provider).to_vec();
    if shipped.is_empty() && base_url.to_ascii_lowercase().contains("deepseek") {
        shipped = preset("deepseek").map_or(Vec::new(), |preset| preset.models.to_vec());
    }
    let mut names: Vec<String> = shipped.iter().map(|name| (*name).to_string()).collect();
    for name in fetched {
        if !names.iter().any(|known| known == name) {
            names.push(name.clone());
        }
    }
    names
}

/// Temperature presets an older `config.json` may still name.
///
/// The slider stores the figure itself, so this only exists to read a file
/// written before that: each name maps to the number it used to send.
pub fn legacy_temperature(name: &str) -> Option<f64> {
    match name {
        "precise" => Some(0.2),
        "balanced" => Some(0.7),
        "loose" => Some(1.0),
        _ => None,
    }
}

/// Whether the model thinks before it answers, and how hard.
///
/// DeepSeek's `deepseek-flash` and `deepseek-v4-pro` think by default. The API
/// switches thinking with `{"thinking": {"type": "disabled"}}` and trades depth
/// for latency with `reasoning_effort` (`low` / `high` / `max`). Other endpoints
/// take neither, so [`Self::Default`] sends nothing and leaves the choice to the
/// provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reasoning {
    /// Send nothing: the endpoint's own default (DeepSeek: thinking on, effort
    /// high).
    #[default]
    Default,
    /// Think briefly, so easy questions come back sooner.
    Low,
    /// The effort the endpoint picks when thinking is on and nothing else is
    /// said.
    High,
    /// Spend the most on thinking, for questions worth the wait.
    Max,
    /// Answer without thinking: faster and cheaper, and nothing to show in the
    /// transcript's thinking rows.
    Off,
}

impl Reasoning {
    /// The name written to `config.json` and sent to the engine.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Low => "low",
            Self::High => "high",
            Self::Max => "max",
            Self::Off => "off",
        }
    }

    /// Read a stored name; unknown names mean the endpoint default.
    pub fn from_name(name: &str) -> Self {
        match name {
            "low" => Self::Low,
            "high" => Self::High,
            "max" => Self::Max,
            "off" => Self::Off,
            _ => Self::Default,
        }
    }
}

/// One configured model endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    /// Stable id the key is filed under in the credential store (`c1`, `c2`, …).
    pub id: String,
    /// Id of the preset this came from, for display only.
    pub provider: String,
    /// Model identifier sent to the provider.
    pub model: String,
    /// Bearer token.
    pub api_key: String,
    /// Endpoint base, without `/chat/completions`.
    pub base_url: String,
    /// True once a test call has answered, which is what makes it selectable.
    pub verified: bool,
    /// Model ids the endpoint reported the last time it was asked, so the
    /// versions on offer survive a restart without another request.
    pub models: Vec<String>,
    /// Temperature sent with every question; `None` leaves it to the endpoint.
    pub temperature: Option<f64>,
    /// Whether the model thinks first; `Default` sends no switch at all.
    pub reasoning: Reasoning,
    /// Cap on generated tokens; zero leaves it to the endpoint.
    pub max_tokens: u32,
    /// Context window in tokens, when the endpoint's own figure is known better
    /// than the name table in [`crate::ai::context_window`]; zero means auto.
    ///
    /// A local or self-hosted model is usually much smaller than the 128k
    /// default, and a model newer than the table is usually larger; either way
    /// the compaction point would be wrong without this.
    pub window: u64,
    /// Whether the endpoint accepts images, which is what makes `read_image` and
    /// `screenshot` worth declaring. A text-only model rejects an `image_url`
    /// part outright, so the two tools are left out of its requests.
    pub vision: bool,
}

impl Connection {
    /// A connection pre-filled from a preset.
    pub fn from_preset(id: &str) -> Option<Self> {
        let preset = preset(id)?;
        Some(Self {
            id: String::new(),
            provider: preset.id.to_string(),
            model: preset.model.to_string(),
            api_key: String::new(),
            base_url: preset.base_url.to_string(),
            verified: false,
            models: Vec::new(),
            temperature: None,
            reasoning: Reasoning::Default,
            max_tokens: 0,
            window: 0,
            // Presets are DeepSeek's current models, which are the vision-capable
            // ones; a text-only endpoint is a switch away in the settings.
            vision: true,
        })
    }

    /// The window the engine should compact against.
    ///
    /// An explicit setting wins; otherwise the name table answers, with its
    /// conservative default for models it does not know.
    pub fn window_tokens(&self) -> u64 {
        if self.window > 0 {
            self.window
        } else {
            crate::ai::context_window(&self.model)
        }
    }

    /// Whether this connection can declare DeepSeek's server tools.
    ///
    /// Only DeepSeek's `deepseek-flash` offers `deepseek_web_search` and
    /// `deepseek_read_url`; sending those entries anywhere else is a rejected
    /// request, so the payload leaves them out instead.
    pub fn supports_web_search(&self) -> bool {
        let host = self.base_url.to_ascii_lowercase();
        let deepseek = self.provider == "deepseek" || host.contains("deepseek");
        deepseek && self.model.to_ascii_lowercase().contains("flash")
    }

    /// Name of the provider, for the picker and the card header.
    pub fn provider_label(&self) -> &'static str {
        preset(&self.provider).map_or("自定义", |preset| preset.label)
    }

    /// Whether the three fields are filled in enough to try a request.
    pub fn ready(&self) -> bool {
        !self.api_key.trim().is_empty()
            && !self.base_url.trim().is_empty()
            && !self.model.trim().is_empty()
    }

    /// Trim the editable fields before storing or sending.
    pub fn normalized(&self) -> Self {
        Self {
            id: self.id.clone(),
            provider: self.provider.clone(),
            model: self.model.trim().to_string(),
            api_key: self.api_key.trim().to_string(),
            base_url: self.base_url.trim().trim_end_matches('/').to_string(),
            verified: self.verified,
            models: self.models.clone(),
            // A hand-edited file could ask for a temperature no endpoint takes;
            // 0–2 is the range every provider reads as "reasonable".
            temperature: self
                .temperature
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0., 2.)),
            reasoning: self.reasoning,
            max_tokens: self.max_tokens,
            // A hand-edited file could ask for an absurd window; the engine
            // clamps its own budgets anyway, but keeping the figure sane here
            // stops a bad number from reaching the gauge.
            window: self.window.min(10_000_000),
            vision: self.vision,
        }
    }
}

/// Default model round-trips per question: the highest the menu offers, so a
/// fresh conversation is never cut short by a budget nobody chose.
pub const DEFAULT_MAX_ROUNDS: u32 = MAX_ROUNDS_CHOICE;

/// Default tool calls per question, likewise the highest on offer.
pub const DEFAULT_MAX_CALLS: u32 = MAX_CALLS_CHOICE;

/// Highest budgets the window offers; the engine clamps again (`MAX_ROUNDS_LIMIT`
/// there is 50 too, and 200 calls).
pub const MAX_ROUNDS_CHOICE: u32 = 50;
/// Highest call budget the window offers.
pub const MAX_CALLS_CHOICE: u32 = 200;

/// Most plugins the window will list, so a stray folder cannot flood the page.
pub const MAX_PLUGINS: usize = 64;

/// Whether a folder name is one this window will hand to the engine.
///
/// The name ends up in a request and in a path, so it is kept to what a folder
/// name can safely be: no separators, no dots that walk upwards.
pub fn is_plugin_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && !is_reserved_name(name)
}

/// Most commands one review list may hold.
pub const MAX_REVIEW_PREFIXES: usize = 64;

/// Longest command prefix the review list keeps, in characters.
pub const MAX_REVIEW_PREFIX: usize = 200;

/// Normalize one `reviewAllow` entry, or drop it.
///
/// A prefix is matched against a whole command line, so a control character
/// (a stray newline from a pasted list) would match nothing at all and is
/// refused here rather than stored and silently useless.
pub fn review_prefix(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_REVIEW_PREFIX {
        return None;
    }
    if trimmed.chars().any(char::is_control) {
        return None;
    }
    Some(trimmed.to_string())
}

/// Whether Windows reserves the name for a device (`con`, `nul`, `com1`, …).
///
/// Such a name cannot be created as a folder or a file, so a plugin or skill
/// carrying it could never be written. The extension is ignored, because
/// `con.json` is refused just like `con`.
pub fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim();
    if stem.eq_ignore_ascii_case("con")
        || stem.eq_ignore_ascii_case("prn")
        || stem.eq_ignore_ascii_case("aux")
        || stem.eq_ignore_ascii_case("nul")
    {
        return true;
    }
    // `com1`…`com9`, `lpt1`…`lpt9`: three letters and one non-zero digit.
    let mut characters = stem.chars();
    let prefix: String = characters.by_ref().take(3).collect();
    let digit = characters.next();
    (prefix.eq_ignore_ascii_case("com") || prefix.eq_ignore_ascii_case("lpt"))
        && matches!(digit, Some('1'..='9'))
        && characters.next().is_none()
}

/// Whether a name is one the MCP page will hand to the engine.
///
/// Like a plugin id, with spaces allowed: MCP clients commonly name servers
/// `my server`, and the name never becomes a path.
pub fn is_server_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ')
        })
}

/// Whether the program may keep its files in `dir`.
///
/// Tested by writing, because a folder can look writable and still refuse:
/// `Program Files` allows reading and denies creating.
fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".dsh-flow-write-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// One plugin folder found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// Folder name, which is the plugin's identity.
    pub id: String,
    /// Name from `plugin.json`, or the folder name when it has none.
    pub name: String,
    /// Version from `plugin.json`, empty when it has none.
    pub version: String,
    /// Why the folder cannot be used; empty when it looks usable.
    pub problem: String,
}

/// Delete one plugin folder, if it exists.
///
/// The id is a direct child of the plugin folder; the caller checks it with
/// [`is_plugin_id`] before reaching here, and so does this function.
pub fn delete_plugin(id: &str) -> Result<(), String> {
    let Some(root) = AiSettings::plugin_dir() else {
        return Err("找不到插件目录".to_string());
    };
    delete_plugin_in(&root, id)
}

/// The same deletion against an arbitrary folder (the tests use this one).
pub fn delete_plugin_in(dir: &Path, id: &str) -> Result<(), String> {
    if !is_plugin_id(id) {
        return Err(format!("插件 id「{id}」不合法"));
    }
    let folder = dir.join(id);
    if !folder.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&folder)
        .map_err(|error| format!("无法删除 {}：{error}", folder.display()))
}

/// List the plugin folders and whatever their `plugin.json` says.
pub fn installed_plugins() -> Vec<InstalledPlugin> {
    AiSettings::plugin_dir()
        .map(|dir| scanned_plugins(&dir))
        .unwrap_or_default()
}

/// The same scan against an arbitrary folder.
///
/// A folder that cannot be read is still listed, with the reason: "it does not
/// show up" is the hardest kind of problem to diagnose.
pub fn scanned_plugins(dir: &Path) -> Vec<InstalledPlugin> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<InstalledPlugin> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if !is_plugin_id(&id) {
            continue;
        }
        found.push(read_plugin(&id, &entry.path()));
    }
    found.sort_by(|left, right| left.id.cmp(&right.id));
    found.truncate(MAX_PLUGINS);
    found
}

/// Read one plugin folder's manifest the way the engine will read it.
///
/// Shared by the page's scan and by the import, so a folder the page would call
/// broken can never be copied in as if it worked.
fn read_plugin(id: &str, folder: &Path) -> InstalledPlugin {
    let empty = |problem: String| InstalledPlugin {
        id: id.to_string(),
        name: id.to_string(),
        version: String::new(),
        problem,
    };
    let text = match std::fs::read_to_string(folder.join("plugin.json")) {
        Ok(text) => text,
        Err(_) => return empty("缺少 plugin.json".to_string()),
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return empty("plugin.json 不是合法 JSON".to_string());
    };
    let field = |name: &str| {
        value
            .get(name)
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let api = value.get("api").and_then(|item| item.as_u64()).unwrap_or(0);
    let main = match field("main") {
        entry if entry.is_empty() => "index.js".to_string(),
        entry => entry,
    };
    let name = match field("name") {
        printed if printed.is_empty() => id.to_string(),
        printed => printed,
    };
    let problem = if api != 1 {
        format!("plugin.json 的 api 是 {api}，这个版本只支持 1")
    } else if escapes_folder(&main) {
        // The engine refuses this too; catching it here keeps a broken folder
        // from being imported and only failing at the next question.
        "plugin.json 的 main 指向了插件目录之外".to_string()
    } else if !folder.join(&main).is_file() {
        format!("找不到入口文件 {main}")
    } else {
        String::new()
    };
    InstalledPlugin {
        id: id.to_string(),
        name,
        version: field("version"),
        problem,
    }
}

/// Whether a manifest's `main` leaves the plugin folder.
fn escapes_folder(main: &str) -> bool {
    main.starts_with('/')
        || main.starts_with('\\')
        || main.contains(':')
        || main.split(['/', '\\']).any(|part| part == "..")
}

/// Copy a plugin folder in from anywhere on disk, then return its id.
///
/// Same rules as the scan: the folder name is the id, the manifest has to be
/// one the engine would load, and an existing id is never overwritten.
pub fn import_plugin(source: &Path) -> Result<String, String> {
    let Some(dir) = AiSettings::plugin_dir() else {
        return Err("找不到插件目录".to_string());
    };
    import_plugin_into(source, &dir)
}

/// The same import against an arbitrary folder (the tests use this one).
pub fn import_plugin_into(source: &Path, dir: &Path) -> Result<String, String> {
    if !source.is_dir() {
        return Err("请选择一个插件文件夹（里面放 plugin.json）。".to_string());
    }
    let Some(id) = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
    else {
        return Err(format!("读不出 {} 的文件夹名", source.display()));
    };
    if !is_plugin_id(&id) {
        return Err(format!(
            "文件夹名「{id}」不能当插件 id：只能用字母、数字、-、_、.，且不能以 . 开头或结尾"
        ));
    }
    let plugin = read_plugin(&id, source);
    if !plugin.problem.is_empty() {
        return Err(format!("这个文件夹不是可用的插件：{}", plugin.problem));
    }
    if scanned_plugins(dir).len() >= MAX_PLUGINS {
        return Err(format!("插件已经有 {MAX_PLUGINS} 个，先删掉一个再添加"));
    }
    let target = dir.join(&id);
    if target.exists() {
        return Err(format!("插件「{id}」已经存在，先删除它再添加"));
    }
    crate::folder::copy_dir(source, &target)?;
    Ok(id)
}

/// Highest number of custom modes the settings page keeps.
pub const MAX_MODES: usize = 32;

/// Built-in mode that sends only the engine's own tools.
pub const MODE_BASIC: &str = "basic";
/// Built-in mode that sends every globally enabled external plugin, skill and MCP server.
pub const MODE_FULL: &str = "full";

/// One named selection of external resources.
///
/// Built-in basic/full modes are implicit; only custom modes are stored. A mode
/// keeps ids even when the resource is globally switched off: switching it back
/// on restores it here, while the request builders intersect with the global
/// enabled lists so an off resource is never sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMode {
    /// Stable id referenced by each conversation.
    pub id: String,
    /// Name shown in settings and the composer.
    pub name: String,
    /// External plugin folder ids.
    pub plugins: Vec<String>,
    /// Skill folder ids.
    pub skills: Vec<String>,
    /// MCP server ids.
    pub mcp_servers: Vec<String>,
}

impl ChatMode {
    /// A blank custom mode.
    pub fn empty(id: String, name: String) -> Self {
        Self {
            id,
            name,
            plugins: Vec::new(),
            skills: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }
}

/// The resources one request actually carries, after intersecting a mode with
/// the user's global switches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeSelection {
    /// Enabled external plugins to load.
    pub plugins: Vec<String>,
    /// Enabled skills whose instructions go into the system prompt.
    pub skills: Vec<String>,
    /// Enabled MCP servers to open.
    pub mcp_servers: Vec<String>,
}

/// Every connection the window knows about, plus which one is in use.
#[derive(Clone, Debug, PartialEq)]
pub struct AiSettings {
    /// Configured endpoints, in the order the settings page shows them.
    pub connections: Vec<Connection>,
    /// Index into [`Self::connections`] of the model used for new questions.
    pub active: usize,
    /// Palette in use: `"light"`, `"dark"`, or empty for the default.
    pub theme: String,
    /// Custom colours laid over the palette, in the order they were saved.
    ///
    /// Only the roles the editor offers are kept; a key the theme no longer
    /// knows is dropped on save rather than carried around forever.
    pub colors: Vec<(String, u32)>,
    /// Plugin folder names that are switched on.
    ///
    /// Only these load: a plugin runs next to the engine with its privileges, so
    /// dropping one in the folder is not enough — someone has to turn it on.
    pub plugins: Vec<String>,
    /// MCP server ids that are switched on.
    ///
    /// A server is another process the engine may start, so like a plugin it
    /// stays off until someone turns it on in the MCP page.
    pub mcp_servers: Vec<String>,
    /// Skill folder names that are switched on.
    ///
    /// A skill is prompt text injected on every question, so it stays off until
    /// someone turns it on in the Skill page.
    pub skills: Vec<String>,
    /// Custom modes, in the order the settings page shows them.
    pub modes: Vec<ChatMode>,
    /// Tokens one conversation may spend before the status strip warns; zero
    /// leaves it unlimited.
    pub budget_tokens: u64,
    /// HTTP proxy the engine's provider calls go through; empty is a direct
    /// connection. Applied to the child processes at startup.
    pub proxy: String,
    /// Extra commands the automatic review may run without a card.
    ///
    /// Each entry is a prefix matched against the whole command line, on top of
    /// the built-in read-only list. It carries the same trust as the list: the
    /// binary and every argument under it run unattended, so `npm test` is a
    /// reasonable entry and `npm` is not.
    pub review_allow: Vec<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            connections: Connection::from_preset("deepseek").into_iter().collect(),
            active: 0,
            theme: String::new(),
            colors: Vec::new(),
            plugins: Vec::new(),
            mcp_servers: Vec::new(),
            skills: Vec::new(),
            modes: Vec::new(),
            budget_tokens: 0,
            proxy: String::new(),
            review_allow: Vec::new(),
        }
    }
}

impl AiSettings {
    /// The connection new questions go through.
    pub fn active(&self) -> Option<&Connection> {
        self.connections.get(self.active)
    }

    /// Where conversations keep their files: `<program folder>\workspace`.
    ///
    /// Fixed next to the executable so the whole installation (config, data,
    /// workspaces, plugins) moves as one folder; there is no setting for it.
    pub fn workspace_dir(&self) -> PathBuf {
        Self::default_workspace()
    }

    /// The folder conversations keep their files in.
    pub fn default_workspace() -> PathBuf {
        Self::home().join("workspace")
    }

    /// Where the window keeps its records: `<program folder>\data`.
    pub fn data_dir(&self) -> PathBuf {
        Self::default_data_dir()
    }

    /// The folder holding `chats`, `workflows` and `sidebar.json`.
    pub fn default_data_dir() -> PathBuf {
        Self::home().join("data")
    }

    /// Drop every custom colour: the theme is then used exactly as it ships.
    pub fn clear_colors(&mut self) {
        self.colors.clear();
    }

    /// The custom colour stored for `key`, if the reader has replaced it.
    pub fn color_of(&self, key: &str) -> Option<u32> {
        let wanted = crate::theme::color_role(key)?.key;
        self.colors
            .iter()
            .find(|(seen, _)| seen == wanted)
            .map(|(_, value)| *value)
    }

    /// Replace one custom colour, or drop it and go back to the theme's own.
    ///
    /// A colour equal to the theme's is stored as "no override", so the file
    /// only ever lists what the reader actually changed.
    /// @returns the number of colours that differ from the theme, or `None` when
    /// the key is not one the editor offers.
    pub fn set_color(
        &mut self,
        key: &str,
        value: Option<u32>,
        theme: crate::theme::ThemeKind,
    ) -> Option<usize> {
        let role = crate::theme::color_role(key)?;
        self.colors.retain(|(seen, _)| seen != role.key);
        if let Some(color) = value
            && (role.get)(&theme.palette()) != color
        {
            self.colors.push((role.key.to_string(), color));
        }
        Some(self.colors.len())
    }

    /// An id no other connection uses, so a key can never land on someone else's.
    pub fn fresh_connection_id(existing: &[Connection]) -> String {
        let used: Vec<String> = existing
            .iter()
            .map(|connection| connection.id.clone())
            .collect();
        fresh_id(&used)
    }

    /// The connections a test call has answered, with their indexes.
    pub fn verified(&self) -> Vec<(usize, &Connection)> {
        self.connections
            .iter()
            .enumerate()
            .filter(|(_, connection)| connection.verified)
            .collect()
    }

    /// True when the model new questions would use has been proven to answer.
    pub fn usable(&self) -> bool {
        self.active().is_some_and(|connection| connection.verified)
    }

    /// Read the stored settings, falling back to defaults when nothing is saved.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        // A build that used to keep its settings in the profile folder hands
        // them over the first time it runs from a writable program folder, so
        // an upgrade does not look like a fresh install.
        Self::adopt_legacy(&path);
        let (settings, rewrite) = Self::load_with(&path, crate::secrets::system());
        if rewrite {
            // An id was handed out, or a key still lived in the file: saving
            // moves the key into the credential store and drops it from the file.
            let _ = settings.save();
        }
        settings
    }

    /// Copy the old profile settings into place when the new file is missing.
    ///
    /// The data and workspace paths inside are left as they are: relocating
    /// someone's conversations is their decision, and the settings page offers
    /// a button that does it.
    fn adopt_legacy(path: &Path) {
        if path.exists() {
            return;
        }
        let Some(profile) = Self::profile_dir() else {
            return;
        };
        let legacy = profile.join("config.json");
        if legacy == *path || !legacy.is_file() {
            return;
        }
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::copy(&legacy, path);
    }

    /// Write the settings next to the program.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "找不到用户目录，无法保存设置".to_string())?;
        self.save_with(&path, crate::secrets::system())
    }

    /// The folder the program keeps everything in: its own directory.
    ///
    /// Config, plugins, conversations and workspaces all live here, so moving
    /// (or zipping) the program folder moves the whole installation. A folder
    /// the program cannot write to — inside `Program Files`, on a read-only
    /// volume — falls back to the old profile location instead of failing to
    /// start, because a window that cannot save is worse than one that is not
    /// portable.
    pub fn home() -> PathBuf {
        static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        HOME.get_or_init(|| {
            if let Some(dir) = Self::program_dir().filter(|dir| writable(dir)) {
                return dir;
            }
            Self::profile_dir().unwrap_or_else(|| std::env::temp_dir().join("dsh-flow-ui"))
        })
        .clone()
    }

    /// The directory the executable sits in, when it can be found.
    fn program_dir() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        exe.parent().map(Path::to_path_buf)
    }

    /// The folder older builds used: `%APPDATA%\dsh-flow-ui`.
    fn profile_dir() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .or_else(|| std::env::var_os("HOME"))?;
        Some(PathBuf::from(base).join("dsh-flow-ui"))
    }

    /// Where the settings file lives: `config.json` next to the program.
    fn path() -> Option<PathBuf> {
        Some(Self::home().join("config.json"))
    }

    /// Folder holding the installed plugins, one folder each.
    ///
    /// Next to the settings file rather than in the workspace: a plugin belongs
    /// to the program, not to one conversation's files.
    pub fn plugin_dir() -> Option<PathBuf> {
        Some(Self::home().join("plugins"))
    }

    /// Folder holding the installed skills, one folder each.
    ///
    /// Next to the program like the plugin and MCP folders: a skill belongs to
    /// the program, not to one conversation.
    pub fn skill_dir() -> Option<PathBuf> {
        Some(Self::home().join("skills"))
    }

    /// Whether a skill folder is switched on.
    pub fn skill_enabled(&self, id: &str) -> bool {
        self.skills.iter().any(|entry| entry == id)
    }

    /// Whether a plugin folder is switched on.
    pub fn plugin_enabled(&self, id: &str) -> bool {
        self.plugins.iter().any(|entry| entry == id)
    }

    /// Whether an MCP server is switched on.
    pub fn mcp_server_enabled(&self, id: &str) -> bool {
        self.mcp_servers.iter().any(|entry| entry == id)
    }

    /// Switch an MCP server on or off.
    /// @returns true when the list changed.
    pub fn set_mcp_server(&mut self, id: &str, enabled: bool) -> bool {
        if !is_server_id(id) {
            return false;
        }
        let before = self.mcp_servers.len();
        self.mcp_servers.retain(|entry| entry != id);
        if enabled {
            self.mcp_servers.push(id.to_string());
            self.mcp_servers.sort_unstable();
        }
        before != self.mcp_servers.len() || enabled
    }

    /// Switch a plugin folder on or off.
    /// @returns true when the list changed.
    pub fn set_plugin(&mut self, id: &str, enabled: bool) -> bool {
        if !is_plugin_id(id) {
            return false;
        }
        let before = self.plugins.len();
        self.plugins.retain(|entry| entry != id);
        if enabled {
            self.plugins.push(id.to_string());
            self.plugins.sort_unstable();
        }
        before != self.plugins.len() || enabled
    }

    /// Name of a built-in or custom mode.
    pub fn mode_label(&self, id: &str) -> String {
        if id == MODE_BASIC {
            return "基础模式".to_string();
        }
        if id == MODE_FULL {
            return "全量模式".to_string();
        }
        self.modes.iter().find(|mode| mode.id == id).map_or_else(
            || "全量模式".to_string(),
            |mode| {
                if mode.name.trim().is_empty() {
                    "自定义模式".to_string()
                } else {
                    mode.name.clone()
                }
            },
        )
    }

    /// Whether a mode id is known. Unknown ids fall back to full mode.
    pub fn mode_exists(&self, id: &str) -> bool {
        id == MODE_BASIC || id == MODE_FULL || self.modes.iter().any(|mode| mode.id == id)
    }

    /// The selection a request with `mode_id` actually carries.
    ///
    /// The global switches are applied last: a resource the user turned off in
    /// its own page is never sent, even if a custom mode still lists it.
    pub fn mode_selection(&self, mode_id: &str) -> ModeSelection {
        if mode_id == MODE_BASIC {
            return ModeSelection::default();
        }
        let mode = self.modes.iter().find(|mode| mode.id == mode_id);
        let plugins = mode.map_or_else(
            || self.plugins.clone(),
            |mode| {
                self.plugins
                    .iter()
                    .filter(|id| mode.plugins.iter().any(|entry| entry == *id))
                    .cloned()
                    .collect()
            },
        );
        let skills = mode.map_or_else(
            || self.skills.clone(),
            |mode| {
                self.skills
                    .iter()
                    .filter(|id| mode.skills.iter().any(|entry| entry == *id))
                    .cloned()
                    .collect()
            },
        );
        let mcp_servers = mode.map_or_else(
            || self.mcp_servers.clone(),
            |mode| {
                self.mcp_servers
                    .iter()
                    .filter(|id| mode.mcp_servers.iter().any(|entry| entry == *id))
                    .cloned()
                    .collect()
            },
        );
        ModeSelection {
            plugins,
            skills,
            mcp_servers,
        }
    }

    /// A custom mode id no other mode uses.
    pub fn fresh_mode_id(&self) -> String {
        let mut used: Vec<String> = vec![MODE_BASIC.to_string(), MODE_FULL.to_string()];
        used.extend(self.modes.iter().map(|mode| mode.id.clone()));
        let mut next = self.modes.len() + 1;
        loop {
            let candidate = format!("mode-{next}");
            if !used.iter().any(|seen| seen == &candidate) {
                return candidate;
            }
            next += 1;
        }
    }

    /// Switch a skill folder on or off.
    /// @returns true when the list changed.
    pub fn set_skill(&mut self, id: &str, enabled: bool) -> bool {
        if !is_plugin_id(id) {
            return false;
        }
        let before = self.skills.len();
        self.skills.retain(|entry| entry != id);
        if enabled {
            self.skills.push(id.to_string());
            self.skills.sort_unstable();
        }
        before != self.skills.len() || enabled
    }

    /// Read settings from an arbitrary file, tolerating a missing or broken one.
    ///
    /// Older builds stored one flat `{model, apiKey, baseUrl}`; that shape is
    /// folded into a single DeepSeek connection instead of being dropped.
    ///
    /// Only tests use this: the window always goes through the credential store.
    #[cfg(test)]
    fn load_from(path: &Path) -> Self {
        Self::parse_file(path)
    }

    /// Read settings and pull the keys out of `secrets`.
    ///
    /// @returns the settings, plus whether the file should be rewritten (an id
    /// was handed out, or a key was still sitting in it).
    fn load_with(path: &Path, secrets: &dyn SecretStore) -> (Self, bool) {
        let mut settings = Self::parse_file(path);
        let mut used: Vec<String> = settings
            .connections
            .iter()
            .map(|connection| connection.id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        let mut rewrite = false;
        for connection in &mut settings.connections {
            if connection.id.is_empty() {
                connection.id = fresh_id(&used);
                used.push(connection.id.clone());
                rewrite = true;
            }
            if connection.api_key.trim().is_empty() {
                connection.api_key = secrets
                    .read(&secret_target(&connection.id))
                    .unwrap_or_default();
            } else {
                rewrite = true;
            }
        }
        (settings, rewrite)
    }

    // Filed field by field: every value is optional in the file, so a plain
    // initializer would have to repeat the default for each one.
    #[allow(clippy::field_reassign_with_default)]
    fn parse_file(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        // Notepad and some editors save JSON with a byte-order mark; the parser
        // rejects it, and a rejected file silently means "fresh install". The
        // mark is stripped so an edited config keeps its contents.
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };

        let mut settings = Self::default();
        settings.theme = text_field(&value, "theme");
        settings.budget_tokens = value
            .get("budgetTokens")
            .and_then(|item| item.as_u64())
            .unwrap_or(0)
            .min(1_000_000_000);
        settings.proxy = text_field(&value, "proxy");
        if let Some(list) = value.get("reviewAllow").and_then(|item| item.as_array()) {
            let mut entries: Vec<String> = Vec::new();
            for entry in list.iter().filter_map(|item| item.as_str()) {
                let Some(prefix) = review_prefix(entry) else {
                    continue;
                };
                if entries.iter().any(|seen| seen == &prefix) {
                    continue;
                }
                entries.push(prefix);
                if entries.len() >= MAX_REVIEW_PREFIXES {
                    break;
                }
            }
            settings.review_allow = entries;
        }
        if let Some(map) = value.get("colors").and_then(|item| item.as_object()) {
            let entries: Vec<(String, u32)> = map
                .iter()
                .filter_map(|(key, item)| {
                    let role = crate::theme::color_role(key)?;
                    let color = crate::theme::parse_hex(item.as_str()?)?;
                    Some((role.key.to_string(), color))
                })
                .collect();
            settings.colors = entries;
        }
        if let Some(list) = value.get("plugins").and_then(|item| item.as_array()) {
            let entries: Vec<String> = list
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|entry| is_plugin_id(entry))
                .map(str::to_string)
                .take(MAX_PLUGINS)
                .collect();
            settings.plugins = entries;
        }
        if let Some(list) = value.get("mcpServers").and_then(|item| item.as_array()) {
            let entries: Vec<String> = list
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|entry| is_server_id(entry))
                .map(str::to_string)
                .take(crate::mcp::MAX_MCP_SERVERS)
                .collect();
            settings.mcp_servers = entries;
        }
        if let Some(list) = value.get("skills").and_then(|item| item.as_array()) {
            let entries: Vec<String> = list
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|entry| is_plugin_id(entry))
                .map(str::to_string)
                .take(MAX_PLUGINS)
                .collect();
            settings.skills = entries;
        }
        if let Some(list) = value.get("modes").and_then(|item| item.as_array()) {
            let mut modes: Vec<ChatMode> = Vec::new();
            for item in list.iter().take(MAX_MODES) {
                let id = text_field(item, "id");
                if !is_plugin_id(&id) || id == MODE_BASIC || id == MODE_FULL {
                    continue;
                }
                if modes.iter().any(|mode| mode.id == id) {
                    continue;
                }
                let name = text_field(item, "name");
                let name = if name.is_empty() {
                    format!("自定义模式 {}", modes.len() + 1)
                } else {
                    name
                };
                let ids = |key: &str, server: bool| -> Vec<String> {
                    item.get(key)
                        .and_then(|list| list.as_array())
                        .map(|list| {
                            list.iter()
                                .filter_map(|entry| entry.as_str())
                                .map(str::trim)
                                .filter(|entry| {
                                    if server {
                                        crate::config::is_server_id(entry)
                                    } else {
                                        is_plugin_id(entry)
                                    }
                                })
                                .map(str::to_string)
                                .take(MAX_PLUGINS)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                modes.push(ChatMode {
                    id,
                    name,
                    plugins: ids("plugins", false),
                    skills: ids("skills", false),
                    mcp_servers: ids("mcpServers", true),
                });
            }
            settings.modes = modes;
        }
        if let Some(list) = value.get("connections").and_then(|item| item.as_array()) {
            let connections: Vec<Connection> = list
                .iter()
                .filter_map(|item| {
                    let model = text_field(item, "model");
                    let api_key = text_field(item, "apiKey");
                    let base_url = text_field(item, "baseUrl");
                    let provider = text_field(item, "provider");
                    if model.is_empty() && api_key.is_empty() && base_url.is_empty() {
                        return None;
                    }
                    Some(Connection {
                        id: text_field(item, "id"),
                        provider: if provider.is_empty() {
                            "custom".to_string()
                        } else {
                            provider
                        },
                        model,
                        api_key,
                        base_url,
                        verified: item
                            .get("verified")
                            .and_then(|flag| flag.as_bool())
                            .unwrap_or(false),
                        models: item
                            .get("models")
                            .and_then(|list| list.as_array())
                            .map(|list| {
                                list.iter()
                                    .filter_map(|model| model.as_str())
                                    .map(str::trim)
                                    .filter(|model| !model.is_empty())
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        // The figure itself; a file written before the slider
                        // names a preset instead (`sampling`), which the legacy
                        // lookup turns back into the number it used to send.
                        temperature: item
                            .get("temperature")
                            .and_then(|value| value.as_f64())
                            .or_else(|| {
                                item.get("sampling")
                                    .and_then(|value| value.as_str())
                                    .and_then(legacy_temperature)
                            }),
                        reasoning: Reasoning::from_name(&text_field(item, "reasoning")),
                        max_tokens: item
                            .get("maxTokens")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                            .min(1_000_000) as u32,
                        window: item
                            .get("window")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                            .min(10_000_000),
                        // Older files have no `vision`: images were declared, so
                        // keeping that is the behaviour they had.
                        vision: item
                            .get("vision")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(true),
                    })
                })
                .collect();
            if !connections.is_empty() {
                settings.connections = connections;
            }
            settings.active = value
                .get("active")
                .and_then(|item| item.as_u64())
                .map_or(0, |index| index as usize)
                .min(settings.connections.len().saturating_sub(1));
            return settings;
        }

        let model = text_field(&value, "model");
        let api_key = text_field(&value, "apiKey");
        let base_url = text_field(&value, "baseUrl");
        if model.is_empty() && api_key.is_empty() && base_url.is_empty() {
            return settings;
        }
        let mut migrated = Connection::from_preset("deepseek").unwrap_or(Connection {
            id: String::new(),
            provider: "custom".to_string(),
            model: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            verified: false,
            models: Vec::new(),
            temperature: None,
            reasoning: Reasoning::Default,
            max_tokens: 0,
            window: 0,
            vision: true,
        });
        if !model.is_empty() {
            migrated.model = model;
        }
        if !base_url.is_empty() {
            migrated.base_url = base_url;
        }
        // The old build let the user send as soon as a key was present, so a key
        // means the connection was in use: keep it selectable.
        migrated.verified = !api_key.is_empty();
        migrated.api_key = api_key;
        settings.connections = vec![migrated];
        settings.active = 0;
        settings
    }

    /// Write settings to an arbitrary file, creating the directory.
    ///
    /// Only tests use this: the window always goes through the credential store.
    #[cfg(test)]
    fn save_to(&self, path: &Path) -> Result<(), String> {
        self.save_with(path, &crate::secrets::NoStore)
    }

    /// Write settings, filing every key in `secrets` first.
    ///
    /// A store that refuses the write keeps the key in the file: losing a key is
    /// worse than storing it the old way, and the settings page says which way
    /// was used.
    fn save_with(&self, path: &Path, secrets: &dyn SecretStore) -> Result<(), String> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("创建 {} 失败：{error}", directory.display()))?;
        }
        let connections: Vec<serde_json::Value> = self
            .connections
            .iter()
            .map(|connection| {
                let key = connection.api_key.trim();
                // Without an id there is nothing to file the key under, so it
                // stays in the file rather than sharing another connection's
                // credential.
                let stored = key.is_empty()
                    || (!connection.id.is_empty()
                        && secrets.write(&secret_target(&connection.id), key).is_ok());
                serde_json::json!({
                    "id": connection.id,
                    "provider": connection.provider,
                    "model": connection.model,
                    // Empty once the key is in the credential store; kept only
                    // when that store refused it.
                    "apiKey": if stored { "" } else { key },
                    "baseUrl": connection.base_url,
                    "verified": connection.verified,
                    "models": connection.models,
                    "reasoning": connection.reasoning.name(),
                    "temperature": connection.temperature,
                    "maxTokens": connection.max_tokens,
                    "window": connection.window,
                    "vision": connection.vision,
                })
            })
            .collect();
        let colors: serde_json::Map<String, serde_json::Value> = self
            .colors
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    serde_json::Value::String(crate::theme::hex(*value)),
                )
            })
            .collect();
        let modes: Vec<serde_json::Value> = self
            .modes
            .iter()
            .map(|mode| {
                serde_json::json!({
                    "id": mode.id,
                    "name": mode.name,
                    "plugins": mode.plugins,
                    "skills": mode.skills,
                    "mcpServers": mode.mcp_servers,
                })
            })
            .collect();
        let value = serde_json::json!({
            "active": self.active,
            "theme": self.theme,
            "colors": colors,
            "plugins": self.plugins,
            "mcpServers": self.mcp_servers,
            "skills": self.skills,
            "modes": modes,
            "budgetTokens": self.budget_tokens,
            "proxy": self.proxy,
            "reviewAllow": self.review_allow,
            "connections": connections,
        });
        let text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
        crate::store::write_atomic(path, &text)
            .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
    }
}

/// Credential-store name for one connection's key.
fn secret_target(id: &str) -> String {
    format!("{TARGET_PREFIX}{id}")
}

/// An id no other connection uses, so a key can never land on someone else's.
fn fresh_id(used: &[String]) -> String {
    let mut next = used.len() + 1;
    loop {
        let candidate = format!("c{next}");
        if !used.iter().any(|seen| seen == &candidate) {
            return candidate;
        }
        next += 1;
    }
}

/// A trimmed string field of a JSON object, empty when absent.
fn text_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary file that deletes itself.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("dsh-flow-ui-{name}-{}.json", std::process::id()));
            let _ = std::fs::remove_file(&path);
            Self(path)
        }

        fn write(&self, text: &str) {
            std::fs::write(&self.0, text).expect("write");
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A throw-away folder, for the plugin scan.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("dsh-flow-ui-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        /// Write one plugin folder with the given manifest and entry source.
        fn plugin(&self, id: &str, manifest: &str, source: Option<&str>) -> &Self {
            let folder = self.0.join(id);
            std::fs::create_dir_all(&folder).expect("plugin folder");
            std::fs::write(folder.join("plugin.json"), manifest).expect("manifest");
            if let Some(source) = source {
                std::fs::write(folder.join("index.js"), source).expect("entry");
            }
            self
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_to_one_deepseek_connection() {
        let settings = AiSettings::default();
        assert_eq!(settings.connections.len(), 1);
        let connection = settings.active().expect("active");
        assert_eq!(connection.provider, "deepseek");
        assert_eq!(connection.model, "deepseek-flash");
        assert_eq!(connection.base_url, "https://api.deepseek.com");
        assert!(!connection.verified);
        assert!(!settings.usable());
    }

    #[test]
    fn a_byte_order_mark_does_not_turn_the_file_into_defaults() {
        // Editors add a BOM; JSON parsers reject it. Without the strip, that
        // reads as "no settings" and the next save wipes the palette.
        let file = TempFile::new("bom");
        file.write("\u{feff}{\"theme\":\"dark\",\"colors\":{\"accent\":\"#ff0000\"}}");
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.colors, vec![("accent".to_string(), 0x00ff_0000)]);
    }

    #[test]
    fn round_trips_through_a_file() {
        let file = TempFile::new("save");
        let settings = AiSettings {
            connections: vec![
                Connection {
                    id: "c1".to_string(),
                    provider: "deepseek".to_string(),
                    model: "deepseek-reasoner".to_string(),
                    api_key: "sk-a".to_string(),
                    base_url: "https://api.deepseek.com".to_string(),
                    verified: true,
                    models: vec!["deepseek-chat".to_string(), "deepseek-v4-pro".to_string()],
                    temperature: Some(0.7),
                    reasoning: Reasoning::High,
                    max_tokens: 4096,
                    // A window narrower than the name table would guess, so the
                    // round trip proves the override survives.
                    window: 262_144,
                    vision: true,
                },
                Connection {
                    id: "c2".to_string(),
                    provider: "custom".to_string(),
                    model: "local-model".to_string(),
                    api_key: "sk-b".to_string(),
                    base_url: "http://127.0.0.1:1234/v1".to_string(),
                    verified: false,
                    models: Vec::new(),
                    temperature: None,
                    reasoning: Reasoning::Default,
                    max_tokens: 0,
                    window: 0,
                    // A text-only endpoint: the two image tools must stay out of
                    // its requests after a restart too.
                    vision: false,
                },
            ],
            active: 1,
            theme: "dark".to_string(),
            colors: Vec::new(),
            plugins: vec!["demo-plugin".to_string()],
            mcp_servers: vec!["demo-server".to_string()],
            skills: vec!["demo-skill".to_string()],
            modes: vec![ChatMode {
                id: "mode-1".to_string(),
                name: "写作".to_string(),
                plugins: vec!["demo-plugin".to_string()],
                skills: vec!["demo-skill".to_string()],
                mcp_servers: vec!["demo-server".to_string()],
            }],
            budget_tokens: 250_000,
            proxy: "http://127.0.0.1:7890".to_string(),
            review_allow: vec!["npm test".to_string()],
        };
        settings.save_to(&file.0).expect("save");
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(loaded, settings);
        assert_eq!(loaded.verified().len(), 1);
        assert!(!loaded.usable(), "the active connection is not verified");
        // Only a plugin someone switched on travels in the file.
        assert_eq!(loaded.plugins, vec!["demo-plugin".to_string()]);
        assert_eq!(loaded.mcp_servers, vec!["demo-server".to_string()]);
        assert_eq!(loaded.skills, vec!["demo-skill".to_string()]);
        assert_eq!(loaded.modes, settings.modes);
        assert_eq!(loaded.mode_label("mode-1"), "写作");
        assert_eq!(
            loaded.mode_selection("mode-1").plugins,
            vec!["demo-plugin".to_string()]
        );
        // The spending guard and the proxy survive a round trip too.
        assert_eq!(loaded.budget_tokens, 250_000);
        assert_eq!(loaded.proxy, "http://127.0.0.1:7890");
        assert_eq!(loaded.review_allow, vec!["npm test".to_string()]);
    }

    #[test]
    fn the_review_list_is_cleaned_and_deduplicated() {
        let file = TempFile::new("review-allow");
        file.write(r#"{"reviewAllow":["  npm test  ","npm test","","   ","pytest, -q"]}"#);
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(
            loaded.review_allow,
            vec!["npm test".to_string(), "pytest, -q".to_string()]
        );

        assert_eq!(review_prefix("npm test"), Some("npm test".to_string()));
        assert_eq!(review_prefix("   "), None);
        // A newline inside an entry could never match a single command line.
        assert_eq!(review_prefix("npm\ntest"), None);
        assert_eq!(review_prefix(&"x".repeat(MAX_REVIEW_PREFIX + 1)), None);
    }

    #[test]
    fn a_budget_and_a_proxy_are_read_and_bounded() {
        let file = TempFile::new("budget");
        file.write(
            r#"{"budgetTokens":75,"proxy":" http://proxy.test:8080 ","connections":[{"id":"c1","provider":"custom","model":"m","baseUrl":"https://x.test","apiKey":"k","verified":true}]}"#,
        );
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(loaded.budget_tokens, 75);
        assert_eq!(loaded.proxy, "http://proxy.test:8080");

        let file = TempFile::new("budget-huge");
        file.write(r#"{"budgetTokens":99999999999999}"#);
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(loaded.budget_tokens, 1_000_000_000, "clamped");
        assert!(loaded.proxy.is_empty());
    }

    #[test]
    fn workspace_and_data_live_next_to_the_program() {
        let settings = AiSettings::default();
        assert_eq!(
            settings.workspace_dir(),
            AiSettings::home().join("workspace")
        );
        assert_eq!(settings.data_dir(), AiSettings::home().join("data"));
    }

    #[test]
    fn migrates_the_flat_file_shape() {
        let file = TempFile::new("migrate");
        file.write(
            r#"{"model":"deepseek-reasoner","apiKey":"sk-old","baseUrl":"https://api.deepseek.com"}"#,
        );
        let settings = AiSettings::load_from(&file.0);
        assert_eq!(settings.connections.len(), 1);
        let connection = settings.active().expect("active");
        assert_eq!(connection.model, "deepseek-reasoner");
        assert_eq!(connection.api_key, "sk-old");
        assert!(
            connection.verified,
            "an existing key keeps the model selectable"
        );
    }

    #[test]
    fn keeps_defaults_when_the_file_is_missing_or_broken() {
        let file = TempFile::new("broken");
        assert_eq!(AiSettings::load_from(&file.0), AiSettings::default());

        file.write("{not json");
        assert_eq!(AiSettings::load_from(&file.0), AiSettings::default());
    }

    #[test]
    fn clamps_an_out_of_range_active_index() {
        let file = TempFile::new("active");
        file.write(
            r#"{"active":7,"connections":[{"provider":"deepseek","model":"m","apiKey":"k","baseUrl":"u","verified":true}]}"#,
        );
        let settings = AiSettings::load_from(&file.0);
        assert_eq!(settings.active, 0);
        assert!(settings.usable());
    }

    #[test]
    fn presets_carry_a_default_model() {
        assert_eq!(
            preset("deepseek").map(|preset| preset.model),
            Some("deepseek-flash")
        );
        assert!(preset("nope").is_none());
        let custom = Connection::from_preset("custom").expect("custom preset");
        assert!(custom.base_url.is_empty());
        assert!(!custom.ready());
    }

    #[test]
    fn offered_models_keep_the_shipped_list_alongside_what_the_endpoint_reported() {
        // A fetch that predates a model, or a gateway that hides one, must not
        // remove it from the picker: the shipped list is the floor.
        let fetched = vec!["deepseek-flash".to_string()];
        assert_eq!(
            offered_models("deepseek", "https://api.deepseek.com", &fetched),
            vec!["deepseek-flash", "deepseek-v4-pro"]
        );
        // Extras the endpoint reported come after the curated order.
        let fetched = vec!["deepseek-flash".to_string(), "my-proxy-model".to_string()];
        assert_eq!(
            offered_models("deepseek", "https://api.deepseek.com", &fetched),
            vec!["deepseek-flash", "deepseek-v4-pro", "my-proxy-model"]
        );
        // A custom connection pointed at DeepSeek still gets the lineup.
        assert_eq!(
            offered_models("custom", "https://api.deepseek.com/v1", &[]),
            vec!["deepseek-flash", "deepseek-v4-pro"]
        );
        // Any other custom endpoint offers exactly what it reported.
        assert_eq!(
            offered_models("custom", "https://example.test/v1", &["m1".to_string()]),
            vec!["m1"]
        );
    }

    #[test]
    fn presets_offer_their_model_versions() {
        let deepseek = model_versions("deepseek");
        assert!(deepseek.contains(&"deepseek-flash"), "{deepseek:?}");
        assert!(deepseek.contains(&"deepseek-v4-pro"), "{deepseek:?}");
        for entry in PRESETS.iter() {
            if entry.models.is_empty() {
                continue;
            }
            assert!(
                entry.models.contains(&entry.model),
                "{} 的默认模型不在版本列表里",
                entry.id
            );
        }
        assert!(model_versions("nope").is_empty());
    }

    #[test]
    fn an_old_preset_name_still_reads_as_its_number() {
        assert_eq!(legacy_temperature("precise"), Some(0.2));
        assert_eq!(legacy_temperature("balanced"), Some(0.7));
        assert_eq!(legacy_temperature("loose"), Some(1.0));
        // "default" sent nothing, and so does an unknown name.
        assert_eq!(legacy_temperature("default"), None);
        assert_eq!(legacy_temperature("weird"), None);
    }

    #[test]
    fn a_file_written_before_the_slider_keeps_its_temperature() {
        // `sampling` was a preset name; the number it stood for is what the
        // slider has to show now.
        let file = TempFile::new("legacy-sampling");
        file.write(
            "{\"connections\":[{\"id\":\"c1\",\"provider\":\"deepseek\",\
             \"model\":\"deepseek-flash\",\"apiKey\":\"sk-a\",\
             \"baseUrl\":\"https://api.deepseek.com\",\"sampling\":\"balanced\"}]}",
        );
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(loaded.connections[0].temperature, Some(0.7));
    }

    #[test]
    fn reasoning_names_round_trip_and_unknown_means_the_endpoint_default() {
        for reasoning in [
            Reasoning::Default,
            Reasoning::Off,
            Reasoning::Low,
            Reasoning::High,
            Reasoning::Max,
        ] {
            assert_eq!(Reasoning::from_name(reasoning.name()), reasoning);
        }
        assert_eq!(Reasoning::from_name("weird"), Reasoning::Default);
        assert_eq!(Reasoning::from_name(""), Reasoning::Default);
    }

    #[test]
    fn a_save_without_a_reasoning_key_keeps_the_endpoint_default() {
        // Files written before the thinking switch existed have no key at all;
        // they must keep the endpoint's own default, not read as "off".
        let file = TempFile::new("no-reasoning");
        file.write(
            "{\"connections\":[{\"id\":\"c1\",\"provider\":\"deepseek\",\
             \"model\":\"deepseek-flash\",\"apiKey\":\"sk-a\",\
             \"baseUrl\":\"https://api.deepseek.com\",\"verified\":true}]}",
        );
        let loaded = AiSettings::load_from(&file.0);
        assert_eq!(loaded.connections.len(), 1);
        assert_eq!(loaded.connections[0].reasoning, Reasoning::Default);
        assert_eq!(loaded.connections[0].temperature, None);
    }

    #[test]
    fn web_search_is_offered_by_deepseek_flash_only() {
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        assert!(connection.supports_web_search());
        connection.model = "deepseek-v4-pro".to_string();
        assert!(
            !connection.supports_web_search(),
            "only flash offers the tools"
        );
        // A custom entry pointing at DeepSeek still counts; anywhere else does not.
        connection.provider = "custom".to_string();
        connection.model = "deepseek-flash".to_string();
        assert!(connection.supports_web_search());
        connection.base_url = "https://api.openai.com/v1".to_string();
        assert!(!connection.supports_web_search());
    }

    #[test]
    fn normalizing_trims_fields() {
        let connection = Connection {
            id: "c1".to_string(),
            provider: "deepseek".to_string(),
            model: "  deepseek-chat ".to_string(),
            api_key: " sk-a ".to_string(),
            base_url: " https://api.deepseek.com/ ".to_string(),
            verified: false,
            models: Vec::new(),
            temperature: None,
            reasoning: Reasoning::Default,
            max_tokens: 0,
            window: 0,
            vision: true,
        };
        let normalized = connection.normalized();
        assert_eq!(normalized.model, "deepseek-chat");
        assert_eq!(normalized.api_key, "sk-a");
        assert_eq!(normalized.base_url, "https://api.deepseek.com");
    }

    #[test]
    fn a_connection_names_its_own_window_when_it_knows_better() {
        let mut connection = Connection::from_preset("deepseek").expect("preset");
        // Without an override the name table answers.
        assert_eq!(
            connection.window_tokens(),
            crate::ai::context_window(&connection.model)
        );
        connection.model = "some-local-model".to_string();
        assert_eq!(
            connection.window_tokens(),
            crate::ai::DEFAULT_CONTEXT_WINDOW,
            "an unknown model falls back to the conservative default"
        );
        connection.window = 32_768;
        assert_eq!(connection.window_tokens(), 32_768, "the setting wins");
        // A hand-edited absurd value is clamped before it reaches the gauge.
        connection.window = u64::MAX;
        assert_eq!(connection.normalized().window, 10_000_000);
    }

    /// A connection with only the fields a store test cares about.
    fn stored(api_key: &str) -> Connection {
        Connection {
            id: "c1".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            api_key: api_key.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            verified: true,
            models: Vec::new(),
            temperature: None,
            reasoning: Reasoning::Default,
            max_tokens: 0,
            window: 0,
            vision: true,
        }
    }

    #[test]
    fn keys_are_filed_in_the_credential_store_not_the_file() {
        let file = TempFile::new("secrets");
        let store = crate::secrets::MemoryStore::new();
        let settings = AiSettings {
            connections: vec![stored("sk-a")],
            ..AiSettings::default()
        };
        settings.save_with(&file.0, &store).expect("save");

        let text = std::fs::read_to_string(&file.0).expect("read");
        assert!(
            !text.contains("sk-a"),
            "the key stays out of the file: {text}"
        );
        assert_eq!(store.get("dsh-flow-ui/c1").as_deref(), Some("sk-a"));

        let (loaded, rewrite) = AiSettings::load_with(&file.0, &store);
        assert_eq!(loaded.connections[0].api_key, "sk-a");
        assert!(!rewrite, "a filed key needs no migration");
    }

    #[test]
    fn a_key_left_in_the_file_moves_into_the_store() {
        let file = TempFile::new("migrate-key");
        let legacy = AiSettings {
            connections: vec![stored("sk-old")],
            ..AiSettings::default()
        };
        // `save_to` is the store-less path: it leaves the key in the file.
        legacy.save_to(&file.0).expect("save");
        let store = crate::secrets::MemoryStore::new();

        let (loaded, rewrite) = AiSettings::load_with(&file.0, &store);
        assert!(rewrite, "a key in the file has to move");
        assert_eq!(loaded.connections[0].api_key, "sk-old");
        assert_eq!(loaded.connections[0].id, "c1");

        loaded.save_with(&file.0, &store).expect("save");
        let text = std::fs::read_to_string(&file.0).expect("read");
        assert!(!text.contains("sk-old"), "{text}");
        assert_eq!(store.get("dsh-flow-ui/c1").as_deref(), Some("sk-old"));
    }

    #[test]
    fn a_connection_without_an_id_gets_one_and_keeps_its_key() {
        // An older `connections` array, written before ids existed.
        let file = TempFile::new("ids");
        std::fs::write(
            &file.0,
            r#"{"active":0,"connections":[{"provider":"custom","model":"m","apiKey":"sk-x","baseUrl":"https://x.test"}]}"#,
        )
        .expect("write");
        let store = crate::secrets::MemoryStore::new();
        let (loaded, rewrite) = AiSettings::load_with(&file.0, &store);
        assert!(rewrite);
        assert!(!loaded.connections[0].id.is_empty());
        assert_eq!(loaded.connections[0].api_key, "sk-x");
    }

    #[test]
    fn one_colour_at_a_time_can_be_replaced_and_put_back() {
        let theme = crate::theme::ThemeKind::Light;
        let mut settings = AiSettings::default();
        assert_eq!(settings.color_of("accent"), None);

        assert_eq!(
            settings.set_color("accent", Some(0x00ff_0000), theme),
            Some(1)
        );
        assert_eq!(settings.color_of("accent"), Some(0x00ff_0000));
        assert_eq!(settings.colors, vec![("accent".to_string(), 0x00ff_0000)]);

        // Replacing the same role keeps a single entry.
        assert_eq!(
            settings.set_color("accent", Some(0x0000_ff00), theme),
            Some(1)
        );
        assert_eq!(settings.color_of("accent"), Some(0x0000_ff00));

        // Picking the theme's own colour is stored as "no override".
        let accent = theme.palette().accent;
        assert_eq!(settings.set_color("accent", Some(accent), theme), Some(0));
        assert_eq!(settings.color_of("accent"), None);

        // `None` drops the override, and unknown roles change nothing.
        settings.set_color("accent", Some(0x00ff_0000), theme);
        assert_eq!(settings.set_color("accent", None, theme), Some(0));
        assert_eq!(settings.color_of("accent"), None);
        assert_eq!(settings.set_color("nope", Some(0x00ff_0000), theme), None);
        assert!(settings.colors.is_empty());

        // What survives a save is what the next run reads back.
        settings.set_color("border", Some(0x0012_3456), theme);
        let file = TempFile::new("colors");
        settings.save_to(&file.0).expect("save");
        assert_eq!(AiSettings::load_from(&file.0).colors, settings.colors);

        // Picking a shipped theme drops the overrides, and the file says so.
        settings.clear_colors();
        assert!(settings.colors.is_empty());
        settings.save_to(&file.0).expect("save");
        assert!(AiSettings::load_from(&file.0).colors.is_empty());
    }

    #[test]
    fn everything_lives_next_to_the_program() {
        // Portable by construction: config, plugins, data and workspace all hang
        // off the folder the executable sits in, so moving that folder moves the
        // installation. Under `cargo test` the "executable" is the test binary in
        // `target/debug/deps`, which is writable, so no fallback applies.
        let exe = std::env::current_exe().expect("exe");
        let dir = exe.parent().expect("program folder").to_path_buf();
        assert_eq!(AiSettings::home(), dir);
        assert_eq!(AiSettings::default_data_dir(), dir.join("data"));
        assert_eq!(AiSettings::default_workspace(), dir.join("workspace"));
        assert_eq!(AiSettings::plugin_dir(), Some(dir.join("plugins")));
        assert_eq!(AiSettings::skill_dir(), Some(dir.join("skills")));
        // Nothing is left behind by the writability probe.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".dsh-flow-write-")
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn deletes_a_plugin_folder_and_refuses_a_path() {
        let dir =
            std::env::temp_dir().join(format!("dsh-flow-ui-plugin-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plugin.json"), "{}").unwrap();
        delete_plugin_in(&dir, "demo").unwrap();
        assert!(!folder.exists());
        assert!(delete_plugin_in(&dir, "../evil").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_a_plugin_folder_with_its_entry() {
        let dir = TempDir::new("plugin-import");
        let source = dir.0.join("source").join("echo");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("plugin.json"),
            r#"{"name":"回显","version":"1.0.0","api":1}"#,
        )
        .unwrap();
        std::fs::write(source.join("index.js"), "export function activate() {}").unwrap();
        let target = dir.0.join("plugins");
        std::fs::create_dir_all(&target).unwrap();

        let id = import_plugin_into(&source, &target).unwrap();
        assert_eq!(id, "echo");
        assert!(target.join("echo").join("index.js").is_file());
        let found = scanned_plugins(&target);
        assert_eq!(found.len(), 1);
        assert!(found[0].problem.is_empty());
        assert_eq!(found[0].name, "回显");
        // The original stays, and the same id cannot land twice.
        assert!(source.join("index.js").is_file());
        assert!(import_plugin_into(&source, &target).is_err());
    }

    #[test]
    fn refuses_a_plugin_folder_the_engine_would_not_load() {
        let dir = TempDir::new("plugin-import-broken");
        let target = dir.0.join("plugins");
        std::fs::create_dir_all(&target).unwrap();

        let missing = dir.0.join("missing");
        std::fs::create_dir_all(&missing).unwrap();
        assert!(
            import_plugin_into(&missing, &target)
                .unwrap_err()
                .contains("plugin.json")
        );

        let wrong_api = dir.0.join("wrong-api");
        std::fs::create_dir_all(&wrong_api).unwrap();
        std::fs::write(wrong_api.join("plugin.json"), r#"{"api":2}"#).unwrap();
        std::fs::write(wrong_api.join("index.js"), "").unwrap();
        assert!(
            import_plugin_into(&wrong_api, &target)
                .unwrap_err()
                .contains("api")
        );

        let no_entry = dir.0.join("no-entry");
        std::fs::create_dir_all(&no_entry).unwrap();
        std::fs::write(no_entry.join("plugin.json"), r#"{"api":1}"#).unwrap();
        assert!(
            import_plugin_into(&no_entry, &target)
                .unwrap_err()
                .contains("入口")
        );

        let escaping = dir.0.join("escaping");
        std::fs::create_dir_all(&escaping).unwrap();
        std::fs::write(
            escaping.join("plugin.json"),
            r#"{"api":1,"main":"../outside.js"}"#,
        )
        .unwrap();
        std::fs::write(dir.0.join("outside.js"), "").unwrap();
        assert!(
            import_plugin_into(&escaping, &target)
                .unwrap_err()
                .contains("之外")
        );

        let spaced = dir.0.join("有 空格");
        std::fs::create_dir_all(&spaced).unwrap();
        std::fs::write(spaced.join("plugin.json"), r#"{"api":1}"#).unwrap();
        std::fs::write(spaced.join("index.js"), "").unwrap();
        assert!(
            import_plugin_into(&spaced, &target)
                .unwrap_err()
                .contains("插件 id")
        );
        assert!(scanned_plugins(&target).is_empty(), "nothing was copied");
    }

    #[test]
    fn server_ids_allow_spaces_but_not_paths() {
        assert!(is_server_id("filesystem"));
        assert!(is_server_id("my server"));
        assert!(!is_server_id(""));
        assert!(!is_server_id("../evil"));
        assert!(!is_server_id(".hidden"));
        assert!(!is_server_id("a/b"));
    }

    #[test]
    fn only_switched_on_plugins_travel_in_the_file() {
        let mut settings = AiSettings::default();
        assert!(!settings.plugin_enabled("demo"));
        assert!(settings.set_plugin("demo", true));
        assert!(settings.plugin_enabled("demo"));
        assert!(settings.set_plugin("other", true));
        // The list is kept sorted, so the file is stable between saves.
        assert_eq!(
            settings.plugins,
            vec!["demo".to_string(), "other".to_string()]
        );
        assert!(settings.set_plugin("demo", false));
        assert_eq!(settings.plugins, vec!["other".to_string()]);
        // A name that could walk the folder tree is refused outright.
        assert!(!settings.set_plugin("../evil", true));
        assert!(!settings.set_plugin("", true));
        assert_eq!(settings.plugins, vec!["other".to_string()]);
    }

    #[test]
    fn only_switched_on_skills_travel_in_the_file() {
        let mut settings = AiSettings::default();
        assert!(!settings.skill_enabled("demo"));
        assert!(settings.set_skill("demo", true));
        assert!(settings.skill_enabled("demo"));
        assert!(settings.set_skill("other", true));
        assert_eq!(
            settings.skills,
            vec!["demo".to_string(), "other".to_string()]
        );
        assert!(settings.set_skill("demo", false));
        assert_eq!(settings.skills, vec!["other".to_string()]);
        // A name that could walk the folder tree is refused outright.
        assert!(!settings.set_skill("../evil", true));
        assert!(!settings.set_skill("", true));
        assert_eq!(settings.skills, vec!["other".to_string()]);
    }

    #[test]
    fn plugin_ids_are_folder_names_only() {
        assert!(is_plugin_id("demo"));
        assert!(is_plugin_id("demo-plugin_2"));
        for bad in [
            "", ".", ".hidden", "..", "a/b", "a\\b", "a b", "插件", "demo.", "con", "NUL", "com1",
        ] {
            assert!(!is_plugin_id(bad), "{bad} should be refused");
        }
    }

    #[test]
    fn scanning_names_what_is_wrong_with_a_plugin() {
        let dir = TempDir::new("scan");
        dir.plugin(
            "good",
            r#"{"name":"Good One","version":"1.2.3","api":1}"#,
            Some("export function activate() {}"),
        );
        dir.plugin(
            "old",
            r#"{"api":99}"#,
            Some("export function activate() {}"),
        );
        dir.plugin("nomodule", r#"{"api":1}"#, None);
        std::fs::create_dir_all(dir.0.join("empty")).expect("folder");
        // Files and names that are not folder names are ignored.
        std::fs::write(dir.0.join("notes.txt"), "x").expect("file");

        let found = scanned_plugins(&dir.0);
        let ids: Vec<&str> = found.iter().map(|plugin| plugin.id.as_str()).collect();
        assert_eq!(ids, vec!["empty", "good", "nomodule", "old"]);

        let good = &found[1];
        assert_eq!(good.name, "Good One");
        assert_eq!(good.version, "1.2.3");
        assert!(good.problem.is_empty(), "{}", good.problem);

        assert!(found[0].problem.contains("plugin.json"));
        assert!(found[2].problem.contains("入口文件"));
        assert!(found[3].problem.contains("只支持"));
    }
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn modes_intersect_the_global_resource_switches() {
        let mut settings = AiSettings::default();
        settings.plugins = vec!["plugin-a".to_string(), "plugin-b".to_string()];
        settings.skills = vec!["skill-a".to_string(), "skill-b".to_string()];
        settings.mcp_servers = vec!["server-a".to_string(), "server-b".to_string()];
        settings.modes = vec![ChatMode {
            id: "mode-1".to_string(),
            name: "写作".to_string(),
            plugins: vec!["plugin-a".to_string(), "plugin-b".to_string()],
            skills: vec!["skill-a".to_string()],
            mcp_servers: vec!["server-a".to_string()],
        }];

        // Basic carries no external resource at all.
        assert_eq!(
            settings.mode_selection(crate::config::MODE_BASIC),
            ModeSelection::default()
        );
        // Full carries everything that is globally enabled.
        assert_eq!(
            settings.mode_selection(crate::config::MODE_FULL),
            ModeSelection {
                plugins: vec!["plugin-a".to_string(), "plugin-b".to_string()],
                skills: vec!["skill-a".to_string(), "skill-b".to_string()],
                mcp_servers: vec!["server-a".to_string(), "server-b".to_string()],
            }
        );
        // A custom mode narrows the lists, and a global off switch wins again.
        settings.plugins.retain(|id| id != "plugin-b");
        settings.skills.retain(|id| id != "skill-a");
        settings.mcp_servers.retain(|id| id != "server-a");
        assert_eq!(
            settings.mode_selection("mode-1"),
            ModeSelection {
                plugins: vec!["plugin-a".to_string()],
                skills: Vec::new(),
                mcp_servers: Vec::new(),
            }
        );
        // An unknown custom mode falls back to full rather than silently empty.
        assert_eq!(
            settings.mode_selection("deleted-mode"),
            settings.mode_selection(crate::config::MODE_FULL)
        );
    }
}
