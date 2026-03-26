use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, GlobalElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window, fill, point, px, relative, rgb, rgba, size,
};

use crate::library::ImportMetadataField;
use crate::theme;

use super::OryxApp;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum TextInputId {
    Query,
    OpenUrl,
    ProviderAuthUsername,
    ProviderAuthPassword,
    ProviderLink,
    ImportManual {
        source_path: PathBuf,
        field: ImportMetadataField,
    },
}

impl TextInputId {
    pub(super) fn is_provider_auth(&self) -> bool {
        matches!(
            self,
            Self::ProviderAuthUsername | Self::ProviderAuthPassword
        )
    }

    pub(super) fn is_masked(&self) -> bool {
        matches!(self, Self::ProviderAuthPassword)
    }
}

pub(super) struct TextInputState {
    content: String,
    cursor: usize,
    selection_anchor: Option<usize>,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
}

impl TextInputState {
    pub(super) fn new(content: String, cursor: usize) -> Self {
        let mut state = Self {
            content,
            cursor: 0,
            selection_anchor: None,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            selecting: false,
        };
        state.cursor = state.clamp_index(cursor);
        state
    }

    pub(super) fn content(&self) -> &str {
        &self.content
    }

    pub(super) fn cursor(&self) -> usize {
        self.clamp_index(self.cursor)
    }

    pub(super) fn selection_anchor(&self) -> Option<usize> {
        self.selection_anchor
    }

    pub(super) fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor() {
            None
        } else {
            Some(anchor.min(self.cursor())..anchor.max(self.cursor()))
        }
    }

    pub(super) fn selected_range(&self) -> Range<usize> {
        self.clamp_range(
            self.selection_range()
                .unwrap_or(self.cursor()..self.cursor()),
        )
    }

    pub(super) fn selection_reversed(&self) -> bool {
        self.selection_anchor
            .map(|anchor| self.clamp_index(anchor) > self.cursor())
            .unwrap_or(false)
    }

    pub(super) fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.clamp_range(range.clone()))
    }

    pub(super) fn clear(&mut self) {
        self.content.clear();
        self.cursor = 0;
        self.selection_anchor = None;
        self.marked_range = None;
    }

    pub(super) fn reset(&mut self, content: String) {
        self.content = content;
        self.cursor = self.content.len();
        self.selection_anchor = None;
        self.marked_range = None;
        self.selecting = false;
    }

    pub(super) fn move_to(&mut self, offset: usize) {
        self.cursor = self.clamp_index(offset);
        self.selection_anchor = None;
        self.marked_range = None;
    }

    pub(super) fn select_to(&mut self, offset: usize) {
        self.selection_anchor.get_or_insert(self.cursor());
        self.cursor = self.clamp_index(offset);
        if self.selection_anchor == Some(self.cursor) {
            self.selection_anchor = None;
        }
        self.marked_range = None;
    }

    pub(super) fn set_selecting(&mut self, selecting: bool) {
        self.selecting = selecting;
    }

    pub(super) fn is_selecting(&self) -> bool {
        self.selecting
    }

    pub(super) fn clamp_index(&self, index: usize) -> usize {
        let clamped = index.min(self.content.len());
        if self.content.is_char_boundary(clamped) {
            return clamped;
        }

        self.content
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx < clamped)
            .last()
            .unwrap_or(0)
    }

    pub(super) fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_index(range.start);
        let end = self.clamp_index(range.end);
        start.min(end)..start.max(end)
    }

    fn resolve_range_utf16(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        self.clamp_range(
            range_utf16
                .as_ref()
                .map(|range| self.range_from_utf16(range))
                .or_else(|| self.marked_range())
                .or_else(|| self.selection_range())
                .unwrap_or_else(|| {
                    let cursor = self.cursor();
                    cursor..cursor
                }),
        )
    }

    pub(super) fn replace_text_in_range_utf16(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
    ) {
        let range = self.resolve_range_utf16(range_utf16);
        self.content.replace_range(range.clone(), text);
        self.cursor = self.clamp_index(range.start + text.len());
        self.selection_anchor = None;
        self.marked_range = None;
    }

    pub(super) fn replace_and_mark_text_in_range_utf16(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        let range = self.resolve_range_utf16(range_utf16);
        self.content.replace_range(range.clone(), new_text);
        self.marked_range = (!new_text.is_empty())
            .then_some(self.clamp_range(range.start..range.start + new_text.len()));

        let selected_range = new_selected_range_utf16
            .as_ref()
            .map(|selected| self.range_from_utf16(selected))
            .map(|selected| {
                self.clamp_range(range.start + selected.start..range.start + selected.end)
            })
            .unwrap_or_else(|| {
                let cursor = self.clamp_index(range.start + new_text.len());
                cursor..cursor
            });

        self.selection_anchor = if selected_range.is_empty() {
            None
        } else {
            Some(selected_range.start)
        };
        self.cursor = selected_range.end;
    }

    pub(super) fn set_cursor(&mut self, cursor: usize, select: bool) {
        let cursor = self.clamp_index(cursor);

        if select {
            self.selection_anchor.get_or_insert(self.cursor());
        } else {
            self.selection_anchor = None;
        }

        self.cursor = cursor;
        if self.selection_anchor == Some(self.cursor) {
            self.selection_anchor = None;
        }
        self.marked_range = None;
    }

    pub(super) fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };

        self.replace_range(range, "");
        true
    }

    pub(super) fn replace_range(&mut self, range: Range<usize>, text: &str) {
        let range = self.clamp_range(range);
        self.content.replace_range(range.clone(), text);
        self.cursor = self.clamp_index(range.start + text.len());
        self.selection_anchor = None;
        self.marked_range = None;
    }

    pub(super) fn move_left(&mut self, select: bool, by_word: bool) {
        if !select && let Some(range) = self.selection_range() {
            self.cursor = range.start;
            self.selection_anchor = None;
            return;
        }

        let target = if by_word {
            self.previous_word_boundary(self.cursor())
        } else {
            self.previous_boundary(self.cursor())
        };
        self.set_cursor(target, select);
    }

    pub(super) fn move_right(&mut self, select: bool, by_word: bool) {
        if !select && let Some(range) = self.selection_range() {
            self.cursor = range.end;
            self.selection_anchor = None;
            return;
        }

        let target = if by_word {
            self.next_word_boundary(self.cursor())
        } else {
            self.next_boundary(self.cursor())
        };
        self.set_cursor(target, select);
    }

    pub(super) fn move_to_start(&mut self, select: bool) {
        self.set_cursor(0, select);
    }

    pub(super) fn move_to_end(&mut self, select: bool) {
        self.set_cursor(self.content.len(), select);
    }

    pub(super) fn select_all(&mut self) {
        self.cursor = self.content.len();
        self.selection_anchor = Some(0);
        self.marked_range = None;
    }

    pub(super) fn delete_backward(&mut self) {
        if self.delete_selection() || self.cursor() == 0 {
            return;
        }

        let start = self.previous_boundary(self.cursor());
        self.replace_range(start..self.cursor(), "");
    }

    pub(super) fn delete_forward(&mut self) {
        if self.delete_selection() || self.cursor() >= self.content.len() {
            return;
        }

        let end = self.next_boundary(self.cursor());
        self.replace_range(self.cursor()..end, "");
    }

    pub(super) fn delete_word_backward(&mut self) {
        if self.delete_selection() || self.cursor() == 0 {
            return;
        }

        let start = self.previous_word_boundary(self.cursor());
        self.replace_range(start..self.cursor(), "");
    }

    pub(super) fn delete_to_start(&mut self) {
        if self.delete_selection() || self.cursor() == 0 {
            return;
        }

        self.replace_range(0..self.cursor(), "");
    }

    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    pub(super) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let range = self.clamp_range(range.clone());
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub(super) fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.clamp_range(
            self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end),
        )
    }

    pub(super) fn display_text(&self, placeholder: &str, masked: bool) -> String {
        if self.content.is_empty() {
            placeholder.to_string()
        } else if masked {
            "*".repeat(self.content.chars().count())
        } else {
            self.content.clone()
        }
    }

    pub(super) fn display_offset_from_content_offset(&self, offset: usize, masked: bool) -> usize {
        if !masked {
            return self.clamp_index(offset);
        }

        let offset = self.clamp_index(offset);
        let mut display_offset = 0;
        let mut utf8_offset = 0;
        for ch in self.content.chars() {
            if utf8_offset >= offset {
                break;
            }
            utf8_offset += ch.len_utf8();
            display_offset += 1;
        }
        display_offset
    }

    pub(super) fn content_offset_from_display_offset(
        &self,
        display_offset: usize,
        masked: bool,
    ) -> usize {
        if !masked {
            return self.clamp_index(display_offset);
        }

        let mut utf8_offset = 0;
        let mut remaining = display_offset;
        for ch in self.content.chars() {
            if remaining == 0 {
                break;
            }
            utf8_offset += ch.len_utf8();
            remaining -= 1;
        }
        utf8_offset
    }

    pub(super) fn display_range_from_content_range(
        &self,
        range: Range<usize>,
        masked: bool,
    ) -> Range<usize> {
        let range = self.clamp_range(range);
        self.display_offset_from_content_offset(range.start, masked)
            ..self.display_offset_from_content_offset(range.end, masked)
    }

    pub(super) fn set_layout(&mut self, line: ShapedLine, bounds: Bounds<Pixels>) {
        self.last_layout = Some(line);
        self.last_bounds = Some(bounds);
    }

    pub(super) fn bounds_for_range(
        &self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        masked: bool,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let range = self.display_range_from_content_range(range, masked);
        Some(Bounds::from_corners(
            point(
                element_bounds.left() + line.x_for_index(range.start),
                element_bounds.top(),
            ),
            point(
                element_bounds.left() + line.x_for_index(range.end),
                element_bounds.bottom(),
            ),
        ))
    }

    pub(super) fn character_index_for_point(
        &self,
        point: Point<Pixels>,
        masked: bool,
    ) -> Option<usize> {
        let bounds = self.last_bounds.as_ref()?;
        let line = self.last_layout.as_ref()?;
        let local_point = bounds.localize(&point)?;
        let display_index = line.index_for_x(point.x - local_point.x)?;
        Some(self.offset_to_utf16(self.content_offset_from_display_offset(display_index, masked)))
    }

    pub(super) fn index_for_mouse_position(&self, position: Point<Pixels>, masked: bool) -> usize {
        let Some(bounds) = self.last_bounds.as_ref() else {
            return self.content.len();
        };
        let Some(line) = self.last_layout.as_ref() else {
            return self.content.len();
        };

        if position.y <= bounds.top() {
            return 0;
        }
        if position.y >= bounds.bottom() {
            return self.content.len();
        }

        self.content_offset_from_display_offset(
            line.closest_index_for_x(position.x - bounds.left()),
            masked,
        )
    }

    fn previous_boundary(&self, index: usize) -> usize {
        if index == 0 {
            0
        } else {
            self.content[..index]
                .char_indices()
                .last()
                .map(|(offset, _)| offset)
                .unwrap_or(0)
        }
    }

    fn next_boundary(&self, index: usize) -> usize {
        if index >= self.content.len() {
            self.content.len()
        } else {
            index
                + self.content[index..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0)
        }
    }

    fn previous_word_boundary(&self, index: usize) -> usize {
        if index == 0 {
            return 0;
        }

        let chars: Vec<(usize, char)> = self.content[..index].char_indices().collect();
        let mut position = chars.len();

        while position > 0 && chars[position - 1].1.is_whitespace() {
            position -= 1;
        }

        if position == 0 {
            return 0;
        }

        let class = Self::char_class(chars[position - 1].1);
        while position > 0 {
            let ch = chars[position - 1].1;
            if ch.is_whitespace() || Self::char_class(ch) != class {
                break;
            }
            position -= 1;
        }

        chars.get(position).map(|(offset, _)| *offset).unwrap_or(0)
    }

    fn next_word_boundary(&self, index: usize) -> usize {
        if index >= self.content.len() {
            return self.content.len();
        }

        let mut found_class = None;
        let mut end = index;
        for (offset, ch) in self.content[index..].char_indices() {
            let absolute_offset = index + offset;
            if found_class.is_none() {
                if ch.is_whitespace() {
                    end = absolute_offset + ch.len_utf8();
                    continue;
                }

                found_class = Some(Self::char_class(ch));
                end = absolute_offset + ch.len_utf8();
                continue;
            }

            if ch.is_whitespace() || Some(Self::char_class(ch)) != found_class {
                return absolute_offset;
            }

            end = absolute_offset + ch.len_utf8();
        }

        end
    }

    fn char_class(ch: char) -> u8 {
        if ch.is_alphanumeric() { 0 } else { 1 }
    }
}

pub(super) struct TextInputElement {
    pub(super) app: Entity<OryxApp>,
    pub(super) input_id: TextInputId,
    pub(super) placeholder: &'static str,
    pub(super) masked: bool,
}

pub(super) struct TextInputPrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl OryxApp {
    pub(super) fn focused_text_input(&self, window: &Window) -> Option<TextInputId> {
        if self.provider_auth_password_focus_handle.is_focused(window) {
            Some(TextInputId::ProviderAuthPassword)
        } else if self.provider_auth_username_focus_handle.is_focused(window) {
            Some(TextInputId::ProviderAuthUsername)
        } else if self.open_url_focus_handle.is_focused(window) {
            Some(TextInputId::OpenUrl)
        } else if self.provider_link_focus_handle.is_focused(window) {
            Some(TextInputId::ProviderLink)
        } else if self.query_focus_handle.is_focused(window) {
            Some(TextInputId::Query)
        } else {
            self.import_review_input_focus_handles
                .iter()
                .find_map(|(input_id, handle)| handle.is_focused(window).then(|| input_id.clone()))
        }
    }

    pub(super) fn focus_text_input(&self, input_id: &TextInputId, window: &mut Window) {
        window.focus(self.text_input_focus_handle(input_id));
    }

    pub(super) fn text_input(&self, input_id: &TextInputId) -> &TextInputState {
        match input_id {
            TextInputId::Query => &self.query_input,
            TextInputId::OpenUrl => &self.open_url_input,
            TextInputId::ProviderAuthUsername => &self.provider_auth_username_input,
            TextInputId::ProviderAuthPassword => &self.provider_auth_password_input,
            TextInputId::ProviderLink => &self.provider_link_input,
            TextInputId::ImportManual { .. } => self
                .import_review_inputs
                .get(&input_id)
                .expect("import review input should exist"),
        }
    }

    pub(super) fn text_input_mut(&mut self, input_id: &TextInputId) -> &mut TextInputState {
        match input_id {
            TextInputId::Query => &mut self.query_input,
            TextInputId::OpenUrl => &mut self.open_url_input,
            TextInputId::ProviderAuthUsername => &mut self.provider_auth_username_input,
            TextInputId::ProviderAuthPassword => &mut self.provider_auth_password_input,
            TextInputId::ProviderLink => &mut self.provider_link_input,
            TextInputId::ImportManual { .. } => self
                .import_review_inputs
                .get_mut(&input_id)
                .expect("import review input should exist"),
        }
    }

    pub(super) fn text_input_focus_handle(&self, input_id: &TextInputId) -> &FocusHandle {
        match input_id {
            TextInputId::Query => &self.query_focus_handle,
            TextInputId::OpenUrl => &self.open_url_focus_handle,
            TextInputId::ProviderAuthUsername => &self.provider_auth_username_focus_handle,
            TextInputId::ProviderAuthPassword => &self.provider_auth_password_focus_handle,
            TextInputId::ProviderLink => &self.provider_link_focus_handle,
            TextInputId::ImportManual { .. } => self
                .import_review_input_focus_handles
                .get(&input_id)
                .expect("import review focus handle should exist"),
        }
    }

    pub(super) fn on_text_input_mouse_down(
        &mut self,
        input_id: TextInputId,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_text_input(&input_id, window);
        let index = self
            .text_input(&input_id)
            .index_for_mouse_position(event.position, input_id.is_masked());
        let input = self.text_input_mut(&input_id);
        input.set_selecting(true);
        if event.modifiers.shift {
            input.select_to(index);
        } else {
            input.move_to(index);
        }
        cx.notify();
    }

    pub(super) fn on_text_input_mouse_up(
        &mut self,
        input_id: TextInputId,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.text_input_mut(&input_id).set_selecting(false);
    }

    pub(super) fn on_text_input_mouse_move(
        &mut self,
        input_id: TextInputId,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next_index = {
            let input = self.text_input(&input_id);
            if !input.is_selecting() {
                return;
            }
            input.index_for_mouse_position(event.position, input_id.is_masked())
        };
        self.text_input_mut(&input_id).select_to(next_index);
        cx.notify();
    }

    pub(super) fn handle_text_input_edited(
        &mut self,
        input_id: TextInputId,
        cx: &mut Context<Self>,
    ) {
        if input_id == TextInputId::Query {
            self.persist_session_snapshot(cx);
        }

        if input_id.is_provider_auth() {
            self.update_ui_state(cx, |state| {
                state.set_provider_auth_error(None);
            });
        }

        if input_id == TextInputId::ProviderLink {
            self.update_ui_state(cx, |state| {
                state.set_provider_link_error(None);
            });
        }

        if input_id == TextInputId::OpenUrl {
            self.update_ui_state(cx, |state| {
                state.set_open_url_error(None);
            });
        }

        if let TextInputId::ImportManual { source_path, field } = input_id {
            let value = self
                .import_review_inputs
                .get(&TextInputId::ImportManual {
                    source_path: source_path.clone(),
                    field,
                })
                .map(|input| input.content().to_string())
                .unwrap_or_default();
            self.update_pending_import_review_manual_field(source_path, field, value, cx);
        }
    }
}

impl EntityInputHandler for OryxApp {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let input_id = self.focused_text_input(window)?;
        let input = self.text_input_mut(&input_id);
        let range = input.range_from_utf16(&range_utf16);
        actual_range.replace(input.range_to_utf16(&range));
        Some(input.content()[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let input_id = self.focused_text_input(window)?;
        let input = self.text_input_mut(&input_id);
        Some(UTF16Selection {
            range: input.range_to_utf16(&input.selected_range()),
            reversed: input.selection_reversed(),
        })
    }

    fn marked_text_range(
        &self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let input_id = self.focused_text_input(window)?;
        self.text_input(&input_id)
            .marked_range()
            .as_ref()
            .map(|range| self.text_input(&input_id).range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        let Some(input_id) = self.focused_text_input(window) else {
            return;
        };
        self.text_input_mut(&input_id).marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input_id) = self.focused_text_input(window) else {
            return;
        };
        self.text_input_mut(&input_id)
            .replace_text_in_range_utf16(range_utf16, text);
        self.handle_text_input_edited(input_id, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input_id) = self.focused_text_input(window) else {
            return;
        };
        self.text_input_mut(&input_id)
            .replace_and_mark_text_in_range_utf16(range_utf16, new_text, new_selected_range_utf16);
        self.handle_text_input_edited(input_id, cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let input_id = self.focused_text_input(window)?;
        self.text_input(&input_id).bounds_for_range(
            range_utf16,
            element_bounds,
            input_id.is_masked(),
        )
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let input_id = self.focused_text_input(window)?;
        self.text_input(&input_id)
            .character_index_for_point(point, input_id.is_masked())
    }
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = TextInputPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let app = self.app.read(cx);
        let input = app.text_input(&self.input_id);
        let content = input.content().to_string();
        let selected_range =
            input.display_range_from_content_range(input.selected_range(), self.masked);
        let cursor = input.display_offset_from_content_offset(input.cursor(), self.masked);
        let style = window.text_style();
        let display_text = input.display_text(self.placeholder, self.masked);

        let base_run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: if content.is_empty() {
                rgb(theme::TEXT_DIM).into()
            } else {
                rgb(theme::TEXT_PRIMARY).into()
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = if let Some(marked_range) = input
            .marked_range()
            .map(|range| input.display_range_from_content_range(range, self.masked))
        {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(base_run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len().saturating_sub(marked_range.end),
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![base_run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text.into(), font_size, &runs, None);

        let selection = if selected_range.is_empty() || content.is_empty() {
            None
        } else {
            Some(fill(
                Bounds::from_corners(
                    point(
                        bounds.left() + line.x_for_index(selected_range.start),
                        bounds.top(),
                    ),
                    point(
                        bounds.left() + line.x_for_index(selected_range.end),
                        bounds.bottom(),
                    ),
                ),
                rgba(0x55EA9738),
            ))
        };

        let cursor = Some(fill(
            Bounds::new(
                point(bounds.left() + line.x_for_index(cursor), bounds.top()),
                size(px(2.), bounds.bottom() - bounds.top()),
            ),
            rgb(theme::ACCENT_PRIMARY),
        ));

        TextInputPrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self
            .app
            .read(cx)
            .text_input_focus_handle(&self.input_id)
            .clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.app.clone()),
            cx,
        );

        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }

        let line = prepaint.line.take().unwrap();
        let _ = line.paint(bounds.origin, window.line_height(), window, cx);

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        let input_id = self.input_id.clone();
        self.app.clone().update(cx, |app, _cx| {
            app.text_input_mut(&input_id).set_layout(line, bounds);
        });
    }
}
