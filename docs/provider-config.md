# Provider files

Oryx reads one TOML file per remote source. The file name must match `id`, such as `example.toml` for `id = "example"`.

Paths:

- Linux: `~/.config/oryx/providers/`
- macOS: `~/Library/Application Support/oryx/providers/`
- Windows: `%AppData%\oryx\providers\`
- Override: `ORYX_PROVIDER_DIR`

Oryx can also read `bundled/providers/` beside the working directory or executable, or the path in `ORYX_BUNDLED_PROVIDER_DIR`. User files take priority.

If `[validation]` is set, changed files must pass its checks before use. If a check fails, Oryx keeps the last valid copy.

## Small JSON example

```toml
id = "example"
display_name = "Example"

[validation]
example_query = "test"

[search.request]
url = "https://example.test/search"

[search.request.query]
q = "{query}"

[search.response]
format = "json"
items_path = "results"

[search.response.fields.id]
path = "id"

[search.response.fields.title]
path = "title"

[search.response.fields.subtitle]
path = "artist"

[search.response.fields.url]
path = "url"

[track_list.request]
url = "{collection.canonical_url}"

[track_list.response]
format = "json"
tracks_path = "tracks"

[track_list.response.collection_fields.title]
path = "title"

[track_list.response.track_fields.id]
path = "id"

[track_list.response.track_fields.title]
path = "title"

[track_list.response.track_fields.artist]
path = "artist"

[track_list.response.track_fields.source_url]
path = "stream_url"
```

## Reference

Top-level keys:

- Required: `id`, `display_name`, `search`, `track_list`
- Optional: `short_display_name`, `search_rank_bias`, `collection_urls`, `default_headers`, `song`, `auth`, `validation`

Requests support `method`, `url`, `headers`, `query`, `form`, `body`, and `content_type`. Methods are `GET` and `POST`.

Request templates:

- `{query}`
- `{collection.id}`, `{collection.kind}`, `{collection.canonical_url}`
- `{track.id}`, `{track.canonical_url}`, `{track.title_hint}`
- `{auth.username}`, `{auth.password}`
- `{provider.id}`, `{provider.display_name}`

Response formats:

- Search: `html`, `json`
- Track list: `html`, `json`, `htmlscript`

HTML fields support `selector`, `attr`, `text`, `value`, `source`, and `transforms`. JSON fields support `path`, `value`, `source`, and `transforms`. JSON paths allow dot keys and indexes such as `results[0].items`.

Transforms are `trim`, `lowercase`, `uppercase`, `normalize_whitespace`, `decode_html`, and `url_path_id`.

Common search fields are `id`, `url`, `title`, `subtitle`, `artist`, `artwork_url`, and `track_count`. Common track fields are `id`, `source_url`, `url`, `title`, `artist`, `album`, `duration_seconds`, and `artwork_url`.

Use `[song]` to mark direct media URLs, page URLs, blocked URLs, and byte-range support. Use `[auth]` for cookie login with optional preflight and verify requests. For less common HTML and auth forms, use the fixtures in `src/provider/generic/tests.rs` as working examples.
