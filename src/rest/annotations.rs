//! Dev Mode annotations and ready-for-dev status, read from the snapshot's own fields.
//! Not compared with same-version Figma exports.
use super::*;

/// Kiwi `AnnotationPropertyType` as REST/Plugin `AnnotationProperty.type`. Both enums list the
/// same 33 properties; unknown values pass through unchanged.
fn property(kiwi: &str) -> &str {
    match kiwi {
        "FILL" => "fills",
        "STROKE" => "strokes",
        "WIDTH" => "width",
        "HEIGHT" => "height",
        "MIN_WIDTH" => "minWidth",
        "MIN_HEIGHT" => "minHeight",
        "MAX_WIDTH" => "maxWidth",
        "MAX_HEIGHT" => "maxHeight",
        "STROKE_WIDTH" => "strokeWeight",
        "CORNER_RADIUS" => "cornerRadius",
        "EFFECT" => "effects",
        "TEXT_STYLE" => "textStyleId",
        "TEXT_ALIGN_HORIZONTAL" => "textAlignHorizontal",
        "FONT_FAMILY" => "fontFamily",
        "FONT_STYLE" => "fontStyle",
        "FONT_SIZE" => "fontSize",
        "FONT_WEIGHT" => "fontWeight",
        "LINE_HEIGHT" => "lineHeight",
        "LETTER_SPACING" => "letterSpacing",
        "STACK_SPACING" => "itemSpacing",
        "STACK_PADDING" => "padding",
        "STACK_MODE" => "layoutMode",
        "STACK_ALIGNMENT" => "alignItems",
        "OPACITY" => "opacity",
        "COMPONENT" => "mainComponent",
        "GRID_ROW_GAP" => "gridRowGap",
        "GRID_COLUMN_GAP" => "gridColumnGap",
        "GRID_ROW_COUNT" => "gridRowCount",
        "GRID_COLUMN_COUNT" => "gridColumnCount",
        "GRID_ROW_ANCHOR_INDEX" => "gridRowAnchorIndex",
        "GRID_COLUMN_ANCHOR_INDEX" => "gridColumnAnchorIndex",
        "GRID_ROW_SPAN" => "gridRowSpan",
        "GRID_COLUMN_SPAN" => "gridColumnSpan",
        other => other,
    }
}

/// REST `annotations` (`label`, `properties`) plus the Plugin API's `labelMarkdown` and
/// `categoryId`. An instance sublayer takes its main's annotations and any override (possibly an
/// empty list) through the expansion's field merge.
pub(super) fn node(k: &Obj, out: &mut Obj) {
    let list: Vec<Json> = at(k, "annotations")
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let mut entry = json!({});
            if let Some(html) = a["label"].as_str().filter(|l| !l.is_empty()) {
                let (plain, markdown) = label(html);
                entry["label"] = plain.into();
                entry["labelMarkdown"] = markdown.into();
            }
            if let Some(props) = a["properties"].as_array().filter(|p| !p.is_empty()) {
                entry["properties"] = props
                    .iter()
                    .filter_map(|p| p["type"].as_str())
                    .map(|t| json!({ "type": property(t) }))
                    .collect();
            }
            // `guid` would print the unset sentinel (u32::MAX twice) as an id.
            if let Some(id) = guid(&a["categoryId"]).filter(|id| id != "4294967295:4294967295") {
                entry["categoryId"] = id.into();
            }
            entry
        })
        .collect();
    if !list.is_empty() {
        out.insert("annotations".into(), list.into());
    }
}

/// REST `devStatus` from one page's `handoffStatusMap` entry; Figma stores Ready for dev as BUILD.
pub(super) fn dev_status(info: &Json) -> Option<Json> {
    let kind = match info["status"].as_str()? {
        "BUILD" => "READY_FOR_DEV",
        "COMPLETED" => "COMPLETED",
        _ => return None,
    };
    let mut status = json!({ "type": kind });
    if let Some(d) = info["description"].as_str().filter(|d| !d.is_empty()) {
        status["description"] = d.into();
    }
    Some(status)
}

/// The stored label is rich-text HTML (`<p>`, lists, links, emphasis). Returns it as plain text
/// (one line per paragraph or list item) and as Markdown. A label without tags is taken as text.
fn label(html: &str) -> (String, String) {
    if !html.contains('<') {
        return (html.to_owned(), html.to_owned());
    }
    let (mut plain, mut md) = (Text::default(), Text::default());
    // Open lists: ordered flag and the next item number.
    let mut lists: Vec<(bool, u32)> = Vec::new();
    let mut hrefs: Vec<Option<String>> = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(start) = rest.find('<') else {
            let text = entities(rest);
            plain.push(&text);
            md.push(&text);
            break;
        };
        let text = entities(&rest[..start]);
        plain.push(&text);
        md.push(&text);
        // A `<` that opens no tag is text.
        let Some(end) = rest[start..].find('>') else {
            plain.push(&rest[start..]);
            md.push(&rest[start..]);
            break;
        };
        let tag = &rest[start + 1..start + end];
        rest = &rest[start + end + 1..];
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let attr = |key: &str| {
            let at = tag.find(&format!("{key}=\""))? + key.len() + 2;
            Some(entities(&tag[at..at + tag[at..].find('"')?]))
        };
        match (name.as_str(), closing) {
            ("p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                plain.block(false);
                md.block(lists.is_empty());
                if let Some(level) = name.strip_prefix('h').and_then(|n| n.parse().ok()) {
                    md.push(&format!("{} ", "#".repeat(level)));
                }
            }
            ("br", _) => {
                plain.push("\n");
                md.push("  \n");
            }
            ("ol" | "ul", false) => lists.push((name == "ol", 1)),
            ("ol" | "ul", true) => {
                lists.pop();
            }
            ("li", false) => {
                plain.block(false);
                md.block(false);
                let depth = lists.len().saturating_sub(1);
                // Quill marks each item's kind; an `<ol>` can hold bullet items.
                let ordered = match attr("data-list").as_deref() {
                    Some("bullet") => false,
                    Some("ordered") => true,
                    _ => lists.last().is_some_and(|l| l.0),
                };
                let marker = match lists.last_mut() {
                    Some(list) if ordered => {
                        list.1 += 1;
                        format!("{}. ", list.1 - 1)
                    }
                    _ => "- ".to_owned(),
                };
                let item = format!("{}{marker}", "  ".repeat(depth));
                plain.push(&item);
                md.push(&item);
            }
            ("strong" | "b", _) => md.push("**"),
            ("em" | "i", _) => md.push("_"),
            ("s" | "strike" | "del", _) => md.push("~~"),
            ("code", _) => md.push("`"),
            ("a", false) => {
                hrefs.push(attr("href"));
                md.push("[");
            }
            ("a", true) => match hrefs.pop().flatten() {
                Some(href) => md.push(&format!("]({href})")),
                None => md.push("]"),
            },
            _ => {}
        }
    }
    (plain.finish(), md.finish())
}

/// Output that separates blocks lazily, so empty paragraphs leave no blank lines.
#[derive(Default)]
struct Text {
    out: String,
    /// The separator owed before the next text: a newline, or a blank line between paragraphs.
    pending: Option<&'static str>,
}

impl Text {
    fn block(&mut self, paragraph: bool) {
        if !self.out.is_empty() {
            self.pending = Some(if paragraph { "\n\n" } else { "\n" });
        }
    }

    fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(sep) = self.pending.take() {
            self.out = self.out.trim_end_matches(' ').to_owned();
            self.out.push_str(sep);
        }
        self.out.push_str(text);
    }

    fn finish(self) -> String {
        self.out.trim().to_owned()
    }
}

fn entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .map_or_else(
                        || name.strip_prefix('#').and_then(|n| n.parse().ok()),
                        |hex| u32::from_str_radix(hex, 16).ok(),
                    )
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quill_lists_become_lines_and_markdown_items() {
        let html = r#"<ol><li value="1" data-list="bullet">Swipe &amp; tap</li><li value="2" data-list="bullet">Hide if one</li></ol>"#;
        assert_eq!(
            label(html),
            (
                "- Swipe & tap\n- Hide if one".into(),
                "- Swipe & tap\n- Hide if one".into()
            )
        );
    }

    #[test]
    fn ordered_items_are_numbered_per_list() {
        let html = r#"<ol><li data-list="ordered">a</li><li data-list="ordered">b</li></ol>"#;
        assert_eq!(label(html).0, "1. a\n2. b");
    }

    #[test]
    fn paragraphs_links_and_emphasis_keep_markdown_only_markers() {
        let html = r#"<p>See <a href="https://example.com/x?a=1&amp;b=2" rel="noopener" target="_blank">spec</a></p><p><em>Note</em><br>two</p><p></p>"#;
        let (plain, md) = label(html);
        assert_eq!(plain, "See spec\nNote\ntwo");
        assert_eq!(
            md,
            "See [spec](https://example.com/x?a=1&b=2)\n\n_Note_  \ntwo"
        );
    }

    #[test]
    fn untagged_labels_are_text_and_a_stray_bracket_keeps_the_rest() {
        assert_eq!(label("Tom &amp; Jerry").0, "Tom &amp; Jerry");
        assert_eq!(label("<p>a</p> < b").0, "a < b");
    }

    #[test]
    fn annotations_map_properties_and_skip_unset_categories() {
        let k = object(json!({"annotations": [
            {"label": "<p>x</p>", "properties": [{"type": "FILL"}, {"type": "STACK_SPACING"}, {"type": "NEW"}],
             "categoryId": {"sessionID": 4294967295u64, "localID": 4294967295u64}},
            {"label": "", "properties": [], "categoryId": {"sessionID": 1, "localID": 2}},
        ]}));
        let mut out = Obj::new();
        node(&k, &mut out);
        assert_eq!(
            out["annotations"],
            json!([
                {"label": "x", "labelMarkdown": "x", "properties": [{"type": "fills"}, {"type": "itemSpacing"}, {"type": "NEW"}]},
                {"categoryId": "1:2"},
            ])
        );
    }

    #[test]
    fn empty_override_list_emits_nothing() {
        let mut out = Obj::new();
        node(&object(json!({"annotations": []})), &mut out);
        assert!(!out.contains_key("annotations"));
    }

    #[test]
    fn build_status_is_ready_for_dev_and_none_is_absent() {
        assert_eq!(
            dev_status(&json!({"status": "BUILD", "description": "v2"})),
            Some(json!({"type": "READY_FOR_DEV", "description": "v2"}))
        );
        assert_eq!(dev_status(&json!({"status": "NONE"})), None);
    }
}
