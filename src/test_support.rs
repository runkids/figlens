use std::path::PathBuf;

pub(crate) struct TestDir(PathBuf);

impl TestDir {
    pub(crate) fn new() -> Self {
        std::fs::create_dir_all("samples").unwrap();
        let mut next = 0;
        loop {
            let path = PathBuf::from(format!("samples/test-{}-{next}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => next += 1,
                // Windows denies a name whose earlier directory is still being deleted.
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && next < 64 => {
                    next += 1
                }
                Err(e) => panic!("{}: {e}", path.display()),
            }
        }
    }

    pub(crate) fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub(crate) fn oversized_image(&self, image_name: &str) -> PathBuf {
        let mut source =
            zip::ZipArchive::new(std::fs::File::open("examples/first-success/design.fig").unwrap())
                .unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let name = format!("images/{image_name}");
        for i in 0..source.len() {
            let mut entry = source.by_index(i).unwrap();
            let original = entry.name().unwrap().into_owned();
            let target = if original.starts_with("images/") {
                name.as_str()
            } else {
                original.as_str()
            };
            zip.start_file(target, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::copy(&mut entry, &mut zip).unwrap();
        }
        let mut bytes = zip.finish().unwrap().into_inner();
        let header = bytes
            .windows(name.len())
            .rposition(|s| s == name.as_bytes())
            .unwrap()
            - 46;
        assert_eq!(&bytes[header..header + 4], b"PK\x01\x02");
        // Keep the payload tiny: the advertised size must be rejected before it is read.
        bytes[header + 24..header + 28].copy_from_slice(&(64 * 1024 * 1024 + 1u32).to_le_bytes());
        let file = self.join("oversized-image.fig");
        std::fs::write(&file, bytes).unwrap();
        file
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn directories_keep_their_own_files_and_cleanup() {
    let first = TestDir::new();
    let second = TestDir::new();
    assert_ne!(first.0, second.0);
    std::fs::write(first.join("owner"), "first").unwrap();
    std::fs::write(second.join("owner"), "second").unwrap();
    let second_path = second.0.clone();
    drop(second);
    assert!(!second_path.exists());
    assert_eq!(
        std::fs::read_to_string(first.join("owner")).unwrap(),
        "first"
    );
}

/// A canvas holding a component set (a state-group frame) with one component main (`SYMBOL`).
pub(crate) fn component_set() -> kiwi_schema::Value<'static> {
    use kiwi_schema::Value;
    use std::collections::HashMap;
    let guid = |l| {
        Value::Object(
            "GUID",
            HashMap::from([("sessionID", Value::UInt(0)), ("localID", Value::UInt(l))]),
        )
    };
    let node = |l: u32, parent: Option<u32>, kind, extra: Vec<(&'static str, Value<'static>)>| {
        let mut fields = HashMap::from([
            ("guid", guid(l)),
            ("type", Value::Enum("NodeType", kind)),
            ("name", Value::String(format!("n{l}"))),
        ]);
        if let Some(p) = parent {
            fields.insert(
                "parentIndex",
                Value::Object(
                    "ParentIndex",
                    HashMap::from([("guid", guid(p)), ("position", Value::String("a".into()))]),
                ),
            );
        }
        fields.extend(extra);
        Value::Object("NodeChange", fields)
    };
    Value::Object(
        "Message",
        HashMap::from([(
            "nodeChanges",
            Value::Array(vec![
                node(0, None, "CANVAS", vec![]),
                node(
                    1,
                    Some(0),
                    "FRAME",
                    vec![("isStateGroup", Value::Bool(true))],
                ),
                node(2, Some(1), "SYMBOL", vec![]),
            ]),
        )]),
    )
}
