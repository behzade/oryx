# Browser URL Handoff Plan

## Scope

This plan covers only Chromium/Chrome extension assets. Oryx receives URLs through its
registered `oryx://` URL scheme.

## User Flow

1. User installs the unpacked Chromium extension from `browser-extension/extension`.
2. User clicks the Oryx toolbar button or uses the page context menu item.
3. The extension opens `oryx://open?url=<encoded>` for the active tab or selected link.
4. The desktop URL handler starts Oryx, or forwards the URL to the running Oryx instance.

## Extension Shape

- Manifest V3 Chromium extension.
- `activeTab` and `contextMenus` permissions.
- Toolbar button for the active tab.
- Context menu entries for the current page and links.
- No remote services, no content scripts, and no page scraping.

## Oryx Contract Assumptions

Oryx should be registered as the handler for:

- Deep link: `oryx://open?url=<encoded>`

The handler executable may also support `--open-url <url>` for local scripts, but the
extension only depends on the deep-link contract.

## Follow-up Work

- Add packaged desktop metadata with `MimeType=x-scheme-handler/oryx;`.
- Add Firefox support only after the Chromium flow is validated.
