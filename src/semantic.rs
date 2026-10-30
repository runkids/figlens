//! Optional local semantic retrieval (issue #94): a trusted embedding child behind a bounded
//! JSON-lines protocol, the profile fingerprint, versioned vectors, exact retrieval and the
//! Reciprocal Rank Fusion shared with lexical search. No model is loaded unless a profile is
//! configured; nothing here downloads, installs or transmits anything.

use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

/// The child protocol; a child announcing another one is refused.
pub const PROTOCOL: &str = "figlens-embed-1";
/// Stored vector format: little-endian f32, L2-normalized, `dim` values.
pub const VECTORS: &str = "f32le-1";
/// Hybrid ranking: `sum(1 / (60 + rank))` over the channels that answered.
pub const FUSION: &str = "rrf-60-1";
const RRF_K: f64 = 60.0;

/// Texts one request may carry, bytes per text, and the widest vector a profile may declare.
pub const MAX_BATCH: usize = 32;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_DIM: usize = 4096;
/// One reply line: every float of a full batch at its longest JSON spelling, plus framing.
const MAX_LINE: usize = MAX_BATCH * MAX_DIM * 16 + 4096;
/// Hello line, and the time a child gets to load its model and to answer one batch.
const MAX_HELLO: usize = 64 * 1024;
const READY: Duration = Duration::from_secs(120);
pub const DEADLINE: Duration = Duration::from_secs(60);
/// How often a waiting request looks at its cancel flag.
const POLL: Duration = Duration::from_millis(25);
/// Time a child gets to exit after stdin closes before it is killed.
const EXIT: Duration = Duration::from_secs(2);
/// Characters per chunk. Every supported profile counts a CJK character as at most one token
/// and a chunk this size stays under its token limit after the instruction; a longer chunk the
/// child refuses is reported as uncovered, never truncated.
pub const CHUNK_CHARS: usize = 400;
/// Vectors one exact scan may compare; beyond it the scan refuses instead of slowing silently.
pub const MAX_SCAN: usize = 200_000;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn fail(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}

/// What a request embeds: the profile applies its query instruction only to queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Query,
    Passage,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Query => "query",
            Kind::Passage => "passage",
        }
    }
}

/// Checks one batch before it reaches a child.
fn check_batch(texts: &[String]) -> Result<()> {
    if texts.is_empty() || texts.len() > MAX_BATCH {
        return Err(fail(format!("a batch holds 1 to {MAX_BATCH} texts")));
    }
    if let Some(t) = texts
        .iter()
        .find(|t| t.is_empty() || t.len() > MAX_TEXT_BYTES)
    {
        return Err(fail(format!(
            "a text holds 1 to {MAX_TEXT_BYTES} bytes, got {}",
            t.len()
        )));
    }
    Ok(())
}

/// Checks that `v` holds `dim` finite values.
pub fn check_vector(v: &[f32], dim: usize) -> Result<()> {
    if v.len() != dim {
        return Err(fail(format!(
            "vector has {} values, profile declares {dim}",
            v.len()
        )));
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err(fail("vector holds a non-finite value"));
    }
    Ok(())
}

/// Serves the child side of the protocol on stdin/stdout until stdin closes: `hello` first,
/// then one reply line per request line. A malformed or oversized request gets an error line
/// and the child keeps serving; `embed` errors are reported the same way.
pub fn serve(
    hello: &Json,
    mut embed: impl FnMut(Kind, &[String]) -> Result<Vec<Vec<f32>>>,
) -> Result<()> {
    let dim = hello["dim"].as_u64().ok_or("hello needs dim")? as usize;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{hello}")?;
    out.flush()?;
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut input)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(());
        }
        let reply = if n > MAX_LINE {
            // Drain the rest of the oversized line so the next one parses.
            let mut rest = Vec::new();
            input.read_until(b'\n', &mut rest)?;
            json!({"id": null, "error": "request too large"})
        } else {
            answer(&line, dim, &mut embed)
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
}

fn answer(
    line: &[u8],
    dim: usize,
    embed: &mut impl FnMut(Kind, &[String]) -> Result<Vec<Vec<f32>>>,
) -> Json {
    let req: Json = match serde_json::from_slice(line) {
        Ok(r) => r,
        Err(_) => return json!({"id": null, "error": "request is not JSON"}),
    };
    let id = req["id"].clone();
    let kind = match req["kind"].as_str() {
        Some("query") => Kind::Query,
        Some("passage") => Kind::Passage,
        _ => return json!({"id": id, "error": "kind is query or passage"}),
    };
    let texts: Option<Vec<String>> = req["texts"]
        .as_array()
        .and_then(|a| a.iter().map(|t| t.as_str().map(str::to_owned)).collect());
    let run = || -> Result<Vec<Vec<f32>>> {
        let texts = texts.ok_or("texts is an array of strings")?;
        check_batch(&texts)?;
        let vectors = embed(kind, &texts)?;
        if vectors.len() != texts.len() {
            return Err(fail("embedder returned the wrong number of vectors"));
        }
        for v in &vectors {
            check_vector(v, dim)?;
        }
        Ok(vectors)
    };
    match run() {
        Ok(vectors) => json!({"id": id, "vectors": vectors}),
        Err(e) => json!({"id": id, "error": e.to_string()}),
    }
}

/// One running child. Its stdout is read on a thread so every wait has a deadline and a cancel
/// check; a timeout or cancel kills the child's process tree and leaves the runtime dead.
pub struct Runtime {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<std::io::Result<Vec<u8>>>,
    /// The child's hello line: its protocol, dimension and model contract.
    pub hello: Json,
    pub dim: usize,
    next: u64,
}

/// Why a request ended without vectors.
#[derive(Debug)]
pub enum Stop {
    Cancelled,
    Timeout,
    /// The child refused the input; it still serves.
    Refused(String),
    /// The child broke the protocol or exited; the runtime is dead.
    Failed(String),
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Stop::Cancelled => f.write_str("cancelled"),
            Stop::Timeout => f.write_str("the model runtime did not answer in time"),
            Stop::Refused(m) => write!(f, "the model runtime refused the input: {m}"),
            Stop::Failed(m) => write!(f, "the model runtime failed: {m}"),
        }
    }
}

impl Error for Stop {}

impl Runtime {
    /// Starts `exe` with `args`, without a shell, and waits for its hello.
    pub fn start(exe: &Path, args: &[&Path], cancel: &AtomicBool) -> Result<Runtime> {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd
            .spawn()
            .map_err(|e| fail(format!("cannot start {}: {e}", exe.display())))?;
        let stdout = child.stdout.take().ok_or("child has no stdout")?;
        let (tx, lines) = sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let got = (&mut reader)
                    .take(MAX_LINE as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .map(|_| line);
                let end = !matches!(&got, Ok(l) if l.ends_with(b"\n"));
                if tx.send(got).is_err() || end {
                    return;
                }
            }
        });
        let mut rt = Runtime {
            stdin: child.stdin.take(),
            child,
            lines,
            hello: Json::Null,
            dim: 0,
            next: 0,
        };
        let line = rt.wait(READY, cancel, MAX_HELLO)?;
        let hello: Json =
            serde_json::from_slice(&line).map_err(|_| rt.dead("hello is not JSON"))?;
        if hello["protocol"] != PROTOCOL {
            return Err(rt
                .dead(&format!(
                    "child speaks {}, not {PROTOCOL}",
                    hello["protocol"]
                ))
                .into());
        }
        let dim = hello["dim"].as_u64().unwrap_or(0) as usize;
        if dim == 0 || dim > MAX_DIM {
            return Err(rt.dead("hello declares no dimension in 1..=4096").into());
        }
        rt.hello = hello;
        rt.dim = dim;
        Ok(rt)
    }

    /// Kills the child and returns why.
    fn dead(&mut self, why: &str) -> Stop {
        self.kill();
        Stop::Failed(why.to_owned())
    }

    /// Kills the child's process group (on Windows the child alone: it starts no processes)
    /// and waits for it to exit, so cancellation leaves nothing running.
    fn kill(&mut self) {
        #[cfg(unix)]
        // SAFETY: plain syscall; the child leads its own process group.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stdin = None;
    }

    pub fn alive(&mut self) -> bool {
        self.stdin.is_some() && matches!(self.child.try_wait(), Ok(None))
    }

    fn wait(
        &mut self,
        limit: Duration,
        cancel: &AtomicBool,
        max: usize,
    ) -> std::result::Result<Vec<u8>, Stop> {
        let end = Instant::now() + limit;
        loop {
            if cancel.load(Ordering::Relaxed) {
                self.kill();
                return Err(Stop::Cancelled);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.kill();
                return Err(Stop::Timeout);
            }
            match self.lines.recv_timeout(left.min(POLL)) {
                Ok(Ok(line)) if line.len() > max || !line.ends_with(b"\n") => {
                    return Err(self.dead(if line.is_empty() {
                        "child exited"
                    } else {
                        "reply too large or cut off"
                    }));
                }
                Ok(Ok(line)) => return Ok(line),
                Ok(Err(e)) => return Err(self.dead(&e.to_string())),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(self.dead("child exited")),
            }
        }
    }

    /// Embeds one batch, validating every vector against the declared dimension.
    pub fn embed(
        &mut self,
        kind: Kind,
        texts: &[String],
        cancel: &AtomicBool,
    ) -> std::result::Result<Vec<Vec<f32>>, Stop> {
        check_batch(texts).map_err(|e| Stop::Refused(e.to_string()))?;
        self.next += 1;
        let id = self.next;
        let req = json!({"id": id, "kind": kind.as_str(), "texts": texts});
        let sent = match self.stdin.as_mut() {
            Some(s) => writeln!(s, "{req}").and_then(|_| s.flush()),
            None => return Err(Stop::Failed("runtime stopped".into())),
        };
        if let Err(e) = sent {
            return Err(self.dead(&e.to_string()));
        }
        let line = self.wait(DEADLINE, cancel, MAX_LINE)?;
        let reply: Json =
            serde_json::from_slice(&line).map_err(|_| self.dead("reply is not JSON"))?;
        if reply["id"] != id {
            return Err(self.dead("reply answers another request"));
        }
        if let Some(e) = reply["error"].as_str() {
            return Err(Stop::Refused(e.to_owned()));
        }
        let parsed: Option<Vec<Vec<f32>>> = reply["vectors"].as_array().and_then(|rows| {
            rows.iter()
                .map(|r| {
                    r.as_array()?
                        .iter()
                        .map(|x| x.as_f64().map(|x| x as f32))
                        .collect()
                })
                .collect()
        });
        let vectors = match parsed {
            Some(v) if v.len() == texts.len() => v,
            _ => return Err(self.dead("reply holds no vector per text")),
        };
        for v in &vectors {
            if let Err(e) = check_vector(v, self.dim) {
                return Err(self.dead(&e.to_string()));
            }
        }
        Ok(vectors)
    }

    /// Closes stdin, gives the child `EXIT` to leave, then kills what is left.
    pub fn stop(mut self) {
        self.stdin = None;
        let end = Instant::now() + EXIT;
        while Instant::now() < end {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(POLL);
        }
        self.kill();
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1 << 20];
    loop {
        match file.read(&mut buf)? {
            0 => break,
            n => hash.update(&buf[..n]),
        }
    }
    Ok(crate::search::hex(&hash.finalize()))
}

/// Verifies every file `SHA256SUMS` lists in `dir` and returns `(name, digest)` sorted. Names
/// stay inside `dir`: no absolute paths, `..` or empty components.
pub fn verify_sums(dir: &Path) -> Result<Vec<(String, String)>> {
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS"))
        .map_err(|e| fail(format!("{}: {e}", dir.join("SHA256SUMS").display())))?;
    let mut files = Vec::new();
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let (digest, name) = line
            .split_once(char::is_whitespace)
            .map(|(d, n)| (d, n.trim_start().trim_start_matches('*')))
            .ok_or_else(|| fail(format!("SHA256SUMS line is not `<digest> <file>`: {line}")))?;
        let safe = !name.is_empty()
            && Path::new(name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !safe || digest.len() != 64 {
            return Err(fail(format!(
                "SHA256SUMS names an unsafe file or digest: {line}"
            )));
        }
        let got = sha256_file(&dir.join(name))?;
        if !got.eq_ignore_ascii_case(digest) {
            return Err(fail(format!("{name}: checksum mismatch")));
        }
        files.push((name.to_owned(), got));
    }
    if files.is_empty() {
        return Err(fail("SHA256SUMS lists no files"));
    }
    files.sort();
    Ok(files)
}

/// A configured local profile: the trusted child executable, its model directory and the
/// fingerprint over everything that decides the vector space.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub exe: PathBuf,
    pub model: PathBuf,
    pub dim: usize,
    /// The child's hello: model contract (pooling, normalization, instruction, token limit) and
    /// runtime version, as the child declared it.
    pub contract: Json,
}

/// `profileId`: SHA-256 over the protocol and vector format, the executable's digest, every
/// model file's digest and the child's declared contract.
pub fn fingerprint(exe_sha: &str, files: &[(String, String)], contract: &Json) -> String {
    let doc = json!({"protocol": PROTOCOL, "vectors": VECTORS, "runtime": exe_sha, "files": files, "contract": contract});
    let digest = Sha256::digest(doc.to_string());
    format!("p1-{}", &crate::search::hex(&digest)[..32])
}

impl Profile {
    /// Verifies the model files, starts the child once to read its contract and fingerprints
    /// the result. Nothing is downloaded: both paths must already exist locally.
    pub fn probe(exe: &Path, model: &Path, cancel: &AtomicBool) -> Result<Profile> {
        let exe =
            std::fs::canonicalize(exe).map_err(|e| fail(format!("{}: {e}", exe.display())))?;
        let model =
            std::fs::canonicalize(model).map_err(|e| fail(format!("{}: {e}", model.display())))?;
        let files = verify_sums(&model)?;
        let exe_sha = sha256_file(&exe)?;
        let rt = Runtime::start(&exe, &[&model], cancel)?;
        let (dim, contract) = (rt.dim, rt.hello.clone());
        rt.stop();
        Ok(Profile {
            id: fingerprint(&exe_sha, &files, &contract),
            exe,
            model,
            dim,
            contract,
        })
    }
}

/// Encodes one vector in `VECTORS`.
pub fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Decodes one stored vector, refusing a wrong length or a non-finite value.
pub fn decode(bytes: &[u8], dim: usize) -> Result<Vec<f32>> {
    if bytes.len() != dim * 4 {
        return Err(fail("stored vector has the wrong length"));
    }
    let v: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    check_vector(&v, dim)?;
    Ok(v)
}

/// The `k` nearest of `vectors` to `query` by dot product (both normalized, so cosine), ties in
/// key order. Refuses more than `MAX_SCAN` vectors rather than scan an unmeasured capacity.
pub fn nearest<'a, K: Ord + Copy + 'a>(
    query: &[f32],
    vectors: impl IntoIterator<Item = (K, &'a [f32])>,
    k: usize,
) -> Result<Vec<(K, f32)>> {
    let mut scored = Vec::new();
    for (n, (key, v)) in vectors.into_iter().enumerate() {
        if n == MAX_SCAN {
            return Err(fail(format!(
                "more than {MAX_SCAN} vectors in one exact scan"
            )));
        }
        scored.push((key, query.iter().zip(v).map(|(a, b)| a * b).sum::<f32>()));
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(k);
    Ok(scored)
}

/// Fuses ranked lists (best first) with `FUSION`; a key's score sums `1 / (60 + rank)` over
/// the lists that hold it. Ties keep key order. Scores are ranks fused, not probabilities.
pub fn fuse<K: Ord + Copy>(lists: &[&[K]]) -> Vec<(K, f64)> {
    let mut scores = std::collections::BTreeMap::new();
    for list in lists {
        for (i, key) in list.iter().enumerate() {
            *scores.entry(*key).or_insert(0.0) += 1.0 / (RRF_K + (i + 1) as f64);
        }
    }
    let mut fused: Vec<(K, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    fused
}

/// One model input: which field of a record it came from and which chunk of that field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub field: &'static str,
    pub index: u32,
    pub text: String,
}

/// Splits `text` into consecutive chunks of at most `CHUNK_CHARS` characters, so a long field
/// is covered to its end, never truncated. Blank text gives no chunk.
pub fn chunks(field: &'static str, text: &str) -> Vec<Chunk> {
    let text = text.trim();
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(CHUNK_CHARS)
        .enumerate()
        .map(|(i, c)| Chunk {
            field,
            index: i as u32,
            text: c.iter().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
