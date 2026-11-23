use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::models::{TogglProject, TogglWorkspace};

#[derive(Debug, Clone, Serialize)]
pub struct TogglTestResult {
    pub email: String,
    pub fullname: String,
    pub default_workspace_id: Option<i64>,
    pub workspaces: Vec<TogglWorkspace>,
    pub projects: Vec<TogglProject>,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
    email: Option<String>,
    fullname: Option<String>,
    default_workspace_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreatedEntry {
    id: i64,
}

fn auth_header(token: &str) -> String {
    let raw = format!("{}:api_token", token.trim());
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
    format!("Basic {}", encoded)
}

#[cfg(windows)]
fn https_request(
    method: &str,
    path: &str,
    auth: &str,
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
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    let agent = to_wide("kiku-jp-immersion/0.1");
    let host = to_wide("api.track.toggl.com");
    let verb = to_wide(method);
    let obj = to_wide(path);

    let session = Handle(unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            null(),
            null(),
            0,
        )
    });
    if session.0.is_null() {
        return Err(format!("WinHttpOpen failed (err {})", unsafe {
            GetLastError()
        }));
    }

    unsafe {
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 15_000);
    }

    let connect = Handle(unsafe {
        WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0)
    });
    if connect.0.is_null() {
        return Err(format!("WinHttpConnect failed (err {})", unsafe {
            GetLastError()
        }));
    }

    let req = Handle(unsafe {
        WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            obj.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        )
    });
    if req.0.is_null() {
        return Err(format!("WinHttpOpenRequest failed (err {})", unsafe {
            GetLastError()
        }));
    }

    let headers_str = format!(
        "Authorization: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\n",
        auth
    );
    let headers_w = to_wide(&headers_str);
    let ok_hdr = unsafe {
        WinHttpAddRequestHeaders(
            req.0,
            headers_w.as_ptr(),
            u32::MAX,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
        )
    };
    if ok_hdr == 0 {
        return Err(format!("WinHttpAddRequestHeaders failed (err {})", unsafe {
            GetLastError()
        }));
    }

    let body_bytes = body.map(|b| b.as_bytes()).unwrap_or(&[]);
    let body_ptr = if body_bytes.is_empty() {
        null_mut()
    } else {
        body_bytes.as_ptr() as *mut c_void
    };
    let body_len = body_bytes.len() as u32;

    let ok_send = unsafe {
        WinHttpSendRequest(
            req.0,
            null(),
            0,
            body_ptr,
            body_len,
            body_len,
            0,
        )
    };
    if ok_send == 0 {
        return Err(format!(
            "Network request to Toggl failed (WinHTTP err {})",
            unsafe { GetLastError() }
        ));
    }

    let ok_recv = unsafe { WinHttpReceiveResponse(req.0, null_mut()) };
    if ok_recv == 0 {
        return Err(format!(
            "No response from Toggl (WinHTTP err {})",
            unsafe { GetLastError() }
        ));
    }

    let mut status_code: u32 = 0;
    let mut status_size: u32 = std::mem::size_of::<u32>() as u32;
    let ok_status = unsafe {
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status_code as *mut u32 as *mut c_void,
            &mut status_size,
            null_mut(),
        )
    };
    if ok_status == 0 {
        return Err(format!("Failed reading status code (err {})", unsafe {
            GetLastError()
        }));
    }

    let mut out = Vec::new();
    loop {
        let mut avail: u32 = 0;
        let ok_avail = unsafe { WinHttpQueryDataAvailable(req.0, &mut avail) };
        if ok_avail == 0 {
            break;
        }
        if avail == 0 {
            break;
        }
        let mut buf = vec![0u8; avail as usize];
        let mut read: u32 = 0;
        let ok_read = unsafe {
            WinHttpReadData(
                req.0,
                buf.as_mut_ptr() as *mut c_void,
                avail,
                &mut read,
            )
        };
        if ok_read == 0 || read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
    }

    let text = String::from_utf8_lossy(&out).into_owned();
    Ok((status_code as u16, text))
}

#[cfg(not(windows))]
fn https_request(
    _method: &str,
    _path: &str,
    _auth: &str,
    _body: Option<&str>,
) -> Result<(u16, String), String> {
    Err("WinHTTP client is Windows-only".into())
}

fn check_status(status: u16, body: &str) -> Result<(), String> {
    if (200..300).contains(&status) {
        Ok(())
    } else if status == 401 || status == 403 {
        Err("Invalid Toggl API token (401/403). Check Profile Settings -> API Token in Toggl Track.".into())
    } else if status == 429 || status == 402 {
        Err(format!("Toggl API rate/quota limit (HTTP {})", status))
    } else {
        let snippet: String = body.chars().take(180).collect();
        Err(format!("Toggl API HTTP {}: {}", status, snippet.trim()))
    }
}

pub fn test_connection(token: &str) -> Result<TogglTestResult, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Enter your Toggl API token first.".into());
    }
    let auth = auth_header(token);

    let (st, body) = https_request("GET", "/api/v9/me", &auth, None)?;
    check_status(st, &body)?;
    let me: MeResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed parsing /me: {}", e))?;

    let workspaces = get_workspaces(token)?;
    let wid = me
        .default_workspace_id
        .or_else(|| workspaces.first().map(|w| w.id));
    let projects = match wid {
        Some(w) => get_projects(token, w).unwrap_or_default(),
        None => Vec::new(),
    };

    Ok(TogglTestResult {
        email: me.email.unwrap_or_default(),
        fullname: me.fullname.unwrap_or_default(),
        default_workspace_id: wid,
        workspaces,
        projects,
    })
}

pub fn get_workspaces(token: &str) -> Result<Vec<TogglWorkspace>, String> {
    let auth = auth_header(token);
    let (st, body) = https_request("GET", "/api/v9/workspaces", &auth, None)?;
    check_status(st, &body)?;
    serde_json::from_str(&body).map_err(|e| format!("Failed parsing workspaces: {}", e))
}

pub fn get_projects(token: &str, workspace_id: i64) -> Result<Vec<TogglProject>, String> {
    let auth = auth_header(token);
    let path = format!("/api/v9/workspaces/{}/projects?active=true", workspace_id);
    let (st, body) = https_request("GET", &path, &auth, None)?;
    check_status(st, &body)?;
    if body.trim() == "null" || body.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&body).map_err(|e| format!("Failed parsing projects: {}", e))
}

pub fn start_running_entry(
    token: &str,
    workspace_id: i64,
    project_id: Option<i64>,
    description: &str,
    tags: &[String],
    start_rfc3339: &str,
) -> Result<i64, String> {
    let auth = auth_header(token);
    let path = format!("/api/v9/workspaces/{}/time_entries", workspace_id);
    let mut payload = serde_json::json!({
        "workspace_id": workspace_id,
        "description": description,
        "start": start_rfc3339,
        "stop": null,
        "duration": -1,
        "tags": tags,
        "billable": false,
        "created_with": "kiku-jp-immersion"
    });
    if let Some(pid) = project_id {
        payload["project_id"] = serde_json::json!(pid);
    }
    let body_str = payload.to_string();
    let (st, body) = https_request("POST", &path, &auth, Some(&body_str))?;
    check_status(st, &body)?;
    let created: CreatedEntry =
        serde_json::from_str(&body).map_err(|e| format!("Failed parsing Toggl start: {}", e))?;
    Ok(created.id)
}

pub fn stop_running_entry(
    token: &str,
    workspace_id: i64,
    entry_id: i64,
) -> Result<(), String> {
    let auth = auth_header(token);
    let path = format!(
        "/api/v9/workspaces/{}/time_entries/{}/stop",
        workspace_id, entry_id
    );
    let (st, body) = https_request("PATCH", &path, &auth, None)?;
    check_status(st, &body)
}

pub fn delete_entry(
    token: &str,
    workspace_id: i64,
    entry_id: i64,
) -> Result<(), String> {
    let auth = auth_header(token);
    let path = format!(
        "/api/v9/workspaces/{}/time_entries/{}",
        workspace_id, entry_id
    );
    let (st, body) = https_request("DELETE", &path, &auth, None)?;
    check_status(st, &body)
}

pub fn create_completed_entry(
    token: &str,
    workspace_id: i64,
    project_id: Option<i64>,
    description: &str,
    tags: &[String],
    start_rfc3339: &str,
    stop_rfc3339: &str,
    duration_secs: u64,
) -> Result<i64, String> {
    let auth = auth_header(token);
    let path = format!("/api/v9/workspaces/{}/time_entries", workspace_id);
    let mut payload = serde_json::json!({
        "workspace_id": workspace_id,
        "description": description,
        "start": start_rfc3339,
        "stop": stop_rfc3339,
        "duration": duration_secs as i64,
        "tags": tags,
        "billable": false,
        "created_with": "kiku-jp-immersion"
    });
    if let Some(pid) = project_id {
        payload["project_id"] = serde_json::json!(pid);
    }
    let body_str = payload.to_string();
    let (st, body) = https_request("POST", &path, &auth, Some(&body_str))?;
    check_status(st, &body)?;
    let created: CreatedEntry =
        serde_json::from_str(&body).map_err(|e| format!("Failed parsing Toggl create: {}", e))?;
    Ok(created.id)
}
