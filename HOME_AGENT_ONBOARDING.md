# Home Agent Onboarding — Cloudflare R2 Remote Media Streaming for Kiku to Yomu

> **Purpose:** This file is a complete handoff brief for the agent running on Nathan's **home PC**. Follow this guide to implement Cloudflare R2 remote audio streaming + upload automation for **Kiku to Yomu (`https://github.com/n8mirai/kikutoyomu`)**, verify everything locally, and push clean commits so Nathan can pull and run immediately on his work PC.

---

## 1. User Goal & Context

- **App:** **Kiku to Yomu (聴く・読む)** — a pure-Rust, single-binary Japanese audio immersion & interactive subtitle desktop app (`kiku.exe`, local HTTP server on `127.0.0.1:41937`, single-file UI in `src/ui.html` launched in Edge `--app` mode).
- **Target Setup:**
  1. **Home PC (where you are):** Houses Nathan's full anime `.mp3` library. Runs Whisper (`tools/sync-all-subtitles.ps1`) to generate speech-synced `.srt` subtitles, uploads `.mp3`s + `manifest.json` to **Cloudflare R2** (10 GB free, **$0 egress fees**), and commits the lightweight `.srt` subtitles + code + `profile/profile.json` to `n8mirai/kikutoyomu`.
  2. **Work PC:** Runs `kiku.exe` locally (zero MP3s stored on disk, zero admin rights, corporate Palo Alto SSL inspection, outbound HTTPS works cleanly). Streams `.mp3` audio seamlessly from Cloudflare R2 with full HTTP 206 seeking, while MeCab tokenization, 219k JMdict + 715k JMnedict dictionary lookups, Krashen T1–T3 heatmaps, Word Maps, and Toggl Track sync all run locally at <1 ms.

---

## 2. Hard Architectural & Security Constraints (READ FIRST!)

1. **PUBLIC GitHub Repo — ZERO SECRETS IN GIT!**
   - `https://github.com/n8mirai/kikutoyomu` is **public**.
   - **NEVER** commit API keys, R2 access keys, Worker bearer/HMAC secrets, or `toggl_api_token` anywhere in `profile/profile.json`, scripts, or source code.
   - All portable user data lives in **`profile/profile.json`** (tracked in git, token fields always cleared on save).
   - All machine-local secrets live in **`data/secrets.json`** (`.gitignore`d). Add any new remote-media secret (`remote_media_secret`) to `LocalSecrets` in `data/secrets.json` ONLY.
2. **Pure-Rust / No-C-Toolchain Build Compatibility (Work PC = `x86_64-pc-windows-gnu` without `dlltool.exe`!)**
   - The work PC compiles using userspace `stable-x86_64-pc-windows-gnu` **without** MinGW/MSVC/C compiler/dlltool.
   - **NEVER** add crates that pull `windows-link` `raw-dylib` or C build scripts (specifically avoid `chrono`, `reqwest`, `openssl-sys`, `zstd-sys`, or default features of `vibrato`).
   - Keep `vibrato = { version = "0.5", default-features = false }` and `ruzstd = "0.7"`.
   - For outbound HTTPS from Rust on Windows (e.g., fetching `manifest.json` or remote `.srt`), reuse the existing **`windows-sys` WinHTTP** pattern from `src/toggl.rs` using **`WINHTTP_ACCESS_TYPE_DEFAULT_PROXY`** (never `AUTOMATIC_PROXY`, which stalls 30s on corporate WPAD!).
   - Keep `CARGO_HTTP_CHECK_REVOKE=false` compatibility in mind (work PC has SSL inspection).
3. **UI Design Ethos ("Whisper-Minimal / Broker Desk Split Flame")**
   - Keep every UI change ultra-minimal, clean, and low-text—no bulky banners or verbose settings pages. Add only compact fields inside the existing `#pop` Settings popover in `src/ui.html`.

---

## 3. Codebase Map

| Path | Role |
|---|---|
| `Cargo.toml` | Pure-Rust deps (`tiny_http`, `serde`, `serde_json`, `base64`, `percent-encoding`, `vibrato` w/o default features, `ruzstd`, `windows-sys`). |
| `src/main.rs` | Single-instance probe on port `41937`, Edge `--app` launcher, server boot. |
| `src/models.rs` | `AppConfig`, `Episode`, `AnimeSeries`, `PersistedData`, `UserProfile` (`profile/profile.json`), `LocalSecrets` (`data/secrets.json`), `PublicConfig`. |
| `src/library.rs` | Scans `media/<series-id>/`, natural episode sort, pure-Rust MP3 duration estimator, `.srt`/`.vtt`/`.ass` pairing, catalog metadata & aliases. |
| `src/server.rs` | HTTP server: `/stream/{anime}/{file}` (HTTP 206), `/api/state`, `/api/rescan`, `/api/subtitles/{a}/{e}`, `/api/corpus/{a}`, `/api/dict`, `/api/vocab`, `/api/config`, `/api/progress`. Loads MeCab + JMdict + JMnedict on background thread. |
| `src/morph.rs` | `vibrato` + `data/ipadic.dic.zst` Japanese morphological analyzer (lemmas, POS, readings, auto-ignore & Word Map content filter). |
| `src/dict.rs` | Local 219k JMdict + 715k JMnedict lookup engine with MeCab exact lemma/POS ranking & de-inflection. |
| `src/toggl.rs` | WinHTTP v9 client (`WINHTTP_ACCESS_TYPE_DEFAULT_PROXY`) — great reference for any WinHTTP GET helper. |
| `src/subs.rs` | `.srt`, `.vtt`, `.ass`/`.ssa` parser. |
| `src/ui.html` | Entire frontend (single HTML/CSS/JS file compiled via `include_str!("ui.html")`). |
| `profile/profile.json` | Nathan's portable profile (known/learning/ignored vocab, history, episode progress & play counts, playlists, non-secret settings). |
| `tools/sync-all-subtitles.ps1` | Batch local `whisper.cpp` (`-l ja -ml 42 -osrt`) subtitle sync script. |
| `setup.ps1` | Fresh-PC bootstrapper (downloads JMdict, JMnedict, Whisper model & builds `kiku.exe`). |

---

## 4. Recommended Implementation Plan (Private R2 + Thin Cloudflare Worker)

Keeping the R2 bucket **private** behind a tiny **Cloudflare Worker** (free tier: 100,000 requests/day, zero egress) with **HMAC-SHA256 signed URLs** gives you:
- Private bucket (MP3s are never publicly browsable or scrapeable).
- Zero double-bandwidth proxying through Rust: Kiku's `/stream/{anime}/{file}` issues a `302 Found` to a short-lived signed Worker URL (`?exp=...&sig=...`), so Edge's `<audio>` element streams and seeks directly from Cloudflare's edge CDN via native HTTP 206!
- Works over plain HTTPS port 443 on corporate networks.

### Step 1: Cloudflare R2 + Worker (`cloud/r2-worker/`)
Create a clean `cloud/r2-worker/` folder in the repo containing:
1. `wrangler.toml` — binds an R2 bucket `MEDIA_BUCKET` (bucket name e.g. `kiku-media`, placeholder account/route fields — no secrets committed!).
2. `src/index.js` (~60 lines):
   - Verifies either:
     - `Authorization: Bearer <KIKU_MEDIA_SECRET>` header (used by Kiku's backend for `GET /manifest.json` and `GET /subs/...`), OR
     - Short-lived HMAC-SHA256 query signature `?exp=<unix_ts>&sig=<hex>` (used by `<audio>` after Kiku redirects `/stream/...` → Worker; HMAC computed over `"${pathname}:${exp}"` with `KIKU_MEDIA_SECRET` stored via `wrangler secret put KIKU_MEDIA_SECRET`).
   - Serves R2 objects with full **HTTP 206 `Range` support** (`MEDIA_BUCKET.get(key, { range: request.headers })`), `Accept-Ranges: bytes`, `Content-Range`, `Content-Length`, `Content-Type`, and clean `CORS` headers for `http://127.0.0.1:41937`.
3. `cloud/r2-worker/README.md` — concise 5-step deploy commands (`npm i -g wrangler`, `wrangler login`, `wrangler r2 bucket create kiku-media`, `wrangler secret put KIKU_MEDIA_SECRET`, `wrangler deploy`).

### Step 2: Home Upload & Manifest Generator Script (`tools/r2-sync.ps1`)
Create `tools/r2-sync.ps1` so adding new anime at home is a single command:
1. Optional `-SyncSubtitles` flag: runs `tools/sync-all-subtitles.ps1` (generalized to accept any series folder under `media/`, not just `example-japanese-stories`) to generate Whisper-synced `.srt` files for any new MP3s.
2. Scans `media/<series-id>/` locally and generates `manifest.json`:
   ```json
   {
     "generated_at": "2026-11-23T18:00:00Z",
     "series": [
       {
         "id": "my-hero-academia",
         "title": "My Hero Academia",
         "jp_title": "僕のヒーローアカデミア",
         "has_cover": true,
         "cover_file": "cover.jpg",
         "episodes": [
           {
             "id": "ep-01",
             "number": 1,
             "title": "Episode 01",
             "filename": "01 - Izuku Midoriya Origin.mp3",
             "file_size": 24819200,
             "duration_secs": 1440,
             "has_subtitles": true,
             "subtitle_file": "01 - Izuku Midoriya Origin.srt"
           }
         ]
       }
     ]
   }
   ```
3. Uploads all `.mp3` files + `manifest.json` (+ optional `.srt`s & `cover.jpg`s) to R2 via **`rclone sync`** (recommended for incremental diff uploads—only uploads new/changed files!) or `wrangler r2 object put`.
4. Leaves `.srt` files in `media/<series-id>/` so Nathan can `git add media/ profile/ && git commit && git push` (or include an optional `-PushSubs` switch).

### Step 3: Rust Backend Remote Streaming Support (`src/models.rs`, `src/library.rs`, `src/server.rs`, `src/remote.rs`)
Keep everything **100% pure Rust + `windows-sys` WinHTTP** (zero C deps!):
1. **`src/models.rs`**:
   - Add `remote_media_url: String` to `AppConfig` (non-secret base URL like `https://kiku-media.<user>.workers.dev`; safe in `profile/profile.json`!).
   - Add `remote_media_secret: String` to `LocalSecrets` (stored ONLY in gitignored `data/secrets.json`!).
   - Add `has_remote_media_secret: bool`, `remote_media_url: String` to `PublicConfig` (never expose the secret itself!).
   - Add `#[serde(default)] pub remote: bool` on `Episode` so the UI/server know whether an episode streams from R2 when missing locally.
2. **`src/remote.rs`** (new small pure-Rust module):
   - Implement a tiny pure-Rust **HMAC-SHA256** (~55 lines of standard SHA-256 + HMAC, zero external crates needed!) to generate short-lived signed URLs (`exp = now + 3600` seconds, `sig = hmac_sha256(secret, "{path}:{exp}")`).
   - Implement `fetch_manifest(base_url, secret)` & `fetch_remote_text(base_url, secret, key)` using **`windows-sys` WinHTTP** (`WINHTTP_ACCESS_TYPE_DEFAULT_PROXY`, mirroring `src/toggl.rs`).
3. **`src/library.rs` & `src/server.rs`**:
   - In `SharedState::load` and `rescan()`: if `remote_media_url` is configured, fetch `manifest.json` (on a background thread at startup so Kiku still binds in <50ms, or synchronously on explicit `POST /api/rescan` / `POST /api/remote/test`) and merge remote episodes into `self.library`:
     - If an episode already exists locally on disk → keep `remote: false` (always prefer local!).
     - If missing locally → insert/update with `available: true, remote: true`, preserving catalog titles/covers!
   - In `GET /stream/{anime_id}/{filename}`:
     - If local file exists → serve via existing `handle_stream` (HTTP 206).
     - Else if `remote_media_url` is set → generate a 1-hour HMAC-signed Worker URL for `/{anime_id}/{filename}` (URL-encoded path segments!) and respond with **`302 Found`** + `Location: <signed_url>` + `Cache-Control: no-store`.
   - In `GET /api/subtitles/{anime_id}/{episode_id}` and `/api/corpus/{anime_id}`:
     - Prefer local `.srt` (all synced `.srt`s committed in git are already local!).
     - If missing locally and remote episode has `subtitle_file` → fetch from Worker, cache locally under `data/remote-subs/{anime_id}/{subtitle_file}`, parse & MeCab-tokenize cleanly!
   - Add `POST /api/remote/test` (saves `remote_media_url` / `remote_media_secret`, fetches `manifest.json`, merges library, returns `{ ok, series_count, episode_count, library, config }`).

### Step 4: Minimal Settings UI (`src/ui.html`)
Inside `#pop` (Settings popover), add one compact section (matching the exact style of the Toggl fields):
- `Remote Media URL` (`<input id="s-remote-url" placeholder="https://kiku-media...workers.dev">`)
- `Media Secret` (`<input id="s-remote-sec" type="password" placeholder="Secret key">`) + `Test` button (`#s-remote-test`)
- Wire into `syncSettingsUI()`, `#s-save`, and `#s-remote-test`.
- Add a tiny subtle cloud dot/tooltip on remote episodes if desired (keep ultra-minimal!).

---

## 5. Verification Checklist Before Pushing

Before pushing your changes to `https://github.com/n8mirai/kikutoyomu`:

1. **Pure-Rust build check:**
   ```powershell
   cargo build --release
   ```
   Verify **0 errors** and **0 new native/C-toolchain dependencies** (confirm `Cargo.lock` does NOT contain `chrono`, `windows-link`, `openssl-sys`, or `zstd-sys`).
2. **End-to-end streaming test:**
   - Upload at least 1 test series/episode + `manifest.json` to Cloudflare R2.
   - Temporarily rename or test without the local `.mp3` on disk.
   - Confirm `/api/remote/test` succeeds, episode appears playable, `<audio>` plays & scrubs cleanly (HTTP 206 from Cloudflare Worker!), subtitles + MeCab + JMdict + progress caching all work identically to local playback.
3. **Strict security audit before `git push`:**
   - Confirm `data/secrets.json` is NOT tracked (`git status`).
   - Confirm `profile/profile.json` contains **zero** API tokens or R2 secrets.
   - Confirm `cloud/r2-worker/wrangler.toml` contains **zero** secrets.
   - Confirm no `.mp3`, `.exe`, `target/`, or >50 MB files are staged.
4. **Push & summarize** what Nathan needs to paste into Settings on his work PC (`Remote Media URL` + `Media Secret`) after he runs `git pull && cargo build --release`.
