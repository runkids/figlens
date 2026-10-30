//! `models list|status|configure|remove`: the local profiles a user configures for semantic
//! (text) and visual (image) search. A profile names model files the user already has; figlens pins their
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
    /// `text` profiles embed design text; `image` profiles embed normalized rasters.
    pub modality: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

pub const KNOWN: &[Known] = &[
    Known {
        name: "bge-m3",
        model: "BAAI/bge-m3",
        revision: "5617a9f61b028005a4858fdac845db406aefb181",
        license: "MIT",
        modality: "text",
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
    },
    Known {
        name: "siglip-base",
        model: "google/siglip-base-patch16-224",
        revision: "7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed",
        license: "Apache-2.0",
        modality: "image",
        files: &[
            (
                "config.json",
                "cd85b3d28829722820bcb89a2cfbb4160e55fd359249a3044da724166a8d9688",
            ),
            (
                "preprocessor_config.json",
                "d11ccb80f15d358a11bdb070e92e2d889005874b7db15823d5f10d9b2533b14a",
            ),
            (
                "model.safetensors",
                "2c63cb7d1f2e95ba501893cbb8faeb4ea9a3af295498d35097126228659c2af8",
            ),
        ],
    },
];

fn fail(kind: ErrorKind, message: impl Into<String>) -> Box<dyn std::error::Error> {
    FigError::new(kind, message).into()
}

/// The documented profile called `name`.
pub fn known(name: &str) -> Result<&'static Known> {
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
    // Through links (macOS reports the invoked path), to the directory figlens really lives in.
    let exe = std::env::current_exe()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok(exe.with_file_name(format!("figlens-embed{}", std::env::consts::EXE_SUFFIX)))
}

fn records(storage: &Storage) -> Result<Vec<Json>> {
    match storage.models()? {
        Some(record) => parse(record),
        None => Ok(Vec::new()),
    }
}

fn parse(record: Json) -> Result<Vec<Json>> {
    if record["version"] != RECORD {
        return Err(fail(
            ErrorKind::IndexIncompatible,
            format!(
                "models.json has version {}; this figlens reads version {RECORD} and left it untouched",
                record["version"]
            ),
        ));
    }
    let list = |key: &str| record[key].as_array().cloned().unwrap_or_default();
    Ok([list("profiles"), list("visualProfiles")].concat())
}

/// A record's modality; one written before visual profiles existed has none and is text.
fn modality(r: &Json) -> &str {
    r["modality"].as_str().unwrap_or("text")
}

/// Replaces the stored profiles with `change` of the current ones, under the config lock.
fn save(storage: &Storage, change: impl FnOnce(Vec<Json>) -> Vec<Json>) -> Result<()> {
    storage.update_models(|record| {
        let profiles = match record {
            None => Vec::new(),
            Some(r) => parse(r)?,
        };
        // Image profiles live apart, so a figlens that predates them never reads one as its
        // text profile; it keeps "profiles" only.
        let (text, image): (Vec<Json>, Vec<Json>) = change(profiles)
            .into_iter()
            .partition(|r| modality(r) == "text");
        Ok(json!({"version": RECORD, "profiles": text, "visualProfiles": image}))
    })
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
                "modality": k.modality,
                "configured": record.is_some(),
                "profileId": record.map(|r| r["profileId"].clone()),
                "dataFlow": format!("local: {} go only to the configured local child; nothing is sent over a network",
                    if k.modality == "image" { "normalized images" } else { "design text" }),
            })
        })
        .collect();
    Ok(json!({"profiles": profiles}))
}

/// Readiness of each configured profile, checked without loading the model: its files and
/// runtime must still exist. A runtime or model file that changed is refused when the child next starts.
/// Also names the config root and the shipped runtime, even with no profile; nothing is written.
pub fn status(storage: &Storage) -> Result<Json> {
    let profiles: Vec<Json> = records(storage)?
        .into_iter()
        .map(|r| {
            let model = PathBuf::from(r["model"].as_str().unwrap_or_default());
            let runtime = Path::new(r["runtime"].as_str().unwrap_or_default()).to_path_buf();
            let files = KNOWN.iter().find(|k| r["name"] == k.name).map_or(&[][..], |k| k.files);
            let missing: Vec<&str> = files.iter().map(|f| f.0).filter(|f| !model.join(f).is_file()).collect();
            // The runtime is hashed again (an upgrade replaces it); model weights are not.
            let changed = runtime.is_file()
                && super::sha256_file(&runtime, &AtomicBool::new(false)).ok().as_ref()
                    != r["stamps"]["runtime"].as_str().map(str::to_owned).as_ref();
            let (state, code, reason) = if !runtime.is_file() {
                ("unavailable", Some("runtime_missing"), Some(format!("runtime {} is missing", runtime.display())))
            } else if changed {
                ("unavailable", Some("runtime_changed"), Some(format!("runtime {} changed since configure (an upgrade replaces it)", runtime.display())))
            } else if !missing.is_empty() {
                ("unavailable", Some("model_missing"), Some(format!("{} missing from {}", missing.join(", "), model.display())))
            } else {
                ("ready", None, None)
            };
            // Configuring again with the same files and the shipped runtime makes it usable;
            // its vectors stay apart under the new profile id.
            let recovery = code.map(|_| {
                json!([{"action": "configure", "tool": "models", "requires": "user_confirmation",
                    "arguments": {"action": "configure", "profile": r["name"], "modelDir": r["model"]},
                    "cli": format!("figlens models configure {} --model {}", r["name"].as_str().unwrap_or_default(), model.display()),
                    "note": "uses the figlens-embed shipped beside figlens; reinstall figlens with its paired runtime if that is missing"}])
            });
            json!({"name": r["name"], "modality": modality(&r), "profileId": r["profileId"], "model": r["model"],
                "runtime": r["runtime"], "dim": r["dim"], "state": state, "code": code, "reason": reason, "recovery": recovery})
        })
        .collect();
    // Setup for every modality no profile serves, so a missing text profile is named even when
    // an image one is configured.
    let next = KNOWN
        .iter()
        .filter(|k| !profiles.iter().any(|p| p["modality"] == k.modality))
        .map(setup)
        .collect::<Vec<_>>();
    // Where configure and remove write, and the runtime they start, for hosts asking permission first.
    let runtime = default_runtime()?;
    Ok(
        json!({"profiles": profiles, "setup": next, "configRoot": storage.config_root(),
        "defaultRuntime": {"path": runtime, "available": runtime.is_file()}}),
    )
}

/// Verifies `model` against `k`'s pinned digests, starts `runtime` once for its contract and
/// records the profile. Replaces an earlier record of the same profile.
pub fn configure(
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
                "no model runtime at {}; install figlens with the figlens-embed it ships beside it, then configure again",
                runtime.display()
            ),
        ));
    }
    // Named before anything is hashed, so the user knows which file to fetch where.
    let missing: Vec<&str> = k
        .files
        .iter()
        .map(|f| f.0)
        .filter(|f| !model.join(f).is_file())
        .collect();
    if !missing.is_empty() {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "{}: {} is missing {}; download {} revision {} ({}) into it, then configure again; nothing was recorded",
                name,
                model.display(),
                missing.join(", "),
                k.model,
                k.revision,
                k.license
            ),
        ));
    }
    let p = Profile::probe(&runtime, model, k.files, cancel).map_err(|e| {
        match e.downcast::<Stop>() {
            Ok(stop) if matches!(*stop, Stop::Cancelled) => fail(
                ErrorKind::Cancelled,
                "models configure cancelled; nothing was recorded",
            ),
            Ok(stop) => fail(ErrorKind::InvalidArguments, format!("{name}: {stop}")),
            Err(e) => fail(ErrorKind::InvalidArguments, format!("{name}: {e}")),
        }
    })?;
    let served = p.contract["modality"].as_str().unwrap_or("text");
    if served != k.modality {
        return Err(fail(
            ErrorKind::InvalidArguments,
            format!(
                "{name}: the runtime serves {served} models, the profile needs {}",
                k.modality
            ),
        ));
    }
    let record = json!({"name": k.name, "modality": k.modality, "profileId": p.id, "model": p.model,
        "runtime": p.exe, "dim": p.dim, "contract": p.contract, "stamps": p.stamps});
    let kept = record.clone();
    save(storage, |profiles| {
        let mut profiles: Vec<Json> = profiles
            .into_iter()
            .filter(|r| r["name"] != k.name)
            .collect();
        profiles.push(kept);
        profiles
    })?;
    Ok(json!({"configured": record, "dataFlow": "local only; no download, no network"}))
}

/// Forgets a profile; its model files are left where they are.
pub fn remove(storage: &Storage, name: &str) -> Result<Json> {
    let mut removed = false;
    if records(storage)?.iter().any(|r| r["name"] == name) {
        save(storage, |profiles| {
            let before = profiles.len();
            let kept: Vec<Json> = profiles.into_iter().filter(|r| r["name"] != name).collect();
            removed = kept.len() != before;
            kept
        })?;
    }
    Ok(json!({"name": name, "removed": removed, "modelFilesKept": true}))
}

/// The profile semantic search uses: the first configured text one.
pub fn active(storage: &Storage) -> Result<Option<Profile>> {
    first(storage, "text")
}

/// The profile visual search uses: the first configured image one.
pub fn active_visual(storage: &Storage) -> Result<Option<Profile>> {
    first(storage, "image")
}

/// Records written before visual profiles existed carry no modality: they are text.
fn first(storage: &Storage, modality: &str) -> Result<Option<Profile>> {
    let Some(r) = records(storage)?
        .into_iter()
        .find(|r| self::modality(r) == modality)
    else {
        return Ok(None);
    };
    Ok(Some(Profile {
        id: r["profileId"].as_str().unwrap_or_default().to_owned(),
        exe: PathBuf::from(r["runtime"].as_str().unwrap_or_default()),
        model: PathBuf::from(r["model"].as_str().unwrap_or_default()),
        dim: r["dim"].as_u64().unwrap_or(0) as usize,
        contract: r["contract"].clone(),
        stamps: r["stamps"].clone(),
    }))
}

/// One warm child per process, reused by every request with the same profile.
static WARM: Mutex<Option<(String, Runtime)>> = Mutex::new(None);

/// Runs `f` on the warm runtime of `profile`, starting it if needed; concurrent requests take
/// turns, each still cancellable while it waits. A dead child or another
/// profile's child is replaced; a child that declares another dimension is refused.
pub fn with_runtime<T>(
    profile: &Profile,
    cancel: &AtomicBool,
    f: impl FnOnce(&mut Runtime) -> std::result::Result<T, Stop>,
) -> std::result::Result<T, Stop> {
    // Another request holds the child; wait for it without ignoring this request's cancel.
    let mut warm = super::lock(&WARM, cancel)?;
    let reuse = match warm.as_mut() {
        Some((id, rt)) => *id == profile.id && rt.alive(),
        None => false,
    };
    if !reuse {
        *warm = None;
        profile
            .unchanged(cancel)
            .map_err(|e| match e.downcast::<Stop>() {
                Ok(stop) if matches!(*stop, Stop::Cancelled) => Stop::Cancelled,
                Ok(stop) => Stop::Failed(format!("{stop}; run models configure again")),
                Err(e) => Stop::Failed(format!("{e}; run models configure again")),
            })?;
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
