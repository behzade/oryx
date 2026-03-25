use gpui::{App, AsyncApp, Context, PathPromptOptions, Window};

use crate::library::{ImportMetadataField, ImportReview};
use crate::provider::{ProviderId, TrackList, TrackSummary};

use super::super::playback::PlaybackContextTrackRemoval;
use super::super::text_input::TextInputId;
use super::super::ui::NotificationLevel;
use super::super::{BrowseMode, OryxApp};

impl OryxApp {
    fn set_pending_import_review_state(&mut self, review: ImportReview, cx: &mut Context<Self>) {
        self.update_ui_state(cx, |state| {
            state.set_pending_import_review(review);
        });
        self.sync_import_review_text_inputs(cx);
    }

    fn clear_pending_import_review_state(&mut self, cx: &mut Context<Self>) {
        self.update_ui_state(cx, |state| {
            state.clear_pending_import_review();
        });
        self.sync_import_review_text_inputs(cx);
    }

    fn with_pending_import_review_mut(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut ImportReview),
    ) -> bool {
        let updated = self.ui_state.update(cx, |state, _cx| {
            let Some(review) = state.pending_import_review.as_mut() else {
                return false;
            };
            update(review);
            review.refresh_album_summaries();
            true
        });
        if updated {
            self.sync_import_review_text_inputs(cx);
        }
        updated
    }

    pub(in crate::app) fn sync_import_review_text_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.ui_state.read(cx).pending_import_review() else {
            self.import_review_inputs.clear();
            self.import_review_input_focus_handles.clear();
            return;
        };

        let desired_inputs = review
            .albums
            .iter()
            .flat_map(|album| album.tracks.iter())
            .filter(|track| track.manual_mode && !track.skipped)
            .flat_map(|track| {
                [
                    (
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::Title,
                        },
                        track.manual_metadata.title.clone(),
                    ),
                    (
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::Artist,
                        },
                        track.manual_metadata.artist.clone(),
                    ),
                    (
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::Album,
                        },
                        track.manual_metadata.album.clone(),
                    ),
                    (
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::AlbumArtist,
                        },
                        track.manual_metadata.album_artist.clone(),
                    ),
                    (
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::DiscNumber,
                        },
                        track.manual_metadata.disc_number.clone(),
                    ),
                    (
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::TrackNumber,
                        },
                        track.manual_metadata.track_number.clone(),
                    ),
                ]
            })
            .collect::<Vec<_>>();

        let desired_ids = desired_inputs
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.import_review_inputs
            .retain(|input_id, _| desired_ids.contains(input_id));
        self.import_review_input_focus_handles
            .retain(|input_id, _| desired_ids.contains(input_id));

        for (input_id, value) in desired_inputs {
            self.import_review_inputs
                .entry(input_id.clone())
                .or_insert_with(|| {
                    let cursor = value.len();
                    super::super::text_input::TextInputState::new(value.clone(), cursor)
                });
            self.import_review_input_focus_handles
                .entry(input_id)
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
    }

    pub(in crate::app) fn update_pending_import_review_manual_field(
        &mut self,
        source_path: std::path::PathBuf,
        field: ImportMetadataField,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.with_pending_import_review_mut(cx, |review| {
            if let Some(track) = review.track_mut(&source_path) {
                track.set_manual_field(field, value);
            }
        }) {
            cx.notify();
        }
    }

    pub(in crate::app) fn skip_pending_import_track(
        &mut self,
        source_path: std::path::PathBuf,
        skipped: bool,
        cx: &mut Context<Self>,
    ) {
        if self.with_pending_import_review_mut(cx, |review| {
            if let Some(track) = review.track_mut(&source_path) {
                track.mark_skipped(skipped);
            }
        }) {
            cx.notify();
        }
    }

    pub(in crate::app) fn begin_pending_import_track_manual_entry(
        &mut self,
        source_path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.with_pending_import_review_mut(cx, |review| {
            if let Some(track) = review.track_mut(&source_path) {
                track.begin_manual_entry();
            }
        }) {
            self.focus_text_input(
                &TextInputId::ImportManual {
                    source_path,
                    field: ImportMetadataField::Title,
                },
                window,
            );
            cx.notify();
        }
    }

    pub(in crate::app) fn resolve_pending_import_track_online(
        &mut self,
        source_path: std::path::PathBuf,
        analysis_path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let library = self.library.clone();
        self.status_message = Some(format!(
            "Resolving '{}' with online services...",
            source_path.display()
        ));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let resolution = cx
                .background_executor()
                .spawn(async move { library.resolve_import_track_online(&analysis_path) })
                .await;

            let _ = this.update_in(cx, |this, _window, cx| {
                match resolution {
                    Ok((metadata, artwork_url)) => {
                        let resolved_title = metadata.title.clone();
                        if this.with_pending_import_review_mut(cx, |review| {
                            if let Some(track) = review.track_mut(&source_path) {
                                track.apply_online_metadata(metadata, artwork_url);
                            }
                        }) {
                            this.status_message = Some(format!(
                                "Resolved '{}' with online metadata.",
                                resolved_title
                            ));
                        }
                    }
                    Err(error) => {
                        let message = format!(
                            "Online lookup failed for '{}': {error:#}",
                            source_path.display()
                        );
                        let _ = this.with_pending_import_review_mut(cx, |review| {
                            if let Some(track) = review.track_mut(&source_path) {
                                track.issue = Some(format!("Online lookup failed: {error:#}"));
                            }
                        });
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Error, cx);
                    }
                }
                cx.notify();
            });

            anyhow::Ok(())
        })
        .detach();
    }

    pub(in crate::app) fn refresh_local_library_views(&mut self, cx: &mut Context<Self>) {
        self.library_catalog
            .update(cx, |catalog, _cx| catalog.refresh());
    }

    pub(in crate::app) fn delete_local_album_from_library(
        &mut self,
        provider: ProviderId,
        collection_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        self.clear_visible_local_track_list_override();
        let library = self.library.clone();
        self.status_message = Some(format!("Removing '{title}' from library..."));
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { library.delete_collection_from_library(provider, &collection_id) },
                )
                .await;

            let _ = this.update(cx, move |this, cx| {
                match result {
                    Ok(deleted_rows) => {
                        this.refresh_local_library_views(cx);
                        this.persist_session_snapshot(cx);
                        let message = if deleted_rows == 0 {
                            format!("No cached tracks found for '{title}'.")
                        } else {
                            format!("Removed '{title}' from library.")
                        };
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Success, cx);
                    }
                    Err(error) => {
                        let message = format!("Failed to remove '{title}': {error}");
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::app) fn delete_local_track_from_library(
        &mut self,
        provider: ProviderId,
        track_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        let browse_mode = self.browse_mode;
        let visible_track_list_before_delete = self.current_visible_track_list_cloned(cx);
        let library = self.library.clone();
        self.status_message = Some(format!("Removing '{title}' from library..."));
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let delete_track_id = track_id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { library.delete_track_from_library(provider, &delete_track_id) })
                .await;

            let _ = this.update(cx, move |this, cx| {
                match result {
                    Ok(deleted) => {
                        this.refresh_local_library_views(cx);
                        if should_preserve_visible_track_list_after_delete(
                            browse_mode,
                            visible_track_list_before_delete.as_ref(),
                            this.current_visible_track_list(cx).as_ref(),
                            provider,
                            &track_id,
                        ) {
                            if let Some(track_list) = visible_track_list_before_delete.as_ref() {
                                this.set_visible_local_track_list_override(
                                    browse_mode,
                                    track_list.clone(),
                                );
                            }
                        } else {
                            this.clear_visible_local_track_list_override();
                        }
                        let playback_change =
                            this.remove_track_from_playback_context(provider, &track_id, cx);
                        this.persist_session_snapshot(cx);
                        let message = if !deleted {
                            format!("No cached track found for '{title}'.")
                        } else if matches!(
                            playback_change,
                            PlaybackContextTrackRemoval::CurrentTrackRemoved
                        ) {
                            format!("Removed '{title}' from library and stopped playback.")
                        } else {
                            format!("Removed '{title}' from library.")
                        };
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Success, cx);
                    }
                    Err(error) => {
                        let message = format!("Failed to remove '{title}': {error}");
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::app) fn local_collections(&self, mode: BrowseMode, cx: &App) -> Vec<TrackList> {
        self.library_catalog.read(cx).local_collections_owned(mode)
    }

    pub(in crate::app) fn selected_local_collection_id(
        &self,
        mode: BrowseMode,
        cx: &App,
    ) -> Option<String> {
        self.library_catalog
            .read(cx)
            .selected_local_collection_id_owned(mode)
    }

    pub(in crate::app) fn select_local_collection(
        &mut self,
        mode: BrowseMode,
        id: String,
        cx: &mut Context<Self>,
    ) {
        self.clear_visible_local_track_list_override();
        self.library_catalog
            .update(cx, |catalog, _cx| catalog.select_local_collection(mode, id));
        self.persist_session_snapshot(cx);
    }

    pub(in crate::app) fn track_is_downloading(&self, track: &TrackSummary, cx: &App) -> bool {
        self.transfer_state.read(cx).track_is_downloading(track)
    }

    fn remove_track_from_playback_context(
        &mut self,
        provider: ProviderId,
        track_id: &str,
        cx: &mut Context<Self>,
    ) -> PlaybackContextTrackRemoval {
        let removal = self
            .playback_state
            .update(cx, |state, _cx| state.remove_track(provider, track_id));

        if matches!(removal, PlaybackContextTrackRemoval::CurrentTrackRemoved) {
            let _ = self.playback_state.read(cx).stop();
            self.playback_state
                .read(cx)
                .publish_restored_media_session();
        }

        removal
    }

    pub(in crate::app) fn prompt_for_import_folder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_message = Some("Choose files or folders to import.".to_string());
        cx.notify();

        let folder_prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Import".into()),
        });
        let library = self.library.clone();

        cx.spawn_in(window, async move |this, cx| {
            let result = folder_prompt.await;
            match result {
                Ok(Ok(Some(paths))) => {
                    if paths.is_empty() {
                        return anyhow::Ok(());
                    }
                    let path_count = paths.len();
                    let selection_label = if path_count == 1 {
                        paths[0].display().to_string()
                    } else {
                        format!("{path_count} selections")
                    };
                    this.update_in(cx, |this, _window, cx| {
                        this.discard_pending_import_review(cx);
                        this.discover.update(cx, |discover, _cx| {
                            discover.close_source_picker();
                        });
                        this.update_ui_state(cx, |state| {
                            state.begin_import_review_analysis();
                        });
                        this.status_message = Some(format!("Analyzing {}...", selection_label));
                        cx.notify();
                    })?;

                    let import_result = cx
                        .background_executor()
                        .spawn(async move { library.stage_local_selection(&paths) })
                        .await;

                    let _ = this.update_in(cx, |this, _window, cx| {
                        this.update_ui_state(cx, |state| {
                            state.finish_import_review_loading();
                        });
                        match import_result {
                            Ok(review) => {
                                let matched = review.matched_track_count();
                                let unresolved = review.unresolved_track_count();
                                let album_count = review.albums.len();
                                this.set_pending_import_review_state(review, cx);
                                this.status_message = Some(format!(
                                    "Review {} album group(s): {} ready offline, {} need attention.",
                                    album_count, matched, unresolved
                                ));
                            }
                            Err(error) => {
                                this.status_message = Some(format!("Import failed: {error:#}"));
                            }
                        }
                        cx.notify();
                    });
                }
                Ok(Ok(None)) => {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        this.status_message = Some("Import selection cancelled.".to_string());
                        cx.notify();
                    });
                }
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        this.status_message =
                            Some(format!("Failed to open import picker: {error}"));
                        cx.notify();
                    });
                }
                Err(_canceled) => {}
            }
            anyhow::Ok(())
        })
        .detach();
    }

    fn discard_pending_import_review(&mut self, cx: &mut Context<Self>) {
        if let Some(review) = self
            .ui_state
            .update(cx, |state, _cx| state.take_pending_import_review())
            && let Err(error) = self.library.cleanup_import_review(&review)
        {
            eprintln!("failed to clean up import review staging: {error:#}");
        }
        self.sync_import_review_text_inputs(cx);
    }

    pub(in crate::app) fn cancel_pending_import_review(&mut self, cx: &mut Context<Self>) {
        self.discard_pending_import_review(cx);
        self.update_ui_state(cx, |state| {
            state.finish_import_review_loading();
        });
        self.status_message = Some("Import review dismissed.".to_string());
        self.show_notification("Import review dismissed.", NotificationLevel::Info, cx);
        cx.notify();
    }

    pub(in crate::app) fn commit_pending_import_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(review) = self.ui_state.read(cx).pending_import_review() else {
            self.status_message = Some("No pending import review.".to_string());
            self.show_notification("No pending import review.", NotificationLevel::Error, cx);
            cx.notify();
            return;
        };

        let library = self.library.clone();
        self.update_ui_state(cx, |state| {
            state.begin_import_review_loading();
        });
        self.status_message = Some("Importing reviewed files...".to_string());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let import_result = cx
                .background_executor()
                .spawn(async move { library.commit_import_review(&review) })
                .await;

            let _ = this.update_in(cx, |this, _window, cx| {
                this.update_ui_state(cx, |state| {
                    state.finish_import_review_loading();
                });
                match import_result {
                    Ok(summary) => {
                        this.clear_pending_import_review_state(cx);
                        this.refresh_local_library_views(cx);
                        let mut message = format!(
                            "Imported {} track(s) across {} album(s). Skipped {} file(s).",
                            summary.imported_tracks, summary.imported_albums, summary.skipped_files
                        );
                        if !summary.error_samples.is_empty() {
                            message.push_str(" Errors: ");
                            message.push_str(&summary.error_samples.join(" | "));
                        }
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Success, cx);
                    }
                    Err(error) => {
                        let message = format!("Import failed: {error:#}");
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Error, cx);
                    }
                }
                cx.notify();
            });

            anyhow::Ok(())
        })
        .detach();
    }

    pub(in crate::app) fn refresh_local_artwork(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let library = self.library.clone();
        self.status_message = Some("Refreshing local artwork...".to_string());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let refresh_result = cx
                .background_executor()
                .spawn(async move { library.backfill_local_artwork() })
                .await;

            let _ = this.update_in(cx, |this, _window, cx| {
                match refresh_result {
                    Ok(summary) => {
                        this.refresh_local_library_views(cx);
                        let mut message = format!(
                            "Checked {} missing-cover album(s). Updated {} album(s). Skipped {} album(s).",
                            summary.inspected_albums,
                            summary.updated_albums,
                            summary.skipped_albums
                        );
                        if !summary.error_samples.is_empty() {
                            message.push_str(" Errors: ");
                            message.push_str(&summary.error_samples.join(" | "));
                        } else if summary.skipped_albums > 0 {
                            message.push_str(" Check terminal logs for skip reasons.");
                        }
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Success, cx);
                    }
                    Err(error) => {
                        let message = format!("Artwork refresh failed: {error:#}");
                        this.status_message = Some(message.clone());
                        this.show_notification(message, NotificationLevel::Error, cx);
                    }
                }
                cx.notify();
            });

            anyhow::Ok(())
        })
        .detach();
    }
}

fn should_preserve_visible_track_list_after_delete(
    browse_mode: BrowseMode,
    previous_track_list: Option<&TrackList>,
    current_track_list: Option<&TrackList>,
    provider: ProviderId,
    track_id: &str,
) -> bool {
    if matches!(browse_mode, BrowseMode::Discover) {
        return false;
    }

    let Some(previous_track_list) = previous_track_list else {
        return false;
    };

    if !track_list_contains_track(previous_track_list, provider, track_id) {
        return false;
    }

    let current_matches_previous_collection = current_track_list.map(|track_list| {
        track_list.collection.reference.id == previous_track_list.collection.reference.id
    });
    let current_contains_deleted_track = current_track_list
        .is_some_and(|track_list| track_list_contains_track(track_list, provider, track_id));

    !current_matches_previous_collection.unwrap_or(false) || !current_contains_deleted_track
}

fn track_list_contains_track(track_list: &TrackList, provider: ProviderId, track_id: &str) -> bool {
    track_list
        .tracks
        .iter()
        .any(|track| track.reference.provider == provider && track.reference.id == track_id)
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
