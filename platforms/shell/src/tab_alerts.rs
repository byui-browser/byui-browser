//! Tab/document-scoped alert delivery. Background alerts wait for activation;
//! navigation, tab closure and window shutdown release blocked scripts.

use crate::app::UserEvent;
use chrome::TabId;
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};
use webapis::AlertSink;
use winit::{event_loop::EventLoopProxy, window::Window};

struct Pending {
    tab: TabId,
    document: u64,
    message: String,
    done: mpsc::Sender<()>,
}
#[derive(Default)]
struct Queue {
    documents: HashMap<TabId, u64>,
    pending: Vec<Pending>,
    closed: bool,
}

pub(crate) struct PageAlerts {
    proxy: Mutex<Option<EventLoopProxy<UserEvent>>>,
    queue: Mutex<Queue>,
}
impl PageAlerts {
    pub fn new() -> Self {
        Self {
            proxy: Mutex::new(None),
            queue: Mutex::new(Queue::default()),
        }
    }
    pub fn set_proxy(&self, proxy: EventLoopProxy<UserEvent>) {
        *self.proxy.lock().expect("alert proxy lock") = Some(proxy);
    }
    pub fn set_document(&self, tab: TabId, document: u64) {
        let mut q = self.queue.lock().expect("alert queue lock");
        q.documents.insert(tab, document);
        q.pending.retain(|p| {
            let keep = p.tab != tab || p.document == document;
            if !keep {
                let _ = p.done.send(());
            }
            keep
        });
    }
    pub fn close_tab(&self, tab: TabId) {
        let mut q = self.queue.lock().expect("alert queue lock");
        q.documents.remove(&tab);
        q.pending.retain(|p| {
            if p.tab == tab {
                let _ = p.done.send(());
                false
            } else {
                true
            }
        });
    }
    pub fn shutdown(&self) {
        let mut q = self.queue.lock().expect("alert queue lock");
        q.closed = true;
        q.documents.clear();
        for p in q.pending.drain(..) {
            let _ = p.done.send(());
        }
    }
    pub fn sink(self: &Arc<Self>, tab: TabId, document: u64) -> Arc<dyn AlertSink> {
        Arc::new(TabAlerts {
            owner: Arc::clone(self),
            tab,
            document,
        })
    }
    fn push(&self, tab: TabId, document: u64, message: &str) -> Option<mpsc::Receiver<()>> {
        let mut q = self.queue.lock().expect("alert queue lock");
        if q.closed || q.documents.get(&tab) != Some(&document) {
            return None;
        }
        let (done, receiver) = mpsc::channel();
        q.pending.push(Pending {
            tab,
            document,
            message: message.to_owned(),
            done,
        });
        Some(receiver)
    }
    fn take_for(&self, active: TabId) -> Vec<Pending> {
        let mut q = self.queue.lock().expect("alert queue lock");
        let pending = std::mem::take(&mut q.pending);
        let mut show = Vec::new();
        for p in pending {
            if q.documents.get(&p.tab) != Some(&p.document) {
                let _ = p.done.send(());
            } else if p.tab == active {
                show.push(p);
            } else {
                q.pending.push(p);
            }
        }
        show
    }
    /// Native dialogs remain parented to the live browser window.
    pub fn present(&self, parent: &Window, active: TabId) {
        for p in self.take_for(active) {
            MessageDialog::new()
                .set_title("BYUI Browser")
                .set_description(p.message)
                .set_level(MessageLevel::Info)
                .set_buttons(MessageButtons::Ok)
                .set_parent(parent)
                .show();
            let _ = p.done.send(());
        }
    }
}
struct TabAlerts {
    owner: Arc<PageAlerts>,
    tab: TabId,
    document: u64,
}
impl AlertSink for TabAlerts {
    fn alert(&self, message: &str) {
        let proxy = self.owner.proxy.lock().expect("alert proxy lock").clone();
        let Some(proxy) = proxy else {
            return;
        };
        let Some(receiver) = self.owner.push(self.tab, self.document, message) else {
            return;
        };
        if proxy.send_event(UserEvent::AlertPending).is_ok() {
            let _ = receiver.recv();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_alerts_wait_for_their_tab_and_keep_their_identity() {
        let alerts = PageAlerts::new();
        let a = TabId(1);
        let b = TabId(2);
        alerts.set_document(a, 10);
        alerts.set_document(b, 20);
        let first = alerts.push(a, 10, "first").unwrap();
        let second = alerts.push(b, 20, "second").unwrap();
        let active = alerts.take_for(a);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].message, "first");
        assert!(second.try_recv().is_err());
        let _ = active[0].done.send(());
        first.recv().unwrap();
        let background = alerts.take_for(b);
        assert_eq!(background.len(), 1);
        assert_eq!(background[0].message, "second");
        let _ = background[0].done.send(());
        second.recv().unwrap();
    }
    #[test]
    fn replacing_closing_and_shutdown_release_scripts_and_reject_late_alerts() {
        let alerts = PageAlerts::new();
        let tab = TabId(1);
        alerts.set_document(tab, 1);
        let old = alerts.push(tab, 1, "old document").unwrap();
        alerts.set_document(tab, 2);
        old.recv().unwrap();
        assert!(alerts.push(tab, 1, "stale").is_none());
        let current = alerts.push(tab, 2, "closing").unwrap();
        alerts.close_tab(tab);
        current.recv().unwrap();
        assert!(alerts.push(tab, 2, "closed").is_none());
        alerts.set_document(tab, 3);
        let pending = alerts.push(tab, 3, "shutdown").unwrap();
        alerts.shutdown();
        pending.recv().unwrap();
        assert!(alerts.push(tab, 3, "late").is_none());
    }
}
