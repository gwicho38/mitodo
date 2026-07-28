use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use log::{debug, warn};
use notify::{EventKind, RecursiveMode, Watcher};
use sha2::{Digest, Sha256};

/// How long to wait for a burst of filesystem events to settle.
///
/// Editors and the atomic temp-file-plus-rename used by `store::write` both
/// produce several events per logical change.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Content digest of a file, or `None` if it cannot be read.
fn digest(path: &Path) -> Option<[u8; 32]> {
    let bytes = std::fs::read(path).ok()?;
    Some(Sha256::digest(&bytes).into())
}

/// Snapshot the digests of every watched todo file.
pub fn digests(files: &[PathBuf]) -> HashMap<PathBuf, [u8; 32]> {
    files
        .iter()
        .filter_map(|f| digest(f).map(|d| (f.clone(), d)))
        .collect()
}

/// True if any watched file's content differs from the snapshot.
///
/// This suppresses the events that carry no content change at all — editors
/// touching mtimes, and the temp-file half of `store::write`'s atomic rename.
/// It cannot tell mitodo's own writes from anyone else's, because the snapshot
/// lives on this thread; `App::reload` compares workspace fingerprints to make
/// that distinction.
pub fn any_changed(files: &[PathBuf], known: &HashMap<PathBuf, [u8; 32]>) -> bool {
    for file in files {
        match (digest(file), known.get(file)) {
            (Some(current), Some(previous)) if &current == previous => {}
            // Appeared, vanished, or differs.
            _ => return true,
        }
    }
    // A file that was known but is no longer listed also counts as a change.
    known.keys().any(|k| !files.contains(k))
}

/// How long to wait before checking whether the consumer is still there.
///
/// Without this the loop would park in `recv` forever, and a watcher that
/// cannot notice its consumer has gone keeps the process alive after quit.
const LIVENESS_POLL: Duration = Duration::from_millis(500);

/// Watch `root` and call `notify` when watched files actually differ.
///
/// `notify(true)` means "the files changed"; `notify(false)` is a periodic
/// liveness check that sends nothing. Either returning `false` stops the watch.
/// Blocking, so callers put it on a dedicated thread.
pub fn watch_blocking<F>(root: &Path, files: Vec<PathBuf>, mut notify: F)
where
    F: FnMut(bool) -> bool,
{
    let (tx, rx) = std_mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    }) {
        Ok(w) => w,
        Err(err) => {
            warn!("could not create a filesystem watcher: {err}");
            return;
        }
    };

    if let Err(err) = watcher.watch(root, RecursiveMode::Recursive) {
        warn!("could not watch {}: {err}", root.display());
        return;
    }

    let mut known = digests(&files);

    loop {
        // Wake periodically even when nothing happens, so a quit is noticed.
        let first = match rx.recv_timeout(LIVENESS_POLL) {
            Ok(event) => event,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                if notify(false) {
                    continue;
                }
                debug!("watch consumer is gone; stopping");
                return;
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                debug!("watcher channel closed");
                return;
            }
        };

        let mut interesting = is_interesting(&first.kind);
        while let Ok(event) = rx.recv_timeout(DEBOUNCE) {
            interesting |= is_interesting(&event.kind);
        }
        if !interesting {
            continue;
        }

        if any_changed(&files, &known) {
            known = digests(&files);
            if !notify(true) {
                return;
            }
        }
    }
}

fn is_interesting(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_file_is_not_reported_as_changed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("TODO.md");
        std::fs::write(&file, "- [ ] a\n").unwrap();

        let files = vec![file.clone()];
        let known = digests(&files);
        assert!(!any_changed(&files, &known));
    }

    #[test]
    fn rewriting_identical_content_is_not_a_change() {
        // This is what makes the gate work: mitodo's own atomic rename
        // produces events, but the bytes are what they were expected to be.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("TODO.md");
        std::fs::write(&file, "- [ ] a\n").unwrap();

        let files = vec![file.clone()];
        let known = digests(&files);
        std::fs::write(&file, "- [ ] a\n").unwrap();
        assert!(!any_changed(&files, &known), "same bytes, no change");
    }

    #[test]
    fn different_content_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("TODO.md");
        std::fs::write(&file, "- [ ] a\n").unwrap();

        let files = vec![file.clone()];
        let known = digests(&files);
        std::fs::write(&file, "- [x] a\n").unwrap();
        assert!(any_changed(&files, &known));
    }

    #[test]
    fn a_deleted_file_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("TODO.md");
        std::fs::write(&file, "- [ ] a\n").unwrap();

        let files = vec![file.clone()];
        let known = digests(&files);
        std::fs::remove_file(&file).unwrap();
        assert!(any_changed(&files, &known));
    }

    #[test]
    fn a_new_file_is_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.md");
        std::fs::write(&first, "- [ ] a\n").unwrap();
        let known = digests(std::slice::from_ref(&first));

        let second = dir.path().join("b.md");
        std::fs::write(&second, "- [ ] b\n").unwrap();
        assert!(
            any_changed(&[first, second], &known),
            "a file with no known digest counts as changed"
        );
    }

    #[test]
    fn the_watch_stops_when_its_consumer_goes_away() {
        // Regression: the loop used to park in recv() forever, so quitting the
        // UI left the process alive with nothing to notify.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("TODO.md");
        std::fs::write(&file, "- [ ] a\n").unwrap();

        let polls = Arc::new(AtomicUsize::new(0));
        let seen = polls.clone();
        let root = dir.path().to_path_buf();
        let files = vec![file];

        let handle = std::thread::spawn(move || {
            watch_blocking(&root, files, move |changed| {
                if !changed {
                    // Report "consumer gone" on the second liveness poll.
                    return seen.fetch_add(1, Ordering::SeqCst) < 1;
                }
                true
            });
        });

        handle
            .join()
            .expect("watch returns rather than parking forever");
        assert!(polls.load(Ordering::SeqCst) >= 1, "liveness was polled");
    }

    #[test]
    fn only_write_like_events_are_interesting() {
        use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
        assert!(is_interesting(&EventKind::Create(CreateKind::File)));
        assert!(is_interesting(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_interesting(&EventKind::Remove(RemoveKind::File)));
        assert!(!is_interesting(&EventKind::Access(AccessKind::Read)));
        assert!(!is_interesting(&EventKind::Any));
    }
}
