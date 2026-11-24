use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::timeutil;

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
