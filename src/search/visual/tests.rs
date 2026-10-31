use super::*;

fn solid(width: u32, height: u32, color: Color) -> Pixmap {
    let mut p = Pixmap::new(width, height).unwrap();
    p.fill(color);
    p
}

#[test]
fn source_scale_does_not_change_an_identical_normalized_image() {
    let a = solid(8, 4, Color::from_rgba8(255, 0, 0, 255));
    let b = solid(16, 8, Color::from_rgba8(255, 0, 0, 255));
    let a = QueryImage::from_png(&a.encode_png().unwrap(), None).unwrap();
    let b = QueryImage::from_png(&b.encode_png().unwrap(), None).unwrap();
    assert_eq!(a.fingerprint, b.fingerprint);
}

#[test]
fn flat_colors_are_distinguished_even_when_the_gradient_is_empty() {
    let a = QueryImage::from_pixmap(&solid(8, 8, Color::from_rgba8(255, 0, 0, 255)), None).unwrap();
    let b = QueryImage::from_pixmap(&solid(8, 8, Color::from_rgba8(0, 0, 255, 255)), None).unwrap();
    let d = a.fingerprint.distance(&b.fingerprint);
    assert_eq!(d.hamming, 0);
    assert_eq!(d.color, 510);
    assert_ne!(a.fingerprint.content, b.fingerprint.content);
}

#[test]
fn transparent_pixels_are_composited_on_the_declared_white_background() {
    let a = QueryImage::from_pixmap(&solid(8, 8, Color::TRANSPARENT), None).unwrap();
    let b = QueryImage::from_pixmap(&solid(8, 8, Color::WHITE), None).unwrap();
    assert_eq!(a.fingerprint, b.fingerprint);
    assert!(a.pixmap.pixels().iter().all(|p| p.alpha() == 255));
}

#[test]
fn a_crop_has_no_color_bleed_from_the_excluded_region() {
    let mut source = solid(4, 2, Color::from_rgba8(0, 0, 255, 255));
    for y in 0..2 {
        for x in 0..2 {
            source.data_mut()[(y * 4 + x) * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
        }
    }
    let cropped = QueryImage::from_pixmap(
        &source,
        Some(Crop {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        }),
    )
    .unwrap();
    let red =
        QueryImage::from_pixmap(&solid(2, 2, Color::from_rgba8(255, 0, 0, 255)), None).unwrap();
    assert_eq!(cropped.fingerprint, red.fingerprint);
}

#[test]
fn aspect_ratio_survives_even_for_a_blank_image() {
    let a = QueryImage::from_pixmap(&solid(8, 4, Color::WHITE), None).unwrap();
    let b = QueryImage::from_pixmap(&solid(4, 8, Color::WHITE), None).unwrap();
    assert_ne!(a.fingerprint.content, b.fingerprint.content);
    assert!(a.fingerprint.distance(&b.fingerprint).aspect > 0);
}

#[test]
fn crop_refuses_empty_outside_and_overflowed_rectangles() {
    let source = solid(8, 8, Color::WHITE);
    for crop in [
        Crop {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        Crop {
            x: 7,
            y: 0,
            width: 2,
            height: 1,
        },
        Crop {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
    ] {
        assert!(QueryImage::from_pixmap(&source, Some(crop)).is_err());
    }
}

#[test]
fn admission_rejects_encoded_and_declared_pixel_budgets_before_decode() {
    assert!(
        QueryImage::from_png(&vec![0; MAX_BYTES + 1], None)
            .err()
            .unwrap()
            .to_string()
            .contains("8 MiB")
    );
    let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    header.extend_from_slice(&4_000_001u32.to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    assert!(
        QueryImage::from_png(&header, None)
            .err()
            .unwrap()
            .to_string()
            .contains("4000000")
    );
    assert!(QueryImage::from_png(b"<svg/>", None).is_err());
    assert!(QueryImage::from_png(b"\x89PNG\r\n\x1a\n", None).is_err());
}

#[test]
fn normalized_png_roundtrips_without_retaining_source_container_bytes() {
    let source = solid(8, 8, Color::from_rgba8(20, 40, 60, 255));
    let mut encoded = source.encode_png().unwrap();
    encoded.extend_from_slice(b"source-only trailing data");
    let query = QueryImage::from_png(&encoded, None).unwrap();
    let normalized = query.png().unwrap();
    assert!(!normalized.windows(10).any(|s| s == b"source-only"));
    let decoded = Pixmap::decode_png(&normalized).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (SIZE, SIZE));
    let expected = QueryImage::from_pixmap(&source, None).unwrap();
    assert_eq!(query.fingerprint, expected.fingerprint);
}

#[test]
fn gradients_distinguish_reversed_shapes_with_the_same_average_color() {
    let mut a = solid(32, 32, Color::WHITE);
    let mut b = a.clone();
    for y in 0..32 {
        for x in 0..32 {
            if x < 16 {
                a.data_mut()[(y * 32 + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 255]);
            } else {
                b.data_mut()[(y * 32 + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
    let a = QueryImage::from_pixmap(&a, None).unwrap();
    let b = QueryImage::from_pixmap(&b, None).unwrap();
    let d = a.fingerprint.distance(&b.fingerprint);
    assert_eq!(d.color, 0);
    assert_eq!(d.aspect, 0);
    assert!(d.hamming > 0);
    assert_eq!(d, b.fingerprint.distance(&a.fingerprint));
    assert!(!a.fingerprint.same_pixels(&b.fingerprint));
}

#[test]
fn cache_blob_roundtrips_and_refuses_unsafe_or_stale_dimensions() {
    let image = QueryImage::from_pixmap(&solid(9, 6, Color::WHITE), None).unwrap();
    let bytes = image.fingerprint.encode();
    assert_eq!(Fingerprint::decode(&bytes).unwrap(), image.fingerprint);
    for len in [0, 4, 62] {
        assert!(Fingerprint::decode(&bytes[..len]).is_err());
    }
    let mut invalid = bytes;
    invalid[0] = b'X';
    assert!(Fingerprint::decode(&invalid).is_err());
    let mut invalid = bytes;
    invalid[55..59].fill(0);
    assert!(Fingerprint::decode(&invalid).is_err());
    invalid[55..63].fill(255);
    assert!(Fingerprint::decode(&invalid).is_err());
}

#[test]
fn units_use_screen_roots_and_components_without_enumerating_decorations() {
    use crate::search::tests::{bounds, node, obj};
    use kiwi_schema::Value as K;
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "SECTION", "Section", vec![]),
            node(3, Some(2), "FRAME", "Screen", vec![]),
            node(4, Some(3), "RECTANGLE", "Decoration", vec![]),
            node(5, Some(3), "COMPONENT", "Nested component", vec![]),
            node(
                6,
                Some(2),
                "FRAME",
                "Hidden",
                vec![("visible", K::Bool(false))],
            ),
            node(
                7,
                Some(0),
                "CANVAS",
                "Internal",
                vec![("internalOnly", K::Bool(true))],
            ),
            node(8, Some(7), "FRAME", "Internal screen", vec![]),
        ]),
    )]);
    let tree = crate::Tree::new(&message);
    let index = super::super::build(&tree, "0:0", bounds(super::super::WORK));
    let found = units(&index, &["0:4".into(), "0:3".into()], &mut bounds(1000)).unwrap();
    assert!(found.complete);
    assert_eq!(
        found.units.iter().map(|u| u.id).collect::<Vec<_>>(),
        ["0:3", "0:4", "0:5", "0:6", "0:8"]
    );
    assert!(found.units[3].hidden);
    assert!(found.units[4].internal);
    assert!(
        found
            .units
            .iter()
            .all(|u| index.records[u.rid as usize].id == u.id)
    );
    assert!(units(&index, &["0:99".into()], &mut bounds(1000)).is_err());
    assert!(units(&index, &["0:1".into()], &mut bounds(1000)).is_err());
    let found = units(&index, &[], &mut bounds(30)).unwrap();
    assert!(!found.complete);
    assert_eq!(found.reason, Some("work_limit"));
    assert!(found.units.is_empty());
    let scoped = super::super::build(&tree, "0:3", bounds(super::super::WORK));
    assert_eq!(
        units(&scoped, &[], &mut bounds(1000))
            .unwrap()
            .units
            .iter()
            .map(|u| u.id)
            .collect::<Vec<_>>(),
        ["0:3", "0:5"]
    );
}

#[test]
fn nearest_is_bounded_stable_and_keeps_distances_separate_from_identity() {
    use crate::search::tests::bounds;
    let q = QueryImage::from_pixmap(&solid(16, 16, Color::from_rgba8(255, 0, 0, 255)), None)
        .unwrap()
        .fingerprint;
    let near = QueryImage::from_pixmap(&solid(16, 16, Color::from_rgba8(254, 1, 0, 255)), None)
        .unwrap()
        .fingerprint;
    let far = QueryImage::from_pixmap(&solid(16, 16, Color::from_rgba8(0, 0, 255, 255)), None)
        .unwrap()
        .fingerprint;
    let candidates = [
        (7, far),
        (4, near.clone()),
        (2, q.clone()),
        (3, q.clone()),
        (1, near),
    ];
    let cancel = AtomicBool::new(false);
    let found = nearest(&q, &candidates, 4, &mut bounds(100), &cancel).unwrap();
    assert!(found.complete);
    assert_eq!(
        found.hits.iter().map(|h| h.rid).collect::<Vec<_>>(),
        [2, 3, 1, 4]
    );
    assert!(found.hits[0].exact);
    assert!(!found.hits[2].exact);
    let partial = nearest(&q, &candidates, 4, &mut bounds(2), &cancel).unwrap();
    assert!(!partial.complete);
    assert_eq!(partial.reason, Some("work_limit"));
    assert_eq!(partial.hits.len(), 2);
    cancel.store(true, Ordering::Relaxed);
    let err = nearest(&q, &candidates, 4, &mut bounds(100), &cancel)
        .err()
        .unwrap();
    assert_eq!(
        err.downcast_ref::<crate::FigError>().unwrap().kind(),
        crate::ErrorKind::Cancelled
    );
    assert!(nearest(&q, &candidates, 0, &mut bounds(100), &cancel).is_err());
}

#[test]
fn unit_selection_accepts_a_fully_loaded_stored_index_but_not_a_partial_query() {
    use crate::search::tests::{bounds, node, obj};
    use kiwi_schema::Value as K;
    let message = obj(vec![(
        "nodeChanges",
        K::Array(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some(0), "CANVAS", "Page", vec![]),
            node(2, Some(1), "FRAME", "Screen", vec![]),
        ]),
    )]);
    let tree = crate::Tree::new(&message);
    let mut index = super::super::build(&tree, "0:0", bounds(super::super::WORK));
    index.rids = (0..index.records.len() as u32).collect();
    assert_eq!(
        units(&index, &[], &mut bounds(100)).unwrap().units[0].id,
        "0:2"
    );
    index.records.pop();
    index.rids.pop();
    assert!(units(&index, &[], &mut bounds(100)).is_err());
}

#[test]
fn snapshot_units_reuse_export_pixels_and_keep_render_evidence() {
    let source = std::path::Path::new("examples/first-success/design.fig");
    let bytes = std::fs::read(source).unwrap();
    let document = crate::Document::from_bytes(&bytes, "visual snapshot").unwrap();
    let value = document.message().unwrap();
    let tree = crate::Tree::new(&value);
    // The documented public screen includes an original bitmap and outlined text.
    let id = "0:1";
    let rendered = render_unit(&tree, &bytes, id).unwrap();
    let (image, report) = crate::export::export(
        &tree,
        source,
        id,
        crate::export::Format::Png,
        false,
        1.0,
        100,
        Some(MAX_BYTES),
        Some(1024),
    )
    .unwrap();
    let expected = QueryImage::from_png(&image.bytes, None).unwrap();
    assert_eq!(rendered.image.fingerprint, expected.fingerprint);
    assert_eq!(rendered.image.png().unwrap(), expected.png().unwrap());
    assert_eq!(rendered.bounds, report);
    assert!(!rendered.diagnostics_truncated);
}

#[test]
fn snapshot_render_keeps_original_image_budget() {
    let dir = crate::test_support::TestDir::new();
    let source = dir.oversized_image("first-success-tile");
    let bytes = std::fs::read(source).unwrap();
    let document = crate::Document::from_bytes(&bytes, "oversized paint").unwrap();
    let value = document.message().unwrap();
    let tree = crate::Tree::new(&value);
    let error = render_unit(&tree, &bytes, "0:1").err().unwrap();
    assert!(error.to_string().contains("image bytes"), "{error}");
}

#[test]
fn render_evidence_reports_missing_paints_and_caps_diagnostics() {
    let node = serde_json::json!({"id": "0:3", "fills": [{"imageRef": "missing"}],
    "_figlens": {"renderDiagnostics": [
        {"feature": "effect", "state": "skipped", "reason": "not drawn"}
    ]}});
    let mut items = Vec::new();
    let mut truncated = false;
    collect_diagnostics(&node, &Default::default(), &mut items, &mut truncated);
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["state"], "missing");
    assert!(!truncated);
    let many = serde_json::json!({"children": vec![node; crate::search::MAX_LIMIT]});
    items.clear();
    collect_diagnostics(&many, &Default::default(), &mut items, &mut truncated);
    assert_eq!(items.len(), crate::search::MAX_LIMIT);
    assert!(truncated);
}

#[test]
fn image_only_validation_keeps_filters_budgets_and_text_validation_separate() {
    let mut request = crate::search::Request {
        query: String::new(),
        scope: "0:0".into(),
        types: vec![],
        include_hidden: false,
        include_internal: false,
        limit: 20,
        cursor: None,
    };
    assert!(request.validate().is_err());
    assert!(request.validate_image().is_ok());
    request.limit = 0;
    assert!(request.validate_image().is_err());
    request.limit = 20;
    request.query = "   ".into();
    assert!(request.validate_image().is_err());
    request.query = "付款".into();
    assert_eq!(
        request.validate_image().unwrap().terms(),
        request.validate().unwrap().terms()
    );
    request.cursor = Some("bad".into());
    assert!(request.validate_image().is_err());
}

fn gradient() -> QueryImage {
    let mut p = Pixmap::new(40, 30).unwrap();
    for (i, px) in p.data_mut().chunks_exact_mut(4).enumerate() {
        px.copy_from_slice(&[(i * 7) as u8, (i * 3) as u8, (i * 11) as u8, 255]);
    }
    QueryImage::from_pixmap(&p, None).unwrap()
}

#[test]
fn a_normalized_image_round_trips_through_rgb_exactly() {
    let rgb = gradient().rgb();
    assert_eq!(rgb.len(), RGB_BYTES);
    // The fingerprint also covers the source aspect, which the raster alone does not carry.
    assert_eq!(QueryImage::from_rgb(&rgb).unwrap().rgb(), rgb);
    assert!(QueryImage::from_rgb(&rgb[1..]).is_err());
}

/// The vision child embeds exactly the model input the parent's raster gives, alone or batched.
#[cfg(feature = "embed")]
#[test]
fn the_child_raster_gives_the_parents_model_input() {
    let blank = || QueryImage::from_pixmap(&Pixmap::new(9, 3).unwrap(), None).unwrap();
    let wire = |i: &QueryImage| QueryImage::from_rgb(&i.rgb()).unwrap();
    let parent = model::pixels(&[gradient(), blank()]).unwrap();
    assert_eq!(
        model::pixels(&[wire(&gradient()), wire(&blank())]).unwrap(),
        parent
    );
    let singles = [
        model::pixels(&[wire(&gradient())]).unwrap(),
        model::pixels(&[wire(&blank())]).unwrap(),
    ]
    .concat();
    assert_eq!(singles, parent);
}
