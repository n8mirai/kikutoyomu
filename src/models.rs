use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtCustomTab {
    pub id: String,
    pub label: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub toggl_api_token: String,
    #[serde(default)]
    pub gemini_api_key: String,
    #[serde(default)]
    pub cloud_sync_url: String,
    #[serde(default)]
    pub yt_tabs: Vec<YtCustomTab>,
    pub toggl_workspace_id: Option<i64>,
    pub toggl_workspace_name: String,
    pub toggl_project_id: Option<i64>,
    pub toggl_project_name: String,
    pub toggl_tags: Vec<String>,
    pub toggl_tag_anime: bool,
    pub toggl_auto_sync: bool,
    pub toggl_min_seconds: u64,
    pub description_template: String,
    pub daily_goal_minutes: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            toggl_api_token: String::new(),
            gemini_api_key: String::new(),
            cloud_sync_url: String::new(),
            yt_tabs: Vec::new(),
            toggl_workspace_id: None,
            toggl_workspace_name: String::new(),
            toggl_project_id: None,
            toggl_project_name: String::new(),
            toggl_tags: vec!["japanese".into(), "immersion".into()],
            toggl_tag_anime: true,
            toggl_auto_sync: true,
            toggl_min_seconds: 5,
            description_template: "{anime} — {episode}".into(),
            daily_goal_minutes: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub number: u32,
    pub title: String,
    pub jp_title: String,
    pub filename: Option<String>,
    pub file_size: u64,
    pub duration_secs: Option<u64>,
    pub available: bool,
    pub has_subtitles: bool,
    pub subtitle_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimeSeries {
    pub id: String,
    pub title: String,
    pub jp_title: String,
    pub subtitle: String,
    pub accent: String,
    pub has_cover: bool,
    pub cover_file: Option<String>,
    pub folder_path: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub anime_id: String,
    pub episode_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub items: Vec<PlaylistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub anime_id: String,
    pub anime_title: String,
    pub episode_id: String,
    pub episode_title: String,
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub duration_secs: u64,
    pub toggl_entry_id: Option<i64>,
    pub toggl_synced: bool,
    pub toggl_error: Option<String>,
    #[serde(default)]
    pub words_looked_up: u32,
    #[serde(default)]
    pub words_marked_known: u32,
    #[serde(default)]
    pub looked_up_words: Vec<String>,
    #[serde(default)]
    pub learning_words: Vec<String>,
    #[serde(default)]
    pub known_words: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSession {
    pub id: String,
    pub anime_id: String,
    pub anime_title: String,
    pub episode_id: String,
    pub episode_title: String,
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub started_at: String,
    pub toggl_entry_id: Option<i64>,
    pub toggl_workspace_id: Option<i64>,
    pub toggl_live: bool,
    pub toggl_error: Option<String>,
    #[serde(default)]
    pub words_looked_up: u32,
    #[serde(default)]
    pub words_marked_known: u32,
    #[serde(default)]
    pub looked_up_words: Vec<String>,
    #[serde(default)]
    pub learning_words: Vec<String>,
    #[serde(default)]
    pub known_words: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EpisodeProgress {
    pub position_secs: f64,
    pub duration_secs: f64,
    pub completions: u32,
    pub last_played_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedData {
    pub config: AppConfig,
    pub custom_anime: Vec<CustomAnimeMeta>,
    pub playlists: Vec<Playlist>,
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub progress: BTreeMap<String, EpisodeProgress>,
}

/// Clean portable user profile stored in `profile/profile.json` (safe to commit & sync via git — zero secrets!).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserProfile {
    #[serde(default = "default_profile_name")]
    pub profile_name: String,
    #[serde(default)]
    pub vocab: crate::vocab::VocabStore,
    #[serde(default)]
    pub config: AppConfig,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub progress: BTreeMap<String, EpisodeProgress>,
    #[serde(default)]
    pub custom_anime: Vec<CustomAnimeMeta>,
}

fn default_profile_name() -> String {
    "default".to_string()
}

/// Google OAuth 2.0 credentials & tokens stored strictly in gitignored `data/secrets.json` (never committed).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoogleOAuthSecrets {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at_unix: u64,
    #[serde(default)]
    pub email: String,
}

/// Machine-local credentials stored in gitignored `data/secrets.json` (never committed).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalSecrets {
    #[serde(default)]
    pub toggl_api_token: String,
    #[serde(default)]
    pub gemini_api_key: String,
    #[serde(default)]
    pub cloud_sync_secret: String,
    #[serde(default)]
    pub google_oauth: GoogleOAuthSecrets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAnimeMeta {
    pub id: String,
    pub title: String,
    pub jp_title: String,
    pub subtitle: String,
    pub accent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TogglWorkspace {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TogglProject {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicConfig {
    pub has_toggl_token: bool,
    pub toggl_token_hint: String,
    pub has_gemini_key: bool,
    pub google_oauth_connected: bool,
    pub google_oauth_email: String,
    pub google_oauth_client_id: String,
    pub has_google_client_secret: bool,
    pub gemini_cli_installed: bool,
    pub gemini_cli_signed_in: bool,
    pub has_ai_auth: bool,
    pub cloud_sync_url: String,
    pub has_cloud_sync_secret: bool,
    pub yt_tabs: Vec<YtCustomTab>,
    pub toggl_workspace_id: Option<i64>,
    pub toggl_workspace_name: String,
    pub toggl_project_id: Option<i64>,
    pub toggl_project_name: String,
    pub toggl_tags: Vec<String>,
    pub toggl_tag_anime: bool,
    pub toggl_auto_sync: bool,
    pub toggl_min_seconds: u64,
    pub description_template: String,
    pub daily_goal_minutes: u32,
}

impl From<&AppConfig> for PublicConfig {
    fn from(c: &AppConfig) -> Self {
        let hint = if c.toggl_api_token.len() > 6 {
            format!(
                "{}••••{}",
                &c.toggl_api_token[..3],
                &c.toggl_api_token[c.toggl_api_token.len() - 3..]
            )
        } else if !c.toggl_api_token.is_empty() {
            "••••••".into()
        } else {
            String::new()
        };
        let has_gem = !c.gemini_api_key.trim().is_empty();
        Self {
            has_toggl_token: !c.toggl_api_token.trim().is_empty(),
            toggl_token_hint: hint,
            has_gemini_key: has_gem,
            google_oauth_connected: false,
            google_oauth_email: String::new(),
            google_oauth_client_id: String::new(),
            has_google_client_secret: false,
            gemini_cli_installed: false,
            gemini_cli_signed_in: false,
            has_ai_auth: has_gem,
            cloud_sync_url: c.cloud_sync_url.clone(),
            has_cloud_sync_secret: false,
            yt_tabs: c.yt_tabs.clone(),
            toggl_workspace_id: c.toggl_workspace_id,
            toggl_workspace_name: c.toggl_workspace_name.clone(),
            toggl_project_id: c.toggl_project_id,
            toggl_project_name: c.toggl_project_name.clone(),
            toggl_tags: c.toggl_tags.clone(),
            toggl_tag_anime: c.toggl_tag_anime,
            toggl_auto_sync: c.toggl_auto_sync,
            toggl_min_seconds: c.toggl_min_seconds,
            description_template: c.description_template.clone(),
            daily_goal_minutes: c.daily_goal_minutes,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImmersionStats {
    pub today_secs: u64,
    pub total_secs: u64,
    pub session_count: usize,
    pub streak_days: u32,
    pub per_anime: Vec<AnimeStat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimeStat {
    pub anime_id: String,
    pub anime_title: String,
    pub total_secs: u64,
    pub sessions: usize,
}
