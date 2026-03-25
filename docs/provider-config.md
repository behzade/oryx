# Generic Provider Config

Oryx loads TOML-backed providers from the provider config directory:

- default: `~/.config/oryx/providers/<id>.toml`
- override: the directory pointed to by `ORYX_PROVIDER_DIR`

The file name must match the provider `id`. For example, `id = "example_provider"` must live at `example_provider.toml`.

Oryx can also read an optional on-disk bundled provider directory with lower precedence than the config dir:

- `bundled/providers/` in the current working directory
- `bundled/providers/` next to the executable
- the directory pointed to by `ORYX_BUNDLED_PROVIDER_DIR`

Bundled directories are optional and are only read if they exist. Oryx does not provision provider files automatically.

Configured providers are not trusted blindly:

- a new or changed manifest is treated as a candidate
- Oryx parses and validates it before promotion
- if validation passes, the manifest becomes the active last-known-good version
- if validation fails, Oryx keeps using the last validated manifest for that provider when one exists
- already cached audio can still play even if a provider config is missing or invalid

## Top-Level Fields

```toml
id = "example_html"
display_name = "Example HTML"
short_display_name = "ExHTML"
search_rank_bias = 10

[collection_urls]
album = "https://example.com/albums/{id}"
playlist = "https://example.com/playlists/{id}"

[default_headers]
Referer = "https://example.com/"
User-Agent = "Mozilla/5.0"

[validation]
example_query = "sample query"
expect_min_results = 1
test_first_collection = true
test_first_track = true
require_stream_url = true
skip_if_not_authenticated = true
```

`{id}` and `{collection_id}` are supported inside `collection_urls`.

## Validation

Use `[validation]` to make Oryx smoke-test a provider before activating a changed manifest.

```toml
[validation]
example_query = "sample query"
expect_min_results = 1
test_first_collection = true
test_first_track = true
require_stream_url = true
skip_if_not_authenticated = true
```

The validation flow is:

- run `search(example_query)`
- require at least `expect_min_results`
- take the first collection result when available and fetch its track list
- resolve the first track into `SongData`
- require a non-empty stream URL when `require_stream_url = true`

If the provider requires auth and `skip_if_not_authenticated = true`, Oryx will not promote an unvalidated manifest over an existing last-known-good revision until credentials are available.

## Requests

Each request supports:

- `method = "GET"` or `method = "POST"`
- `url`
- `headers`
- `query`
- `form`
- `body`
- `content_type`

Templates are supported in request values:

- `{query}`
- `{collection.id}`
- `{collection.kind}`
- `{collection.canonical_url}`
- `{track.id}`
- `{track.canonical_url}`
- `{track.title_hint}`
- `{auth.username}`
- `{auth.password}`
- `{provider.id}`
- `{provider.display_name}`

## HTML Search Example

```toml
id = "example_html"
display_name = "Example HTML"

[default_headers]
Referer = "https://example.com/"
User-Agent = "Mozilla/5.0"

[search.request]
method = "GET"
url = "https://example.com/search"

[search.request.query]
q = "{query}"

[search.response]
format = "html"
item_selector = "article.search-result"

[search.response.result_kind]
field = "kind"
default = "track"

[[search.response.result_kind.rules]]
contains = "album"
result = "collection:album"

[[search.response.result_kind.rules]]
contains = "playlist"
result = "collection:playlist"

[search.response.fields.url]
selector = "a.result-link"
attr = "href"

[search.response.fields.id]
source = "url"
transforms = ["url_path_id"]

[search.response.fields.title]
selector = ".title"
text = true
transforms = ["normalize_whitespace"]

[search.response.fields.kind]
selector = ".badge"
text = true
transforms = ["lowercase"]

[search.response.fields.subtitle]
selector = ".subtitle"
text = true
```

## JSON Track List Example

```toml
[track_list.request]
url = "{collection.canonical_url}"

[track_list.response]
format = "json"
tracks_path = "album.tracks"

[track_list.response.collection_fields.title]
path = "album.title"

[track_list.response.collection_fields.subtitle]
path = "album.artist"

[track_list.response.collection_fields.artwork_url]
path = "album.cover"

[track_list.response.track_fields.source_url]
path = "src"

[track_list.response.track_fields.title]
path = "title"

[track_list.response.track_fields.artist]
path = "artist"

[track_list.response.track_fields.duration_seconds]
path = "duration"
```

JSON paths support dot access and numeric array indexes like `results[0].items`.

## HTML Script Track Lists

Some sites expose playable track metadata inside a JS array embedded in HTML instead of plain DOM rows. For that case, use `format = "htmlscript"`:

```toml
[track_list.response]
format = "htmlscript"
script_start = "curplaylist = ["
script_end = "];"
strict_indexed_field = "title"
count_mismatch_message = "Only resolved {resolved} of {expected} visible tracks"
no_tracks_message = "No playable tracks were available."

[track_list.response.collection_fields.title]
selector = ".albumtitle"
text = true

[track_list.response.indexed_html_fields.title]
selector = ".track .title"
text = true

[track_list.response.track_fields.source_url]
field = "src"
raw = true

[track_list.response.track_fields.title]
field = "title"

[track_list.response.track_fields.artist]
field = "artist"

[track_list.response.track_fields.duration_seconds]
field = "lenght"

[[track_list.response.skip_if_field_contains]]
field = "source_url"
contains = "/preview/"
```

`indexed_html_fields` are collected from the visible DOM and overlaid by index onto the JS objects. That is useful when the visible title and the raw JS title disagree slightly.

## Song Resolution

```toml
[song]
media_url_suffixes = [".mp3", ".flac"]
page_url_prefixes = ["https://example.com/albums/"]
blocked_url_patterns = ["/preview/"]
blocked_url_message = "Subscription required for full playback."
supports_byte_ranges = true

[song.page_request]
url = "{track.canonical_url}"
```

If a track URL matches `media_url_*`, Oryx streams it directly. If it matches `page_url_*`, Oryx fetches the page and reuses the configured `track_list` parser to resolve the playable track.

## Authentication

```toml
[auth]
required = true

[auth.preflight]
method = "GET"
url = "https://example.com/login"

[auth.submit]
username_field = "username"
password_field = "password"

[auth.submit.request]
method = "POST"
url = "https://example.com/login"

[auth.submit.request.form]
csrf = ""

[auth.verify]

[auth.verify.request]
method = "GET"
url = "https://example.com/account"

contains = ["Logout"]
not_contains = ["name=\"password\""]
```

The runtime collects `Set-Cookie` headers during auth and sends them on later requests.

## Field Transforms

Supported transforms:

- `trim`
- `lowercase`
- `uppercase`
- `normalize_whitespace`
- `decode_html`
- `url_path_id`

JS-backed track fields support:

- `field`
- `value`
- `source`
- `raw`
- `transforms`

## Notes

- Search responses can emit either tracks or collections.
- Track lists always emit a collection plus tracks.
- Artwork downloads reuse `default_headers`.
- Configured providers and the local library can coexist. The runtime only activates manifests that pass validation.
