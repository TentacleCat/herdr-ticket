use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

pub struct TicketWatcher {
    _watcher: RecommendedWatcher,
    events: Receiver<notify::Result<Event>>,
    tickets_dir: PathBuf,
    watching_tickets: bool,
}

impl TicketWatcher {
    pub fn new(tickets_dir: &Path) -> notify::Result<Self> {
        let tickets_dir = absolute_path(tickets_dir);
        let watching_tickets = tickets_dir.is_dir();
        let watch_path = if watching_tickets {
            tickets_dir.clone()
        } else {
            tickets_dir
                .parent()
                .filter(|parent| parent.is_dir())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        };

        let (sender, events) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            Config::default(),
        )?;
        watcher.watch(&watch_path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            events,
            tickets_dir,
            watching_tickets,
        })
    }

    /// Consume queued filesystem events and report whether ticket data changed.
    /// The watcher only subscribes to `.tickets/` (or its parent until the
    /// directory is created), so ordinary source-control activity is ignored.
    pub fn tickets_changed(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.events.try_recv() {
                Ok(Ok(event)) => {
                    if self.watching_tickets {
                        changed |= event.paths.iter().any(|path| {
                            path.parent() == Some(self.tickets_dir.as_path())
                                && path.extension().is_some_and(|extension| extension == "md")
                        });
                    } else if event.paths.iter().any(|path| path == &self.tickets_dir)
                        && self.tickets_dir.is_dir()
                    {
                        if self
                            ._watcher
                            .watch(&self.tickets_dir, RecursiveMode::NonRecursive)
                            .is_ok()
                        {
                            self.watching_tickets = true;
                            changed = true;
                        }
                    }
                }
                Ok(Err(_)) => changed = true,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        changed
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
