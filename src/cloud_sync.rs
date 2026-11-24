use std::collections::BTreeMap;
use std::path::Path;

use crate::models::{HistoryEntry, UserProfile};

/// Clean URL parser for `https://host[:port]/base_path` (no external crates).
fn parse_https_url(raw: &str) -> Result<(String, u16, String, bool), String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let (secure, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("Cloud Sync URL must start with https://".into());
    };
    let (host_port, base_path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{}", p.trim_end_matches('/'))),
        None => (rest, String::new()),
    };
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().unwrap_or(if secure { 443 } else { 80 }),
        ),
        None => (host_port.to_string(), if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err("Invalid Cloud Sync host".into());
    }
    Ok((host, port, base_path, secure))
}

#[cfg(windows)]
fn winhttp_json_req(
    method: &str,
    base_url: &str,
    subpath: &str,
    secret: &str,
    body: Option<&str>,
) -> Result<(u16, String), String> {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::GetLastError;
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

    let (host_s, port, base_path, secure) = parse_https_url(base_url)?;
    let full_path = format!("{}{}", base_path, subpath);

    let agent = to_wide("kiku-jp-immersion/0.1");
    let host = to_wide(&host_s);
    let verb = to_wide(method);
    let path = to_wide(&full_path);

    let session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
    });
    if session.0.is_null() {
        return Err(format!("WinHttpOpen failed ({})", unsafe { GetLastError() }));
    }
    unsafe { WinHttpSetTimeouts(session.0, 8_000, 8_000, 15_000, 20_000); }

    let connect = Handle(unsafe { WinHttpConnect(session.0, host.as_ptr(), port, 0) });
    if connect.0.is_null() {
        return Err(format!("WinHttpConnect failed ({})", unsafe { GetLastError() }));
    }

    let flags = if secure { WINHTTP_FLAG_SECURE } else { 0 };
    let request = Handle(unsafe {
        WinHttpOpenRequest(connect.0, verb.as_ptr(), path.as_ptr(), null(), null(), null(), flags)
    });
    if request.0.is_null() {
        return Err(format!("WinHttpOpenRequest failed ({})", unsafe { GetLastError() }));
    }

    let mut hdrs = String::from("Content-Type: application/json\r\nAccept: application/json\r\n");
    if !secret.trim().is_empty() {
        hdrs.push_str(&format!("Authorization: Bearer {}\r\n", secret.trim()));
    }
    let headers_w = to_wide(&hdrs);
    let bytes = body.map(|b| b.as_bytes()).unwrap_or(&[]);
    let ptr = if bytes.is_empty() {
        null_mut()
    } else {
        bytes.as_ptr() as *mut c_void
    };

    let ok = unsafe {
        WinHttpSendRequest(
            request.0,
            headers_w.as_ptr(),
            (headers_w.len() - 1) as u32,
            ptr,
            bytes.len() as u32,
            bytes.len() as u32,
            0,
        )
    };
    if ok == 0 {
        return Err(format!("Cloud Sync send failed ({})", unsafe { GetLastError() }));
    }
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(format!("Cloud Sync receive failed ({})", unsafe { GetLastError() }));
    }

    let mut status: u32 = 0;
    let mut slen: u32 = std::mem::size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut c_void,
            &mut slen,
            null_mut(),
        );
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

    Ok((status as u16, String::from_utf8_lossy(&resp_bytes).to_string()))
}

#[cfg(not(windows))]
fn winhttp_json_req(
    _method: &str,
    _base_url: &str,
    _subpath: &str,
    _secret: &str,
    _body: Option<&str>,
) -> Result<(u16, String), String> {
    Err("Cloud sync requires Windows WinHTTP".into())
}

pub fn pull_profile(base_url: &str, secret: &str) -> Result<Option<UserProfile>, String> {
    let (status, body) = winhttp_json_req("GET", base_url, "/profile", secret, None)?;
    if status == 404 {
        return Ok(None);
    }
    if status < 200 || status >= 300 {
        return Err(format!("Cloud pull HTTP {}: {}", status, body.chars().take(160).collect::<String>()));
    }
    if body.trim().is_empty() || body.trim() == "null" {
        return Ok(None);
    }
    let prof: UserProfile =
        serde_json::from_str(&body).map_err(|e| format!("Invalid cloud profile JSON: {}", e))?;
    Ok(Some(prof))
}

pub fn push_profile(base_url: &str, secret: &str, profile: &UserProfile) -> Result<(), String> {
    let mut clean = profile.clone();
    clean.config.toggl_api_token.clear();
    clean.config.gemini_api_key.clear();
    let json = serde_json::to_string(&clean).map_err(|e| format!("Serialize error: {}", e))?;
    let (status, body) = winhttp_json_req("PUT", base_url, "/profile", secret, Some(&json))?;
    if status < 200 || status >= 300 {
        return Err(format!("Cloud push HTTP {}: {}", status, body.chars().take(160).collect::<String>()));
    }
    Ok(())
}

/// Lossless union-merge of remote cloud profile into local profile so multiple Kiku instances
/// never clobber each other's known words, listening history, or episode progress!
pub fn merge_profiles(local: &mut UserProfile, remote: UserProfile) {
    // 1. Vocab: union known, learning, ignored (known wins over learning)
    for w in remote.vocab.known {
        local.vocab.learning.remove(&w);
        local.vocab.ignored.remove(&w);
        local.vocab.priority.remove(&w);
        local.vocab.known.insert(w);
    }
    for w in remote.vocab.priority {
        if !local.vocab.known.contains(&w) && !local.vocab.ignored.contains(&w) {
            local.vocab.priority.insert(w);
        }
    }
    for w in remote.vocab.learning {
        if !local.vocab.known.contains(&w) && !local.vocab.ignored.contains(&w) {
            local.vocab.learning.insert(w);
        }
    }
    for w in remote.vocab.ignored {
        if !local.vocab.known.contains(&w) {
            local.vocab.learning.remove(&w);
            local.vocab.ignored.insert(w);
        }
    }

    // 2. History: deduplicate by id, keeping longer duration & toggl_synced=true if either synced
    let mut hist_map: BTreeMap<String, HistoryEntry> = BTreeMap::new();
    for h in local.history.drain(..) {
        hist_map.insert(h.id.clone(), h);
    }
    for rh in remote.history {
        if let Some(lh) = hist_map.get_mut(&rh.id) {
            if rh.duration_secs > lh.duration_secs {
                lh.duration_secs = rh.duration_secs;
                lh.ended_at = rh.ended_at.clone();
            }
            if rh.toggl_synced && !lh.toggl_synced {
                lh.toggl_synced = true;
                lh.toggl_entry_id = rh.toggl_entry_id;
                lh.toggl_error = None;
            }
        } else {
            hist_map.insert(rh.id.clone(), rh);
        }
    }
    let mut merged_hist: Vec<HistoryEntry> = hist_map.into_values().collect();
    merged_hist.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    local.history = merged_hist;

    // 3. Episode progress: keep higher completions & newer last_played_at position
    for (k, rp) in remote.progress {
        match local.progress.get_mut(&k) {
            Some(lp) => {
                if rp.completions > lp.completions {
                    lp.completions = rp.completions;
                }
                if rp.last_played_at > lp.last_played_at {
                    lp.position_secs = rp.position_secs;
                    lp.last_played_at = rp.last_played_at;
                    if rp.duration_secs > 0.0 {
                        lp.duration_secs = rp.duration_secs;
                    }
                }
            }
            None => {
                local.progress.insert(k, rp);
            }
        }
    }

    // 4. Custom anime metadata: union by id
    for ca in remote.custom_anime {
        if !local.custom_anime.iter().any(|x| x.id == ca.id) {
            local.custom_anime.push(ca);
        }
    }

    // 5. Playlists: union by id (keep local version if same id)
    for pl in remote.playlists {
        if !local.playlists.iter().any(|x| x.id == pl.id) {
            local.playlists.push(pl);
        }
    }
}

/// Finds the official Google `gemini` CLI (bundled portable tools/node OR global npm/PATH).
pub fn find_gemini_cli(app_root: &Path) -> Option<std::path::PathBuf> {
    let portable = app_root
        .join("tools")
        .join("node")
        .join("node_modules")
        .join(".bin")
        .join("gemini.cmd");
    if portable.exists() {
        return Some(portable);
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let global = Path::new(&appdata).join("npm").join("gemini.cmd");
        if global.exists() {
            return Some(global);
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in ["gemini.cmd", "gemini.exe", "gemini"] {
                let p = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Checks whether the user has already signed into Google via the official `gemini` CLI.
pub fn is_gemini_cli_signed_in() -> bool {
    let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) else {
        return false;
    };
    let gdir = Path::new(&home).join(".gemini");
    gdir.join("oauth_creds.json").exists() || gdir.join("google_accounts.json").exists()
}
