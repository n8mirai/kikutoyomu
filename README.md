# Kiku to Yomu (聴く・読む)

A minimal, responsive Japanese audio immersion & interactive subtitle desk written in pure Rust.

## Features

- **MeCab / IPADIC Morphological Parser (`vibrato`)**: Accurate word segmentation, dictionary base forms (lemmas), contextual readings, and part-of-speech tagging (handles hiragana verbs, conjugated forms, particles, and auxiliaries cleanly).
- **Offline 219k JMdict + 715k JMnedict Dictionary**: Instant (<1 ms) `Shift + Hover` floating popout dictionary supporting general vocabulary, de-inflected verbs/adjectives, and Japanese proper names (places, surnames, characters), complete with **1–5★ Zipf-standardized series frequency ranks**.
- **Krashen i+1 Heatmap (`T1` / `T2` / `T3`)**: Code-diff style high-contrast highlighting on target unknown words inside `T1` (emerald + surfacing water-droplet chime), `T2` (warm gold), and `T3` (softer gold) sentences, with single-click & `1`/`2`/`3`/`4` vocab tagging.
- **Top-Down `浮かぶ` Reader & Ambient Idle Veil**: Continuous smooth scrolling with top/bottom surface fade optics, `Auto` scroll toggle (pauses while interacting), and a focus-aware ambient subtitle overlay when sitting idle on menus or the Word Map.
- **Series & Episode Word Frequency Map**: Lemmatized frequency map of proper content words (nouns, verbs, adjectives) with live `% known` coverage.
- **Audio Desk & Toggl Track Sync**: HTTP 206 streaming, multi-anime playlists, per-episode resume caching & full-listen play counters, bulk MP3/SRT upload with natural episode sorting, 6 curated themes, and automatic Toggl Track v9 logging.
- **Portable User Profile (`profile/profile.json`)**: All known/learning/ignored vocabulary, listening history, episode progress, completion counts, playlists, and non-secret preferences live cleanly in a single portable JSON file. Machine-local API tokens stay isolated in gitignored `data/secrets.json`.

## Setup on a Fresh Machine

```powershell
# 1. Clone repo
git clone https://github.com/n8mirai/kikutoyomu.git
cd kikutoyomu

# 2. Download offline dictionaries (JMdict + JMnedict + Whisper model) & build kiku.exe
.\setup.ps1

# 3. Launch
.\kiku.exe
```

Drop your own `.mp3` audio files (or `.srt`/`.ass` subtitles) into `media/<series-id>/` or drag-and-drop them directly onto the Kiku window.
