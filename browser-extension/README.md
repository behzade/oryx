# Browser extension

The Chromium extension sends the current page or a chosen link to Oryx.

## Install

1. Open `chrome://extensions` or `chromium://extensions`.
2. Enable developer mode.
3. Choose **Load unpacked** and select `browser-extension/extension`.

The checked-in extension ID is `gaiomjoeonfapknnlcfcfmccapfeekon`.

## Native host

Build Oryx, then install the host file:

```sh
cargo build
./browser-extension/install-native-host.sh target/debug/oryx
```

For another extension ID:

```sh
./browser-extension/install-native-host.sh <extension-id> target/debug/oryx
```

Without the native host, the extension uses `oryx://open?url=<encoded-url>`. The installed app must handle that URL scheme.
