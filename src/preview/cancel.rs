//! Revision-owned model requests: a same-origin cancellation stops only the named call.
use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub(super) struct Requests(Mutex<State>);

#[derive(Default)]
struct State {
    running: HashMap<String, Arc<AtomicBool>>,
    // A cancel can reach another worker before its request. Keep only bounded revision-local IDs.
    early: std::collections::VecDeque<String>,
}

pub(super) struct Running<'a> {
    owner: &'a Requests,
    id: String,
    pub(super) cancel: Arc<AtomicBool>,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if let Ok(mut calls) = self.owner.0.lock() {
            calls.running.remove(&self.id);
        }
    }
}

impl Requests {
    pub(super) fn begin(&self, id: Option<&str>, ended: &AtomicBool) -> Result<Running<'_>> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        if id.is_some_and(|id| !valid(id)) {
            return Err("invalid requestId".into());
        }
        let id = id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("server:{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut calls = self.0.lock().map_err(|_| "requests poisoned")?;
        if ended.load(Ordering::SeqCst) {
            return Err(STALE.into());
        }
        if let Some(at) = calls.early.iter().position(|early| early == &id) {
            calls.early.remove(at);
            return Err(FigError::new(crate::ErrorKind::Cancelled, "search cancelled").into());
        }
        if calls.running.contains_key(&id) {
            return Err("requestId is already running".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        calls.running.insert(id.clone(), cancel.clone());
        Ok(Running {
            owner: self,
            id,
            cancel,
        })
    }

    pub(super) fn cancel(&self, id: &str) -> Result<bool> {
        if !valid(id) {
            return Err("invalid requestId".into());
        }
        let mut calls = self.0.lock().map_err(|_| "requests poisoned")?;
        if let Some(cancel) = calls.running.get(id) {
            cancel.store(true, Ordering::SeqCst);
            Ok(true)
        } else {
            if !calls.early.iter().any(|early| early == id) {
                if calls.early.len() == 64 {
                    calls.early.pop_front();
                }
                calls.early.push_back(id.to_owned());
            }
            Ok(false)
        }
    }

    pub(super) fn stop(&self) {
        if let Ok(calls) = self.0.lock() {
            for cancel in calls.running.values() {
                cancel.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
