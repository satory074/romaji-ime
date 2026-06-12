#!/usr/bin/env bash
# RomajiIME — end-user installer (no developer toolchain required).
#
# Ships inside the distribution zip next to RomajiIME.app. It:
#   1. clears the Gatekeeper quarantine on the bundle (you downloaded it, so you
#      trust it) — needed because the build is ad-hoc signed, not notarized;
#   2. installs the app into ~/Library/Input Methods/ (per-user, no admin);
#   3. writes ~/Library/Application Support/RomajiIME/config.json with your own
#      (free) Gemini API key, so the AI conversion works.
#
# Usage:
#   ./install.sh                 # interactive: prompts for your Gemini key
#   GEMINI_API_KEY=AIza... ./install.sh
#   ./install.sh --skip-config   # install only, keep any existing config.json
#
# Get a free key at https://aistudio.google.com/apikey (see INSTALL.md).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_SRC="$SCRIPT_DIR/RomajiIME.app"
DEST_DIR="$HOME/Library/Input Methods"
DEST_APP="$DEST_DIR/RomajiIME.app"
CONFIG_DIR="$HOME/Library/Application Support/RomajiIME"
CONFIG="$CONFIG_DIR/config.json"
SKIP_CONFIG=0
[[ "${1:-}" == "--skip-config" ]] && SKIP_CONFIG=1

say() { printf '%s\n' "$*"; }

if [[ ! -d "$APP_SRC" ]]; then
    say "✗ RomajiIME.app がこのスクリプトの隣に見つかりません ($APP_SRC)。"
    say "  zip を展開したフォルダの中で実行してください。"
    exit 1
fi

# 1. Trust + install the bundle.
say ">> Gatekeeper の検疫属性を解除します（ダウンロードしたアプリを信頼）"
xattr -dr com.apple.quarantine "$APP_SRC" 2>/dev/null || true

say ">> インストール: $DEST_APP"
mkdir -p "$DEST_DIR"
rm -rf "$DEST_APP"
cp -R "$APP_SRC" "$DEST_APP"

# 2. Configure the cloud-AI key (per-user, never committed, chmod 600).
if [[ "$SKIP_CONFIG" -eq 1 ]]; then
    say ">> --skip-config: config.json はそのままにします"
elif [[ -f "$CONFIG" ]]; then
    say ">> 既存の設定が見つかりました: $CONFIG"
    read -r -p "   上書きして新しいキーを設定しますか？ [y/N] " ans
    [[ "${ans:-N}" =~ ^[Yy]$ ]] && SKIP_CONFIG=0 || SKIP_CONFIG=1
fi

if [[ "$SKIP_CONFIG" -eq 0 ]]; then
    KEY="${GEMINI_API_KEY:-}"
    if [[ -z "$KEY" ]]; then
        say ""
        say "Gemini の無料 API キーを貼り付けてください。"
        say "  取得: https://aistudio.google.com/apikey （Create API key）"
        read -r -s -p "API key: " KEY
        say ""
    fi
    if [[ -z "$KEY" ]]; then
        say "✗ キーが空です。あとで $CONFIG を作るか、再実行してください。"
    else
        mkdir -p "$CONFIG_DIR"
        umask 077  # config holds a secret -> owner-only
        cat > "$CONFIG" <<JSON
{
  "provider": "gemini",
  "api_key": "$KEY",
  "model": "gemini-2.0-flash",
  "timeout_ms": 5000,
  "auto_convert": true,
  "auto_convert_delay_ms": 500
}
JSON
        chmod 600 "$CONFIG"
        say ">> 設定を書き込みました: $CONFIG （所有者のみ読み取り可）"
    fi
fi

cat <<'NEXT'

✓ インストール完了！ 有効化の手順（初回のみ）:

  1. システム設定 ▸ キーボード ▸ 入力ソース ▸ 「編集…」
  2. 「＋」▸ 日本語 ▸ RomajiIME ▸ 追加
  3. Ctrl+Space（または入力メニュー）で RomajiIME に切り替え
  4. テキスト編集アプリで  nihongo  と打ち、少し待つ → 候補が出る
       ・そのまま Enter → 打った通り（nihongo）
       ・Space → Enter → 変換（日本語）

リストに出てこない場合は、一度ログアウト→ログインしてください。
キーを変えたいときは ./install.sh を再実行（または config.json を編集）。
NEXT
