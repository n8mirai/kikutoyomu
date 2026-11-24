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

        // Post-passes for natural spoken Japanese morphology & canonical dictionary lemmas:
        // 1. Repair 1998-newspaper IPADIC mis-parses on spoken colloquial Japanese:
        //    - i-dropped progressive + explanatory ん (待って+たん+だ -> 待って+た+ん+だ, 読んで+るん+だ -> 読んで+る+ん+だ)
        //    - topic+final particle swallow (服+はね、 -> 服+は+ね、)
        //    - colloquial grammar contractions (かもしんない, んすね/っす, 読まんかった, すげえ, あんま, したとこ)
        // 2. Merge split hiragana compound verbs (こび+つか) & JMdict-validated kanji+verb compounds (紐+づける -> 紐づける)
        // 3. Merge prefixes (現+説 -> 現説, 不+自然 -> 不自然), numeric JMdict compounds (万+人 -> 万人),
        //    and JMdict-validated compound nouns & kanji noun suffixes (看護+婦 -> 看護婦, 暴力+的 -> 暴力的)
        // 4. Attach bound verb/adjective conjugations & contracted spoken progressives (待ってた, 書かれてた, 食べてた, 怒られなかった)
        // 5. Canonicalize every token's lemma against JMdict so conjugations, stems, potentials, -く forms & okurigana variants
        //    always map to ONE single dictionary base form!
        let t0 = repair_colloquial_japanese(raw_tokens);
        let t1 = merge_broken_hiragana_verbs(t0);
        let t2 = merge_compound_nouns(t1, dict);
        let t3 = merge_compound_verbs(t2, dict);
        let mut t4 = merge_verb_adj_conjugations(t3);
        if let Some(d) = dict {
            for tok in &mut t4 {
                if !tok.ignore {
                    let canon = d.canonicalize_lemma(&tok.lemma, &tok.pos);
                    if !canon.is_empty() && canon != tok.lemma {
                        tok.lemma = canon;
                        if is_basic_grammar_lemma(&tok.lemma, &tok.pos) {
                            tok.ignore = true;
                            tok.content = false;
                        } else if tok.content && is_wordmap_stopword(&tok.lemma) {
                            tok.content = false;
                        }
                    }
                }
            }
        }
        t4
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

fn is_clause_or_copula_follow(tok: Option<&MorphToken>) -> bool {
    let Some(t) = tok else { return true };
    matches!(t.pos.as_str(), "助動詞" | "記号" | "接続詞")
        || (t.pos == "助詞"
            && matches!(
                t.surface.as_str(),
                "よ" | "ね" | "な" | "か" | "わ" | "ぞ" | "ぜ" | "さ" | "の" | "って" | "けど"
                    | "けれど" | "けれども" | "けども" | "から" | "ので" | "のに" | "し" | "もん" | "じゃん"
            ))
        || matches!(t.surface.as_str(), "だ" | "です" | "だろ" | "だろう" | "でしょ" | "でしょう" | "な" | "じゃ")
}

/// Repairs systemic spoken-Japanese mis-parses caused by IPADIC's formal-newspaper training data
/// (い-dropped progressives + explanatory ん, swallowed particles like はね, colloquial contractions).
fn repair_colloquial_japanese(tokens: Vec<MorphToken>) -> Vec<MorphToken> {
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len() + 4);
    let mut i = 0;
    while i < tokens.len() {
        let cur = &tokens[i];
        let prev = out.last();
        let next = tokens.get(i + 1);

        // 1. Spoken い-dropped progressive + explanatory ん after て/で-form:
        //    待っ + て + たん + だ -> 待っ + て + た(助動詞) + ん(名詞,非自立) + だ
        //    読ん + で + るん + だ -> 読ん + で + る(動詞,非自立) + ん(名詞,非自立) + だ
        let after_te_de = prev.map_or(false, |p| {
            (p.pos == "助詞" && p.sub_pos == "接続助詞" && matches!(p.surface.as_str(), "て" | "で"))
                || (p.pos == "動詞" && (p.surface.ends_with('て') || p.surface.ends_with('で')))
        });
        if after_te_de
            && cur.pos == "名詞"
            && matches!(cur.surface.as_str(), "たん" | "るん")
            && is_clause_or_copula_follow(next)
        {
            if cur.surface == "たん" {
                out.push(MorphToken {
                    surface: "た".into(),
                    lemma: "た".into(),
                    pos: "助動詞".into(),
                    sub_pos: "*".into(),
                    reading: "た".into(),
                    ignore: true,
                    content: false,
                });
            } else {
                out.push(MorphToken {
                    surface: "る".into(),
                    lemma: "てる".into(),
                    pos: "動詞".into(),
                    sub_pos: "非自立".into(),
                    reading: "る".into(),
                    ignore: true,
                    content: false,
                });
            }
            out.push(MorphToken {
                surface: "ん".into(),
                lemma: "ん".into(),
                pos: "名詞".into(),
                sub_pos: "非自立".into(),
                reading: "ん".into(),
                ignore: true,
                content: false,
            });
            i += 1;
            continue;
        }

        // 2. Topic + final particle swallowed into verb はねる (e.g. 今日の服 + はね + 、 / これ + はね + 次回)
        let after_nominal = prev.map_or(false, |p| matches!(p.pos.as_str(), "名詞" | "助詞"));
        let next_is_verb_infl = next.map_or(false, |n| {
            (n.pos == "助動詞" && !matches!(n.lemma.as_str(), "だ" | "です"))
                || (n.pos == "助詞" && n.sub_pos == "接続助詞" && matches!(n.surface.as_str(), "て" | "ば" | "たら"))
                || (n.pos == "動詞" && matches!(n.sub_pos.as_str(), "接尾" | "非自立"))
        });
        if after_nominal
            && cur.surface == "はね"
            && cur.pos == "動詞"
            && cur.lemma == "はねる"
            && !next_is_verb_infl
        {
            out.push(MorphToken {
                surface: "は".into(),
                lemma: "は".into(),
                pos: "助詞".into(),
                sub_pos: "係助詞".into(),
                reading: "は".into(),
                ignore: true,
                content: false,
            });
            out.push(MorphToken {
                surface: "ね".into(),
                lemma: "ね".into(),
                pos: "助詞".into(),
                sub_pos: "終助詞".into(),
                reading: "ね".into(),
                ignore: true,
                content: false,
            });
            i += 1;
            continue;
        }

        // 3. Colloquial かもしんない (かも + しん[名詞] + ない)
        if cur.surface == "しん"
            && cur.pos == "名詞"
            && prev.map_or(false, |p| p.surface == "かも")
            && next.map_or(false, |n| n.lemma == "ない")
        {
            out.push(MorphToken {
                surface: "しん".into(),
                lemma: "しれる".into(),
                pos: "動詞".into(),
                sub_pos: "非自立".into(),
                reading: "しん".into(),
                ignore: true,
                content: false,
            });
            i += 1;
            continue;
        }

        // 4. Casual contracted copula です -> すね / すよ / すか (after ん/っ/noun/adj -> mis-parsed as 脛!)
        if cur.pos == "名詞"
            && matches!(cur.surface.as_str(), "すね" | "すよ" | "すか" | "すわ")
            && prev.map_or(false, |p| matches!(p.surface.as_str(), "ん" | "っ" | "の") || p.pos == "形容詞")
            && is_clause_or_copula_follow(next)
        {
            let mut tk = cur.clone();
            tk.pos = "助動詞".into();
            tk.lemma = "です".into();
            tk.ignore = true;
            tk.content = false;
            out.push(tk);
            i += 1;
            continue;
        }

        // 5. Western/colloquial negative past 〜んかった (読まん + かった[動詞/かる] -> 助動詞)
        if cur.surface == "かった"
            && cur.pos == "動詞"
            && cur.lemma == "かる"
            && prev.map_or(false, |p| p.surface == "ん" || p.surface.ends_with('ん'))
        {
            out.push(MorphToken {
                surface: "かった".into(),
                lemma: "ない".into(),
                pos: "助動詞".into(),
                sub_pos: "*".into(),
                reading: "かった".into(),
                ignore: true,
                content: false,
            });
            i += 1;
            continue;
        }

        // 6. Spoken ai/oi -> ee i-adjective contractions (すげ+え -> すげえ [lemma: すごい], やべ+え -> やばい, etc.)
        if let Some(n) = next {
            if matches!(n.surface.as_str(), "え" | "ぇ" | "ー") {
                let adj_canon = match cur.surface.as_str() {
                    "すげ" => Some("すごい"),
                    "やべ" => Some("やばい"),
                    "うめ" if cur.pos == "動詞" => Some("うまい"),
                    "でけ" => Some("でかい"),
                    "つえ" => Some("つよい"),
                    "うぜ" => Some("うざい"),
                    _ => None,
                };
                if let Some(canon) = adj_canon {
                    out.push(MorphToken {
                        surface: format!("{}{}", cur.surface, n.surface),
                        lemma: canon.into(),
                        pos: "形容詞".into(),
                        sub_pos: "自立".into(),
                        reading: format!("{}{}", cur.reading, n.reading),
                        ignore: false,
                        content: true,
                    });
                    i += 2;
                    continue;
                }
            }
        }

        // 7. Spoken contractions of basic grammar words: とこ (ところ after predicate) & あんま (あんまり, not 按摩)
        if cur.surface == "とこ"
            && cur.pos == "名詞"
            && prev.map_or(false, |p| matches!(p.pos.as_str(), "動詞" | "助動詞" | "形容詞" | "連体詞"))
        {
            let mut tk = cur.clone();
            tk.lemma = "ところ".into();
            tk.sub_pos = "非自立".into();
            tk.ignore = true;
            tk.content = false;
            out.push(tk);
            i += 1;
            continue;
        }
        if cur.surface == "あんま" && cur.pos == "名詞" {
            let mut tk = cur.clone();
            tk.lemma = "あんまり".into();
            tk.pos = "副詞".into();
            tk.sub_pos = "一般".into();
            tk.content = false;
            out.push(tk);
            i += 1;
            continue;
        }

        out.push(cur.clone());
        i += 1;
    }
    out
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

fn has_kanji_only(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || c == '々')
}

/// Merges IPADIC-split compound nouns, kanji prefixes (接頭詞+名詞 like 現+説 -> 現説, 不+自然 -> 不自然,
/// 無+意識 -> 無意識), numeric JMdict compounds (万+人 -> 万人, 一+瞬 -> 一瞬), and JMdict-validated
/// compounds & kanji suffixes (看護+婦 -> 看護婦, 診察+室 -> 診察室, 暴力+的 -> 暴力的).
fn merge_compound_nouns(tokens: Vec<MorphToken>, dict: Option<&LocalJmDict>) -> Vec<MorphToken> {
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let mut cur = tokens[i].clone();

        // Case A: Prefix (接頭詞,名詞接続) + Noun
        //   - Kanji prefix (現, 不, 無, 非, 未, 再, 反, 超, 最, 新, 旧, 元, 全, 各, 毎, etc.) + kanji/kata noun -> always merge!
        //   - Honorific kana prefix (お, ご) + noun -> merge when combined word exists in JMdict (お話, お金, ご飯).
        if cur.pos == "接頭詞" && i + 1 < tokens.len() {
            let next = &tokens[i + 1];
            if next.pos == "名詞"
                && !matches!(next.sub_pos.as_str(), "数" | "非自立" | "代名詞" | "特殊")
                && has_kanji_or_kata(&next.surface)
            {
                let merged = format!("{}{}", cur.surface, next.surface);
                let in_dict = dict.map(|d| d.has_exact_word(&merged)).unwrap_or(false);
                let is_kanji_prefix = has_kanji_only(&cur.surface);
                if is_kanji_prefix || in_dict {
                    cur.surface = merged.clone();
                    cur.lemma = merged;
                    cur.reading = format!("{}{}", cur.reading, next.reading);
                    cur.pos = "名詞".to_string();
                    cur.sub_pos = "一般".to_string();
                    cur.ignore = false;
                    cur.content = !is_wordmap_stopword(&cur.lemma);
                    i += 1;
                }
            }
        }

        // Case B: Single-kanji numeric noun (名詞,数 like 万, 一, 二, 半, 百, 千) + Noun when JMdict validates the compound (万人, 一瞬, 一生, 半分)
        if cur.pos == "名詞"
            && cur.sub_pos == "数"
            && has_kanji_only(&cur.surface)
            && cur.surface.chars().count() == 1
            && i + 1 < tokens.len()
        {
            let next = &tokens[i + 1];
            if next.pos == "名詞" && has_kanji_or_kata(&next.surface) {
                let merged = format!("{}{}", cur.surface, next.surface);
                if dict.map(|d| d.has_exact_word(&merged)).unwrap_or(false) {
                    cur.surface = merged.clone();
                    cur.lemma = merged;
                    cur.reading = format!("{}{}", cur.reading, next.reading);
                    cur.sub_pos = "一般".to_string();
                    cur.ignore = false;
                    cur.content = !is_wordmap_stopword(&cur.lemma);
                    i += 1;
                }
            }
        }

        // Case C: Chain JMdict-validated compound nouns & single-kanji nominal suffixes (婦, 室, 欲, 浴, 的, 化, 性, etc.)
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

/// Merges JMdict-validated kanji noun + verb compounds that IPADIC splits (e.g. 紐+づける -> 紐づける,
/// 位置+づける -> 位置づける, 関連+づける -> 関連づける, 傷+つける -> 傷つける).
fn merge_compound_verbs(tokens: Vec<MorphToken>, dict: Option<&LocalJmDict>) -> Vec<MorphToken> {
    let Some(d) = dict else { return tokens };
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if i + 1 < tokens.len() {
            let a = &tokens[i];
            let b = &tokens[i + 1];
            if a.pos == "名詞"
                && !matches!(a.sub_pos.as_str(), "数" | "非自立" | "代名詞" | "特殊")
                && has_kanji_only(&a.surface)
                && b.pos == "動詞"
                && b.sub_pos == "自立"
                && matches!(
                    b.lemma.as_str(),
                    "づける" | "づく" | "つける" | "つく" | "かける" | "だす" | "こむ" | "とる" | "きる"
                )
            {
                let merged_lemma = format!("{}{}", a.surface, b.lemma);
                if d.has_verb(&merged_lemma) {
                    out.push(MorphToken {
                        surface: format!("{}{}", a.surface, b.surface),
                        lemma: merged_lemma,
                        pos: "動詞".to_string(),
                        sub_pos: "自立".to_string(),
                        reading: format!("{}{}", a.reading, b.reading),
                        ignore: false,
                        content: true,
                    });
                    i += 2;
                    continue;
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

/// General Japanese predicate conjugation merger (活用語 + 活用語尾):
/// Attaches every bound inflectional morpheme of a verb or i-adjective (including basic verbs like
/// なる/ある/する and spoken contracted progressives like てる/てた/てて/ちゃう/とく so 待ってた,
/// 書かれてた, 食べてた, 怒られなかった never get sliced mid-conjugation!) into ONE cohesive
/// surface unit while preserving the canonical dictionary base form (`lemma`):
///   - `動詞,接尾` (causative せる/させる, passive/potential れる/られる)
///   - `動詞,非自立` spoken contractions (`てる`/`でる`/`ちゃう`/`じゃう`/`とく`/`どく`)
///   - `助動詞` (tense/negation/politeness/volitional: た/だ[lemma=た]/ない/ます/たい/ぬ/ず/う/よう/まい)
///   - Conjugational `助詞,接続助詞` (te-form て/で, conditionals ば/たら, たり, ちゃ/じゃ)
/// True independent clause conjunctions (ながら, から, ので, けど, のに) stay cleanly separate.
fn merge_verb_adj_conjugations(tokens: Vec<MorphToken>) -> Vec<MorphToken> {
    let mut out: Vec<MorphToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let mut cur = tokens[i].clone();
        let is_predicate = matches!(cur.pos.as_str(), "動詞" | "形容詞")
            && matches!(cur.sub_pos.as_str(), "自立" | "非自立");
        if is_predicate {
            while i + 1 < tokens.len() {
                let next = &tokens[i + 1];
                let is_bound_inflection = (next.pos == "動詞" && next.sub_pos == "接尾")
                    || (next.pos == "動詞"
                        && next.sub_pos == "非自立"
                        && matches!(
                            next.lemma.as_str(),
                            "てる" | "でる" | "ちゃう" | "じゃう" | "とく" | "どく"
                        ))
                    || (next.pos == "助動詞"
                        && !matches!(next.lemma.as_str(), "だ" | "です" | "らしい" | "みたい"))
                    || (next.pos == "助詞"
                        && next.sub_pos == "接続助詞"
                        && matches!(
                            next.surface.as_str(),
                            "て" | "で" | "ば" | "たら" | "だら" | "たり" | "だり" | "ちゃ" | "じゃ"
                        ));
                if is_bound_inflection {
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
