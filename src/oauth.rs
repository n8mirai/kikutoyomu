use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Deserialize;

use crate::models::GoogleOAuthSecrets;

pub const GOOGLE_SCOPES: &str =
    "openid email https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/generative-language.retriever";

#[derive(Debug, Clone)]
pub struct PendingOAuth {
    pub state: String,
    pub code_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
pub enum AiAuth {
    GeminiCli(PathBuf),
    Bearer(String),
    ApiKey(String),
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pure-Rust SHA-256 (FIPS 180-4) — zero external crates, used for PKCE S256 & future signing.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1);
            d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

fn random_url_token(salt: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let stack_addr = (&nanos as *const u128) as usize;
    let seed = format!("{}:{}:{}:{}", nanos, pid, stack_addr, salt);
    let h1 = sha256(seed.as_bytes());
    let h2 = sha256(&h1);
    let mut raw = [0u8; 32];
    for i in 0..32 {
        raw[i] = h1[i] ^ h2[(i + 7) % 32];
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn start_google_pkce(client_id: &str, redirect_uri: &str) -> (PendingOAuth, String) {
    let state = random_url_token("kiku-oauth-state");
    let code_verifier = format!("{}{}", random_url_token("pkce-a"), random_url_token("pkce-b"));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha256(code_verifier.as_bytes()));

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&state={}&code_challenge={}&code_challenge_method=S256",
        url_encode(client_id.trim()),
        url_encode(redirect_uri),
        url_encode(GOOGLE_SCOPES),
        url_encode(&state),
        url_encode(&challenge),
    );
    (
        PendingOAuth {
            state,
            code_verifier,
            redirect_uri: redirect_uri.to_string(),
        },
        auth_url,
    )
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    id_token: Option<String>,
}

fn extract_email_from_id_token(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let val: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    val.get("email")?.as_str().map(|s| s.to_string())
}

#[cfg(windows)]
fn winhttp_post_form(host_s: &str, path_s: &str, form_body: &str) -> Result<(u16, String), String> {
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

    let agent = to_wide("kiku-jp-immersion/0.1");
    let host = to_wide(host_s);
    let verb = to_wide("POST");
    let path = to_wide(path_s);

    let session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
    });
    if session.0.is_null() {
        return Err(format!("WinHttpOpen failed ({})", unsafe { GetLastError() }));
    }
    unsafe { WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 20_000); }

    let connect = Handle(unsafe {
        WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0)
    });
    if connect.0.is_null() {
        return Err(format!("WinHttpConnect failed ({})", unsafe { GetLastError() }));
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
        return Err(format!("WinHttpOpenRequest failed ({})", unsafe { GetLastError() }));
    }

    let headers = to_wide("Content-Type: application/x-www-form-urlencoded\r\nAccept: application/json\r\n");
    let bytes = form_body.as_bytes();
    let ok = unsafe {
        WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            bytes.as_ptr() as *mut c_void,
            bytes.len() as u32,
            bytes.len() as u32,
            0,
        )
    };
    if ok == 0 {
        return Err(format!("OAuth HTTP send failed ({})", unsafe { GetLastError() }));
    }
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(format!("OAuth HTTP receive failed ({})", unsafe { GetLastError() }));
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
fn winhttp_post_form(_host_s: &str, _path_s: &str, _form_body: &str) -> Result<(u16, String), String> {
    Err("OAuth token exchange requires Windows WinHTTP".into())
}

pub fn exchange_google_code(
    secrets: &mut GoogleOAuthSecrets,
    code: &str,
    pending: &PendingOAuth,
) -> Result<(), String> {
    let mut form = format!(
        "code={}&client_id={}&redirect_uri={}&code_verifier={}&grant_type=authorization_code",
        url_encode(code),
        url_encode(secrets.client_id.trim()),
        url_encode(&pending.redirect_uri),
        url_encode(&pending.code_verifier),
    );
    if !secrets.client_secret.trim().is_empty() {
        form.push_str("&client_secret=");
        form.push_str(&url_encode(secrets.client_secret.trim()));
    }

    let (status, body) = winhttp_post_form("oauth2.googleapis.com", "/token", &form)?;
    if status < 200 || status >= 300 {
        return Err(format!("Google token exchange HTTP {}: {}", status, body.chars().take(200).collect::<String>()));
    }
    let parsed: TokenResp =
        serde_json::from_str(&body).map_err(|e| format!("Invalid token JSON: {}", e))?;
    secrets.access_token = parsed.access_token;
    if let Some(rt) = parsed.refresh_token {
        if !rt.is_empty() {
            secrets.refresh_token = rt;
        }
    }
    secrets.expires_at_unix = now_unix_secs() + parsed.expires_in.max(60);
    if let Some(idt) = parsed.id_token {
        if let Some(email) = extract_email_from_id_token(&idt) {
            secrets.email = email;
        }
    }
    Ok(())
}

pub fn refresh_google_token(secrets: &mut GoogleOAuthSecrets) -> Result<(), String> {
    if secrets.refresh_token.trim().is_empty() || secrets.client_id.trim().is_empty() {
        return Err("Google OAuth refresh token missing — please sign in again".into());
    }
    let mut form = format!(
        "client_id={}&refresh_token={}&grant_type=refresh_token",
        url_encode(secrets.client_id.trim()),
        url_encode(secrets.refresh_token.trim()),
    );
    if !secrets.client_secret.trim().is_empty() {
        form.push_str("&client_secret=");
        form.push_str(&url_encode(secrets.client_secret.trim()));
    }
    let (status, body) = winhttp_post_form("oauth2.googleapis.com", "/token", &form)?;
    if status < 200 || status >= 300 {
        return Err(format!("Google token refresh HTTP {}: {}", status, body.chars().take(200).collect::<String>()));
    }
    let parsed: TokenResp =
        serde_json::from_str(&body).map_err(|e| format!("Invalid refresh JSON: {}", e))?;
    secrets.access_token = parsed.access_token;
    secrets.expires_at_unix = now_unix_secs() + parsed.expires_in.max(60);
    Ok(())
}
