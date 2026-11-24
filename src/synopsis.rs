use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dict::LocalJmDict;
use crate::morph::MorphAnalyzer;
use crate::oauth::AiAuth;
use crate::subs::SubtitleCue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneBeat {
    pub start: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EpisodeSynopsis {
    #[serde(default)]
    pub episode_summary: String,
    #[serde(default)]
    pub series_summary: String,
    #[serde(default)]
    pub series_title: String,
    #[serde(default)]
    pub ai_generated: bool,
    #[serde(default)]
    pub beats: Vec<SceneBeat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SynopsisStore {
    #[serde(default)]
    pub series: BTreeMap<String, String>,
    #[serde(default)]
    pub episodes: BTreeMap<String, EpisodeSynopsis>,
}

impl SynopsisStore {
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

/// Built-in series & literary work synopses so every catalog anime and all 8 Japanese Stories
/// have rich, accurate English context out of the box (zero API key required!).
pub fn builtin_series_synopsis(anime_id: &str, episode_title: &str) -> (String, String) {
    if anime_id == "example-japanese-stories" {
        let low = episode_title.to_ascii_lowercase();
        if low.contains("toshishun") {
            return (
                "Toshishun — Ryūnosuke Akutagawa".into(),
                "In Tang-dynasty Luoyang, a ruined young man named Toshishun is granted immense wealth twice by an enigmatic sage, only to lose his friends whenever his fortune vanishes. Disillusioned with human greed, he asks to become an immortal hermit on Mount Emei—where a terrifying final trial forces him to choose between transcendence and human love.".into(),
            );
        }
        if low.contains("garasudo") {
            return (
                "Inside My Glass Doors — Natsume Sōseki".into(),
                "A collection of quiet, intimate late-life essays written by Natsume Sōseki from his study. Looking out through his glass doors, he reflects on visitors, stray dogs, childhood memories of old Tokyo, mortality, and the gentle absurdities of everyday life.".into(),
            );
        }
        if low.contains("hyakunin") {
            return (
                "Hyakunin Isshu — Classical Waka Anthology".into(),
                "Japan's most beloved anthology of one hundred classical 31-syllable waka poems compiled in the 13th century by Fujiwara no Teika, weaving together seasonal nature imagery, longing, courtly love, and the impermanence of time.".into(),
            );
        }
        if low.contains("gongitsune") {
            return (
                "Gon, the Little Fox — Nankichi Niimi".into(),
                "A poignant Japanese folk classic about Gon, a lonely mischievous orphan fox living near Nakayama village. After a thoughtless prank harms a poor villager named Hyōjū on the day his mother dies, Gon secretly leaves chestnuts and mushrooms at Hyōjū's door every day to atone—leading to a heartbreaking misunderstanding.".into(),
            );
        }
        if low.contains("akai rosoku") {
            return (
                "Red Candles and the Mermaid — Mimei Ogawa".into(),
                "A haunting modern fairy tale set in a northern coastal town. A mermaid leaves her infant daughter with a kind elderly candle-making couple; the girl grows up painting enchanting red candles that protect sailors at sea, until human greed shatters their quiet life.".into(),
            );
        }
        if low.contains("good-bye") {
            return (
                "Good-Bye — Osamu Dazai".into(),
                "Osamu Dazai's brisk, darkly comic final unfinished novel. Shūji Tajima, a roguish Tokyo magazine editor entangled with too many women, recruits the strikingly beautiful yet fiercely blunt black-market peddler Kinuko to pose as his wife so he can break off each affair one by one.".into(),
            );
        }
        if low.contains("kaze tachinu") {
            return (
                "The Wind Has Risen — Tatsuo Hori".into(),
                "A lyrical, deeply moving autobiographical novella set in the mountain highlands of Karuizawa. A young writer stays by the side of his fiancée Setsuko at a tuberculosis sanatorium, finding fragile beauty and quiet devotion in the mountain wind, light, and passing seasons.".into(),
            );
        }
        if low.contains("botchan") {
            return (
                "Botchan — Natsume Sōseki".into(),
                "One of Japan's most famous comic novels. A hot-headed, brutally honest young man from Tokyo takes a job teaching mathematics at a rural middle school in Shikoku, where he clashes hilariously with scheming colleagues—nicknaming them Redshirt, Clown, and Porcupine—while stubbornly defending his sense of justice.".into(),
            );
        }
        return (
            "Japanese Stories (Classic Literature)".into(),
            "Nine hours of unabridged public-domain Japanese literary classics—featuring Akutagawa, Natsume Sōseki, Osamu Dazai, Tatsuo Hori, Nankichi Niimi, and Mimei Ogawa—paired with speech-synced Japanese subtitles.".into(),
        );
    }

    match anime_id {
        "my-hero-academia" => (
            "My Hero Academia".into(),
            "In a world where almost everyone is born with a superpower called a Quirk, Quirkless teenager Izuku Midoriya inherits the legendary power of All Might and enrolls at U.A. High School to become a true hero.".into(),
        ),
        "monster" => (
            "Monster".into(),
            "Brilliant Japanese neurosurgeon Dr. Kenzo Tenma risks his career in 1980s Germany to save a young boy's life over a politician's—only to discover years later that the boy has grown into Johan Liebert, a charismatic sociopath.".into(),
        ),
        "great-pretender" => (
            "Great Pretender".into(),
            "Self-proclaimed Japan's greatest swindler Makoto Edamura gets outsmarted by suave French con man Laurent Thierry and swept into globe-trotting, high-stakes heists targeting corrupt billionaires.".into(),
        ),
        "death-note" => (
            "Death Note".into(),
            "Genius high schooler Light Yagami discovers a supernatural notebook that kills anyone whose name is written inside, igniting a ruthless cat-and-mouse battle of wits against the enigmatic detective L.".into(),
        ),
        "spy-x-family" => (
            "Spy × Family".into(),
            "To preserve peace between rival nations, master spy Twilight must build a fake family overnight—unaware that his new wife is a deadly assassin and his adopted daughter is a telepath who knows everyone's secrets.".into(),
        ),
        "fullmetal-alchemist-brotherhood" => (
            "Fullmetal Alchemist: Brotherhood".into(),
            "After a forbidden alchemical ritual costs Edward Elric his limbs and traps his brother Alphonse's soul in a suit of armor, the two brothers search for the Philosopher's Stone—uncovering a vast conspiracy beneath Amestris.".into(),
        ),
        "steins-gate" => (
            "Steins;Gate".into(),
            "Eccentric self-proclaimed mad scientist Rintaro Okabe and his Akihabara labmates accidentally invent a microwave capable of sending text messages into the past, spiraling into a desperate race across diverging worldlines.".into(),
        ),
        "attack-on-titan" => (
            "Attack on Titan".into(),
            "Humanity survives behind towering concentric walls to keep out man-eating Titans—until a Colossal Titan breaches the outer gate, driving Eren Yeager, Mikasa, and Armin to uncover the truth beyond the walls.".into(),
        ),
        _ => (
            anime_id.replace('-', " "),
            "Japanese audio immersion series synced in Kiku.".into(),
        ),
    }
}

pub fn builtin_episode_synopsis(anime_id: &str, episode_title: &str) -> String {
    if anime_id != "example-japanese-stories" {
        return String::new();
    }
    let low = episode_title.to_ascii_lowercase();
    if low.contains("toshishun") {
        if low.contains("pt 1") {
            return "Impoverished under the west gate of Luoyang, Toshishun meets the mysterious old sage Tekkanshi, who reveals a buried cart of gold and makes him the richest man in the capital overnight.".into();
        }
        if low.contains("pt 2") {
            return "Once wealthy again, Toshishun throws lavish banquets and is surrounded by flattering friends—until his fortune runs dry after three years and everyone immediately abandons him.".into();
        }
        if low.contains("pt 3") {
            return "Granted a second fortune by the sage and betrayed by fair-weather friends once more, Toshishun grows disillusioned with humanity and begs Tekkanshi to take him as a disciple of the immortal arts.".into();
        }
        if low.contains("pt 4") {
            return "Tekkanshi carries Toshishun on a bamboo staff to the misty peaks of Mount Emei and warns him that no matter what terrors appear before him, he must remain completely silent.".into();
        }
        if low.contains("pt 5") {
            return "Tigers, serpents, thunder, and towering demons assault Toshishun, eventually dragging his soul down into the underworld before King Enma—yet he refuses to utter a single word.".into();
        }
        if low.contains("pt 6") {
            return "Seeing his dead parents transformed into starving horses and beaten by demons, Toshishun cries out 'Mother!'—failing the trial, yet discovering through the sage's warmth that he wants to live an honest human life.".into();
        }
    }
    if low.contains("gongitsune") {
        if low.contains("pt 1") {
            return "Near the old castle ruins of Nakayama lives Gon, a lonely orphan fox who loves sneaking into the village for mischievous pranks. One rainy autumn day he spies Hyōjū catching eels in the river.".into();
        }
        if low.contains("pt 2") {
            return "Gon steals the fish from Hyōjū's basket and runs off with the slippery eel wrapped around his neck—only to learn days later that Hyōjū's sick mother had wanted eels right before she passed away.".into();
        }
        if low.contains("pt 3") {
            return "Filled with quiet remorse during the funeral, Gon resolves to make amends and begins secretly leaving sardines and forest chestnuts at Hyōjū's house.".into();
        }
        if low.contains("pt 4") {
            return "Perplexed by the daily gifts appearing at his doorway, Hyōjū talks with his friend Kasuke and begins to wonder if a mountain deity is watching over him.".into();
        }
        if low.contains("pt 5") {
            return "Quietly trailing behind Hyōjū and Kasuke on the moonlit path, Gon realizes Hyōjū credits the gods rather than him—leaving the little fox wistful, yet still devoted.".into();
        }
        if low.contains("pt 6") {
            return "The next day Gon slips inside Hyōjū's shed to leave fresh chestnuts; mistaking him for a thief, Hyōjū fires his matchlock—only to spot the neat pile of chestnuts too late as Gon closes his eyes.".into();
        }
    }
    if low.contains("kaze tachinu") {
        if low.contains("ch 1") {
            return "Prelude ('The Wind Has Risen'): In the bright summer highlands of Karuizawa, the narrator recalls first meeting Setsuko while painting beneath the wind-swept trees and watching their quiet bond take root.".into();
        }
        if low.contains("ch 2") {
            return "Spring: As Setsuko's illness deepens, the narrator accompanies her to the mountain sanatorium, cherishing small ordinary walks and conversations while trying to keep fear at bay.".into();
        }
        if low.contains("ch 3") {
            return "The Wind Has Risen: Settling into daily life at the mountain sanatorium, the narrator walks past the examination room, nurses, and patients sunbathing on balconies, holding onto vivid moments of calm and tenderness with Setsuko.".into();
        }
        if low.contains("ch 4") {
            return "Winter: Snow closes in around the sanatorium as the seasons turn colder; the narrator watches over Setsuko through quiet winter days, reflecting on love, transience, and the fragile light outside.".into();
        }
        if low.contains("ch 5") {
            return "Valley of the Shadow of Death: Returning alone to a snowbound mountain cottage in winter, the narrator listens to the wind and writes through grief, discovering that Setsuko's memory still gently sustains him.".into();
        }
    }
    if low.contains("botchan") {
        return "A chapter from Natsume Sōseki's classic comic novel following the blunt, hot-headed Tokyo math teacher navigating eccentric colleagues and mischievous students in rural Shikoku.".into();
    }
    if low.contains("good-bye") {
        return "A brisk chapter from Osamu Dazai's comic final novel as Tajima and the fiercely pragmatic Kinuko plot their next theatrical breakup with one of his entangled mistresses.".into();
    }
    if low.contains("garasudo") {
        return "A quiet, reflective essay by Natsume Sōseki observing daily visitors, old Tokyo memories, and human nature from behind his study's glass doors.".into();
    }
    if low.contains("akai rosoku") {
        return "A chapter from Mimei Ogawa's haunting fairy tale of the mermaid's daughter raised by elderly candle makers in a northern seaside town.".into();
    }
    if low.contains("hyakunin") {
        return "A recitation of classical 31-syllable Japanese waka poems from the Hyakunin Isshu anthology, evoking nature, longing, and courtly memory.".into();
    }
    String::new()
}

/// Cleans a JMdict English gloss into a short, readable concept phrase (e.g. "(way of) life" -> "way of life").
fn clean_gloss(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Strip leading parenthetical notes like "(common) " or "(e.g. ...)"
    while let Some(rest) = s.strip_prefix('(') {
        if let Some((_, after)) = rest.split_once(')') {
            s = after.trim().to_string();
        } else {
            break;
        }
    }
    if let Some((before, _)) = s.split_once('(') {
        let trimmed = before.trim();
        if trimmed.len() >= 3 {
            s = trimmed.to_string();
        }
    }
    s.trim_end_matches(&['.', ',', ';'][..]).trim().to_string()
}

/// Zero-API offline fallback: groups synced subtitles into ~22s scene windows and builds concise
/// ephemeral English concept traces using MeCab + JMdict (so ANY episode has live traces immediately!).
pub fn build_offline_beats(
    cues: &[SubtitleCue],
    morph: Option<&MorphAnalyzer>,
    jmdict: Option<&LocalJmDict>,
) -> Vec<SceneBeat> {
    if cues.is_empty() {
        return Vec::new();
    }
    let (Some(mp), Some(jm)) = (morph, jmdict) else {
        return Vec::new();
    };

    let mut beats: Vec<SceneBeat> = Vec::new();
    let mut win_start = cues[0].start;
    let mut win_cues: Vec<&SubtitleCue> = Vec::new();

    let flush_window = |start_t: f64, group: &[&SubtitleCue], out: &mut Vec<SceneBeat>| {
        if group.is_empty() {
            return;
        }
        let mut score_map: HashMap<String, (u32, String)> = HashMap::new();
        for c in group {
            let toks = mp.tokenize_line(&c.text, Some(jm));
            for tk in toks {
                if !tk.content || tk.ignore {
                    continue;
                }
                let entry = score_map.entry(tk.lemma.clone()).or_insert((0, tk.pos.clone()));
                // Prefer nouns slightly over generic verbs for crisp scene imagery
                let boost = if tk.pos == "名詞" { 2 } else { 1 };
                entry.0 += boost;
            }
        }
        let mut ranked: Vec<(String, u32, String)> = score_map
            .into_iter()
            .map(|(w, (sc, p))| (w, sc, p))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.chars().count().cmp(&a.0.chars().count())));

        let mut concepts: Vec<String> = Vec::new();
        for (lemma, _sc, pos) in ranked.into_iter().take(12) {
            let hits = jm.lookup(&lemma, &lemma, &lemma, &pos, None);
            if let Some(first) = hits.first() {
                if let Some(s0) = first.senses.first() {
                    if let Some(g0) = s0.defs.first() {
                        let g = clean_gloss(g0);
                        if !g.is_empty()
                            && g.len() <= 28
                            && !concepts.iter().any(|x| x.eq_ignore_ascii_case(&g))
                        {
                            concepts.push(g);
                            if concepts.len() >= 4 {
                                break;
                            }
                        }
                    }
                }
            }
        }
        if !concepts.is_empty() {
            out.push(SceneBeat {
                start: start_t,
                text: concepts.join(" · "),
            });
        }
    };

    for cue in cues {
        if !win_cues.is_empty() && (cue.start - win_start >= 22.0 || win_cues.len() >= 4) {
            flush_window(win_start, &win_cues, &mut beats);
            win_cues.clear();
            win_start = cue.start;
        }
        win_cues.push(cue);
    }
    flush_window(win_start, &win_cues, &mut beats);
    beats
}

/// Optional 1-click AI narrative scene-trace & synopsis generator using Gemini 2.5 Flash via pure-Rust WinHTTP.
/// Supports BOTH Google OAuth 2.0 Bearer tokens AND plain Gemini API keys!
#[cfg(windows)]
pub fn generate_ai_synopsis(
    auth: &AiAuth,
    series_title: &str,
    episode_title: &str,
    cues: &[SubtitleCue],
) -> Result<EpisodeSynopsis, String> {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Networking::WinHttp::*;

    if cues.is_empty() {
        return Err("No subtitles available for this episode".into());
    }

    // Build compact timestamped transcript (capped to ~14,000 chars so prompt stays fast & clean)
    let mut transcript = String::new();
    for c in cues {
        let m = (c.start as u64) / 60;
        let s = (c.start as u64) % 60;
        let line = format!("[{:02}:{:02} | {}s] {}\n", m, s, c.start as u64, c.text.trim());
        if transcript.len() + line.len() > 14_000 {
            break;
        }
        transcript.push_str(&line);
    }

    let prompt = format!(
        "You are an English immersion companion for a Japanese learner listening to \"{}\" ({}).\n\
        Given the timestamped Japanese transcript below, return ONLY valid JSON (no markdown fences) with this exact shape:\n\
        {{\"episode_summary\":\"2-3 concise English sentences summarizing this episode/chapter.\",\n\
          \"series_summary\":\"2 concise English sentences summarizing the overall series/work.\",\n\
          \"beats\":[{{\"start\":0,\"text\":\"One short evocative present-tense English line (<= 95 chars) describing what is happening in this moment\"}}]}}\n\
        Generate one beat roughly every 25-45 seconds of audio (covering the full transcript). Keep each beat subtle, natural, and present-tense like a calm scene trace.\n\nTranscript:\n{}",
        series_title, episode_title, transcript
    );

    let req_body = serde_json::json!({
        "contents": [{ "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0.35,
            "responseMimeType": "application/json"
        }
    })
    .to_string();

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
    let host = to_wide("generativelanguage.googleapis.com");
    let verb = to_wide("POST");
    let (path_str, hdr_str) = match auth {
        AiAuth::Bearer(tok) => (
            "/v1beta/models/gemini-2.5-flash:generateContent".to_string(),
            format!(
                "Content-Type: application/json\r\nAccept: application/json\r\nAuthorization: Bearer {}\r\n",
                tok.trim()
            ),
        ),
        AiAuth::ApiKey(key) => (
            format!("/v1beta/models/gemini-2.5-flash:generateContent?key={}", key.trim()),
            "Content-Type: application/json\r\nAccept: application/json\r\n".to_string(),
        ),
    };
    let path = to_wide(&path_str);

    let session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0)
    });
    if session.0.is_null() {
        return Err(format!("WinHttpOpen failed ({})", unsafe { GetLastError() }));
    }
    unsafe { WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 45_000); }

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

    let headers = to_wide(&hdr_str);
    let bytes = req_body.as_bytes();
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
        return Err(format!("Gemini request failed ({})", unsafe { GetLastError() }));
    }
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(format!("Gemini response failed ({})", unsafe { GetLastError() }));
    }

    let mut status: u32 = 0;
    let mut status_len: u32 = std::mem::size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut c_void,
            &mut status_len,
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
        return Err(format!("Gemini API HTTP {}: {}", status, resp_str.chars().take(180).collect::<String>()));
    }

    let root: serde_json::Value =
        serde_json::from_str(&resp_str).map_err(|e| format!("Invalid Gemini JSON: {}", e))?;
    let text = root["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| "Empty Gemini response".to_string())?;
    let clean = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut parsed: EpisodeSynopsis =
        serde_json::from_str(clean).map_err(|e| format!("Failed parsing synopsis JSON: {}", e))?;
    parsed.ai_generated = true;
    parsed.series_title = series_title.to_string();
    parsed.beats.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(parsed)
}

#[cfg(not(windows))]
pub fn generate_ai_synopsis(
    _auth: &AiAuth,
    _series_title: &str,
    _episode_title: &str,
    _cues: &[SubtitleCue],
) -> Result<EpisodeSynopsis, String> {
    Err("AI synopsis generation requires Windows WinHTTP".into())
}
