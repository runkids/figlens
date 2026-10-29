//! Read-only local viewer and self-contained snapshots. No filesystem routes are exposed.
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, SystemTime};

use crate::{
    Document, FigError, Tree,
    render::{self, Prepared},
    search,
};
use serde_json::{Value, json};
use std::hash::{BuildHasher, Hasher};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self> {
        for n in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "figlens-preview-{}-{}-{n}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .as_nanos()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err("cannot create preview workspace".into())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The viewer's layer index from `0:0`, or the layer-depth error before recursing into it.
fn checked_catalog(tree: &Tree) -> Result<Value> {
    crate::expand::check_depth("0:0", |id| tree.children.get(id))?;
    Ok(catalog(tree, "0:0"))
}

fn catalog(tree: &Tree, id: &str) -> Value {
    let n = tree
        .nodes
        .get(id)
        .map(|n| crate::to_json(n))
        .unwrap_or_default();
    // Page-level layers (and those inside sections) carry their absolute box for the page canvas.
    let placed = matches!(n["type"].as_str(), Some("CANVAS" | "SECTION"));
    let children = tree.children.get(id).into_iter().flatten().map(|c| {
        let mut child = catalog(tree, c);
        if placed && let Some(node) = tree.nodes.get(c) {
            let size = node.get("size");
            let f = |k: &str| {
                size.and_then(|s| s.get(k))
                    .map_or(0.0, |v| v.as_float() as f64)
            };
            let s = crate::expand::Vec2 {
                x: f("x"),
                y: f("y"),
            };
            let m = crate::expand::abs_matrix(tree, c, "");
            let (o, b) = (m.origin(s), m.bbox(s));
            child["box"] = json!([o.x, o.y, b.x, b.y]);
        }
        child
    });
    let kind = tree.nodes.get(id).copied().map(crate::expand::plugin_type);
    let mut out = json!({"id":id,"name":n["name"],"type":kind,"text":n["textData"]["characters"],
        "children":children.collect::<Vec<_>>()});
    // Figma hides its internal canvas (library components, styles, variables) from the page list.
    if n["internalOnly"] == true {
        out["internal"] = json!(true);
    }
    out
}

/// Share the exact source/override evidence emitted by the effective subtree resolver.
fn interactions(n: &Value, out: &mut serde_json::Map<String, Value>) {
    if let Some(id) = n["id"].as_str()
        && n["_figlens"]["prototype"].is_object()
    {
        out.insert(id.to_owned(), n["_figlens"]["prototype"].clone());
    }
    for child in n["children"].as_array().into_iter().flatten() {
        interactions(child, out);
    }
}

/// Picking data for layers whose bounding box is not their shape: `[a, b, c, d, tx, ty, width,
/// height, ellipse]`, the layer's absolute matrix and size, for rotated or skewed layers and ellipses.
/// Axis-aligned boxes need nothing, which keeps the payload small.
/// Exact outlines for vector-like layers, within a byte budget so huge icons sets stay light;
/// layers past the budget keep box or matrix picking.
fn pick_paths(
    tree: &Tree,
    n: &Value,
    out: &mut serde_json::Map<String, Value>,
    budget: &mut usize,
) {
    if let Some(id) = n["id"].as_str() {
        let paths = render::hit_paths(tree, n);
        let size: usize = paths.iter().map(|(d, _)| d.len()).sum();
        if !paths.is_empty() && size <= 20_000 && size <= *budget {
            *budget -= size;
            out.insert(id.to_owned(), json!(paths));
        }
    }
    for child in n["children"].as_array().into_iter().flatten() {
        pick_paths(tree, child, out, budget);
    }
}

fn pick_geometry(n: &Value, out: &mut serde_json::Map<String, Value>) {
    let r = &n["_r"];
    let m: Vec<f64> = r["m"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    if let (Some(id), [a, b, c, d, tx, ty], Some(w), Some(h)) = (
        n["id"].as_str(),
        m.as_slice(),
        r["size"]["x"].as_f64(),
        r["size"]["y"].as_f64(),
    ) {
        let ellipse = n["type"] == "ELLIPSE";
        if ellipse || b.abs() > 1e-9 || c.abs() > 1e-9 {
            out.insert(
                id.to_owned(),
                json!([a, b, c, d, tx, ty, w, h, u8::from(ellipse)]),
            );
        }
    }
    for child in n["children"].as_array().into_iter().flatten() {
        pick_geometry(child, out);
    }
}

fn data(tree: &Tree, file: &Path, id: &str, image: bool) -> Result<Value> {
    if !tree.nodes.contains_key(crate::export::root_of(id)) {
        return Err("node not found".into());
    }
    let p = Prepared::new(tree, file, id)?;
    let area = render::export_area(p.node()).ok_or("node has no export area")?;
    let (mut geometry, mut paths) = (serde_json::Map::new(), serde_json::Map::new());
    let mut prototype = serde_json::Map::new();
    interactions(p.node(), &mut prototype);
    pick_geometry(p.node(), &mut geometry);
    pick_paths(tree, p.node(), &mut paths, &mut (512 << 10));
    let mut doc = p.document.clone();
    doc["document"] = p.node().clone();
    crate::bundle::strip(&mut doc);
    let assets = crate::bundle::plan(&doc["document"]).images.into_iter().map(|(key, node)| {
        match p.images.get(&key) {
            // Live payloads fetch originals through the asset route; snapshots must embed them.
            Some(bytes) if image => json!({"key":key,"nodeId":node,"url":format!("data:{};base64,{}",render::media_type(bytes),render::base64(bytes))}),
            Some(_) => json!({"key":key,"nodeId":node}),
            None => json!({"key":key,"nodeId":node,"error":"Original image missing from .fig"}),
        }
    }).collect::<Vec<_>>();
    let png = if image {
        Some(render::png(
            &p.export_svg(tree, 1.0).ok_or("nothing to render")?,
        )?)
    } else {
        None
    };
    Ok(
        json!({"nodes":doc,"area":area,"geometry":geometry,"paths":paths,"interactions":prototype,"variables":crate::variables::variables(tree,Some(p.node())),"assets":assets,
        "image":png.map(|b|format!("data:image/png;base64,{}",render::base64(&b)))}),
    )
}

fn html(value: &Value) -> String {
    // '<' must not terminate the inert script element, including mixed-case closing tags.
    let json = value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    include_str!("preview/viewer.html")
        .replace("/*STYLE*/", include_str!("preview/viewer.css"))
        .replace("/*SCRIPT*/", include_str!("preview/viewer.js"))
        .replacen("/*DATA*/", &json, 1)
}

pub fn snapshot(file: &Path, id: &str, out: &Path) -> Result<()> {
    if !out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("html"))
    {
        return Err("--out must end in .html".into());
    }
    if out.exists() && fs::canonicalize(file)? == fs::canonicalize(out)? {
        return Err("snapshot output must not overwrite the source design".into());
    }
    let mut doc = Document::open(file)?;
    let message = doc.take_message()?;
    let tree = Tree::new(&message);
    let workspace = Workspace::new()?;
    let zip = workspace.0.join("scope.zip");
    let warnings = crate::bundle::write(&tree, file, None, None, id, &zip)?;
    let mut payload = data(&tree, file, id, true)?;
    payload["bundle"] = json!(render::base64(&fs::read(zip)?));
    payload["warnings"] = json!(warnings);
    crate::output::write(file, out, |file| {
        file.write_all(html(&json!({"snapshot":payload})).as_bytes())?;
        Ok(())
    })?;
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 1024
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || b":;I".contains(&b))
}

/// `<format>-<scale>-<quality>` of an export route, limited to the Export panel's choices.
fn export_spec(spec: &str) -> Option<(crate::export::Format, &'static str, f64, u8)> {
    use crate::export::Format;
    let mut parts = spec.split('-');
    let (format, mime) = match parts.next()? {
        "png" => (Format::Png, "image/png"),
        "jpg" => (Format::Jpg, "image/jpeg"),
        "webp" => (Format::Webp, "image/webp"),
        "avif" => (Format::Avif, "image/avif"),
        "svg" => (Format::Svg, "image/svg+xml"),
        _ => return None,
    };
    let scale = match parts.next()? {
        "0.5" => 0.5,
        "1" => 1.0,
        "2" => 2.0,
        "3" => 3.0,
        "4" => 4.0,
        _ => return None,
    };
    let quality = parts
        .next()?
        .parse()
        .ok()
        .filter(|q| (1..=100).contains(q))?;
    parts
        .next()
        .is_none()
        .then_some((format, mime, scale, quality))
}

/// Longest side of a page-canvas tile render; the viewer picks the tier its display needs.
fn view_size(px: &str) -> Option<f64> {
    matches!(px, "256" | "512" | "1024" | "2048" | "4096").then(|| px.parse().ok())?
}

/// Full renders are capped: a section can span tens of thousands of pixels.
const MAX_RENDER_SIDE: f64 = 8192.0;

fn scope_route(route: &str, generation: u64) -> Result<(&str, &str)> {
    let mut parts = route.splitn(3, '/');
    let action = parts.next().unwrap_or_default();
    let revision = parts.next().ok_or("unknown route")?.parse::<u64>()?;
    let id = parts.next().ok_or("unknown route")?;
    let valid = match action {
        "data" | "image" | "thumb" | "bundle" => valid_id(id),
        "asset" => id.rsplit_once('/').is_some_and(|(scope, key)| {
            valid_id(scope)
                && (1..=64).contains(&key.len())
                && key.bytes().all(|b| b.is_ascii_hexdigit())
        }),
        "export" => id
            .rsplit_once('/')
            .is_some_and(|(scope, spec)| valid_id(scope) && export_spec(spec).is_some()),
        "view" => id
            .rsplit_once('/')
            .is_some_and(|(scope, px)| valid_id(scope) && view_size(px).is_some()),
        _ => false,
    };
    if !valid {
        return Err("unknown route".into());
    }
    if revision != generation {
        return Err(STALE.into());
    }
    Ok((action, id))
}

/// Renders that survive restarts: `<cache>/preview/<file>/<revision>/<route>`. The revision folder is
/// named by figlens version, modification time and size, so a save or an upgrade starts a new one and
/// the old ones are removed. Any cache failure falls back to rendering.
struct DiskCache {
    dir: Option<PathBuf>,
    used: u64,
}

/// Where preview renders are kept: `<figlens cache>/preview`.
pub fn cache_dir() -> Result<PathBuf> {
    Ok(crate::remote::cache_root()?.join("preview"))
}

/// Bytes stored under `dir`, counted recursively.
pub fn cache_bytes(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => cache_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Removes `dir` and everything in it; returns the bytes it held. A missing folder is already clear.
pub fn clear_cache(dir: &Path) -> Result<u64> {
    let bytes = cache_bytes(dir);
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(bytes),
    }
}

const DISK_CACHE_BYTES: u64 = 512 << 20;

fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl DiskCache {
    fn open(file: &Path, stamp: SystemTime) -> DiskCache {
        let open = || -> Result<(PathBuf, u64)> {
            let own = crate::remote::cache_root()?
                .join("preview")
                .join(digest(&fs::canonicalize(file)?.to_string_lossy()));
            let nanos = stamp.duration_since(SystemTime::UNIX_EPOCH)?.as_nanos();
            let revision = digest(&format!(
                "{}:{nanos}:{}",
                env!("CARGO_PKG_VERSION"),
                fs::metadata(file)?.len()
            ));
            let dir = own.join(revision);
            fs::create_dir_all(&dir)?;
            for entry in fs::read_dir(&own)? {
                let path = entry?.path();
                if path != dir {
                    fs::remove_dir_all(path)?;
                }
            }
            let used = fs::read_dir(&dir)?
                .filter_map(|e| e.ok()?.metadata().ok())
                .map(|m| m.len())
                .sum();
            Ok((dir, used))
        };
        match open() {
            Ok((dir, used)) => DiskCache {
                dir: Some(dir),
                used,
            },
            Err(e) => {
                eprintln!("Render cache unavailable ({e}); rendering without it.");
                DiskCache { dir: None, used: 0 }
            }
        }
    }

    /// Empties this file's revision folder (the viewer's "Clear render cache"); returns the bytes freed.
    fn clear(&mut self) -> Result<u64> {
        let Some(dir) = &self.dir else { return Ok(0) };
        let freed = clear_cache(dir)?;
        fs::create_dir_all(dir)?;
        self.used = 0;
        Ok(freed)
    }

    /// Entries are `<mime>\n<area>\n<bytes>`.
    fn get(&self, key: &str) -> Option<(&'static str, Option<String>, Vec<u8>)> {
        let bytes = fs::read(self.dir.as_ref()?.join(digest(key))).ok()?;
        let mut parts = bytes.splitn(3, |b| *b == b'\n');
        let mime = match parts.next()? {
            b"image/png" => "image/png",
            b"image/jpeg" => "image/jpeg",
            b"image/webp" => "image/webp",
            b"image/avif" => "image/avif",
            b"image/svg+xml" => "image/svg+xml",
            _ => return None,
        };
        let area = String::from_utf8(parts.next()?.to_vec())
            .ok()
            .filter(|a| !a.is_empty());
        Some((mime, area, parts.next()?.to_vec()))
    }

    fn put(&mut self, key: &str, mime: &str, area: Option<&str>, body: &[u8]) {
        let Some(dir) = &self.dir else { return };
        if self.used + body.len() as u64 > DISK_CACHE_BYTES {
            return;
        }
        let path = dir.join(digest(key));
        let part = path.with_extension("part");
        let mut entry = format!("{mime}\n{}\n", area.unwrap_or_default()).into_bytes();
        entry.extend_from_slice(body);
        match fs::write(&part, &entry).and_then(|()| fs::rename(&part, &path)) {
            Ok(()) => self.used += entry.len() as u64,
            Err(e) => {
                eprintln!("Render cache disabled after a write failure ({e}).");
                self.dir = None;
            }
        }
    }
}

/// A connection that sends nothing, such as a browser's speculative preconnect.
const IDLE: &str = "idle connection";

fn request(stream: &TcpStream, authority: &str, token: &str) -> Result<String> {
    // Requests are served one at a time, so an idle socket must not hold the service for long.
    stream.set_read_timeout(Some(Duration::from_millis(300)))?;
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    let read = reader.by_ref().take(4097).read_line(&mut first);
    if first.is_empty() {
        return Err(IDLE.into());
    }
    read?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    if first.len() > 4096 {
        return Err("request too large".into());
    }
    let parts: Vec<_> = first.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
        return Err("only HTTP/1.1 GET supported".into());
    }
    let mut host = false;
    let (mut cross_site, mut navigate, mut document) = (false, false, false);
    let mut total = first.len();
    loop {
        let mut line = String::new();
        reader.by_ref().take(4097).read_line(&mut line)?;
        total += line.len();
        if total > 8192 || line.is_empty() {
            return Err("invalid headers".into());
        }
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("host") {
                host = value.trim() == authority;
            }
            cross_site |=
                key.eq_ignore_ascii_case("sec-fetch-site") && value.trim() == "cross-site";
            navigate |= key.eq_ignore_ascii_case("sec-fetch-mode") && value.trim() == "navigate";
            document |= key.eq_ignore_ascii_case("sec-fetch-dest") && value.trim() == "document";
            if key.eq_ignore_ascii_case("origin") && value.trim() != format!("http://{authority}") {
                return Err("origin rejected".into());
            }
        }
    }
    if !host {
        return Err("host rejected".into());
    }
    let route = parts[1]
        .strip_prefix(&format!("/{token}/"))
        .ok_or("route rejected")?;
    // A link from another site may open the page itself; it cannot read it or call the data routes.
    if cross_site && !(route.is_empty() && navigate && document) {
        return Err("cross-site request rejected".into());
    }
    Ok(route.to_owned())
}

fn reply(mut stream: TcpStream, result: Result<(&str, Vec<u8>)>, area: Option<&str>) {
    let (status, mime, body) = match result {
        Ok((mime, body)) => ("200 OK", mime, body),
        Err(e) => (
            "400 Bad Request",
            "text/plain; charset=utf-8",
            e.to_string().into_bytes(),
        ),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob: 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'\r\nConnection: close\r\n\r\n",
        body.len(),
        // Where a rendered image sits in page coordinates, for placing it on the page canvas.
        area.map(|a| format!("X-Export-Area: {a}\r\n"))
            .unwrap_or_default()
    );
    let _ = stream.write_all(&body);
}

pub fn serve(file: &Path, initial: Option<&str>, address: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(address)?;
    let port = listener.local_addr()?.port();
    let host = if address.ip().is_unspecified() {
        if address.is_ipv4() {
            "127.0.0.1".parse()?
        } else {
            "::1".parse()?
        }
    } else {
        address.ip()
    };
    let authority = SocketAddr::new(host, port).to_string();
    let workspace = Workspace::new()?;
    let token = (0..2)
        .map(|_| {
            let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
            hash.write(workspace.0.to_string_lossy().as_bytes());
            format!("{:016x}", hash.finish())
        })
        .collect::<String>();
    eprintln!(
        "Preview: http://{authority}/{token}/\nRead-only local service. Press Ctrl+C to stop."
    );
    let (mut document, mut version, mut source) = staged(file)?;
    let mut generation = 1u64;
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 8));
    // A connection accepted just as a save was noticed is answered after the reload.
    let mut pending: Option<TcpStream> = None;
    loop {
        // Decode once per revision; the revision's workers share the tree and finish before a reload.
        let (fresh, stamp, identity) = {
            let message = document.take_message()?;
            let tree = Tree::new(&message);
            let ctx = Revision {
                tree: &tree,
                index: checked_catalog(&tree)?,
                file,
                initial,
                generation,
                version,
                workspace: &workspace.0,
                authority: &authority,
                token: &token,
                memory: Mutex::new(Memory::default()),
                disk: Mutex::new(DiskCache::open(file, version)),
                bundles: AtomicUsize::new(0),
                source: source.clone(),
                indexes: Mutex::default(),
            };
            let (jobs, queue) = mpsc::channel::<TcpStream>();
            let queue = Mutex::new(queue);
            std::thread::scope(|scope| -> Result<Staged> {
                for _ in 0..workers {
                    // Rendering recurses through the node tree: give workers the CLI's 8 MiB stack.
                    std::thread::Builder::new()
                        .stack_size(8 << 20)
                        .spawn_scoped(scope, || {
                            // A poisoned or closed queue means the revision is ending.
                            while let Some(stream) = queue.lock().ok().and_then(|q| q.recv().ok()) {
                                ctx.serve(stream);
                            }
                        })?;
                }
                // Owned here so the queue closes, and the workers stop, when this revision ends.
                let jobs = jobs;
                loop {
                    let stream = match pending.take() {
                        Some(stream) => stream,
                        None => listener.accept()?.0,
                    };
                    if let Ok(stamp) = fs::metadata(file).and_then(|m| m.modified())
                        && stamp != version
                    {
                        match reload(file, stamp) {
                            Ok(fresh) => {
                                pending = Some(stream);
                                return Ok(fresh);
                            }
                            Err(e) => {
                                reply(stream, Err(e), None);
                                continue;
                            }
                        }
                    }
                    jobs.send(stream)?;
                }
            })?
        };
        document = fresh;
        version = stamp;
        source = identity;
        generation += 1;
    }
}

/// Renders and originals of the current revision (256 MB cap), with each render's export area.
#[derive(Default)]
struct Memory {
    renders: HashMap<String, (&'static str, Vec<u8>)>,
    areas: HashMap<String, String>,
    used: usize,
}

/// One saved revision, shared read-only by the request workers.
struct Revision<'r> {
    tree: &'r Tree<'r>,
    index: Value,
    file: &'r Path,
    initial: Option<&'r str>,
    generation: u64,
    version: SystemTime,
    workspace: &'r Path,
    authority: &'r str,
    token: &'r str,
    memory: Mutex<Memory>,
    disk: Mutex<DiskCache>,
    bundles: AtomicUsize,
    /// `sourceId` and `snapshotId` of the bytes this revision decoded.
    source: Value,
    /// Search indexes of this revision's scopes, most recently used last.
    indexes: Mutex<Vec<(String, Built)>>,
}

/// One scope's index, built once by the first request and shared by those that wait on it.
type Built = Arc<OnceLock<std::result::Result<Arc<search::Index>, String>>>;

/// Scopes indexed at once per revision; each index stays within search's build bounds, and an
/// index still in use or building is never evicted, so no more than this many are alive.
const INDEXED_SCOPES: usize = 2;
/// Query parameters one search or usages route takes.
const MAX_PARAMS: usize = 32;

impl Revision<'_> {
    fn serve(&self, stream: TcpStream) {
        let route = request(&stream, self.authority, self.token);
        // Close idle sockets silently: an error written there would answer the browser's next request.
        if matches!(&route, Err(e) if e.to_string() == IDLE) {
            return;
        }
        if stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .is_err()
        {
            return;
        }
        let key = route.as_ref().ok().cloned();
        let result = route.and_then(|route| self.respond(&route));
        let area = key.and_then(|k| self.memory.lock().ok()?.areas.get(&k).cloned());
        reply(stream, result, area.as_deref());
    }

    fn respond(&self, route: &str) -> Result<(&'static str, Vec<u8>)> {
        let (tree, file) = (self.tree, self.file);
        if route.is_empty() {
            return Ok((
                "text/html; charset=utf-8",
                html(&json!({"initial":self.initial})).into_bytes(),
            ));
        }
        if route == "revision" {
            return Ok((
                "application/json",
                json!({"generation":self.generation,"source":self.source})
                    .to_string()
                    .into_bytes(),
            ));
        }
        // State-changing but scoped: it only drops this file's cached renders, never design data.
        if route == format!("clear-cache/{}", self.generation) {
            *self.memory.lock().map_err(|_| "cache poisoned")? = Memory::default();
            let freed = self.disk.lock().map_err(|_| "cache poisoned")?.clear()?;
            return Ok((
                "application/json",
                json!({"freed":freed}).to_string().into_bytes(),
            ));
        }
        if let Some((mime, bytes)) = self
            .memory
            .lock()
            .map_err(|_| "cache poisoned")?
            .renders
            .get(route)
        {
            return Ok((*mime, bytes.clone()));
        }
        if route == "catalog" {
            return Ok((
                "application/json",
                json!({"generation":self.generation,"source":self.source,"document":self.index})
                    .to_string()
                    .into_bytes(),
            ));
        }
        if let Some(rest) = route.strip_prefix("search/") {
            return Ok(("application/json", self.relations(false, rest)?));
        }
        if let Some(rest) = route.strip_prefix("usages/") {
            return Ok(("application/json", self.relations(true, rest)?));
        }
        let (action, id) = scope_route(route, self.generation)?;
        let (id, key) = id.rsplit_once('/').unwrap_or((id, ""));
        if !tree.nodes.contains_key(crate::export::root_of(id)) {
            return Err("node not found".into());
        }
        // Renders are keyed without the revision number, which restarts at 1 every run.
        let lasting = matches!(action, "image" | "thumb" | "view" | "export")
            .then(|| format!("{action}/{id}/{key}"));
        let hit = lasting
            .as_deref()
            .and_then(|k| self.disk.lock().ok()?.get(k));
        if let Some((mime, area, bytes)) = hit {
            let mut memory = self.memory.lock().map_err(|_| "cache poisoned")?;
            if let Some(area) = area {
                memory.areas.insert(route.to_owned(), area);
            }
            memory
                .renders
                .insert(route.to_owned(), (mime, bytes.clone()));
            return Ok((mime, bytes));
        }
        let mut area = None;
        let response = match action {
            "asset" => {
                // Read just this image; preparing the whole scope per image is far slower.
                let mut zip = crate::open_zip(file)?;
                let image = zip
                    .by_name(&format!("images/{key}"))
                    .map_err(|_| "original image missing")?;
                let size = image.size();
                let mut remaining = crate::parse::IMAGE_TOTAL_BYTES;
                let bytes = crate::read_image(image, size, &mut remaining)?;
                (render::media_type(&bytes), bytes)
            }
            "export" => {
                let (format, mime, scale, quality) = export_spec(key).ok_or("unknown route")?;
                let (image, _) = crate::export::export(
                    tree, file, id, format, false, scale, quality, None, None,
                )?;
                (mime, image.bytes)
            }
            "data" => (
                "application/json",
                data(tree, file, id, false)?.to_string().into_bytes(),
            ),
            "bundle" => {
                // Each request writes its own archive: workers run side by side.
                let n = self.bundles.fetch_add(1, Ordering::Relaxed);
                let zip = self.workspace.join(format!("scope-{n}.zip"));
                crate::bundle::write(tree, file, None, None, id, &zip)?;
                let bytes = fs::read(&zip)?;
                fs::remove_file(&zip)?;
                ("application/zip", bytes)
            }
            _ => {
                let p = Prepared::new(tree, file, id)?;
                let a = render::export_area(p.node()).ok_or("no export area")?;
                area = Some(format!("{},{},{},{}", a[0], a[1], a[2], a[3]));
                let side = (a[2] - a[0]).max(a[3] - a[1]);
                let scale = match action {
                    "thumb" => 180.0 / side,
                    "view" => view_size(key).ok_or("unknown route")? / side,
                    _ => MAX_RENDER_SIDE / side,
                }
                .min(1.0);
                let (image, _) = crate::export::export(
                    tree,
                    file,
                    id,
                    crate::export::Format::Png,
                    false,
                    scale,
                    80,
                    None,
                    None,
                )?;
                ("image/png", image.bytes)
            }
        };
        if fs::metadata(file)?.modified()? != self.version {
            return Err("Save in progress; view is stale. Retrying.".into());
        }
        if matches!(action, "image" | "thumb" | "view" | "asset" | "export") {
            let mut memory = self.memory.lock().map_err(|_| "cache poisoned")?;
            memory.used += response.1.len();
            if memory.used > 256 << 20 {
                memory.renders.clear();
                memory.areas.clear();
                memory.used = response.1.len();
            }
            if let Some(area) = &area {
                memory.areas.insert(route.to_owned(), area.clone());
            }
            memory.renders.insert(route.to_owned(), response.clone());
        }
        if let (Some(k), Ok(mut disk)) = (&lasting, self.disk.lock()) {
            disk.put(k, response.0, area.as_deref(), &response.1);
        }
        Ok(response)
    }
}

impl Revision<'_> {
    /// Ranked search (`search/<revision>?q=…`) or exact usages (`usages/<revision>?target=…`)
    /// over this revision's index of `scope`. Refused queries answer `{"error": …}`.
    fn relations(&self, usages: bool, rest: &str) -> Result<Vec<u8>> {
        let (revision, query) = rest.split_once('?').unwrap_or((rest, ""));
        if revision.parse::<u64>().map_err(|_| "unknown route")? != self.generation {
            return Err(STALE.into());
        }
        let params = form(query)?;
        let known: &[&str] = if usages {
            &[
                "target", "resource", "scope", "relation", "outgoing", "hidden", "limit", "cursor",
                "snapshot",
            ]
        } else {
            &[
                "q", "scope", "type", "hidden", "internal", "limit", "cursor",
            ]
        };
        if let Some((key, _)) = params.iter().find(|(k, _)| !known.contains(&k.as_str())) {
            return Err(format!("unknown parameter {key}").into());
        }
        let one = |key: &str| {
            params
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        let all = |key: &str| {
            params
                .iter()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .collect()
        };
        let flag = |key: &str| match one(key).as_deref() {
            None | Some("0") => Ok(false),
            Some("1") => Ok(true),
            _ => Err(format!("{key} takes 0 or 1")),
        };
        let limit = |default: usize| match one("limit") {
            None => Ok(default),
            Some(n) => n
                .parse::<usize>()
                .map_err(|_| "limit takes a number".to_owned()),
        };
        let scope = one("scope").unwrap_or_else(|| "0:0".to_owned());
        if !valid_id(&scope) {
            return Err("scope takes a node ID".into());
        }
        let reply = (|| -> Result<Value> {
            let index = self.index(&scope)?;
            if usages {
                let req = search::usages::Request {
                    target: one("target").unwrap_or_default(),
                    resource: flag("resource")?,
                    scope,
                    relations: all("relation"),
                    outgoing: flag("outgoing")?,
                    snapshot: one("snapshot"),
                    include_hidden: flag("hidden")?,
                    limit: limit(search::usages::DEFAULT_LIMIT)?,
                    cursor: one("cursor"),
                };
                search::usages_index(&index, &req, self.source.clone())
            } else {
                let req = search::Request {
                    query: one("q").unwrap_or_default(),
                    scope,
                    types: all("type"),
                    include_hidden: flag("hidden")?,
                    include_internal: flag("internal")?,
                    limit: limit(search::DEFAULT_LIMIT)?,
                    cursor: one("cursor"),
                };
                search::search_index(&index, &req, self.source.clone())
            }
        })()
        .unwrap_or_else(|e| {
            let kind = e
                .downcast_ref::<FigError>()
                .map(|f| format!("{:?}", f.kind()));
            json!({"error": {"kind": kind, "message": e.to_string()}, "source": self.source})
        });
        Ok(reply.to_string().into_bytes())
    }

    /// The index of `scope`, built on first use. A new scope replaces the least recently used
    /// index nobody holds; when every slot is building or in use the request is refused.
    fn index(&self, scope: &str) -> Result<Arc<search::Index>> {
        let cell = {
            let mut cells = self.indexes.lock().map_err(|_| "index cache poisoned")?;
            if let Some(i) = cells.iter().position(|(s, _)| s == scope) {
                let entry = cells.remove(i);
                cells.push(entry);
            } else {
                if cells.len() >= INDEXED_SCOPES {
                    let idle = |cell: &Built| {
                        Arc::strong_count(cell) == 1
                            && cell.get().is_some_and(|r| {
                                r.as_ref().map_or(true, |i| Arc::strong_count(i) == 1)
                            })
                    };
                    let i = cells.iter().position(|(_, c)| idle(c)).ok_or_else(|| {
                        FigError::new(
                            crate::ErrorKind::WorkspaceBusy,
                            "every search index of this revision is building or in use; retry shortly",
                        )
                    })?;
                    cells.remove(i);
                }
                cells.push((scope.to_owned(), Built::default()));
            }
            cells.last().expect("pushed").1.clone()
        };
        // Built outside the lock, so other scopes and other routes are not held up.
        cell.get_or_init(|| {
            search::index_tree(self.tree, scope, search::deadline())
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(Into::into)
    }
}

const STALE: &str = "Saved design changed; wait for the preview to refresh.";

/// `application/x-www-form-urlencoded` pairs, as `URLSearchParams` writes them: `+` is a space.
fn form(query: &str) -> Result<Vec<(String, String)>> {
    let decode = |text: &str| -> Result<String> {
        let mut out = Vec::with_capacity(text.len());
        let mut bytes = text.bytes();
        while let Some(b) = bytes.next() {
            out.push(match b {
                b'+' => b' ',
                b'%' => {
                    let mut hex = || bytes.next().and_then(|h| (h as char).to_digit(16));
                    match (hex(), hex()) {
                        (Some(hi), Some(lo)) => (hi * 16 + lo) as u8,
                        _ => return Err("bad percent escape".into()),
                    }
                }
                b => b,
            });
        }
        Ok(String::from_utf8(out).map_err(|_| "query is not UTF-8")?)
    };
    let pairs: Vec<&str> = query.split('&').filter(|p| !p.is_empty()).collect();
    if pairs.len() > MAX_PARAMS {
        return Err("too many query parameters".into());
    }
    pairs
        .into_iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Ok((decode(key)?, decode(value)?))
        })
        .collect()
}

/// Opens a saved revision only once it decodes and its timestamp is stable.
fn reload(file: &Path, stamp: SystemTime) -> Result<Staged> {
    let fresh = staged(file)?;
    checked_catalog(&Tree::new(&fresh.0.message()?))?;
    if fs::metadata(file)?.modified()? != stamp {
        return Err("Save in progress; previous view is stale. Retrying.".into());
    }
    Ok(fresh)
}

/// A revision's document, modification time and source identity: `snapshotId` hashes exactly
/// the bytes decoded, so search and usages refs name the revision this viewer serves.
type Staged = (Document, SystemTime, Value);

fn staged(file: &Path) -> Result<Staged> {
    let canonical = fs::canonicalize(file)?;
    let (bytes, (modified, _)) = search::stage(&canonical)?;
    let source = json!({"sourceId": search::source_id(&canonical),
        "snapshotId": search::snapshot_id(&bytes)});
    let document = Document::from_bytes(&bytes, &file.display().to_string())?;
    Ok((document, modified, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_the_rest_types_nodes_uses() {
        let message = crate::test_support::component_set();
        let set = &catalog(&Tree::new(&message), "0:0")["children"][0];
        assert_eq!(
            (&set["type"], &set["children"][0]["type"]),
            (&json!("COMPONENT_SET"), &json!("COMPONENT"))
        );
    }

    #[test]
    fn a_too_deep_file_fails_the_catalog_instead_of_aborting() {
        let id = |k: usize| {
            if k == 0 {
                "0:0".to_owned()
            } else {
                k.to_string()
            }
        };
        let tree = Tree {
            nodes: std::collections::HashMap::new(),
            children: (0..=crate::expand::MAX_DEPTH)
                .map(|k| (id(k), vec![id(k + 1)]))
                .collect(),
            blobs: Vec::new(),
        };
        let error = checked_catalog(&tree).unwrap_err();
        assert!(error.to_string().contains("layer depth"), "{error}");
    }

    #[test]
    fn live_original_image_route_obeys_the_image_budget() {
        let dir = crate::test_support::TestDir::new();
        let file = dir.oversized_image("a1b2");
        let doc = Document::open(&file).unwrap();
        let message = doc.message().unwrap();
        let tree = Tree::new(&message);
        let version = fs::metadata(&file).unwrap().modified().unwrap();
        let revision = Revision {
            tree: &tree,
            index: Value::Null,
            file: &file,
            initial: None,
            generation: 1,
            version,
            workspace: &dir.join("bundles"),
            authority: "127.0.0.1:1234",
            token: "test",
            memory: Mutex::new(Memory::default()),
            disk: Mutex::new(DiskCache {
                dir: Some(dir.join("cache")),
                used: 0,
            }),
            bundles: AtomicUsize::new(0),
            source: Value::Null,
            indexes: Mutex::default(),
        };
        let error = revision.respond("asset/1/0:1/a1b2").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("parse budget exceeded: image bytes")
        );
        assert!(revision.respond("catalog").is_ok());
    }
    fn served<T>(check: impl FnOnce(&Revision) -> T) -> T {
        let file = Path::new("examples/first-success/design.fig");
        let (doc, version, source) = staged(file).unwrap();
        let message = doc.message().unwrap();
        let tree = Tree::new(&message);
        let dir = crate::test_support::TestDir::new();
        let revision = Revision {
            tree: &tree,
            index: Value::Null,
            file,
            initial: None,
            generation: 1,
            version,
            workspace: &dir.join("bundles"),
            authority: "127.0.0.1:1234",
            token: "test",
            memory: Mutex::new(Memory::default()),
            disk: Mutex::new(DiskCache { dir: None, used: 0 }),
            bundles: AtomicUsize::new(0),
            source,
            indexes: Mutex::default(),
        };
        check(&revision)
    }

    fn reply(revision: &Revision, route: &str) -> Value {
        serde_json::from_slice(&revision.respond(route).unwrap().1).unwrap()
    }

    #[test]
    fn search_and_usages_name_the_snapshot_of_the_bytes_served() {
        let bytes = fs::read("examples/first-success/design.fig").unwrap();
        let snapshot = search::snapshot_id(&bytes);
        served(|r| {
            let found = reply(r, "search/1?q=card");
            let usages = reply(r, "usages/1?target=0%3A2&relation=contains");
            assert_eq!(
                (
                    &found["results"][0]["ref"]["snapshotId"],
                    &found["results"][0]["ref"]["nodeId"],
                    &usages["source"]["snapshotId"],
                    usages["error"].is_null(),
                    &reply(r, "revision")["source"]["snapshotId"],
                ),
                (
                    &json!(snapshot),
                    &json!("0:2"),
                    &json!(snapshot),
                    true,
                    &json!(snapshot)
                )
            );
        });
    }

    #[test]
    fn relation_routes_refuse_stale_revisions_and_unknown_or_bad_parameters() {
        served(|r| {
            let refused = |route: &str| r.respond(route).unwrap_err().to_string();
            assert_eq!(
                [
                    refused("search/2?q=card"),
                    refused("search/1?q=card&path=x"),
                    refused("search/1?q=card&scope=..%2F"),
                ],
                [
                    STALE.to_owned(),
                    "unknown parameter path".to_owned(),
                    "scope takes a node ID".to_owned(),
                ]
            );
            // Refused values answer as data, with the served source.
            assert_eq!(
                (
                    &reply(r, "usages/1?target=0:2&hidden=yes")["error"]["message"],
                    reply(r, "search/1?q=%20")["error"]["message"].is_string(),
                ),
                (&json!("hidden takes 0 or 1"), true)
            );
        });
    }

    #[test]
    fn query_decoding_matches_url_search_params() {
        let pairs = form("q=a+b%2Bc%E4%B8%AD&type=TEXT&type=FRAME&cursor=").unwrap();
        let text = |i: usize| (pairs[i].0.as_str(), pairs[i].1.as_str());
        assert_eq!(
            (text(0), text(1), text(2), text(3)),
            (
                ("q", "a b+c中"),
                ("type", "TEXT"),
                ("type", "FRAME"),
                ("cursor", "")
            )
        );
        assert!(form("q=%E4%B8").is_err() && form("q=%zz").is_err());
        assert!(form(&"a=1&".repeat(MAX_PARAMS + 1)).is_err());
    }

    #[test]
    fn one_index_per_scope_is_shared_and_held_ones_are_never_evicted() {
        served(|r| {
            let (a, again) = (r.index("0:0").unwrap(), r.index("0:0").unwrap());
            let b = r.index("0:1").unwrap();
            let busy = r.index("0:2").map(|_| ()).unwrap_err().to_string();
            drop((again, b));
            let c = r.index("0:2").map(|_| ());
            let kept = Arc::ptr_eq(&a, &r.index("0:0").unwrap());
            assert_eq!(
                (busy.contains("building or in use"), c.is_ok(), kept),
                (true, true, true)
            );
        });
    }

    #[test]
    fn disk_cache_round_trips_renders_and_their_export_area() {
        let dir = crate::test_support::TestDir::new();
        let mut disk = DiskCache {
            dir: Some(dir.join("")),
            used: 0,
        };
        disk.put(
            "image/1:2/",
            "image/png",
            Some("0,0,10,20"),
            b"\x89PNG\nbytes",
        );
        let (mime, area, bytes) = disk.get("image/1:2/").unwrap();
        assert_eq!(
            (mime, area.as_deref(), bytes.as_slice()),
            ("image/png", Some("0,0,10,20"), &b"\x89PNG\nbytes"[..])
        );
        assert!(disk.get("image/1:3/").is_none());
        disk.used = DISK_CACHE_BYTES;
        disk.put("thumb/1:2/", "image/png", None, b"x");
        assert!(
            disk.get("thumb/1:2/").is_none(),
            "the size cap stops new entries"
        );
        assert!(disk.clear().unwrap() > 0);
        assert!(
            disk.get("image/1:2/").is_none(),
            "clearing removes cached renders"
        );
        disk.put("image/1:2/", "image/png", None, b"again");
        assert!(
            disk.get("image/1:2/").is_some(),
            "the cache keeps working after a clear"
        );
        assert_eq!(clear_cache(&dir.join("missing")).unwrap(), 0);
    }

    #[test]
    fn pick_geometry_keeps_only_shapes_their_box_misdescribes() {
        let node = |id: &str, kind: &str, m: [f64; 6]| json!({"id": id, "type": kind, "_r": {"m": m, "size": {"x": 10.0, "y": 20.0}}});
        let mut root = node("1:1", "FRAME", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        root["children"] = json!([
            node("1:2", "RECTANGLE", [0.0, 1.0, -1.0, 0.0, 5.0, 5.0]),
            node("1:3", "ELLIPSE", [1.0, 0.0, 0.0, 1.0, 2.0, 3.0]),
            node("1:4", "RECTANGLE", [1.0, 0.0, 0.0, 1.0, 2.0, 3.0]),
        ]);
        let mut out = serde_json::Map::new();
        pick_geometry(&root, &mut out);
        assert_eq!(out.keys().collect::<Vec<_>>(), ["1:2", "1:3"]);
        assert_eq!(
            out["1:2"],
            json!([0.0, 1.0, -1.0, 0.0, 5.0, 5.0, 10.0, 20.0, 0])
        );
        assert_eq!(out["1:3"][8], json!(1));
    }

    #[test]
    fn embedded_design_cannot_close_script() {
        let page = html(&json!({"name":"</ScRiPt><script>globalThis.pwned=1</script>&"}));
        assert!(!page.contains("</ScRiPt>"));
        assert!(page.contains("\\u003c/ScRiPt\\u003e"));
        let page = html(&json!({"name":"/*SCRIPT*/ /*STYLE*/ /*DATA*/"}));
        assert!(page.contains(r#"{"name":"/*SCRIPT*/ /*STYLE*/ /*DATA*/"}"#));
    }
    #[test]
    fn requests_reject_cross_origin_hosts_methods_and_oversized_headers() {
        fn parse(input: &str) -> Result<String> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let mut client = TcpStream::connect(listener.local_addr()?)?;
            let (server, _) = listener.accept()?;
            client.write_all(input.as_bytes())?;
            client.shutdown(std::net::Shutdown::Write)?;
            request(&server, "127.0.0.1:1234", "secret")
        }
        assert_eq!(
            parse("GET /secret/data/I1:2;3:4 HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n").unwrap(),
            "data/I1:2;3:4"
        );
        assert_eq!(
            parse("GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nSec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n\r\n").unwrap(),
            ""
        );
        for input in [
            "GET /secret/catalog HTTP/1.1\r\nHost: evil.test\r\n\r\n".to_owned(),
            String::new(),
            "GET /secret/catalog HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nOrigin: https://evil.test\r\n\r\n".to_owned(),
            "GET /secret/catalog HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nSec-Fetch-Site: cross-site\r\n\r\n".to_owned(),
            "GET /secret/catalog HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nSec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n\r\n".to_owned(),
            "GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nSec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: iframe\r\n\r\n".to_owned(),
            "POST /secret/bundle/1:2 HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n".to_owned(),
            "GET /wrong/catalog HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n".to_owned(),
            format!("GET /secret/catalog HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nX: {}\r\n\r\n", "x".repeat(9000)),
        ] { assert!(parse(&input).is_err(), "{input}"); }
    }

    #[test]
    fn scoped_routes_pin_images_and_bundles_to_the_loaded_revision() {
        assert_eq!(
            scope_route("image/7/I1:2;3:4", 7).unwrap(),
            ("image", "I1:2;3:4")
        );
        assert_eq!(
            scope_route("asset/7/I1:2;3:4/a1b2c3", 7).unwrap(),
            ("asset", "I1:2;3:4/a1b2c3")
        );
        assert_eq!(
            scope_route("export/7/I1:2;3:4/webp-0.5-80", 7).unwrap(),
            ("export", "I1:2;3:4/webp-0.5-80")
        );
        assert_eq!(
            scope_route("export/7/1:2/svg-1-80", 7).unwrap(),
            ("export", "1:2/svg-1-80")
        );
        assert_eq!(
            scope_route("view/7/1:2/1024", 7).unwrap(),
            ("view", "1:2/1024")
        );
        for route in [
            "image/6/1:2",
            "image/7/../secret",
            "image/7/%31:2",
            "write/7/1:2",
            "image/7/1:2?path=/tmp/x",
            "asset/7/1:2",
            "asset/7/1:2/../x",
            "asset/7/1:2/abc?x",
            "asset/7/../x/abcd",
            "export/7/1:2/gif-1-80",
            "export/7/1:2/png-9-80",
            "export/7/1:2/png-1-0",
            "export/7/1:2/png-1-101",
            "export/7/1:2/png-1-80-x",
            "view/7/1:2/100",
            "view/7/1:2/99999",
        ] {
            assert!(scope_route(route, 7).is_err(), "{route}");
        }
    }

    #[test]
    fn routes_only_accept_exact_instance_ids() {
        for id in ["12:34", "I1:2;I3:4;5:6"] {
            assert!(valid_id(id));
        }
        for id in ["", "../secret", "1:2?x=3", "%2e", "<script>"] {
            assert!(!valid_id(id));
        }
    }
}
