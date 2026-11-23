use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::models::{AnimeSeries, CustomAnimeMeta, Episode, Playlist, PlaylistItem};
use crate::subs::is_subtitle_file;

struct CatalogSeed {
    id: &'static str,
    title: &'static str,
    jp_title: &'static str,
    subtitle: &'static str,
    accent: &'static str,
    aliases: &'static [&'static str],
    episodes: &'static [(u32, &'static str, &'static str)],
}

const BUILTIN_CATALOG: &[CatalogSeed] = &[
    CatalogSeed {
        id: "my-hero-academia",
        title: "My Hero Academia",
        jp_title: "僕のヒーローアカデミア",
        subtitle: "Season 1",
        accent: "#EE4766",
        aliases: &[
            "mha",
            "bnha",
            "my hero",
            "myhero",
            "hero academia",
            "boku no hero",
            "bokunohero",
            "僕のヒーロー",
            "ヒロアカ",
        ],
        episodes: &[
            (1, "Izuku Midoriya: Origin", "緑谷出久：オリジン"),
            (2, "What It Takes to Be a Hero", "ヒーローの条件"),
            (3, "Roaring Muscles", "うなれ筋肉"),
            (4, "Start Line", "スタートライン"),
            (5, "What I Can Do For Now", "今 僕に出来ることを"),
            (6, "Rage, You Damned Nerd", "猛れクソナード"),
            (7, "Deku vs. Kacchan", "デクvsかっちゃん"),
            (8, "Bakugo's Start Line", "スタートライン、爆豪の。"),
            (9, "Yeah, Just Do Your Best, Ida!", "いいぞガンバレ飯田くん！"),
            (10, "Encounter with the Unknown", "未知との遭遇"),
            (11, "Game Over", "ゲームオーバー"),
            (12, "All Might", "オールマイト"),
            (13, "In Each of Our Hearts", "各々の胸に"),
        ],
    },
    CatalogSeed {
        id: "monster",
        title: "Monster",
        jp_title: "モンスター",
        subtitle: "Episodes 1–24",
        accent: "#B49DE0",
        aliases: &["monster", "モンスター", "tenma", "johan"],
        episodes: &[
            (1, "Herr Dr. Tenma", "ヘルDr.テンマ"),
            (2, "Downfall", "転落"),
            (3, "Murder Event", "殺人事件"),
            (4, "The Night of Execution", "処刑の夜"),
            (5, "The Girl from Heidelberg", "ハイデルベルクの少女"),
            (6, "Disappearance Report", "失踪記事"),
            (7, "Mansion of Tragedy", "惨劇の館"),
            (8, "The Fugitive", "追われる身"),
            (9, "The Old Soldier and the Girl", "老兵と少女"),
            (10, "An Erased Past", "消された過去"),
            (11, "511 Kinderheim", "511キンダーハイム"),
            (12, "A Modest Experiment", "ささやかな実験"),
            (13, "Petra and Schuhmann", "ペトラとシューマン"),
            (14, "The Last Man, the Last Woman", "残された男・残された女"),
            (15, "Be My Baby", "ビー・マイ・ベイビー"),
            (16, "Wolf's Confession", "ヴォルフの告白"),
            (17, "Reunion", "再会"),
            (18, "Five Spoonfuls of Sugar", "五杯目の砂糖"),
            (19, "The Monster's Abyss", "怪物の深淵"),
            (20, "Journey to Freiham", "フライハムへの旅"),
            (21, "A Wonderful Holiday", "幸せな休日"),
            (22, "Lunge's Trap", "ルンゲの罠"),
            (23, "Eva's Confession", "エヴァの告白"),
            (24, "The Men's Dining Table", "男達の食卓"),
        ],
    },
    CatalogSeed {
        id: "example-japanese-stories",
        title: "Japanese Stories",
        jp_title: "日本語の物語",
        subtitle: "Public Domain LibriVox",
        accent: "#2ECC40",
        aliases: &[
            "toshishun",
            "garasudo",
            "hyakunin",
            "botchan",
            "akutagawa",
            "soseki",
            "stories",
            "example",
        ],
        episodes: &[],
    },
    CatalogSeed {
        id: "great-pretender",
        title: "Great Pretender",
        jp_title: "グレートプリテンダー",
        subtitle: "Cases 1–3",
        accent: "#F6D47A",
        aliases: &["great pretender", "greatpretender", "pretender", "グレートプリテンダー"],
        episodes: &[
            (1, "Case 1_1: Los Angeles Connection", "CASE1_1 ロサンゼルス・コネクション"),
            (2, "Case 1_2: Los Angeles Connection", "CASE1_2 ロサンゼルス・コネクション"),
            (3, "Case 1_3: Los Angeles Connection", "CASE1_3 ロサンゼルス・コネクション"),
            (4, "Case 1_4: Los Angeles Connection", "CASE1_4 ロサンゼルス・コネクション"),
            (5, "Case 1_5: Los Angeles Connection", "CASE1_5 ロサンゼルス・コネクション"),
            (6, "Case 2_1: Singapore Sky", "CASE2_1 シンガポール・スカイ"),
            (7, "Case 2_2: Singapore Sky", "CASE2_2 シンガポール・スカイ"),
            (8, "Case 2_3: Singapore Sky", "CASE2_3 シンガポール・スカイ"),
            (9, "Case 2_4: Singapore Sky", "CASE2_4 シンガポール・スカイ"),
            (10, "Case 2_5: Singapore Sky", "CASE2_5 シンガポール・スカイ"),
            (11, "Case 3_1: Snow of London", "CASE3_1 ロンドンの雪"),
            (12, "Case 3_2: Snow of London", "CASE3_2 ロンドンの雪"),
            (13, "Case 3_3: Snow of London", "CASE3_3 ロンドンの雪"),
            (14, "Case 3_4: Snow of London", "CASE3_4 ロンドンの雪"),
        ],
    },
    CatalogSeed {
        id: "death-note",
        title: "Death Note",
        jp_title: "デスノート",
        subtitle: "Episodes 1–12",
        accent: "#EE4766",
        aliases: &["death note", "deathnote", "デスノート"],
        episodes: &[
            (1, "Rebirth", "新生"),
            (2, "Confrontation", "対決"),
            (3, "Dealings", "取引"),
            (4, "Pursuit", "追跡"),
            (5, "Tactics", "駆引"),
            (6, "Unraveling", "綻び"),
            (7, "Overcast", "曇天"),
            (8, "Glare", "目線"),
            (9, "Encounter", "接触"),
            (10, "Doubt", "疑惑"),
            (11, "Assault", "突入"),
            (12, "Love", "恋心"),
        ],
    },
    CatalogSeed {
        id: "spy-x-family",
        title: "Spy × Family",
        jp_title: "スパイファミリー",
        subtitle: "Cour 1",
        accent: "#FF7A8F",
        aliases: &["spy x family", "spyxfamily", "spy family", "スパイファミリー"],
        episodes: &[
            (1, "Operation Strix", "オペレーション〈梟〉"),
            (2, "Secure a Wife", "妻役を確保せよ"),
            (3, "Prepare for the Interview", "受験対策をせよ"),
            (4, "The Prestigious School's Interview", "名門校面接試験"),
            (5, "Will They Pass or Fail?", "合否の行方"),
            (6, "The Friendship Scheme", "ナカヨシ作戦"),
            (7, "The Target's Second Son", "標的の次男"),
            (8, "The Counter-Secret Police Cover Operation", "対秘密警察偽装作戦"),
            (9, "Show Off How in Love You Are", "ラブラブを見せつけよ"),
            (10, "The Great Dodgeball Plan", "ドッジボール大作戦"),
            (11, "Stella", "〈星〉"),
            (12, "Penguin Park", "ペンギンパーク"),
        ],
    },
    CatalogSeed {
        id: "fullmetal-alchemist-brotherhood",
        title: "Fullmetal Alchemist: Brotherhood",
        jp_title: "鋼の錬金術師",
        subtitle: "Episodes 1–12",
        accent: "#F9A893",
        aliases: &[
            "fmab",
            "fma",
            "fullmetal",
            "brotherhood",
            "hagaren",
            "鋼の錬金術師",
        ],
        episodes: &[
            (1, "Fullmetal Alchemist", "鋼の錬金術師"),
            (2, "The First Day", "はじまりの日"),
            (3, "City of Heresy", "邪教の街"),
            (4, "An Alchemist's Anguish", "錬金術師の苦悩"),
            (5, "Rain of Sorrows", "哀しみの雨"),
            (6, "Road of Hope", "希望の道"),
            (7, "Hidden Truths", "隠された真実"),
            (8, "The Fifth Laboratory", "第五研究所"),
            (9, "Created Feelings", "創られた想い"),
            (10, "Separate Destinations", "それぞれの行く先"),
            (11, "Miracle at Rush Valley", "ラッシュバレーの奇跡"),
            (12, "One Is All, All Is One", "一は全、全は一"),
        ],
    },
    CatalogSeed {
        id: "steins-gate",
        title: "Steins;Gate",
        jp_title: "シュタインズ・ゲート",
        subtitle: "Episodes 1–12",
        accent: "#B49DE0",
        aliases: &["steins gate", "steinsgate", "steins;gate", "シュタインズ", "シュタゲ"],
        episodes: &[
            (1, "Turning Point", "始まりと終わりのプロローグ"),
            (2, "Time Travel Paranoia", "時間跳躍のパラノイア"),
            (3, "Parallel World Paranoia", "並列過程のパラノイア"),
            (4, "Interpreter Rendezvous", "空理彷徨のランデヴー"),
            (5, "Starmine Rendezvous", "電荷衝突のランデヴー"),
            (6, "Butterfly Effect's Divergence", "蝶翼のダイバージェンス"),
            (7, "Divergence Singularity", "断層のダイバージェンス"),
            (8, "Chaos Theory Homeostasis I", "夢幻のホメオスタシス"),
            (9, "Chaos Theory Homeostasis II", "幻相のホメオスタシス"),
            (10, "Chaos Theory Homeostasis III", "相生のホメオスタシス"),
            (11, "Dogma in Event Horizon", "時空境界のドグマ"),
            (12, "Dogma in Ergosphere", "静止限界のドグマ"),
        ],
    },
    CatalogSeed {
        id: "attack-on-titan",
        title: "Attack on Titan",
        jp_title: "進撃の巨人",
        subtitle: "Season 1",
        accent: "#EE4766",
        aliases: &[
            "aot",
            "snk",
            "attack on titan",
            "attackontitan",
            "shingeki",
            "進撃の巨人",
        ],
        episodes: &[
            (1, "To You, in 2000 Years", "二千年後の君へ"),
            (2, "That Day", "その日"),
            (3, "A Dim Light Amid Despair", "絶望の中で鈍く光る"),
            (4, "The Night of the Closing Ceremony", "解散式の夜"),
            (5, "First Battle", "初陣"),
            (6, "The World the Girl Saw", "少女が見た世界"),
            (7, "Small Blade", "小さな刃"),
            (8, "I Can Hear His Heartbeat", "心臓の鼓動が聞こえる"),
            (9, "Whereabouts of His Left Arm", "左腕の行方"),
            (10, "Response", "応える"),
            (11, "Idol", "偶像"),
            (12, "Wound", "傷"),
        ],
    },
];

pub fn is_audio_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp3" | "m4a" | "ogg" | "opus" | "wav" | "flac" | "webm" | "aac"
    )
}

pub fn is_cover_file(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let stem_low = stem.to_ascii_lowercase();
    let ext_low = ext.to_ascii_lowercase();
    matches!(
        stem_low.as_str(),
        "cover" | "folder" | "poster" | "thumb" | "thumbnail"
    ) && matches!(ext_low.as_str(), "jpg" | "jpeg" | "png" | "webp" | "svg")
}

pub fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        format!("anime-{}", crate::timeutil::now_unix_secs())
    } else {
        trimmed
    }
}

/// Natural numeric-aware string comparison: "ep2.mp3" < "ep10.mp3", "S01E02" < "S01E10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();

    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(&c) = ia.peek() {
                    if c.is_ascii_digit() {
                        na.push(c);
                        ia.next();
                    } else {
                        break;
                    }
                }
                let mut nb = String::new();
                while let Some(&c) = ib.peek() {
                    if c.is_ascii_digit() {
                        nb.push(c);
                        ib.next();
                    } else {
                        break;
                    }
                }
                let va = na.trim_start_matches('0');
                let vb = nb.trim_start_matches('0');
                let cmp = va.len().cmp(&vb.len()).then_with(|| va.cmp(vb));
                if cmp != Ordering::Equal {
                    return cmp;
                }
                let len_cmp = na.len().cmp(&nb.len());
                if len_cmp != Ordering::Equal {
                    return len_cmp;
                }
            }
            (Some(ca), Some(cb)) => {
                ia.next();
                ib.next();
                let cmp = ca.to_ascii_lowercase().cmp(&cb.to_ascii_lowercase());
                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
        }
    }
}

pub fn detect_anime_for_file(
    filename: &str,
    rel_path: &str,
    library: &[AnimeSeries],
) -> Option<String> {
    let hay = format!("{} {}", rel_path, filename).to_lowercase();
    let hay_compact: String = hay
        .chars()
        .filter(|c| c.is_alphanumeric() || !c.is_ascii())
        .collect();

    for seed in BUILTIN_CATALOG {
        for &alias in seed.aliases {
            let a_low = alias.to_lowercase();
            let a_comp: String = a_low
                .chars()
                .filter(|c| c.is_alphanumeric() || !c.is_ascii())
                .collect();
            if hay.contains(&a_low) || (!a_comp.is_empty() && hay_compact.contains(&a_comp)) {
                return Some(seed.id.to_string());
            }
        }
    }

    for anime in library {
        let id_sp = anime.id.replace('-', " ");
        let title_low = anime.title.to_lowercase();
        let title_comp: String = title_low
            .chars()
            .filter(|c| c.is_alphanumeric() || !c.is_ascii())
            .collect();
        if hay.contains(&anime.id.to_lowercase())
            || hay.contains(&id_sp)
            || (!title_low.is_empty() && hay.contains(&title_low))
            || (title_comp.len() >= 4 && hay_compact.contains(&title_comp))
            || (!anime.jp_title.is_empty() && hay.contains(&anime.jp_title.to_lowercase()))
        {
            return Some(anime.id.clone());
        }
    }

    None
}

pub fn ensure_seed_folders(media_root: &Path) {
    let _ = fs::create_dir_all(media_root);
    for seed in BUILTIN_CATALOG {
        let _ = fs::create_dir_all(media_root.join(seed.id));
    }
}

pub fn japanese_stories_items(library: &[AnimeSeries]) -> Vec<PlaylistItem> {
    let Some(stories) = library.iter().find(|a| a.id == "example-japanese-stories") else {
        return Vec::new();
    };
    stories
        .episodes
        .iter()
        .filter(|e| e.available)
        .map(|e| PlaylistItem {
            anime_id: "example-japanese-stories".into(),
            episode_id: e.id.clone(),
        })
        .collect()
}

pub fn default_playlists(library: &[AnimeSeries]) -> Vec<Playlist> {
    vec![
        Playlist {
            id: "pl-japanese-stories".into(),
            name: "Japanese Stories".into(),
            description: "Classic Japanese literature narration (Akutagawa, Sōseki & Hyakunin Isshu)".into(),
            items: japanese_stories_items(library),
        },
        Playlist {
            id: "pl-mha-monster".into(),
            name: "MHA × Monster".into(),
            description: "Alternating episodes — plays automatically once MP3s are added".into(),
            items: vec![
                PlaylistItem {
                    anime_id: "my-hero-academia".into(),
                    episode_id: "ep-01".into(),
                },
                PlaylistItem {
                    anime_id: "monster".into(),
                    episode_id: "ep-01".into(),
                },
                PlaylistItem {
                    anime_id: "my-hero-academia".into(),
                    episode_id: "ep-02".into(),
                },
                PlaylistItem {
                    anime_id: "monster".into(),
                    episode_id: "ep-02".into(),
                },
            ],
        },
    ]
}

pub fn sync_japanese_stories_playlist(playlists: &mut Vec<Playlist>, library: &[AnimeSeries]) {
    let items = japanese_stories_items(library);
    if items.is_empty() {
        return;
    }
    // Upgrade legacy "pl-starter" id or update existing "pl-japanese-stories"
    if let Some(pl) = playlists
        .iter_mut()
        .find(|p| p.id == "pl-japanese-stories" || p.id == "pl-starter")
    {
        pl.id = "pl-japanese-stories".into();
        pl.name = "Japanese Stories".into();
        pl.description =
            "Classic Japanese literature narration (Akutagawa, Sōseki & Hyakunin Isshu)".into();
        pl.items = items;
    } else {
        playlists.insert(
            0,
            Playlist {
                id: "pl-japanese-stories".into(),
                name: "Japanese Stories".into(),
                description:
                    "Classic Japanese literature narration (Akutagawa, Sōseki & Hyakunin Isshu)"
                        .into(),
                items,
            },
        );
    }
}

pub fn extract_season_episode(filename: &str) -> (u32, Option<u32>) {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let upper = stem.to_ascii_uppercase();
    let bytes = upper.as_bytes();

    // 1. SxxExx pattern
    for i in 0..bytes.len() {
        if bytes[i] == b'S' {
            let mut j = i + 1;
            let s_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > s_start && j < bytes.len() && bytes[j] == b'E' {
                let season = upper[s_start..j].parse::<u32>().unwrap_or(1);
                j += 1;
                let e_start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > e_start {
                    if let Ok(ep) = upper[e_start..j].parse::<u32>() {
                        if ep > 0 && ep < 2000 {
                            return (season.max(1), Some(ep));
                        }
                    }
                }
            }
        }
    }

    // 2. EPxx / E04 / Episode 04 pattern
    for i in 0..bytes.len() {
        if bytes[i] == b'E' {
            if i > 0 && bytes[i - 1].is_ascii_alphabetic() {
                continue;
            }
            let mut j = i + 1;
            if upper[j..].starts_with("PISODE") {
                j += 6;
            } else if j < bytes.len() && bytes[j] == b'P' {
                j += 1;
            }
            while j < bytes.len() && matches!(bytes[j], b' ' | b'.' | b'_' | b'-' | b'#') {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                if let Ok(n) = upper[start..j].parse::<u32>() {
                    if n > 0 && n < 2000 {
                        return (1, Some(n));
                    }
                }
            }
        }
    }

    // 3. Leading number
    let trimmed = upper.trim_start();
    let tb = trimmed.as_bytes();
    let mut end = 0;
    while end < tb.len() && tb[end].is_ascii_digit() {
        end += 1;
    }
    if end > 0 {
        if let Ok(n) = trimmed[..end].parse::<u32>() {
            if n > 0 && n < 2000 {
                return (1, Some(n));
            }
        }
    }

    // 4. Standalone number
    let mut last_num: Option<u32> = None;
    let mut k = 0;
    while k < bytes.len() {
        if bytes[k].is_ascii_digit() {
            let s = k;
            while k < bytes.len() && bytes[k].is_ascii_digit() {
                k += 1;
            }
            let is_kbps = upper[k..].starts_with("KB") || upper[k..].starts_with("P");
            if !is_kbps {
                if let Ok(n) = upper[s..k].parse::<u32>() {
                    if n > 0 && n < 1000 && n != 720 && n != 480 && n != 360 {
                        last_num = Some(n);
                    }
                }
            }
        } else {
            k += 1;
        }
    }

    (1, last_num)
}

fn clean_display_title(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let mut s = stem.replace('_', " ").trim().to_string();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() {
        let rest = s[i..].trim_start();
        if let Some(after) = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix('.'))
            .or_else(|| rest.strip_prefix('–'))
        {
            let trimmed = after.trim();
            if !trimmed.is_empty() {
                s = trimmed.to_string();
            }
        }
    }
    s
}

pub fn estimate_mp3_duration(path: &Path, file_size: u64) -> Option<u64> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "mp3" || file_size < 128 {
        return None;
    }
    let mut f = fs::File::open(path).ok()?;
    let mut header10 = [0u8; 10];
    f.read_exact(&mut header10).ok()?;

    let mut offset: u64 = 0;
    if &header10[0..3] == b"ID3" {
        let sz = ((header10[6] as u64 & 0x7F) << 21)
            | ((header10[7] as u64 & 0x7F) << 14)
            | ((header10[8] as u64 & 0x7F) << 7)
            | (header10[9] as u64 & 0x7F);
        offset = 10 + sz;
    }
    f.seek(SeekFrom::Start(offset)).ok()?;

    let mut buf = vec![0u8; 16384];
    let n = f.read(&mut buf).ok()?;
    if n < 4 {
        return None;
    }

    for i in 0..n.saturating_sub(4) {
        if buf[i] == 0xFF && (buf[i + 1] & 0xE0) == 0xE0 {
            let version_bits = (buf[i + 1] >> 3) & 0x03;
            let layer_bits = (buf[i + 1] >> 1) & 0x03;
            let bitrate_idx = ((buf[i + 2] >> 4) & 0x0F) as usize;
            let sr_idx = ((buf[i + 2] >> 2) & 0x03) as usize;

            if version_bits == 1
                || layer_bits != 1
                || bitrate_idx == 0
                || bitrate_idx == 15
                || sr_idx == 3
            {
                continue;
            }
            const BR_V1_L3: [u64; 16] = [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
            ];
            const BR_V2_L3: [u64; 16] = [
                0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
            ];
            let kbps = if version_bits == 3 {
                BR_V1_L3[bitrate_idx]
            } else {
                BR_V2_L3[bitrate_idx]
            };
            if kbps == 0 {
                continue;
            }
            let audio_bytes = file_size.saturating_sub(offset);
            let secs = (audio_bytes * 8) / (kbps * 1000);
            if secs > 0 {
                return Some(secs);
            }
        }
    }
    None
}

fn title_from_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn scan_library(media_root: &Path, custom_meta: &[CustomAnimeMeta]) -> Vec<AnimeSeries> {
    ensure_seed_folders(media_root);

    let custom_map: HashMap<&str, &CustomAnimeMeta> =
        custom_meta.iter().map(|m| (m.id.as_str(), m)).collect();
    let seed_map: HashMap<&str, &CatalogSeed> =
        BUILTIN_CATALOG.iter().map(|s| (s.id, s)).collect();

    let mut ordered_ids: Vec<String> = BUILTIN_CATALOG.iter().map(|s| s.id.to_string()).collect();
    let mut seen: HashSet<String> = ordered_ids.iter().cloned().collect();

    for c in custom_meta {
        if seen.insert(c.id.clone()) {
            ordered_ids.push(c.id.clone());
        }
    }

    if let Ok(entries) = fs::read_dir(media_root) {
        let mut discovered: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        discovered.sort_by(|a, b| natural_cmp(a, b));
        for dir_name in discovered {
            if seen.insert(dir_name.clone()) {
                ordered_ids.push(dir_name);
            }
        }
    }

    let mut library = Vec::new();

    for id in ordered_ids {
        let folder = media_root.join(&id);
        if !folder.exists() {
            let _ = fs::create_dir_all(&folder);
        }

        let mut cover_file: Option<String> = None;
        let mut audio_files: Vec<(String, PathBuf, u64)> = Vec::new();
        let mut sub_files: Vec<String> = Vec::new();

        if let Ok(entries) = fs::read_dir(&folder) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let Some(name) = p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                else {
                    continue;
                };
                if is_cover_file(&p) {
                    let replace = match &cover_file {
                        None => true,
                        Some(prev) => prev.ends_with(".svg") && !name.ends_with(".svg"),
                    };
                    if replace {
                        cover_file = Some(name);
                    }
                } else if is_audio_file(&p) {
                    let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    audio_files.push((name, p, sz));
                } else if is_subtitle_file(&p) {
                    sub_files.push(name);
                }
            }
        }
        sub_files.sort_by(|a, b| natural_cmp(a, b));

        let find_sub = |ep_num: u32, audio_name: Option<&str>| -> Option<String> {
            // 1. Exact stem match (or stem + .ja/.jp/.jpn)
            if let Some(af) = audio_name {
                let stem = Path::new(af)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if let Some(m) = sub_files.iter().find(|sf| {
                    let ss = Path::new(sf)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    ss == stem
                        || ss == format!("{}.ja", stem)
                        || ss == format!("{}.jp", stem)
                        || ss == format!("{}.jpn", stem)
                }) {
                    return Some(m.clone());
                }
            }
            // 2. Episode number match
            sub_files
                .iter()
                .find(|sf| extract_season_episode(sf).1 == Some(ep_num))
                .cloned()
        };

        audio_files.sort_by(|a, b| {
            let (sa, ea) = extract_season_episode(&a.0);
            let (sb, eb) = extract_season_episode(&b.0);
            sa.cmp(&sb)
                .then_with(|| ea.unwrap_or(u32::MAX).cmp(&eb.unwrap_or(u32::MAX)))
                .then_with(|| natural_cmp(&a.0, &b.0))
        });

        let seed = seed_map.get(id.as_str()).copied();
        let custom = custom_map.get(id.as_str()).copied();

        let (title, jp_title, subtitle, accent) = if let Some(c) = custom {
            (
                c.title.clone(),
                c.jp_title.clone(),
                c.subtitle.clone(),
                c.accent.clone(),
            )
        } else if let Some(s) = seed {
            (
                s.title.to_string(),
                s.jp_title.to_string(),
                s.subtitle.to_string(),
                s.accent.to_string(),
            )
        } else {
            (
                title_from_slug(&id),
                String::new(),
                "Local Folder".into(),
                "#B49DE0".into(),
            )
        };

        let mut episodes: Vec<Episode> = Vec::new();
        let mut matched_files: HashSet<String> = HashSet::new();

        if let Some(s) = seed {
            if !s.episodes.is_empty() {
                let mut by_num: BTreeMap<u32, &(String, PathBuf, u64)> = BTreeMap::new();
                for af in &audio_files {
                    let (season, ep_opt) = extract_season_episode(&af.0);
                    if season == 1 {
                        if let Some(n) = ep_opt {
                            by_num.entry(n).or_insert(af);
                        }
                    }
                }

                for &(num, ep_title, ep_jp) in s.episodes {
                    if let Some((fname, fpath, fsize)) = by_num.get(&num).copied() {
                        matched_files.insert(fname.clone());
                        let sub = find_sub(num, Some(fname));
                        episodes.push(Episode {
                            id: format!("ep-{:02}", num),
                            number: num,
                            title: ep_title.to_string(),
                            jp_title: ep_jp.to_string(),
                            filename: Some(fname.clone()),
                            file_size: *fsize,
                            duration_secs: estimate_mp3_duration(fpath, *fsize),
                            available: true,
                            has_subtitles: sub.is_some(),
                            subtitle_file: sub,
                        });
                    } else {
                        let sub = find_sub(num, None);
                        episodes.push(Episode {
                            id: format!("ep-{:02}", num),
                            number: num,
                            title: ep_title.to_string(),
                            jp_title: ep_jp.to_string(),
                            filename: None,
                            file_size: 0,
                            duration_secs: None,
                            available: false,
                            has_subtitles: sub.is_some(),
                            subtitle_file: sub,
                        });
                    }
                }
            }
        }

        let mut next_num = episodes.iter().map(|e| e.number).max().unwrap_or(0) + 1;
        for (fname, fpath, fsize) in audio_files {
            if matched_files.contains(&fname) {
                continue;
            }
            let (season, ep_opt) = extract_season_episode(&fname);
            let num = if season == 1 {
                ep_opt
                    .filter(|n| !episodes.iter().any(|e| e.number == *n))
                    .unwrap_or_else(|| {
                        let n = next_num;
                        next_num += 1;
                        n
                    })
            } else {
                let n = next_num;
                next_num += 1;
                n
            };
            let ep_id = format!("ep-{:02}", num);
            let final_id = if episodes.iter().any(|e| e.id == ep_id) {
                format!("file-{}", slugify(&fname))
            } else {
                ep_id
            };
            let jp = if id == "example-japanese-stories" {
                if fname.contains("Toshishun") {
                    "芥川龍之介・杜子春".into()
                } else if fname.contains("Garasudo") {
                    "夏目漱石・硝子戸の中".into()
                } else if fname.contains("Hyakunin") {
                    "藤原定家・小倉百人一首".into()
                } else if fname.contains("Botchan") {
                    "夏目漱石・坊っちゃん".into()
                } else if fname.contains("Good-Bye") {
                    "太宰治・グッド・バイ".into()
                } else if fname.contains("Gongitsune") {
                    "新美南吉・ごんぎつね".into()
                } else if fname.contains("Akai Rosoku") {
                    "小川未明・赤い蝋燭と人魚".into()
                } else if fname.contains("Kaze Tachinu") {
                    "堀辰雄・風立ちぬ".into()
                } else {
                    "日本語朗読音声".into()
                }
            } else if season > 1 {
                format!("Season {}", season)
            } else {
                String::new()
            };
            let sub = find_sub(num, Some(&fname));
            episodes.push(Episode {
                id: final_id,
                number: num,
                title: clean_display_title(&fname),
                jp_title: jp,
                filename: Some(fname),
                file_size: fsize,
                duration_secs: estimate_mp3_duration(&fpath, fsize),
                available: true,
                has_subtitles: sub.is_some(),
                subtitle_file: sub,
            });
        }

        episodes.sort_by(|a, b| {
            a.number.cmp(&b.number).then_with(|| {
                natural_cmp(
                    a.filename.as_deref().unwrap_or(&a.title),
                    b.filename.as_deref().unwrap_or(&b.title),
                )
            })
        });

        library.push(AnimeSeries {
            id,
            title,
            jp_title,
            subtitle,
            accent,
            has_cover: cover_file.is_some(),
            cover_file,
            folder_path: folder.to_string_lossy().into_owned(),
            episodes,
        });
    }

    library
}
