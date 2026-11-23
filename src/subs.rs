use std::fs;
use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub fn is_subtitle_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "srt" | "vtt" | "ass" | "ssa")
}

fn parse_timestamp(raw: &str) -> Option<f64> {
    let s = raw.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].trim().parse().ok()?;
            let m: f64 = parts[1].trim().parse().ok()?;
            let sec: f64 = parts[2].trim().parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        2 => {
            let m: f64 = parts[0].trim().parse().ok()?;
            let sec: f64 = parts[1].trim().parse().ok()?;
            Some(m * 60.0 + sec)
        }
        _ => None,
    }
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn strip_ass_overrides(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_brace = false;
    for ch in s.chars() {
        match ch {
            '{' => in_brace = true,
            '}' => in_brace = false,
            _ if !in_brace => out.push(ch),
            _ => {}
        }
    }
    out.replace("\\N", "\n")
        .replace("\\n", "\n")
        .replace("\\h", " ")
        .trim()
        .to_string()
}

fn parse_srt_or_vtt(content: &str) -> Vec<SubtitleCue> {
    let normalized = content
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        // Find the line containing "-->"
        let Some(arrow_idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let timing = lines[arrow_idx];
        let Some((left, right)) = timing.split_once("-->") else {
            continue;
        };
        let right_clean = right.trim().split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (parse_timestamp(left), parse_timestamp(right_clean)) else {
            continue;
        };
        let text = lines[arrow_idx + 1..]
            .iter()
            .map(|l| strip_html_tags(l))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if !text.is_empty() {
            cues.push(SubtitleCue { start, end, text });
        }
    }

    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

fn parse_ass(content: &str) -> Vec<SubtitleCue> {
    let mut cues = Vec::new();
    let normalized = content.trim_start_matches('\u{feff}');
    for line in normalized.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("Dialogue:") else {
            continue;
        };
        // Standard ASS Dialogue has 10 comma-separated fields; text is field 9+
        let mut fields = rest.splitn(10, ',');
        let _layer = fields.next();
        let start_s = fields.next().unwrap_or("");
        let end_s = fields.next().unwrap_or("");
        let _style = fields.next();
        let _name = fields.next();
        let _ml = fields.next();
        let _mr = fields.next();
        let _mv = fields.next();
        let _effect = fields.next();
        let text_raw = fields.next().unwrap_or("");

        let (Some(start), Some(end)) = (parse_timestamp(start_s), parse_timestamp(end_s)) else {
            continue;
        };
        let text = strip_ass_overrides(text_raw);
        if !text.is_empty() {
            cues.push(SubtitleCue { start, end, text });
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

pub fn load_subtitles(path: &Path) -> Result<Vec<SubtitleCue>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("Failed reading subtitle file: {}", e))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let cues = if ext == "ass" || ext == "ssa" {
        parse_ass(&raw)
    } else {
        parse_srt_or_vtt(&raw)
    };
    Ok(cues)
}
