//! `gs://` inputs, fetched with the user's `gcloud` login and cached per object generation:
//! an unchanged object downloads once, a re-uploaded one is fetched again. Each download names
//! the generation it was looked up at (`URI#G`), so the cached bytes are that generation's.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// A local path for `input`: itself, or the cached copy of a `gs://` object.
pub fn local(input: &str) -> Result<PathBuf, Box<dyn Error>> {
    if !input.starts_with("gs://") {
        return Ok(PathBuf::from(input));
    }
    Ok(fetch(input)?.0)
}

/// The cached copy of the `gs://` object `input` and the generation its bytes were downloaded
/// at: the current one, looked up first, then fetched pinned to it.
pub fn fetch(input: &str) -> Result<(PathBuf, String), Box<dyn Error>> {
    let generation = gcloud(&[
        "storage",
        "objects",
        "describe",
        input,
        "--format=value(generation)",
    ])?;
    let generation = generation.trim().to_owned();
    if generation.is_empty() || !generation.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{input}: unexpected generation {generation:?} from gcloud").into());
    }
    let path = cache_path(&cache_root()?, input, &generation);
    if path.exists() {
        return Ok((path, generation));
    }
    let dir = path.parent().ok_or("cache path has no parent")?;
    std::fs::create_dir_all(dir)?;
    // Older generations of this object are stale once a newer one is fetched.
    for entry in std::fs::read_dir(dir)? {
        std::fs::remove_file(entry?.path())?;
    }
    let part = path.with_extension("part");
    // Pinned: an upload replacing the object meanwhile fails this copy instead of changing it.
    gcloud(&[
        "storage",
        "cp",
        &format!("{input}#{generation}"),
        part.to_str().ok_or("cache path is not UTF-8")?,
    ])?;
    std::fs::rename(&part, &path)?;
    Ok((path, generation))
}

/// `<root>/<sha256 of the URL>/<generation>.pinned.fig`; hashing keeps object names like `../x`
/// inside the cache. Only pinned downloads use the name: older unpinned `<generation>.fig`
/// copies are never reused as that generation.
fn cache_path(root: &Path, input: &str, generation: &str) -> PathBuf {
    let hash: String = Sha256::digest(input.as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect();
    root.join(hash).join(format!("{generation}.pinned.fig"))
}

pub(crate) fn cache_root() -> Result<PathBuf, Box<dyn Error>> {
    let base = match (
        std::env::var_os("XDG_CACHE_HOME"),
        std::env::var_os("LOCALAPPDATA"),
    ) {
        (Some(dir), _) => PathBuf::from(dir),
        // Windows has no HOME; its per-user cache lives under %LOCALAPPDATA%.
        (None, Some(dir)) if cfg!(windows) => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(".cache"),
    };
    Ok(base.join("figlens"))
}

/// Runs `gcloud` and returns its stdout; its stderr becomes the error.
fn gcloud(args: &[&str]) -> Result<String, Box<dyn Error>> {
    // On Windows the SDK installs a batch file, which Command does not find by its bare name.
    let program = if cfg!(windows) {
        "gcloud.cmd"
    } else {
        "gcloud"
    };
    let out = Command::new(program).args(args).output().map_err(|e| {
        format!("reading gs:// needs gcloud (https://cloud.google.com/sdk/docs/install): {e}")
    })?;
    if !out.status.success() {
        return Err(format!(
            "gcloud {}: {}",
            args[..2].join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_object_names_inside_the_cache() {
        let root = Path::new("/cache/figlens");
        let path = cache_path(root, "gs://b/../../etc/passwd", "7");
        assert_eq!(path.parent().unwrap().parent().unwrap(), root);
    }

    #[test]
    fn a_new_generation_is_a_new_file() {
        let root = Path::new("/c");
        assert_ne!(
            cache_path(root, "gs://b/x.fig", "1"),
            cache_path(root, "gs://b/x.fig", "2")
        );
    }
}
