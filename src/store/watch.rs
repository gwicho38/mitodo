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

/// Watch `root` and invoke `on_change` when watched files actually differ.
///
/// Runs until the callback returns `false`. Blocking, so callers put it on a
/// dedicated thread.
pub fn watch_blocking<F>(root: &Path, files: Vec<PathBuf>, mut on_change: F)
where
    F: FnMut() -> bool,
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
        // Block until something happens, then drain the burst.
        let Ok(first) = rx.recv() else {
            debug!("watcher channel closed");
            return;
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
            if !on_change() {
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
    fn only_write_like_events_are_interesting() {
        use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
        assert!(is_interesting(&EventKind::Create(CreateKind::File)));
        assert!(is_interesting(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_interesting(&EventKind::Remove(RemoveKind::File)));
        assert!(!is_interesting(&EventKind::Access(AccessKind::Read)));
        assert!(!is_interesting(&EventKind::Any));
    }
}
