pub(super) fn sanitized_media_artwork_url() -> Option<&'static str> {
    // souvlaki 0.8.3 crashes in its macOS backend when NSImage fails to load
    // the provided cover URL. Until that dependency is patched or replaced,
    // never publish artwork to the media session on macOS.
    None
}
