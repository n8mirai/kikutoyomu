use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use ruzstd::StreamingDecoder;
use serde::{Deserialize, Serialize};
use vibrato::{Dictionary, Tokenizer};

use crate::dict::LocalJmDict;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorphToken {
    /// Exact text as it appears in the subtitle
    pub surface: String,
    /// Canonical dictionary base form (基本形), e.g. 離れ -> 離れる, しまっ -> しまう
    pub lemma: String,
    /// Major IPADIC POS (名詞, 動詞, 形容詞, 副詞, 助詞, 助動詞, 記号, ...)
    pub pos: String,
    /// IPADIC sub-POS (一般, 固有名詞, 自立, 非自立, 接尾, 代名詞, 数, サ変接続, ...)
    pub sub_pos: String,
    /// Contextual reading in hiragana from MeCab/IPADIC Viterbi decoding
    pub reading: String,
    /// True for punctuation, particles, auxiliaries, dependent verbs (しまう/いる/ある), counters, etc.
    pub ignore: bool,
    /// True for proper content words (nouns, independent verbs, adjectives) suitable for the Word Map
    pub content: bool,
}

pub struct MorphAnalyzer {
    tokenizer: Tokenizer,
}

fn katakana_to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (0x30A1..=0x30F6).contains(&u) {
                char::from_u32(u - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

fn is_punct_or_symbol_str(s: &str) -> bool {
    s.chars().all(|c| {
        c.is_whitespace()
            || c.is_ascii_punctuation()
            || matches!(
                c,
                '。' | '、'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '（'
                    | '）'
                    | '【'
                    | '】'
                    | '〈'
                    | '〉'
                    | '《'
                    | '》'
                    | '〔'
                    | '〕'
                    | '｛'
                    | '｝'
                    | '・'
                    | '！'
                    | '？'
                    | '…'
                    | '―'
                    | 'ー'
                    | '〜'
                    | '～'
                    | '　'
                    | '♪'
                    | '☆'
                    | '★'
            )
    })
}

fn is_numeric_str(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c) || matches!(c, '.' | ',' | '%' | '％'))
}

fn is_single_kana(s: &str) -> bool {
    let mut it = s.chars();
    let Some(c) = it.next() else { return false };
    if it.next().is_some() {
        return false;
    }
    ('\u{3040}'..='\u{309F}').contains(&c) || ('\u{30A0}'..='\u{30FF}').contains(&c)
}

fn is_small_kana_only(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            matches!(
                c,
                'ぁ' | 'ぃ'
                    | 'ぅ'
                    | 'ぇ'
                    | 'ぉ'
                    | 'っ'
                    | 'ゃ'
                    | 'ゅ'
                    | 'ょ'
                    | 'ゎ'
                    | 'ァ'
                    | 'ィ'
                    | 'ゥ'
                    | 'ェ'
                    | 'ォ'
                    | 'ッ'
                    | 'ャ'
                    | 'ュ'
                    | 'ョ'
                    | 'ヮ'
                    | 'ー'
            )
        })
}

fn is_basic_grammar_lemma(lemma: &str, pos: &str) -> bool {
    if pos == "動詞" {
        return matches!(
            lemma,
            "する" | "なる" | "いる" | "ある" | "おる" | "しまう" | "くる" | "いく" | "できる"
        );
    }
    if pos == "形容詞" {
        return matches!(lemma, "ない" | "よい" | "いい");
    }
    if pos == "名詞" {
        return matches!(
            lemma,
            "こと" | "もの" | "ところ" | "ため" | "よう" | "の" | "ん" | "わけ" | "はず" | "うち" | "さん" | "くん" | "ちゃん"
        );
    }
    false
}

/// Ultra-common basic words excluded from the Word Frequency Map (so only meaningful domain nouns, verbs & adjectives show up).
fn is_wordmap_stopword(lemma: &str) -> bool {
    matches!(
        lemma,
        "何" | "誰"
            | "私"
            | "僕"
            | "俺"
            | "君"
            | "彼"
            | "彼女"
            | "自分"
            | "皆"
            | "みんな"
            | "人"
            | "事"
            | "物"
            | "時"
            | "所"
            | "方"
            | "今"
            | "前"
            | "後"
            | "中"
            | "上"
            | "下"
            | "内"
            | "外"
            | "間"
            | "気"
            | "手"
            | "目"
            | "口"
            | "日"
            | "年"
            | "月"
            | "一"
            | "二"
            | "三"
            | "四"
            | "五"
            | "六"
            | "七"
            | "八"
            | "九"
            | "十"
            | "百"
            | "千"
            | "万"
            | "言う"
            | "思う"
            | "見る"
            | "来る"
            | "行く"
            | "出る"
            | "入る"
            | "持つ"
            | "取る"
            | "知る"
            | "分かる"
            | "分る"
            | "聞く"
            | "話す"
            | "書く"
            | "する"
            | "なる"
            | "いる"
            | "ある"
            | "おる"
            | "ない"
            | "よい"
            | "いい"
            | "できる"
            | "しまう"
            | "くれる"
            | "もらう"
            | "あげる"
            | "やる"
            | "いう"
            | "つく"
            | "みる"
            | "おく"
            | "リブリボックス"
            | "パブリック"
            | "ドメイン"
            | "ボランティア"
            | "サイト"
            | "録音"
    )
}

impl MorphAnalyzer {
    pub fn load_from_zst(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Cannot open IPADIC: {}", e))?;
        let buf = BufReader::with_capacity(1 << 20, file);
        let decoder = StreamingDecoder::new(buf).map_err(|e| format!("zstd init error: {}", e))?;
        let dict = Dictionary::read(decoder).map_err(|e| format!("Vibrato dict read error: {}", e))?;
        let tokenizer = Tokenizer::new(dict);
        Ok(Self { tokenizer })
    }

    pub fn tokenize_line(&self, text: &str, dict: Option<&LocalJmDict>) -> Vec<MorphToken> {
        if text.is_empty() {
            return Vec::new();
        }
        let mut worker = self.tokenizer.new_worker();
        worker.reset_sentence(text);
        worker.tokenize();

        let n = worker.num_tokens();
        let mut raw_tokens: Vec<MorphToken> = Vec::with_capacity(n);

        for i in 0..n {
            let tok = worker.token(i);
            let surface = tok.surface().to_string();
            let feat = tok.feature();
            // IPADIC 9-field CSV:
            // 0:POS, 1:sub1, 2:sub2, 3:sub3, 4:conj_type, 5:conj_form, 6:lemma(原形), 7:reading(読み), 8:pron
            let parts: Vec<&str> = feat.split(',').collect();
            let pos = parts.first().copied().unwrap_or("").to_string();
            let sub_pos = parts.get(1).copied().unwrap_or("").to_string();
            let raw_lemma = parts.get(6).copied().unwrap_or("*");
            let lemma = if raw_lemma.is_empty() || raw_lemma == "*" {
                surface.clone()
            } else {
                raw_lemma.to_string()
            };
            let raw_read = parts.get(7).copied().unwrap_or("*");
            let reading = if raw_read.is_empty() || raw_read == "*" {
                katakana_to_hiragana(&surface)
            } else {
                katakana_to_hiragana(raw_read)
            };

            let (ignore, content) = classify_token(&surface, &lemma, &pos, &sub_pos);

            raw_tokens.push(MorphToken {
                surface,
                lemma,
                pos,
                sub_pos,
                reading,
                ignore,
                content,
            });
        }

        // Post-passes for natural Japanese word boundaries:
        // 1. Merge split hiragana compound verbs (e.g. こび + つか)
        // 2. Merge JMdict-validated compound nouns & kanji noun suffixes (看護+婦 -> 看護婦, 診察+室 -> 診察室, パレット+ナイフ -> パレットナイフ)
        // 3. Attach bound verb/adjective conjugations (上がら+せ -> 上がらせ, 離れ+た -> 離れた, 食べ+られ+なかった -> 食べられなかった)
        let t1 = merge_broken_hiragana_verbs(raw_tokens);
        let t2 = merge_compound_nouns(t1, dict);
        merge_verb_adj_conjugations(t2)
    }
}

fn classify_token(surface: &str, lemma: &str, pos: &str, sub_pos: &str) -> (bool, bool) {
    let trimmed = surface.trim();
    if trimmed.is_empty() || is_punct_or_symbol_str(trimmed) || is_numeric_str(trimmed) || is_small_kana_only(trimmed) {
        return (true, false);
    }
    // Grammar & non-content parts of speech -> auto-ignore!
    if matches!(pos, "記号" | "助詞" | "助動詞" | "フィラー" | "その他") {
        return (true, false);
    }
    if pos == "名詞" && matches!(sub_pos, "数" | "非自立" | "接尾" | "代名詞" | "特殊") {
        return (true, false);
    }
    if pos == "動詞" && matches!(sub_pos, "非自立" | "接尾") {
        return (true, false);
    }
    if pos == "形容詞" && matches!(sub_pos, "非自立" | "接尾") {
        return (true, false);
    }
    if pos == "接頭詞" || pos == "連体詞" || pos == "接続詞" || pos == "感動詞" {
        return (true, false);
    }
    if is_basic_grammar_lemma(lemma, pos) {
        return (true, false);
    }
    // Single pure kana characters are ignored UNLESS IPADIC recognized a multi-char dictionary lemma (e.g. こ -> 来る or い -> 言う/居る)
    if is_single_kana(trimmed) && lemma.chars().count() <= 1 {
        return (true, false);
    }

    // Proper content words for Word Map: Nouns, Independent Verbs, Independent Adjectives
    let is_content_pos = (pos == "名詞" && matches!(sub_pos, "一般" | "サ変接続" | "固有名詞" | "形容動詞語幹" | "副詞可能" | "ナイ形容詞語幹"))
        || (pos == "動詞" && sub_pos == "自立")
        || (pos == "形容詞" && sub_pos == "自立");

    let content = is_content_pos && !is_wordmap_stopword(lemma) && lemma.chars().count() >= 1 && !is_single_kana(lemma);
    (false, content)
}

/// Handles edge cases where colloquial/Whisper hiragana verbs get split across a 2-kana stem + verb (e.g. こび + つか(せる) -> こびつか / こびつく).
fn merge_broken_hiragana_verbs(tokens: Vec<MorphToken>) -> Vec<MorphToken> {
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        // Case: pure-hiragana verb stem (e.g. こび / こびり) immediately followed by 動詞(自立) like つく/つくす/かける/出す in hiragana
        // ONLY merge if both are pure hiragana and the first is a short 2-3 char non-ignored hiragana verb/adv stem followed by つく/だす/こむ/きる/あう/かける
        if i + 1 < tokens.len() {
            let a = &tokens[i];
            let b = &tokens[i + 1];
            let a_hira = !a.surface.is_empty() && a.surface.chars().all(|c| ('\u{3040}'..='\u{309F}').contains(&c));
            let b_hira = !b.surface.is_empty() && b.surface.chars().all(|c| ('\u{3040}'..='\u{309F}').contains(&c));
            if a_hira
                && b_hira
                && !a.ignore
                && b.pos == "動詞"
                && b.sub_pos == "自立"
                && matches!(b.lemma.as_str(), "つく" | "付く" | "だす" | "出す" | "こむ" | "込む" | "きる" | "切る")
                && a.surface.chars().count() <= 3
                && matches!(a.pos.as_str(), "動詞" | "副詞" | "名詞")
            {
                let merged_surface = format!("{}{}", a.surface, b.surface);
                let merged_lemma = format!("{}{}", a.surface, b.lemma);
                let merged_read = format!("{}{}", a.reading, b.reading);
                out.push(MorphToken {
                    surface: merged_surface,
                    lemma: merged_lemma,
                    pos: "動詞".to_string(),
                    sub_pos: "自立".to_string(),
                    reading: merged_read,
                    ignore: false,
                    content: true,
                });
                i += 2;
                continue;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

fn has_kanji_or_kata(s: &str) -> bool {
    s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{30A0}'..='\u{30FF}').contains(&c) || c == '々')
}

/// Merges IPADIC-split compound nouns validated by JMdict (e.g. 看護+婦 -> 看護婦, 診察+室 -> 診察室, パレット+ナイフ -> パレットナイフ).
fn merge_compound_nouns(tokens: Vec<MorphToken>, dict: Option<&LocalJmDict>) -> Vec<MorphToken> {
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let mut cur = tokens[i].clone();
        if cur.pos == "名詞"
            && !matches!(cur.sub_pos.as_str(), "数" | "非自立" | "代名詞" | "特殊")
            && has_kanji_or_kata(&cur.surface)
        {
            while i + 1 < tokens.len() {
                let next = &tokens[i + 1];
                if next.pos != "名詞" || !has_kanji_or_kata(&next.surface) {
                    break;
                }
                if matches!(next.sub_pos.as_str(), "数" | "非自立" | "代名詞" | "特殊") {
                    break;
                }
                let merged = format!("{}{}", cur.surface, next.surface);
                let in_dict = dict.map(|d| d.has_exact_word(&merged)).unwrap_or(false);
                // Merge if validated in JMdict, OR if it's a single-kanji nominal suffix (like 婦, 室, 欲, 浴, 的)
                let is_kanji_suffix = next.sub_pos == "接尾"
                    && next.surface.chars().count() == 1
                    && next.surface.chars().all(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
                if in_dict || is_kanji_suffix {
                    cur.surface = merged.clone();
                    cur.lemma = merged;
                    cur.reading = format!("{}{}", cur.reading, next.reading);
                    cur.sub_pos = "一般".to_string();
                    cur.ignore = false;
                    cur.content = !is_wordmap_stopword(&cur.lemma);
                    i += 1;
                } else {
                    break;
                }
            }
        }
        out.push(cur);
        i += 1;
    }
    out
}

/// Attaches bound inflectional conjugation suffixes (causative せ/させ, passive/potential れ/られ,
/// tense/negation/politeness た/ない/ます/たい/ぬ/う/よう) directly onto the main verb or i-adjective
/// so `上がらせ`, `離れた`, `食べられなかった`, `高かった` highlight & hover as ONE cohesive word
/// while preserving the canonical base form lemma (`上がる`, `離れる`, `食べる`, `高い`)!
fn merge_verb_adj_conjugations(tokens: Vec<MorphToken>) -> Vec<MorphToken> {
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let mut cur = tokens[i].clone();
        let is_conjugatable = !cur.ignore
            && ((cur.pos == "動詞" && cur.sub_pos == "自立")
                || (cur.pos == "形容詞" && cur.sub_pos == "自立"));
        if is_conjugatable {
            while i + 1 < tokens.len() {
                let next = &tokens[i + 1];
                let is_bound_suffix = (next.pos == "動詞" && next.sub_pos == "接尾")
                    || (next.pos == "助動詞"
                        && !matches!(next.lemma.as_str(), "だ" | "です" | "らしい" | "みたい"));
                if is_bound_suffix {
                    cur.surface.push_str(&next.surface);
                    cur.reading.push_str(&next.reading);
                    i += 1;
                } else {
                    break;
                }
            }
        }
        out.push(cur);
        i += 1;
    }
    out
}
