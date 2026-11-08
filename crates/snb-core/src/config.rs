use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

use crate::error::{AppError, AppResult};
use crate::util::{sanitize_file_name, validate_url};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

const MAX_DESCRIPTION: usize = 500;
const MAX_BODY: usize = 6_000;
const MAX_TITLE: usize = 140;

fn truncate_chars(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

/// Accepts either a plain string (used for both languages) or
/// `{ "en": "...", "fr": "..." }` for translator-authored content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalizedText {
    pub en: String,
    pub fr: String,
}

impl LocalizedText {
    pub fn one(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            en: text.clone(),
            fr: text,
        }
    }

    pub fn pick(&self, language: &str) -> &str {
        if language.eq_ignore_ascii_case("fr") && !self.fr.trim().is_empty() {
            &self.fr
        } else {
            &self.en
        }
    }

    pub fn is_empty(&self) -> bool {
        self.en.trim().is_empty() && self.fr.trim().is_empty()
    }

    fn clean(&mut self, max: usize) {
        self.en = truncate_chars(&self.en, max);
        self.fr = truncate_chars(&self.fr, max);
    }
}

impl<'de> Deserialize<'de> for LocalizedText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LocalizedVisitor;

        impl<'de> Visitor<'de> for LocalizedVisitor {
            type Value = LocalizedText;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string or an object with en/fr strings")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LocalizedText::one(value))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LocalizedText::one(value))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut en: Option<String> = None;
                let mut fr: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "en" => en = Some(map.next_value()?),
                        "fr" => fr = Some(map.next_value()?),
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                let en = en.unwrap_or_default();
                let fr = fr.unwrap_or_else(|| en.clone());
                Ok(LocalizedText { en, fr })
            }
        }

        deserializer.deserialize_any(LocalizedVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEndpoint {
    pub host: String,
    pub port: u16,
}

impl ServerEndpoint {
    pub fn parse(host: &str, port: u16) -> AppResult<Self> {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if port == 0 {
            return Err(AppError::new("endpoint_port_invalid"));
        }
        if host.is_empty() || host.len() > 253 {
            return Err(AppError::new("endpoint_host_invalid"));
        }
        if host.parse::<IpAddr>().is_ok() {
            return Err(AppError::new("endpoint_ip_literal"));
        }
        const INTERNAL_SUFFIXES: &[&str] = &[
            "localhost", ".local", ".internal", ".lan", ".home", ".test", ".invalid",
        ];
        if host == "localhost"
            || INTERNAL_SUFFIXES
                .iter()
                .any(|suffix| host.ends_with(suffix))
        {
            return Err(AppError::new("endpoint_private_host"));
        }
        if !host.is_ascii()
            || host.contains(|c: char| {
                !(c.is_ascii_alphanumeric() || c == '-' || c == '.')
            })
        {
            return Err(AppError::new("endpoint_host_invalid"));
        }
        let labels: Vec<&str> = host.split('.').collect();
        if labels.len() < 2
            || labels.iter().any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
            })
        {
            return Err(AppError::new("endpoint_host_invalid"));
        }
        let tld = labels.last().copied().unwrap_or("");
        if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(AppError::new("endpoint_host_invalid"));
        }
        Ok(Self { host, port })
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn is_loopback(&self) -> bool {
        self.host
            .parse::<Ipv4Addr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
            || self.host == "localhost"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerInfoRow {
    pub label: LocalizedText,
    pub value: LocalizedText,
}

impl Default for ServerInfoRow {
    fn default() -> Self {
        Self {
            label: LocalizedText::default(),
            value: LocalizedText::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfig {
    pub schema_version: u32,
    pub endpoint: Option<ServerEndpoint>,
    pub display_name: String,
    pub description: LocalizedText,
    pub maintenance: bool,
    pub maintenance_message: LocalizedText,
    pub info: Vec<ServerInfoRow>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            endpoint: None,
            display_name: "Sky & Beyond".to_string(),
            description: LocalizedText::default(),
            maintenance: false,
            maintenance_message: LocalizedText::default(),
            info: Vec::new(),
        }
    }
}

impl ServerConfig {
    pub fn validate(mut self) -> AppResult<Self> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(AppError::new("config_schema_unsupported").param(
                "schemaVersion",
                self.schema_version.to_string(),
            ));
        }
        self.display_name = truncate_chars(&self.display_name, 80);
        if self.display_name.is_empty() {
            self.display_name = "Sky & Beyond".to_string();
        }
        self.description.clean(MAX_DESCRIPTION);
        self.maintenance_message.clean(MAX_DESCRIPTION);
        self.info.truncate(8);
        for row in &mut self.info {
            row.label.clean(60);
            row.value.clean(120);
        }
        self.info.retain(|row| !row.label.is_empty());
        if let Some(endpoint) = self.endpoint.take() {
            self.endpoint = Some(ServerEndpoint::parse(&endpoint.host, endpoint.port)?);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MaintenanceState {
    pub active: bool,
    pub message: LocalizedText,
}

impl Default for MaintenanceState {
    fn default() -> Self {
        Self {
            active: false,
            message: LocalizedText::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Announcement {
    pub id: String,
    pub level: String,
    pub title: LocalizedText,
    pub body: LocalizedText,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub link: Option<String>,
    pub dismissible: bool,
}

impl Default for Announcement {
    fn default() -> Self {
        Self {
            id: String::new(),
            level: "info".to_string(),
            title: LocalizedText::default(),
            body: LocalizedText::default(),
            starts_at: None,
            ends_at: None,
            link: None,
            dismissible: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteButton {
    pub id: String,
    pub label: LocalizedText,
    pub url: String,
    pub icon: String,
    pub style: String,
}

impl Default for RemoteButton {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: LocalizedText::default(),
            url: String::new(),
            icon: "external".to_string(),
            style: "secondary".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteLinks {
    pub discord: Option<String>,
    pub website: Option<String>,
    pub support: Option<String>,
}

impl RemoteLinks {
    fn clean(&mut self) {
        self.discord = clean_https_url(self.discord.take());
        self.website = clean_https_url(self.website.take());
        self.support = clean_https_url(self.support.take());
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LauncherInfo {
    pub schema_version: u32,
    pub latest_version: Option<String>,
    pub minimum_version: Option<String>,
    pub notes: LocalizedText,
    pub announcements: Vec<Announcement>,
    pub buttons: Vec<RemoteButton>,
    pub links: RemoteLinks,
    pub maintenance: MaintenanceState,
}

impl Default for LauncherInfo {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            latest_version: None,
            minimum_version: None,
            notes: LocalizedText::default(),
            announcements: Vec::new(),
            buttons: Vec::new(),
            links: RemoteLinks::default(),
            maintenance: MaintenanceState::default(),
        }
    }
}

fn clean_https_url(value: Option<String>) -> Option<String> {
    let value = value?;
    validate_url(&value, false)
        .ok()
        .map(|url: Url| url.to_string())
}

fn clean_asset_url(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    if value.is_empty() || value.len() > 240 {
        return None;
    }
    if value.starts_with('/') {
        let safe = value.len() > 1
            && !value.starts_with("//")
            && !value.contains("..")
            && !value.contains('\\')
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'));
        return safe.then_some(value);
    }
    clean_https_url(Some(value))
}

fn clean_version(value: Option<String>) -> Option<String> {
    let value = value?.trim().trim_start_matches('v').to_string();
    if value.is_empty() || value.len() > 40 {
        return None;
    }
    semver::Version::parse(&value).ok().map(|v| v.to_string())
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
        if result.len() >= 48 {
            break;
        }
    }
    result.trim_matches('-').to_string()
}

impl LauncherInfo {
    pub fn validate(mut self) -> AppResult<Self> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(AppError::new("config_schema_unsupported").param(
                "schemaVersion",
                self.schema_version.to_string(),
            ));
        }
        self.latest_version = clean_version(self.latest_version);
        self.minimum_version = clean_version(self.minimum_version);
        self.notes.clean(MAX_BODY);
        self.maintenance.message.clean(MAX_DESCRIPTION);
        self.links.clean();

        self.announcements.truncate(10);
        let mut seen = std::collections::HashSet::new();
        let mut cleaned_announcements = Vec::new();
        for (index, mut announcement) in self.announcements.into_iter().enumerate() {
            announcement.title.clean(MAX_TITLE);
            announcement.body.clean(MAX_BODY);
            if announcement.title.is_empty() {
                continue;
            }
            let level = announcement.level.to_ascii_lowercase();
            announcement.level = match level.as_str() {
                "important" | "maintenance" | "event" | "warning" => level,
                _ => "info".to_string(),
            };
            let mut id = announcement
                .id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(64)
                .collect::<String>();
            if id.is_empty() {
                id = slug(&announcement.title.en);
            }
            if id.is_empty() {
                id = format!("announcement-{index}");
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            announcement.id = id;
            announcement.link = clean_https_url(announcement.link.take());
            cleaned_announcements.push(announcement);
        }
        self.announcements = cleaned_announcements;

        self.buttons.truncate(6);
        let mut buttons = Vec::new();
        for (index, mut button) in self.buttons.into_iter().enumerate() {
            button.label.clean(40);
            if button.label.is_empty() {
                continue;
            }
            let Some(url) = clean_https_url(Some(button.url.clone())) else {
                continue;
            };
            button.url = url;
            button.style = if button.style.eq_ignore_ascii_case("primary") {
                "primary".to_string()
            } else {
                "secondary".to_string()
            };
            button.icon = truncate_chars(&button.icon, 24);
            if button.icon.is_empty() {
                button.icon = "external".to_string();
            }
            let id = button
                .id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(48)
                .collect::<String>();
            button.id = if id.is_empty() {
                format!("button-{index}")
            } else {
                id
            };
            buttons.push(button);
        }
        self.buttons = buttons;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemeColors {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub primary: String,
    pub secondary: String,
    pub accent: String,
}

impl ThemeColors {
    pub fn dark_defaults() -> Self {
        Self {
            background: "#060912".to_string(),
            surface: "#0d1526".to_string(),
            text: "#f4f7ff".to_string(),
            muted: "#93a4bd".to_string(),
            border: "#26344c".to_string(),
            primary: "#9cc9ff".to_string(),
            secondary: "#b8b0ff".to_string(),
            accent: "#ffd27d".to_string(),
        }
    }

    pub fn light_defaults() -> Self {
        Self {
            background: "#eef3fb".to_string(),
            surface: "#ffffff".to_string(),
            text: "#0b1425".to_string(),
            muted: "#56667d".to_string(),
            border: "#c9d6e8".to_string(),
            primary: "#2d6fb8".to_string(),
            secondary: "#6858c7".to_string(),
            accent: "#a86b12".to_string(),
        }
    }

    fn normalize_color(value: &str, fallback: &str) -> String {
        let value = value.trim().to_ascii_lowercase();
        let valid = (value.len() == 4 || value.len() == 7 || value.len() == 9)
            && value.starts_with('#')
            && value[1..].chars().all(|c| c.is_ascii_hexdigit());
        if valid {
            value
        } else {
            fallback.to_string()
        }
    }

    fn clean(&mut self, fallback: &ThemeColors) {
        self.background = Self::normalize_color(&self.background, &fallback.background);
        self.surface = Self::normalize_color(&self.surface, &fallback.surface);
        self.text = Self::normalize_color(&self.text, &fallback.text);
        self.muted = Self::normalize_color(&self.muted, &fallback.muted);
        self.border = Self::normalize_color(&self.border, &fallback.border);
        self.primary = Self::normalize_color(&self.primary, &fallback.primary);
        self.secondary = Self::normalize_color(&self.secondary, &fallback.secondary);
        self.accent = Self::normalize_color(&self.accent, &fallback.accent);
    }
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self::dark_defaults()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LightThemeColors(pub ThemeColors);

impl Default for LightThemeColors {
    fn default() -> Self {
        Self(ThemeColors::light_defaults())
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawThemeColors {
    background: Option<String>,
    surface: Option<String>,
    text: Option<String>,
    muted: Option<String>,
    border: Option<String>,
    primary: Option<String>,
    secondary: Option<String>,
    accent: Option<String>,
}

impl<'de> Deserialize<'de> for LightThemeColors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawThemeColors::deserialize(deserializer)?;
        let base = ThemeColors::light_defaults();
        Ok(LightThemeColors(ThemeColors {
            background: raw.background.unwrap_or(base.background),
            surface: raw.surface.unwrap_or(base.surface),
            text: raw.text.unwrap_or(base.text),
            muted: raw.muted.unwrap_or(base.muted),
            border: raw.border.unwrap_or(base.border),
            primary: raw.primary.unwrap_or(base.primary),
            secondary: raw.secondary.unwrap_or(base.secondary),
            accent: raw.accent.unwrap_or(base.accent),
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub schema_version: u32,
    pub logo_url: Option<String>,
    pub background_url: Option<String>,
    pub banner_url: Option<String>,
    pub player_icon_url: Option<String>,
    pub tagline: LocalizedText,
    pub colors: ThemeColors,
    pub light_colors: LightThemeColors,
    pub background_overlay: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            logo_url: None,
            background_url: None,
            banner_url: None,
            player_icon_url: None,
            tagline: LocalizedText::default(),
            colors: ThemeColors::dark_defaults(),
            light_colors: LightThemeColors::default(),
            background_overlay: 0.55,
        }
    }
}

impl Appearance {
    pub fn validate(mut self) -> AppResult<Self> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(AppError::new("config_schema_unsupported").param(
                "schemaVersion",
                self.schema_version.to_string(),
            ));
        }
        self.logo_url = clean_asset_url(self.logo_url.take());
        self.background_url = clean_asset_url(self.background_url.take());
        self.banner_url = clean_asset_url(self.banner_url.take());
        self.player_icon_url = clean_asset_url(self.player_icon_url.take());
        self.tagline.clean(90);
        let dark = ThemeColors::dark_defaults();
        let light = ThemeColors::light_defaults();
        self.colors.clean(&dark);
        self.light_colors.0.clean(&light);
        if !self.background_overlay.is_finite() {
            self.background_overlay = 0.55;
        }
        self.background_overlay = self.background_overlay.clamp(0.0, 0.9);
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewsItem {
    pub id: String,
    pub title: LocalizedText,
    pub summary: LocalizedText,
    pub body: LocalizedText,
    pub image_url: Option<String>,
    pub published_at: Option<String>,
    pub link: Option<String>,
    pub pinned: bool,
}

impl Default for NewsItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: LocalizedText::default(),
            summary: LocalizedText::default(),
            body: LocalizedText::default(),
            image_url: None,
            published_at: None,
            link: None,
            pinned: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewsFeed {
    pub schema_version: u32,
    pub items: Vec<NewsItem>,
}

impl Default for NewsFeed {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

impl NewsFeed {
    pub fn validate(mut self) -> AppResult<Self> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(AppError::new("config_schema_unsupported").param(
                "schemaVersion",
                self.schema_version.to_string(),
            ));
        }
        self.items.truncate(30);
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        for (index, mut item) in self.items.into_iter().enumerate() {
            item.title.clean(MAX_TITLE);
            item.summary.clean(400);
            item.body.clean(MAX_BODY);
            if item.title.is_empty() {
                continue;
            }
            let mut id = item
                .id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(64)
                .collect::<String>();
            if id.is_empty() {
                id = slug(&item.title.en);
            }
            if id.is_empty() {
                id = format!("news-{index}");
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            item.id = id;
            item.image_url = clean_https_url(item.image_url.take());
            item.link = clean_https_url(item.link.take());
            item.published_at = item
                .published_at
                .take()
                .map(|d| truncate_chars(&d, 40))
                .filter(|d| !d.is_empty());
            items.push(item);
        }
        items.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.published_at.cmp(&a.published_at))
        });
        self.items = items;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MinecraftRuntime {
    pub version: String,
    pub manifest_url: String,
}

impl Default for MinecraftRuntime {
    fn default() -> Self {
        Self {
            version: "1.21.1".to_string(),
            manifest_url: "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NeoForgeRuntime {
    pub version: String,
    pub maven_base: String,
}

impl Default for NeoForgeRuntime {
    fn default() -> Self {
        Self {
            version: "21.1.248".to_string(),
            maven_base: "https://maven.neoforged.net/releases/".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoModpackRuntime {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub file_name: String,
}

impl Default for AutoModpackRuntime {
    fn default() -> Self {
        Self {
            version: "4.0.7".to_string(),
            url: "https://github.com/Skidamek/AutoModpack/releases/download/v4.0.7/automodpack-mc1.21.1-neoforge-4.0.7.jar".to_string(),
            sha256: "c76ac8e4c754ba39d1baee3a8ac146be9bfddfad58fa10cc48527d4accd47ce7".to_string(),
            file_name: "automodpack-mc1.21.1-neoforge-4.0.7.jar".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateRuntime {
    pub channel: String,
    pub public_key: String,
}

impl Default for UpdateRuntime {
    fn default() -> Self {
        Self {
            channel: "stable".to_string(),
            public_key: String::new(),
        }
    }
}

/// Build-time configurable internals (Minecraft/NeoForge/Java versions, pinned
/// AutoModpack artifact, update channel and public key). Player-editable
/// settings live in `Settings`; appearance/content live in remote JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    pub remote_base_url: String,
    pub minecraft: MinecraftRuntime,
    pub neoforge: NeoForgeRuntime,
    pub java_major: u32,
    pub adoptium_api_base: String,
    pub automodpack: AutoModpackRuntime,
    pub update: UpdateRuntime,
    pub server_entry_name: String,
    pub backup_folder_name: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            remote_base_url: "https://skyandbeyond.github.io/".to_string(),
            minecraft: MinecraftRuntime::default(),
            neoforge: NeoForgeRuntime::default(),
            java_major: 21,
            adoptium_api_base: "https://api.adoptium.net/v3/assets/latest/21/hotspot"
                .to_string(),
            automodpack: AutoModpackRuntime::default(),
            update: UpdateRuntime::default(),
            server_entry_name: "Sky & Beyond".to_string(),
            backup_folder_name: "snb-backups".to_string(),
        }
    }
}

fn validate_identifier(value: &str, label: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 48
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(AppError::with_detail(
            "runtime_config_invalid",
            format!("invalid {label}: {value:?}"),
        ));
    }
    Ok(())
}

impl RuntimeConfig {
    pub fn from_json(text: &str, allow_loopback_http: bool) -> AppResult<Self> {
        let config: RuntimeConfig = serde_json::from_str(text)?;
        config.validate(allow_loopback_http)?;
        Ok(config)
    }

    pub fn validate(&self, allow_loopback_http: bool) -> AppResult<()> {
        if self.schema_version != 1 {
            return Err(AppError::new("runtime_config_invalid"));
        }
        validate_url(&self.remote_base_url, allow_loopback_http).map_err(|_| {
            AppError::with_detail("runtime_config_invalid", "remoteBaseUrl must be HTTPS")
        })?;
        validate_url(&self.minecraft.manifest_url, false).map_err(|_| {
            AppError::with_detail("runtime_config_invalid", "minecraft.manifestUrl must be HTTPS")
        })?;
        validate_url(&self.neoforge.maven_base, false).map_err(|_| {
            AppError::with_detail("runtime_config_invalid", "neoforge.mavenBase must be HTTPS")
        })?;
        validate_url(&self.automodpack.url, false).map_err(|_| {
            AppError::with_detail("runtime_config_invalid", "automodpack.url must be HTTPS")
        })?;
        validate_url(&self.adoptium_api_base, false).map_err(|_| {
            AppError::with_detail("runtime_config_invalid", "adoptiumApiBase must be HTTPS")
        })?;
        validate_identifier(&self.minecraft.version, "minecraft.version")?;
        validate_identifier(&self.neoforge.version, "neoforge.version")?;
        if !(17..=99).contains(&self.java_major) {
            return Err(AppError::new("runtime_config_invalid"));
        }
        if self.automodpack.sha256.len() != 64
            || !self.automodpack.sha256.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(AppError::with_detail(
                "runtime_config_invalid",
                "automodpack.sha256 must be a 64-character hex digest",
            ));
        }
        let file_name = sanitize_file_name(&self.automodpack.file_name)
            .map_err(|_| AppError::new("runtime_config_invalid"))?;
        if !file_name.to_ascii_lowercase().ends_with(".jar") {
            return Err(AppError::new("runtime_config_invalid"));
        }
        if !matches!(self.update.channel.as_str(), "stable" | "beta") {
            return Err(AppError::new("runtime_config_invalid"));
        }
        if !self.update.public_key.trim().is_empty() {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(self.update.public_key.trim())
                .map_err(|_| AppError::new("runtime_config_invalid"))?;
            if decoded.len() != 32 {
                return Err(AppError::with_detail(
                    "runtime_config_invalid",
                    "update.publicKey must be a 32-byte Ed25519 public key in base64",
                ));
            }
        }
        if self.server_entry_name.trim().is_empty() || self.server_entry_name.len() > 60 {
            return Err(AppError::new("runtime_config_invalid"));
        }
        if sanitize_file_name(&self.backup_folder_name).is_err() {
            return Err(AppError::new("runtime_config_invalid"));
        }
        Ok(())
    }

    pub fn neoforge_installer_url(&self) -> String {
        format!(
            "{}/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar",
            self.neoforge.maven_base.trim_end_matches('/'),
            v = self.neoforge.version
        )
    }

    pub fn remote_url(&self, relative: &str) -> String {
        format!(
            "{}/{}",
            self.remote_base_url.trim_end_matches('/'),
            relative.trim_start_matches('/')
        )
    }

    pub fn update_manifest_relative(&self) -> &'static str {
        if self.update.channel == "beta" {
            "launcher/update-manifest-beta.json"
        } else {
            "launcher/update-manifest.json"
        }
    }

    pub fn update_manifest_url(&self) -> String {
        self.remote_url(self.update_manifest_relative())
    }

    pub fn java_download_api_url(&self) -> String {
        let os = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "mac",
            other => other,
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "aarch64",
            "x86" => "x86",
            other => other,
        };
        format!(
            "{base}?architecture={arch}&image_type=jre&os={os}&vendor=eclipse",
            base = self.adoptium_api_base.trim_end_matches('?'),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn localized_text_accepts_string_and_object() {
        let one: LocalizedText = serde_json::from_str("\"Bonjour\"").unwrap();
        assert_eq!(one.en, "Bonjour");
        assert_eq!(one.fr, "Bonjour");
        let two: LocalizedText =
            serde_json::from_str(r#"{"en":"Hello","fr":"Salut","extra":3}"#).unwrap();
        assert_eq!(two.pick("fr"), "Salut");
        assert_eq!(two.pick("en"), "Hello");
        let missing: LocalizedText = serde_json::from_str(r#"{"en":"Only English"}"#).unwrap();
        assert_eq!(missing.fr, "Only English");
    }

    #[test]
    fn server_endpoint_rejects_ips_and_internal_hosts() {
        assert_eq!(
            ServerEndpoint::parse("192.0.2.44", 25565).unwrap_err().code(),
            "endpoint_ip_literal"
        );
        assert_eq!(
            ServerEndpoint::parse("2001:db8::1", 25565).unwrap_err().code(),
            "endpoint_ip_literal"
        );
        assert_eq!(
            ServerEndpoint::parse("localhost", 25565).unwrap_err().code(),
            "endpoint_private_host"
        );
        assert_eq!(
            ServerEndpoint::parse("server.local", 25565).unwrap_err().code(),
            "endpoint_private_host"
        );
        assert_eq!(
            ServerEndpoint::parse("no-tld", 25565).unwrap_err().code(),
            "endpoint_host_invalid"
        );
        let endpoint = ServerEndpoint::parse(" SkyAndBeyond.Playit.gg. ", 25565).unwrap();
        assert_eq!(endpoint.address(), "skyandbeyond.playit.gg:25565");
        assert!(ServerEndpoint::parse("skyandbeyond.playit.gg", 0).is_err());
    }

    #[test]
    fn server_config_defaults_and_sanitises() {
        let config: ServerConfig = serde_json::from_str(
            r#"{"endpoint":{"host":"skyandbeyond.playit.gg","port":25565},"displayName":" Sky & Beyond "}"#,
        )
        .unwrap();
        let config = config.validate().unwrap();
        assert_eq!(config.display_name, "Sky & Beyond");
        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.endpoint.unwrap().port, 25565);

        let bad: ServerConfig =
            serde_json::from_str(r#"{"endpoint":{"host":"192.0.2.1","port":25565}}"#).unwrap();
        assert_eq!(bad.validate().unwrap_err().code(), "endpoint_ip_literal");
    }

    #[test]
    fn appearance_rejects_unknown_colors_and_urls() {
        let appearance: Appearance = serde_json::from_str(
            r##"{
                "backgroundUrl":"http://insecure.example/bg.png",
                "logoUrl":"https://example.com/logo.svg",
                "playerIconUrl":"/branding/player.png",
                "colors":{"background":"not-a-color","primary":"#ABC"},
                "lightColors":{},
                "backgroundOverlay":2.0
            }"##,
        )
        .unwrap();
        let appearance = appearance.validate().unwrap();
        assert!(appearance.background_url.is_none());
        assert_eq!(appearance.logo_url.as_deref(), Some("https://example.com/logo.svg"));
        assert_eq!(appearance.player_icon_url.as_deref(), Some("/branding/player.png"));
        assert_eq!(appearance.colors.background, "#060912");
        assert_eq!(appearance.colors.primary, "#abc");
        assert_eq!(appearance.background_overlay, 0.9);
        assert_eq!(appearance.light_colors.0.background, "#eef3fb");
    }

    #[test]
    fn appearance_accepts_bundled_brand_assets() {
        let appearance: Appearance = serde_json::from_str(
            r##"{
                "logoUrl":"/branding/logo.png",
                "backgroundUrl":"/branding/background.jpg",
                "playerIconUrl":"/branding/player.png"
            }"##,
        )
        .unwrap();
        let appearance = appearance.validate().unwrap();
        assert_eq!(appearance.logo_url.as_deref(), Some("/branding/logo.png"));
        assert_eq!(appearance.background_url.as_deref(), Some("/branding/background.jpg"));
        assert_eq!(appearance.player_icon_url.as_deref(), Some("/branding/player.png"));

        let traversal: Appearance =
            serde_json::from_str(r#"{"logoUrl":"/branding/../settings.json"}"#).unwrap();
        assert!(traversal.validate().unwrap().logo_url.is_none());
    }

    #[test]
    fn launcher_info_drops_invalid_buttons_and_deduplicates_announcements() {
        let info: LauncherInfo = serde_json::from_str(
            r#"{
                "latestVersion":"v0.2.0",
                "minimumVersion":"0.1.0",
                "buttons":[
                    {"label":"Discord","url":"https://discord.gg/sky"},
                    {"label":"Evil","url":"javascript:alert(1)"},
                    {"label":"Site","url":"https://example.com"}
                ],
                "announcements":[
                    {"id":"event","title":"Event","level":"event","body":"Come play"},
                    {"id":"event","title":"Duplicate"},
                    {"title":"Maintenance","level":"maintenance"}
                ],
                "links":{"discord":"https://discord.gg/sky","website":"ftp://example.com"}
            }"#,
        )
        .unwrap();
        let info = info.validate().unwrap();
        assert_eq!(info.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(info.buttons.len(), 2);
        assert_eq!(info.announcements.len(), 2);
        assert_eq!(info.announcements[0].level, "event");
        assert_eq!(info.links.discord.as_deref(), Some("https://discord.gg/sky"));
        assert!(info.links.website.is_none());
    }

    #[test]
    fn news_is_sorted_and_sanitised() {
        let feed: NewsFeed = serde_json::from_str(
            r#"{"items":[
                {"title":"Older","publishedAt":"2026-01-01"},
                {"title":"Newer","publishedAt":"2026-06-01"},
                {"title":"Important","pinned":true},
                {"title":""},
                {"title":"Bad image","imageUrl":"http://example.com/a.png"}
            ]}"#,
        )
        .unwrap();
        let feed = feed.validate().unwrap();
        assert_eq!(feed.items.len(), 4);
        assert_eq!(feed.items[0].title.en, "Important");
        assert_eq!(feed.items[1].title.en, "Newer");
        assert!(feed.items[3].image_url.is_none());
        assert!(!feed.items[0].id.is_empty());
    }

    #[test]
    fn runtime_config_is_validated() {
        let config = RuntimeConfig::default();
        assert!(config.validate(true).is_ok());
        assert!(config
            .neoforge_installer_url()
            .contains("net/neoforged/neoforge/21.1.248"));
        assert_eq!(config.update_manifest_relative(), "launcher/update-manifest.json");
        let mut invalid = config.clone();
        invalid.remote_base_url = "http://example.com/".to_string();
        assert!(invalid.validate(true).is_err());
        invalid.remote_base_url = "https://example.com/".to_string();
        invalid.automodpack.sha256 = "xyz".to_string();
        assert!(invalid.validate(true).is_err());
        invalid.automodpack.sha256 = config.automodpack.sha256.clone();
        invalid.automodpack.file_name = "../evil.jar".to_string();
        assert!(invalid.validate(true).is_err());
    }

    #[test]
    fn repository_config_examples_validate() {
        let server: ServerConfig =
            serde_json::from_str(include_str!("../../../repository/config/server.json")).unwrap();
        let server = server.validate().unwrap();
        assert_eq!(server.display_name, "Sky & Beyond");
        assert!(server.endpoint.is_some());
        let launcher: LauncherInfo =
            serde_json::from_str(include_str!("../../../repository/config/launcher.json")).unwrap();
        launcher.validate().unwrap();
        let appearance: Appearance =
            serde_json::from_str(include_str!("../../../repository/config/appearance.json")).unwrap();
        appearance.validate().unwrap();
        let news: NewsFeed =
            serde_json::from_str(include_str!("../../../repository/config/news.json")).unwrap();
        news.validate().unwrap();
    }

    #[test]
    fn repository_runtime_resource_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/resources/launcher-config.json");
        let text = std::fs::read_to_string(path).expect("bundled runtime config must exist");
        let config = RuntimeConfig::from_json(&text, false).expect("bundled config must be valid");
        assert_eq!(config.minecraft.version, "1.21.1");
        assert_eq!(config.neoforge.version, "21.1.248");
        assert_eq!(config.java_major, 21);
        assert_eq!(config.automodpack.version, "4.0.7");
    }
}
