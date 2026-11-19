//! One writer owns settings/recovery files; only the newest pending snapshot of each is kept.
use crate::settings::Settings;
use std::sync::{Arc, Condvar, Mutex, mpsc};

pub(crate) enum Snapshot {
    Settings(Arc<Settings>),
    Recovery(Arc<String>),
}

#[derive(Default)]
struct Pending {
    settings: Option<Snapshot>,
    recovery: Option<Snapshot>,
    writing: bool,
    stopping: bool,
}

impl Pending {
    fn has_work(&self) -> bool { self.settings.is_some() || self.recovery.is_some() }
}

pub(crate) struct Writer {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    completed: mpsc::Receiver<(Snapshot, Result<(), String>)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    pub(crate) fn new(ctx: egui::Context) -> Self {
        Self::start(|snapshot| match snapshot {
            Snapshot::Settings(settings) => settings.save_checked(),
            Snapshot::Recovery(json) => crate::recovery::save(json),
        }, move || ctx.request_repaint())
    }

    fn start(
        save: impl Fn(&Snapshot) -> Result<(), String> + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let (tx, completed) = mpsc::channel();
        let worker = shared.clone();
        let thread = std::thread::spawn(move || loop {
            let (lock, ready) = &*worker;
            let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !pending.has_work() && !pending.stopping {
                pending = ready.wait(pending).unwrap_or_else(|e| e.into_inner());
            }
            if !pending.has_work() { break; }
            let jobs = [pending.settings.take(), pending.recovery.take()];
            pending.writing = true;
            drop(pending);
            for job in jobs.into_iter().flatten() {
                let result = save(&job);
                if let Err(error) = &result { log::warn!("background persistence: {error}"); }
                // The UI drains acknowledgments every frame, before it submits more work.
                let _ = tx.send((job, result));
                wake();
            }
            let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
            pending.writing = false;
            ready.notify_all();
        });
        Self { shared, completed, thread: Some(thread) }
    }

    pub(crate) fn submit(&self, snapshot: Snapshot) {
        let (lock, ready) = &*self.shared;
        let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
        match snapshot {
            Snapshot::Settings(_) => pending.settings = Some(snapshot),
            Snapshot::Recovery(_) => pending.recovery = Some(snapshot),
        }
        ready.notify_one();
    }

    pub(crate) fn completed(&self) -> Option<(Snapshot, Result<(), String>)> {
        self.completed.try_recv().ok()
    }

    /// Only explicit durable operations/exit wait; ordinary frame updates never do.
    pub(crate) fn flush(&self) {
        let (lock, ready) = &*self.shared;
        let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
        while pending.has_work() || pending.writing {
            pending = ready.wait(pending).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let (lock, ready) = &*self.shared;
        lock.lock().unwrap_or_else(|e| e.into_inner()).stopping = true;
        ready.notify_one();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slow_storage_never_blocks_submission_and_coalesces_pending_snapshots() {
        let (started, running) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let written = writes.clone();
        let writer = Writer::start(move |snapshot| {
            let value = match snapshot {
                Snapshot::Recovery(value) => value.as_str(),
                Snapshot::Settings(_) => "settings",
            };
            if value == "first" {
                started.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            written.lock().unwrap().push(value.to_string());
            Ok(())
        }, || {});
        writer.submit(Snapshot::Recovery(Arc::new("first".into())));
        running.recv_timeout(Duration::from_secs(5)).unwrap();
        // The first write cannot finish yet; submissions must still return.
        for i in 0..100 { writer.submit(Snapshot::Recovery(Arc::new(i.to_string()))); }
        writer.submit(Snapshot::Settings(Arc::new(Settings::default())));
        release.send(()).unwrap();
        writer.flush();
        assert_eq!(*writes.lock().unwrap(), ["first", "settings", "99"]);
        assert!(writer.completed().unwrap().1.is_ok());
        assert!(writer.completed().unwrap().1.is_ok());
        assert!(writer.completed().unwrap().1.is_ok());
        assert!(writer.completed().is_none());
    }

    #[test]
    fn failed_saves_are_reported_and_shutdown_flushes_the_retry() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("hookecho-persistence-{}-{nonce}.json", std::process::id()));
        let destination = path.clone();
        let writer = Writer::start(move |snapshot| {
            let Snapshot::Recovery(value) = snapshot else { panic!("unexpected settings") };
            if value.as_str() == "fail" { return Err("disk full".into()); }
            crate::settings::atomic_write(&destination, value.as_bytes()).map_err(|e| e.to_string())
        }, || {});
        writer.submit(Snapshot::Recovery(Arc::new("fail".into())));
        writer.flush();
        assert_eq!(writer.completed().unwrap().1.unwrap_err(), "disk full");
        assert!(!path.exists());
        writer.submit(Snapshot::Recovery(Arc::new("saved".into())));
        drop(writer);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved");
        std::fs::remove_file(path).unwrap();
    }
}
