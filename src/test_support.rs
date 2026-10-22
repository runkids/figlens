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
                Err(e) => panic!("{}: {e}", path.display()),
            }
        }
    }

    pub(crate) fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
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
