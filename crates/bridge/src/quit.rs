use std::sync::{Arc, atomic::AtomicBool};

use parking_lot::Mutex;

struct QuitCoordinatorInner {
    ready: [AtomicBool; 2],
    on_quit: Mutex<Option<Box<dyn FnOnce() + Send + Sync>>>,
}

#[derive(Clone)]
pub struct QuitCoordinator {
    index: usize,
    shared: Arc<QuitCoordinatorInner>,
}

impl QuitCoordinator {
    pub fn new(on_quit: Box<dyn FnOnce() + Send + Sync>) -> Self {
        Self {
            index: 0,
            shared: Arc::new(QuitCoordinatorInner {
                ready: [AtomicBool::new(false), AtomicBool::new(false)],
                on_quit: Mutex::new(Some(on_quit)),
            }),
        }
    }

    pub fn fork(&self) -> Self {
        Self {
            index: 1,
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn set_can_quit(&self, can_quit: bool) {
        self.shared.ready[self.index].store(can_quit, std::sync::atomic::Ordering::SeqCst);
        if can_quit && self.shared.ready.iter().all(|ready| ready.load(std::sync::atomic::Ordering::SeqCst)) {
            if let Some(on_quit) = self.shared.on_quit.lock().take() {
                (on_quit)();
            }
        }
    }
}
