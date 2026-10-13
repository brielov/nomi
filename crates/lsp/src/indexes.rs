//! The latest index of each project, shared by the requests and the
//! diagnostics worker, so that one index serves both and each is built
//! from the one before it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use nomi_engine::index::{self, Index, Memo};
use nomi_engine::{Revision, Snapshot, Store};

#[derive(Default)]
pub(crate) struct Indexes {
    /// By manifest, the latest index and the revision it was made at.
    latest: Mutex<HashMap<PathBuf, (Revision, Arc<Index>)>>,
    /// Held while one is built: whoever wants the next waits for it
    /// rather than build it twice.
    building: Mutex<()>,
}

impl Indexes {
    /// The index of the project that owns `path`, as of `snapshot`: the
    /// latest where it's as new, or else one made from it, with the
    /// packages `memo` has built.
    pub(crate) fn get(
        &self,
        snapshot: &Snapshot,
        store: &Store,
        memo: &mut Memo,
        path: &Path,
    ) -> Option<Arc<Index>> {
        let manifest = index::manifest(snapshot, path)?;
        let revision = snapshot.revision();
        let _building = self.building.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = {
            let latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
            match latest.get(&manifest) {
                Some((r, index)) if *r == revision => return Some(index.clone()),
                Some((_, index)) => Some(index.clone()),
                None => None,
            }
        };
        let index = index::index_from(snapshot, store, path, memo, previous.as_deref())?;
        let index = Arc::new(index);
        (self.latest.lock().unwrap_or_else(PoisonError::into_inner))
            .insert(manifest, (revision, index.clone()));
        Some(index)
    }
}
