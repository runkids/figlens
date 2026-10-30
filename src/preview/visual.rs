//! Body-carrying image search for the loaded revision. Browser requests never name a path.
use super::*;
use crate::workspace::{
    self,
    visual_search::{ImageQuery, Input},
};

fn params(rest: &str, generation: u64) -> Result<Vec<(String, String)>> {
    let (revision, query) = rest.split_once('?').unwrap_or((rest, ""));
    if revision.parse::<u64>().map_err(|_| "unknown route")? != generation {
        return Err(STALE.into());
    }
    let fields = form(query)?;
    let known = [
        "q",
        "scope",
        "type",
        "hidden",
        "internal",
        "limit",
        "cursor",
        "mode",
        "crop",
        "requestId",
    ];
    for (i, (key, _)) in fields.iter().enumerate() {
        if !known.contains(&key.as_str()) {
            return Err(format!("unknown parameter {key}").into());
        }
        if key != "type" && fields[..i].iter().any(|(k, _)| k == key) {
            return Err(format!("duplicate parameter {key}").into());
        }
    }
    Ok(fields)
}

fn one<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn request(fields: &[(String, String)]) -> Result<search::Request> {
    let flag = |key| match one(fields, key) {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        _ => Err(format!("{key} takes 0 or 1")),
    };
    let scope = one(fields, "scope").unwrap_or("0:0").to_owned();
    if !valid_id(&scope) {
        return Err("scope takes a stored node ID".into());
    }
    let req = search::Request {
        query: one(fields, "q").unwrap_or_default().to_owned(),
        scope,
        types: fields
            .iter()
            .filter(|(k, _)| k == "type")
            .map(|(_, v)| v.clone())
            .collect(),
        include_hidden: flag("hidden")?,
        include_internal: flag("internal")?,
        limit: one(fields, "limit")
            .map(str::parse)
            .transpose()?
            .unwrap_or(search::DEFAULT_LIMIT),
        cursor: one(fields, "cursor").map(str::to_owned),
    };
    req.validate_image()?;
    Ok(req)
}

impl Revision<'_> {
    /// The selected stored layer of this file, under the workspace's retained source identity.
    fn visual_reference(&self, node: &str) -> Result<Input> {
        if !valid_id(node) || !self.tree.nodes.contains_key(node) {
            return Err("select a stored layer of this revision".into());
        }
        let storage = workspace::storage(None)?;
        let source = self.indexed_source(&storage)?;
        Ok(Input::Ref {
            source_id: source,
            snapshot_id: self.source["snapshotId"]
                .as_str()
                .ok_or("no loaded snapshot")?
                .to_owned(),
            node_id: node.to_owned(),
        })
    }

    pub(super) fn visual_post(
        &self,
        route: &str,
        body: Vec<u8>,
    ) -> Result<(&'static str, Vec<u8>)> {
        let result = (|| -> Result<Value> {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(STALE.into());
            }
            let (rest, reference) = if let Some(rest) = route.strip_prefix("image/") {
                (rest, false)
            } else {
                (
                    route.strip_prefix("image-ref/").ok_or("unknown route")?,
                    true,
                )
            };
            let fields = params(rest, self.generation)?;
            let req = request(&fields)?;
            let crop = one(&fields, "crop")
                .map(workspace::visual_search::crop)
                .transpose()?;
            let input = if reference {
                let object: serde_json::Map<String, Value> = serde_json::from_slice(&body)?;
                let (1, Some(node)) = (object.len(), object.get("nodeId").and_then(Value::as_str))
                else {
                    return Err("takes exactly one nodeId of this revision".into());
                };
                self.visual_reference(node)?
            } else {
                Input::Png(body)
            };
            let image = ImageQuery::new(input, crop, one(&fields, "mode"), &req.query)?;
            let storage = workspace::storage(None)?;
            let call = self
                .queries
                .begin(one(&fields, "requestId"), &self.cancel)?;
            let source = self.indexed_source(&storage)?;
            let out = workspace::query::search_image(
                &storage,
                &self.select()?,
                &[source],
                &req,
                &image,
                &call.cancel,
            )?;
            self.indexed_reply(out)
        })()
        .unwrap_or_else(|e| refusal(&*e, &self.source));
        Ok(("application/json", result.to_string().into_bytes()))
    }

    /// The exact cached source raster the selected-reference crop controls address.
    pub(super) fn visual_thumbnail(&self, rest: &str) -> Result<(&'static str, Vec<u8>)> {
        let result = (|| -> Result<Vec<u8>> {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(STALE.into());
            }
            let (revision, tail) = rest.split_once('/').ok_or("unknown route")?;
            let (node, query) = tail.split_once('?').unwrap_or((tail, ""));
            let fields = params(&format!("{revision}?{query}"), self.generation)?;
            if fields.iter().any(|(k, _)| k != "scope") {
                return Err("thumbnail takes only scope".into());
            }
            let scope = one(&fields, "scope").unwrap_or("0:0");
            if !valid_id(scope) {
                return Err("scope takes a stored node ID".into());
            }
            let reference = self.visual_reference(node)?;
            let storage = workspace::storage(None)?;
            workspace::query::reference_image(
                &storage,
                &self.select()?,
                scope,
                &reference,
                &self.cancel,
            )?
            .png()
        })();
        match result {
            Ok(png) => Ok(("image/png", png)),
            Err(e) => Ok((
                "application/json",
                refusal(&*e, &self.source).to_string().into_bytes(),
            )),
        }
    }
}
