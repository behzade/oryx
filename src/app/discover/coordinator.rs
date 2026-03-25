use std::collections::HashMap;

use anyhow::Result;
use gpui::{AsyncApp, Context};

use crate::provider::{
    CollectionKind, CollectionRef, CollectionSummary, SearchResult, SharedProvider, TrackList,
};

use super::super::ui::NotificationLevel;
use super::super::{OryxApp, collection_entity_key};
use crate::library::Library;

impl OryxApp {
    pub(in crate::app) fn start_search(&mut self, cx: &mut Context<Self>) {
        let query = self.query_input.content().trim().to_string();
        if query.is_empty() {
            self.status_message = Some("Search query is empty.".to_string());
            self.show_notification("Search query is empty.", NotificationLevel::Error, cx);
            cx.notify();
            return;
        }

        let providers = self.discover.read(cx).active_providers(&self.providers);
        if providers.is_empty() {
            self.status_message = Some("No search providers are enabled.".to_string());
            self.show_notification(
                "No search providers are enabled.",
                NotificationLevel::Error,
                cx,
            );
            cx.notify();
            return;
        }
        let search_nonce = self
            .discover
            .update(cx, |discover, _cx| discover.begin_search());
        let source_label = self.discover.read(cx).active_source_label(&self.providers);
        self.status_message = Some(format!("Searching {} for '{query}'", source_label));
        self.persist_session_snapshot(cx);
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { search_collections(providers, query.clone()).await })
                .await;

            let _ = this.update(cx, move |this, cx| {
                let matches_nonce = this.discover.read(cx).search_nonce_matches(search_nonce);
                if !matches_nonce {
                    return;
                }
                match result {
                    Ok((query, results)) => {
                        let result_count = results.len();
                        this.discover.update(cx, |discover, _cx| {
                            discover.finish_search(results);
                        });
                        this.status_message = Some(match result_count {
                            0 => format!("No albums or playlists found for '{query}'."),
                            count => format!("Found {count} result(s) for '{query}'."),
                        });
                        this.persist_session_snapshot(cx);
                        this.hydrate_search_results_artwork(search_nonce, cx);
                    }
                    Err(error) => {
                        this.discover.update(cx, |discover, _cx| {
                            discover.fail_search();
                        });
                        let message = format!("Search failed: {error}");
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::app) fn load_collection(
        &mut self,
        collection: CollectionRef,
        cx: &mut Context<Self>,
    ) {
        let collection_key = collection_entity_key(&collection);
        let should_skip = self
            .discover
            .read(cx)
            .should_skip_collection_load(collection_key.as_str());
        if should_skip {
            return;
        }
        let track_list_nonce = self.discover.update(cx, |discover, _cx| {
            discover.begin_collection_load(collection_key.clone())
        });
        let Some(provider) = self.provider_for_id(collection.provider) else {
            self.status_message = Some(format!(
                "Provider '{}' is not available.",
                collection.provider
            ));
            cx.notify();
            return;
        };
        let library = self.library.clone();

        match self.library.load_collection_track_list(&collection) {
            Ok(Some(cached_track_list)) => {
                let mut cached_track_list = cached_track_list;
                self.library_catalog
                    .read(cx)
                    .enrich_track_list(&mut cached_track_list);
                let collection_title = cached_track_list.collection.title.clone();
                let count = cached_track_list.tracks.len();
                self.discover.update(cx, |discover, _cx| {
                    discover.finish_track_list_load(cached_track_list);
                });
                self.status_message =
                    Some(format!("Loaded {count} tracks from {collection_title}."));
                self.persist_session_snapshot(cx);
                self.hydrate_track_list_artwork(track_list_nonce, cx);
                cx.notify();
                return;
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!(
                    "failed to load cached collection track list for '{}:{}': {error:#}",
                    collection.provider.as_str(),
                    collection.id
                );
            }
        }

        self.discover.update(cx, |discover, _cx| {
            discover.begin_track_list_fetch();
        });
        self.status_message = Some("Loading track list".to_string());
        self.persist_session_snapshot(cx);
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let track_list = provider.get_track_list(&collection).await?;
                    if let Err(error) = library.save_collection_track_list(&collection, &track_list)
                    {
                        eprintln!(
                            "failed to persist collection track list for '{}:{}': {error:#}",
                            collection.provider.as_str(),
                            collection.id
                        );
                    }
                    Ok::<TrackList, anyhow::Error>(track_list)
                })
                .await;

            let _ = this.update(cx, move |this, cx| {
                let matches_nonce = this
                    .discover
                    .read(cx)
                    .track_list_nonce_matches(track_list_nonce);
                if !matches_nonce {
                    return;
                }
                match result {
                    Ok(track_list) => {
                        let mut track_list = track_list;
                        this.library_catalog
                            .read(cx)
                            .enrich_track_list(&mut track_list);
                        let collection_title = track_list.collection.title.clone();
                        let count = track_list.tracks.len();
                        this.discover.update(cx, |discover, _cx| {
                            discover.finish_track_list_load(track_list);
                        });
                        this.status_message =
                            Some(format!("Loaded {count} tracks from {collection_title}."));
                        this.persist_session_snapshot(cx);
                        this.hydrate_track_list_artwork(track_list_nonce, cx);
                    }
                    Err(error) => {
                        this.discover.update(cx, |discover, _cx| {
                            discover.fail_track_list_load();
                        });
                        this.status_message = Some(format!("Failed to load track list: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn hydrate_search_results_artwork(&mut self, search_nonce: u64, cx: &mut Context<Self>) {
        let search_results = self.discover.read(cx).search_results();
        if search_results.is_empty() {
            return;
        }

        let providers = self.providers.clone();
        let library = self.library.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let hydrated = cx
                .background_executor()
                .spawn(async move {
                    hydrate_collection_batch_artwork(library, providers, search_results)
                })
                .await;

            let _ = this.update(cx, move |this, cx| {
                let matches_nonce = this.discover.read(cx).search_nonce_matches(search_nonce);
                if !matches_nonce {
                    return;
                }

                this.discover.update(cx, |discover, _cx| {
                    discover.replace_search_results(hydrated);
                });
                this.persist_session_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn hydrate_track_list_artwork(&mut self, track_list_nonce: u64, cx: &mut Context<Self>) {
        let Some(track_list) = self.discover.read(cx).track_list() else {
            return;
        };

        let Some(provider) = self.provider_for_id(track_list.collection.reference.provider) else {
            self.status_message = Some(format!(
                "Provider '{}' is not available.",
                track_list.collection.reference.provider
            ));
            cx.notify();
            return;
        };
        let library = self.library.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let hydrated = cx
                .background_executor()
                .spawn(async move { hydrate_track_list_artwork(library, provider, track_list) })
                .await;

            let _ = this.update(cx, move |this, cx| {
                let matches_nonce = this
                    .discover
                    .read(cx)
                    .track_list_nonce_matches(track_list_nonce);
                if !matches_nonce {
                    return;
                }

                let collection_key = collection_entity_key(&hydrated.collection.reference);
                this.discover.update(cx, |discover, _cx| {
                    discover.replace_track_list(hydrated.clone());
                });
                let _ = this
                    .library
                    .save_collection_track_list(&hydrated.collection.reference, &hydrated);
                if this
                    .playback_state
                    .read(cx)
                    .playback_context()
                    .as_ref()
                    .map(|context| {
                        collection_entity_key(&context.collection.reference) == collection_key
                    })
                    .unwrap_or(false)
                {
                    this.update_playback_state(cx, |state| {
                        state.replace_playback_context(hydrated.clone());
                    });
                }
                this.persist_session_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

async fn search_collections(
    providers: Vec<SharedProvider>,
    query: String,
) -> Result<(String, Vec<CollectionSummary>)> {
    let mut collections = Vec::new();

    for provider in providers {
        let results = provider.search(&query).await?;
        collections.extend(results.into_iter().filter_map(|result| match result {
            SearchResult::Collection(collection)
                if matches!(
                    collection.reference.kind,
                    CollectionKind::Album | CollectionKind::Playlist
                ) =>
            {
                Some(collection)
            }
            SearchResult::Collection(_) | SearchResult::Track(_) => None,
        }));
    }

    collections.sort_by(|left, right| {
        rank_collection(right, &query)
            .cmp(&rank_collection(left, &query))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            .then_with(|| {
                left.reference
                    .provider
                    .as_str()
                    .cmp(right.reference.provider.as_str())
            })
    });

    Ok((query, collections))
}

fn hydrate_collection_batch_artwork(
    library: Library,
    providers: Vec<SharedProvider>,
    collections: Vec<CollectionSummary>,
) -> Vec<CollectionSummary> {
    let provider_map = providers
        .into_iter()
        .map(|provider| (provider.id(), provider))
        .collect::<HashMap<_, _>>();

    collections
        .into_iter()
        .map(|mut collection| {
            let Some(provider) = provider_map.get(&collection.reference.provider) else {
                return collection;
            };

            match library.ensure_collection_artwork_cached(provider.as_ref(), &collection) {
                Ok(Some(path)) => {
                    collection.artwork_url = Some(path.to_string_lossy().into_owned());
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!(
                        "failed to hydrate artwork for collection '{}': {error}",
                        collection.title
                    );
                }
            }
            collection
        })
        .collect()
}

fn rank_collection(collection: &CollectionSummary, query: &str) -> i32 {
    let normalized_query = query.trim().to_ascii_lowercase();
    let normalized_title = collection.title.to_ascii_lowercase();
    let normalized_subtitle = collection
        .subtitle
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut score = collection.reference.provider.search_rank_bias();

    if normalized_title == normalized_query {
        score += 500;
    } else if normalized_title.contains(&normalized_query) {
        score += 250;
    }

    if !normalized_query.is_empty()
        && normalized_query
            .split_whitespace()
            .all(|term| normalized_title.contains(term) || normalized_subtitle.contains(term))
    {
        score += 120;
    }

    if normalized_subtitle.contains(&normalized_query) {
        score += 60;
    }

    score += match collection.reference.kind {
        CollectionKind::Album => 20,
        CollectionKind::Playlist => 10,
    };

    score += collection.track_count.unwrap_or(0).min(99) as i32;
    score
}

fn hydrate_track_list_artwork(
    library: Library,
    provider: SharedProvider,
    mut track_list: TrackList,
) -> TrackList {
    match library.ensure_collection_artwork_cached(provider.as_ref(), &track_list.collection) {
        Ok(Some(path)) => {
            track_list.collection.artwork_url = Some(path.to_string_lossy().into_owned());
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!(
                "failed to hydrate artwork for track list '{}': {error}",
                track_list.collection.title
            );
        }
    }

    track_list
}
