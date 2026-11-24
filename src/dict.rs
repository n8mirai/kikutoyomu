use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictSense {
    pub pos: Vec<String>,
    pub defs: Vec<String>,
    pub info: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictForm {
    pub word: String,
    pub reading: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub word: String,
    pub reading: String,
    pub is_common: bool,
    pub jlpt: Vec<String>,
    pub senses: Vec<DictSense>,
    pub other_forms: Vec<DictForm>,
}

// Compact in-memory JMdict representation (~210k words, < 0.1ms lookups!)
#[derive(Debug, Clone)]
struct CompactSense {
    pos: Vec<String>,
    defs: Vec<String>,
    uk: bool,
    arch_or_obs: bool,
}

#[derive(Debug, Clone)]
struct CompactWord {
    kanji: Vec<(String, bool, bool)>, // (text, common, rare)
    kana: Vec<(String, bool, bool)>,  // (text, common, rare)
    senses: Vec<CompactSense>,
}

#[derive(Default)]
pub struct LocalJmDict {
    words: Vec<CompactWord>,
    kanji_index: HashMap<String, Vec<u32>>,
    kana_index: HashMap<String, Vec<u32>>,
}

#[derive(Deserialize)]
struct RawJmWord {
    #[serde(default)]
    kanji: Vec<RawJmForm>,
    #[serde(default)]
    kana: Vec<RawJmForm>,
    #[serde(default)]
    sense: Vec<RawJmSense>,
}

#[derive(Deserialize)]
struct RawJmForm {
    text: String,
    #[serde(default)]
    common: bool,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct RawJmSense {
    #[serde(default, rename = "partOfSpeech")]
    part_of_speech: Vec<String>,
    #[serde(default)]
    misc: Vec<String>,
    #[serde(default)]
    gloss: Vec<RawJmGloss>,
}

#[derive(Deserialize)]
struct RawJmGloss {
    text: String,
}

fn friendly_pos(code: &str) -> &'static str {
    match code {
        "n" | "n-t" | "n-adv" | "n-pref" | "n-suf" => "Noun",
        "vs" | "vs-i" | "vs-s" => "Suru verb",
        "v1" | "v1-s" | "vz" => "Ichidan verb",
        "v5u" | "v5u-s" | "v5k" | "v5k-s" | "v5g" | "v5s" | "v5t" | "v5n" | "v5b" | "v5m"
        | "v5r" | "v5r-i" | "v5aru" | "v5uru" => "Godan verb",
        "vk" => "Kuru verb",
        "vi" => "Intransitive",
        "vt" => "Transitive",
        "adj-i" | "adj-ix" => "i-adjective",
        "adj-na" => "na-adjective",
        "adj-no" => "no-adjective",
        "adj-pn" => "Pre-noun adj",
        "adv" | "adv-to" => "Adverb",
        "exp" => "Expression",
        "pn" => "Pronoun",
        "prt" => "Particle",
        "conj" => "Conjunction",
        "int" => "Interjection",
        "aux" | "aux-v" | "aux-adj" => "Auxiliary",
        "pref" => "Prefix",
        "suf" => "Suffix",
        "ctr" => "Counter",
        _ => "",
    }
}

fn is_verb_pos(pos: &[String]) -> bool {
    pos.iter()
        .any(|p| p.starts_with('v') && p != "vidg" && p != "vulg")
}

fn is_noun_pos(pos: &[String]) -> bool {
    pos.iter().any(|p| p == "n" || p.starts_with("n-"))
}

fn is_suru_pos(pos: &[String]) -> bool {
    pos.iter().any(|p| p == "vs" || p == "vs-i" || p == "vs-s")
}

fn is_adj_i_pos(pos: &[String]) -> bool {
    pos.iter().any(|p| p == "adj-i" || p == "adj-ix")
}

fn is_adj_na_pos(pos: &[String]) -> bool {
    pos.iter().any(|p| p == "adj-na")
}

fn pos_hint_bonus(pos_hint: &str, all_pos: &[String]) -> i32 {
    match pos_hint {
        "動詞" => {
            if is_verb_pos(all_pos) { 42 } else { -20 }
        }
        "名詞" => {
            if is_noun_pos(all_pos) || is_adj_na_pos(all_pos) { 36 } else { -12 }
        }
        "形容詞" => {
            if is_adj_i_pos(all_pos) { 42 } else { -18 }
        }
        "副詞" => {
            if all_pos.iter().any(|p| p == "adv" || p == "adv-to") { 38 } else { -10 }
        }
        _ => 0,
    }
}

fn has_kanji(s: &str) -> bool {
    s.chars()
        .any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || c == '々')
}

fn is_pure_kana(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            ('\u{3040}'..='\u{309F}').contains(&c)
                || ('\u{30A0}'..='\u{30FF}').contains(&c)
                || c == 'ー'
        })
}

fn is_verb_continuation(after: &str) -> bool {
    const AUX: &[&str] = &[
        "ます", "まし", "ませ", "ない", "なく", "なか", "て", "で", "た", "だ", "れる", "られ",
        "せる", "させ", "たい", "たく", "よう", "ながら", "つつ", "ぬ", "ず",
    ];
    AUX.iter().any(|a| after.starts_with(a))
}

impl LocalJmDict {
    /// Streams jmdict-eng.json line-by-line (each word entry is one line inside `"words": [...]`).
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Cannot open JMdict: {}", e))?;
        let reader = BufReader::with_capacity(1 << 20, file);

        let mut dict = LocalJmDict {
            words: Vec::with_capacity(215_000),
            kanji_index: HashMap::with_capacity(220_000),
            kana_index: HashMap::with_capacity(240_000),
        };

        let mut in_words = false;
        for line_res in reader.lines() {
            let Ok(line) = line_res else { continue };
            let t = line.trim();
            if !in_words {
                if t.starts_with("\"words\"") {
                    in_words = true;
                }
                continue;
            }
            if t.starts_with(']') {
                break;
            }
            if !t.starts_with('{') {
                continue;
            }
            let json_str = t.trim_end_matches(',');
            let Ok(raw) = serde_json::from_str::<RawJmWord>(json_str) else {
                continue;
            };

            let id = dict.words.len() as u32;
            let mut kanji = Vec::with_capacity(raw.kanji.len());
            for k in raw.kanji {
                let rare = k.tags.iter().any(|tg| tg == "rK" || tg == "sK" || tg == "iK" || tg == "oK");
                dict.kanji_index.entry(k.text.clone()).or_default().push(id);
                kanji.push((k.text, k.common, rare));
            }
            let mut kana = Vec::with_capacity(raw.kana.len());
            for r in raw.kana {
                let rare = r.tags.iter().any(|tg| tg == "rk" || tg == "sk" || tg == "ik" || tg == "ok");
                dict.kana_index.entry(r.text.clone()).or_default().push(id);
                kana.push((r.text, r.common, rare));
            }
            let mut senses = Vec::with_capacity(raw.sense.len());
            for s in raw.sense {
                let defs: Vec<String> = s.gloss.into_iter().map(|g| g.text).collect();
                if defs.is_empty() {
                    continue;
                }
                let uk = s.misc.iter().any(|m| m == "uk");
                let arch_or_obs = s.misc.iter().any(|m| m == "arch" || m == "obs" || m == "rare");
                senses.push(CompactSense {
                    pos: s.part_of_speech,
                    defs,
                    uk,
                    arch_or_obs,
                });
            }
            if !senses.is_empty() {
                dict.words.push(CompactWord {
                    kanji,
                    kana,
                    senses,
                });
            }
        }
        Ok(dict)
    }

    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// O(1) check whether `word` is marked as a common vocabulary entry in JMdict.
    pub fn is_common_word(&self, word: &str) -> bool {
        if let Some(ids) = self.kanji_index.get(word) {
            for &id in ids {
                if let Some(w) = self.words.get(id as usize) {
                    if w.kanji.iter().any(|(t, c, _)| t == word && *c)
                        || w.kana.iter().any(|(_, c, _)| *c)
                    {
                        return true;
                    }
                }
            }
        }
        if let Some(ids) = self.kana_index.get(word) {
            for &id in ids {
                if let Some(w) = self.words.get(id as usize) {
                    if w.kana.iter().any(|(t, c, _)| t == word && *c) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn has_exact_word(&self, term: &str) -> bool {
        self.kanji_index.contains_key(term) || self.kana_index.contains_key(term)
    }

    fn get_ids(&self, term: &str) -> Vec<u32> {
        let mut out = Vec::new();
        if let Some(ids) = self.kanji_index.get(term) {
            out.extend_from_slice(ids);
        }
        if let Some(ids) = self.kana_index.get(term) {
            for &id in ids {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// Yomitan-grade longest-prefix + MeCab lemma/POS hints + multi-step de-inflection + grammatical context ranking,
    /// seamlessly backed by JMnedict for Japanese proper names (places, surnames, characters, works)!
    pub fn lookup(
        &self,
        surface: &str,
        ctx: &str,
        lemma_hint: &str,
        pos_hint: &str,
        names: Option<&LocalNameDict>,
    ) -> Vec<DictEntry> {
        let strip_chars = |c: char| {
            matches!(
                c,
                '。' | '、'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '！'
                    | '？'
                    | '…'
                    | '・'
                    | '（'
                    | '）'
                    | '.'
                    | ','
                    | '!'
                    | '?'
            )
        };
        let q = surface.trim().trim_matches(strip_chars);
        let lh = lemma_hint.trim().trim_matches(strip_chars);
        if q.is_empty() && lh.is_empty() {
            return Vec::new();
        }
        let q_eff = if q.is_empty() { lh } else { q };
        let ctx_clean = ctx.trim().trim_start_matches(strip_chars);
        let search_str = if ctx_clean.starts_with(q_eff) {
            ctx_clean
        } else {
            q_eff
        };
        let search_chars: Vec<char> = search_str.chars().collect();
        let q_len = q_eff.chars().count();
        let max_len = search_chars.len().min(12);

        // Pass 0 (MeCab Exact Mode): When MeCab/IPADIC gave us an exact morphological lemma & POS
        // (e.g. surface="もっと", lemma="もっと", pos="副詞" inside "もっともっと好きになる"),
        // look up that exact token FIRST without greedily stealing characters across the next word boundary!
        let mut best_scores: HashMap<u32, i32> = HashMap::new();
        if !lh.is_empty() {
            let mut exact_terms: Vec<(&str, i32)> = vec![(lh, 64)];
            if !q.is_empty() && q != lh {
                exact_terms.push((q, 32));
            }
            for (term, base_bonus) in exact_terms {
                let t_len = term.chars().count();
                let t_kana = is_pure_kana(term);
                let t_kanji = has_kanji(term);
                for id in self.get_ids(term) {
                    let w = &self.words[id as usize];
                    let all_pos: Vec<String> = w.senses.iter().flat_map(|s| s.pos.iter().cloned()).collect();
                    let mut sc: i32 = (t_len as i32) * 32 + base_bonus;
                    let exact_kanji = w.kanji.iter().find(|(t, _, _)| t == term);
                    let exact_kana = w.kana.iter().find(|(t, _, _)| t == term);
                    if let Some((_, common, rare)) = exact_kanji {
                        sc += 80 + if *common { 34 } else { 0 } - if *rare { 35 } else { 0 };
                    } else if let Some((_, common, rare)) = exact_kana {
                        sc += (if t_kana { 76 } else { 30 }) + if *common { 34 } else { 0 } - if *rare { 35 } else { 0 };
                    }
                    if w.kanji.iter().any(|x| x.1) || w.kana.iter().any(|x| x.1) {
                        sc += 24;
                    }
                    if t_kana && w.senses.iter().any(|s| s.uk) {
                        sc += 34;
                    }
                    if t_kanji && exact_kanji.is_some() {
                        sc += 20;
                    }
                    if w.senses.iter().all(|s| s.arch_or_obs) {
                        sc -= 45;
                    }
                    sc += pos_hint_bonus(pos_hint, &all_pos);
                    let prev = best_scores.entry(id).or_insert(i32::MIN);
                    if sc > *prev {
                        *prev = sc;
                    }
                }
            }
            // Also check de-inflected forms of exact surface (without crossing into the next word)
            if best_scores.is_empty() && !q.is_empty() {
                let q_chars: Vec<char> = q.chars().collect();
                best_scores = self.score_range(&q_chars, q_chars.len(), q_chars.len(), q_chars.len(), pos_hint, &strip_chars);
            }
            // If MeCab gave us a proper noun (e.g. 中山) not in JMdict, resolve via JMnedict immediately!
            if best_scores.is_empty() {
                if let Some(nd) = names {
                    let hits = nd.lookup_exact(lh);
                    if !hits.is_empty() {
                        return hits;
                    }
                    if !q.is_empty() && q != lh {
                        let hits2 = nd.lookup_exact(q);
                        if !hits2.is_empty() {
                            return hits2;
                        }
                    }
                }
            }
        }

        // Pass 1 (Fallback / Compound Scan): Only scan across `ctx` if MeCab exact lookup found nothing
        // (e.g. split compound like 使い + 尽くして, or lookups without MeCab lemma_hint).
        if best_scores.is_empty() {
            best_scores = self.score_range(&search_chars, q_len, max_len, q_len, pos_hint, &strip_chars);
        }

        // Pass 2: If still no general JMdict entry at >= q_len, check JMnedict spans, then shorter prefixes
        if best_scores.is_empty() {
            if let Some(nd) = names {
                let name_hits = nd.lookup_span(&search_chars, q_len, max_len, &strip_chars);
                if !name_hits.is_empty() {
                    return name_hits;
                }
            }
            if q_len > 1 {
                best_scores = self.score_range(&search_chars, 1, q_len - 1, q_len, pos_hint, &strip_chars);
            }
        }

        let mut ranked: Vec<(i32, u32)> = best_scores.into_iter().map(|(id, sc)| (sc, id)).collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

        let mut out: Vec<DictEntry> = ranked
            .into_iter()
            .take(5)
            .map(|(_sc, id)| self.to_dict_entry(&self.words[id as usize], q_eff))
            .collect();

        // If JMnedict also has an exact proper-name match for a 2+ char kanji surface, append as an extra sense
        if q_len >= 2 && has_kanji(q_eff) && out.len() < 5 {
            if let Some(nd) = names {
                if let Some(n_entry) = nd.lookup_exact(q_eff).into_iter().next() {
                    if !out.iter().any(|e| e.word == n_entry.word && e.reading == n_entry.reading) {
                        out.push(n_entry);
                    }
                }
            }
        }

        out
    }

    fn score_range(
        &self,
        search_chars: &[char],
        min_len: usize,
        max_len: usize,
        q_len: usize,
        pos_hint: &str,
        strip_chars: &dyn Fn(char) -> bool,
    ) -> HashMap<u32, i32> {
        let mut best_scores: HashMap<u32, i32> = HashMap::new();

        for len in (min_len..=max_len).rev() {
            let sub: String = search_chars[..len].iter().collect();
            if sub.chars().any(strip_chars) {
                continue;
            }
            let after: String = search_chars[len..].iter().collect();
            let sub_is_kana = is_pure_kana(&sub);
            let sub_has_kanji = has_kanji(&sub);
            let verb_after = is_verb_continuation(&after);

            // Check exact form + all de-inflected forms of `sub`
            let cands = deinflect_multi(&sub);
            for (cand, deinf_mode, expect_kind) in cands {
                let ids = self.get_ids(&cand);
                if ids.is_empty() {
                    continue;
                }
                for id in ids {
                    let w = &self.words[id as usize];
                    // If de-inflected as a verb/adj, verify the JMdict entry actually HAS that POS!
                    let all_pos: Vec<String> = w
                        .senses
                        .iter()
                        .flat_map(|s| s.pos.iter().cloned())
                        .collect();
                    let is_v = is_verb_pos(&all_pos);
                    let is_n = is_noun_pos(&all_pos);
                    let is_vs = is_suru_pos(&all_pos);
                    let is_ai = is_adj_i_pos(&all_pos);
                    let is_ana = is_adj_na_pos(&all_pos);

                    if deinf_mode != DeinflectMode::Exact {
                        match expect_kind {
                            DeinflectKind::Verb if !is_v => continue,
                            DeinflectKind::AdjI if !is_ai => continue,
                            DeinflectKind::None => {}
                            _ => {}
                        }
                    }

                    // Score this match!
                    let mut sc: i32 = 0;

                    // 1. Span length in actual sentence (longer real compound/verb wins!)
                    sc += (len as i32) * 32;
                    if len == q_len {
                        sc += 18;
                    }

                    // 2. Exact kanji/kana form quality
                    let exact_kanji = w.kanji.iter().find(|(t, _, _)| t == &cand);
                    let exact_kana = w.kana.iter().find(|(t, _, _)| t == &cand);
                    if let Some((_, common, rare)) = exact_kanji {
                        sc += 80;
                        if *common {
                            sc += 34;
                        }
                        if *rare {
                            sc -= 35;
                        }
                    } else if let Some((_, common, rare)) = exact_kana {
                        sc += if sub_is_kana { 76 } else { 30 };
                        if *common {
                            sc += 34;
                        }
                        if *rare {
                            sc -= 35;
                        }
                    }

                    // Any common form bonus
                    let any_common = w.kanji.iter().any(|x| x.1) || w.kana.iter().any(|x| x.1);
                    if any_common {
                        sc += 24;
                    }

                    // Kana-only ("uk") preference when sentence uses pure kana
                    let uk = w.senses.iter().any(|s| s.uk);
                    if sub_is_kana && uk {
                        sc += 30;
                    }
                    if sub_has_kanji && exact_kanji.is_some() {
                        sc += 20;
                    }

                    // Penalize archaic/obsolete senses
                    if w.senses.iter().all(|s| s.arch_or_obs) {
                        sc -= 45;
                    }
                    sc += pos_hint_bonus(pos_hint, &all_pos);

                    // 3. Grammatical context sensitivity!
                    let noun_after = after.starts_with('の')
                        || after.starts_with('を')
                        || after.starts_with('が')
                        || after.starts_with('は')
                        || after.starts_with('へ')
                        || after.starts_with('で')
                        || after.starts_with('に')
                        || after.starts_with('も')
                        || after.starts_with("です")
                        || after.starts_with("だ");

                    match deinf_mode {
                        DeinflectMode::Conjugated => {
                            // Genuine conjugated verb/adj matched across `sub` (e.g. 使い尽くして -> 使い尽くす, 始めました -> 始める, 行かなかった -> 行く)
                            sc += 48;
                        }
                        DeinflectMode::Stem => {
                            // Bare stem (e.g. 始め -> 始める, 暮らし -> 暮らす): only boost when followed by verb aux; penalize when clearly a noun!
                            if verb_after {
                                sc += 30;
                            } else if noun_after {
                                sc -= 28;
                            } else {
                                sc -= 12;
                            }
                        }
                        DeinflectMode::Exact => {
                            if verb_after {
                                if is_v {
                                    sc += 35;
                                } else if is_n && !is_vs {
                                    sc -= 30;
                                }
                            } else if after.starts_with("する")
                                || after.starts_with("し")
                                || after.starts_with("さ")
                                || after.starts_with("でき")
                            {
                                if is_vs {
                                    sc += 42;
                                }
                            } else if noun_after {
                                if is_n {
                                    sc += 36;
                                }
                            } else if after.starts_with('な') {
                                if is_ana {
                                    sc += 32;
                                }
                            } else if after.starts_with('く') || after.starts_with("かった") {
                                if is_ai {
                                    sc += 30;
                                }
                            }
                        }
                    }

                    let prev = best_scores.entry(id).or_insert(i32::MIN);
                    if sc > *prev {
                        *prev = sc;
                    }
                }
            }
        }
        best_scores
    }

    fn to_dict_entry(&self, w: &CompactWord, fallback: &str) -> DictEntry {
        // Pick primary kanji (preferring common, non-rare) and primary kana
        let best_kanji = w
            .kanji
            .iter()
            .find(|x| x.1 && !x.2)
            .or_else(|| w.kanji.iter().find(|x| !x.2))
            .or_else(|| w.kanji.first());
        let best_kana = w
            .kana
            .iter()
            .find(|x| x.1 && !x.2)
            .or_else(|| w.kana.iter().find(|x| !x.2))
            .or_else(|| w.kana.first());

        let uk_all = w.senses.first().map(|s| s.uk).unwrap_or(false);
        let reading = best_kana.map(|k| k.0.clone()).unwrap_or_default();
        let word = if uk_all && best_kanji.is_none() {
            reading.clone()
        } else {
            best_kanji
                .map(|k| k.0.clone())
                .unwrap_or_else(|| if reading.is_empty() { fallback.to_string() } else { reading.clone() })
        };

        let is_common = w.kanji.iter().any(|x| x.1) || w.kana.iter().any(|x| x.1);
        let mut other_forms = Vec::new();
        for (k, _, rare) in &w.kanji {
            if !*rare && k != &word {
                other_forms.push(DictForm {
                    word: k.clone(),
                    reading: reading.clone(),
                });
                if other_forms.len() >= 4 {
                    break;
                }
            }
        }

        let senses = w
            .senses
            .iter()
            .take(6)
            .map(|s| {
                let mut pos_labels = Vec::new();
                for p in &s.pos {
                    let lbl = friendly_pos(p);
                    if !lbl.is_empty() && !pos_labels.contains(&lbl.to_string()) {
                        pos_labels.push(lbl.to_string());
                    }
                }
                DictSense {
                    pos: pos_labels,
                    defs: s.defs.clone(),
                    info: Vec::new(),
                }
            })
            .collect();

        DictEntry {
            word,
            reading,
            is_common,
            jlpt: Vec::new(),
            senses,
            other_forms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeinflectKind {
    None,
    Verb,
    AdjI,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeinflectMode {
    Exact,
    Conjugated,
    Stem,
}

/// Fast multi-step Japanese verb/adjective de-inflector (handles polite, past, te-form, negative,
/// passive, causative, potential, tai-form, i-adj inflections, and masu-stems!).
fn deinflect_multi(surface: &str) -> Vec<(String, DeinflectMode, DeinflectKind)> {
    let mut results: Vec<(String, DeinflectMode, DeinflectKind)> =
        vec![(surface.to_string(), DeinflectMode::Exact, DeinflectKind::None)];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(surface.to_string());

    // Rules: (inflected_suffix, replacement_endings, mode, kind)
    const RULES: &[(&str, &[&str], DeinflectMode, DeinflectKind)] = &[
        // Polite & compound verb endings
        ("ませんでした", &["る", "う", "く", "ぐ", "す", "つ", "ぬ", "ぶ", "む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ました", &["る", "う", "く", "ぐ", "す", "つ", "ぬ", "ぶ", "む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ません", &["る", "う", "く", "ぐ", "す", "つ", "ぬ", "ぶ", "む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ます", &["る", "う", "く", "ぐ", "す", "つ", "ぬ", "ぶ", "む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ましょう", &["る", "う", "く", "ぐ", "す", "つ", "ぬ", "ぶ", "む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ている", &["る", "う", "く", "す", "つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ていた", &["る", "う", "く", "す", "つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ていました", &["る", "う", "く", "す", "つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("てきた", &["る", "う", "く", "す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("てきます", &["る", "う", "く", "す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("てきました", &["る", "う", "く", "す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        // Godan past-negative & negative-te (わなかった / かなかった / etc.)
        ("わなかった", &["う"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("かなかった", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("がなかった", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("さなかった", &["す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("たなかった", &["つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ばなかった", &["ぶ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("まなかった", &["む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("らなかった", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("わなくて", &["う"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("かなくて", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("がなくて", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("さなくて", &["す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("たなくて", &["つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ばなくて", &["ぶ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("まなくて", &["む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("らなくて", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("なかった", &["る", "う", "く", "ぐ", "す", "つ", "ぬ", "ぶ", "む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("なくて", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("られない", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("られた", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("られる", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("させた", &["る", "う", "く", "す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("させる", &["る", "う", "く", "す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("された", &["する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("される", &["する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("しない", &["する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("しなかった", &["する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("しました", &["する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("します", &["する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        // Godan & Ichidan te / ta / tari / tara forms
        ("って", &["う", "つ", "る", "く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("った", &["う", "つ", "る", "く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ったら", &["う", "つ", "る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("いて", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("いた", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("いで", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("いだ", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("して", &["す", "する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("した", &["す", "する"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("んで", &["む", "ぶ", "ぬ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("んだ", &["む", "ぶ", "ぬ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        // Godan negative, passive, causative
        ("わない", &["う"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("かない", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("がない", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("さない", &["す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("たない", &["つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ばない", &["ぶ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("まない", &["む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("らない", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("われる", &["う"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("かれる", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("がれる", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("される", &["す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("たれる", &["つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ばれる", &["ぶ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("まれる", &["む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("られる", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("わせる", &["う"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("かせる", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("がせる", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("させる", &["す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("たせる", &["つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ばせる", &["ぶ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ませる", &["む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("らせる", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        // Godan conditional (-eba) & Ichidan general endings
        ("えば", &["う"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("けば", &["く"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("げば", &["ぐ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("せば", &["す"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("てば", &["つ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("べば", &["ぶ"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("めば", &["む"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("れば", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("て", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("た", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("ない", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("たい", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        ("よう", &["る"], DeinflectMode::Conjugated, DeinflectKind::Verb),
        // Bare stems (Godan i-stem & Ichidan stem — ambiguous with nominalized stems!)
        ("い", &["う"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("き", &["く"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("ぎ", &["ぐ"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("し", &["す"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("ち", &["つ"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("に", &["ぬ"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("び", &["ぶ"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("み", &["む"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("り", &["る"], DeinflectMode::Stem, DeinflectKind::Verb),
        ("", &["る"], DeinflectMode::Stem, DeinflectKind::Verb),
        // i-adjective inflections
        ("くなかった", &["い"], DeinflectMode::Conjugated, DeinflectKind::AdjI),
        ("くない", &["い"], DeinflectMode::Conjugated, DeinflectKind::AdjI),
        ("かった", &["い"], DeinflectMode::Conjugated, DeinflectKind::AdjI),
        ("くて", &["い"], DeinflectMode::Conjugated, DeinflectKind::AdjI),
        ("ければ", &["い"], DeinflectMode::Conjugated, DeinflectKind::AdjI),
        ("く", &["い"], DeinflectMode::Stem, DeinflectKind::AdjI),
        ("さ", &["い"], DeinflectMode::Stem, DeinflectKind::AdjI),
    ];

    for &(suffix, repls, mode, kind) in RULES {
        if suffix.is_empty() {
            // Only append る to stems that end in an e-row or i-row kana (e.g. 始め -> 始める)
            if let Some(last) = surface.chars().last() {
                let is_ichidan_stem_kana = "えけせてねへめれげぜでべぺいきしちにひみりぎじぢびぴ".contains(last);
                if is_ichidan_stem_kana && surface.chars().count() >= 2 {
                    let cand = format!("{}る", surface);
                    if seen.insert(cand.clone()) {
                        results.push((cand, mode, kind));
                    }
                }
            }
            continue;
        }
        if let Some(stem) = surface.strip_suffix(suffix) {
            if stem.chars().count() >= 1 {
                for &r in repls {
                    let cand = format!("{}{}", stem, r);
                    if seen.insert(cand.clone()) {
                        results.push((cand, mode, kind));
                    }
                }
            }
        }
    }

    // Second pass for chained endings like 使い尽くしていました -> 使い尽くして -> 使い尽くす
    // or 始めました -> 始め -> 始める
    let first_pass: Vec<(String, DeinflectMode, DeinflectKind)> = results
        .iter()
        .skip(1)
        .filter(|(s, m, _)| *m == DeinflectMode::Conjugated && s.chars().count() >= 2)
        .map(|(s, m, k)| (s.clone(), *m, *k))
        .collect();

    const STEP2: &[(&str, &[&str], DeinflectKind)] = &[
        ("い", &["う"], DeinflectKind::Verb),
        ("き", &["く"], DeinflectKind::Verb),
        ("ぎ", &["ぐ"], DeinflectKind::Verb),
        ("し", &["す"], DeinflectKind::Verb),
        ("ち", &["つ"], DeinflectKind::Verb),
        ("び", &["ぶ"], DeinflectKind::Verb),
        ("み", &["む"], DeinflectKind::Verb),
        ("り", &["る"], DeinflectKind::Verb),
        ("っ", &["う", "つ", "る"], DeinflectKind::Verb),
        ("ん", &["む", "ぶ", "ぬ"], DeinflectKind::Verb),
        ("れ", &["る"], DeinflectKind::Verb),
        ("せ", &["す", "る"], DeinflectKind::Verb),
    ];
    for (mid, _, _) in first_pass {
        if let Some(base) = mid.strip_suffix('る') {
            for &(suf, repls, kind) in STEP2 {
                if let Some(st) = base.strip_suffix(suf) {
                    if st.chars().count() >= 1 {
                        for &r in repls {
                            let cand = format!("{}{}", st, r);
                            if seen.insert(cand.clone()) {
                                results.push((cand, DeinflectMode::Conjugated, kind));
                            }
                        }
                    }
                }
            }
        }
    }

    results
}

/* ================= COMPACT LOCAL JMNEDICT (740K JAPANESE PROPER NAMES) ================= */
#[derive(Debug, Clone)]
struct CompactName {
    reading: String,
    kind: String,
    gloss: String,
    priority: u8,
}

#[derive(Default)]
pub struct LocalNameDict {
    by_surface: HashMap<String, Vec<CompactName>>,
    count: usize,
}

#[derive(Deserialize)]
struct RawNmWord {
    #[serde(default)]
    kanji: Vec<RawNmText>,
    #[serde(default)]
    kana: Vec<RawNmText>,
    #[serde(default)]
    translation: Vec<RawNmTrans>,
}

#[derive(Deserialize)]
struct RawNmText {
    text: String,
}

#[derive(Deserialize)]
struct RawNmTrans {
    #[serde(default, rename = "type")]
    kind: Vec<String>,
    #[serde(default)]
    translation: Vec<RawNmText>,
}

fn friendly_name_type(code: &str) -> (&'static str, u8) {
    match code {
        "place" => ("Place", 40),
        "surname" => ("Surname", 36),
        "person" => ("Person", 30),
        "work" => ("Work", 26),
        "station" => ("Station", 24),
        "fict" | "char" => ("Character", 25),
        "company" => ("Company", 22),
        "organization" | "group" => ("Org", 20),
        "product" => ("Product", 18),
        "myth" | "leg" | "dei" => ("Myth/Deity", 22),
        "given" | "masc" | "fem" => ("Given name", 14),
        _ => ("Proper noun", 8),
    }
}

impl LocalNameDict {
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Cannot open JMnedict: {}", e))?;
        let reader = BufReader::with_capacity(1 << 20, file);

        let mut dict = LocalNameDict {
            by_surface: HashMap::with_capacity(320_000),
            count: 0,
        };

        let mut in_words = false;
        for line_res in reader.lines() {
            let Ok(line) = line_res else { continue };
            let t = line.trim();
            if !in_words {
                if t.starts_with("\"words\"") {
                    in_words = true;
                }
                continue;
            }
            if t.starts_with(']') {
                break;
            }
            if !t.starts_with('{') {
                continue;
            }
            let json_str = t.trim_end_matches(',');
            let Ok(raw) = serde_json::from_str::<RawNmWord>(json_str) else {
                continue;
            };
            let Some(reading) = raw.kana.first().map(|k| k.text.clone()) else {
                continue;
            };
            let mut prio_sum: u16 = 0;
            let mut kinds: Vec<&'static str> = Vec::new();
            let mut glosses: Vec<String> = Vec::new();
            for tr in raw.translation {
                for tc in &tr.kind {
                    let (lbl, pr) = friendly_name_type(tc);
                    if !kinds.contains(&lbl) {
                        kinds.push(lbl);
                        prio_sum = prio_sum.saturating_add(pr as u16);
                    }
                }
                for g in tr.translation {
                    if !g.text.is_empty() && !glosses.contains(&g.text) && glosses.len() < 3 {
                        glosses.push(g.text);
                    }
                }
            }
            if glosses.is_empty() {
                continue;
            }
            // Prefer native Japanese readings when a name has both Place & Surname (e.g. 中山 なかやま vs ちゅうざん)
            if kinds.contains(&"Place") && kinds.contains(&"Surname") {
                prio_sum = prio_sum.saturating_add(25);
            }
            let best_prio: u8 = prio_sum.min(255) as u8;
            let kind = if kinds.is_empty() {
                "Proper noun".to_string()
            } else {
                kinds.join(", ")
            };
            let gloss = glosses.join("; ");

            // Index kanji forms (or kana form if pure katakana/kana proper name like ゴン)
            let surfaces: Vec<String> = if !raw.kanji.is_empty() {
                raw.kanji.into_iter().map(|k| k.text).collect()
            } else if reading.chars().count() >= 2 {
                vec![reading.clone()]
            } else {
                continue;
            };

            for surf in surfaces {
                if surf.chars().count() < 2 {
                    continue;
                }
                let slot = dict.by_surface.entry(surf).or_default();
                // Keep at most 4 highest-priority readings per surface; merge same reading cumulatively!
                if let Some(existing) = slot.iter_mut().find(|x| x.reading == reading) {
                    existing.priority = existing.priority.saturating_add(best_prio);
                    for k in kind.split(", ") {
                        if !existing.kind.split(", ").any(|ek| ek == k) {
                            existing.kind.push_str(", ");
                            existing.kind.push_str(k);
                        }
                    }
                    if !existing.gloss.contains(&gloss) && existing.gloss.len() < 60 {
                        existing.gloss.push_str("; ");
                        existing.gloss.push_str(&gloss);
                    }
                } else if slot.len() < 4 {
                    slot.push(CompactName {
                        reading: reading.clone(),
                        kind: kind.clone(),
                        gloss: gloss.clone(),
                        priority: best_prio,
                    });
                    dict.count += 1;
                } else if let Some((min_idx, min_item)) =
                    slot.iter().enumerate().min_by_key(|(_, x)| x.priority)
                {
                    if best_prio > min_item.priority {
                        slot[min_idx] = CompactName {
                            reading: reading.clone(),
                            kind: kind.clone(),
                            gloss: gloss.clone(),
                            priority: best_prio,
                        };
                    }
                }
            }
        }
        Ok(dict)
    }

    pub fn name_count(&self) -> usize {
        self.count
    }

    pub fn lookup_exact(&self, term: &str) -> Vec<DictEntry> {
        let Some(items) = self.by_surface.get(term) else {
            return Vec::new();
        };
        let mut sorted = items.clone();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.reading.cmp(&b.reading)));
        sorted
            .into_iter()
            .take(4)
            .map(|it| DictEntry {
                word: term.to_string(),
                reading: it.reading,
                is_common: false,
                jlpt: Vec::new(),
                senses: vec![DictSense {
                    pos: vec![it.kind],
                    defs: vec![it.gloss],
                    info: Vec::new(),
                }],
                other_forms: Vec::new(),
            })
            .collect()
    }

    pub fn lookup_span(
        &self,
        search_chars: &[char],
        min_len: usize,
        max_len: usize,
        strip_chars: &dyn Fn(char) -> bool,
    ) -> Vec<DictEntry> {
        for len in (min_len..=max_len).rev() {
            let sub: String = search_chars[..len].iter().collect();
            if sub.chars().any(strip_chars) {
                continue;
            }
            let hits = self.lookup_exact(&sub);
            if !hits.is_empty() {
                return hits;
            }
        }
        Vec::new()
    }
}
