//! Optional local semantic retrieval (issue #94): a trusted embedding child behind a bounded
//! JSON-lines protocol, the profile fingerprint, versioned vectors, exact retrieval and the
//! Reciprocal Rank Fusion shared with lexical search. No model is loaded unless a profile is
//! configured; nothing here downloads, installs or transmits anything.

use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
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

/// The search mode a caller asks for; `visual` is reserved for #95.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Text,
    Semantic,
    Hybrid,
}

impl Mode {
    pub fn parse(mode: &str) -> Option<Mode> {
        match mode {
            "text" => Some(Mode::Text),
            "semantic" => Some(Mode::Semantic),
            "hybrid" => Some(Mode::Hybrid),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Text => "text",
            Mode::Semantic => "semantic",
            Mode::Hybrid => "hybrid",
        }
    }
}

/// Locks `m`, giving up when `cancel` is set while another request holds it.
pub(crate) fn lock<'a, T>(
    m: &'a std::sync::Mutex<T>,
    cancel: &AtomicBool,
) -> std::result::Result<std::sync::MutexGuard<'a, T>, Stop> {
    loop {
        match m.try_lock() {
            Ok(g) => return Ok(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Ok(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if cancel.load(Ordering::Relaxed) => {
                return Err(Stop::Cancelled);
            }
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(POLL),
        }
    }
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

/// A text the model can never embed as given (over its token limit, empty or oversized): the
/// one refusal kept as a permanent uncovered chunk. Any other error stays retryable.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for Refused {}

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
    // Ranking reads dot products as cosine, so every vector must be unit length.
    if (v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() > 1e-3 {
        return Err(fail("vector is not L2-normalized"));
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
        // A line of exactly MAX_LINE + 1 bytes already ended with its newline.
        let reply = if n > MAX_LINE && line.last() != Some(&b'\n') {
            // Skip the rest of the oversized line without holding it, so the next one parses.
            loop {
                let buf = input.fill_buf()?;
                match buf.iter().position(|&b| b == b'\n') {
                    Some(i) => break input.consume(i + 1),
                    None if buf.is_empty() => break,
                    None => {
                        let n = buf.len();
                        input.consume(n)
                    }
                }
            }
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
        check_batch(&texts).map_err(|e| Refused(e.to_string()))?;
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
        Err(e) => json!({"id": id, "error": e.to_string(), "refused": e.is::<Refused>()}),
    }
}

/// One running child. Its stdin is written and its stdout read on threads, so a child that
/// stops reading or answering never blocks a caller past its deadline or cancel check; a
/// timeout or cancel kills the child's process tree and leaves the runtime dead.
pub struct Runtime {
    child: Child,
    tree: Tree,
    /// Request lines for the writer thread; dropping it closes the child's stdin.
    requests: Option<SyncSender<Vec<u8>>>,
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
        let (mut child, tree) = spawn_tree(&mut cmd)
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
        let mut stdin = child.stdin.take().ok_or("child has no stdin")?;
        let (requests, queued) = sync_channel::<Vec<u8>>(1);
        // Ends when the sender drops or a write fails (the child died), closing stdin.
        std::thread::spawn(move || {
            for line in queued {
                if stdin.write_all(&line).and_then(|_| stdin.flush()).is_err() {
                    return;
                }
            }
        });
        let mut rt = Runtime {
            requests: Some(requests),
            tree,
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
        self.tree.kill(&self.child);
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The writer ends once the pipe closes. It is not joined: a descendant that inherited
        // the pipe could keep a blocked write alive, and the caller must not wait on that.
        self.requests = None;
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn alive(&mut self) -> bool {
        self.requests.is_some() && matches!(self.child.try_wait(), Ok(None))
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
        let mut req = json!({"id": id, "kind": kind.as_str(), "texts": texts}).to_string();
        req.push('\n');
        // The writer is idle: every earlier request was answered, or the runtime was killed.
        let sent = match &self.requests {
            Some(r) => r.try_send(req.into_bytes()).is_ok(),
            None => return Err(Stop::Failed("runtime stopped".into())),
        };
        if !sent {
            return Err(self.dead("the child stopped reading requests"));
        }
        let line = self.wait(DEADLINE, cancel, MAX_LINE)?;
        let mut reply: Json =
            serde_json::from_slice(&line).map_err(|_| self.dead("reply is not JSON"))?;
        if reply["id"] != id {
            return Err(self.dead("reply answers another request"));
        }
        // Only a declared refusal is the input's fault; any other error is the runtime's.
        if let Some(e) = reply["error"].as_str() {
            return Err(match reply["refused"] == true {
                true => Stop::Refused(e.to_owned()),
                false => Stop::Failed(e.to_owned()),
            });
        }
        let parsed: Option<Vec<Vec<f32>>> = serde_json::from_value(reply["vectors"].take()).ok();
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
        self.requests = None;
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

/// What ends a child started by `spawn_tree` together with every process it started.
pub(crate) struct Tree {
    /// Windows: the job object the child runs in, so killing it ends every descendant.
    #[cfg(windows)]
    job: Job,
}

impl Tree {
    /// Kills `child`'s process group (Unix) or job (Windows); the caller still reaps `child`.
    fn kill(&self, child: &Child) {
        #[cfg(unix)]
        // SAFETY: plain syscall; the child leads its own process group.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        let _ = child;
        #[cfg(windows)]
        self.job.terminate();
    }
}

/// Spawns `cmd` (no shell) as the root of its own process tree: its own process group on
/// Unix; on Windows suspended until it is in a kill-on-close job, so nothing it starts escapes.
pub(crate) fn spawn_tree(cmd: &mut Command) -> Result<(Child, Tree)> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, CREATE_SUSPENDED);
    let mut child = cmd.spawn()?;
    #[cfg(windows)]
    let job = match Job::assign(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    #[cfg(unix)]
    let _ = &mut child;
    Ok((
        child,
        Tree {
            #[cfg(windows)]
            job,
        },
    ))
}

/// Trees of children `spawn_owned` started, by process id, until `kill_tree` or `forget_tree`.
static TREES: std::sync::Mutex<Vec<(u32, Tree)>> = std::sync::Mutex::new(Vec::new());

/// `spawn_tree` for a caller that keeps only the `Child`: the tree is kept here by process id.
pub(crate) fn spawn_owned(cmd: &mut Command) -> Result<Child> {
    let (child, tree) = spawn_tree(cmd)?;
    TREES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((child.id(), tree));
    Ok(child)
}

fn take_tree(pid: u32) -> Option<Tree> {
    let mut trees = TREES.lock().unwrap_or_else(|e| e.into_inner());
    let at = trees.iter().position(|(p, _)| *p == pid)?;
    Some(trees.swap_remove(at).1)
}

/// Kills the whole tree of `child` when `spawn_owned` started it; any other child is left
/// alone. The caller still kills and reaps `child` itself.
pub(crate) fn kill_tree(child: &Child) {
    if let Some(tree) = take_tree(child.id()) {
        tree.kill(child);
    }
}

/// Forgets the tree of a reaped `spawn_owned` child, before its process id can be reused.
pub(crate) fn forget_tree(child: &Child) {
    let _ = take_tree(child.id());
}

#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    /// Resumes every thread of a process; std gives no thread handle to resume one.
    fn NtResumeProcess(process: windows_sys::Win32::Foundation::HANDLE) -> i32;
}

/// A Windows job object that kills its processes when terminated or closed.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: a job handle may be used and closed from any thread.
#[cfg(windows)]
unsafe impl Send for Job {}

#[cfg(windows)]
impl Job {
    /// Puts the suspended `child` in a new kill-on-close job, then resumes it, so every process
    /// it starts belongs to the job.
    fn assign(child: &Child) -> Result<Job> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        // SAFETY: plain Win32 calls on a handle this function owns and the child's own handle.
        unsafe {
            let job = Job(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
            if job.0.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 || AssignProcessToJobObject(job.0, child.as_raw_handle()) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let status = NtResumeProcess(child.as_raw_handle());
            if status < 0 {
                return Err(fail(format!(
                    "cannot resume the model runtime (NTSTATUS {status:#x})"
                )));
            }
            Ok(job)
        }
    }

    fn terminate(&self) {
        // SAFETY: the handle stays open until drop.
        unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) };
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: closed once; kill-on-close ends what is still running.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}

fn sha256_file(path: &Path, cancel: &AtomicBool) -> Result<String> {
    // Checked before open: opening a FIFO or device would block or read without end.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(fail(format!("{} is not a regular file", path.display())));
    }
    let mut file =
        std::fs::File::open(path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled.into());
        }
        match file.read(&mut buf)? {
            0 => break,
            n => hash.update(&buf[..n]),
        }
    }
    Ok(crate::search::hex(&hash.finalize()))
}

/// Checks each `(file, sha256)` in `dir` and returns them sorted. Every name is a fixed
/// relative path from a pinned profile, so nothing outside `dir` is read.
pub fn verify_files(
    dir: &Path,
    files: &[(&str, &str)],
    cancel: &AtomicBool,
) -> Result<Vec<(String, String)>> {
    let mut checked = Vec::new();
    for (name, digest) in files {
        let got = sha256_file(&dir.join(name), cancel)?;
        if got != *digest {
            return Err(fail(format!(
                "{name} in {} is not the pinned file (sha256 {got}, expected {digest})",
                dir.display()
            )));
        }
        checked.push((name.to_string(), got));
    }
    checked.sort();
    Ok(checked)
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
    /// What `unchanged` checks before each start: the executable's and each pinned model
    /// file's SHA-256 at configure.
    pub stamps: Json,
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
    pub fn probe(
        exe: &Path,
        model: &Path,
        files: &[(&str, &str)],
        cancel: &AtomicBool,
    ) -> Result<Profile> {
        let exe =
            std::fs::canonicalize(exe).map_err(|e| fail(format!("{}: {e}", exe.display())))?;
        let model =
            std::fs::canonicalize(model).map_err(|e| fail(format!("{}: {e}", model.display())))?;
        let files = verify_files(&model, files, cancel)?;
        let exe_sha = sha256_file(&exe, cancel)?;
        let rt = Runtime::start(&exe, &[&model], cancel)?;
        let (dim, contract) = (rt.dim, rt.hello.clone());
        rt.stop();
        Ok(Profile {
            id: fingerprint(&exe_sha, &files, &contract),
            exe,
            model,
            dim,
            contract,
            stamps: json!({"runtime": exe_sha, "files": files}),
        })
    }

    /// Refuses a profile whose executable or model files changed since configure, hashing each
    /// again before a child loads them; a loaded child is not rechecked per query.
    pub fn unchanged(&self, cancel: &AtomicBool) -> Result<()> {
        if sha256_file(&self.exe, cancel)? != self.stamps["runtime"] {
            return Err(fail(format!(
                "{} changed since models configure",
                self.exe.display()
            )));
        }
        let files = self.stamps["files"]
            .as_array()
            .ok_or("profile record has no file digests")?;
        for file in files {
            let (Some(name), Some(digest)) = (file[0].as_str(), file[1].as_str()) else {
                return Err(fail("profile record has a malformed file digest"));
            };
            if sha256_file(&self.model.join(name), cancel)? != digest {
                return Err(fail(format!(
                    "{name} in {} changed since models configure",
                    self.model.display()
                )));
            }
        }
        Ok(())
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

/// Splits `text` into consecutive chunks of at most `CHUNK_CHARS` characters, so a long field
/// is covered to its end, never truncated, without copying it. Blank text gives no chunk.
pub fn chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text.trim();
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .char_indices()
            .nth(CHUNK_CHARS)
            .map_or(rest.len(), |(i, _)| i);
        let (head, tail) = rest.split_at(end);
        rest = tail;
        Some(head)
    })
}

pub mod profile;

#[cfg(test)]
mod tests;
