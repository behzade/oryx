use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, anyhow, bail};
use html_escape::decode_html_entities;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;

use super::super::CollectionKind;
use super::config::{
    FieldTransform, HtmlFieldSpec, JsFieldSpec, JsonFieldSpec, SearchItemKind, SearchResultKindSpec,
};

pub(super) fn collection_kind_label(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Album => "album",
        CollectionKind::Playlist => "playlist",
    }
}

pub(super) fn render_optional_request_url(
    template: Option<&str>,
    context: &HashMap<String, String>,
) -> Result<String> {
    let Some(template) = template else {
        bail!("request is missing a url");
    };

    Ok(render_template(template, context))
}

pub(super) fn render_template(template: &str, context: &HashMap<String, String>) -> String {
    let mut rendered = String::new();
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '{' {
            rendered.push(ch);
            continue;
        }

        let mut key = String::new();
        while let Some(next) = chars.peek().copied() {
            chars.next();
            if next == '}' {
                break;
            }
            key.push(next);
        }

        if key.is_empty() {
            rendered.push_str("{}");
            continue;
        }

        if let Some(value) = context.get(&key) {
            rendered.push_str(value);
        } else {
            rendered.push('{');
            rendered.push_str(&key);
            rendered.push('}');
        }
    }

    rendered
}

pub(super) fn selector(pattern: &str) -> Result<Selector> {
    Selector::parse(pattern).map_err(|error| anyhow!("invalid CSS selector {pattern:?}: {error}"))
}

fn normalized_text(element: ElementRef<'_>) -> String {
    normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "))
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn extract_html_fields_from_document(
    document: &Html,
    specs: &BTreeMap<String, HtmlFieldSpec>,
) -> Result<HashMap<String, String>> {
    extract_html_fields(specs, |field| {
        extract_html_field_from_document(document, field)
    })
}

pub(super) fn extract_html_fields_from_item(
    item: ElementRef<'_>,
    specs: &BTreeMap<String, HtmlFieldSpec>,
) -> Result<HashMap<String, String>> {
    extract_html_fields(specs, |field| extract_html_field_from_item(item, field))
}

pub(super) fn extract_indexed_html_fields_from_document(
    document: &Html,
    specs: &BTreeMap<String, HtmlFieldSpec>,
) -> Result<HashMap<String, Vec<String>>> {
    let mut values = HashMap::new();

    for (name, spec) in specs {
        let Some(selector_text) = spec.selector.as_deref() else {
            continue;
        };
        let selector = selector(selector_text)?;
        let mut items = Vec::new();
        for element in document.select(&selector) {
            if let Some(value) = extract_html_field_value(element, spec) {
                items.push(apply_transforms(value, &spec.transforms));
            }
        }
        values.insert(name.clone(), items);
    }

    Ok(values)
}

fn extract_html_fields<F>(
    specs: &BTreeMap<String, HtmlFieldSpec>,
    mut direct_extractor: F,
) -> Result<HashMap<String, String>>
where
    F: FnMut(&HtmlFieldSpec) -> Result<Option<String>>,
{
    let mut values = HashMap::new();

    for (name, spec) in specs {
        if spec.source.is_some() {
            continue;
        }

        if let Some(value) = direct_extractor(spec)? {
            values.insert(name.clone(), apply_transforms(value, &spec.transforms));
        }
    }

    let mut pending = specs
        .iter()
        .filter(|(_, spec)| spec.source.is_some() || spec.value.is_some())
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let mut last_pending_len = pending.len() + 1;

    while !pending.is_empty() && pending.len() < last_pending_len {
        last_pending_len = pending.len();
        pending.retain(|name| {
            let spec = &specs[name];
            if let Some(value) = resolve_deferred_html_field(spec, &values) {
                values.insert(name.clone(), apply_transforms(value, &spec.transforms));
                false
            } else {
                true
            }
        });
    }

    Ok(values)
}

fn resolve_deferred_html_field(
    spec: &HtmlFieldSpec,
    values: &HashMap<String, String>,
) -> Option<String> {
    if let Some(source) = spec.source.as_deref() {
        values.get(source).cloned()
    } else if let Some(value) = spec.value.as_deref() {
        Some(render_template(value, values))
    } else {
        None
    }
}

fn extract_html_field_from_document(
    document: &Html,
    spec: &HtmlFieldSpec,
) -> Result<Option<String>> {
    if let Some(value) = spec.value.as_deref() {
        return Ok(Some(value.to_string()));
    }
    let Some(selector_text) = spec.selector.as_deref() else {
        return Ok(None);
    };
    let selector = selector(selector_text)?;
    let Some(element) = document.select(&selector).next() else {
        return Ok(None);
    };
    Ok(extract_html_field_value(element, spec))
}

fn extract_html_field_from_item(
    item: ElementRef<'_>,
    spec: &HtmlFieldSpec,
) -> Result<Option<String>> {
    if let Some(value) = spec.value.as_deref() {
        return Ok(Some(value.to_string()));
    }
    let Some(selector_text) = spec.selector.as_deref() else {
        return Ok(extract_html_field_value(item, spec));
    };
    let selector = selector(selector_text)?;
    let Some(element) = item.select(&selector).next() else {
        return Ok(None);
    };
    Ok(extract_html_field_value(element, spec))
}

fn extract_html_field_value(element: ElementRef<'_>, spec: &HtmlFieldSpec) -> Option<String> {
    if let Some(attr) = spec.attr.as_deref() {
        element
            .value()
            .attr(attr)
            .map(decode_html_entities)
            .map(|value| value.to_string())
    } else if spec.text || spec.selector.is_some() {
        Some(normalized_text(element))
    } else {
        None
    }
}

pub(super) fn extract_json_fields(
    json: &Value,
    specs: &BTreeMap<String, JsonFieldSpec>,
) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();

    for (name, spec) in specs {
        if spec.source.is_some() {
            continue;
        }
        if let Some(value) = extract_json_field(json, spec)? {
            values.insert(name.clone(), apply_transforms(value, &spec.transforms));
        }
    }

    let mut pending = specs
        .iter()
        .filter(|(_, spec)| spec.source.is_some() || spec.value.is_some())
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let mut last_pending_len = pending.len() + 1;

    while !pending.is_empty() && pending.len() < last_pending_len {
        last_pending_len = pending.len();
        pending.retain(|name| {
            let spec = &specs[name];
            let value = if let Some(source) = spec.source.as_deref() {
                values.get(source).cloned()
            } else if let Some(value) = spec.value.as_deref() {
                Some(render_template(value, &values))
            } else {
                None
            };
            if let Some(value) = value {
                values.insert(name.clone(), apply_transforms(value, &spec.transforms));
                false
            } else {
                true
            }
        });
    }

    Ok(values)
}

pub(super) fn extract_js_fields(
    object: &str,
    specs: &BTreeMap<String, JsFieldSpec>,
) -> HashMap<String, String> {
    let mut values = HashMap::new();

    for (name, spec) in specs {
        if spec.source.is_some() {
            continue;
        }

        let value = if let Some(field) = spec.field.as_deref() {
            if spec.raw {
                extract_js_string_field_raw(object, field)
            } else {
                extract_js_string_field(object, field)
            }
        } else {
            spec.value.clone()
        };

        if let Some(value) = value {
            values.insert(name.clone(), apply_transforms(value, &spec.transforms));
        }
    }

    let mut pending = specs
        .iter()
        .filter(|(_, spec)| spec.source.is_some() || spec.value.is_some())
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let mut last_pending_len = pending.len() + 1;

    while !pending.is_empty() && pending.len() < last_pending_len {
        last_pending_len = pending.len();
        pending.retain(|name| {
            let spec = &specs[name];
            let value = if let Some(source) = spec.source.as_deref() {
                values.get(source).cloned()
            } else {
                spec.value
                    .as_deref()
                    .map(|value| render_template(value, &values))
            };

            if let Some(value) = value {
                values.insert(name.clone(), apply_transforms(value, &spec.transforms));
                false
            } else {
                true
            }
        });
    }

    values
}

fn extract_json_field(json: &Value, spec: &JsonFieldSpec) -> Result<Option<String>> {
    if let Some(value) = spec.value.as_deref() {
        return Ok(Some(value.to_string()));
    }

    let Some(path) = spec.path.as_deref() else {
        return Ok(None);
    };
    let Some(value) = resolve_json_value(json, path)? else {
        return Ok(None);
    };
    Ok(json_value_to_string(value))
}

fn json_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        _ => Some(value.to_string()),
    }
}

fn apply_transforms(mut value: String, transforms: &[FieldTransform]) -> String {
    for transform in transforms {
        value = match transform {
            FieldTransform::Trim => value.trim().to_string(),
            FieldTransform::Lowercase => value.to_ascii_lowercase(),
            FieldTransform::Uppercase => value.to_ascii_uppercase(),
            FieldTransform::NormalizeWhitespace => normalize_whitespace(&value),
            FieldTransform::DecodeHtml => decode_html_entities(&value).to_string(),
            FieldTransform::UrlPathId => url_path_id(&value),
        };
    }
    value
}

pub(super) fn resolve_search_item_kind(
    fields: &HashMap<String, String>,
    spec: &SearchResultKindSpec,
) -> Option<SearchItemKind> {
    let field_value = spec
        .field
        .as_deref()
        .and_then(|field| fields.get(field))
        .map(|value| value.to_ascii_lowercase());

    if let Some(field_value) = field_value.as_deref() {
        for rule in &spec.rules {
            if field_value.contains(&rule.contains.to_ascii_lowercase()) {
                return parse_search_item_kind(&rule.result);
            }
        }
    }

    spec.default.as_deref().and_then(parse_search_item_kind)
}

fn parse_search_item_kind(value: &str) -> Option<SearchItemKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "track" => Some(SearchItemKind::Track),
        "collection:album" | "album" => Some(SearchItemKind::Collection(CollectionKind::Album)),
        "collection:playlist" | "playlist" => {
            Some(SearchItemKind::Collection(CollectionKind::Playlist))
        }
        _ => None,
    }
}

pub(super) fn resolve_json_items<'a>(json: &'a Value, path: &str) -> Result<Vec<&'a Value>> {
    let Some(value) = resolve_json_value(json, path)? else {
        return Ok(Vec::new());
    };

    match value {
        Value::Array(items) => Ok(items.iter().collect()),
        _ => Ok(vec![value]),
    }
}

fn resolve_json_value<'a>(json: &'a Value, path: &str) -> Result<Option<&'a Value>> {
    if path.trim().is_empty() || path == "$" {
        return Ok(Some(json));
    }

    let mut current = json;
    for raw_segment in path.split('.') {
        if raw_segment.is_empty() {
            continue;
        }

        let (field, index) = parse_json_path_segment(raw_segment)?;
        if let Some(field) = field {
            current = match current {
                Value::Object(map) => match map.get(field) {
                    Some(value) => value,
                    None => return Ok(None),
                },
                _ => return Ok(None),
            };
        }

        if let Some(index) = index {
            current = match current {
                Value::Array(items) => items.get(index).ok_or_else(|| {
                    anyhow!("json path segment {raw_segment:?} indexed beyond array length")
                })?,
                _ => return Ok(None),
            };
        }
    }

    Ok(Some(current))
}

fn parse_json_path_segment(segment: &str) -> Result<(Option<&str>, Option<usize>)> {
    if let Some((field, rest)) = segment.split_once('[') {
        let Some(index_text) = rest.strip_suffix(']') else {
            bail!("invalid json path segment {segment:?}");
        };
        let index = index_text
            .parse::<usize>()
            .map_err(|_| anyhow!("invalid array index in json path segment {segment:?}"))?;
        let field = (!field.is_empty()).then_some(field);
        Ok((field, Some(index)))
    } else {
        Ok((Some(segment), None))
    }
}

pub(super) fn parse_duration_value(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .or_else(|| parse_clock_duration(value))
}

fn parse_clock_duration(value: &str) -> Option<u32> {
    let parts = value
        .trim()
        .split(':')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    match parts.as_slice() {
        [minutes, seconds] => {
            Some(minutes.parse::<u32>().ok()? * 60 + seconds.parse::<u32>().ok()?)
        }
        [hours, minutes, seconds] => Some(
            hours.parse::<u32>().ok()? * 3600
                + minutes.parse::<u32>().ok()? * 60
                + seconds.parse::<u32>().ok()?,
        ),
        _ => None,
    }
}

pub(super) fn merge_response_cookies(
    response: &ureq::Response,
    cookie_jar: &mut BTreeMap<String, String>,
) {
    for cookie in response.all("Set-Cookie") {
        if let Some((name, value)) = parse_set_cookie(cookie) {
            cookie_jar.insert(name, value);
        }
    }
}

fn parse_set_cookie(cookie: &str) -> Option<(String, String)> {
    let pair = cookie.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    Some((name.to_string(), value.to_string()))
}

pub(super) fn cookie_header(cookie_jar: &BTreeMap<String, String>) -> Option<String> {
    (!cookie_jar.is_empty()).then(|| {
        cookie_jar
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

pub(super) fn url_path_id(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(url)
        .to_string()
}

pub(super) fn extract_script_block(
    html: &str,
    start_marker: &str,
    end_marker: &str,
) -> Result<String> {
    let start = html
        .find(start_marker)
        .ok_or_else(|| anyhow!("html document did not contain script marker {start_marker:?}"))?;
    let script = &html[start..];
    let end = script
        .find(end_marker)
        .ok_or_else(|| anyhow!("script block starting with {start_marker:?} was not terminated"))?;

    Ok(script[..end + end_marker.len() - 1].to_string())
}

pub(super) fn split_js_objects(script: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut string_delimiter: Option<char> = None;
    let mut prev = '\0';

    for ch in script.chars() {
        if let Some(delimiter) = string_delimiter {
            if depth > 0 {
                current.push(ch);
            }
            if ch == delimiter && prev != '\\' {
                string_delimiter = None;
            }
            prev = ch;
            continue;
        }

        if (ch == '\'' || ch == '"') && prev != '\\' {
            string_delimiter = Some(ch);
            if depth > 0 {
                current.push(ch);
            }
            prev = ch;
            continue;
        }

        if ch == '{' {
            depth += 1;
        }
        if depth > 0 {
            current.push(ch);
        }
        if ch == '}' && depth > 0 {
            depth -= 1;
            if depth == 0 {
                objects.push(current.clone());
                current.clear();
            }
        }

        prev = ch;
    }

    objects
}

fn extract_js_string_field(object: &str, field: &str) -> Option<String> {
    extract_js_string_field_raw(object, field).map(|value| normalize_whitespace(&value))
}

fn extract_js_string_field_raw(object: &str, field: &str) -> Option<String> {
    let needle = format!("{field}:");
    let start = object.find(&needle)?;
    let rest = object[start + needle.len()..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let quoted = &rest[quote.len_utf8()..];
    let end = find_unescaped_quote(quoted, quote)?;

    Some(decode_html_entities(&quoted[..end]).into_owned())
}

fn find_unescaped_quote(value: &str, quote: char) -> Option<usize> {
    let mut prev = '\0';

    for (index, ch) in value.char_indices() {
        if ch == quote && prev != '\\' {
            return Some(index);
        }
        prev = ch;
    }

    None
}

pub(super) fn mime_type_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();

    if path.ends_with(".png") {
        Some("image/png".to_string())
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        Some("image/jpeg".to_string())
    } else if path.ends_with(".webp") {
        Some("image/webp".to_string())
    } else if path.ends_with(".gif") {
        Some("image/gif".to_string())
    } else if path.ends_with(".svg") {
        Some("image/svg+xml".to_string())
    } else if path.ends_with(".bmp") {
        Some("image/bmp".to_string())
    } else if path.ends_with(".tif") || path.ends_with(".tiff") {
        Some("image/tiff".to_string())
    } else {
        None
    }
}
