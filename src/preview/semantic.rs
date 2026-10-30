//! The live viewer's optional semantic search (#94): read-only readiness, the explicit Enable
//! index action and the jobs it owns. Text search never comes here, so opening a design and
//! searching its text create no workspace and start no model.
use super::*;
use crate::workspace::{self, PrepareOptions, Select, jobs};
use std::sync::atomic::AtomicBool;

/// The one index build this viewer owns at a time: the scope the user enabled for a revision.
#[derive(Default)]
pub(super) struct Indexing(pub(super) Mutex<Option<Owned>>);

pub(super) struct Owned {
    scope: String,
    cancel: Arc<AtomicBool>,
    /// The latest job report, or the error that ended the run.
    state: Arc<Mutex<Value>>,
    thread: std::thread::JoinHandle<()>,
}

impl Drop for Indexing {
    /// However the viewer ends, the build it owns ends with it.
    fn drop(&mut self) {
        self.stop();
    }
}

impl Indexing {
    /// Cancels the owned run and waits until its jobs, and the workers they own, have ended.
    pub(super) fn stop(&self) {
        if let Some(owned) = self.0.lock().ok().and_then(|mut o| o.take()) {
            owned.cancel.store(true, Ordering::SeqCst);
            let _ = owned.thread.join();
        }
    }
}

/// The text build of `scope`, then, with a configured profile, its model layer: the jobs a CLI
/// prepare and index embed submit, run to their end by this process, which owns them.
fn build(file: &Path, scope: &str, cancel: &AtomicBool, state: &Mutex<Value>) -> Result<()> {
    let note = |report: &Value| {
        if let Ok(mut s) = state.lock() {
            *s = report.clone();
        }
    };
    let storage = workspace::storage(None)?;
    let options = PrepareOptions {
        scope: Some(scope),
        ..Default::default()
    };
    let prepared = workspace::prepare(&storage, file, None, options)?;
    note(&prepared.report);
    let ws = prepared.report["workspaceId"]
        .as_str()
        .ok_or("prepare named no workspace")?
        .to_owned();
    let mut runs = prepared.run;
    let mut embedding = false;
    loop {
        for job in &runs {
            let report = jobs::run(&storage, &ws, job, cancel, &mut |r| note(r))?;
            note(&report);
            if report["state"] != "succeeded" {
                return Ok(());
            }
        }
        if embedding
            || cancel.load(Ordering::SeqCst)
            || crate::semantic::profile::active(&storage)?.is_none()
        {
            return Ok(());
        }
        let embed = jobs::embed(&storage, &Select::Id(ws.clone()), None, Some(scope), None)?;
        note(&embed.report);
        (runs, embedding) = (embed.run, true);
    }
}

impl Revision<'_> {
    /// The scope a semantic route names for `revision`: a stored layer of this revision, never
    /// widened.
    fn indexed_scope(&self, revision: &str, scope: Option<&str>) -> Result<String> {
        if revision.parse::<u64>().map_err(|_| "unknown route")? != self.generation {
            return Err(STALE.into());
        }
        let scope = scope
            .filter(|s| valid_id(s))
            .ok_or("takes exactly one scope")?;
        if !self.tree.nodes.contains_key(scope) {
            return Err("scope takes a stored layer of this file".into());
        }
        Ok(scope.to_owned())
    }

    pub(super) fn select(&self) -> Result<Select> {
        Ok(
            Select::from_choice(None, Some(self.file), None)?
                .ok_or("no workspace for this file")?,
        )
    }

    /// A semantic or hybrid search of this file's workspace index, served only while that index
    /// holds exactly this revision's bytes; the reply then names them as `source`, as text's does.
    pub(super) fn indexed_search(
        &self,
        req: &search::Request,
        mode: crate::semantic::Mode,
    ) -> Result<Value> {
        let storage = workspace::storage(None)?;
        let mut out =
            workspace::query::search_mode(&storage, &self.select()?, &[], req, mode, &self.cancel)?;
        // The file at this path with exactly these bytes. A relocated workspace keeps the source
        // id it registered, so the id is translated to this viewer's, as its refs expect.
        let current = match out["sources"].as_array().map(Vec::as_slice) {
            Some([one]) => {
                one["state"] == "ready"
                    && one["path"] == self.source["path"]
                    && one["snapshotId"] == self.source["snapshotId"]
            }
            _ => false,
        };
        if !current {
            return Err(FigError::new(
                crate::ErrorKind::ModeUnavailable,
                "this revision has no ready index of the scope; enable it for semantic search",
            )
            .into());
        }
        for result in out["results"].as_array_mut().into_iter().flatten() {
            result["ref"]["sourceId"] = self.source["sourceId"].clone();
        }
        out["source"] = self.source.clone();
        Ok(out)
    }

    /// `semantic/<revision>?scope=ID`, read-only: the model profiles' readiness, this file's
    /// workspace (a missing one stays missing) and the viewer's own build of `scope`, if any.
    pub(super) fn semantic(&self, rest: &str) -> Result<Value> {
        let (revision, query) = rest.split_once('?').unwrap_or((rest, ""));
        let scope = match form(query)?.as_slice() {
            [(key, scope)] if key == "scope" => self.indexed_scope(revision, Some(scope))?,
            _ => self.indexed_scope(revision, None)?,
        };
        let storage = workspace::storage(None)?;
        let models = crate::semantic::profile::status(&storage)?;
        let index = workspace::status(&storage, &self.select()?)?;
        let owned = self.indexing.0.lock().map_err(|_| "indexing poisoned")?;
        let job = owned.as_ref().filter(|o| o.scope == scope).map(|o| {
            let report = o.state.lock().map_or(Value::Null, |s| s.clone());
            json!({"running": !o.thread.is_finished(), "report": report})
        });
        Ok(
            json!({"scope": scope, "models": {"profiles": models["profiles"], "setup": models["setup"]},
            "index": index, "job": job}),
        )
    }

    /// `POST enable/<revision>` with `{"scope": ID}`, the user's explicit Enable index: starts
    /// this viewer's build of `scope` (rejoining it while it runs). The only route that creates
    /// a workspace.
    pub(super) fn post(&self, route: &str, body: &[u8]) -> Result<(&'static str, Vec<u8>)> {
        let revision = route.strip_prefix("enable/").ok_or("unknown route")?;
        let reply = serde_json::from_slice::<serde_json::Map<String, Value>>(body)
            .map_err(|_| "takes a JSON object".into())
            .and_then(
                |o| match (o.len(), o.get("scope").and_then(Value::as_str)) {
                    (1, Some(scope)) => self.enable(revision, scope),
                    _ => Err("takes exactly one scope".into()),
                },
            )
            .unwrap_or_else(|e| refusal(&*e, &self.source));
        Ok(("application/json", reply.to_string().into_bytes()))
    }

    fn enable(&self, revision: &str, scope: &str) -> Result<Value> {
        let scope = self.indexed_scope(revision, Some(scope))?;
        let mut owned = self.indexing.0.lock().map_err(|_| "indexing poisoned")?;
        // Checked under the lock the revision's end takes to stop the build: a request that
        // outlived its revision never admits a build the next revision would inherit.
        if self.cancel.load(Ordering::SeqCst) {
            return Err(STALE.into());
        }
        if let Some(o) = owned.as_ref()
            && !o.thread.is_finished()
        {
            if o.scope == scope {
                return Ok(json!({"started": false, "scope": scope}));
            }
            return Err(FigError::new(
                crate::ErrorKind::WorkspaceBusy,
                format!(
                    "an index build of {} is running; retry when it ends",
                    o.scope
                ),
            )
            .into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(json!({"state": "starting"})));
        let (file, run_scope) = (self.file.to_owned(), scope.clone());
        let (stop, report) = (cancel.clone(), state.clone());
        let thread = std::thread::Builder::new()
            .name("preview-index".into())
            .spawn(move || {
                if let Err(e) = build(&file, &run_scope, &stop, &report)
                    && let Ok(mut s) = report.lock()
                {
                    *s = refusal(&*e, &Value::Null);
                }
            })?;
        *owned = Some(Owned {
            scope: scope.clone(),
            cancel,
            state,
            thread,
        });
        Ok(json!({"started": true, "scope": scope}))
    }
}
