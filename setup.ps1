# Kiku (聴く) — One-command setup for dictionaries & Whisper model on a fresh Windows PC
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$DataDir = Join-Path $Root "data"
$WhisperDir = Join-Path $Root "tools\whisper"
New-Item -ItemType Directory -Force $DataDir, $WhisperDir | Out-Null

Write-Host "==> Checking offline Japanese dictionaries (JMdict + JMnedict)..." -ForegroundColor Cyan
$rel = Invoke-RestMethod "https://api.github.com/repos/scriptin/jmdict-simplified/releases/latest"

# 1. JMdict (~219k general Japanese words)
$jmPath = Join-Path $DataDir "jmdict-eng.json"
if (-not (Test-Path $jmPath)) {
  $asset = $rel.assets | Where-Object { $_.name -like "jmdict-eng-3.*.json.zip" -and $_.name -notlike "*common*" } | Select-Object -First 1
  Write-Host "  Downloading $($asset.name)..."
  $zip = Join-Path $DataDir "jmdict.zip"
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip
  $tmp = Join-Path $DataDir "jmdict_tmp"
  Expand-Archive -Path $zip -DestinationPath $tmp -Force
  Move-Item -Force (Get-ChildItem $tmp -Filter "*.json" | Select-Object -First 1).FullName $jmPath
  Remove-Item -Recurse -Force $tmp, $zip
  Write-Host "  Installed jmdict-eng.json" -ForegroundColor Green
} else {
  Write-Host "  jmdict-eng.json already present." -ForegroundColor DarkGray
}

# 2. JMnedict (~715k Japanese proper names: places, surnames, characters)
$nmPath = Join-Path $DataDir "jmnedict.json"
if (-not (Test-Path $nmPath)) {
  $asset = $rel.assets | Where-Object { $_.name -like "jmnedict-all-*.json.zip" } | Select-Object -First 1
  Write-Host "  Downloading $($asset.name)..."
  $zip = Join-Path $DataDir "jmnedict.zip"
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip
  $tmp = Join-Path $DataDir "jmnedict_tmp"
  Expand-Archive -Path $zip -DestinationPath $tmp -Force
  Move-Item -Force (Get-ChildItem $tmp -Filter "*.json" | Select-Object -First 1).FullName $nmPath
  Remove-Item -Recurse -Force $tmp, $zip
  Write-Host "  Installed jmnedict.json" -ForegroundColor Green
} else {
  Write-Host "  jmnedict.json already present." -ForegroundColor DarkGray
}

# 3. IPADIC morphological dictionary (usually tracked in repo; download fallback if missing)
$ipaPath = Join-Path $DataDir "ipadic.dic.zst"
if (-not (Test-Path $ipaPath)) {
  Write-Host "  Downloading Vibrato IPADIC..."
  $tar = Join-Path $DataDir "ipadic.tar.xz"
  Invoke-WebRequest -Uri "https://github.com/daac-tools/vibrato/releases/download/v0.5.0/ipadic-mecab-2_7_0.tar.xz" -OutFile $tar
  $tmp = Join-Path $DataDir "ipadic_tmp"
  New-Item -ItemType Directory -Force $tmp | Out-Null
  tar.exe -xf $tar -C $tmp
  Move-Item -Force (Get-ChildItem -Recurse $tmp -Filter "system.dic.zst" | Select-Object -First 1).FullName $ipaPath
  Remove-Item -Recurse -Force $tmp, $tar
  Write-Host "  Installed ipadic.dic.zst" -ForegroundColor Green
}

# 4. Optional Whisper.cpp binary + ggml-small-q5_1 model for generating synced .srt subtitles
$whisperModel = Join-Path $WhisperDir "ggml-small-q5_1.bin"
if (-not (Test-Path $whisperModel)) {
  Write-Host "==> Downloading Whisper small-q5_1 model (190 MB)..." -ForegroundColor Cyan
  Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin" -OutFile $whisperModel
  Write-Host "  Installed ggml-small-q5_1.bin" -ForegroundColor Green
}

# 5. Portable Node.js 22 LTS + official @google/gemini-cli (for 1-click "Sign in with Google" AI features)
$NodeDir = Join-Path $Root "tools\node"
$GeminiCmd = Join-Path $NodeDir "node_modules\.bin\gemini.cmd"
if (-not (Test-Path $GeminiCmd)) {
  Write-Host "==> Installing portable Node.js + @google/gemini-cli..." -ForegroundColor Cyan
  New-Item -ItemType Directory -Force $NodeDir | Out-Null
  if (-not (Test-Path (Join-Path $NodeDir "node.exe"))) {
    $idx = Invoke-RestMethod "https://nodejs.org/dist/index.json"
    $lts = $idx | Where-Object { $_.lts -and $_.version -like "v22.*" } | Select-Object -First 1
    $ver = $lts.version
    $zipUrl = "https://nodejs.org/dist/$ver/node-$ver-win-x64.zip"
    $nzip = Join-Path $Root "tools\node.zip"
    Invoke-WebRequest -Uri $zipUrl -OutFile $nzip
    Expand-Archive -Path $nzip -DestinationPath (Join-Path $Root "tools") -Force
    $extracted = Join-Path $Root "tools\node-$ver-win-x64"
    Copy-Item -Recurse -Force (Join-Path $extracted "*") $NodeDir
    Remove-Item -Recurse -Force $extracted, $nzip
  }
  $env:Path = "$NodeDir;" + $env:Path
  $prevPref = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  & (Join-Path $NodeDir "npm.cmd") install --prefix $NodeDir @google/gemini-cli --no-audit --no-fund
  $ErrorActionPreference = $prevPref
  if (Test-Path $GeminiCmd) {
    Write-Host "  Installed @google/gemini-cli (Sign in with Google ready!)" -ForegroundColor Green
  }
} else {
  Write-Host "  @google/gemini-cli already present." -ForegroundColor DarkGray
}

Write-Host "==> Building Kiku release binary..." -ForegroundColor Cyan
Set-Location $Root
cargo build --release
Copy-Item -Force (Join-Path $Root "target\release\kiku.exe") (Join-Path $Root "kiku.exe")
Write-Host "Done! Run .\kiku.exe to launch." -ForegroundColor Green
