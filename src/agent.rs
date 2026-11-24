use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::oauth::AiAuth;
use crate::subs::SubtitleCue;
use crate::synopsis::EpisodeSynopsis;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub role: String, // "user" | "model"
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentAction {
    pub tool: String,
    pub summary: String,
}

#[cfg(windows)]
fn winhttp_get_bytes(host_s: &str, path_s: &str) -> Result<Vec<u8>, String> {
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
    if status < 200 || status >= 300 {
        return Err(format!("HTTP {}", status));
    }

    let mut out = Vec::new();
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
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(not(windows))]
fn winhttp_get_bytes(_host_s: &str, _path_s: &str) -> Result<Vec<u8>, String> {
    Err("Windows WinHTTP required".into())
}

#[cfg(windows)]
pub fn gemini_generate_raw(
    auth: &AiAuth,
    model: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
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

    let (path_str, hdr_str) = match auth {
        AiAuth::ApiKey(key) => (
            format!("/v1beta/models/{}:generateContent?key={}", model, key.trim()),
            "Content-Type: application/json\r\nAccept: application/json\r\n".to_string(),
        ),
        AiAuth::Bearer(tok) => (
            format!("/v1beta/models/{}:generateContent", model),
            format!(
                "Content-Type: application/json\r\nAccept: application/json\r\nAuthorization: Bearer {}\r\n",
                tok.trim()
            ),
        ),
        AiAuth::GeminiCli(_) => {
            return Err("Add a Gemini API key in Settings (Google deprecated individual Gemini CLI login)".into());
        }
    };

    let req_body = payload.to_string();
    let agent = to_wide("kiku-jp-immersion/0.1");
    let host = to_wide("generativelanguage.googleapis.com");
    let verb = to_wide("POST");
    let path = to_wide(&path_str);

    let session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
    });
    if session.0.is_null() {
        return Err(format!("WinHttpOpen failed ({})", unsafe { GetLastError() }));
    }
    unsafe { WinHttpSetTimeouts(session.0, 6_000, 6_000, 12_000, 18_000); }

    let connect = Handle(unsafe {
        WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0)
    });
    if connect.0.is_null() {
        return Err(format!("WinHttpConnect failed ({})", unsafe { GetLastError() }));
    }
    let request = Handle(unsafe {
        WinHttpOpenRequest(connect.0, verb.as_ptr(), path.as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE)
    });
    if request.0.is_null() {
        return Err(format!("WinHttpOpenRequest failed ({})", unsafe { GetLastError() }));
    }

    let headers = to_wide(&hdr_str);
    let bytes = req_body.as_bytes();
    if unsafe {
        WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            bytes.as_ptr() as *mut c_void,
            bytes.len() as u32,
            bytes.len() as u32,
            0,
        )
    } == 0
    {
        return Err(format!("Gemini send failed ({})", unsafe { GetLastError() }));
    }
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(format!("Gemini receive failed ({})", unsafe { GetLastError() }));
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

    let resp_str = String::from_utf8_lossy(&resp_bytes).to_string();
    if status < 200 || status >= 300 {
        return Err(format!("Gemini HTTP {}: {}", status, resp_str.chars().take(200).collect::<String>()));
    }
    serde_json::from_str(&resp_str).map_err(|e| format!("Invalid Gemini JSON: {}", e))
}

#[cfg(not(windows))]
pub fn gemini_generate_raw(
    _auth: &AiAuth,
    _model: &str,
    _payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Err("Windows WinHTTP required".into())
}

/// Resilient modern Gemini Flash cascade: fails over cleanly across timeouts (12002), 503, 429, and 404.
pub fn gemini_generate_flash(
    auth: &AiAuth,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let models = [
        "gemini-flash-latest",
        "gemini-3.7-flash",
        "gemini-3.6-flash",
        "gemini-3.5-flash",
        "gemini-3.5-flash-lite",
    ];
    let mut last_err = String::from("No Gemini model succeeded");
    for m in models {
        match gemini_generate_raw(auth, m, payload) {
            Ok(v) => return Ok(v),
            Err(e) => {
                let retryable = e.contains("HTTP 503")
                    || e.contains("HTTP 404")
                    || e.contains("HTTP 429")
                    || e.contains("12002")
                    || e.contains("UNAVAILABLE");
                last_err = e;
                if !retryable {
                    return Err(last_err);
                }
            }
        }
    }
    Err(last_err)
}

/// Suggests a clean 1-2 word broad category label when a user's YT search query is messy/long.
pub fn suggest_search_category(auth: &AiAuth, raw_query: &str) -> Option<String> {
    let payload = serde_json::json!({
        "contents": [{ "parts": [{ "text": format!(
            "Convert this YouTube search query into a clean 1-2 word category tab name (e.g. Gaming, Minecraft, Cooking, Tech, Vlog, Anime, Podcast, Travel). Return ONLY the short label, nothing else.\nQuery: {}",
            raw_query
        ) }] }],
        "generationConfig": { "temperature": 0.2, "maxOutputTokens": 16 }
    });
    let root = gemini_generate_flash(auth, &payload).ok()?;
    let txt = root["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()?
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    if txt.is_empty() || txt.len() > 24 {
        None
    } else {
        Some(txt)
    }
}

/// Converts a non-Japanese search query into a natural native Japanese YouTube search phrase
/// (e.g. "horror games" -> "ホラーゲーム 実況", "street food" -> "屋台 グルメ 日本").
pub fn localize_search_query_jp(auth: &AiAuth, raw_query: &str) -> Option<String> {
    let payload = serde_json::json!({
        "contents": [{ "parts": [{ "text": format!(
            "Translate/adapt this search query into the most natural 1-3 word Japanese YouTube search query that a native Japanese viewer would type to find Japanese videos on this topic. Return ONLY the Japanese search words, no quotes, no explanation.\nQuery: {}",
            raw_query
        ) }] }],
        "generationConfig": { "temperature": 0.2, "maxOutputTokens": 24 }
    });
    let root = gemini_generate_flash(auth, &payload).ok()?;
    let txt = root["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()?
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if txt.is_empty() || txt.chars().count() > 40 {
        None
    } else {
        Some(txt)
    }
}

/// Token-efficient multimodal YouTube synopsis generator:
/// Intakes compact timestamped Japanese transcript + 4 YouTube keyframe JPEGs (`hqdefault`, `hq1`, `hq2`, `hq3`)
/// using `gemini-2.5-flash-lite` (~$0.0005 per video, permanently cached in `synopses.json`!).
pub fn generate_yt_multimodal_synopsis(
    auth: &AiAuth,
    video_id: &str,
    channel: &str,
    title: &str,
    cues: &[SubtitleCue],
) -> Result<EpisodeSynopsis, String> {
    if cues.is_empty() {
        return Err("No subtitles available for this video".into());
    }

    // Compact transcript capped at ~10,000 chars for ultra-high token efficiency
    let mut transcript = String::new();
    for c in cues {
        let line = format!("[{}s] {}\n", c.start as u64, c.text.trim());
        if transcript.len() + line.len() > 10_000 {
            break;
        }
        transcript.push_str(&line);
    }

    // Fetch up to 4 lightweight YouTube keyframes in parallel (hqdefault, hq1, hq2, hq3)
    let names = ["hqdefault.jpg", "hq1.jpg", "hq2.jpg", "hq3.jpg"];
    let mut handles = Vec::new();
    for name in names {
        let path = format!("/vi/{}/{}", video_id, name);
        handles.push(std::thread::spawn(move || winhttp_get_bytes("i.ytimg.com", &path)));
    }
    let mut parts: Vec<serde_json::Value> = Vec::new();
    for h in handles {
        if let Ok(Ok(bytes)) = h.join() {
            if bytes.len() > 1000 && bytes.len() < 150_000 {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                parts.push(serde_json::json!({
                    "inlineData": { "mimeType": "image/jpeg", "data": b64 }
                }));
            }
        }
    }

    let prompt = format!(
        "You are an English immersion companion for a Japanese learner watching \"{}\" on channel \"{}\".\n\
        Using the keyframes and timestamped Japanese transcript below, return ONLY valid JSON (no markdown fences):\n\
        {{\"episode_summary\":\"2-3 concise English sentences summarizing this video (incorporating visual context from the frames).\",\n\
          \"series_summary\":\"1-2 concise English sentences describing this channel's style and vibe.\",\n\
          \"beats\":[{{\"start\":0,\"text\":\"One short evocative present-tense English line (<= 90 chars) describing what is happening right now\"}}]}}\n\
        Generate one beat roughly every 30-50 seconds covering the transcript.\n\nTranscript:\n{}",
        title, channel, transcript
    );
    parts.push(serde_json::json!({ "text": prompt }));

    let payload = serde_json::json!({
        "contents": [{ "parts": parts }],
        "generationConfig": {
            "temperature": 0.3,
            "responseMimeType": "application/json"
        }
    });

    // Resilient modern Gemini 3.x Flash cascade (multimodal keyframes + transcript, cached permanently!)
    let root = gemini_generate_flash(auth, &payload)?;

    let text = root["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| "Empty Gemini response".to_string())?;
    let clean = match (text.find('{'), text.rfind('}')) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => text.trim(),
    };
    let mut parsed: EpisodeSynopsis =
        serde_json::from_str(clean).map_err(|e| format!("Failed parsing YT synopsis JSON: {}", e))?;
    parsed.ai_generated = true;
    parsed.series_title = channel.to_string();
    parsed.beats.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(parsed)
}

/// Returns the MCP-compatible Gemini tool declarations for the Kiku Immersion Agent.
pub fn tool_declarations() -> serde_json::Value {
    serde_json::json!([{
        "functionDeclarations": [
            {
                "name": "mark_vocab",
                "description": "Update a Japanese word's knowledge status in the user's Kiku VocabStore (known, learning, unknown, ignore).",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "word": { "type": "STRING", "description": "Canonical Japanese dictionary form (lemma), e.g. 食べる or 無限" },
                        "status": { "type": "STRING", "enum": ["known", "learning", "unknown", "ignore"] }
                    },
                    "required": ["word", "status"]
                }
            },
            {
                "name": "suggest_calibration_words",
                "description": "Run Kiku's probabilistic knowledge model to find untagged Japanese words the user probably already knows, so you can ask them to confirm.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "count": { "type": "INTEGER", "description": "Number of candidate words to return (1-12)" }
                    }
                }
            }
        ]
    }])
}
