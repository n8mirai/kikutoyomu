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
    dedupe_rolling_captions(cues)
}

/// Cleans YouTube auto-generated rolling WebVTT cues where consecutive cues repeat the previous
/// line as a 2-line window (e.g. "A" -> "A\nB" -> "B\nC") or emit 10ms duplicate micro-cues.
fn dedupe_rolling_captions(raw: Vec<SubtitleCue>) -> Vec<SubtitleCue> {
    let mut out: Vec<SubtitleCue> = Vec::with_capacity(raw.len());
    let mut prev_line = String::new();

    for cue in raw {
        let mut kept_lines: Vec<String> = Vec::new();
        for line in cue.text.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if t == prev_line {
                continue;
            }
            kept_lines.push(t.to_string());
        }
        let cleaned = kept_lines.join(" ").trim().to_string();
        if cleaned.is_empty() {
            if let Some(last) = out.last_mut() {
                if cue.end > last.end {
                    last.end = cue.end;
                }
            }
            continue;
        }
        if let Some(last_kept) = kept_lines.last() {
            prev_line = last_kept.clone();
        }
        // Skip ultra-short duplicate/prefix flashes (<0.08s) when identical to previous cue
        if let Some(last) = out.last_mut() {
            if last.text == cleaned {
                if cue.end > last.end {
                    last.end = cue.end;
                }
                continue;
            }
            if cleaned.starts_with(&last.text) && (cue.start - last.start).abs() < 0.25 {
                last.text = cleaned;
                if cue.end > last.end {
                    last.end = cue.end;
                }
                continue;
            }
        }
        out.push(SubtitleCue {
            start: cue.start,
            end: cue.end,
            text: cleaned,
        });
    }
    out
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
    Ok(normalize_japanese_cues(cues))
}

fn ends_with_sentence_terminal(s: &str) -> bool {
    let Some(c) = s.trim_end().chars().last() else {
        return true;
    };
    matches!(c, '。' | '！' | '？' | '」' | '』' | '…' | '!' | '?' | '.')
}

fn is_jp_continuation_start(s: &str) -> bool {
    let Some(c) = s.trim_start().chars().next() else {
        return false;
    };
    matches!(
        c,
        'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'ー'
            | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ'
            | 'て' | 'で' | 'た' | 'だ' | 'る' | 'れ' | 'せ' | 'さ' | 'な'
            | 'に' | 'の' | 'を' | 'が' | 'は' | 'も' | 'と' | 'へ' | 'か'
            | 'よ' | 'ね' | 'わ' | '。' | '、' | '」' | '』' | '）'
    )
}

fn is_kanji_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || c == '々'
}

fn is_kana_char(c: char) -> bool {
    ('\u{3040}'..='\u{30FF}').contains(&c) || c == 'ー'
}

fn should_stitch(cur: &SubtitleCue, next: &SubtitleCue) -> bool {
    let gap = next.start - cur.end;
    if gap > 1.25 {
        return false;
    }
    let cur_t = cur.text.trim_end();
    let next_t = next.text.trim_start();
    if cur_t.is_empty() || next_t.is_empty() {
        return false;
    }
    if ends_with_sentence_terminal(cur_t) {
        return false;
    }
    let cur_len = cur_t.chars().count();
    let next_len = next_t.chars().count();
    let cur_last = cur_t.chars().last().unwrap();
    let next_first = next_t.chars().next().unwrap();

    // 1. Mid-word fracture (e.g. 関 + 係, 帰 + ってきた, パレット + ナイフ, 裸体 + を)
    let mid_word = is_jp_continuation_start(next_t)
        || (is_kanji_char(cur_last) && (is_kanji_char(next_first) || is_kana_char(next_first)))
        || (is_kana_char(cur_last) && is_kana_char(next_first) && cur_last != '、');
    if mid_word && cur_len + next_len <= 110 {
        return true;
    }
    // 2. Short dangling tail (e.g. "ってきた。", "です。")
    if next_len <= 14 && gap <= 0.9 && cur_len + next_len <= 110 {
        return true;
    }
    // 3. Mid-clause without comma and still concise
    if cur_last != '、' && gap <= 0.65 && cur_len + next_len <= 70 {
        return true;
    }
    false
}

/// Splits an overly long stitched cue ONLY on natural Japanese comma (`、`) boundaries
/// (never mid-word!), interpolating timestamps proportionally by character length.
fn split_long_cue_on_commas(cue: SubtitleCue, out: &mut Vec<SubtitleCue>) {
    let chars: Vec<char> = cue.text.chars().collect();
    let total = chars.len();
    if total <= 54 {
        out.push(cue);
        return;
    }
    // Find all comma positions that leave >= 14 chars on both sides
    let commas: Vec<usize> = chars
        .iter()
        .enumerate()
        .filter_map(|(i, &c)| {
            if c == '、' && i + 1 >= 14 && total.saturating_sub(i + 1) >= 14 {
                Some(i + 1)
            } else {
                None
            }
        })
        .collect();

    if commas.is_empty() {
        out.push(cue);
        return;
    }
    let mid = total / 2;
    let split_at = *commas
        .iter()
        .min_by_key(|&&pos| pos.abs_diff(mid))
        .unwrap();

    let left_text: String = chars[..split_at].iter().collect();
    let right_text: String = chars[split_at..].iter().collect();
    let frac = split_at as f64 / total as f64;
    let mid_t = cue.start + (cue.end - cue.start) * frac;

    split_long_cue_on_commas(
        SubtitleCue {
            start: cue.start,
            end: mid_t,
            text: left_text.trim().to_string(),
        },
        out,
    );
    split_long_cue_on_commas(
        SubtitleCue {
            start: mid_t,
            end: cue.end,
            text: right_text.trim().to_string(),
        },
        out,
    );
}

/// Repairs mid-word & mid-clause Whisper `-ml` splits while keeping clean sentence/clause chunks.
fn normalize_japanese_cues(raw: Vec<SubtitleCue>) -> Vec<SubtitleCue> {
    if raw.len() <= 1 {
        return raw;
    }
    let mut stitched: Vec<SubtitleCue> = Vec::with_capacity(raw.len());
    for cue in raw {
        if let Some(last) = stitched.last_mut() {
            if should_stitch(last, &cue) {
                last.text = format!("{}{}", last.text.trim_end(), cue.text.trim_start());
                if cue.end > last.end {
                    last.end = cue.end;
                }
                continue;
            }
        }
        stitched.push(cue);
    }

    let mut result: Vec<SubtitleCue> = Vec::with_capacity(stitched.len());
    for c in stitched {
        split_long_cue_on_commas(c, &mut result);
    }
    result
}
