//! A selected-frame fetch owns loading state only until it is replaced or completed.
#[derive(Default)]
pub struct Request {
    generation: u64,
    name: Option<String>,
    abort: Option<futures_util::future::AbortHandle>,
}
impl Request {
    pub fn start(&mut self, name: String) -> u64 {
        self.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.name = Some(name);
        self.generation
    }
    pub fn pending_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn matches(&self, name: &str, generation: u64) -> bool {
        self.generation == generation && self.pending_name() == Some(name)
    }
    pub fn attach(&mut self, generation: u64, handle: futures_util::future::AbortHandle) {
        if self.generation == generation && self.name.is_some() {
            self.abort = Some(handle);
        } else {
            handle.abort();
        }
    }
    pub fn cancel(&mut self) {
        self.name = None;
        if let Some(handle) = self.abort.take() {
            handle.abort();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn superseding_and_cache_completion_cancel_previous_work() {
        let mut request = Request::default();
        let first = request.start("A".into());
        let (old, _registration) = futures_util::future::AbortHandle::new_pair();
        request.attach(first, old.clone());
        let second = request.start("B".into());
        assert!(old.is_aborted());
        let (current, _registration) = futures_util::future::AbortHandle::new_pair();
        request.attach(second, current.clone());
        request.cancel();
        assert!(current.is_aborted());
    }

    #[test]
    fn revisiting_a_frame_does_not_revive_its_previous_request() {
        let mut request = Request::default();
        let first = request.start("A".into());
        request.start("B".into());
        let latest = request.start("A".into());
        assert!(!request.matches("A", first));
        assert!(request.matches("A", latest));
        request.cancel();
        assert!(
            !request.matches("A", latest),
            "late phases and errors cannot overwrite a cache hit or success"
        );
    }
}
