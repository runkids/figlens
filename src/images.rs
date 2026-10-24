//! Original bitmap evidence for image paints; no export or quality diagnosis.

use std::error::Error;
use std::path::Path;

use serde_json::{Value, json};

const LIMIT: usize = 200;

/// `document` is an expanded render node, including its local `_r.size`.
pub fn inspect(fig: &Path, document: &Value) -> Result<Value, Box<dyn Error>> {
    let mut items = Vec::new();
    let mut truncated = false;
    collect(document, &mut items, &mut truncated);
    let originals = crate::render::images(fig, &json!(items))?;
    for item in &mut items {
        let bytes = originals.get(item["imageRef"].as_str().unwrap());
        item["source"] = match bytes {
            None => json!({"status": "missing"}),
            Some(bytes) => match imagesize::blob_size(bytes) {
                Ok(size) if size.width > 0 && size.height > 0 => json!({
                    "status": "available", "width": size.width, "height": size.height,
                    "bytes": bytes.len(),
                }),
                _ => json!({"status": "unrecognized", "bytes": bytes.len()}),
            },
        };
        let (rect, reason) = source_rect(item);
        item["sourceRect"] = rect;
        if let Some(reason) = reason {
            item["sourceRectUnknownReason"] = reason.into();
        }
    }
    Ok(json!({
        "items": items, "truncated": truncated,
        "visualAssessment": "not performed",
        "note": "sourceRect is a rectangle in the original image's pixel coordinates, before masks, effects and ancestor clipping; not rendered-image dimensions or a blur diagnosis. FIT uses the whole source with letterboxing inside nodeSize.",
    }))
}

fn collect(node: &Value, items: &mut Vec<Value>, truncated: &mut bool) {
    for (paint, raw) in [("fills", "fillPaints"), ("strokes", "strokePaints")] {
        collect_paints(
            node,
            paint,
            &node[paint],
            &node["_r"][raw],
            None,
            items,
            truncated,
        );
        if *truncated {
            return;
        }
    }
    for (style_id, style) in node["styleOverrideTable"].as_object().into_iter().flatten() {
        let raw = node["_r"]["styleOverrideTable"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["styleID"]
                    .as_u64()
                    .is_some_and(|id| id.to_string() == style_id.as_str())
            })
            .unwrap_or(&Value::Null);
        collect_paints(
            node,
            "fills",
            &style["fills"],
            &raw["fillPaints"],
            Some(style_id),
            items,
            truncated,
        );
        if *truncated {
            return;
        }
    }
    for child in node["children"].as_array().into_iter().flatten() {
        collect(child, items, truncated);
        if *truncated {
            return;
        }
    }
}

fn collect_paints(
    node: &Value,
    paint: &str,
    paints: &Value,
    raw_paints: &Value,
    text_style: Option<&str>,
    items: &mut Vec<Value>,
    truncated: &mut bool,
) {
    for (index, fill) in paints.as_array().into_iter().flatten().enumerate() {
        let Some(image_ref) = fill["imageRef"].as_str() else {
            continue;
        };
        if items.len() == LIMIT {
            *truncated = true;
            return;
        }
        let mut item = json!({
            "nodeId": node["id"], "paint": paint, "index": index,
            "imageRef": image_ref, "scaleMode": fill["scaleMode"],
            "nodeSize": {"width": node["_r"]["size"]["x"], "height": node["_r"]["size"]["y"]},
        });
        if let Some(style) = text_style {
            item["textStyleId"] = style.into();
        }
        if let Some(rotation) = raw_paints[index]["rotation"].as_f64().filter(|r| *r != 0.0) {
            item["imageRotation"] = rotation.into();
        }
        for (key, value) in [
            ("nodeVisible", &node["visible"]),
            ("paintVisible", &fill["visible"]),
            ("imageTransform", &fill["imageTransform"]),
        ] {
            if !value.is_null() {
                item[key] = value.clone();
            }
        }
        items.push(item);
    }
}

fn source_rect(item: &Value) -> (Value, Option<&'static str>) {
    let unknown = |reason| (Value::Null, Some(reason));
    let Some(iw) = item["source"]["width"].as_f64() else {
        return unknown("source dimensions unavailable");
    };
    let ih = item["source"]["height"].as_f64().unwrap();
    if item["scaleMode"] != "STRETCH"
        && item["imageRotation"]
            .as_f64()
            .is_some_and(|r| r.rem_euclid(360.0) != 0.0)
    {
        return unknown("rotated image paint");
    }
    let rect = |x, y, w, h| {
        (
            json!({"coordinateSpace":"source-image-pixels", "x":x, "y":y, "width": w, "height": h}),
            None,
        )
    };
    match item["scaleMode"].as_str() {
        Some("FIT") => rect(0.0, 0.0, iw, ih),
        Some("FILL") => {
            let (Some(w), Some(h)) = (
                item["nodeSize"]["width"].as_f64(),
                item["nodeSize"]["height"].as_f64(),
            ) else {
                return unknown("local node size unavailable");
            };
            if w <= 0.0 || h <= 0.0 {
                return unknown("empty node size");
            }
            let scale = (w / iw).max(h / ih);
            let (width, height) = (w / scale, h / scale);
            rect((iw - width) / 2.0, (ih - height) / 2.0, width, height)
        }
        Some("STRETCH") => {
            let matrix = &item["imageTransform"];
            let values: Option<Vec<f64>> = (0..2)
                .flat_map(|r| (0..3).map(move |c| (r, c)))
                .map(|(r, c)| matrix[r][c].as_f64())
                .collect();
            let Some(v) = values else {
                return unknown("image transform unavailable");
            };
            let [a, c, x, b, d, y] = v[..] else {
                unreachable!()
            };
            if b != 0.0 || c != 0.0 || a == 0.0 || d == 0.0 {
                return unknown("rotated, skewed or singular image transform");
            }
            if x.min(x + a) < -1e-6
                || x.max(x + a) > 1.0 + 1e-6
                || y.min(y + d) < -1e-6
                || y.max(y + d) > 1.0 + 1e-6
            {
                return unknown("image transform extends outside the source");
            }
            rect(
                x.min(x + a) * iw,
                y.min(y + d) * ih,
                a.abs() * iw,
                d.abs() * ih,
            )
        }
        _ => unknown("unsupported scale mode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn shared_source_keeps_each_usage_crop_and_reports_missing_or_unrecognized_bytes() {
        let dir = crate::test_support::TestDir::new();
        let file = dir.join("images.fig");
        let png = crate::render::png(r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200"><rect width="400" height="200" fill="red"/></svg>"#).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        for (name, bytes) in [("shared", png.as_slice()), ("bad", b"unknown".as_slice())] {
            zip.start_file(
                format!("images/{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let node = |id, image, a, d| json!({"id":id,"_r":{"size":{"x":40,"y":40}},"fills":[{"imageRef":image,"scaleMode":"STRETCH","imageTransform":[[a,0,0],[0,d,0]]}]});
        let rotated = json!({"id":"5","_r":{"size":{"x":100,"y":100},"fillPaints":[{"rotation":90}]},"fills":[{"imageRef":"shared","scaleMode":"FILL"}]});
        let result = inspect(&file, &json!({"children":[node("I1;1","shared",0.25,0.5),node("I1;2","shared",0.5,0.25),node("3","missing",1.0,1.0),node("4","bad",1.0,1.0),rotated]})).unwrap();
        let items = &result["items"];
        assert_eq!(
            items[0]["source"],
            json!({"status":"available","width":400,"height":200,"bytes":png.len()})
        );
        assert_eq!(
            items[0]["sourceRect"],
            json!({"coordinateSpace":"source-image-pixels","x":0.0,"y":0.0,"width":100.0,"height":100.0})
        );
        assert_eq!(
            items[1]["sourceRect"],
            json!({"coordinateSpace":"source-image-pixels","x":0.0,"y":0.0,"width":200.0,"height":50.0})
        );
        assert_eq!(items[2]["source"]["status"], "missing");
        assert_eq!(items[3]["source"]["status"], "unrecognized");
        assert!(items[2]["sourceRect"].is_null());
        assert_eq!(items[4]["imageRotation"].as_f64(), Some(90.0));
        assert!(items[4]["sourceRect"].is_null());
        assert_eq!(items[4]["sourceRectUnknownReason"], "rotated image paint");
    }

    #[test]
    fn text_override_images_keep_style_identity_crop_and_raw_rotation() {
        let dir = crate::test_support::TestDir::new();
        let file = dir.join("text-images.fig");
        let png = crate::render::png(r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200"><rect width="400" height="200" fill="red"/></svg>"#).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        zip.start_file("images/shared", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&png).unwrap();
        zip.finish().unwrap();
        let document = json!({
            "id":"text", "type":"TEXT", "characters":"ABC", "characterStyleOverrides":[1,2,1],
            "fills":[{"type":"SOLID"}],
            "strokes":[{"type":"IMAGE","imageRef":"shared","scaleMode":"FIT"}],
            "styleOverrideTable":{
                "1":{"fills":[{"type":"IMAGE","imageRef":"shared","scaleMode":"FILL"}]},
                "2":{"fills":[{"type":"IMAGE","imageRef":"shared","scaleMode":"STRETCH","imageTransform":[[0.25,0,0.25],[0,0.5,0]]}]}
            },
            "_r":{"size":{"x":100,"y":100},"styleOverrideTable":[
                {"styleID":2,"fillPaints":[{"rotation":0}]},
                {"styleID":1,"fillPaints":[{"rotation":90}]}
            ]}
        });
        let mut text_only = document.clone();
        text_only["strokes"] = json!([]);
        let evidence = inspect(&file, &text_only).unwrap();
        assert_eq!(evidence["items"].as_array().unwrap().len(), 2);
        assert_eq!(evidence["truncated"], false);
        let result = inspect(&file, &document).unwrap();
        let items = result["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["paint"], "strokes");
        assert!(items[0].get("textStyleId").is_none());
        let rotated = items.iter().find(|i| i["textStyleId"] == "1").unwrap();
        assert_eq!(rotated["nodeId"], "text");
        assert_eq!(rotated["paint"], "fills");
        assert_eq!(rotated["index"], 0);
        assert_eq!(rotated["source"]["bytes"], png.len());
        assert_eq!(rotated["imageRotation"].as_f64(), Some(90.0));
        assert_eq!(rotated["sourceRectUnknownReason"], "rotated image paint");
        let cropped = items.iter().find(|i| i["textStyleId"] == "2").unwrap();
        assert_eq!(
            cropped["sourceRect"],
            json!({"coordinateSpace":"source-image-pixels","x":100.0,"y":0.0,"width":100.0,"height":100.0})
        );
        assert_eq!(result["truncated"], false);

        let fills: Vec<_> = (0..=LIMIT).map(|_| json!({"imageRef":"shared"})).collect();
        let mut items = Vec::new();
        let mut truncated = false;
        collect(
            &json!({"styleOverrideTable":{"9":{"fills":fills}}}),
            &mut items,
            &mut truncated,
        );
        assert_eq!(items.len(), LIMIT);
        assert!(truncated);
    }

    #[test]
    fn footprint_handles_fill_fit_flip_and_leaves_unsupported_transforms_unknown() {
        let mut item = json!({"source":{"width":400,"height":200},"nodeSize":{"width":100,"height":100},"scaleMode":"FILL"});
        assert_eq!(
            source_rect(&item).0,
            json!({"coordinateSpace":"source-image-pixels","x":100.0,"y":0.0,"width":200.0,"height":200.0})
        );
        item["scaleMode"] = "FIT".into();
        assert_eq!(
            source_rect(&item).0,
            json!({"coordinateSpace":"source-image-pixels","x":0.0,"y":0.0,"width":400.0,"height":200.0})
        );
        item["imageRotation"] = 90.into();
        assert!(source_rect(&item).0.is_null());
        item["scaleMode"] = "FILL".into();
        assert!(source_rect(&item).0.is_null());
        item["imageRotation"] = 0.into();
        item["scaleMode"] = "STRETCH".into();
        item["imageTransform"] = json!([[-0.25, 0, 0.5], [0, 0.5, 0]]);
        assert_eq!(
            source_rect(&item).0,
            json!({"coordinateSpace":"source-image-pixels","x":100.0,"y":0.0,"width":100.0,"height":100.0})
        );
        for matrix in [
            json!([[1, 0, 0.1], [0, 1, 0]]),
            json!([[0.5, 0.1, 0], [0, 0.5, 0]]),
            json!([[0, 0, 0], [0, 0.5, 0]]),
        ] {
            item["imageTransform"] = matrix;
            assert!(source_rect(&item).0.is_null());
        }
    }

    #[test]
    fn no_images_needs_no_archive_and_usage_limit_is_explicit() {
        assert_eq!(
            inspect(Path::new("absent.fig"), &json!({"id":"vector"})).unwrap()["items"],
            json!([])
        );
        let children: Vec<_> = (0..=LIMIT)
            .map(|i| json!({"id":i,"fills":[{"imageRef":"a"}]}))
            .collect();
        let mut items = Vec::new();
        let mut truncated = false;
        collect(&json!({"children":children}), &mut items, &mut truncated);
        assert_eq!(items.len(), LIMIT);
        assert!(truncated);
    }
}
