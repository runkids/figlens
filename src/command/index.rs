//! Index action semantics shared by the CLI and MCP adapters. Parsing, path admission and
//! foreground/background job execution remain with the adapter.

use super::*;
use figlens::workspace::{self, Outcome, PrepareOptions, Select, Storage};

#[derive(Default)]
pub struct Request<'a> {
    pub action: &'a str,
    pub select: Option<Select>,
    pub file: Option<&'a Path>,
    pub project: Option<&'a Path>,
    pub source: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub request: Option<&'a str>,
    pub replace: bool,
    pub cursor: Option<&'a str>,
    pub manifest: Option<&'a str>,
    pub dry_run: bool,
}

pub enum Reply {
    Report(Value),
    Jobs(Outcome),
}

impl Request<'_> {
    /// Reject action-inapplicable options before any workspace operation can write.
    pub fn applicable(&self) -> bool {
        (self.scope.is_none()
            || matches!(
                self.action,
                "prepare" | "refresh" | "rebuild" | "track" | "untrack" | "embed" | "visual"
            ))
            && (self.request.is_none()
                || matches!(
                    self.action,
                    "prepare" | "refresh" | "rebuild" | "track" | "embed" | "visual"
                ))
            && (!self.replace || self.action == "prepare")
            && (!(self.cursor.is_some() || self.manifest.is_some() || self.dry_run)
                || self.action == "add")
            && (self.source.is_none()
                || matches!(
                    self.action,
                    "remove"
                        | "relocate"
                        | "refresh"
                        | "rebuild"
                        | "track"
                        | "untrack"
                        | "embed"
                        | "visual"
                ))
    }

    pub fn run(&self, storage: &Storage) -> Result<Reply, Box<dyn Error>> {
        let missing = |what: &str| -> Box<dyn Error> {
            figlens::FigError::new(
                figlens::ErrorKind::InvalidArguments,
                format!("index {} needs {what}", self.action),
            )
            .into()
        };
        if !self.applicable() {
            return Err(figlens::FigError::new(
                figlens::ErrorKind::InvalidArguments,
                format!(
                    "an argument given does not apply to index {}; see the tool description",
                    self.action
                ),
            )
            .into());
        }
        let chosen = || {
            self.select
                .as_ref()
                .ok_or_else(|| missing("workspace, workspaceId or file"))
        };
        let file = || self.file.ok_or_else(|| missing("file"));
        let source = || self.source.ok_or_else(|| missing("sourceId"));
        let scope = || self.scope.ok_or_else(|| missing("scope"));
        let report = match self.action {
            "prepare" => {
                return Ok(Reply::Jobs(workspace::prepare(
                    storage,
                    file()?,
                    self.project,
                    PrepareOptions {
                        scope: self.scope,
                        request_id: self.request,
                        replace_unavailable: self.replace,
                    },
                )?));
            }
            "refresh" => {
                return Ok(Reply::Jobs(workspace::refresh(
                    storage,
                    chosen()?,
                    self.source,
                    self.scope,
                    self.request,
                )?));
            }
            "rebuild" => {
                return Ok(Reply::Jobs(workspace::jobs::rebuild_scope(
                    storage,
                    chosen()?,
                    self.source,
                    self.scope,
                    self.request,
                )?));
            }
            "embed" => {
                return Ok(Reply::Jobs(workspace::jobs::embed(
                    storage,
                    chosen()?,
                    self.source,
                    self.scope,
                    self.request,
                )?));
            }
            "visual" => {
                return Ok(Reply::Jobs(workspace::jobs::visual(
                    storage,
                    chosen()?,
                    self.source,
                    self.scope,
                    self.request,
                )?));
            }
            "track" => {
                let scope = scope()?;
                return Ok(Reply::Jobs(workspace::jobs::track(
                    storage,
                    chosen()?,
                    self.source,
                    scope,
                    self.request,
                )?));
            }
            "untrack" => {
                let scope = scope()?;
                workspace::jobs::untrack(storage, chosen()?, self.source, scope)?
            }
            "status" => workspace::status(storage, chosen()?)?,
            "add" if self.dry_run => workspace::list_add(storage, chosen()?, file()?, self.cursor)?,
            "add" if self.manifest.is_some() || self.cursor.is_some() => {
                workspace::add_dir(storage, chosen()?, file()?, self.manifest, self.cursor)?
            }
            "add" => add(storage, chosen()?, file()?)?,
            "remove" => workspace::remove(storage, chosen()?, source()?)?,
            "relocate" => workspace::relocate(storage, chosen()?, source()?, file()?)?,
            "clear_cache" => workspace::clear_cache(storage, chosen()?)?,
            "reset" => workspace::reset(storage, chosen()?)?,
            _ => return Err(missing("a supported action")),
        };
        Ok(Reply::Report(report))
    }
}

/// A remote add pins the downloaded object generation before registration.
fn add(storage: &Storage, select: &Select, file: &Path) -> Result<Value, Box<dyn Error>> {
    match file.to_str().filter(|f| f.starts_with("gs://")) {
        Some(uri) => {
            let (local, generation) =
                figlens::remote::fetch(uri, true).map_err(|e| crate::contract::source(uri, e))?;
            workspace::add_remote(storage, select, uri, &local, &generation)
        }
        None => workspace::add(storage, select, file),
    }
}

#[cfg(test)]
mod tests;
