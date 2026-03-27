use std::collections::{HashMap, HashSet};

use crate::library::{LIKED_PLAYLIST_ID, Library};
use crate::provider::{CollectionKind, CollectionRef, TrackList, TrackSummary};

use super::super::{
    BrowseMode, collection_browser_key, local_collection_selection_key, pick_existing_or_first,
    selected_local_track_list, track_cache_key,
};
use super::build::{
    build_cached_quality_maps, build_local_artist_lists, enrich_track_list_with_cached_qualities,
    filtered_cached_album_track_lists, filtered_cached_library_tracks,
};
pub(in crate::app) use super::quality::{
    AudioQuality, CollectionQualitySummary, normalized_audio_quality_grade,
    normalized_audio_quality_label,
};

pub(in crate::app) struct LibraryModule {
    library: Library,
    local_albums: Vec<TrackList>,
    local_artists: Vec<TrackList>,
    local_playlists: Vec<TrackList>,
    selected_local_album_id: Option<String>,
    selected_local_artist_id: Option<String>,
    selected_local_playlist_id: Option<String>,
    cached_track_qualities: HashMap<String, AudioQuality>,
    cached_collection_qualities: HashMap<String, CollectionQualitySummary>,
    cached_track_ids: HashSet<String>,
    liked_track_ids: HashSet<String>,
}

impl LibraryModule {
    pub(in crate::app) fn new(library: Library) -> Self {
        let mut catalog = Self {
            library,
            local_albums: Vec::new(),
            local_artists: Vec::new(),
            local_playlists: Vec::new(),
            selected_local_album_id: None,
            selected_local_artist_id: None,
            selected_local_playlist_id: None,
            cached_track_qualities: HashMap::new(),
            cached_collection_qualities: HashMap::new(),
            cached_track_ids: HashSet::new(),
            liked_track_ids: HashSet::new(),
        };
        catalog.refresh();
        catalog
    }

    pub(in crate::app) fn refresh(&mut self) {
        if let Ok(tracks) = self.library.cached_library_tracks() {
            let tracks = filtered_cached_library_tracks(tracks);
            let cached_track_ids = tracks
                .iter()
                .map(|cached_track| track_cache_key(&cached_track.track))
                .collect::<HashSet<_>>();
            let (cached_track_qualities, cached_collection_qualities) =
                build_cached_quality_maps(&tracks);
            self.cached_track_qualities = cached_track_qualities;
            self.cached_collection_qualities = cached_collection_qualities;
            self.local_albums = filtered_cached_album_track_lists(
                self.library.entity_album_track_lists().unwrap_or_default(),
                &cached_track_ids,
            );
            self.local_artists = build_local_artist_lists(&self.local_albums);
        }

        if let Ok(mut playlists) = self.library.entity_playlist_track_lists() {
            if let Ok(Some(recently_played)) = self.library.load_recently_played_playlist() {
                let insert_index = playlists
                    .iter()
                    .position(|playlist| playlist.collection.reference.id == LIKED_PLAYLIST_ID)
                    .map(|index| index + 1)
                    .unwrap_or(0);
                playlists.insert(insert_index, recently_played);
            }
            self.local_playlists = playlists
                .into_iter()
                .filter(|playlist| playlist.collection.reference.kind == CollectionKind::Playlist)
                .collect();
        }

        if let Ok(cached_track_ids) = self.library.all_cached_track_ids() {
            self.cached_track_ids = cached_track_ids;
        }

        if let Ok(liked_track_ids) = self.library.liked_track_keys() {
            self.liked_track_ids = liked_track_ids;
        }

        self.selected_local_album_id = pick_existing_or_first(
            BrowseMode::Albums,
            self.selected_local_album_id.take(),
            &self.local_albums,
        );
        self.selected_local_artist_id = pick_existing_or_first(
            BrowseMode::Artists,
            self.selected_local_artist_id.take(),
            &self.local_artists,
        );
        self.selected_local_playlist_id = pick_existing_or_first(
            BrowseMode::Playlists,
            self.selected_local_playlist_id.take(),
            &self.local_playlists,
        );
    }

    pub(in crate::app) fn refresh_album_collection(
        &mut self,
        provider: crate::provider::ProviderId,
        collection_id: &str,
    ) {
        let collection = CollectionRef::new(provider, collection_id.to_string(), CollectionKind::Album, None);
        let cached_tracks = self
            .library
            .cached_library_tracks_for_collection(provider, collection_id)
            .unwrap_or_default();
        let cached_track_ids = cached_tracks
            .iter()
            .map(|cached_track| track_cache_key(&cached_track.track))
            .collect::<HashSet<_>>();

        self.remove_cached_state_for_collection(&collection);
        self.insert_cached_state(&cached_tracks);

        let updated_album = self
            .library
            .load_collection_track_list(&collection)
            .ok()
            .flatten()
            .and_then(|track_list| {
                filtered_cached_album_track_lists(vec![track_list], &cached_track_ids)
                    .into_iter()
                    .next()
            });

        self.upsert_local_album(updated_album, &collection);
        self.local_artists = build_local_artist_lists(&self.local_albums);
        self.reconcile_local_selections();
    }

    pub(in crate::app) fn album_count(&self) -> usize {
        self.local_albums.len()
    }

    pub(in crate::app) fn artist_count(&self) -> usize {
        self.local_artists.len()
    }

    pub(in crate::app) fn playlist_count(&self) -> usize {
        self.local_playlists.len()
    }

    pub(in crate::app) fn local_collections(&self, mode: BrowseMode) -> &[TrackList] {
        match mode {
            BrowseMode::Albums => &self.local_albums,
            BrowseMode::Artists => &self.local_artists,
            BrowseMode::Playlists => &self.local_playlists,
            BrowseMode::Discover => &[],
        }
    }

    pub(in crate::app) fn local_collections_owned(&self, mode: BrowseMode) -> Vec<TrackList> {
        self.local_collections(mode).to_vec()
    }

    pub(in crate::app) fn selected_local_collection_id(&self, mode: BrowseMode) -> Option<&str> {
        match mode {
            BrowseMode::Albums => self.selected_local_album_id.as_deref(),
            BrowseMode::Artists => self.selected_local_artist_id.as_deref(),
            BrowseMode::Playlists => self.selected_local_playlist_id.as_deref(),
            BrowseMode::Discover => None,
        }
    }

    pub(in crate::app) fn selected_local_collection_id_owned(
        &self,
        mode: BrowseMode,
    ) -> Option<String> {
        self.selected_local_collection_id(mode).map(str::to_string)
    }

    pub(in crate::app) fn select_local_collection(&mut self, mode: BrowseMode, id: String) {
        let resolved_id = self
            .local_collections(mode)
            .iter()
            .find(|list| {
                list.collection.reference.id == id
                    || local_collection_selection_key(mode, &list.collection.reference) == id
            })
            .map(|list| local_collection_selection_key(mode, &list.collection.reference))
            .unwrap_or(id);
        match mode {
            BrowseMode::Albums => self.selected_local_album_id = Some(resolved_id),
            BrowseMode::Artists => self.selected_local_artist_id = Some(resolved_id),
            BrowseMode::Playlists => self.selected_local_playlist_id = Some(resolved_id),
            BrowseMode::Discover => {}
        }
    }

    pub(in crate::app) fn current_local_track_list(&self, mode: BrowseMode) -> Option<&TrackList> {
        selected_local_track_list(
            mode,
            self.local_collections(mode),
            self.selected_local_collection_id(mode),
        )
    }

    pub(in crate::app) fn current_local_track_list_owned(
        &self,
        mode: BrowseMode,
    ) -> Option<TrackList> {
        self.current_local_track_list(mode).cloned()
    }

    pub(in crate::app) fn track_quality(&self, track: &TrackSummary) -> Option<AudioQuality> {
        self.cached_track_qualities
            .get(&track_cache_key(track))
            .cloned()
    }

    pub(in crate::app) fn collection_quality(
        &self,
        collection: &CollectionRef,
    ) -> Option<CollectionQualitySummary> {
        self.cached_collection_qualities
            .get(&collection_browser_key(collection))
            .cloned()
    }

    pub(in crate::app) fn track_is_cached(&self, track: &TrackSummary) -> bool {
        self.cached_track_ids.contains(&track_cache_key(track))
    }

    pub(in crate::app) fn track_is_liked(&self, track: &TrackSummary) -> bool {
        self.liked_track_ids.contains(&track_cache_key(track))
    }

    pub(in crate::app) fn enrich_track_list(&self, track_list: &mut TrackList) {
        enrich_track_list_with_cached_qualities(track_list, &self.cached_track_qualities);
    }

    fn insert_cached_state(&mut self, tracks: &[crate::library::CachedLibraryTrack]) {
        let (track_qualities, collection_qualities) = build_cached_quality_maps(tracks);
        self.cached_track_ids.extend(
            tracks
                .iter()
                .map(|cached_track| track_cache_key(&cached_track.track)),
        );
        self.cached_track_qualities.extend(track_qualities);
        self.cached_collection_qualities.extend(collection_qualities);
    }

    fn remove_cached_state_for_collection(&mut self, collection: &CollectionRef) {
        if let Some(existing_album) = self.local_albums.iter().find(|track_list| {
            track_list.collection.reference.provider == collection.provider
                && track_list.collection.reference.id == collection.id
        }) {
            for track in &existing_album.tracks {
                let track_key = track_cache_key(track);
                self.cached_track_ids.remove(&track_key);
                self.cached_track_qualities.remove(&track_key);
            }
        }
        self.cached_collection_qualities
            .remove(&collection_browser_key(collection));
    }

    fn upsert_local_album(&mut self, track_list: Option<TrackList>, collection: &CollectionRef) {
        self.local_albums.retain(|existing| {
            !(existing.collection.reference.provider == collection.provider
                && existing.collection.reference.id == collection.id)
        });

        if let Some(track_list) = track_list {
            self.local_albums.push(track_list);
            self.local_albums.sort_by(|left, right| {
                left.collection
                    .title
                    .to_lowercase()
                    .cmp(&right.collection.title.to_lowercase())
                    .then_with(|| {
                        left.collection
                            .subtitle
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .cmp(
                                &right
                                    .collection
                                    .subtitle
                                    .as_deref()
                                    .unwrap_or("")
                                    .to_lowercase(),
                            )
                    })
            });
        }
    }

    fn reconcile_local_selections(&mut self) {
        self.selected_local_album_id = pick_existing_or_first(
            BrowseMode::Albums,
            self.selected_local_album_id.take(),
            &self.local_albums,
        );
        self.selected_local_artist_id = pick_existing_or_first(
            BrowseMode::Artists,
            self.selected_local_artist_id.take(),
            &self.local_artists,
        );
        self.selected_local_playlist_id = pick_existing_or_first(
            BrowseMode::Playlists,
            self.selected_local_playlist_id.take(),
            &self.local_playlists,
        );
    }
}
