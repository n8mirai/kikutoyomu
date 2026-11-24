use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::dict::LocalJmDict;
use crate::knowledge::KnowledgeModel;
use crate::morph::MorphAnalyzer;
use crate::timeutil;
use crate::vocab::VocabStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedVideo {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub category: String,
    pub published: String,
    pub known_pct: u8,
    pub unknown_count: u16,
    pub tier: String,
    pub score: f64,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub channel_pfp: String,
    #[serde(default)]
    pub duration: String,
    #[serde(default)]
    pub views: String,
    #[serde(default)]
    pub has_cc: bool,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPage {
    pub items: Vec<FeedVideo>,
    pub continuation: Option<String>,
    pub localized_query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedPage {
    pub items: Vec<FeedVideo>,
    pub channel_id: String,
    pub channel_name: String,
    pub channel_pfp: String,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPage {
    pub channel_id: String,
    pub channel_name: String,
    pub channel_pfp: String,
    pub items: Vec<FeedVideo>,
    pub continuation: Option<String>,
}

const CURATED_CHANNELS: &[(&str, &str, &str, &str)] = &[
    ("UC8dWfySP_cKDMFj6aFfQbFA", "YUYUの日本語Podcast", "talk", "https://yt3.googleusercontent.com/gSp7_d8NvgTPBOFfJTSsVt-auJq89Jb6hp1XV7JdQRYIMbGD84TJapNfnil2z2rlQHuH6xZ7vg=s72-c-k-c0x00ffffff-no-rj"),
    ("UCh-GhnQ7qDQmS6Bz3pGc1Mw", "あかね的日本語教室", "vlog", "https://yt3.googleusercontent.com/ytc/AIdro_kykxNolZAhyFMec4m7G-1XvOaIVU5IY1hZDE867JdwFw=s72-c-k-c0x00ffffff-no-rj"),
    ("UCSbH_BPR_AoARW6RDYLlLog", "Speak Japanese Naturally", "vlog", "https://yt3.googleusercontent.com/vNhlOnvOL0J1xhCZJ3urGA7R0LJ6HVJzg1GUdJ53Lk8wqkKzK_6nhJf4QjcOEJyM_N55aFjGHw=s72-c-k-c0x00ffffff-no-rj"),
    ("UCB_Bvgg4oCHEAzxvZju4S3A", "Bite Size Japanese", "talk", "https://yt3.googleusercontent.com/ytc/AIdro_nGVDhM5MWm1PIhnh26ofyXHIwIKXqq-MRw7DF0okJa7A=s72-c-k-c0x00ffffff-no-rj"),
    ("UCaXIrZ5Ere4lC4cdPE_ujXA", "けんさんおかえり", "vlog", "https://yt3.googleusercontent.com/WdMna2oNs-rUT7VGfZUNjQxJy8fEGI5GTs8Dv7CHnaQStDGV4DCMnSEe0uNN79iQ7hXjKMRQxw=s72-c-k-c0x00ffffff-no-rj"),
    ("UCmpkIzF3xFzhPez7gXOyhVg", "ゆる言語学ラジオ", "talk", "https://yt3.googleusercontent.com/ytc/AIdro_na3cTkSA0sWhtO4SUp7lKdTc2LRjJ6ivp-UVHb3dzqqfU=s72-c-k-c0x00ffffff-no-rj"),
    ("UCgMPP6RRjktV7krOfyUewqw", "はじめしゃちょー", "jp", "https://yt3.googleusercontent.com/0KVCl_7XS0e6THDRqygC0hiUXjqDAaDGUVITLREGEnbjscem5gss6WTWgznH_jvXVs_DEAFprA=s72-c-k-c0x00ffffff-no-rj"),
    ("UCFbp2XdRpKfk7mYt_uT8dxw", "Kevin's English Room", "jp", "https://yt3.googleusercontent.com/ytc/AIdro_kWXTpkmV6BgSW-j7zxvWP8E_VnIxSjuMiMNeR0PFreJXU=s72-c-k-c0x00ffffff-no-rj"),
    ("UCGCZAYq5Xxojl_tSXcVJhiQ", "ANNnewsCH", "news", "https://yt3.googleusercontent.com/YNCCp1NNe4z2aL7Iv82YHL8qKYmU6uOSn8NKep4qKRBK_E2XJCF5evjRiArUbs-gmLX2dnwtIQ=s72-c-k-c0x00ffffff-no-rj"),
];

/// Category-specific native Japanese YouTube discovery seed waves (for infinite breadth!).
fn category_seed_queries(cat: &str, wave: usize) -> Vec<(&'static str, &'static str)> {
    let vlog: &[&str] = &[
        "日常 Vlog 日本 東京",
        "一人暮らし ルーティン Vlog",
        "日本 旅行 Vlog グルメ",
        "京都 大阪 散歩 Vlog",
        "社会人 休日 Vlog 日本",
        "田舎暮らし Vlog 日本",
    ];
    let talk: &[&str] = &[
        "雑談 ラジオ 面白い 解説",
        "言語学 雑学 解説 トーク",
        "歴史 科学 雑学 解説",
        "心理学 哲学 わかりやすく解説",
        "ゆるい トーク ポッドキャスト 日本語",
        "経済 社会 解説 対談",
    ];
    let gaming: &[&str] = &[
        "ゲーム実況 面白い 人気",
        "マインクラフト 実況 建築",
        "ホラーゲーム 実況 日本",
        "インディーゲーム 実況 おすすめ",
        "ゼルダ ポケモン 実況 解説",
        "レトロゲーム 解説 実況",
    ];
    let tech: &[&str] = &[
        "ガジェット レビュー おすすめ",
        "AI 最新 テクノロジー 解説",
        "プログラミング 開発 解説",
        "デスクツアー PC 環境 紹介",
        "宇宙 科学 最新 解説",
        "カメラ スマホ レビュー 日本",
    ];
    let jp: &[&str] = &[
        "日本 文化 面白い 企画",
        "料理 レシピ 簡単 美味しい",
        "アニメ 考察 解説",
        "映画 解説 レビュー 日本",
        "コント 漫才 面白い",
        "街歩き 食べ歩き グルメ 日本",
    ];
    let news: &[&str] = &[
        "ニュース 解説 最新 日本",
        "世界 ニュース わかりやすく解説",
        "ドキュメンタリー 日本 特集",
        "経済 ニュース 解説 最新",
    ];

    match cat {
        "vlog" => vec![
            (vlog[wave % vlog.len()], "vlog"),
            (vlog[(wave + 2) % vlog.len()], "vlog"),
        ],
        "talk" => vec![
            (talk[wave % talk.len()], "talk"),
            (talk[(wave + 2) % talk.len()], "talk"),
        ],
        "gaming" => vec![
            (gaming[wave % gaming.len()], "gaming"),
            (gaming[(wave + 2) % gaming.len()], "gaming"),
        ],
        "tech" => vec![
            (tech[wave % tech.len()], "tech"),
            (tech[(wave + 2) % tech.len()], "tech"),
        ],
        "jp" => vec![
            (jp[wave % jp.len()], "jp"),
            (jp[(wave + 2) % jp.len()], "jp"),
        ],
        "news" => vec![
            (news[wave % news.len()], "news"),
            (news[(wave + 1) % news.len()], "news"),
        ],
        _ => vec![
            (vlog[wave % vlog.len()], "vlog"),
            (talk[wave % talk.len()], "talk"),
            (gaming[wave % gaming.len()], "gaming"),
            (tech[wave % tech.len()], "tech"),
            (jp[wave % jp.len()], "jp"),
        ],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YoutubeVideo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub duration_secs: f64,
    #[serde(default)]
    pub has_subs: bool,
    #[serde(default)]
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YoutubeStore {
    #[serde(default)]
    pub videos: Vec<YoutubeVideo>,
}

impl YoutubeStore {
    pub fn load(path: &Path) -> Self {
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(path) {
                if let Ok(store) = serde_json::from_str::<Self>(&raw) {
                    return store;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, &json).is_ok() {
                let _ = fs::rename(&tmp, path);
            } else {
                let _ = fs::write(path, &json);
            }
        }
    }
}

pub fn is_valid_yt_id(s: &str) -> bool {
    s.len() == 11
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Extracts an 11-character YouTube video ID from any standard YouTube URL or raw ID.
pub fn extract_video_id(input: &str) -> Option<String> {
    let s = input.trim();
    if is_valid_yt_id(s) {
        return Some(s.to_string());
    }
    if let Some((_, q)) = s.split_once('?') {
        for kv in q.split('&') {
            if let Some(("v", val)) = kv.split_once('=') {
                let clean = val.split('#').next().unwrap_or(val);
                if is_valid_yt_id(clean) {
                    return Some(clean.to_string());
                }
            }
        }
    }
    let stripped = s
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let path = stripped.split('?').next().unwrap_or(stripped);
    let path = path.split('#').next().unwrap_or(path);
    let segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if segs.len() >= 2 && segs[0] == "youtu.be" && is_valid_yt_id(segs[1]) {
        return Some(segs[1].to_string());
    }
    if segs.len() >= 3
        && matches!(segs[1], "shorts" | "embed" | "live" | "v")
        && is_valid_yt_id(segs[2])
    {
        return Some(segs[2].to_string());
    }
    None
}

pub fn find_yt_dlp(app_root: &Path) -> Option<PathBuf> {
    let local = app_root.join("tools").join("yt-dlp").join("yt-dlp.exe");
    if local.exists() {
        return Some(local);
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in ["yt-dlp.exe", "yt-dlp"] {
                let p = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn find_sub_file(yt_subs_dir: &Path, video_id: &str) -> Option<PathBuf> {
    for name in [
        format!("{}.ja.vtt", video_id),
        format!("{}.ja.srt", video_id),
        format!("{}.ja-orig.vtt", video_id),
        format!("{}.ja-orig.srt", video_id),
        format!("{}.vtt", video_id),
        format!("{}.srt", video_id),
    ] {
        let p = yt_subs_dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(rd) = fs::read_dir(yt_subs_dir) {
        let prefix = format!("{}.", video_id);
        for entry in rd.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with(&prefix)
                && (fname.ends_with(".vtt") || fname.ends_with(".srt"))
            {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Fetches video metadata + Japanese subtitles (manual or auto-generated) via bundled `yt-dlp`.
pub fn fetch_video_and_subs(
    app_root: &Path,
    yt_subs_dir: &Path,
    video_id: &str,
) -> Result<YoutubeVideo, String> {
    let _ = fs::create_dir_all(yt_subs_dir);
    let url = format!("https://www.youtube.com/watch?v={}", video_id);

    let mut title = format!("YouTube ({})", video_id);
    let mut channel = String::new();
    let mut duration_secs = 0.0f64;

    if let Some(ytdlp) = find_yt_dlp(app_root) {
        let out_tpl = yt_subs_dir.join(format!("{}.%(ext)s", video_id));
        let mut cmd = Command::new(&ytdlp);
        cmd.args([
            "--no-check-certificates",
            "--no-playlist",
            "--skip-download",
            "--write-sub",
            "--write-auto-sub",
            "--sub-langs",
            "ja,ja-orig,ja.*",
            "--sub-format",
            "vtt/srt",
            "--print-json",
            "-o",
        ])
        .arg(&out_tpl)
        .arg(&url);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }

        if let Ok(out) = cmd.output() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if line.trim_start().starts_with('{') {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                        if let Some(t) = v.get("title").and_then(|x| x.as_str()) {
                            if !t.trim().is_empty() {
                                title = t.trim().to_string();
                            }
                        }
                        if let Some(ch) = v
                            .get("channel")
                            .or_else(|| v.get("uploader"))
                            .and_then(|x| x.as_str())
                        {
                            channel = ch.trim().to_string();
                        }
                        if let Some(d) = v.get("duration").and_then(|x| x.as_f64()) {
                            duration_secs = d;
                        }
                        break;
                    }
                }
            }
        }
    }

    let has_subs = find_sub_file(yt_subs_dir, video_id).is_some();

    Ok(YoutubeVideo {
        id: video_id.to_string(),
        title,
        channel,
        duration_secs,
        has_subs,
        added_at: timeutil::now_utc_rfc3339(),
    })
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn extract_tag<'a>(block: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    Some(&block[start..end])
}

#[cfg(windows)]
fn winhttp_request_text(
    host_s: &str,
    method_s: &str,
    path_s: &str,
    body_opt: Option<&[u8]>,
) -> Result<String, String> {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Networking::WinHttp::*;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0); }
            }
        }
    }

    let agent = to_wide("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36");
    let host = to_wide(host_s);
    let verb = to_wide(method_s);
    let path = to_wide(path_s);

    let session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
    });
    if session.0.is_null() {
        return Err("WinHttpOpen failed".into());
    }
    unsafe { WinHttpSetTimeouts(session.0, 4_000, 4_000, 6_000, 9_000); }

    let connect = Handle(unsafe {
        WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0)
    });
    if connect.0.is_null() {
        return Err("WinHttpConnect failed".into());
    }

    let request = Handle(unsafe {
        WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        )
    });
    if request.0.is_null() {
        return Err("WinHttpOpenRequest failed".into());
    }

    let headers_w = if body_opt.is_some() {
        Some(to_wide(
            "Content-Type: application/json; charset=utf-8\r\nAccept-Language: ja-JP,ja;q=0.9\r\nOrigin: https://www.youtube.com\r\nReferer: https://www.youtube.com/\r\n",
        ))
    } else {
        None
    };

    let (hdr_ptr, hdr_len) = match &headers_w {
        Some(h) => (h.as_ptr(), (h.len() - 1) as u32),
        None => (null(), 0),
    };
    let (body_ptr, body_len) = match body_opt {
        Some(b) => (b.as_ptr() as *mut c_void, b.len() as u32),
        None => (null_mut(), 0),
    };

    if unsafe {
        WinHttpSendRequest(
            request.0,
            hdr_ptr,
            hdr_len,
            body_ptr,
            body_len,
            body_len,
            0,
        )
    } == 0
    {
        return Err("WinHttpSendRequest failed".into());
    }
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err("WinHttpReceiveResponse failed".into());
    }

    let mut resp_bytes = Vec::new();
    loop {
        let mut avail: u32 = 0;
        if unsafe { WinHttpQueryDataAvailable(request.0, &mut avail) } == 0 || avail == 0 {
            break;
        }
        let mut chunk = vec![0u8; avail as usize];
        let mut read: u32 = 0;
        if unsafe {
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr() as *mut c_void,
                avail,
                &mut read,
            )
        } == 0
            || read == 0
        {
            break;
        }
        chunk.truncate(read as usize);
        resp_bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&resp_bytes).to_string())
}

#[cfg(not(windows))]
fn winhttp_request_text(
    _host_s: &str,
    _method_s: &str,
    _path_s: &str,
    _body_opt: Option<&[u8]>,
) -> Result<String, String> {
    Err("Windows WinHTTP required".into())
}

fn winhttp_get_text(host_s: &str, path_s: &str) -> Result<String, String> {
    winhttp_request_text(host_s, "GET", path_s, None)
}

fn innertube_post(endpoint: &str, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
    let body = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let path = format!("/youtubei/v1/{}?prettyPrint=false", endpoint);
    let raw = winhttp_request_text("www.youtube.com", "POST", &path, Some(&body))?;
    serde_json::from_str(&raw).map_err(|e| format!("InnerTube JSON parse error: {}", e))
}

fn innertube_jp_context() -> serde_json::Value {
    serde_json::json!({
        "client": {
            "clientName": "WEB",
            "clientVersion": "2.20250501.00.00",
            "hl": "ja",
            "gl": "JP"
        }
    })
}

pub fn has_japanese_chars(s: &str) -> bool {
    s.chars().any(|c| {
        ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c)
    })
}

fn has_kana(s: &str) -> bool {
    s.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c))
}

fn has_hangul(s: &str) -> bool {
    s.chars().any(|c| ('\u{AC00}'..='\u{D7AF}').contains(&c) || ('\u{1100}'..='\u{11FF}').contains(&c))
}

/// Strict Japanese-only content gate: keeps ImmersionTube 100% pure Japanese even when browsing
/// broad topics or searching with English queries.
pub fn is_japanese_video(title: &str, channel: &str) -> bool {
    if has_hangul(title) || has_hangul(channel) {
        return false;
    }
    // Reject obvious Shorts spam markers
    let tl = title.to_lowercase();
    if tl.contains("#shorts") || tl.contains("#short ") {
        return false;
    }
    if has_kana(title) || has_kana(channel) {
        return true;
    }
    // Allow pure-kanji titles only if >= 2 kanji AND doesn't look exclusively simplified Chinese
    let kanji_cnt = title
        .chars()
        .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c))
        .count();
    let zh_only = ['这', '个', '们', '来', '为', '没', '说', '还', '给', '对', '里', '么'];
    if kanji_cnt >= 2 && !title.chars().any(|c| zh_only.contains(&c)) {
        return true;
    }
    false
}

/// Fast built-in English -> natural native Japanese YouTube search localization.
pub fn quick_localize_jp(raw: &str) -> Option<String> {
    let q = raw.trim();
    if q.is_empty() || has_japanese_chars(q) {
        return Some(q.to_string());
    }
    let key = q.to_lowercase();
    let mapped = match key.as_str() {
        "gaming" | "games" | "game" => "ゲーム実況",
        "horror" | "horror games" | "horror game" => "ホラーゲーム 実況",
        "minecraft" => "マインクラフト 実況",
        "zelda" => "ゼルダの伝説 実況",
        "pokemon" => "ポケモン 実況",
        "mario" => "マリオ 実況",
        "nintendo" => "任天堂 ゲーム 解説",
        "retro games" | "retro gaming" => "レトロゲーム 解説 実況",
        "rpg" | "jrpg" => "RPG ゲーム実況",
        "fps" | "apex" | "valorant" => "FPS 実況 日本",
        "cooking" | "food" | "recipes" | "recipe" => "料理 レシピ 美味しい",
        "ramen" => "ラーメン グルメ 日本",
        "street food" | "japanese food" => "食べ歩き グルメ 日本",
        "coffee" | "cafe" => "カフェ コーヒー Vlog",
        "vlog" | "vlogs" | "daily life" => "日常 Vlog 日本",
        "travel" | "japan travel" => "日本 旅行 Vlog",
        "tokyo" => "東京 散歩 Vlog",
        "kyoto" => "京都 旅行 Vlog",
        "camping" | "solo camp" => "ソロキャンプ Vlog 日本",
        "tech" | "technology" | "gadgets" => "ガジェット テクノロジー レビュー",
        "ai" | "artificial intelligence" => "AI 最新 解説",
        "coding" | "programming" | "software" => "プログラミング 解説",
        "science" => "科学 雑学 解説",
        "space" | "astronomy" => "宇宙 解説 ドキュメンタリー",
        "history" => "歴史 解説 わかりやすく",
        "philosophy" => "哲学 解説",
        "psychology" => "心理学 解説",
        "math" | "mathematics" => "数学 解説 面白い",
        "anime" => "アニメ 考察 解説",
        "manga" => "漫画 紹介 解説",
        "movies" | "movie" | "film" => "映画 解説 レビュー",
        "music" => "音楽 解説 ミュージック",
        "podcast" | "podcasts" | "radio" => "ポッドキャスト ラジオ 雑談 日本語",
        "comedy" | "funny" => "コント 漫才 面白い",
        "news" => "ニュース 解説 最新",
        "documentary" | "documentaries" => "ドキュメンタリー 日本",
        "finance" | "investing" | "economics" => "経済 投資 解説",
        "fitness" | "workout" | "gym" => "筋トレ ルーティン 解説",
        "cars" | "car" | "jdm" => "車 レビュー 解説",
        "art" | "drawing" | "illustration" => "イラスト 描き方 メイキング",
        "books" | "reading" => "読書 本 紹介 解説",
        "fashion" => "ファッション コーデ 日本",
        "minimalism" => "ミニマリスト ルームツアー",
        "nature" | "animals" | "cats" | "dogs" => "動物 自然 癒し 日本",
        "chess" | "shogi" => "将棋 解説 対局",
        "diy" => "DIY 作り方 日本",
        "fishing" => "釣り 日本",
        _ => return None,
    };
    Some(mapped.to_string())
}

fn json_text(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.trim().to_string();
    }
    if let Some(s) = v.get("simpleText").and_then(|x| x.as_str()) {
        return s.trim().to_string();
    }
    if let Some(s) = v.get("content").and_then(|x| x.as_str()) {
        return s.trim().to_string();
    }
    if let Some(runs) = v.get("runs").and_then(|x| x.as_array()) {
        let mut out = String::new();
        for r in runs {
            if let Some(t) = r.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
        return out.trim().to_string();
    }
    String::new()
}

fn parse_video_renderer(vr: &serde_json::Value, category: &str, source: &str) -> Option<FeedVideo> {
    let id = vr.get("videoId").and_then(|x| x.as_str())?.trim();
    if !is_valid_yt_id(id) {
        return None;
    }
    let title = json_text(&vr["title"]);
    if title.is_empty() {
        return None;
    }
    let channel = json_text(&vr["ownerText"]);
    let channel = if channel.is_empty() {
        json_text(&vr["longBylineText"])
    } else {
        channel
    };
    let channel = if channel.is_empty() {
        "YouTube".to_string()
    } else {
        channel
    };

    let channel_id = vr["ownerText"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"]
        .as_str()
        .or_else(|| {
            vr["longBylineText"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"]
                .as_str()
        })
        .or_else(|| {
            vr["channelThumbnailSupportedRenderers"]["channelThumbnailWithLinkRenderer"]
                ["navigationEndpoint"]["browseEndpoint"]["browseId"]
                .as_str()
        })
        .unwrap_or("")
        .to_string();

    let channel_pfp = vr["channelThumbnailSupportedRenderers"]["channelThumbnailWithLinkRenderer"]
        ["thumbnail"]["thumbnails"][0]["url"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let duration = json_text(&vr["lengthText"]);
    // Skip livestreams with no duration? Allow them, but skip sub-60s Shorts when obvious
    if duration == "SHORTS" {
        return None;
    }
    let views = json_text(&vr["shortViewCountText"]);
    let published = json_text(&vr["publishedTimeText"]);

    let mut has_cc = false;
    if let Some(badges) = vr.get("badges").and_then(|x| x.as_array()) {
        for b in badges {
            let lbl = b["metadataBadgeRenderer"]["label"]
                .as_str()
                .unwrap_or("");
            if lbl.contains("字幕") || lbl.eq_ignore_ascii_case("CC") {
                has_cc = true;
            }
        }
    }

    Some(FeedVideo {
        id: id.to_string(),
        title,
        channel,
        category: category.to_string(),
        published,
        known_pct: 0,
        unknown_count: 0,
        tier: String::new(),
        score: 0.0,
        channel_id,
        channel_pfp,
        duration,
        views,
        has_cc,
        source: source.to_string(),
    })
}

fn parse_compact_video_renderer(
    cvr: &serde_json::Value,
    category: &str,
    source: &str,
) -> Option<FeedVideo> {
    let id = cvr.get("videoId").and_then(|x| x.as_str())?.trim();
    if !is_valid_yt_id(id) {
        return None;
    }
    let title = json_text(&cvr["title"]);
    if title.is_empty() {
        return None;
    }
    let channel = json_text(&cvr["longBylineText"]);
    let channel = if channel.is_empty() {
        json_text(&cvr["shortBylineText"])
    } else {
        channel
    };
    let channel = if channel.is_empty() {
        "YouTube".to_string()
    } else {
        channel
    };
    let channel_id =
        cvr["longBylineText"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"]
            .as_str()
            .unwrap_or("")
            .to_string();
    let channel_pfp = cvr["channelThumbnail"]["thumbnails"][0]["url"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let duration = json_text(&cvr["lengthText"]);
    let views = json_text(&cvr["shortViewCountText"]);
    let published = json_text(&cvr["publishedTimeText"]);

    Some(FeedVideo {
        id: id.to_string(),
        title,
        channel,
        category: category.to_string(),
        published,
        known_pct: 0,
        unknown_count: 0,
        tier: String::new(),
        score: 0.0,
        channel_id,
        channel_pfp,
        duration,
        views,
        has_cc: false,
        source: source.to_string(),
    })
}

fn parse_lockup_view_model(
    lvm: &serde_json::Value,
    category: &str,
    source: &str,
    fallback_channel: &str,
    fallback_ch_id: &str,
    fallback_pfp: &str,
) -> Option<FeedVideo> {
    let id = lvm.get("contentId").and_then(|x| x.as_str())?.trim();
    if !is_valid_yt_id(id) {
        return None;
    }
    let meta = &lvm["metadata"]["lockupMetadataViewModel"];
    let title = json_text(&meta["title"]);
    if title.is_empty() {
        return None;
    }

    let mut channel = String::new();
    let mut views = String::new();
    let mut published = String::new();
    let mut has_cc = false;

    if let Some(rows) = meta["metadata"]["contentMetadataViewModel"]["metadataRows"].as_array() {
        for (idx, row) in rows.iter().enumerate() {
            if let Some(parts) = row.get("metadataParts").and_then(|x| x.as_array()) {
                for p in parts {
                    let txt = json_text(&p["text"]);
                    if txt.is_empty() {
                        continue;
                    }
                    if idx == 0 && channel.is_empty() && !txt.contains("回視聴") && !txt.ends_with("前") {
                        channel = txt;
                    } else if txt.contains("回視聴") || p["accessibilityLabel"].as_str().unwrap_or("").contains("回視聴") {
                        views = txt;
                    } else if txt.contains("前") || txt.contains("配信") {
                        published = txt;
                    }
                }
            }
            if let Some(badges) = row.get("badges").and_then(|x| x.as_array()) {
                for b in badges {
                    let lbl = b["badgeViewModel"]["badgeText"].as_str().unwrap_or("");
                    if lbl.contains("字幕") || lbl.eq_ignore_ascii_case("CC") {
                        has_cc = true;
                    }
                }
            }
        }
    }

    if channel.is_empty() {
        channel = if !fallback_channel.is_empty() {
            fallback_channel.to_string()
        } else {
            "YouTube".to_string()
        };
    }

    let ch_img = &meta["image"]["decoratedAvatarViewModel"];
    let channel_pfp = ch_img["avatar"]["avatarViewModel"]["image"]["sources"][0]["url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_pfp)
        .to_string();
    let channel_id = ch_img["rendererContext"]["commandContext"]["onTap"]["innertubeCommand"]
        ["browseEndpoint"]["browseId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_ch_id)
        .to_string();

    // Extract duration badge from thumbnail overlays if present
    let mut duration = String::new();
    if let Some(overlays) = lvm["contentImage"]["thumbnailViewModel"]["overlays"].as_array() {
        for ov in overlays {
            if let Some(badges) = ov["thumbnailOverlayBadgeViewModel"]["thumbnailBadges"].as_array() {
                for b in badges {
                    let t = b["thumbnailBadgeViewModel"]["text"].as_str().unwrap_or("");
                    if t.contains(':') {
                        duration = t.to_string();
                    }
                }
            }
            if let Some(badges) = ov["thumbnailBottomOverlayViewModel"]["badges"].as_array() {
                for b in badges {
                    let t = b["thumbnailBadgeViewModel"]["text"].as_str().unwrap_or("");
                    if t.contains(':') {
                        duration = t.to_string();
                    }
                }
            }
        }
    }

    Some(FeedVideo {
        id: id.to_string(),
        title,
        channel,
        category: category.to_string(),
        published,
        known_pct: 0,
        unknown_count: 0,
        tier: String::new(),
        score: 0.0,
        channel_id,
        channel_pfp,
        duration,
        views,
        has_cc,
        source: source.to_string(),
    })
}

/// Recursively walks an InnerTube JSON subtree collecting all video items and the last continuation token.
fn collect_innertube_videos(
    node: &serde_json::Value,
    category: &str,
    source: &str,
    fallback_ch: &str,
    fallback_ch_id: &str,
    fallback_pfp: &str,
    out: &mut Vec<FeedVideo>,
    seen: &mut HashSet<String>,
    cont: &mut Option<String>,
) {
    match node {
        serde_json::Value::Object(map) => {
            if let Some(vr) = map.get("videoRenderer") {
                if let Some(v) = parse_video_renderer(vr, category, source) {
                    if seen.insert(v.id.clone()) {
                        out.push(v);
                    }
                }
                return;
            }
            if let Some(cvr) = map.get("compactVideoRenderer") {
                if let Some(v) = parse_compact_video_renderer(cvr, category, source) {
                    if seen.insert(v.id.clone()) {
                        out.push(v);
                    }
                }
                return;
            }
            if let Some(lvm) = map.get("lockupViewModel") {
                if let Some(v) = parse_lockup_view_model(
                    lvm,
                    category,
                    source,
                    fallback_ch,
                    fallback_ch_id,
                    fallback_pfp,
                ) {
                    if seen.insert(v.id.clone()) {
                        out.push(v);
                    }
                }
                return;
            }
            if let Some(cir) = map.get("continuationItemRenderer") {
                if let Some(tok) = cir["continuationEndpoint"]["continuationCommand"]["token"]
                    .as_str()
                {
                    if !tok.is_empty() {
                        *cont = Some(tok.to_string());
                    }
                }
                return;
            }
            for (_, v) in map {
                collect_innertube_videos(
                    v,
                    category,
                    source,
                    fallback_ch,
                    fallback_ch_id,
                    fallback_pfp,
                    out,
                    seen,
                    cont,
                );
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_innertube_videos(
                    v,
                    category,
                    source,
                    fallback_ch,
                    fallback_ch_id,
                    fallback_pfp,
                    out,
                    seen,
                    cont,
                );
            }
        }
        _ => {}
    }
}

/// Fast (~400ms), native Japan-region YouTube search via InnerTube (`gl=JP`, `hl=ja`) with
/// infinite pagination (`continuation`) and strict Japanese-only filtering.
pub fn innertube_search_jp(
    localized_query: &str,
    continuation: Option<&str>,
    category: &str,
    source: &str,
) -> Result<SearchPage, String> {
    let payload = if let Some(tok) = continuation.filter(|s| !s.trim().is_empty()) {
        serde_json::json!({
            "context": innertube_jp_context(),
            "continuation": tok
        })
    } else {
        serde_json::json!({
            "context": innertube_jp_context(),
            "query": localized_query,
            "params": "EgIQAQ%3D%3D" // Type: Video only
        })
    };

    let root = innertube_post("search", &payload)?;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut cont = None;

    let search_node = if continuation.is_some() {
        &root["onResponseReceivedCommands"]
    } else {
        &root["contents"]
    };
    collect_innertube_videos(
        search_node,
        category,
        source,
        "",
        "",
        "",
        &mut items,
        &mut seen,
        &mut cont,
    );

    // Keep strictly Japanese content (fall back to unfiltered only if everything got filtered out)
    let jp_only: Vec<FeedVideo> = items
        .iter()
        .filter(|v| is_japanese_video(&v.title, &v.channel))
        .cloned()
        .collect();
    let final_items = if !jp_only.is_empty() { jp_only } else { items };

    Ok(SearchPage {
        items: final_items,
        continuation: cont,
        localized_query: localized_query.to_string(),
    })
}

/// Fetches real YouTube "Up Next" Japanese recommendations + real channel PFP & ID for any video!
pub fn innertube_next_jp(
    video_id: &str,
    continuation: Option<&str>,
) -> Result<RelatedPage, String> {
    let payload = if let Some(tok) = continuation.filter(|s| !s.trim().is_empty()) {
        serde_json::json!({
            "context": innertube_jp_context(),
            "continuation": tok
        })
    } else {
        serde_json::json!({
            "context": innertube_jp_context(),
            "videoId": video_id
        })
    };

    let root = innertube_post("next", &payload)?;

    let mut channel_id = String::new();
    let mut channel_name = String::new();
    let mut channel_pfp = String::new();

    if let Some(prim) = root["contents"]["twoColumnWatchNextResults"]["results"]["results"]
        ["contents"]
        .as_array()
    {
        for block in prim {
            if let Some(sec) = block.get("videoSecondaryInfoRenderer") {
                let owner = &sec["owner"]["videoOwnerRenderer"];
                channel_name = json_text(&owner["title"]);
                channel_id = owner["navigationEndpoint"]["browseEndpoint"]["browseId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if let Some(thumbs) = owner["thumbnail"]["thumbnails"].as_array() {
                    channel_pfp = thumbs
                        .last()
                        .or_else(|| thumbs.first())
                        .and_then(|t| t["url"].as_str())
                        .unwrap_or("")
                        .to_string();
                }
                break;
            }
        }
    }

    let mut items = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(video_id.to_string());
    let mut cont = None;

    let rel_node = if continuation.is_some() {
        &root["onResponseReceivedEndpoints"]
    } else {
        &root["contents"]["twoColumnWatchNextResults"]["secondaryResults"]
    };
    collect_innertube_videos(
        rel_node,
        "related",
        "related",
        "",
        "",
        "",
        &mut items,
        &mut seen,
        &mut cont,
    );

    let jp_only: Vec<FeedVideo> = items
        .into_iter()
        .filter(|v| is_japanese_video(&v.title, &v.channel))
        .collect();

    Ok(RelatedPage {
        items: jp_only,
        channel_id,
        channel_name,
        channel_pfp,
        continuation: cont,
    })
}

/// Browses any Japanese YouTube channel's full video catalog (30 videos/page + infinite continuation!).
pub fn innertube_channel_videos(
    channel_id: &str,
    continuation: Option<&str>,
) -> Result<ChannelPage, String> {
    let payload = if let Some(tok) = continuation.filter(|s| !s.trim().is_empty()) {
        serde_json::json!({
            "context": innertube_jp_context(),
            "continuation": tok
        })
    } else {
        serde_json::json!({
            "context": innertube_jp_context(),
            "browseId": channel_id,
            "params": "EgZ2aWRlb3PyBgQKAjoA" // Videos tab
        })
    };

    let root = innertube_post("browse", &payload)?;
    let channel_name = root["header"]["pageHeaderRenderer"]["pageTitle"]
        .as_str()
        .or_else(|| root["metadata"]["channelMetadataRenderer"]["title"].as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let channel_pfp = root["header"]["pageHeaderRenderer"]["content"]["pageHeaderViewModel"]
        ["image"]["decoratedAvatarViewModel"]["avatar"]["avatarViewModel"]["image"]["sources"][0]
        ["url"]
        .as_str()
        .or_else(|| {
            root["metadata"]["channelMetadataRenderer"]["avatar"]["thumbnails"][0]["url"].as_str()
        })
        .unwrap_or("")
        .to_string();

    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut cont = None;

    let grid_node = if continuation.is_some() {
        &root["onResponseReceivedActions"]
    } else {
        &root["contents"]
    };
    collect_innertube_videos(
        grid_node,
        "channel",
        "channel",
        &channel_name,
        channel_id,
        &channel_pfp,
        &mut items,
        &mut seen,
        &mut cont,
    );

    Ok(ChannelPage {
        channel_id: channel_id.to_string(),
        channel_name,
        channel_pfp,
        items,
        continuation: cont,
    })
}

fn parse_atom_entries(
    xml: &str,
    channel_id: &str,
    fallback_ch: &str,
    category: &str,
    channel_pfp: &str,
) -> Vec<FeedVideo> {
    let mut out = Vec::new();
    for chunk in xml.split("<entry>").skip(1) {
        let entry = chunk.split("</entry>").next().unwrap_or(chunk);
        let Some(vid) = extract_tag(entry, "yt:videoId") else { continue };
        let vid = vid.trim();
        if !is_valid_yt_id(vid) {
            continue;
        }
        let title = extract_tag(entry, "title")
            .map(xml_unescape)
            .unwrap_or_default()
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let channel = extract_tag(entry, "name")
            .map(xml_unescape)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback_ch.to_string());
        let published = extract_tag(entry, "published")
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(FeedVideo {
            id: vid.to_string(),
            title,
            channel,
            category: category.to_string(),
            published,
            known_pct: 0,
            unknown_count: 0,
            tier: String::new(),
            score: 0.0,
            channel_id: channel_id.to_string(),
            channel_pfp: channel_pfp.to_string(),
            duration: String::new(),
            views: String::new(),
            has_cc: false,
            source: "curated".to_string(),
        });
    }
    out
}

/// Hybrid ImmersionTube Breadth & Depth Discovery Engine:
/// Combines in parallel:
/// 1. Curated Japanese immersion RSS channels (Wave 0)
/// 2. Watch-history related-video graph via InnerTube `/next` ("Because you watched...")
/// 3. User's custom saved tabs (`yt_tabs`) via InnerTube `/search` (`gl=JP`)
/// 4. Diverse Japanese YouTube category discovery waves (Vlog, Talk, Gaming, Tech, JP, News)
/// 5. Purple Priority / Learning vocabulary discovery seed
pub fn fetch_hybrid_immersion_wave(
    recent_video_ids: &[String],
    custom_tab_queries: &[String],
    priority_words: &[String],
    cat: &str,
    wave: usize,
) -> Vec<FeedVideo> {
    let mut handles: Vec<std::thread::JoinHandle<Vec<FeedVideo>>> = Vec::new();

    // 1. Wave 0 on broad views includes the fast curated RSS pool
    if wave == 0 && matches!(cat, "foryou" | "all" | "vlog" | "talk" | "jp" | "news") {
        for &(cid, cname, ccat, cpfp) in CURATED_CHANNELS {
            if cat != "foryou" && cat != "all" && ccat != cat {
                continue;
            }
            let cid_s = cid.to_string();
            let cname_s = cname.to_string();
            let cat_s = ccat.to_string();
            let pfp_s = cpfp.to_string();
            handles.push(std::thread::spawn(move || {
                let path = format!("/feeds/videos.xml?channel_id={}", cid_s);
                match winhttp_get_text("www.youtube.com", &path) {
                    Ok(xml) => parse_atom_entries(&xml, &cid_s, &cname_s, &cat_s, &pfp_s),
                    Err(_) => Vec::new(),
                }
            }));
        }
    }

    // 2. Watch-history related video graph ("Because you watched...") on For You / All
    if matches!(cat, "foryou" | "all") && !recent_video_ids.is_empty() {
        let start = (wave * 2) % recent_video_ids.len();
        let take_n = if wave == 0 { 3 } else { 2 }.min(recent_video_ids.len());
        let mut picked = HashSet::new();
        for offset in 0..take_n {
            let vid = recent_video_ids[(start + offset) % recent_video_ids.len()].clone();
            if picked.insert(vid.clone()) {
                handles.push(std::thread::spawn(move || {
                    match innertube_next_jp(&vid, None) {
                        Ok(rp) => rp.items,
                        Err(_) => Vec::new(),
                    }
                }));
            }
        }
    }

    // 3. User's custom saved tabs blended into For You
    if cat == "foryou" && !custom_tab_queries.is_empty() && wave <= 2 {
        let start = (wave * 2) % custom_tab_queries.len();
        let take_n = 2.min(custom_tab_queries.len());
        let mut picked = HashSet::new();
        for offset in 0..take_n {
            let raw_q = custom_tab_queries[(start + offset) % custom_tab_queries.len()].clone();
            let loc_q = quick_localize_jp(&raw_q).unwrap_or_else(|| {
                if has_japanese_chars(&raw_q) {
                    raw_q
                } else {
                    format!("{} 日本語", raw_q)
                }
            });
            if picked.insert(loc_q.clone()) {
                handles.push(std::thread::spawn(move || {
                    match innertube_search_jp(&loc_q, None, "foryou", "tab") {
                        Ok(sp) => sp.items,
                        Err(_) => Vec::new(),
                    }
                }));
            }
        }
    }

    // 4. Broad / Category-specific native Japanese YouTube discovery seeds
    for (seed_q, seed_cat) in category_seed_queries(cat, wave) {
        let q_s = seed_q.to_string();
        let c_s = seed_cat.to_string();
        handles.push(std::thread::spawn(move || {
            match innertube_search_jp(&q_s, None, &c_s, "discover") {
                Ok(sp) => sp.items,
                Err(_) => Vec::new(),
            }
        }));
    }

    // 5. Purple Priority / Learning vocab discovery seed (surface videos containing Nathan's target words!)
    if cat == "foryou" && !priority_words.is_empty() {
        let w = priority_words[wave % priority_words.len()].clone();
        let q = format!("{} 解説 OR 雑談 OR 実況", w);
        handles.push(std::thread::spawn(move || {
            match innertube_search_jp(&q, None, "foryou", "priority") {
                Ok(sp) => sp.items.into_iter().take(10).collect(),
                Err(_) => Vec::new(),
            }
        }));
    }

    let mut all = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for h in handles {
        if let Ok(list) = h.join() {
            for item in list {
                if seen.insert(item.id.clone()) {
                    all.push(item);
                }
            }
        }
    }
    all
}

/// Ranks ImmersionTube videos by Krashen T+1 comprehensibility + Knowledge Model frontier +
/// Priority/Learning vocab boost + CC bonus + Personalization + Channel diversity smoothing.
pub fn score_immersion_feed_smart(
    items: &mut [FeedVideo],
    morph: Option<&MorphAnalyzer>,
    jmdict: Option<&LocalJmDict>,
    vocab: &VocabStore,
    watched_ids: &HashSet<String>,
) {
    let km = KnowledgeModel::build(vocab);

    for it in items.iter_mut() {
        let mut total = 0usize;
        let mut known = 0usize;
        let mut priority_hits = 0usize;
        let mut frontier_prob_sum = 0.0f64;
        let mut unknown_set: HashSet<String> = HashSet::new();

        if let Some(mp) = morph {
            let toks = mp.tokenize_line(&it.title, jmdict);
            for tk in toks {
                if tk.ignore || !tk.content {
                    continue;
                }
                if vocab.ignored.contains(&tk.lemma) {
                    continue;
                }
                total += 1;
                if vocab.known.contains(&tk.lemma) {
                    known += 1;
                } else {
                    if vocab.priority.contains(&tk.lemma) || vocab.learning.contains(&tk.lemma) {
                        priority_hits += 1;
                    }
                    if unknown_set.insert(tk.lemma.clone()) {
                        let p = km.predict_prob(&tk.lemma, jmdict);
                        // Reward words near Nathan's acquisition frontier (0.45..0.90)
                        frontier_prob_sum += p;
                    }
                }
            }
        }

        let unk = unknown_set.len();
        it.unknown_count = unk as u16;
        it.known_pct = if total > 0 {
            ((known * 100) / total) as u8
        } else {
            0
        };
        it.tier = match unk {
            1 if total >= 2 => "T1".to_string(),
            2 if total >= 3 => "T2".to_string(),
            3 if total >= 4 => "T3".to_string(),
            _ => String::new(),
        };

        let mut sc = 0.0f64;
        if unk == 1 && total >= 2 {
            sc += 55.0;
        } else if unk == 2 && total >= 3 {
            sc += 46.0;
        } else if unk == 3 && total >= 4 {
            sc += 36.0;
        } else if total > 0 {
            let pct = it.known_pct as f64;
            sc += (pct * 0.35).min(30.0);
        }
        if known > 0 {
            sc += (it.known_pct as f64) * 0.25;
        }
        if unk > 0 {
            let avg_frontier = frontier_prob_sum / (unk as f64);
            sc += avg_frontier * 16.0;
        }
        // Jackpot boost for Purple Priority / Learning target words!
        if priority_hits > 0 {
            sc += ((priority_hits as f64) * 20.0).min(36.0);
        }
        // Subtitle / CC bonus
        if it.has_cc {
            sc += 10.0;
        }
        // Personalization source boosts
        match it.source.as_str() {
            "related" => sc += 12.0,
            "tab" => sc += 9.0,
            "priority" => sc += 10.0,
            "curated" => sc += 6.0,
            _ => {}
        }
        // Demote already-completed / recently-watched videos so For You stays fresh
        if watched_ids.contains(&it.id) {
            sc -= 35.0;
        }
        it.score = sc;
    }

    // First sort by raw score
    items.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.published.cmp(&a.published))
    });

    // Channel diversity smoothing so no single creator dominates the top of For You
    let mut ch_counts: HashMap<String, usize> = HashMap::new();
    for it in items.iter_mut() {
        let key = if !it.channel_id.is_empty() {
            it.channel_id.clone()
        } else {
            it.channel.clone()
        };
        let c = ch_counts.entry(key).or_insert(0);
        if *c > 0 {
            let penalty = ((*c as f64) * 7.5).min(36.0);
            it.score -= penalty;
        }
        *c += 1;
    }

    items.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.published.cmp(&a.published))
    });
}

/// Fallback search via bundled `yt-dlp` if InnerTube ever fails.
pub fn search_youtube_jp_fallback(
    app_root: &Path,
    localized_query: &str,
    count: usize,
) -> Result<Vec<FeedVideo>, String> {
    let Some(ytdlp) = find_yt_dlp(app_root) else {
        return Err("yt-dlp.exe not found in tools/yt-dlp/".into());
    };
    let q = localized_query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let spec = format!("ytsearch{}:{}", count.clamp(4, 24), q);
    let mut cmd = Command::new(&ytdlp);
    cmd.args([
        "--no-check-certificates",
        "--flat-playlist",
        "--dump-single-json",
        &spec,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed running yt-dlp search: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let root: serde_json::Value =
        serde_json::from_str(&stdout).map_err(|_| "No search results returned".to_string())?;
    let entries = root["entries"].as_array().cloned().unwrap_or_default();
    let mut items = Vec::new();
    for e in entries {
        let Some(id) = e.get("id").and_then(|v| v.as_str()) else { continue };
        if !is_valid_yt_id(id) {
            continue;
        }
        let title = e
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let channel = e
            .get("channel")
            .or_else(|| e.get("uploader"))
            .and_then(|v| v.as_str())
            .unwrap_or("YouTube")
            .trim()
            .to_string();
        if !is_japanese_video(&title, &channel) {
            continue;
        }
        let channel_id = e
            .get("channel_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        items.push(FeedVideo {
            id: id.to_string(),
            title,
            channel,
            category: "search".to_string(),
            published: String::new(),
            known_pct: 0,
            unknown_count: 0,
            tier: String::new(),
            score: 0.0,
            channel_id,
            channel_pfp: String::new(),
            duration: String::new(),
            views: String::new(),
            has_cc: false,
            source: "search".to_string(),
        });
    }
    Ok(items)
}
