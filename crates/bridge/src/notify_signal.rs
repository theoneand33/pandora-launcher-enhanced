use tokio::sync::watch;

// ponytail: sticky notify via watch — replaces hand-rolled AtomicBool+Semaphore
#[derive(Debug, Clone)]
pub struct NotifySignal {
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl NotifySignal {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }

    pub fn notify(&self) {
        let _ = self.tx.send(true);
    }

    pub fn is_notified(&self) -> bool {
        *self.rx.borrow()
    }

    pub async fn await_notification(&self) {
        if self.is_notified() {
            return;
        }
        let mut rx = self.rx.clone();
        let _ = rx.wait_for(|v| *v).await;
    }
}

#[derive(Debug)]
pub struct KeepAliveNotifySignal(NotifySignal);

impl KeepAliveNotifySignal {
    pub fn new() -> Self {
        Self(NotifySignal::new())
    }

    pub fn notify(self) {
        std::mem::drop(self);
    }

    pub fn create_handle(&self) -> KeepAliveNotifySignalHandle {
        KeepAliveNotifySignalHandle(self.0.clone())
    }
}

impl Drop for KeepAliveNotifySignal {
    fn drop(&mut self) {
        self.0.notify();
    }
}

#[derive(Debug, Clone)]
pub struct KeepAliveNotifySignalHandle(NotifySignal);

impl KeepAliveNotifySignalHandle {
    pub async fn await_notification(&self) {
        self.0.await_notification().await
    }

    pub fn is_notified(&self) -> bool {
        self.0.is_notified()
    }
}
