# Plan

## Current Architecture

- Library/cache-derived state is split into a dedicated catalog entity.
- Discover/search state is split into its own entity.
- Playback state is split into its own entity.
- Transfer orchestration is separated from playback startup, with cached playback bypassing provider resolution.
- Transfer/download progress is entity-backed rather than living as ad hoc root maps.
- In-app notifications exist and cover meaningful outcomes/errors.

## Ranked Next Work

1. Build a reusable modal/sheet system and move provider auth onto it first. Done.
2. Add a downloads surface backed by transfer state, likely in that modal. Done.
3. Move import review into the shared modal once the primitive is proven. Done.
4. Polish metadata presentation so shared album/provider/quality information is shown once at the album level instead of repeated on every track row.
5. Add richer now-playing polish such as a visualizer after the structural UI surfaces settle.

## Import Workflow Polish

- Move local import review out of the center browse panel into a dedicated import surface, likely a modal sheet or dedicated import screen.
- Make the import pipeline explicitly staged: `scan -> resolve -> review -> commit -> artwork enrichment`.
- Persist pending import reviews so app restart does not force reanalysis or reimport.
- Reduce network churn by detecting the album/release once, then matching remaining tracks locally against that release.
- Improve recovery UX for partial matches: retry unresolved tracks, retry artwork, and accept partial import intentionally.
- Keep metadata/autotag logic isolated from browse/playback code by strengthening the `metadata` and import workflow boundaries.
- Tighten failure reporting so metadata, artwork, and network errors are visible in UI without relying on terminal logs.
- Revisit artwork backfill and metadata refresh as first-class maintenance actions for existing local imports.

## Provider Direction

- Preserve the core reason for the app: internet access can be cut off, so Oryx should remain useful with local files and whatever providers are reachable on the intranet at the time.
- Favor providers that tolerate durable local caching and offline replay, since the current playback/cache model stores audio files on disk and reuses them later.
- Prioritize `Audius` first as the best remote-provider fit for the current architecture.
- Evaluate `ccMixter` next as another license-compatible catalog for offline-capable caching, with attribution and per-track license handling.
- Keep user-owned downloads and imports as a first-class path, including files obtained from lawful sources such as Bandcamp purchases or other DRM-free catalogs.
- Do not plan around `SoundCloud` or `Jamendo` for the current cache model; both are poor fits for persistent offline playback under their published API terms.
- If intranet-only mirrors or local gateways exist for otherwise remote catalogs, treat them as separate providers with their own access and licensing assumptions instead of assuming the public-service terms apply unchanged.
