# Oryx Browser Extension

Local development assets for sending the current Chromium/Chrome tab URL to Oryx.

## What It Adds

- A Manifest V3 Chromium extension with:
  - toolbar button: sends the active tab URL to Oryx
  - page context menu item: sends the current page URL
  - link context menu item: sends the selected link URL
- Deep-link launch using `oryx://open?url=<encoded>`.

## Install the Extension

1. Open `chrome://extensions` or `chromium://extensions`.
2. Enable Developer mode.
3. Choose "Load unpacked".
4. Select `browser-extension/extension`.

## Configure Oryx Launching

The extension opens:

```text
oryx://open?url=<encoded-url>
```

Register Oryx as the handler for the `oryx` URL scheme. For local development, install a desktop entry that runs:

```sh
oryx %u
```
