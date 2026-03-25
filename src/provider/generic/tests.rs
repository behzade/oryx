use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::super::{
    CollectionKind, CollectionRef, ProviderCollectionUrlTemplates, ProviderId, SearchResult,
};
use super::config::{
    HtmlFieldSpec, HtmlSearchResponseSpec, HtmlTrackListResponseSpec, JsonFieldSpec,
    JsonSearchResponseSpec, JsonTrackListResponseSpec, ProviderManifest, RequestSpec,
    SearchOperation, SearchResponseSpec, SearchResultKindRule, SearchResultKindSpec, SongOperation,
    TrackListOperation, TrackListResponseSpec,
};
use super::loader::{
    ConfiguredProviderImportStatus, export_provider_link_from_toml, import_provider_link_into_dir,
    load_configured_providers_from_sources,
};
use super::provider::{ConfiguredProvider, SmokeTestStatus};
use crate::library::Library;

const HTML_PROVIDER_MANIFEST: &str = r##"
id = "fixture_html_provider"
display_name = "Fixture HTML Provider"
short_display_name = "FixtureHTML"
search_rank_bias = 20

[collection_urls]
album = "https://catalog.example.test/items/{id}/"

[default_headers]
Referer = "https://catalog.example.test/"
User-Agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36"

[search.request]
method = "GET"
url = "https://catalog.example.test/"

[search.request.query]
s = "{query}"

[search.response]
format = "html"
item_selector = "section.box-i section.posting article.postbox-i"

[search.response.result_kind]
field = "kind_label"
default = "track"

[[search.response.result_kind.rules]]
contains = "پلی"
result = "collection:playlist"

[[search.response.result_kind.rules]]
contains = "playlist"
result = "collection:playlist"

[[search.response.result_kind.rules]]
contains = "تک آهنگ"
result = "collection:album"

[[search.response.result_kind.rules]]
contains = "single"
result = "collection:album"

[[search.response.result_kind.rules]]
contains = "آلبوم"
result = "collection:album"

[[search.response.result_kind.rules]]
contains = "album"
result = "collection:album"

[search.response.fields.url]
selector = ".post-img-hover a"
attr = "href"

[search.response.fields.id]
source = "url"
transforms = ["url_path_id"]

[search.response.fields.title]
selector = "li.index-al"
text = true
transforms = ["normalize_whitespace"]

[search.response.fields.artist]
selector = "li.index-ar a"
text = true
transforms = ["normalize_whitespace"]

[search.response.fields.subtitle]
source = "artist"

[search.response.fields.artwork_url]
selector = "img"
attr = "src"

[search.response.fields.kind_label]
selector = ".TSale"
text = true
transforms = ["normalize_whitespace", "lowercase"]

[track_list.request]
url = "{collection.canonical_url}"

[track_list.response]
format = "html"
track_item_selector = "ul.audioplayer-audios > li"

[track_list.response.collection_fields.title]
selector = "h2.AL-Si"
text = true
transforms = ["normalize_whitespace"]

[track_list.response.collection_fields.subtitle]
selector = ".AR-Si a"
text = true
transforms = ["normalize_whitespace"]

[track_list.response.collection_fields.artwork_url]
selector = "figure.pic-s img"
attr = "src"

[track_list.response.track_fields.source_url]
attr = "data-src"
transforms = ["decode_html", "normalize_whitespace"]

[track_list.response.track_fields.title]
attr = "data-title"
transforms = ["decode_html", "normalize_whitespace"]

[track_list.response.track_fields.artist]
attr = "data-artist"
transforms = ["decode_html", "normalize_whitespace"]

[track_list.response.track_fields.album]
attr = "data-album"
transforms = ["decode_html", "normalize_whitespace"]

[track_list.response.track_fields.duration_seconds]
attr = "data-duration"
transforms = ["decode_html", "trim"]

[track_list.response.track_fields.artwork_url]
attr = "data-image"
transforms = ["decode_html", "normalize_whitespace"]

[song]
media_url_contains = [".example.test/media/"]
page_url_prefixes = ["https://catalog.example.test/items/"]
supports_byte_ranges = true
"##;

const SCRIPT_PROVIDER_MANIFEST: &str = r##"
id = "fixture_script_provider"
display_name = "Fixture Script Provider"
short_display_name = "FixtureScript"
search_rank_bias = 30

[collection_urls]
album = "https://secure.example.test/p/{id}"

[default_headers]
Accept = "*/*"
Accept-Language = "en-US,en;q=0.9,fa;q=0.8"
Referer = "https://secure.example.test/"
User-Agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36"

[auth]
required = true

[auth.preflight]
method = "GET"
url = "https://secure.example.test/user/?p=login"

[auth.submit]
username_field = "username"
password_field = "password"

[auth.submit.request]
method = "POST"
url = "https://secure.example.test/user/?p=login"

[auth.submit.request.form]
ci_csrf_token = ""

[auth.verify]
not_contains = ["name=\"loginform\"", "name=\"password\""]

[auth.verify.request]
method = "GET"
url = "https://secure.example.test/user/?p=login"

[search.request]
method = "GET"
url = "https://secure.example.test/s/"

[search.request.query]
q = "{query}"

[search.response]
format = "html"
item_selector = ".sbsub .sbartist.sbalbum"

[search.response.result_kind]
default = "collection:album"

[search.response.fields.url]
selector = "a"
attr = "href"

[search.response.fields.id]
source = "url"
transforms = ["url_path_id"]

[search.response.fields.title]
selector = ".sbartistname"
text = true
transforms = ["normalize_whitespace"]

[search.response.fields.artwork_url]
selector = "img"
attr = "src"

[track_list.request]
url = "{collection.canonical_url}"

[track_list.response]
format = "htmlscript"
script_start = "curplaylist = ["
script_end = "];"
strict_indexed_field = "title"
count_mismatch_message = "Track parser only resolved {resolved} of {expected} visible tracks"
no_tracks_message = "Authentication is required for full playback. Preview streams are blocked."

[track_list.response.collection_fields.title]
selector = ".prodinfobox .albumtitle"
text = true
transforms = ["normalize_whitespace"]

[track_list.response.collection_fields.subtitle]
selector = ".prodinfobox .albumartist a"
text = true
transforms = ["normalize_whitespace"]

[track_list.response.collection_fields.artwork_url]
selector = "#productimage"
attr = "src"

[track_list.response.indexed_html_fields.title]
selector = ".newmobtracklist .track .title"
text = true
transforms = ["normalize_whitespace"]

[track_list.response.track_fields.source_url]
field = "src"
raw = true

[track_list.response.track_fields.title]
field = "title"
transforms = ["normalize_whitespace"]

[track_list.response.track_fields.artist]
field = "artist"
transforms = ["normalize_whitespace"]

[track_list.response.track_fields.artwork_url]
field = "artwork"
raw = true

[track_list.response.track_fields.duration_seconds]
field = "lenght"
transforms = ["trim"]

[[track_list.response.skip_if_field_contains]]
field = "source_url"
contains = "/preview/"

[[track_list.response.skip_if_field_contains]]
field = "source_url"
contains = "/low-bitrate-preview/"

[song]
media_url_contains = ["media.example.test/"]
page_url_prefixes = ["https://secure.example.test/p/"]
blocked_url_patterns = ["/preview/", "/low-bitrate-preview/"]
blocked_url_message = "Authentication is required for full playback. Preview streams are blocked."
supports_byte_ranges = true

[song.page_request]
url = "{track.canonical_url}"
"##;

const HTML_PROVIDER_SEARCH_FIXTURE: &str = r#"
    <section class="box-i">
        <section class="posting">
            <article class="postbox-i">
                <div class="post-img-hover">
                    <a href="https://catalog.example.test/55498/">
                        <img src="https://catalog.example.test/assets/collection-55498.jpg" />
                        <div class="TSale">آلبوم</div>
                    </a>
                </div>
                <section class="postinfo">
                    <ul>
                        <li class="index-al">Collection One (Expanded Edition)</li>
                        <li class="index-ar">
                            <a href="https://catalog.example.test/artist/creator-one/">Creator One</a>
                        </li>
                    </ul>
                </section>
            </article>
        </section>
    </section>
"#;

const SCRIPT_PROVIDER_TRACK_LIST_FIXTURE: &str = r#"
    <div class="prodinfobox">
        <div class="albumtitle">Collection Two</div>
        <div class="albumartist"><a>Creator Two</a></div>
    </div>
    <img id="productimage" src="https://secure.example.test/assets/collection-two.jpg"/>
    <div class="mainbox newmobtracklist">
        <div class="track"><div class="title">Track One</div></div>
        <div class="track"><div class="title">Track Two</div></div>
        <div class="track"><div class="title">Track Three</div></div>
        <div class="track"><div class="title">Track Four</div></div>
    </div>
    <script>
    curplaylist = [
        { src: "https://secure.example.test/preview/collection-two.mp3", title: 'Collection Two (preview)', artist: 'Creator Two', artwork:'https://secure.example.test/assets/collection-two-thumb.jpg', trackId:200001, pId:'collection-two' },
        { src: "https://media.example.test/audio/collection-two/01-track-one.mp3", title: 'Track One', artist: 'Creator Two', artwork:'https://secure.example.test/assets/collection-two-thumb.jpg', lenght:'03:13', trackId:200002, pId:'collection-two' },
        { src: "https://media.example.test/audio/collection-two/02-track-two.mp3", title: 'Track Two', artist: 'Creator Two', artwork:'https://secure.example.test/assets/collection-two-thumb.jpg', lenght:'02:10', trackId:200003, pId:'collection-two' },
        { src: "https://media.example.test/audio/collection-two/03-track-three.mp3", title: 'Track Three', artist: 'Creator Two', artwork:'https://secure.example.test/assets/collection-two-thumb.jpg', lenght:'06:32', trackId:200004, pId:'collection-two' },
        { src: "https://media.example.test/audio/collection-two/04-track-four.mp3", title: 'Track Four', artist: 'Creator Two', artwork:'https://secure.example.test/assets/collection-two-thumb.jpg', lenght:'02:51', trackId:200005, pId:'collection-two' }
    ];
    </script>
"#;

#[test]
fn parses_html_search_results_from_manifest_schema() {
    let provider = ConfiguredProvider::from_manifest(ProviderManifest {
        id: "fixture_html".to_string(),
        display_name: "Fixture HTML".to_string(),
        short_display_name: None,
        search_rank_bias: 0,
        collection_urls: ProviderCollectionUrlTemplates::default(),
        default_headers: BTreeMap::new(),
        search: SearchOperation {
            request: RequestSpec::default(),
            response: SearchResponseSpec::Html(HtmlSearchResponseSpec {
                item_selector: "article".to_string(),
                fields: BTreeMap::from([
                    (
                        "url".to_string(),
                        HtmlFieldSpec {
                            selector: Some("a".to_string()),
                            attr: Some("href".to_string()),
                            text: false,
                            value: None,
                            source: None,
                            transforms: Vec::new(),
                        },
                    ),
                    (
                        "title".to_string(),
                        HtmlFieldSpec {
                            selector: Some(".title".to_string()),
                            attr: None,
                            text: true,
                            value: None,
                            source: None,
                            transforms: Vec::new(),
                        },
                    ),
                    (
                        "kind".to_string(),
                        HtmlFieldSpec {
                            selector: Some(".kind".to_string()),
                            attr: None,
                            text: true,
                            value: None,
                            source: None,
                            transforms: vec![super::config::FieldTransform::Lowercase],
                        },
                    ),
                ]),
                result_kind: SearchResultKindSpec {
                    field: Some("kind".to_string()),
                    rules: vec![SearchResultKindRule {
                        contains: "album".to_string(),
                        result: "collection:album".to_string(),
                    }],
                    default: Some("track".to_string()),
                },
            }),
        },
        track_list: TrackListOperation {
            request: RequestSpec::default(),
            response: TrackListResponseSpec::Html(HtmlTrackListResponseSpec {
                collection_fields: BTreeMap::new(),
                track_item_selector: "li".to_string(),
                track_fields: BTreeMap::new(),
            }),
        },
        song: SongOperation::default(),
        auth: None,
        validation: None,
    })
    .expect("provider should build");

    let results = provider
        .parse_search_results(
            r#"<article><a href="https://example.com/albums/collection-one"></a><span class="title">Collection One</span><span class="kind">Album</span></article>"#,
        )
        .expect("html search should parse");

    assert_eq!(results.len(), 1);
    match &results[0] {
        SearchResult::Collection(collection) => {
            assert_eq!(collection.reference.id, "collection-one");
            assert_eq!(collection.title, "Collection One");
        }
        SearchResult::Track(_) => panic!("expected collection result"),
    }
}

#[test]
fn parses_json_track_list_from_manifest_schema() {
    let provider = ConfiguredProvider::from_manifest(ProviderManifest {
        id: "fixture_json".to_string(),
        display_name: "Fixture JSON".to_string(),
        short_display_name: None,
        search_rank_bias: 0,
        collection_urls: ProviderCollectionUrlTemplates::default(),
        default_headers: BTreeMap::new(),
        search: SearchOperation {
            request: RequestSpec::default(),
            response: SearchResponseSpec::Json(JsonSearchResponseSpec {
                items_path: "results".to_string(),
                fields: BTreeMap::new(),
                result_kind: SearchResultKindSpec {
                    field: None,
                    rules: Vec::new(),
                    default: None,
                },
            }),
        },
        track_list: TrackListOperation {
            request: RequestSpec::default(),
            response: TrackListResponseSpec::Json(JsonTrackListResponseSpec {
                collection_fields: BTreeMap::from([(
                    "title".to_string(),
                    JsonFieldSpec {
                        path: Some("album.title".to_string()),
                        value: None,
                        source: None,
                        transforms: Vec::new(),
                    },
                )]),
                tracks_path: "album.tracks".to_string(),
                track_fields: BTreeMap::from([
                    (
                        "source_url".to_string(),
                        JsonFieldSpec {
                            path: Some("src".to_string()),
                            value: None,
                            source: None,
                            transforms: Vec::new(),
                        },
                    ),
                    (
                        "title".to_string(),
                        JsonFieldSpec {
                            path: Some("title".to_string()),
                            value: None,
                            source: None,
                            transforms: Vec::new(),
                        },
                    ),
                    (
                        "duration_seconds".to_string(),
                        JsonFieldSpec {
                            path: Some("duration".to_string()),
                            value: None,
                            source: None,
                            transforms: Vec::new(),
                        },
                    ),
                ]),
            }),
        },
        song: SongOperation::default(),
        auth: None,
        validation: None,
    })
    .expect("provider should build");

    let collection = CollectionRef::new(
        ProviderId::parse("fixture_json").expect("provider id"),
        "album-1",
        CollectionKind::Album,
        None,
    );
    let track_list = provider
        .parse_track_list(
            &collection,
            r#"{"album":{"title":"Collection One","tracks":[{"src":"https://cdn.example.com/01.mp3","title":"Track One","duration":456}]}}"#,
        )
        .expect("json track list should parse");

    assert_eq!(track_list.collection.title, "Collection One");
    assert_eq!(track_list.tracks.len(), 1);
    assert_eq!(track_list.tracks[0].duration_seconds, Some(456));
}

#[test]
fn fixture_html_manifest_matches_search_behavior() {
    let provider = provider_from_manifest(HTML_PROVIDER_MANIFEST);
    let results = provider
        .parse_search_results(HTML_PROVIDER_SEARCH_FIXTURE)
        .expect("fixture html manifest search should parse");

    assert_eq!(results.len(), 1);
    match &results[0] {
        SearchResult::Collection(collection) => {
            assert_eq!(
                collection.reference.provider,
                ProviderId::parse("fixture_html_provider").unwrap()
            );
            assert_eq!(collection.reference.id, "55498");
            assert_eq!(collection.title, "Collection One (Expanded Edition)");
            assert_eq!(collection.subtitle.as_deref(), Some("Creator One"));
        }
        SearchResult::Track(_) => panic!("expected collection result"),
    }
}

#[test]
fn fixture_script_manifest_matches_track_listing_behavior() {
    let provider = provider_from_manifest(SCRIPT_PROVIDER_MANIFEST);
    let collection = CollectionRef::new(
        ProviderId::parse("fixture_script_provider").unwrap(),
        "collection-two",
        CollectionKind::Album,
        Some("https://secure.example.test/p/collection-two".to_string()),
    );

    let track_list = provider
        .parse_track_list(&collection, SCRIPT_PROVIDER_TRACK_LIST_FIXTURE)
        .expect("fixture script manifest track list should parse");

    assert_eq!(track_list.collection.title, "Collection Two");
    assert_eq!(
        track_list.collection.subtitle.as_deref(),
        Some("Creator Two")
    );
    assert_eq!(track_list.collection.track_count, Some(4));
    assert_eq!(track_list.tracks.len(), 4);
    assert_eq!(track_list.tracks[0].title, "Track One");
    assert_eq!(track_list.tracks[0].duration_seconds, Some(193));
    assert_eq!(track_list.tracks[3].title, "Track Four");
    assert_eq!(track_list.tracks[3].duration_seconds, Some(171));
}

#[test]
fn bundled_provider_manifests_load_with_configured_ids() {
    let root = std::env::temp_dir().join(format!(
        "oryx-provider-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos()
    ));
    let config_dir = root.join("config");
    let bundled_dir = root.join("bundled");
    fs::create_dir_all(&config_dir).expect("temp config provider dir should be created");
    fs::create_dir_all(&bundled_dir).expect("temp bundled provider dir should be created");
    fs::write(
        bundled_dir.join("fixture_html_provider.toml"),
        HTML_PROVIDER_MANIFEST,
    )
    .expect("fixture html manifest should be written");
    fs::write(
        bundled_dir.join("fixture_script_provider.toml"),
        SCRIPT_PROVIDER_MANIFEST,
    )
    .expect("fixture script manifest should be written");

    let providers =
        load_configured_providers_from_sources(&config_dir, &[bundled_dir.clone()], None)
            .expect("configured providers should load from configured sources");
    let ids = providers
        .iter()
        .map(|provider| provider.id())
        .collect::<Vec<_>>();

    assert!(ids.contains(&ProviderId::parse("fixture_html_provider").unwrap()));
    assert!(ids.contains(&ProviderId::parse("fixture_script_provider").unwrap()));

    fs::remove_dir_all(&root).expect("temp provider dir should be removed");
}

fn provider_from_manifest(contents: &str) -> ConfiguredProvider {
    let manifest: ProviderManifest = toml::from_str(contents).expect("manifest should deserialize");
    ConfiguredProvider::from_manifest(manifest).expect("manifest should build provider")
}

#[test]
fn validation_smoke_test_passes_for_search_listing_and_track_resolution() {
    let server = TestServer::spawn();
    let manifest = provider_from_manifest(&live_validation_manifest(&server.base_url(), "article"));

    let status = manifest
        .validate_configuration()
        .expect("validation should pass against the local fixture server");

    assert_eq!(status, SmokeTestStatus::Passed);
}

#[test]
fn loader_falls_back_to_last_validated_manifest_when_candidate_breaks() {
    let server = TestServer::spawn();
    let root = temp_test_dir("oryx-provider-loader");
    let provider_dir = root.join("providers");
    let app_root = root.join("app");
    fs::create_dir_all(&provider_dir).expect("provider dir should exist");

    let library = Library::new_in(app_root).expect("test library should initialize");
    let provider_id = ProviderId::parse("fixturelive").expect("provider id should parse");
    let valid_manifest = live_validation_manifest(&server.base_url(), "article");
    fs::write(provider_dir.join("fixturelive.toml"), &valid_manifest)
        .expect("valid manifest should be written");

    let providers = load_configured_providers_from_sources(&provider_dir, &[], Some(&library))
        .expect("valid manifest should load");
    assert_eq!(providers.len(), 1);

    let stored = library
        .load_provider_runtime_state(provider_id)
        .expect("runtime state should load")
        .expect("runtime state should exist after validation");
    let active_manifest = stored
        .active_manifest_toml
        .clone()
        .expect("validated manifest should be stored");
    assert!(active_manifest.contains("item_selector = \"article\""));

    fs::write(
        provider_dir.join("fixturelive.toml"),
        live_validation_manifest(&server.base_url(), ".missing"),
    )
    .expect("broken manifest should be written");

    let providers = load_configured_providers_from_sources(&provider_dir, &[], Some(&library))
        .expect("loader should fall back to last validated manifest");
    assert_eq!(providers.len(), 1);

    let stored = library
        .load_provider_runtime_state(provider_id)
        .expect("runtime state should reload")
        .expect("runtime state should still exist");
    assert_eq!(
        stored.active_manifest_toml.as_deref(),
        Some(active_manifest.as_str())
    );

    fs::remove_dir_all(&root).expect("temp dir should be removed");
}

#[test]
fn provider_links_round_trip_through_import() {
    let root = temp_test_dir("oryx-provider-link");
    let provider_dir = root.join("providers");
    let app_root = root.join("app");
    fs::create_dir_all(&provider_dir).expect("provider dir should exist");
    let library = Library::new_in(app_root).expect("test library should initialize");
    let manifest = r#"
id = "fixturelink"
display_name = "Fixture Link"

[search.request]
method = "GET"
url = "https://example.com/search"

[search.response]
format = "json"
items_path = "results"

[track_list.response]
format = "json"
tracks_path = "tracks"

[song]
media_url_prefixes = ["https://example.com/media/"]
"#;

    let link = export_provider_link_from_toml(manifest).expect("provider link should encode");
    let imported = import_provider_link_into_dir(&library, &link, &provider_dir)
        .expect("provider link should import");

    assert_eq!(
        imported.provider_id,
        ProviderId::parse("fixturelink").unwrap()
    );
    assert_eq!(imported.status, ConfiguredProviderImportStatus::Activated);
    assert_eq!(
        fs::read_to_string(provider_dir.join("fixturelink.toml"))
            .expect("imported manifest should be written")
            .trim(),
        manifest.trim()
    );

    let raw_imported = import_provider_link_into_dir(&library, manifest, &provider_dir)
        .expect("raw manifest import should also work");
    assert_eq!(
        raw_imported.status,
        ConfiguredProviderImportStatus::Activated
    );

    fs::remove_dir_all(&root).expect("temp dir should be removed");
}

fn live_validation_manifest(base_url: &str, item_selector: &str) -> String {
    format!(
        r#"
id = "fixturelive"
display_name = "Fixture Live"

[search.request]
method = "GET"
url = "{base_url}/search"

[search.request.query]
q = "{{query}}"

[search.response]
format = "html"
item_selector = "{item_selector}"

[search.response.result_kind]
default = "collection:album"

[search.response.fields.url]
selector = "a"
attr = "href"

[search.response.fields.id]
source = "url"
transforms = ["url_path_id"]

[search.response.fields.title]
selector = ".title"
text = true

[track_list.request]
url = "{{collection.canonical_url}}"

[track_list.response]
format = "html"
track_item_selector = "li"

[track_list.response.collection_fields.title]
selector = "h1.album-title"
text = true

[track_list.response.track_fields.source_url]
attr = "data-src"

[track_list.response.track_fields.title]
attr = "data-title"

[song]
media_url_prefixes = ["{base_url}/media/"]

[validation]
example_query = "sample"
"#
    )
}

fn temp_test_dir(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos()
    ))
}

struct TestServer {
    address: String,
    shutdown_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        listener
            .set_nonblocking(true)
            .expect("test server should be nonblocking");
        let address = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("test server should expose a local address")
        );
        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        let response_base_url = address.clone();
        let join_handle = thread::spawn(move || {
            loop {
                match shutdown_rx.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }

                match listener.accept() {
                    Ok((stream, _)) => handle_test_request(stream, &response_base_url),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("test server accept failed: {error}"),
                }
            }
        });

        Self {
            address,
            shutdown_tx,
            join_handle: Some(join_handle),
        }
    }

    fn base_url(&self) -> &str {
        &self.address
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

fn handle_test_request(mut stream: TcpStream, base_url: &str) {
    let mut request_line = String::new();
    {
        let mut reader = BufReader::new(&stream);
        reader
            .read_line(&mut request_line)
            .expect("request line should be readable");
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");

    let (status, body) = if path.starts_with("/search") {
        (
            "200 OK",
            format!(
                "<article><a href=\"{base_url}/albums/collection-one\"></a><span class=\"title\">Collection One</span></article>"
            ),
        )
    } else if path == "/albums/collection-one" {
        (
            "200 OK",
            format!(
                "<h1 class=\"album-title\">Collection One</h1><ul><li data-src=\"{base_url}/media/01.mp3\" data-title=\"Track One\"></li></ul>"
            ),
        )
    } else {
        ("404 Not Found", "not found".to_string())
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .expect("response should be writable");
    stream.flush().expect("response should flush");
}
