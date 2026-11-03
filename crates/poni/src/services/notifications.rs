//! Other apps' notifications for Notifications, through Windows'
//! notification listener (`Windows.UI.Notifications.Management`).
//!
//! Windows lets only an app with a package identity listen (the capability
//! is in installer/msix/AppxManifest.xml), so the download from GitHub
//! offers no Notifications (`AppState::notify_can`). In the Store copy it's
//! on by default: the page's Allow, or switching it back on in Settings'
//! tabs row, asks Windows once (its question needs the UI thread); once
//! it's allowed, the
//! listener's NotificationChanged event brings each new notification on a
//! thread of Windows' own, and its app and words go to the UI thread as
//! `AppEvent::Notify`. Every answer from Windows reaches the UI thread as
//! `NotifyEvent::Access`, which the app hands back to [`Listener::follow`]. Nothing polls and no thread of ours waits, so it
//! costs nothing at rest; nothing is written down
//! (`poni_core::notifications` decides what the notice says).

use crate::events::{AppEvent, EventSink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::state::NotifyAccess;
use poni_core::notifications::Seen;
use windows::core::Result;
use windows::Foundation::{AsyncOperationCompletedHandler, AsyncStatus, EventRegistrationToken, TypedEventHandler};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification, UserNotificationChangedEventArgs, UserNotificationChangedKind};

/// What the listener tells the UI thread.
#[derive(Debug)]
pub enum NotifyEvent {
    /// What Windows allows now (the app passes it to [`Listener::follow`]).
    Access(NotifyAccess),
    /// The apps with notifications in Windows' list when listening began
    /// (id, name), so the page lists them before they send a new one.
    Apps(Vec<(String, String)>),
    /// A new notification.
    Seen(Seen),
}

/// The listener while Notifications is on (None without a package).
pub struct Listener {
    listener: UserNotificationListener,
    token: Option<EventRegistrationToken>,
    /// Without a package identity Windows still lets this copy read its
    /// list, but not hear the event: a thread looks at the list every
    /// `POLL` instead (set to stop it).
    poll: Option<Arc<AtomicBool>>,
    sink: EventSink,
}

const POLL: std::time::Duration = std::time::Duration::from_millis(1500);

impl Listener {
    /// Look at what Windows allows and listen if it lets us; `ask`: the
    /// person just switched Notifications on, so Windows' question comes now
    /// (at a start with it on, the page offers Allow instead).
    /// None, with the reason sent, when this copy can't listen at all.
    pub fn start(sink: EventSink, ask: bool) -> Option<Self> {
        let listener = match UserNotificationListener::Current() {
            Ok(l) => l,
            Err(e) => {
                crate::pwarn!("notifications: no listener: {e}");
                sink.send(AppEvent::Notify(NotifyEvent::Access(NotifyAccess::Failed)));
                return None;
            }
        };
        let mut me = Self { listener, token: None, poll: None, sink };
        match me.access() {
            NotifyAccess::Ask if ask => me.ask(),
            a => me.tell(a),
        }
        Some(me)
    }

    /// What Windows says now (`Ask` for "not asked yet").
    fn access(&self) -> NotifyAccess {
        match self.listener.GetAccessStatus() {
            Ok(s) => access_of(s, false),
            Err(e) => {
                crate::pwarn!("notifications: access unreadable: {e}");
                NotifyAccess::Failed
            }
        }
    }

    /// Ask Windows (its question). On the UI thread: Windows wants it there.
    /// The answer comes back as `NotifyEvent::Access`.
    pub fn ask(&mut self) {
        let op = match self.listener.RequestAccessAsync() {
            Ok(op) => op,
            Err(e) => {
                crate::pwarn!("notifications: couldn't ask Windows: {e}");
                self.tell(NotifyAccess::Failed);
                return;
            }
        };
        self.sink.send(AppEvent::Notify(NotifyEvent::Access(NotifyAccess::Asking)));
        let sink = self.sink.clone();
        let done = op.SetCompleted(&AsyncOperationCompletedHandler::new(move |op, status| {
            let access = match (status, op.map(|o| o.GetResults())) {
                (AsyncStatus::Completed, Some(Ok(s))) => access_of(s, true),
                _ => NotifyAccess::Failed,
            };
            crate::pinfo!("notifications: Windows answered {access:?}");
            sink.send(AppEvent::Notify(NotifyEvent::Access(access)));
            Ok(())
        }));
        if let Err(e) = done {
            crate::pwarn!("notifications: no answer will come: {e}");
            self.tell(NotifyAccess::Failed);
        }
    }

    /// Look again (the page opened): the person may have changed it in
    /// Windows' settings meanwhile.
    pub fn check(&mut self) {
        let a = self.access();
        self.tell(a);
    }

    /// Pass what Windows allows to the UI thread, which hands it to `follow`.
    fn tell(&self, access: NotifyAccess) {
        self.sink.send(AppEvent::Notify(NotifyEvent::Access(access)));
    }

    /// Do what Windows allows (the app calls it for each `NotifyEvent::Access`):
    /// listen once it's allowed, stop if it no longer is. Sends nothing back
    /// unless listening fails.
    pub fn follow(&mut self, access: NotifyAccess) {
        match access {
            NotifyAccess::Allowed => self.listen(),
            NotifyAccess::Asking => {}
            _ => self.stop(),
        }
    }

    /// Start hearing new notifications, and list the apps already in
    /// Windows' list for the page (once, on a thread of its own).
    fn listen(&mut self) {
        if self.token.is_some() || self.poll.is_some() {
            return;
        }
        let sink = self.sink.clone();
        let handler = TypedEventHandler::<UserNotificationListener, UserNotificationChangedEventArgs>::new(move |sender, args| {
            let (Some(listener), Some(args)) = (sender.as_ref(), args.as_ref()) else { return Ok(()) };
            if args.ChangeKind()? != UserNotificationChangedKind::Added {
                return Ok(());
            }
            // One that can't be read (gone already, no words) is skipped.
            match listener.GetNotification(args.UserNotificationId()?).and_then(|n| read(&n)) {
                Ok(seen) => sink.send(AppEvent::Notify(NotifyEvent::Seen(seen))),
                Err(e) => crate::pinfo!("notifications: one skipped: {e}"),
            }
            Ok(())
        });
        match self.listener.NotificationChanged(&handler) {
            Ok(token) => {
                crate::pinfo!("notifications: listening");
                self.token = Some(token);
            }
            Err(e) if crate::autostart::packaged() => {
                crate::pwarn!("notifications: couldn't listen: {e}");
                self.tell(NotifyAccess::Failed);
                return;
            }
            // No package identity: no event, so the list is looked at.
            Err(_) => {
                if !self.start_poll() {
                    self.tell(NotifyAccess::Failed);
                    return;
                }
            }
        }
        let (listener, sink) = (self.listener.clone(), self.sink.clone());
        let spawned = std::thread::Builder::new().name("poni-notify-apps".into()).spawn(move || {
            let listener = UserNotificationListener::Current().unwrap_or(listener);
            // `.get()` waits, so it runs here, never on the UI thread.
            let apps = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.get()).and_then(|list| {
                let mut apps: Vec<(String, String)> = Vec::new();
                for i in 0..list.Size()? {
                    if let Ok((id, name)) = list.GetAt(i).and_then(|n| app_of(&n)) {
                        if !apps.iter().any(|a| a.0 == id) {
                            apps.push((id, name));
                        }
                    }
                }
                Ok(apps)
            });
            match apps {
                Ok(apps) => sink.send(AppEvent::Notify(NotifyEvent::Apps(apps))),
                Err(e) => crate::pinfo!("notifications: Windows' list unreadable: {e}"),
            }
        });
        if let Err(e) = spawned {
            crate::pwarn!("notifications: no thread to list the apps on: {e}");
        }
    }

    /// Look at Windows' list every `POLL` and pass on the notifications that
    /// weren't in it the last time (the ones there at the start are old).
    fn start_poll(&mut self) -> bool {
        let stop = Arc::new(AtomicBool::new(false));
        let (sink, quit) = (self.sink.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name("poni-notify-poll".into()).spawn(move || {
            // The UI thread's listener can't be used here (not agile without
            // a package): this thread takes its own.
            let Ok(listener) = UserNotificationListener::Current() else {
                crate::pwarn!("notifications: no listener on the thread that looks");
                return;
            };
            let ids = |list: &windows::Foundation::Collections::IVectorView<UserNotification>| -> Vec<(u32, UserNotification)> {
                (0..list.Size().unwrap_or(0)).filter_map(|i| list.GetAt(i).ok()).filter_map(|n| n.Id().ok().map(|id| (id, n))).collect()
            };
            let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
            let mut first = true;
            while !quit.load(Ordering::Relaxed) {
                if let Ok(list) = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.get()) {
                    let now = ids(&list);
                    for (id, n) in &now {
                        if !seen.contains(id) && !first {
                            match read(n) {
                                Ok(s) => {
                                    crate::pinfo!("notifications: one from {}", s.app);
                                    sink.send(AppEvent::Notify(NotifyEvent::Seen(s)))
                                }
                                Err(e) => crate::pinfo!("notifications: one skipped: {e}"),
                            }
                        }
                    }
                    seen = now.into_iter().map(|(id, _)| id).collect();
                    first = false;
                }
                std::thread::sleep(POLL);
            }
        });
        match spawned {
            Ok(_) => {
                crate::pinfo!("notifications: listening (looking every {} ms: no package identity)", POLL.as_millis());
                self.poll = Some(stop);
                true
            }
            Err(e) => {
                crate::pwarn!("notifications: no thread to look on: {e}");
                false
            }
        }
    }

    /// Stop hearing them (Notifications off, or Windows no longer allows it).
    pub fn stop(&mut self) {
        if let Some(p) = self.poll.take() {
            p.store(true, Ordering::Relaxed);
            crate::pinfo!("notifications: stopped looking");
        }
        if let Some(token) = self.token.take() {
            if let Err(e) = self.listener.RemoveNotificationChanged(token) {
                crate::pwarn!("notifications: stopping failed: {e}");
            }
            crate::pinfo!("notifications: stopped listening");
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Windows' status as the page shows it; `asked`: the answer to our
/// question, where "unspecified" means it went unanswered.
fn access_of(s: UserNotificationListenerAccessStatus, asked: bool) -> NotifyAccess {
    match s {
        UserNotificationListenerAccessStatus::Allowed => NotifyAccess::Allowed,
        UserNotificationListenerAccessStatus::Denied => NotifyAccess::Denied,
        _ if asked => NotifyAccess::Unanswered,
        _ => NotifyAccess::Ask,
    }
}

/// The app a notification came from: its AppUserModelID and name.
fn app_of(n: &UserNotification) -> Result<(String, String)> {
    let info = n.AppInfo()?;
    let id = info.AppUserModelId()?.to_string();
    let name = info.DisplayInfo().and_then(|d| d.DisplayName()).map(|h| h.to_string()).unwrap_or_default();
    Ok((id, name))
}

/// A notification's app and its lines of text (a toast's text elements).
fn read(n: &UserNotification) -> Result<Seen> {
    let (app_id, app) = app_of(n)?;
    let binding = n.Notification()?.Visual()?.GetBinding(&KnownNotificationBindings::ToastGeneric()?)?;
    let texts = binding.GetTextElements()?;
    let mut lines = Vec::new();
    for i in 0..texts.Size()? {
        if let Ok(t) = texts.GetAt(i).and_then(|t| t.Text()) {
            lines.push(t.to_string());
        }
    }
    Ok(Seen { app_id, app, lines })
}

/// For the self-test: what this copy can do, without asking Windows anything.
/// Err when a packaged copy can't reach the listener.
pub fn describe() -> std::result::Result<String, String> {
    let status = UserNotificationListener::Current().and_then(|l| l.GetAccessStatus()).map_err(|e| format!("packaged: listener unavailable: {e}"))?;
    Ok(format!("packaged: Windows says {:?}", access_of(status, false)))
}
