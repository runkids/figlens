//! `models list|status|configure|remove`: the local text profiles a user configures for
//! semantic search. A profile names model files the user already has; figlens pins their
//! digests, never downloads them, and keeps one record in `<config>/models.json`.

use super::{Profile, Result, Runtime, Stop};
use crate::workspace::Storage;
use crate::{ErrorKind, FigError};
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

const RECORD: u64 = 1;

/// A model figlens documents, with the files its child reads and their pinned SHA-256.
pub struct Known {
    pub name: &'static str,
    pub model: &'static str,
    pub revision: &'static str,
    pub license: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

pub const KNOWN: &[Known] = &[Known {
    name: "bge-m3",
    model: "BAAI/bge-m3",
    revision: "5617a9f61b028005a4858fdac845db406aefb181",
    license: "MIT",
    files: &[
        (
            "config.json",
            "26159e7ad065073448460117eb24b7a4572f6f4e78eadff65dc0a11c052449fa",
        ),
        (
            "pytorch_model.bin",
            "b5e0ce3470abf5ef3831aa1bd5553b486803e83251590ab7ff35a117cf6aad38",
        ),
        (
            "tokenizer.json",
            "21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08",
        ),
    ],
}];

fn fail(kind: ErrorKind, message: impl Into<String>) -> Box<dyn std::error::Error> {
    FigError::new(kind, message).into()
}

fn known(name: &str) -> Result<&'static Known> {
    KNOWN.iter().find(|k| k.name == name).ok_or_else(|| {
        let names: Vec<&str> = KNOWN.iter().map(|k| k.name).collect();
        fail(
            ErrorKind::InvalidArguments,
            format!("unknown profile {name}; known: {}", names.join(", ")),
        )
    })
}

/// The runtime figlens ships beside itself.
pub fn default_runtime() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(exe.with_file_name(format!("figlens-embed{}", std::env::consts::EXE_SUFFIX)))
}

fn records(storage: &Storage) -> Result<Vec<Json>> {
    let Some(record) = storage.models()? else {
        return Ok(Vec::new());
    };
    if record["version"] != RECORD {
        return Err(fail(
            ErrorKind::IndexIncompatible,
            format!(
                "models.json has version {}; this figlens reads version {RECORD} and left it untouched",
                record["version"]
            ),
        ));
    }
    Ok(record["profiles"].as_array().cloned().unwrap_or_default())
}

fn save(storage: &Storage, profiles: Vec<Json>) -> Result<()> {
    storage.set_models(&json!({"version": RECORD, "profiles": profiles}))
}

fn setup(k: &Known) -> Json {
    json!({
        "action": "configure",
        "requires": "user_confirmation",
        "steps": [
            format!("download {} at revision {} ({}) yourself; figlens never downloads models", k.model, k.revision, k.license),
            format!("figlens models configure {} --model <dir holding {}>", k.name,
                k.files.iter().map(|f| f.0).collect::<Vec<_>>().join(", ")),
        ],
    })
}

/// Every known profile and whether it is configured.
pub fn list(storage: &Storage) -> Result<Json> {
    let configured = records(storage)?;
    let profiles: Vec<Json> = KNOWN
        .iter()
        .map(|k| {
            let record = configured.iter().find(|r| r["name"] == k.name);
            json!({
                "name": k.name, "model": k.model, "revision": k.revision, "license": k.license,
                "files": k.files.iter().map(|f| f.0).collect::<Vec<_>>(),
                "configured": record.is_some(),
                "profileId": record.map(|r| r["profileId"].clone()),
                "dataFlow": "local: design text goes only to the configured local child; nothing is sent over a network",
            })
        })
        .collect();
    Ok(json!({"profiles": profiles}))
}

/// Readiness of each configured profile, checked without loading the model: its files and
/// runtime must still exist. A configured profile whose files changed is caught at use.
pub fn status(storage: &Storage) -> Result<Json> {
    let profiles: Vec<Json> = records(storage)?
        .into_iter()
        .map(|r| {
            let model = PathBuf::from(r["model"].as_str().unwrap_or_default());
            let runtime = Path::new(r["runtime"].as_str().unwrap_or_default()).to_path_buf();
            let files = KNOWN.iter().find(|k| r["name"] == k.name).map_or(&[][..], |k| k.files);
            let missing: Vec<&str> = files.iter().map(|f| f.0).filter(|f| !model.join(f).is_file()).collect();
            let (state, reason) = if !runtime.is_file() {
                ("unavailable", Some(format!("runtime {} is missing", runtime.display())))
            } else if !missing.is_empty() {
                ("unavailable", Some(format!("{} missing from {}", missing.join(", "), model.display())))
            } else {
                ("ready", None)
            };
            json!({"name": r["name"], "profileId": r["profileId"], "model": r["model"], "runtime": r["runtime"],
                "dim": r["dim"], "state": state, "reason": reason})
        })
        .collect();
    let next = if profiles.is_empty() {
        KNOWN.iter().map(setup).collect()
    } else {
        Vec::new()
    };
    Ok(json!({"profiles": profiles, "setup": next}))
}

/// Verifies `model` against the pinned digests, starts `runtime` once for its contract and
/// records the profile. Replaces an earlier record of the same profile.
pub fn configure(
    storage: &Storage,
    name: &str,
    model: &Path,
    runtime: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<Json> {
    configure_known(storage, known(name)?, model, runtime, cancel)
}

fn configure_known(
    storage: &Storage,
    k: &Known,
    model: &Path,
    runtime: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<Json> {
    let name = k.name;
    let runtime = match runtime {
        Some(r) => r.to_path_buf(),
        None => default_runtime()?,
    };
    if !runtime.is_file() {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "no model runtime at {}; install the figlens-embed that ships with figlens, or pass --runtime",
                runtime.display()
            ),
        ));
    }
    let p = Profile::probe(&runtime, model, k.files, cancel)
        .map_err(|e| fail(ErrorKind::InvalidArguments, format!("{name}: {e}")))?;
    let record = json!({"name": k.name, "profileId": p.id, "model": p.model, "runtime": p.exe,
        "dim": p.dim, "contract": p.contract});
    let mut profiles: Vec<Json> = records(storage)?
        .into_iter()
        .filter(|r| r["name"] != k.name)
        .collect();
    profiles.push(record.clone());
    save(storage, profiles)?;
    Ok(json!({"configured": record, "dataFlow": "local only; no download, no network"}))
}

/// Forgets a profile; its model files are left where they are.
pub fn remove(storage: &Storage, name: &str) -> Result<Json> {
    let all = records(storage)?;
    let kept: Vec<Json> = all.iter().filter(|r| r["name"] != name).cloned().collect();
    let removed = kept.len() != all.len();
    if removed {
        save(storage, kept)?;
    }
    Ok(json!({"name": name, "removed": removed, "modelFilesKept": true}))
}

/// The profile semantic search uses: the first configured one.
pub fn active(storage: &Storage) -> Result<Option<Profile>> {
    let Some(r) = records(storage)?.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(Profile {
        id: r["profileId"].as_str().unwrap_or_default().to_owned(),
        exe: PathBuf::from(r["runtime"].as_str().unwrap_or_default()),
        model: PathBuf::from(r["model"].as_str().unwrap_or_default()),
        dim: r["dim"].as_u64().unwrap_or(0) as usize,
        contract: r["contract"].clone(),
    }))
}

/// One warm child per process, reused by every request with the same profile.
static WARM: Mutex<Option<(String, Runtime)>> = Mutex::new(None);

/// Runs `f` on the warm runtime of `profile`, starting it if needed. A dead child or another
/// profile's child is replaced; a child that declares another dimension is refused.
pub fn with_runtime<T>(
    profile: &Profile,
    cancel: &AtomicBool,
    f: impl FnOnce(&mut Runtime) -> std::result::Result<T, Stop>,
) -> std::result::Result<T, Stop> {
    let mut warm = WARM.lock().unwrap_or_else(|e| e.into_inner());
    let reuse = match warm.as_mut() {
        Some((id, rt)) => *id == profile.id && rt.alive(),
        None => false,
    };
    if !reuse {
        *warm = None;
        let rt = Runtime::start(&profile.exe, &[&profile.model], cancel).map_err(|e| match e
            .downcast::<Stop>()
        {
            Ok(stop) => *stop,
            Err(e) => Stop::Failed(e.to_string()),
        })?;
        if rt.dim != profile.dim || rt.hello != profile.contract {
            return Err(Stop::Failed(
                "the runtime's contract changed since configure; run models configure again".into(),
            ));
        }
        *warm = Some((profile.id.clone(), rt));
    }
    let (_, rt) = warm.as_mut().expect("started above");
    f(rt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use sha2::{Digest, Sha256};

    fn fixture() -> (TestDir, Storage, PathBuf) {
        let dir = TestDir::new();
        let storage =
            crate::workspace::test_storage(&std::path::absolute(dir.join("home")).unwrap());
        let model = dir.join("model");
        std::fs::create_dir(&model).unwrap();
        std::fs::write(model.join("w"), "weights").unwrap();
        (dir, storage, model)
    }

    #[test]
    fn configure_records_a_verified_profile_and_remove_forgets_it() {
        let (_dir, storage, model) = fixture();
        let digest = crate::search::hex(&Sha256::digest("weights"));
        let files: &'static [(&str, &str)] = Box::leak(Box::new([(
            "w",
            Box::leak(digest.into_boxed_str()) as &str,
        )]));
        let k = Known {
            name: "fake",
            model: "fake",
            revision: "0",
            license: "MIT",
            files,
        };
        assert!(
            status(&storage).unwrap()["setup"]
                .as_array()
                .is_some_and(|s| !s.is_empty())
        );
        let got = configure_known(
            &storage,
            &k,
            &model,
            Some(&super::super::tests::fake()),
            &AtomicBool::new(false),
        )
        .unwrap();
        let current = active(&storage).unwrap().unwrap();
        assert_eq!(
            (got["configured"]["profileId"].as_str(), current.dim),
            (Some(current.id.as_str()), 8)
        );
        assert_eq!(status(&storage).unwrap()["profiles"][0]["state"], "ready");
        assert_eq!(remove(&storage, "fake").unwrap()["removed"], true);
        assert!(active(&storage).unwrap().is_none());
    }

    #[test]
    fn configure_refuses_files_that_are_not_the_pinned_model() {
        let (_dir, storage, model) = fixture();
        let fake = super::super::tests::fake();
        let err = configure(
            &storage,
            "bge-m3",
            &model,
            Some(&fake),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(
            err.downcast_ref::<FigError>().map(FigError::kind),
            Some(ErrorKind::InvalidArguments)
        );
        assert!(storage.models().unwrap().is_none());
    }

    #[test]
    fn warm_runtime_is_reused_across_requests() {
        let (_dir, _storage, model) = fixture();
        let digest = crate::search::hex(&Sha256::digest("weights"));
        let p = Profile::probe(
            &super::super::tests::fake(),
            &model,
            &[("w", &digest)],
            &AtomicBool::new(false),
        )
        .unwrap();
        let no = AtomicBool::new(false);
        let pid = |rt: &mut Runtime| Ok(rt.pid());
        let first = with_runtime(&p, &no, pid).unwrap();
        assert_eq!(with_runtime(&p, &no, pid).unwrap(), first);
    }
}
