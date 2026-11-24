use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::agent::{self, AgentAction, AgentMessage};
use crate::cloud_sync;
use crate::dict::{LocalJmDict, LocalNameDict};
use crate::knowledge::{CalibrationCandidate, KnowledgeModel};
use crate::library::{
    default_playlists, detect_anime_for_file, scan_library, slugify, sync_japanese_stories_playlist,
};
use crate::models::{
    ActiveSession, AnimeSeries, AnimeStat, AppConfig, CustomAnimeMeta, EpisodeProgress,
    GoogleOAuthSecrets, HistoryEntry, ImmersionStats, LocalSecrets, PersistedData, Playlist,
    PlaylistItem, PublicConfig, UserProfile, YtCustomTab,
};
use crate::morph::{MorphAnalyzer, MorphToken};
use crate::oauth::{self, AiAuth, PendingOAuth};
use crate::subs;
use crate::synopsis::{self, EpisodeSynopsis, SynopsisStore};
use crate::timeutil;
use crate::toggl;
use crate::vocab::VocabStore;
use crate::youtube::{self, YoutubeStore};

const UI_HTML: &str = include_str!("ui.html");

pub struct SharedState {
    pub app_root: PathBuf,
    pub profile_file: PathBuf,
    pub secrets_file: PathBuf,
    pub vocab_file: PathBuf,
    pub synopsis_file: PathBuf,
    pub youtube_file: PathBuf,
    pub yt_subs_dir: PathBuf,
    pub media_root: PathBuf,
    pub data: PersistedData,
    pub library: Vec<AnimeSeries>,
    pub active_session: Option<ActiveSession>,
    pub morph: Arc<Mutex<Option<MorphAnalyzer>>>,
    pub jmdict: Arc<Mutex<Option<LocalJmDict>>>,
    pub jmnedict: Arc<Mutex<Option<LocalNameDict>>>,
    pub vocab: VocabStore,
    pub synopses: SynopsisStore,
    pub youtube: YoutubeStore,
    pub yt_feed_cache: Vec<youtube::FeedVideo>,
    pub yt_feed_fetched_unix: u64,
    pub google_oauth: GoogleOAuthSecrets,
    pub pending_oauth: Option<PendingOAuth>,
    pub cloud_sync_secret: String,
}

impl SharedState {
    pub fn load(app_root: &Path) -> Self {
        let data_dir = app_root.join("data");
        let profile_dir = app_root.join("profile");
        let _ = fs::create_dir_all(&data_dir);
        let _ = fs::create_dir_all(&profile_dir);

        let profile_file = profile_dir.join("profile.json");
        let secrets_file = data_dir.join("secrets.json");
        let legacy_state_file = data_dir.join("state.json");
        let vocab_file = data_dir.join("vocab.json");
        let synopsis_file = data_dir.join("synopses.json");
        let youtube_file = data_dir.join("youtube.json");
        let yt_subs_dir = data_dir.join("yt_subs");
        let _ = fs::create_dir_all(&yt_subs_dir);
        let ipadic_file = data_dir.join("ipadic.dic.zst");
        let jmdict_file = data_dir.join("jmdict-eng.json");
        let jmnedict_file = data_dir.join("jmnedict.json");
        let media_root = app_root.join("media");

        // 1. Load portable user profile from `profile/profile.json`, or auto-migrate from legacy `data/state.json` + `data/vocab.json`
        let (mut data, vocab) = if profile_file.exists() {
            let prof: UserProfile = fs::read_to_string(&profile_file)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let pd = PersistedData {
                config: prof.config,
                custom_anime: prof.custom_anime,
                playlists: prof.playlists,
                history: prof.history,
                progress: prof.progress,
            };
            (pd, prof.vocab)
        } else {
            let pd: PersistedData = if legacy_state_file.exists() {
                fs::read_to_string(&legacy_state_file)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default()
            } else {
                PersistedData::default()
            };
            let vb = VocabStore::load(&vocab_file);
            (pd, vb)
        };

        // 2. Load machine-local secrets (Toggl + optional Gemini API key) from gitignored `data/secrets.json`
        let loaded_secrets = fs::read_to_string(&secrets_file)
            .ok()
            .and_then(|s| serde_json::from_str::<LocalSecrets>(&s).ok())
            .unwrap_or_default();
        let mut token = loaded_secrets.toggl_api_token;
        if token.is_empty() && !data.config.toggl_api_token.is_empty() {
            token = data.config.toggl_api_token.clone();
        }
        if token.is_empty() && legacy_state_file.exists() {
            if let Some(leg) = fs::read_to_string(&legacy_state_file)
                .ok()
                .and_then(|s| serde_json::from_str::<PersistedData>(&s).ok())
            {
                if !leg.config.toggl_api_token.is_empty() {
                    token = leg.config.toggl_api_token;
                }
            }
        }
        data.config.toggl_api_token = token;
        if !loaded_secrets.gemini_api_key.is_empty() {
            data.config.gemini_api_key = loaded_secrets.gemini_api_key;
        }
        let google_oauth = loaded_secrets.google_oauth;
        let cloud_sync_secret = loaded_secrets.cloud_sync_secret;
        let synopses = SynopsisStore::load(&synopsis_file);
        let youtube = YoutubeStore::load(&youtube_file);

        let library = scan_library(&media_root, &data.custom_anime);
        if data.playlists.is_empty() {
            data.playlists = default_playlists(&library);
        } else {
            sync_japanese_stories_playlist(&mut data.playlists, &library);
        }

        // Load MeCab/IPADIC (~150ms!) FIRST, then 219k JMdict + 715k JMnedict on background thread
        let morph: Arc<Mutex<Option<MorphAnalyzer>>> = Arc::new(Mutex::new(None));
        let jmdict: Arc<Mutex<Option<LocalJmDict>>> = Arc::new(Mutex::new(None));
        let jmnedict: Arc<Mutex<Option<LocalNameDict>>> = Arc::new(Mutex::new(None));
        {
            let slot_mp = Arc::clone(&morph);
            let slot_jm = Arc::clone(&jmdict);
            let slot_nm = Arc::clone(&jmnedict);
            thread::spawn(move || {
                if ipadic_file.exists() {
                    if let Ok(loaded) = MorphAnalyzer::load_from_zst(&ipadic_file) {
                        *slot_mp.lock().unwrap() = Some(loaded);
                    }
                }
                if jmdict_file.exists() {
                    if let Ok(loaded) = LocalJmDict::load_from_file(&jmdict_file) {
                        *slot_jm.lock().unwrap() = Some(loaded);
                    }
                }
                if jmnedict_file.exists() {
                    if let Ok(loaded) = LocalNameDict::load_from_file(&jmnedict_file) {
                        *slot_nm.lock().unwrap() = Some(loaded);
                    }
                }
            });
        }

        let state = Self {
            app_root: app_root.to_path_buf(),
            profile_file,
            secrets_file,
            vocab_file,
            synopsis_file,
            youtube_file,
            yt_subs_dir,
            media_root,
            data,
            library,
            active_session: None,
            morph,
            jmdict,
            jmnedict,
            vocab,
            synopses,
            youtube,
            yt_feed_cache: Vec::new(),
            yt_feed_fetched_unix: 0,
            google_oauth,
            pending_oauth: None,
            cloud_sync_secret,
        };
        state.save();
        state
    }

    pub fn find_whisper(&self) -> Option<(PathBuf, PathBuf)> {
        let wdir = self.app_root.join("tools").join("whisper");
        let exe = wdir.join("whisper-cli.exe");
        if !exe.exists() {
            return None;
        }
        for name in [
            "ggml-small-q5_1.bin",
            "ggml-small.bin",
            "ggml-base.bin",
            "ggml-medium-q5_0.bin",
            "ggml-tiny.bin",
        ] {
            let m = wdir.join(name);
            if m.exists() {
                return Some((exe, m));
            }
        }
        None
    }

    pub fn to_clean_profile(&self) -> UserProfile {
        let mut clean_cfg = self.data.config.clone();
        clean_cfg.toggl_api_token.clear();
        clean_cfg.gemini_api_key.clear();
        UserProfile {
            profile_name: "default".to_string(),
            vocab: self.vocab.clone(),
            config: clean_cfg,
            playlists: self.data.playlists.clone(),
            history: self.data.history.clone(),
            progress: self.data.progress.clone(),
            custom_anime: self.data.custom_anime.clone(),
        }
    }

    pub fn apply_merged_profile(&mut self, prof: UserProfile) {
        self.vocab = prof.vocab;
        self.data.playlists = prof.playlists;
        self.data.history = prof.history;
        self.data.progress = prof.progress;
        self.data.custom_anime = prof.custom_anime;
        sync_japanese_stories_playlist(&mut self.data.playlists, &self.library);
    }

    pub fn save(&self) {
        // 1. Write portable, secret-free user profile to `profile/profile.json`
        let prof = self.to_clean_profile();
        if let Ok(json) = serde_json::to_string_pretty(&prof) {
            let tmp = self.profile_file.with_extension("json.tmp");
            if fs::write(&tmp, &json).is_ok() {
                let _ = fs::rename(&tmp, &self.profile_file);
            } else {
                let _ = fs::write(&self.profile_file, &json);
            }
        }
        // 2. Write machine-local secrets (Toggl + Gemini + Google OAuth + Cloud Sync) to gitignored `data/secrets.json`
        let sec = LocalSecrets {
            toggl_api_token: self.data.config.toggl_api_token.clone(),
            gemini_api_key: self.data.config.gemini_api_key.clone(),
            google_oauth: self.google_oauth.clone(),
            cloud_sync_secret: self.cloud_sync_secret.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&sec) {
            let tmp = self.secrets_file.with_extension("json.tmp");
            if fs::write(&tmp, &json).is_ok() {
                let _ = fs::rename(&tmp, &self.secrets_file);
            } else {
                let _ = fs::write(&self.secrets_file, &json);
            }
        }
    }

    pub fn public_config(&self) -> PublicConfig {
        let mut pc = PublicConfig::from(&self.data.config);
        let oauth_ok = !self.google_oauth.access_token.trim().is_empty()
            || !self.google_oauth.refresh_token.trim().is_empty();
        let gcli_installed = cloud_sync::find_gemini_cli(&self.app_root).is_some();
        let gcli_signed_in = gcli_installed && cloud_sync::is_gemini_cli_signed_in();
        pc.google_oauth_connected = oauth_ok;
        pc.google_oauth_email = self.google_oauth.email.clone();
        pc.google_oauth_client_id = self.google_oauth.client_id.clone();
        pc.has_google_client_secret = !self.google_oauth.client_secret.trim().is_empty();
        pc.gemini_cli_installed = gcli_installed;
        pc.gemini_cli_signed_in = gcli_signed_in;
        pc.cloud_sync_url = self.data.config.cloud_sync_url.clone();
        pc.has_cloud_sync_secret = !self.cloud_sync_secret.trim().is_empty();
        pc.has_ai_auth = pc.has_gemini_key || oauth_ok;
        pc
    }

    pub fn rescan(&mut self) {
        self.library = scan_library(&self.media_root, &self.data.custom_anime);
        sync_japanese_stories_playlist(&mut self.data.playlists, &self.library);
    }

    pub fn compute_stats(&self) -> ImmersionStats {
        let today = timeutil::today_local_day_index();
        let mut today_secs: u64 = 0;
        let mut total_secs: u64 = 0;
        let mut days: BTreeSet<i64> = BTreeSet::new();
        let mut by_anime: BTreeMap<String, (String, u64, usize)> = BTreeMap::new();

        for h in &self.data.history {
            total_secs += h.duration_secs;
            if let Some(unix) = timeutil::parse_rfc3339_unix(&h.started_at) {
                let local_day = timeutil::local_day_index(unix);
                days.insert(local_day);
                if local_day == today {
                    today_secs += h.duration_secs;
                }
            }
            let entry = by_anime
                .entry(h.anime_id.clone())
                .or_insert_with(|| (h.anime_title.clone(), 0, 0));
            entry.1 += h.duration_secs;
            entry.2 += 1;
        }

        // Streak calculation (today or yesterday counts as active streak)
        let mut streak: u32 = 0;
        let mut cursor = if days.contains(&today) {
            today
        } else if days.contains(&(today - 1)) {
            today - 1
        } else {
            today
        };
        while days.contains(&cursor) {
            streak += 1;
            cursor -= 1;
        }

        let mut per_anime: Vec<AnimeStat> = by_anime
            .into_iter()
            .map(|(anime_id, (anime_title, total_secs, sessions))| AnimeStat {
                anime_id,
                anime_title,
                total_secs,
                sessions,
            })
            .collect();
        per_anime.sort_by(|a, b| b.total_secs.cmp(&a.total_secs));

        ImmersionStats {
            today_secs,
            total_secs,
            session_count: self.data.history.len(),
            streak_days: streak,
            per_anime,
        }
    }
}

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap()
}

fn respond_json<T: Serialize>(req: Request, status: u16, value: &T) {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    let resp = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(json_header());
    let _ = req.respond(resp);
}

fn respond_err(req: Request, status: u16, msg: &str) {
    respond_json(req, status, &serde_json::json!({ "error": msg }));
}

fn read_body_string(req: &mut Request) -> Result<String, String> {
    let mut s = String::new();
    req.as_reader()
        .read_to_string(&mut s)
        .map_err(|e| format!("Failed reading request body: {}", e))?;
    Ok(s)
}

fn mime_for_ext(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "webm" => "audio/webm",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn safe_segment(seg: &str) -> Option<String> {
    let decoded = percent_decode_str(seg).decode_utf8().ok()?.into_owned();
    if decoded.is_empty()
        || decoded.contains('/')
        || decoded.contains('\\')
        || decoded.contains("..")
        || decoded.contains('\0')
    {
        None
    } else {
        Some(decoded)
    }
}

fn build_toggl_description(cfg: &AppConfig, anime_title: &str, episode_title: &str) -> String {
    let tpl = if cfg.description_template.trim().is_empty() {
        "{anime} — {episode}"
    } else {
        cfg.description_template.as_str()
    };
    tpl.replace("{anime}", anime_title)
        .replace("{episode}", episode_title)
}

fn build_toggl_tags(cfg: &AppConfig, anime_title: &str) -> Vec<String> {
    let mut tags: Vec<String> = cfg
        .toggl_tags
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if cfg.toggl_tag_anime && !anime_title.trim().is_empty() {
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(anime_title)) {
            tags.push(anime_title.to_string());
        }
    }
    tags
}

fn stop_active_session_internal(
    state_arc: &Arc<Mutex<SharedState>>,
    override_duration_secs: Option<u64>,
) -> Option<HistoryEntry> {
    let (active, cfg) = {
        let mut st = state_arc.lock().unwrap();
        let active = st.active_session.take()?;
        (active, st.data.config.clone())
    };

    let now_unix = timeutil::now_unix_secs();
    let ended_at = timeutil::format_utc_rfc3339(now_unix);
    let wall_secs = timeutil::parse_rfc3339_unix(&active.started_at)
        .map(|s| (now_unix - s).max(0) as u64)
        .unwrap_or(0);
    let duration_secs = override_duration_secs
        .map(|d| d.max(1))
        .unwrap_or(wall_secs)
        .max(1);

    let min_secs = cfg.toggl_min_seconds;
    if duration_secs < min_secs {
        return None;
    }

    // Airgapped local logging: NEVER hit Toggl API on pause/stop!
    // Coalesce consecutive unsynced chunks for the same episode so frequent pause/resume
    // becomes ONE clean consolidated entry synced on the 15-min cycle or on app close.
    let mut st = state_arc.lock().unwrap();
    if let Some(prev) = st.data.history.first_mut() {
        if !prev.toggl_synced
            && prev.anime_id == active.anime_id
            && prev.episode_id == active.episode_id
        {
            prev.duration_secs += duration_secs;
            prev.ended_at = ended_at;
            prev.words_looked_up = prev.words_looked_up.saturating_add(active.words_looked_up);
            prev.words_marked_known = prev.words_marked_known.saturating_add(active.words_marked_known);
            for w in active.looked_up_words {
                if !prev.looked_up_words.contains(&w) { prev.looked_up_words.push(w); }
            }
            for w in active.learning_words {
                if !prev.learning_words.contains(&w) { prev.learning_words.push(w); }
            }
            for w in active.known_words {
                if !prev.known_words.contains(&w) { prev.known_words.push(w); }
            }
            let merged = prev.clone();
            st.save();
            return Some(merged);
        }
    }

    let entry = HistoryEntry {
        id: active.id,
        anime_id: active.anime_id,
        anime_title: active.anime_title,
        episode_id: active.episode_id,
        episode_title: active.episode_title,
        playlist_id: active.playlist_id,
        playlist_name: active.playlist_name,
        started_at: active.started_at,
        ended_at,
        duration_secs,
        toggl_entry_id: None,
        toggl_synced: false,
        toggl_error: None,
        words_looked_up: active.words_looked_up,
        words_marked_known: active.words_marked_known,
        looked_up_words: active.looked_up_words,
        learning_words: active.learning_words,
        known_words: active.known_words,
    };
    st.data.history.insert(0, entry.clone());
    st.save();
    Some(entry)
}

/// Batch-commits all pending unsynced local history entries to Toggl (called on 15-min cycle & on app close).
fn flush_pending_toggl(state_arc: &Arc<Mutex<SharedState>>) -> usize {
    let (cfg, pending): (AppConfig, Vec<HistoryEntry>) = {
        let st = state_arc.lock().unwrap();
        let cfg = st.data.config.clone();
        if !cfg.toggl_auto_sync || cfg.toggl_api_token.trim().is_empty() || cfg.toggl_workspace_id.is_none() {
            return 0;
        }
        let items: Vec<HistoryEntry> = st
            .data
            .history
            .iter()
            .filter(|h| !h.toggl_synced && h.duration_secs >= cfg.toggl_min_seconds)
            .cloned()
            .collect();
        (cfg, items)
    };

    if pending.is_empty() {
        return 0;
    }
    let wid = cfg.toggl_workspace_id.unwrap();
    let mut synced_count = 0usize;

    for entry in pending {
        let desc = build_toggl_description(&cfg, &entry.anime_title, &entry.episode_title);
        let tags = build_toggl_tags(&cfg, &entry.anime_title);
        match toggl::create_completed_entry(
            &cfg.toggl_api_token,
            wid,
            cfg.toggl_project_id,
            &desc,
            &tags,
            &entry.started_at,
            &entry.ended_at,
            entry.duration_secs,
        ) {
            Ok(eid) => {
                let mut st = state_arc.lock().unwrap();
                if let Some(h) = st.data.history.iter_mut().find(|h| h.id == entry.id) {
                    h.toggl_entry_id = Some(eid);
                    h.toggl_synced = true;
                    h.toggl_error = None;
                }
                st.save();
                synced_count += 1;
            }
            Err(e) => {
                let mut st = state_arc.lock().unwrap();
                if let Some(h) = st.data.history.iter_mut().find(|h| h.id == entry.id) {
                    h.toggl_error = Some(e);
                }
                st.save();
                // Stop batch early on rate-limit / network error so we don't burn quota
                break;
            }
        }
    }
    synced_count
}

#[derive(Deserialize)]
struct CreateAnimeReq {
    title: String,
    #[serde(default)]
    jp_title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    accent: String,
}

#[derive(Deserialize)]
struct PlaylistReq {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    items: Vec<PlaylistItem>,
}

#[derive(Deserialize)]
struct SessionStartReq {
    anime_id: String,
    episode_id: String,
    #[serde(default)]
    playlist_id: Option<String>,
}

#[derive(Deserialize)]
struct SessionStopReq {
    #[serde(default)]
    elapsed_secs: Option<u64>,
}

#[derive(Deserialize)]
struct ConfigUpdateReq {
    #[serde(default)]
    toggl_api_token: Option<String>,
    #[serde(default)]
    clear_toggl_token: bool,
    #[serde(default)]
    gemini_api_key: Option<String>,
    #[serde(default)]
    toggl_workspace_id: Option<i64>,
    #[serde(default)]
    toggl_workspace_name: Option<String>,
    #[serde(default)]
    toggl_project_id: Option<i64>,
    #[serde(default)]
    clear_toggl_project: bool,
    #[serde(default)]
    toggl_project_name: Option<String>,
    #[serde(default)]
    toggl_tags: Option<Vec<String>>,
    #[serde(default)]
    toggl_tag_anime: Option<bool>,
    #[serde(default)]
    toggl_auto_sync: Option<bool>,
    #[serde(default)]
    toggl_min_seconds: Option<u64>,
    #[serde(default)]
    description_template: Option<String>,
    #[serde(default)]
    daily_goal_minutes: Option<u32>,
    #[serde(default)]
    cloud_sync_url: Option<String>,
    #[serde(default)]
    cloud_sync_secret: Option<String>,
    #[serde(default)]
    yt_tabs: Option<Vec<YtCustomTab>>,
}

#[derive(Deserialize)]
struct TogglTestReq {
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize)]
struct VocabSetReq {
    word: String,
    status: String,
}

#[derive(Deserialize)]
struct VocabImportReq {
    text: String,
    #[serde(default = "default_known_status")]
    status: String,
}

#[derive(Deserialize)]
struct ProgressUpdateReq {
    anime_id: String,
    episode_id: String,
    position_secs: f64,
    #[serde(default)]
    duration_secs: f64,
    #[serde(default)]
    completed: bool,
}

fn default_known_status() -> String {
    "known".into()
}

fn handle_stream(req: Request, file_path: &Path) {
    let mut file = match File::open(file_path) {
        Ok(f) => f,
        Err(_) => {
            respond_err(req, 404, "Audio file not found");
            return;
        }
    };
    let file_len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => {
            respond_err(req, 500, "Could not inspect audio file");
            return;
        }
    };
    let mime = mime_for_ext(file_path);

    let range_hdr = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());

    if let Some(rh) = range_hdr {
        if let Some(spec) = rh.strip_prefix("bytes=") {
            let parts: Vec<&str> = spec.split(',').next().unwrap_or("").split('-').collect();
            if parts.len() == 2 {
                let start: u64 = parts[0].trim().parse().unwrap_or(0);
                let end: u64 = if parts[1].trim().is_empty() {
                    // Stream in smooth 1 MB chunks or up to EOF
                    (start + 1_048_575).min(file_len.saturating_sub(1))
                } else {
                    parts[1]
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(file_len.saturating_sub(1))
                        .min(file_len.saturating_sub(1))
                };
                if start >= file_len || start > end {
                    let resp = Response::empty(StatusCode(416)).with_header(
                        Header::from_bytes("Content-Range", format!("bytes */{}", file_len))
                            .unwrap(),
                    );
                    let _ = req.respond(resp);
                    return;
                }
                let chunk_len = (end - start) + 1;
                if file.seek(SeekFrom::Start(start)).is_ok() {
                    let mut buf = vec![0u8; chunk_len as usize];
                    if file.read_exact(&mut buf).is_ok() {
                        let resp = Response::from_data(buf)
                            .with_status_code(StatusCode(206))
                            .with_header(Header::from_bytes("Content-Type", mime).unwrap())
                            .with_header(Header::from_bytes("Accept-Ranges", "bytes").unwrap())
                            .with_header(
                                Header::from_bytes(
                                    "Content-Range",
                                    format!("bytes {}-{}/{}", start, end, file_len),
                                )
                                .unwrap(),
                            )
                            .with_header(Header::from_bytes("Cache-Control", "no-cache").unwrap());
                        let _ = req.respond(resp);
                        return;
                    }
                }
            }
        }
    }

    let resp = Response::from_file(file)
        .with_status_code(StatusCode(200))
        .with_header(Header::from_bytes("Content-Type", mime).unwrap())
        .with_header(Header::from_bytes("Accept-Ranges", "bytes").unwrap());
    let _ = req.respond(resp);
}

/// Resolves active AI auth:
/// (1) Gemini API key (aistudio.google.com/apikey — fast, reliable, token-efficient),
/// (2) Google OAuth 2.0 Bearer with automatic token refresh.
fn resolve_ai_auth(state_arc: &Arc<Mutex<SharedState>>) -> Result<AiAuth, String> {
    let (mut oauth_sec, api_key) = {
        let st = state_arc.lock().unwrap();
        (st.google_oauth.clone(), st.data.config.gemini_api_key.clone())
    };
    if !api_key.trim().is_empty() {
        return Ok(AiAuth::ApiKey(api_key));
    }
    if !oauth_sec.access_token.trim().is_empty() || !oauth_sec.refresh_token.trim().is_empty() {
        let now = oauth::now_unix_secs();
        if oauth_sec.access_token.trim().is_empty() || now >= oauth_sec.expires_at_unix.saturating_sub(60) {
            oauth::refresh_google_token(&mut oauth_sec)?;
            let mut st = state_arc.lock().unwrap();
            st.google_oauth = oauth_sec.clone();
            st.save();
        }
        return Ok(AiAuth::Bearer(oauth_sec.access_token));
    }
    Err("Add a Gemini API key in Settings first".into())
}

#[derive(Deserialize)]
struct AiAskReq {
    message: String,
    #[serde(default)]
    history: Vec<AgentMessage>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    timestamp: Option<f64>,
    #[serde(default)]
    context_lines: Vec<String>,
}

#[derive(Deserialize)]
struct YtAddReq {
    url: String,
}

#[derive(Deserialize)]
struct CloudSyncReq {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    secret: Option<String>,
}

fn sync_cloud_profile_internal(state_arc: &Arc<Mutex<SharedState>>) -> Result<(), String> {
    let (url, secret) = {
        let st = state_arc.lock().unwrap();
        (
            st.data.config.cloud_sync_url.trim().to_string(),
            st.cloud_sync_secret.trim().to_string(),
        )
    };
    if url.is_empty() {
        return Err("Enter your Cloudflare Worker URL in Settings first".into());
    }
    let remote = cloud_sync::pull_profile(&url, &secret)?;
    let merged_to_push = {
        let mut st = state_arc.lock().unwrap();
        let mut local = st.to_clean_profile();
        if let Some(rem) = remote {
            cloud_sync::merge_profiles(&mut local, rem);
            st.apply_merged_profile(local.clone());
            let vf = st.vocab_file.clone();
            st.vocab.save(&vf);
            st.save();
        }
        st.to_clean_profile()
    };
    cloud_sync::push_profile(&url, &secret, &merged_to_push)?;
    Ok(())
}

#[derive(Deserialize)]
struct OAuthStartReq {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
}

fn tokenize_cues(
    cues: &[subs::SubtitleCue],
    morph_arc: &Arc<Mutex<Option<MorphAnalyzer>>>,
    jmdict_arc: &Arc<Mutex<Option<LocalJmDict>>>,
    vocab: &VocabStore,
) -> Vec<serde_json::Value> {
    let guard = morph_arc.lock().unwrap();
    let dict_guard = jmdict_arc.lock().unwrap();
    let dict_ref = dict_guard.as_ref();
    let km = KnowledgeModel::build(vocab);
    cues.iter()
        .map(|c| {
            let tokens: Vec<serde_json::Value> = if let Some(ref analyzer) = *guard {
                analyzer
                    .tokenize_line(&c.text, dict_ref)
                    .into_iter()
                    .map(|tk: MorphToken| {
                        let likely = tk.content
                            && !tk.ignore
                            && !vocab.known.contains(&tk.lemma)
                            && !vocab.ignored.contains(&tk.lemma)
                            && km.is_likely_known(&tk.lemma, dict_ref);
                        serde_json::json!({
                            "surface": tk.surface,
                            "lemma": tk.lemma,
                            "reading": tk.reading,
                            "pos": tk.pos,
                            "sub_pos": tk.sub_pos,
                            "ignore": tk.ignore,
                            "content": tk.content,
                            "likely": likely,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            serde_json::json!({
                "start": c.start,
                "end": c.end,
                "text": c.text,
                "tokens": tokens,
            })
        })
        .collect()
}

fn collect_calibration_words(
    state_arc: &Arc<Mutex<SharedState>>,
    limit: usize,
) -> Vec<CalibrationCandidate> {
    let (vocab, morph_arc, jmdict_arc, yt_subs_dir, media_root, yt_ids, lib_paths) = {
        let st = state_arc.lock().unwrap();
        let yids: Vec<String> = st.youtube.videos.iter().take(6).map(|v| v.id.clone()).collect();
        let lpaths: Vec<PathBuf> = st
            .library
            .iter()
            .flat_map(|a| {
                a.episodes.iter().filter_map(|e| {
                    e.subtitle_file
                        .as_ref()
                        .map(|sf| st.media_root.join(&a.id).join(sf))
                })
            })
            .take(6)
            .collect();
        (
            st.vocab.clone(),
            Arc::clone(&st.morph),
            Arc::clone(&st.jmdict),
            st.yt_subs_dir.clone(),
            st.media_root.clone(),
            yids,
            lpaths,
        )
    };
    let _ = media_root;
    let mut sub_paths: Vec<PathBuf> = yt_ids
        .iter()
        .filter_map(|id| youtube::find_sub_file(&yt_subs_dir, id))
        .collect();
    sub_paths.extend(lib_paths);

    let mut lemmas: Vec<String> = Vec::new();
    let mp_g = morph_arc.lock().unwrap();
    let jm_g = jmdict_arc.lock().unwrap();
    if let Some(ref mp) = *mp_g {
        for p in sub_paths.iter().take(8) {
            if let Ok(cues) = subs::load_subtitles(p) {
                for c in cues.iter().take(120) {
                    for tk in mp.tokenize_line(&c.text, jm_g.as_ref()) {
                        if tk.content && !tk.ignore {
                            lemmas.push(tk.lemma);
                        }
                    }
                }
            }
        }
    }
    let km = KnowledgeModel::build(&vocab);
    km.suggest_calibration(&vocab, jm_g.as_ref(), &lemmas, limit.clamp(1, 20))
}

fn handle_request(mut req: Request, state_arc: Arc<Mutex<SharedState>>) {
    let raw_url = req.url().to_string();
    let (path_only, query) = raw_url.split_once('?').unwrap_or((&raw_url, ""));
    let method = req.method().clone();

    // Static UI
    if method == Method::Get && (path_only == "/" || path_only == "/index.html") {
        let resp = Response::from_string(UI_HTML)
            .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
        let _ = req.respond(resp);
        return;
    }

    // Cover image
    if method == Method::Get && path_only.starts_with("/cover/") {
        let id = path_only.trim_start_matches("/cover/");
        let Some(anime_id) = safe_segment(id) else {
            respond_err(req, 400, "Invalid anime id");
            return;
        };
        let cover_path = {
            let st = state_arc.lock().unwrap();
            st.library
                .iter()
                .find(|a| a.id == anime_id)
                .and_then(|a| a.cover_file.as_ref())
                .map(|cf| st.media_root.join(&anime_id).join(cf))
        };
        if let Some(p) = cover_path {
            if let Ok(file) = File::open(&p) {
                let mime = mime_for_ext(&p);
                let resp = Response::from_file(file)
                    .with_header(Header::from_bytes("Content-Type", mime).unwrap())
                    .with_header(Header::from_bytes("Cache-Control", "max-age=300").unwrap());
                let _ = req.respond(resp);
                return;
            }
        }
        let _ = req.respond(Response::empty(StatusCode(404)));
        return;
    }

    // Audio streaming with HTTP 206 Range support
    if method == Method::Get && path_only.starts_with("/stream/") {
        let rest = path_only.trim_start_matches("/stream/");
        let Some((id_part, file_part)) = rest.split_once('/') else {
            respond_err(req, 400, "Invalid stream path");
            return;
        };
        let (Some(anime_id), Some(filename)) = (safe_segment(id_part), safe_segment(file_part))
        else {
            respond_err(req, 400, "Unsafe stream path");
            return;
        };
        let audio_path = {
            let st = state_arc.lock().unwrap();
            st.media_root.join(&anime_id).join(&filename)
        };
        handle_stream(req, &audio_path);
        return;
    }

    // POST /api/subtitles/{anime_id}/{episode_id}/generate (local whisper.cpp speech-synced SRT)
    if method == Method::Post
        && path_only.starts_with("/api/subtitles/")
        && path_only.ends_with("/generate")
    {
        let mid = path_only
            .trim_start_matches("/api/subtitles/")
            .trim_end_matches("/generate");
        let Some((aid_raw, eid_raw)) = mid.split_once('/') else {
            return respond_err(req, 400, "Invalid subtitle path");
        };
        let (Some(anime_id), Some(episode_id)) = (safe_segment(aid_raw), safe_segment(eid_raw))
        else {
            return respond_err(req, 400, "Unsafe subtitle path");
        };

        let (whisper_opt, audio_path_opt) = {
            let st = state_arc.lock().unwrap();
            let ap = st
                .library
                .iter()
                .find(|a| a.id == anime_id)
                .and_then(|a| a.episodes.iter().find(|e| e.id == episode_id))
                .and_then(|e| e.filename.as_ref())
                .map(|f| st.media_root.join(&anime_id).join(f));
            (st.find_whisper(), ap)
        };

        let Some((exe, model)) = whisper_opt else {
            return respond_err(
                req,
                400,
                "Local Whisper engine not found in tools/whisper/",
            );
        };
        let Some(audio_path) = audio_path_opt else {
            return respond_err(req, 404, "Audio file not found for episode");
        };
        let out_base = audio_path.with_extension("");
        let srt_path = audio_path.with_extension("srt");

        let mut cmd = Command::new(&exe);
        cmd.arg("-m")
            .arg(&model)
            .arg("-l")
            .arg("ja")
            .arg("-t")
            .arg("8")
            .arg("-ml")
            .arg("42")
            .arg("-osrt")
            .arg("-of")
            .arg(&out_base)
            .arg(&audio_path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        match cmd.output() {
            Ok(out) if out.status.success() && srt_path.exists() => {
                let library = {
                    let mut st = state_arc.lock().unwrap();
                    st.rescan();
                    st.library.clone()
                };
                match subs::load_subtitles(&srt_path) {
                    Ok(cues) => {
                        let (morph_arc, jmdict_arc, vocab_snap) = {
                            let st = state_arc.lock().unwrap();
                            (Arc::clone(&st.morph), Arc::clone(&st.jmdict), st.vocab.clone())
                        };
                        let rich_cues = tokenize_cues(&cues, &morph_arc, &jmdict_arc, &vocab_snap);
                        respond_json(
                            req,
                            200,
                            &serde_json::json!({ "cues": rich_cues, "library": library }),
                        );
                    }
                    Err(e) => respond_err(req, 500, &e),
                }
            }
            Ok(out) => {
                let err_txt = String::from_utf8_lossy(&out.stderr);
                let tail: String = err_txt.chars().rev().take(200).collect::<String>().chars().rev().collect();
                respond_err(req, 500, &format!("Whisper failed: {}", tail.trim()));
            }
            Err(e) => respond_err(req, 500, &format!("Could not run whisper-cli: {}", e)),
        }
        return;
    }

    // POST /api/synopsis/{anime_id}/{episode_id}/generate (optional 1-click Gemini AI narrative beats)
    if method == Method::Post
        && path_only.starts_with("/api/synopsis/")
        && path_only.ends_with("/generate")
    {
        let mid = path_only
            .trim_start_matches("/api/synopsis/")
            .trim_end_matches("/generate");
        let Some((aid_raw, eid_raw)) = mid.split_once('/') else {
            return respond_err(req, 400, "Invalid synopsis path");
        };
        let (Some(anime_id), Some(episode_id)) = (safe_segment(aid_raw), safe_segment(eid_raw))
        else {
            return respond_err(req, 400, "Unsafe synopsis path");
        };
        let (anime_title, ep_title, sub_path, syn_file) = {
            let st = state_arc.lock().unwrap();
            let anime = st.library.iter().find(|a| a.id == anime_id);
            let at = anime.map(|a| a.title.clone()).unwrap_or_else(|| anime_id.clone());
            let ep = anime.and_then(|a| a.episodes.iter().find(|e| e.id == episode_id));
            let et = ep.map(|e| e.title.clone()).unwrap_or_else(|| episode_id.clone());
            let sp = ep
                .and_then(|e| e.subtitle_file.as_ref())
                .map(|sf| st.media_root.join(&anime_id).join(sf));
            (at, et, sp, st.synopsis_file.clone())
        };
        let force = query.contains("force=1");
        let key = format!("{}/{}", anime_id, episode_id);
        if !force {
            let cached = {
                let st = state_arc.lock().unwrap();
                st.synopses
                    .episodes
                    .get(&key)
                    .filter(|e| e.ai_generated && !e.beats.is_empty())
                    .cloned()
            };
            if let Some(hit) = cached {
                respond_json(req, 200, &hit);
                return;
            }
        }
        let auth = match resolve_ai_auth(&state_arc) {
            Ok(a) => a,
            Err(e) => return respond_err(req, 400, &e),
        };
        let Some(p) = sub_path else {
            return respond_err(req, 400, "No subtitles synced for this episode yet");
        };
        let cues = match subs::load_subtitles(&p) {
            Ok(c) => c,
            Err(e) => return respond_err(req, 500, &e),
        };
        let (work_title, _) = synopsis::builtin_series_synopsis(&anime_id, &ep_title);
        let display_series = if anime_id == "example-japanese-stories" { work_title } else { anime_title };
        match synopsis::generate_ai_synopsis(&auth, &display_series, &ep_title, &cues) {
            Ok(gen) => {
                let key = format!("{}/{}", anime_id, episode_id);
                {
                    let mut st = state_arc.lock().unwrap();
                    st.synopses.episodes.insert(key, gen.clone());
                    st.synopses.save(&syn_file);
                }
                respond_json(req, 200, &gen);
            }
            Err(e) => respond_err(req, 400, &e),
        }
        return;
    }

    // GET /api/synopsis/{anime_id}/{episode_id} (ephemeral scene trace + episode & series synopsis!)
    if method == Method::Get && path_only.starts_with("/api/synopsis/") {
        let rest = path_only.trim_start_matches("/api/synopsis/");
        let Some((aid_raw, eid_raw)) = rest.split_once('/') else {
            return respond_err(req, 400, "Invalid synopsis path");
        };
        let (Some(anime_id), Some(episode_id)) = (safe_segment(aid_raw), safe_segment(eid_raw))
        else {
            return respond_err(req, 400, "Unsafe synopsis path");
        };
        let key = format!("{}/{}", anime_id, episode_id);
        let (cached_ep, cached_series, ep_title, sub_path, morph_arc, jmdict_arc) = {
            let st = state_arc.lock().unwrap();
            let anime = st.library.iter().find(|a| a.id == anime_id);
            let ep = anime.and_then(|a| a.episodes.iter().find(|e| e.id == episode_id));
            let et = ep.map(|e| e.title.clone()).unwrap_or_else(|| episode_id.clone());
            let sp = ep
                .and_then(|e| e.subtitle_file.as_ref())
                .map(|sf| st.media_root.join(&anime_id).join(sf));
            (
                st.synopses.episodes.get(&key).cloned(),
                st.synopses.series.get(&anime_id).cloned(),
                et,
                sp,
                Arc::clone(&st.morph),
                Arc::clone(&st.jmdict),
            )
        };

        let (builtin_title, builtin_series) = synopsis::builtin_series_synopsis(&anime_id, &ep_title);
        let builtin_ep = synopsis::builtin_episode_synopsis(&anime_id, &ep_title);

        let mut out: EpisodeSynopsis = cached_ep.unwrap_or_default();
        if out.series_title.is_empty() {
            out.series_title = builtin_title;
        }
        if out.series_summary.is_empty() {
            out.series_summary = cached_series.unwrap_or(builtin_series);
        }
        if out.episode_summary.is_empty() {
            out.episode_summary = builtin_ep;
        }
        // Zero-API offline fallback: synthesize clean ~22s scene concept traces from MeCab + JMdict!
        if out.beats.is_empty() {
            if let Some(p) = sub_path {
                if let Ok(cues) = subs::load_subtitles(&p) {
                    let mp_guard = morph_arc.lock().unwrap();
                    let jm_guard = jmdict_arc.lock().unwrap();
                    out.beats = synopsis::build_offline_beats(&cues, mp_guard.as_ref(), jm_guard.as_ref());
                }
            }
        }
        respond_json(req, 200, &out);
        return;
    }

    // GET /api/subtitles/{anime_id}/{episode_id} (pre-tokenized by MeCab/IPADIC + JMdict compound merge!)
    if method == Method::Get && path_only.starts_with("/api/subtitles/") {
        let rest = path_only.trim_start_matches("/api/subtitles/");
        let Some((aid_raw, eid_raw)) = rest.split_once('/') else {
            return respond_err(req, 400, "Invalid subtitle path");
        };
        let (Some(anime_id), Some(episode_id)) = (safe_segment(aid_raw), safe_segment(eid_raw))
        else {
            return respond_err(req, 400, "Unsafe subtitle path");
        };
        let (sub_path, morph_arc, jmdict_arc, vocab_snap) = {
            let st = state_arc.lock().unwrap();
            let sp = st
                .library
                .iter()
                .find(|a| a.id == anime_id)
                .and_then(|a| a.episodes.iter().find(|e| e.id == episode_id))
                .and_then(|e| e.subtitle_file.as_ref())
                .map(|sf| st.media_root.join(&anime_id).join(sf));
            (sp, Arc::clone(&st.morph), Arc::clone(&st.jmdict), st.vocab.clone())
        };
        let Some(p) = sub_path else {
            return respond_json(req, 200, &serde_json::json!({ "cues": [] }));
        };
        match subs::load_subtitles(&p) {
            Ok(cues) => {
                let rich_cues = tokenize_cues(&cues, &morph_arc, &jmdict_arc, &vocab_snap);
                respond_json(req, 200, &serde_json::json!({ "cues": rich_cues }));
            }
            Err(e) => respond_err(req, 500, &e),
        }
        return;
    }

    // GET /api/corpus/{anime_id}?episode_id=... (MeCab/IPADIC lemmatized Word Map & Series Frequency stats!)
    if method == Method::Get && path_only.starts_with("/api/corpus/") {
        let id_raw = path_only.trim_start_matches("/api/corpus/");
        let Some(anime_id) = safe_segment(id_raw) else {
            return respond_err(req, 400, "Invalid anime id");
        };
        let ep_filter: Option<String> = query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            if k == "episode_id" && !v.is_empty() {
                safe_segment(v)
            } else {
                None
            }
        });
        let (paths, morph_arc, jmdict_arc) = {
            let st = state_arc.lock().unwrap();
            let Some(anime) = st.library.iter().find(|a| a.id == anime_id) else {
                return respond_err(req, 404, "Anime not found");
            };
            let ps: Vec<PathBuf> = anime
                .episodes
                .iter()
                .filter(|e| ep_filter.as_ref().map(|ef| ef == &e.id).unwrap_or(true))
                .filter_map(|e| {
                    e.subtitle_file
                        .as_ref()
                        .map(|sf| st.media_root.join(&anime_id).join(sf))
                })
                .collect();
            (ps, Arc::clone(&st.morph), Arc::clone(&st.jmdict))
        };
        let mut lines: Vec<String> = Vec::new();
        for p in &paths {
            if let Ok(cues) = subs::load_subtitles(p) {
                for c in cues {
                    if !c.text.trim().is_empty() {
                        lines.push(c.text);
                    }
                }
            }
        }

        // Aggregate true MeCab/IPADIC lemma counts across the corpus in Rust (<30ms for 7,000+ lines!)
        let words: Vec<serde_json::Value> = {
            let guard = morph_arc.lock().unwrap();
            let dict_guard = jmdict_arc.lock().unwrap();
            let dict_ref = dict_guard.as_ref();
            if let Some(ref analyzer) = *guard {
                let mut agg: HashMap<String, (u32, bool, String, String)> = HashMap::new();
                for line in &lines {
                    let toks = analyzer.tokenize_line(line, dict_ref);
                    let mut char_pos: usize = 0;
                    for tok in toks {
                        let start_pos = char_pos;
                        char_pos += tok.surface.chars().count();
                        if tok.ignore {
                            continue;
                        }
                        let entry = agg.entry(tok.lemma.clone()).or_insert_with(|| {
                            let ctx: String = line
                                .chars()
                                .skip(start_pos)
                                .take(14)
                                .filter(|c| !c.is_whitespace())
                                .collect();
                            (0, tok.content, tok.pos.clone(), ctx)
                        });
                        entry.0 += 1;
                        if tok.content {
                            entry.1 = true;
                        }
                    }
                }
                let mut list: Vec<(String, u32, bool, String, String)> = agg
                    .into_iter()
                    .map(|(w, (c, cont, pos, ctx))| (w, c, cont, pos, ctx))
                    .collect();
                list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                list.into_iter()
                    .map(|(w, c, cont, pos, ctx)| {
                        serde_json::json!({
                            "word": w,
                            "count": c,
                            "content": cont,
                            "pos": pos,
                            "ctx": ctx,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };

        respond_json(
            req,
            200,
            &serde_json::json!({
                "anime_id": anime_id,
                "episode_id": ep_filter,
                "files": paths.len(),
                "lines": lines,
                "words": words,
            }),
        );
        return;
    }

    // GET /api/dict?q=<word>&lemma=<lemma>&pos=<pos>&ctx=<context> (instant local JMdict + JMnedict!)
    if method == Method::Get && path_only == "/api/dict" {
        let mut q_raw = String::new();
        let mut ctx_raw = String::new();
        let mut lemma_raw = String::new();
        let mut pos_raw = String::new();
        for kv in query.split('&') {
            if let Some((k, v)) = kv.split_once('=') {
                if let Ok(dec) = percent_decode_str(v).decode_utf8() {
                    match k {
                        "q" => q_raw = dec.into_owned(),
                        "ctx" => ctx_raw = dec.into_owned(),
                        "lemma" => lemma_raw = dec.into_owned(),
                        "pos" => pos_raw = dec.into_owned(),
                        _ => {}
                    }
                }
            }
        }
        let q = q_raw.trim().to_string();
        let ctx = ctx_raw.trim().to_string();
        let lemma = lemma_raw.trim().to_string();
        let pos = pos_raw.trim().to_string();
        if q.is_empty() && lemma.is_empty() {
            return respond_json(req, 200, &serde_json::json!({ "query": q, "entries": [] }));
        }

        let (jmdict_arc, jmnedict_arc) = {
            let mut st = state_arc.lock().unwrap();
            if let Some(ref mut sess) = st.active_session {
                let w = if !lemma.is_empty() { lemma.clone() } else { q.clone() };
                if !w.is_empty() && !sess.looked_up_words.contains(&w) {
                    sess.looked_up_words.push(w);
                    sess.words_looked_up = sess.looked_up_words.len() as u32;
                }
            }
            (Arc::clone(&st.jmdict), Arc::clone(&st.jmnedict))
        };
        let (entries, dict_words, name_words) = {
            let jm_guard = jmdict_arc.lock().unwrap();
            let nm_guard = jmnedict_arc.lock().unwrap();
            let nm_ref = nm_guard.as_ref();
            let nw = nm_ref.map(|n| n.name_count()).unwrap_or(0);
            if let Some(ref local) = *jm_guard {
                (local.lookup(&q, &ctx, &lemma, &pos, nm_ref), local.word_count(), nw)
            } else {
                (Vec::new(), 0, nw)
            }
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "query": q, "entries": entries, "local": true, "dict_words": dict_words, "name_words": name_words }),
        );
        return;
    }

    // PUT /api/vocab (tag word as known | learning | unknown | ignore)
    if method == Method::Put && path_only == "/api/vocab" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: VocabSetReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let vocab = {
            let mut st = state_arc.lock().unwrap();
            let vf = st.vocab_file.clone();
            let clean_w = parsed.word.trim().to_string();
            let was_known = st.vocab.known.contains(&clean_w);
            st.vocab.set_status(&clean_w, &parsed.status);
            if let Some(ref mut sess) = st.active_session {
                if parsed.status == "known" && !was_known && !clean_w.is_empty() {
                    if !sess.known_words.contains(&clean_w) {
                        sess.known_words.push(clean_w.clone());
                        sess.words_marked_known = sess.known_words.len() as u32;
                    }
                } else if parsed.status == "learning" && !clean_w.is_empty() {
                    if !sess.learning_words.contains(&clean_w) {
                        sess.learning_words.push(clean_w);
                    }
                }
            }
            st.vocab.save(&vf);
            st.save();
            st.vocab.clone()
        };
        respond_json(req, 200, &serde_json::json!({ "vocab": vocab }));
        return;
    }

    // POST /api/vocab/import (import known word list from txt/csv/json)
    if method == Method::Post && path_only == "/api/vocab/import" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: VocabImportReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let (count, vocab) = {
            let mut st = state_arc.lock().unwrap();
            let vf = st.vocab_file.clone();
            let c = st.vocab.import_bulk(&parsed.text, &parsed.status);
            st.vocab.save(&vf);
            st.save();
            (c, st.vocab.clone())
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "imported": count, "vocab": vocab }),
        );
        return;
    }

    // GET /api/vocab/export?format=json|txt
    if method == Method::Get && path_only == "/api/vocab/export" {
        let is_txt = query.contains("format=txt");
        let (content, mime, fname) = {
            let st = state_arc.lock().unwrap();
            if is_txt {
                (
                    st.vocab.export_txt(),
                    "text/plain; charset=utf-8",
                    "kiku-known-words.txt",
                )
            } else {
                (
                    serde_json::to_string_pretty(&st.vocab).unwrap_or_else(|_| "{}".into()),
                    "application/json; charset=utf-8",
                    "kiku-vocab.json",
                )
            }
        };
        let resp = Response::from_string(content)
            .with_header(Header::from_bytes("Content-Type", mime).unwrap())
            .with_header(
                Header::from_bytes(
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", fname),
                )
                .unwrap(),
            );
        let _ = req.respond(resp);
        return;
    }

    // POST /api/progress or PUT /api/progress (cache audio position + full-listen count)
    if (method == Method::Put || method == Method::Post) && path_only == "/api/progress" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: ProgressUpdateReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let key = format!("{}/{}", parsed.anime_id, parsed.episode_id);
        let entry = {
            let mut st = state_arc.lock().unwrap();
            let p = st
                .data
                .progress
                .entry(key.clone())
                .or_insert_with(EpisodeProgress::default);
            if parsed.duration_secs > 0.0 {
                p.duration_secs = parsed.duration_secs;
            }
            let near_end = p.duration_secs > 15.0
                && (parsed.position_secs >= (p.duration_secs - 10.0)
                    || (parsed.position_secs / p.duration_secs) >= 0.95);
            if parsed.completed {
                p.completions = p.completions.saturating_add(1);
                p.position_secs = 0.0;
            } else if near_end {
                p.position_secs = 0.0;
            } else {
                p.position_secs = parsed.position_secs.max(0.0);
            }
            p.last_played_at = timeutil::now_utc_rfc3339();
            let cloned = p.clone();
            st.save();
            cloned
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "key": key, "progress": entry }),
        );
        return;
    }

    // GET /api/state
    if method == Method::Get && path_only == "/api/state" {
        let payload = {
            let st = state_arc.lock().unwrap();
            serde_json::json!({
                "config": st.public_config(),
                "library": st.library,
                "playlists": st.data.playlists,
                "history": st.data.history,
                "active_session": st.active_session,
                "stats": st.compute_stats(),
                "media_root": st.media_root.to_string_lossy(),
                "whisper_ready": st.find_whisper().is_some(),
                "vocab": st.vocab,
                "progress": st.data.progress,
                "youtube": st.youtube.videos,
            })
        };
        respond_json(req, 200, &payload);
        return;
    }

    // POST /api/rescan
    if method == Method::Post && path_only == "/api/rescan" {
        let library = {
            let mut st = state_arc.lock().unwrap();
            st.rescan();
            st.library.clone()
        };
        respond_json(req, 200, &serde_json::json!({ "library": library }));
        return;
    }

    // POST /api/open-media-root
    if method == Method::Post && path_only == "/api/open-media-root" {
        let root = { state_arc.lock().unwrap().media_root.clone() };
        let _ = fs::create_dir_all(&root);
        let _ = Command::new("explorer.exe").arg(&root).spawn();
        respond_json(
            req,
            200,
            &serde_json::json!({ "opened": root.to_string_lossy() }),
        );
        return;
    }

    // POST /api/anime (create new anime folder)
    if method == Method::Post && path_only == "/api/anime" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: CreateAnimeReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        if parsed.title.trim().is_empty() {
            return respond_err(req, 400, "Anime title is required");
        }
        let id = slugify(&parsed.title);
        let library = {
            let mut st = state_arc.lock().unwrap();
            let folder = st.media_root.join(&id);
            let _ = fs::create_dir_all(&folder);
            if !st.data.custom_anime.iter().any(|a| a.id == id) {
                st.data.custom_anime.push(CustomAnimeMeta {
                    id: id.clone(),
                    title: parsed.title.trim().to_string(),
                    jp_title: parsed.jp_title.trim().to_string(),
                    subtitle: if parsed.subtitle.trim().is_empty() {
                        "Local Anime Folder · Drop MP3s here".into()
                    } else {
                        parsed.subtitle.trim().to_string()
                    },
                    accent: if parsed.accent.trim().is_empty() {
                        "#EE4766".into()
                    } else {
                        parsed.accent.trim().to_string()
                    },
                });
                st.save();
            }
            st.rescan();
            st.library.clone()
        };
        respond_json(req, 200, &serde_json::json!({ "id": id, "library": library }));
        return;
    }

    // POST /api/anime/{id}/open-folder
    if method == Method::Post
        && path_only.starts_with("/api/anime/")
        && path_only.ends_with("/open-folder")
    {
        let id = path_only
            .trim_start_matches("/api/anime/")
            .trim_end_matches("/open-folder");
        let Some(anime_id) = safe_segment(id) else {
            return respond_err(req, 400, "Invalid anime id");
        };
        let folder = { state_arc.lock().unwrap().media_root.join(&anime_id) };
        let _ = fs::create_dir_all(&folder);
        let _ = Command::new("explorer.exe").arg(&folder).spawn();
        respond_json(
            req,
            200,
            &serde_json::json!({ "opened": folder.to_string_lossy() }),
        );
        return;
    }

    // POST /api/upload/bulk-item (bulk MP3 upload with auto-route + auto-sort)
    if method == Method::Post && path_only == "/api/upload/bulk-item" {
        let header_val = |name: &'static str| -> String {
            req.headers()
                .iter()
                .find(|h| h.field.equiv(name))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default()
        };
        let raw_name = header_val("X-Filename");
        let Some(filename) = safe_segment(&raw_name) else {
            return respond_err(req, 400, "Missing or invalid X-Filename header");
        };
        let rel_path = percent_decode_str(&header_val("X-RelPath"))
            .decode_utf8()
            .map(|c| c.into_owned())
            .unwrap_or_default();
        let target_hdr = header_val("X-Target-Anime");
        let skip_rescan = header_val("X-Skip-Rescan") == "1";

        let mut bytes = Vec::new();
        if let Err(e) = req.as_reader().read_to_end(&mut bytes) {
            return respond_err(req, 400, &format!("Upload read failed: {}", e));
        }

        let (resolved_anime_id, anime_title, library_opt) = {
            let mut st = state_arc.lock().unwrap();
            let anime_id = if !target_hdr.is_empty() && target_hdr != "auto" {
                safe_segment(&target_hdr).unwrap_or_else(|| "my-hero-academia".into())
            } else if let Some(detected) =
                detect_anime_for_file(&filename, &rel_path, &st.library)
            {
                detected
            } else if let Some(first_dir) = rel_path
                .split(['/', '\\'])
                .find(|s| !s.trim().is_empty() && *s != filename)
            {
                // Auto-create anime folder named after the dropped parent folder
                let slug = slugify(first_dir);
                if !st.library.iter().any(|a| a.id == slug)
                    && !st.data.custom_anime.iter().any(|a| a.id == slug)
                {
                    st.data.custom_anime.push(CustomAnimeMeta {
                        id: slug.clone(),
                        title: first_dir.trim().to_string(),
                        jp_title: String::new(),
                        subtitle: "Imported Folder".into(),
                        accent: "#B49DE0".into(),
                    });
                    st.save();
                }
                slug
            } else {
                // Default fallback to first non-example anime or custom unsorted folder
                "my-hero-academia".into()
            };

            let dir = st.media_root.join(&anime_id);
            let _ = fs::create_dir_all(&dir);
            let target = dir.join(&filename);
            if let Err(e) = fs::write(&target, &bytes) {
                return respond_err(req, 500, &format!("Failed writing file: {}", e));
            }
            if !skip_rescan {
                st.rescan();
            }
            let title = st
                .library
                .iter()
                .find(|a| a.id == anime_id)
                .map(|a| a.title.clone())
                .unwrap_or_else(|| anime_id.clone());
            let lib = if skip_rescan {
                None
            } else {
                Some(st.library.clone())
            };
            (anime_id, title, lib)
        };

        respond_json(
            req,
            200,
            &serde_json::json!({
                "saved": filename,
                "anime_id": resolved_anime_id,
                "anime_title": anime_title,
                "bytes": bytes.len(),
                "library": library_opt,
            }),
        );
        return;
    }

    // POST /api/anime/{id}/upload (upload MP3 or cover file straight to disk)
    if method == Method::Post
        && path_only.starts_with("/api/anime/")
        && path_only.ends_with("/upload")
    {
        let id = path_only
            .trim_start_matches("/api/anime/")
            .trim_end_matches("/upload");
        let Some(anime_id) = safe_segment(id) else {
            return respond_err(req, 400, "Invalid anime id");
        };
        let raw_name = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("X-Filename"))
            .map(|h| h.value.as_str().to_string())
            .unwrap_or_default();
        let Some(filename) = safe_segment(&raw_name) else {
            return respond_err(req, 400, "Missing or invalid X-Filename header");
        };
        let mut bytes = Vec::new();
        if let Err(e) = req.as_reader().read_to_end(&mut bytes) {
            return respond_err(req, 400, &format!("Upload read failed: {}", e));
        }
        let target = {
            let st = state_arc.lock().unwrap();
            let dir = st.media_root.join(&anime_id);
            let _ = fs::create_dir_all(&dir);
            dir.join(&filename)
        };
        if let Err(e) = fs::write(&target, &bytes) {
            return respond_err(req, 500, &format!("Failed writing file: {}", e));
        }
        let library = {
            let mut st = state_arc.lock().unwrap();
            st.rescan();
            st.library.clone()
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "saved": filename, "bytes": bytes.len(), "library": library }),
        );
        return;
    }

    // POST /api/playlists (create)
    if method == Method::Post && path_only == "/api/playlists" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: PlaylistReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        if parsed.name.trim().is_empty() {
            return respond_err(req, 400, "Playlist name is required");
        }
        let pl = Playlist {
            id: format!("pl-{}", timeutil::now_unix_millis()),
            name: parsed.name.trim().to_string(),
            description: parsed.description.trim().to_string(),
            items: parsed.items,
        };
        let playlists = {
            let mut st = state_arc.lock().unwrap();
            st.data.playlists.push(pl.clone());
            st.save();
            st.data.playlists.clone()
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "playlist": pl, "playlists": playlists }),
        );
        return;
    }

    // PUT /api/playlists/{id}
    if method == Method::Put && path_only.starts_with("/api/playlists/") {
        let id = path_only.trim_start_matches("/api/playlists/");
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: PlaylistReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let playlists = {
            let mut st = state_arc.lock().unwrap();
            let Some(pl) = st.data.playlists.iter_mut().find(|p| p.id == id) else {
                return respond_err(req, 404, "Playlist not found");
            };
            if !parsed.name.trim().is_empty() {
                pl.name = parsed.name.trim().to_string();
            }
            pl.description = parsed.description.trim().to_string();
            pl.items = parsed.items;
            st.save();
            st.data.playlists.clone()
        };
        respond_json(req, 200, &serde_json::json!({ "playlists": playlists }));
        return;
    }

    // DELETE /api/playlists/{id}
    if method == Method::Delete && path_only.starts_with("/api/playlists/") {
        let id = path_only.trim_start_matches("/api/playlists/");
        let playlists = {
            let mut st = state_arc.lock().unwrap();
            st.data.playlists.retain(|p| p.id != id);
            st.save();
            st.data.playlists.clone()
        };
        respond_json(req, 200, &serde_json::json!({ "playlists": playlists }));
        return;
    }

    // POST /api/session/start
    if method == Method::Post && path_only == "/api/session/start" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: SessionStartReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };

        // Close any previous active session cleanly first
        let _ = stop_active_session_internal(&state_arc, None);

        let (anime_title, episode_title, playlist_name, cfg) = {
            let st = state_arc.lock().unwrap();
            if parsed.anime_id == "yt" {
                let yt_vid = st.youtube.videos.iter().find(|v| v.id == parsed.episode_id);
                let feed_vid = st.yt_feed_cache.iter().find(|v| v.id == parsed.episode_id);
                let ch = yt_vid
                    .map(|v| v.channel.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| feed_vid.map(|v| v.channel.as_str()).filter(|s| !s.is_empty()))
                    .unwrap_or("YouTube");
                let title = yt_vid
                    .map(|v| v.title.clone())
                    .or_else(|| feed_vid.map(|v| v.title.clone()))
                    .unwrap_or_else(|| parsed.episode_id.clone());
                (
                    format!("YT · {}", ch),
                    title,
                    None,
                    st.data.config.clone(),
                )
            } else {
                let anime = st.library.iter().find(|a| a.id == parsed.anime_id);
                let anime_title = anime
                    .map(|a| a.title.clone())
                    .unwrap_or_else(|| parsed.anime_id.clone());
                let episode_title = anime
                    .and_then(|a| a.episodes.iter().find(|e| e.id == parsed.episode_id))
                    .map(|e| format!("Ep {:02}: {}", e.number, e.title))
                    .unwrap_or_else(|| parsed.episode_id.clone());
                let playlist_name = parsed.playlist_id.as_ref().and_then(|pid| {
                    st.data
                        .playlists
                        .iter()
                        .find(|p| &p.id == pid)
                        .map(|p| p.name.clone())
                });
                (anime_title, episode_title, playlist_name, st.data.config.clone())
            }
        };

        let started_at = timeutil::now_utc_rfc3339();
        let toggl_wid = cfg.toggl_workspace_id;

        // Airgapped: NEVER call Toggl API on play/pause! Sessions accumulate locally and
        // batch-commit on the 15-minute cycle or when closing the app.
        let session = ActiveSession {
            id: format!("ses-{}", timeutil::now_unix_millis()),
            anime_id: parsed.anime_id,
            anime_title,
            episode_id: parsed.episode_id,
            episode_title,
            playlist_id: parsed.playlist_id,
            playlist_name,
            started_at,
            toggl_entry_id: None,
            toggl_workspace_id: toggl_wid,
            toggl_live: false,
            toggl_error: None,
            words_looked_up: 0,
            words_marked_known: 0,
            looked_up_words: Vec::new(),
            learning_words: Vec::new(),
            known_words: Vec::new(),
        };

        let (history, stats) = {
            let mut st = state_arc.lock().unwrap();
            st.active_session = Some(session.clone());
            (st.data.history.clone(), st.compute_stats())
        };

        respond_json(
            req,
            200,
            &serde_json::json!({
                "active_session": session,
                "history": history,
                "stats": stats,
            }),
        );
        return;
    }

    // POST /api/session/stop
    if method == Method::Post && path_only == "/api/session/stop" {
        let body = read_body_string(&mut req).unwrap_or_default();
        let parsed: SessionStopReq = serde_json::from_str(&body).unwrap_or(SessionStopReq {
            elapsed_secs: None,
        });
        let logged = stop_active_session_internal(&state_arc, parsed.elapsed_secs);
        let (history, stats) = {
            let st = state_arc.lock().unwrap();
            (st.data.history.clone(), st.compute_stats())
        };
        respond_json(
            req,
            200,
            &serde_json::json!({
                "logged": logged,
                "history": history,
                "stats": stats,
            }),
        );
        return;
    }

    // POST /api/history/{id}/sync-toggl
    if method == Method::Post
        && path_only.starts_with("/api/history/")
        && path_only.ends_with("/sync-toggl")
    {
        let id = path_only
            .trim_start_matches("/api/history/")
            .trim_end_matches("/sync-toggl");
        let (entry_opt, cfg) = {
            let st = state_arc.lock().unwrap();
            let e = st.data.history.iter().find(|h| h.id == id).cloned();
            (e, st.data.config.clone())
        };
        let Some(entry) = entry_opt else {
            return respond_err(req, 404, "History entry not found");
        };
        if cfg.toggl_api_token.trim().is_empty() {
            return respond_err(req, 400, "Add your Toggl API token in Settings first.");
        }
        let Some(wid) = cfg.toggl_workspace_id else {
            return respond_err(req, 400, "Select a Toggl workspace in Settings first.");
        };

        let desc = build_toggl_description(&cfg, &entry.anime_title, &entry.episode_title);
        let tags = build_toggl_tags(&cfg, &entry.anime_title);
        match toggl::create_completed_entry(
            &cfg.toggl_api_token,
            wid,
            cfg.toggl_project_id,
            &desc,
            &tags,
            &entry.started_at,
            &entry.ended_at,
            entry.duration_secs,
        ) {
            Ok(eid) => {
                let history = {
                    let mut st = state_arc.lock().unwrap();
                    if let Some(h) = st.data.history.iter_mut().find(|h| h.id == id) {
                        h.toggl_entry_id = Some(eid);
                        h.toggl_synced = true;
                        h.toggl_error = None;
                    }
                    st.save();
                    st.data.history.clone()
                };
                respond_json(
                    req,
                    200,
                    &serde_json::json!({ "synced": true, "toggl_entry_id": eid, "history": history }),
                );
            }
            Err(e) => {
                {
                    let mut st = state_arc.lock().unwrap();
                    if let Some(h) = st.data.history.iter_mut().find(|h| h.id == id) {
                        h.toggl_error = Some(e.clone());
                    }
                    st.save();
                }
                respond_err(req, 502, &e);
            }
        }
        return;
    }

    // DELETE /api/history/{id}
    if method == Method::Delete && path_only.starts_with("/api/history/") {
        let id = path_only.trim_start_matches("/api/history/");
        let (history, stats) = {
            let mut st = state_arc.lock().unwrap();
            st.data.history.retain(|h| h.id != id);
            st.save();
            (st.data.history.clone(), st.compute_stats())
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "history": history, "stats": stats }),
        );
        return;
    }

    // PUT /api/config
    if method == Method::Put && path_only == "/api/config" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let upd: ConfigUpdateReq = match serde_json::from_str(&body) {
            Ok(u) => u,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let pub_cfg = {
            let mut st = state_arc.lock().unwrap();
            let c = &mut st.data.config;
            if upd.clear_toggl_token {
                c.toggl_api_token.clear();
            } else if let Some(tok) = upd.toggl_api_token {
                if !tok.trim().is_empty() {
                    c.toggl_api_token = tok.trim().to_string();
                }
            }
            if let Some(gkey) = upd.gemini_api_key {
                if !gkey.trim().is_empty() {
                    c.gemini_api_key = gkey.trim().to_string();
                }
            }
            if let Some(wid) = upd.toggl_workspace_id {
                c.toggl_workspace_id = Some(wid);
            }
            if let Some(wn) = upd.toggl_workspace_name {
                c.toggl_workspace_name = wn;
            }
            if upd.clear_toggl_project {
                c.toggl_project_id = None;
                c.toggl_project_name.clear();
            } else if let Some(pid) = upd.toggl_project_id {
                c.toggl_project_id = Some(pid);
            }
            if let Some(pn) = upd.toggl_project_name {
                c.toggl_project_name = pn;
            }
            if let Some(tags) = upd.toggl_tags {
                c.toggl_tags = tags;
            }
            if let Some(ta) = upd.toggl_tag_anime {
                c.toggl_tag_anime = ta;
            }
            if let Some(auto) = upd.toggl_auto_sync {
                c.toggl_auto_sync = auto;
            }
            if let Some(min_s) = upd.toggl_min_seconds {
                c.toggl_min_seconds = min_s.min(600);
            }
            if let Some(tpl) = upd.description_template {
                c.description_template = tpl;
            }
            if let Some(goal) = upd.daily_goal_minutes {
                c.daily_goal_minutes = goal.clamp(5, 720);
            }
            if let Some(curl) = upd.cloud_sync_url {
                c.cloud_sync_url = curl.trim().trim_end_matches('/').to_string();
            }
            if let Some(csec) = upd.cloud_sync_secret {
                if !csec.trim().is_empty() {
                    st.cloud_sync_secret = csec.trim().to_string();
                }
            }
            if let Some(tabs) = upd.yt_tabs {
                st.data.config.yt_tabs = tabs;
            }
            st.save();
            st.public_config()
        };
        respond_json(req, 200, &serde_json::json!({ "config": pub_cfg }));
        return;
    }

    // GET /api/yt/feed (Curated ImmersionTube feed ranked by T+1 comprehensibility against user's VocabStore)
    if method == Method::Get && path_only == "/api/yt/feed" {
        let force_refresh = query.contains("refresh=1");
        let now = oauth::now_unix_secs();
        let cached_opt = {
            let st = state_arc.lock().unwrap();
            if !force_refresh
                && !st.yt_feed_cache.is_empty()
                && now.saturating_sub(st.yt_feed_fetched_unix) < 600
            {
                Some(st.yt_feed_cache.clone())
            } else {
                None
            }
        };

        let mut items = match cached_opt {
            Some(c) => c,
            None => {
                let fresh = youtube::fetch_raw_immersion_feed();
                let mut st = state_arc.lock().unwrap();
                st.yt_feed_cache = fresh.clone();
                st.yt_feed_fetched_unix = now;
                fresh
            }
        };

        // Always re-score live against the user's current VocabStore & MeCab/IPADIC!
        let (morph_arc, jmdict_arc, vocab_snap) = {
            let st = state_arc.lock().unwrap();
            (Arc::clone(&st.morph), Arc::clone(&st.jmdict), st.vocab.clone())
        };
        {
            let mp_g = morph_arc.lock().unwrap();
            let jm_g = jmdict_arc.lock().unwrap();
            youtube::score_immersion_feed(&mut items, mp_g.as_ref(), jm_g.as_ref(), &vocab_snap);
        }

        respond_json(req, 200, &serde_json::json!({ "items": items }));
        return;
    }

    // POST /api/yt/search (Japanese YouTube search + T+1 scoring + AI category suggestion for messy queries)
    if method == Method::Post && path_only == "/api/yt/search" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let v: serde_json::Value = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let q = v["query"].as_str().unwrap_or("").trim().to_string();
        if q.is_empty() {
            return respond_err(req, 400, "Enter a search term or YouTube URL");
        }
        let (app_root, morph_arc, jmdict_arc, vocab_snap) = {
            let st = state_arc.lock().unwrap();
            (
                st.app_root.clone(),
                Arc::clone(&st.morph),
                Arc::clone(&st.jmdict),
                st.vocab.clone(),
            )
        };
        let mut items = match youtube::search_youtube_jp(&app_root, &q, 16) {
            Ok(list) => list,
            Err(e) => return respond_err(req, 400, &e),
        };
        {
            let mp_g = morph_arc.lock().unwrap();
            let jm_g = jmdict_arc.lock().unwrap();
            youtube::score_immersion_feed(&mut items, mp_g.as_ref(), jm_g.as_ref(), &vocab_snap);
        }
        // If the query is messy/long, ask the agent for a clean broad category name; otherwise title-case cleanly
        let word_count = q.split_whitespace().count();
        let suggested = if word_count > 2 || q.chars().count() > 16 {
            resolve_ai_auth(&state_arc)
                .ok()
                .and_then(|auth| agent::suggest_search_category(&auth, &q))
                .unwrap_or_else(|| q.split_whitespace().next().unwrap_or(&q).to_string())
        } else {
            let mut chs = q.chars();
            match chs.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chs.as_str(),
                None => q.clone(),
            }
        };
        respond_json(
            req,
            200,
            &serde_json::json!({
                "query": q,
                "suggested_category": suggested,
                "items": items,
            }),
        );
        return;
    }

    // POST /api/yt/add (extract YouTube ID, fetch title + Japanese subtitles via yt-dlp, return tokenized cues)
    if method == Method::Post && path_only == "/api/yt/add" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: YtAddReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let Some(video_id) = youtube::extract_video_id(&parsed.url) else {
            return respond_err(req, 400, "Paste a valid YouTube URL or 11-char video ID");
        };
        let (app_root, yt_subs_dir, yt_file, morph_arc, jmdict_arc) = {
            let st = state_arc.lock().unwrap();
            (
                st.app_root.clone(),
                st.yt_subs_dir.clone(),
                st.youtube_file.clone(),
                Arc::clone(&st.morph),
                Arc::clone(&st.jmdict),
            )
        };
        let video = match youtube::fetch_video_and_subs(&app_root, &yt_subs_dir, &video_id) {
            Ok(v) => v,
            Err(e) => return respond_err(req, 500, &e),
        };
        let videos = {
            let mut st = state_arc.lock().unwrap();
            st.youtube.videos.retain(|v| v.id != video.id);
            st.youtube.videos.insert(0, video.clone());
            st.youtube.save(&yt_file);
            st.youtube.videos.clone()
        };
        let rich_cues = if let Some(sub_p) = youtube::find_sub_file(&yt_subs_dir, &video_id) {
            let vocab_snap = { state_arc.lock().unwrap().vocab.clone() };
            match subs::load_subtitles(&sub_p) {
                Ok(cues) => tokenize_cues(&cues, &morph_arc, &jmdict_arc, &vocab_snap),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "video": video, "videos": videos, "cues": rich_cues }),
        );
        return;
    }

    // POST /api/yt/{id}/synopsis/generate (AI moment-by-moment context trace + video/channel synopsis)
    if method == Method::Post
        && path_only.starts_with("/api/yt/")
        && path_only.ends_with("/synopsis/generate")
    {
        let id_raw = path_only
            .trim_start_matches("/api/yt/")
            .trim_end_matches("/synopsis/generate");
        let Some(video_id) = youtube::extract_video_id(id_raw) else {
            return respond_err(req, 400, "Invalid YouTube video ID");
        };
        let force = query.contains("force=1");
        let key = format!("yt/{}", video_id);
        if !force {
            let cached = {
                let st = state_arc.lock().unwrap();
                st.synopses
                    .episodes
                    .get(&key)
                    .filter(|e| e.ai_generated && !e.beats.is_empty())
                    .cloned()
            };
            if let Some(hit) = cached {
                respond_json(req, 200, &hit);
                return;
            }
        }
        let auth = match resolve_ai_auth(&state_arc) {
            Ok(a) => a,
            Err(e) => return respond_err(req, 400, &e),
        };
        let (yt_subs_dir, syn_file, title, channel) = {
            let st = state_arc.lock().unwrap();
            let yt_v = st.youtube.videos.iter().find(|v| v.id == video_id);
            let fd_v = st.yt_feed_cache.iter().find(|v| v.id == video_id);
            let t = yt_v
                .map(|v| v.title.clone())
                .or_else(|| fd_v.map(|v| v.title.clone()))
                .unwrap_or_else(|| video_id.clone());
            let ch = yt_v
                .map(|v| v.channel.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| fd_v.map(|v| v.channel.clone()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "YouTube".to_string());
            (st.yt_subs_dir.clone(), st.synopsis_file.clone(), t, ch)
        };
        let Some(sub_p) = youtube::find_sub_file(&yt_subs_dir, &video_id) else {
            return respond_err(req, 400, "No Japanese subtitles cached for this video yet");
        };
        let cues = match subs::load_subtitles(&sub_p) {
            Ok(c) => c,
            Err(e) => return respond_err(req, 500, &e),
        };
        match agent::generate_yt_multimodal_synopsis(&auth, &video_id, &channel, &title, &cues) {
            Ok(gen) => {
                let key = format!("yt/{}", video_id);
                {
                    let mut st = state_arc.lock().unwrap();
                    st.synopses.episodes.insert(key, gen.clone());
                    st.synopses.save(&syn_file);
                }
                respond_json(req, 200, &gen);
            }
            Err(e) => respond_err(req, 400, &e),
        }
        return;
    }

    // GET /api/yt/{id}/synopsis (ephemeral scene trace + video/channel synopsis for YT videos)
    if method == Method::Get
        && path_only.starts_with("/api/yt/")
        && path_only.ends_with("/synopsis")
    {
        let id_raw = path_only
            .trim_start_matches("/api/yt/")
            .trim_end_matches("/synopsis");
        let Some(video_id) = youtube::extract_video_id(id_raw) else {
            return respond_err(req, 400, "Invalid YouTube video ID");
        };
        let key = format!("yt/{}", video_id);
        let (cached_ep, yt_subs_dir, title, channel, morph_arc, jmdict_arc) = {
            let st = state_arc.lock().unwrap();
            let yt_v = st.youtube.videos.iter().find(|v| v.id == video_id);
            let fd_v = st.yt_feed_cache.iter().find(|v| v.id == video_id);
            let t = yt_v
                .map(|v| v.title.clone())
                .or_else(|| fd_v.map(|v| v.title.clone()))
                .unwrap_or_else(|| video_id.clone());
            let ch = yt_v
                .map(|v| v.channel.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| fd_v.map(|v| v.channel.clone()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "YouTube".to_string());
            (
                st.synopses.episodes.get(&key).cloned(),
                st.yt_subs_dir.clone(),
                t,
                ch,
                Arc::clone(&st.morph),
                Arc::clone(&st.jmdict),
            )
        };
        let mut out: EpisodeSynopsis = cached_ep.unwrap_or_default();
        if out.series_title.is_empty() {
            out.series_title = channel.clone();
        }
        if out.beats.is_empty() {
            if let Some(p) = youtube::find_sub_file(&yt_subs_dir, &video_id) {
                if let Ok(cues) = subs::load_subtitles(&p) {
                    let mp_g = morph_arc.lock().unwrap();
                    let jm_g = jmdict_arc.lock().unwrap();
                    out.beats = synopsis::build_offline_beats(&cues, mp_g.as_ref(), jm_g.as_ref());
                }
            }
        }
        respond_json(req, 200, &out);
        return;
    }

    // GET /api/yt/{id}/subtitles (MeCab/IPADIC tokenized Japanese captions for a YouTube video)
    if method == Method::Get
        && path_only.starts_with("/api/yt/")
        && path_only.ends_with("/subtitles")
    {
        let id_raw = path_only
            .trim_start_matches("/api/yt/")
            .trim_end_matches("/subtitles");
        let Some(video_id) = youtube::extract_video_id(id_raw) else {
            return respond_err(req, 400, "Invalid YouTube video ID");
        };
        let (app_root, yt_subs_dir, morph_arc, jmdict_arc) = {
            let st = state_arc.lock().unwrap();
            (
                st.app_root.clone(),
                st.yt_subs_dir.clone(),
                Arc::clone(&st.morph),
                Arc::clone(&st.jmdict),
            )
        };
        let sub_path = youtube::find_sub_file(&yt_subs_dir, &video_id).or_else(|| {
            let _ = youtube::fetch_video_and_subs(&app_root, &yt_subs_dir, &video_id);
            youtube::find_sub_file(&yt_subs_dir, &video_id)
        });
        let rich_cues = if let Some(p) = sub_path {
            let vocab_snap = { state_arc.lock().unwrap().vocab.clone() };
            match subs::load_subtitles(&p) {
                Ok(cues) => tokenize_cues(&cues, &morph_arc, &jmdict_arc, &vocab_snap),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        respond_json(req, 200, &serde_json::json!({ "cues": rich_cues }));
        return;
    }

    // DELETE /api/yt/{id}
    if method == Method::Delete && path_only.starts_with("/api/yt/") {
        let id_raw = path_only.trim_start_matches("/api/yt/");
        let Some(video_id) = safe_segment(id_raw) else {
            return respond_err(req, 400, "Invalid YouTube ID");
        };
        let videos = {
            let mut st = state_arc.lock().unwrap();
            let yf = st.youtube_file.clone();
            st.youtube.videos.retain(|v| v.id != video_id);
            st.youtube.save(&yf);
            st.youtube.videos.clone()
        };
        respond_json(req, 200, &serde_json::json!({ "videos": videos }));
        return;
    }

    // GET /api/knowledge/calibrate?exclude=w1,w2 (Probabilistic Knowledge Model — likely-known untagged words)
    if method == Method::Get && path_only == "/api/knowledge/calibrate" {
        let mut excluded: std::collections::HashSet<String> = std::collections::HashSet::new();
        for kv in query.split('&') {
            if let Some(("exclude", val)) = kv.split_once('=') {
                if let Ok(dec) = percent_decode_str(val).decode_utf8() {
                    for w in dec.split(',') {
                        let t = w.trim();
                        if !t.is_empty() {
                            excluded.insert(t.to_string());
                        }
                    }
                }
            }
        }
        let mut items = collect_calibration_words(&state_arc, 40);
        if !excluded.is_empty() {
            items.retain(|c| !excluded.contains(&c.word));
        }
        items.truncate(12);
        respond_json(req, 200, &serde_json::json!({ "candidates": items }));
        return;
    }

    // POST /api/ai/ask (Kiku Immersion Agent + MCP-style Tool Router!)
    if method == Method::Post && path_only == "/api/ai/ask" {
        let body = match read_body_string(&mut req) {
            Ok(b) => b,
            Err(e) => return respond_err(req, 400, &e),
        };
        let parsed: AiAskReq = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => return respond_err(req, 400, &format!("Invalid JSON: {}", e)),
        };
        let auth = match resolve_ai_auth(&state_arc) {
            Ok(a) => a,
            Err(e) => return respond_err(req, 400, &e),
        };

        // Compact, token-efficient user immersion context snapshot
        let sys_ctx = {
            let st = state_arc.lock().unwrap();
            let stats = st.compute_stats();
            let learning: Vec<&str> = st.vocab.learning.iter().take(30).map(|s| s.as_str()).collect();
            let recent: Vec<String> = st
                .data
                .history
                .iter()
                .take(8)
                .map(|h| format!("{} — {} ({}m)", h.anime_title, h.episode_title, (h.duration_secs / 60).max(1)))
                .collect();
            let mut ctx = format!(
                "You are Kiku, a concise, insightful Japanese immersion AI companion.\n\
                User profile: {} known words, {} learning words (sample: {}), {}m immersed today ({}d streak).\n\
                Recent immersion: {}.\n\
                You have tools to update the user's vocabulary (mark_vocab) and surface likely-known calibration words (suggest_calibration_words). Keep answers crisp, natural, and directly helpful.",
                st.vocab.known.len(),
                st.vocab.learning.len(),
                learning.join("、"),
                stats.today_secs / 60,
                stats.streak_days,
                if recent.is_empty() { "none yet".to_string() } else { recent.join(" | ") },
            );
            if let Some(ref t) = parsed.title {
                ctx.push_str(&format!("\nCurrently watching/listening: \"{}\"", t));
                if let Some(ref ch) = parsed.channel {
                    ctx.push_str(&format!(" ({})", ch));
                }
                if let Some(ts) = parsed.timestamp {
                    ctx.push_str(&format!(" at {:02}:{:02}", (ts as u64) / 60, (ts as u64) % 60));
                }
            }
            if !parsed.context_lines.is_empty() {
                ctx.push_str("\nNearby Japanese lines:\n");
                for l in parsed.context_lines.iter().take(12) {
                    ctx.push_str(l);
                    ctx.push('\n');
                }
            }
            ctx
        };

        let mut contents: Vec<serde_json::Value> = Vec::new();
        for m in parsed.history.iter().rev().take(8).rev() {
            let role = if m.role == "model" { "model" } else { "user" };
            contents.push(serde_json::json!({ "role": role, "parts": [{ "text": m.text }] }));
        }
        contents.push(serde_json::json!({ "role": "user", "parts": [{ "text": parsed.message }] }));

        let mut actions: Vec<AgentAction> = Vec::new();
        let mut calibration: Vec<CalibrationCandidate> = Vec::new();
        let mut vocab_changed = false;
        let mut final_reply = String::new();

        for _hop in 0..3 {
            let payload = serde_json::json!({
                "system_instruction": { "parts": [{ "text": sys_ctx }] },
                "contents": contents,
                "tools": agent::tool_declarations(),
                "generationConfig": { "temperature": 0.4, "maxOutputTokens": 700 }
            });
            let resp = match agent::gemini_generate_flash(&auth, &payload) {
                Ok(v) => v,
                Err(e) => return respond_err(req, 400, &e),
            };

            let parts = resp["candidates"][0]["content"]["parts"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            let mut tool_calls: Vec<(String, serde_json::Value)> = Vec::new();
            for p in &parts {
                if let Some(txt) = p.get("text").and_then(|t| t.as_str()) {
                    if !txt.trim().is_empty() {
                        if !final_reply.is_empty() {
                            final_reply.push('\n');
                        }
                        final_reply.push_str(txt.trim());
                    }
                }
                if let Some(fc) = p.get("functionCall") {
                    if let Some(name) = fc.get("name").and_then(|n| n.as_str()) {
                        let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                        tool_calls.push((name.to_string(), args));
                    }
                }
            }

            if tool_calls.is_empty() {
                break;
            }

            contents.push(serde_json::json!({ "role": "model", "parts": parts }));
            let mut fn_resp_parts: Vec<serde_json::Value> = Vec::new();
            for (name, args) in tool_calls {
                let result_val = match name.as_str() {
                    "mark_vocab" => {
                        let word = args["word"].as_str().unwrap_or("").trim().to_string();
                        let status = args["status"].as_str().unwrap_or("known").trim().to_string();
                        if !word.is_empty() {
                            let mut st = state_arc.lock().unwrap();
                            let vf = st.vocab_file.clone();
                            st.vocab.set_status(&word, &status);
                            st.vocab.save(&vf);
                            st.save();
                            vocab_changed = true;
                            actions.push(AgentAction {
                                tool: "mark_vocab".into(),
                                summary: format!("Marked {} → {}", word, status),
                            });
                            serde_json::json!({ "ok": true, "word": word, "status": status })
                        } else {
                            serde_json::json!({ "ok": false, "error": "empty word" })
                        }
                    }
                    "suggest_calibration_words" => {
                        let cnt = args["count"].as_u64().unwrap_or(8) as usize;
                        let cands = collect_calibration_words(&state_arc, cnt);
                        calibration = cands.clone();
                        actions.push(AgentAction {
                            tool: "suggest_calibration_words".into(),
                            summary: format!("Found {} likely-known words", cands.len()),
                        });
                        serde_json::json!({ "candidates": cands })
                    }
                    _ => serde_json::json!({ "error": "unknown tool" }),
                };
                fn_resp_parts.push(serde_json::json!({
                    "functionResponse": { "name": name, "response": result_val }
                }));
            }
            contents.push(serde_json::json!({ "role": "user", "parts": fn_resp_parts }));
        }

        let vocab_out = if vocab_changed {
            Some(state_arc.lock().unwrap().vocab.clone())
        } else {
            None
        };
        respond_json(
            req,
            200,
            &serde_json::json!({
                "reply": final_reply,
                "actions": actions,
                "calibration": calibration,
                "vocab_changed": vocab_changed,
                "vocab": vocab_out,
            }),
        );
        return;
    }

    // POST /api/ai/gemini-cli/login (launches official Google `gemini` CLI interactive Google sign-in)
    if method == Method::Post && path_only == "/api/ai/gemini-cli/login" {
        let app_root = { state_arc.lock().unwrap().app_root.clone() };
        let Some(bin) = cloud_sync::find_gemini_cli(&app_root) else {
            return respond_err(
                req,
                400,
                "Gemini CLI not installed — run setup.ps1 to install portable Node + @google/gemini-cli",
            );
        };
        let node_dir = bin
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let ps_cmd = format!(
            "$env:Path = '{};' + $env:Path; Write-Host '=== Kiku: Sign in with Google (Google AI Plus) ===' -ForegroundColor Cyan; Write-Host 'Choose Login with Google below. Once signed in, close this window.' -ForegroundColor DarkGray; & '{}'",
            node_dir.replace('\'', "''"),
            bin.to_string_lossy().replace('\'', "''"),
        );
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &ps_cmd]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE (visible foreground terminal!)
        }
        let launched = cmd.spawn();
        match launched {
            Ok(_) => respond_json(req, 200, &serde_json::json!({ "launched": true })),
            Err(e) => respond_err(req, 500, &format!("Could not launch Google sign-in: {}", e)),
        }
        return;
    }

    // POST /api/cloud/sync (pull remote profile -> union-merge -> save -> push merged profile to Cloudflare Worker)
    if method == Method::Post && path_only == "/api/cloud/sync" {
        let body = read_body_string(&mut req).unwrap_or_default();
        let parsed: CloudSyncReq = serde_json::from_str(&body).unwrap_or(CloudSyncReq {
            url: None,
            secret: None,
        });
        {
            let mut st = state_arc.lock().unwrap();
            let mut changed = false;
            if let Some(u) = parsed.url {
                let clean = u.trim().trim_end_matches('/').to_string();
                if !clean.is_empty() && st.data.config.cloud_sync_url != clean {
                    st.data.config.cloud_sync_url = clean;
                    changed = true;
                }
            }
            if let Some(s) = parsed.secret {
                let clean = s.trim().to_string();
                if !clean.is_empty() && st.cloud_sync_secret != clean {
                    st.cloud_sync_secret = clean;
                    changed = true;
                }
            }
            if changed {
                st.save();
            }
        }
        match sync_cloud_profile_internal(&state_arc) {
            Ok(()) => {
                let payload = {
                    let st = state_arc.lock().unwrap();
                    serde_json::json!({
                        "ok": true,
                        "config": st.public_config(),
                        "vocab": st.vocab,
                        "history": st.data.history,
                        "progress": st.data.progress,
                        "playlists": st.data.playlists,
                        "stats": st.compute_stats(),
                    })
                };
                respond_json(req, 200, &payload);
            }
            Err(e) => respond_err(req, 400, &e),
        }
        return;
    }

    // POST /api/oauth/google/start (RFC 8252 Desktop App PKCE flow!)
    if method == Method::Post && path_only == "/api/oauth/google/start" {
        let body = read_body_string(&mut req).unwrap_or_default();
        let parsed: OAuthStartReq = serde_json::from_str(&body).unwrap_or(OAuthStartReq {
            client_id: None,
            client_secret: None,
        });
        let host_hdr = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .map(|h| h.value.as_str().to_string())
            .unwrap_or_else(|| "127.0.0.1:41937".to_string());
        let redirect_uri = format!("http://{}/api/oauth/google/callback", host_hdr);

        let (client_id, auth_url) = {
            let mut st = state_arc.lock().unwrap();
            if let Some(cid) = parsed.client_id {
                if !cid.trim().is_empty() {
                    st.google_oauth.client_id = cid.trim().to_string();
                }
            }
            if let Some(csec) = parsed.client_secret {
                if !csec.trim().is_empty() {
                    st.google_oauth.client_secret = csec.trim().to_string();
                }
            }
            let cid = st.google_oauth.client_id.trim().to_string();
            if cid.is_empty() {
                return respond_err(req, 400, "Enter your Google Desktop OAuth Client ID first");
            }
            let (pending, url) = oauth::start_google_pkce(&cid, &redirect_uri);
            st.pending_oauth = Some(pending);
            st.save();
            (cid, url)
        };
        respond_json(
            req,
            200,
            &serde_json::json!({ "auth_url": auth_url, "client_id": client_id, "redirect_uri": redirect_uri }),
        );
        return;
    }

    // GET /api/oauth/google/callback?code=...&state=...
    if method == Method::Get && path_only == "/api/oauth/google/callback" {
        let mut code = String::new();
        let mut state_param = String::new();
        let mut err_param = String::new();
        for kv in query.split('&') {
            if let Some((k, v)) = kv.split_once('=') {
                if let Ok(dec) = percent_decode_str(v).decode_utf8() {
                    match k {
                        "code" => code = dec.into_owned(),
                        "state" => state_param = dec.into_owned(),
                        "error" => err_param = dec.into_owned(),
                        _ => {}
                    }
                }
            }
        }

        let html_page = |ok: bool, title: &str, msg: &str| {
            let accent = if ok { "#2ECC40" } else { "#EE4766" };
            let auto_close = if ok {
                "<script>try{if(window.opener)window.opener.postMessage('kiku-oauth-ok','*');}catch(e){} setTimeout(()=>window.close(),1400);</script>"
            } else {
                ""
            };
            format!(
                "<!doctype html><html><head><meta charset='utf-8'><title>Kiku OAuth</title>\
                <style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#120A1C;color:#F4EAF3;font-family:Inter,Segoe UI,sans-serif}}\
                .card{{max-width:420px;padding:24px 28px;border-radius:16px;background:#180F24;border:1px solid rgba(180,157,224,.22);text-align:center}}\
                h2{{margin:0 0 8px;font-size:17px;color:{}}}p{{margin:0;font-size:13px;color:#B6A5C7;line-height:1.5}}</style></head>\
                <body><div class='card'><h2>{}</h2><p>{}</p></div>{}</body></html>",
                accent, title, msg, auto_close
            )
        };

        if !err_param.is_empty() {
            let resp = Response::from_string(html_page(false, "Sign-in cancelled", &err_param))
                .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
            let _ = req.respond(resp);
            return;
        }

        let (pending_opt, mut oauth_sec) = {
            let mut st = state_arc.lock().unwrap();
            (st.pending_oauth.take(), st.google_oauth.clone())
        };
        let Some(pending) = pending_opt else {
            let resp = Response::from_string(html_page(false, "Session expired", "Please click Sign in with Google again from Kiku."))
                .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
            let _ = req.respond(resp);
            return;
        };
        if pending.state != state_param || code.is_empty() {
            let resp = Response::from_string(html_page(false, "Invalid OAuth state", "State mismatch — please retry sign-in from Kiku."))
                .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
            let _ = req.respond(resp);
            return;
        }

        match oauth::exchange_google_code(&mut oauth_sec, &code, &pending) {
            Ok(()) => {
                let email = oauth_sec.email.clone();
                {
                    let mut st = state_arc.lock().unwrap();
                    st.google_oauth = oauth_sec;
                    st.save();
                }
                let msg = if email.is_empty() {
                    "Google OAuth connected. You can close this tab.".to_string()
                } else {
                    format!("Signed in as {}. You can close this tab.", email)
                };
                let resp = Response::from_string(html_page(true, "Connected to Kiku", &msg))
                    .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
                let _ = req.respond(resp);
            }
            Err(e) => {
                let resp = Response::from_string(html_page(false, "OAuth exchange failed", &e))
                    .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
                let _ = req.respond(resp);
            }
        }
        return;
    }

    // POST /api/oauth/google/disconnect
    if method == Method::Post && path_only == "/api/oauth/google/disconnect" {
        let pub_cfg = {
            let mut st = state_arc.lock().unwrap();
            st.google_oauth.access_token.clear();
            st.google_oauth.refresh_token.clear();
            st.google_oauth.expires_at_unix = 0;
            st.google_oauth.email.clear();
            st.pending_oauth = None;
            st.save();
            st.public_config()
        };
        respond_json(req, 200, &serde_json::json!({ "config": pub_cfg }));
        return;
    }

    // POST /api/toggl/flush (called on app close or manually to batch-commit all pending local entries + cloud profile push)
    if method == Method::Post && path_only == "/api/toggl/flush" {
        let body = read_body_string(&mut req).unwrap_or_default();
        let parsed: SessionStopReq = serde_json::from_str(&body).unwrap_or(SessionStopReq {
            elapsed_secs: None,
        });
        let _ = stop_active_session_internal(&state_arc, parsed.elapsed_secs);
        let st_bg = Arc::clone(&state_arc);
        thread::spawn(move || {
            let _ = flush_pending_toggl(&st_bg);
            let _ = sync_cloud_profile_internal(&st_bg);
        });
        respond_json(req, 200, &serde_json::json!({ "flushing": true }));
        return;
    }

    // POST /api/toggl/test
    if method == Method::Post && path_only == "/api/toggl/test" {
        let body = read_body_string(&mut req).unwrap_or_default();
        let parsed: TogglTestReq =
            serde_json::from_str(&body).unwrap_or(TogglTestReq { token: None });
        let token = {
            let st = state_arc.lock().unwrap();
            parsed
                .token
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| st.data.config.toggl_api_token.clone())
        };
        match toggl::test_connection(&token) {
            Ok(res) => {
                // Persist token + default workspace if none set yet
                let pub_cfg = {
                    let mut st = state_arc.lock().unwrap();
                    st.data.config.toggl_api_token = token.trim().to_string();
                    if st.data.config.toggl_workspace_id.is_none() {
                        if let Some(wid) = res.default_workspace_id {
                            st.data.config.toggl_workspace_id = Some(wid);
                            if let Some(w) = res.workspaces.iter().find(|w| w.id == wid) {
                                st.data.config.toggl_workspace_name = w.name.clone();
                            }
                        }
                    }
                    st.save();
                    st.public_config()
                };
                respond_json(
                    req,
                    200,
                    &serde_json::json!({ "toggl": res, "config": pub_cfg }),
                );
            }
            Err(e) => respond_err(req, 400, &e),
        }
        return;
    }

    // GET /api/toggl/projects?workspace_id=...
    if method == Method::Get && path_only == "/api/toggl/projects" {
        let wid: Option<i64> = query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            if k == "workspace_id" {
                v.parse().ok()
            } else {
                None
            }
        });
        let (token, fallback_wid) = {
            let st = state_arc.lock().unwrap();
            (
                st.data.config.toggl_api_token.clone(),
                st.data.config.toggl_workspace_id,
            )
        };
        let Some(workspace_id) = wid.or(fallback_wid) else {
            return respond_err(req, 400, "Missing workspace_id");
        };
        match toggl::get_projects(&token, workspace_id) {
            Ok(projects) => respond_json(req, 200, &serde_json::json!({ "projects": projects })),
            Err(e) => respond_err(req, 400, &e),
        }
        return;
    }

    respond_err(req, 404, "Not found");
}

pub fn run_server(port: u16, state: Arc<Mutex<SharedState>>) -> Result<(u16, Server), String> {
    for p in port..port.saturating_add(25) {
        let addr = format!("127.0.0.1:{}", p);
        if let Ok(server) = Server::http(&addr) {
            return Ok((p, server));
        }
    }
    let _ = state;
    Err(format!("Could not bind to localhost starting at port {}", port))
}

pub fn serve_loop(server: Server, state: Arc<Mutex<SharedState>>) {
    // Background startup cloud profile pull & lossless union-merge (if Cloudflare Worker URL is configured)
    {
        let st_boot = Arc::clone(&state);
        thread::spawn(move || {
            let _ = sync_cloud_profile_internal(&st_boot);
        });
    }
    // Airgapped Toggl + Cloud Profile periodic batch-sync thread (commits every 15 min)
    {
        let st_cycle = Arc::clone(&state);
        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_secs(900));
            let _ = flush_pending_toggl(&st_cycle);
            let _ = sync_cloud_profile_internal(&st_cycle);
        });
    }
    for req in server.incoming_requests() {
        let st = Arc::clone(&state);
        thread::spawn(move || {
            handle_request(req, st);
        });
    }
}
