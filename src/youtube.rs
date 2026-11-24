use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::dict::LocalJmDict;
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
}

const CURATED_CHANNELS: &[(&str, &str, &str)] = &[
    ("UC8dWfySP_cKDMFj6aFfQbFA", "YUYUの日本語Podcast", "talk"),
    ("UCh-GhnQ7qDQmS6Bz3pGc1Mw", "あかね的日本語教室", "vlog"),
    ("UCSbH_BPR_AoARW6RDYLlLog", "Speak Japanese Naturally", "vlog"),
    ("UCB_Bvgg4oCHEAzxvZju4S3A", "Bite Size Japanese", "talk"),
    ("UCaXIrZ5Ere4lC4cdPE_ujXA", "けんさんおかえり", "vlog"),
    ("UCmpkIzF3xFzhPez7gXOyhVg", "ゆる言語学ラジオ", "talk"),
    ("UCgMPP6RRjktV7krOfyUewqw", "はじめしゃちょー", "jp"),
    ("UCFbp2XdRpKfk7mYt_uT8dxw", "Kevin's English Room", "jp"),
    ("UCGCZAYq5Xxojl_tSXcVJhiQ", "ANNnewsCH", "news"),
];

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

fn is_valid_yt_id(s: &str) -> bool {
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
    // Check ?v= or &v=
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
    // Check youtu.be/<id>, /shorts/<id>, /embed/<id>, /live/<id>
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
    // Scan directory for any file starting with video_id + "."
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
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
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
fn winhttp_get_text(host_s: &str, path_s: &str) -> Result<String, String> {
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

    let agent = to_wide("kiku-jp-immersion/0.1");
    let host = to_wide(host_s);
    let verb = to_wide("GET");
    let path = to_wide(path_s);

    let session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
    });
    if session.0.is_null() {
        return Err("WinHttpOpen failed".into());
    }
    unsafe { WinHttpSetTimeouts(session.0, 4_000, 4_000, 5_000, 6_000); }

    let connect = Handle(unsafe {
        WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0)
    });
    if connect.0.is_null() {
        return Err("WinHttpConnect failed".into());
    }

    let request = Handle(unsafe {
        WinHttpOpenRequest(connect.0, verb.as_ptr(), path.as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE)
    });
    if request.0.is_null() {
        return Err("WinHttpOpenRequest failed".into());
    }

    if unsafe { WinHttpSendRequest(request.0, null(), 0, null_mut(), 0, 0, 0) } == 0 {
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
        if unsafe { WinHttpReadData(request.0, chunk.as_mut_ptr() as *mut c_void, avail, &mut read) } == 0
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
fn winhttp_get_text(_host_s: &str, _path_s: &str) -> Result<String, String> {
    Err("Windows WinHTTP required".into())
}

fn parse_atom_entries(xml: &str, fallback_ch: &str, category: &str) -> Vec<FeedVideo> {
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
        });
    }
    out
}

/// Fetches fresh videos from all curated Japanese ImmersionTube channels in parallel.
pub fn fetch_raw_immersion_feed() -> Vec<FeedVideo> {
    let mut handles = Vec::new();
    for &(cid, cname, cat) in CURATED_CHANNELS {
        let cid_s = cid.to_string();
        let cname_s = cname.to_string();
        let cat_s = cat.to_string();
        handles.push(std::thread::spawn(move || {
            let path = format!("/feeds/videos.xml?channel_id={}", cid_s);
            match winhttp_get_text("www.youtube.com", &path) {
                Ok(xml) => parse_atom_entries(&xml, &cname_s, &cat_s),
                Err(_) => Vec::new(),
            }
        }));
    }
    let mut all = Vec::new();
    for h in handles {
        if let Ok(mut list) = h.join() {
            all.append(&mut list);
        }
    }
    all
}

/// Ranks ImmersionTube videos by Krashen T+1 comprehensibility against the user's live VocabStore.
pub fn score_immersion_feed(
    items: &mut [FeedVideo],
    morph: Option<&MorphAnalyzer>,
    jmdict: Option<&LocalJmDict>,
    vocab: &VocabStore,
) {
    for it in items.iter_mut() {
        let mut total = 0usize;
        let mut known = 0usize;
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
                    unknown_set.insert(tk.lemma);
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

        // T+1 HackerTube scoring: reward 1-3 unknown content words + high comprehensibility + CI/talk clarity
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
        if it.category == "vlog" || it.category == "talk" {
            sc += 8.0;
        }
        it.score = sc;
    }

    items.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.published.cmp(&a.published))
    });
}

fn has_japanese_chars(s: &str) -> bool {
    s.chars().any(|c| {
        ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c)
    })
}

/// Searches YouTube for Japanese immersion content via bundled `yt-dlp` (`ytsearchN:...`).
pub fn search_youtube_jp(
    app_root: &Path,
    raw_query: &str,
    count: usize,
) -> Result<Vec<FeedVideo>, String> {
    let Some(ytdlp) = find_yt_dlp(app_root) else {
        return Err("yt-dlp.exe not found in tools/yt-dlp/".into());
    };
    let q = raw_query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let search_q = if has_japanese_chars(q) {
        q.to_string()
    } else {
        format!("{} 日本語", q)
    };
    let spec = format!("ytsearch{}:{}", count.clamp(4, 20), search_q);
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
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
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
        });
    }
    Ok(items)
}
