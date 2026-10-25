//! Read-only local viewer and self-contained snapshots. No filesystem routes are exposed.
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, SystemTime};

use crate::{
    Document, Tree,
    render::{self, Prepared},
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
    let mut out = json!({"id":id,"name":n["name"],"type":n["type"],"text":n["textData"]["characters"],
        "children":children.collect::<Vec<_>>()});
    // Figma hides its internal canvas (library components, styles, variables) from the page list.
    if n["internalOnly"] == true {
        out["internal"] = json!(true);
    }
    out
}

/// Prototype interactions of every layer in a scope, from the raw `.fig` node (an expanded instance
/// child reads its source layer): trigger, action and, when it exists, the destination and its name.
fn interactions(tree: &Tree, n: &Value, out: &mut serde_json::Map<String, Value>) {
    if let Some(id) = n["id"].as_str() {
        let source = id.rsplit(';').next().unwrap_or(id).trim_start_matches('I');
        let raw = tree
            .nodes
            .get(source)
            .map(|v| crate::to_json(v))
            .unwrap_or_default();
        let list: Vec<Value> = raw["prototypeInteractions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| i["isDeleted"] != true)
            .flat_map(|i| {
                let trigger = i["event"]["interactionType"].clone();
                i["actions"].as_array().into_iter().flatten().map(move |a| {
                    let t = &a["transitionNodeID"];
                    let target = format!("{}:{}", t["sessionID"], t["localID"]);
                    let name = tree
                        .nodes
                        .get(&target)
                        .map(|v| crate::to_json(v)["name"].clone());
                    json!({
                        "trigger": trigger,
                        "action": a["navigationType"].as_str().or(a["connectionType"].as_str()),
                        "target": name.is_some().then_some(target),
                        "targetName": name,
                    })
                })
            })
            .collect();
        if !list.is_empty() {
            out.insert(id.to_owned(), Value::Array(list));
        }
    }
    for child in n["children"].as_array().into_iter().flatten() {
        interactions(tree, child, out);
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
    interactions(tree, p.node(), &mut prototype);
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
    let doc = Document::open(file)?;
    let message = doc.message()?;
    let tree = Tree::new(&message);
    let workspace = Workspace::new()?;
    let zip = workspace.0.join("scope.zip");
    let warnings = crate::bundle::write(&tree, file, None, None, id, &zip)?;
    let mut payload = data(&tree, file, id, true)?;
    payload["bundle"] = json!(render::base64(&fs::read(zip)?));
    payload["warnings"] = json!(warnings);
    fs::write(out, html(&json!({"snapshot":payload})))?;
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
        return Err("Saved design changed; wait for the preview to refresh.".into());
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
    let mut document = Document::open(file)?;
    let mut version = fs::metadata(file)?.modified()?;
    let mut generation = 1u64;
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 8));
    // A connection accepted just as a save was noticed is answered after the reload.
    let mut pending: Option<TcpStream> = None;
    loop {
        // Decode once per revision; the revision's workers share the tree and finish before a reload.
        let (fresh, stamp) = {
            let message = document.message()?;
            let tree = Tree::new(&message);
            let ctx = Revision {
                tree: &tree,
                index: catalog(&tree, "0:0"),
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
            };
            let (jobs, queue) = mpsc::channel::<TcpStream>();
            let queue = Mutex::new(queue);
            std::thread::scope(|scope| -> Result<(Document, SystemTime)> {
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
                                return Ok((fresh, stamp));
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
}

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
                json!({"generation":self.generation})
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
                json!({"generation":self.generation,"document":self.index})
                    .to_string()
                    .into_bytes(),
            ));
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
                let mut bytes = Vec::new();
                zip.by_name(&format!("images/{key}"))
                    .map_err(|_| "original image missing")?
                    .read_to_end(&mut bytes)?;
                (render::media_type(&bytes), bytes)
            }
            "export" => {
                let (format, mime, scale, quality) = export_spec(key).ok_or("unknown route")?;
                let image =
                    crate::export::export(tree, file, id, format, scale, quality, None, None)?;
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
                let image = crate::export::export(
                    tree,
                    file,
                    id,
                    crate::export::Format::Png,
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

/// Opens a saved revision only once it decodes and its timestamp is stable.
fn reload(file: &Path, stamp: SystemTime) -> Result<Document> {
    let fresh = Document::open(file)?;
    catalog(&Tree::new(&fresh.message()?), "0:0");
    if fs::metadata(file)?.modified()? != stamp {
        return Err("Save in progress; previous view is stale. Retrying.".into());
    }
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;
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
