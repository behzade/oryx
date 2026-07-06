# Oryx Browser Extension

Local development assets for sending the current Chromium/Chrome tab URL to Oryx.

## What It Adds

- A Manifest V3 Chromium extension with:
  - toolbar button: sends the active tab URL to Oryx
  - page context menu item: sends the current page URL
  - link context menu item: sends the selected link URL
- Native messaging handoff that opens Oryx without a browser tab.
- Deep-link launch using `oryx://open?url=<encoded>`.
- Oryx app icons for the toolbar and extension listing.

## Install the Extension

1. Open `chrome://extensions` or `chromium://extensions`.
2. Enable Developer mode.
3. Choose "Load unpacked".
4. Select `browser-extension/extension`.

The unpacked extension has a checked-in manifest key, so Chrome should assign the
same ID on every install: `gaiomjoeonfapknnlcfcfmccapfeekon`.

## Configure Native Messaging

Native messaging is the preferred handoff path. It sends the URL directly to Oryx
without opening a tab or protocol prompt.

1. Build or install Oryx so there is an executable binary:

```sh
cargo build
```

2. Install the native messaging host manifest:

```sh
./browser-extension/install-native-host.sh target/debug/oryx
```

For a custom extension build with a different ID, pass the ID explicitly:

```sh
./browser-extension/install-native-host.sh <extension-id> target/debug/oryx
```

## Configure Oryx Launching

If native messaging is not configured, the extension falls back to opening:

```text
oryx://open?url=<encoded-url>
```

Register Oryx as the handler for the `oryx` URL scheme. For local development, install a desktop entry that runs:

```sh
oryx %u
```

Chromium opens custom protocols through a tab and may show a protocol prompt.
That fallback tab stays open so the user can approve the launch.
