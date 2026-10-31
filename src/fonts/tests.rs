use super::*;
use crate::test_support::TestDir;

/// A minimal sfnt: one `name` table with PostScript name, family and subfamily records.
pub(crate) fn sfnt(ps: &str, family: &str, sub: &str) -> Vec<u8> {
    let records = [(6u16, ps), (1, family), (2, sub)];
    let mut strings = vec![];
    let mut table = vec![
        0,
        0,
        0,
        records.len() as u8,
        0,
        6 + 12 * records.len() as u8,
    ];
    for (id, s) in records {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3u16, 1, 0x409, id, utf16.len() as u16, strings.len() as u16] {
            table.extend(v.to_be_bytes());
        }
        strings.extend(utf16);
    }
    table.extend(strings);
    let mut font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    font.extend(b"name\0\0\0\0\0\0\0\x1c");
    font.extend((table.len() as u32).to_be_bytes());
    font.extend(table);
    font
}

fn want<'a>(family: &'a str, style: &'a str, ps: &'a str) -> Want<'a> {
    Want {
        family,
        style,
        post_script_name: ps,
    }
}

fn find(dir: &TestDir, w: Want) -> (Vec<Hit>, Summary) {
    let (mut hits, summary) = lookup(&[(dir.join(""), true)], &[w]);
    (hits.remove(0), summary)
}

#[test]
fn a_font_is_matched_by_its_own_name_table_not_its_file_name() {
    let dir = TestDir::new();
    std::fs::write(dir.join("a.ttf"), sfnt("Inter-Bold", "Inter", "Bold")).unwrap();
    let by_ps = find(&dir, want("Nope", "Nope", "inter-bold")).0;
    let by_family = find(&dir, want("inter", "Bold", "Other-Name")).0;
    let none = find(&dir, want("Inter", "Light", "Inter-Light")).0;
    assert_eq!(
        (by_ps[0].by, by_family[0].by, none.len()),
        ("postScriptName", "family+style", 0)
    );
}

#[test]
fn woff2_is_matched_by_file_name_only_and_labelled_so() {
    let dir = TestDir::new();
    std::fs::write(dir.join("Open-Sans_Regular.woff2"), b"not a font").unwrap();
    let (hits, summary) = find(&dir, want("Open Sans", "Regular", "OpenSans-Regular"));
    assert_eq!((hits[0].by, summary.files_read), ("filename", 0));
}

#[test]
fn non_ascii_names_are_not_erased_when_matching_file_names() {
    let dir = TestDir::new();
    std::fs::write(dir.join("字体乙-Regular.woff2"), b"x").unwrap();
    assert_eq!(find(&dir, want("字体甲", "Regular", "")).0.len(), 0);
    std::fs::write(dir.join("字体甲-Regular.woff2"), b"x").unwrap();
    assert_eq!(find(&dir, want("字体甲", "Regular", "")).0.len(), 1);
}

#[test]
fn a_subtree_past_the_depth_limit_marks_the_lookup_truncated() {
    let dir = TestDir::new();
    let deep: String = (0..=MAX_DEPTH + 1).map(|i| format!("d{i}/")).collect();
    std::fs::create_dir_all(dir.join(&deep)).unwrap();
    assert!(find(&dir, want("X", "Regular", "X")).1.truncated);
}

#[test]
fn malformed_and_truncated_fonts_are_skipped_within_budget() {
    let dir = TestDir::new();
    let good = sfnt("X-Regular", "X", "Regular");
    std::fs::write(dir.join("truncated.ttf"), &good[..good.len() - 10]).unwrap();
    // Declares 65535 tables.
    let mut many = good.clone();
    many[4..6].copy_from_slice(&[0xff, 0xff]);
    std::fs::write(dir.join("many.ttf"), many).unwrap();
    // A name table of 4 GiB.
    let mut huge = good.clone();
    huge[24..28].copy_from_slice(&[0xff; 4]);
    std::fs::write(dir.join("huge.ttf"), huge).unwrap();
    // A record count the table cannot hold.
    let mut count = good;
    count[30..32].copy_from_slice(&[0xff, 0xff]);
    std::fs::write(dir.join("count.ttf"), count).unwrap();
    std::fs::write(
        dir.join("collection.ttc"),
        b"ttcf\0\x01\0\0\xff\xff\xff\xff",
    )
    .unwrap();
    let (hits, summary) = find(&dir, want("X", "Regular", "X-Regular"));
    assert_eq!(
        (hits.len(), summary.files_read, summary.truncated),
        (0, 5, false)
    );
}

#[test]
fn a_collection_matches_any_of_its_fonts() {
    let dir = TestDir::new();
    let (a, b) = (
        sfnt("A-Regular", "A", "Regular"),
        sfnt("B-Bold", "B", "Bold"),
    );
    let head = 12 + 8;
    let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
    ttc.extend((head as u32).to_be_bytes());
    ttc.extend(((head + a.len()) as u32).to_be_bytes());
    // Table offsets inside a collection are from the file start.
    for (font, base) in [(&a, head), (&b, head + a.len())] {
        let mut font = font.clone();
        let off = u32::from_be_bytes(font[20..24].try_into().unwrap()) as usize + base;
        font[20..24].copy_from_slice(&(off as u32).to_be_bytes());
        ttc.extend(font);
    }
    std::fs::write(dir.join("c.ttc"), ttc).unwrap();
    assert_eq!(
        find(&dir, want("B", "Bold", "B-Bold")).0[0].by,
        "postScriptName"
    );
}

#[test]
fn the_snippet_points_at_the_file_relative_to_the_project_folder() {
    let w = want("My \"Font\"", "Bold Italic", "x");
    let root = Path::new("fonts");
    let hit = |p: &str| Hit {
        path: root.join(p),
        by: "filename",
        project: true,
    };
    let css = font_face(&w, Some(700), &hit("sub dir/My.woff2"), root).unwrap();
    assert_eq!(
        css,
        "@font-face {\n  font-family: \"My \\\"Font\\\"\";\n  font-weight: 700;\n  font-style: italic;\n  src: url(\"sub%20dir/My.woff2\") format(\"woff2\");\n  font-display: swap;\n}"
    );
    assert!(font_face(&w, None, &hit("a.ttc"), root).is_none());
    // No recorded weight: no font-weight claim.
    assert!(
        !font_face(&w, None, &hit("a.otf"), root)
            .unwrap()
            .contains("font-weight")
    );
}

#[test]
fn many_wanted_faces_do_not_multiply_the_cost_of_every_file() {
    let dir = TestDir::new();
    for i in 0..3000 {
        std::fs::write(dir.join(&format!("File{i}-Regular.woff2")), b"x").unwrap();
    }
    let owned: Vec<_> = (0..20_000).map(|i| format!("Family{i}")).collect();
    let wants: Vec<_> = owned.iter().map(|f| want(f, "Regular", f)).collect();
    let start = std::time::Instant::now();
    let (hits, _) = lookup(&[(dir.join(""), true)], &wants);
    assert_eq!(
        (
            hits.iter().all(Vec::is_empty),
            start.elapsed().as_secs() < 2
        ),
        (true, true)
    );
}

#[test]
fn a_family_with_a_newline_stays_one_css_string() {
    let w = want("A\nB", "Regular", "x");
    let root = Path::new("f");
    let hit = Hit {
        path: root.join("a.ttf"),
        by: "filename",
        project: true,
    };
    assert!(
        font_face(&w, None, &hit, root)
            .unwrap()
            .contains("font-family: \"A\\A B\";")
    );
}

#[test]
#[cfg(unix)]
fn a_project_link_leading_outside_the_folder_is_not_read() {
    let (project, outside) = (TestDir::new(), TestDir::new());
    let font = sfnt("Secret-Regular", "Secret", "Regular");
    std::fs::write(outside.join("Secret.ttf"), &font).unwrap();
    std::fs::write(project.join("Own.ttf"), &font).unwrap();
    let outside_file = std::fs::canonicalize(outside.join("Secret.ttf")).unwrap();
    std::os::unix::fs::symlink(outside_file, project.join("Secret.ttf")).unwrap();
    std::os::unix::fs::symlink("Own.ttf", project.join("Alias.ttf")).unwrap();
    let (hits, summary) = find(&project, want("Secret", "Regular", "Secret-Regular"));
    let names: Vec<_> = hits
        .iter()
        .map(|h| h.path.file_name().unwrap().to_str().unwrap())
        .collect();
    // The link inside the folder is read; the one leading out is not.
    assert_eq!(
        (names, summary.files_read),
        (vec!["Alias.ttf", "Own.ttf"], 2)
    );
}
