#!/usr/bin/env sh
set -eu

HOST_NAME="dev.oryx.app"
DEFAULT_EXTENSION_ID="gaiomjoeonfapknnlcfcfmccapfeekon"

if [ "$#" -gt 2 ]; then
  echo "Usage: $0 [oryx-binary]" >&2
  echo "       $0 <extension-id> [oryx-binary]" >&2
  exit 2
fi

EXTENSION_ID="$DEFAULT_EXTENSION_ID"
ORYX_BIN=""

is_chrome_extension_id() {
  [ ${#1} -eq 32 ] || return 1
  case "$1" in
    *[!abcdefghijklmnop]*) return 1 ;;
    *) return 0 ;;
  esac
}

if [ "$#" -eq 1 ]; then
  if is_chrome_extension_id "$1"; then
    EXTENSION_ID="$1"
  else
    ORYX_BIN="$1"
  fi
elif [ "$#" -eq 2 ]; then
  EXTENSION_ID="$1"
  ORYX_BIN="$2"
fi

if ! is_chrome_extension_id "$EXTENSION_ID"; then
  echo "Invalid Chrome extension ID: $EXTENSION_ID" >&2
  exit 2
fi

if [ -z "$ORYX_BIN" ]; then
  ORYX_BIN="$(command -v oryx || true)"
fi

if [ -z "$ORYX_BIN" ]; then
  echo "Pass the Oryx binary path, for example: $0 target/debug/oryx" >&2
  exit 2
fi

case "$ORYX_BIN" in
  /*) ;;
  *) ORYX_BIN="$(pwd)/$ORYX_BIN" ;;
esac

if [ ! -x "$ORYX_BIN" ]; then
  echo "Oryx binary is not executable: $ORYX_BIN" >&2
  exit 2
fi

case "$(uname -s)" in
  Darwin)
    HOST_DIRS="$(printf '%s\n%s\n' \
      "$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts" \
      "$HOME/Library/Application Support/Chromium/NativeMessagingHosts")"
    ;;
  Linux)
    HOST_DIRS="$(printf '%s\n%s\n' \
      "$HOME/.config/google-chrome/NativeMessagingHosts" \
      "$HOME/.config/chromium/NativeMessagingHosts")"
    ;;
  *)
    echo "Unsupported OS for this installer. Create the native messaging host manifest manually." >&2
    exit 2
    ;;
esac

printf '%s\n' "$HOST_DIRS" | while IFS= read -r HOST_DIR; do
  [ -n "$HOST_DIR" ] || continue
  mkdir -p "$HOST_DIR"
  WRAPPER_PATH="$HOST_DIR/$HOST_NAME.sh"
  cat > "$WRAPPER_PATH" <<EOF
#!/usr/bin/env sh
exec "$ORYX_BIN" --native-messaging
EOF
  chmod 755 "$WRAPPER_PATH"

  MANIFEST_PATH="$HOST_DIR/$HOST_NAME.json"
  cat > "$MANIFEST_PATH" <<EOF
{
  "name": "$HOST_NAME",
  "description": "Open URLs in Oryx",
  "path": "$WRAPPER_PATH",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://$EXTENSION_ID/"
  ]
}
EOF
  echo "Installed $MANIFEST_PATH"
done
