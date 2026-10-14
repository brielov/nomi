//! The scheduler and the artifact cache together: a build of toy modules
//! whose interface is the part of their source before `|`, cut off early
//! where an interface comes out the same.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use nomi_engine::Vfs;
use nomi_engine::artifacts::{Artifacts, Hash, Key};
use nomi_engine::paths::Cache;
use nomi_engine::schedule::{Cancelled, Outcome, run, run_gated};

/// What building a toy module makes: its interface's hash.
type Made = Hash;

/// Builds `sources`, where module `i` imports `imports[i]`, through the
/// cache; returns the outcomes and which modules were compiled rather
/// than read back.
fn build(
    cache: &Path,
    sources: &[&str],
    imports: &[Vec<usize>],
) -> (Vec<Outcome<Made, String>>, Vec<usize>) {
    let artifacts = Artifacts::new(&Cache::new(cache.to_owned()));
    let compiled = Mutex::new(Vec::new());
    let outcomes = run(None, imports, |i, made: &[&Made]| {
        let source = sources[i];
        if source.starts_with('!') {
            return Err(format!("module {i} fails"));
        }
        let key = made
            .iter()
            .fold(Key::new("toy").part("source", source.as_bytes()), |k, h| {
                k.hash("import", **h)
            })
            .finish();
        if let Some(bytes) = artifacts.get(key) {
            return Ok(Hash(bytes.try_into().unwrap()));
        }
        compiled.lock().unwrap().push(i);
        let interface = Hash::of(source.split('|').next().unwrap().as_bytes());
        artifacts.put(key, &interface.0).unwrap();
        Ok(interface)
    })
    .unwrap();
    let mut compiled = compiled.into_inner().unwrap();
    compiled.sort_unstable();
    (outcomes, compiled)
}

#[test]
fn unchanged_interfaces_cut_rebuilds_off() {
    let dir = tempfile::tempdir().unwrap();
    // 0 <- 1 <- 2, and 3 imports 0 and 2.
    let imports = vec![vec![], vec![0], vec![1], vec![0, 2]];
    let (_, compiled) = build(dir.path(), &["a|1", "b|1", "c|1", "d|1"], &imports);
    assert_eq!(compiled, [0, 1, 2, 3]);
    let (_, compiled) = build(dir.path(), &["a|1", "b|1", "c|1", "d|1"], &imports);
    assert!(compiled.is_empty(), "{compiled:?}");
    // A body changes: only that module compiles.
    let (_, compiled) = build(dir.path(), &["a|2", "b|1", "c|1", "d|1"], &imports);
    assert_eq!(compiled, [0]);
    // An interface changes: its importers compile, and theirs stop where
    // interfaces hold.
    let (_, compiled) = build(dir.path(), &["A|2", "b|1", "c|1", "d|1"], &imports);
    assert_eq!(compiled, [0, 1, 3]);
}

#[test]
fn failures_block_what_imports_them() {
    let dir = tempfile::tempdir().unwrap();
    let imports = vec![vec![], vec![0], vec![1], vec![], vec![3, 0]];
    let (outcomes, _) = build(dir.path(), &["a", "!b", "c", "d", "e"], &imports);
    assert!(matches!(outcomes[0], Outcome::Done(_)));
    assert_eq!(outcomes[1], Outcome::Failed("module 1 fails".into()));
    assert_eq!(outcomes[2], Outcome::Blocked);
    assert!(matches!(outcomes[3], Outcome::Done(_)));
    assert!(matches!(outcomes[4], Outcome::Done(_)));
}

#[test]
fn modules_start_once_their_imports_finish() {
    // A wide, deep DAG: each module imports a few before it.
    let n = 2000;
    let imports: Vec<Vec<usize>> = (0..n)
        .map(|i: usize| {
            [i / 2, i / 3, i.saturating_sub(1)]
                .into_iter()
                .filter(|&d| d < i)
                .collect()
        })
        .collect();
    let order = AtomicUsize::new(0);
    let outcomes = run(None, &imports, |i, made: &[&usize]| {
        assert_eq!(made.len(), imports[i].len());
        Ok::<_, ()>(order.fetch_add(1, Ordering::SeqCst))
    })
    .unwrap();
    for (i, deps) in imports.iter().enumerate() {
        let Outcome::Done(at) = outcomes[i] else {
            panic!()
        };
        for &d in deps {
            let Outcome::Done(before) = outcomes[d] else {
                panic!()
            };
            assert!(before < at, "{d} before {i}");
        }
    }
}

#[test]
fn a_stale_snapshot_stops_the_run() {
    let vfs = Vfs::new();
    let snapshot = vfs.snapshot();
    let imports = vec![vec![], vec![0], vec![1]];
    let ran = AtomicUsize::new(0);
    let result = run(Some(&snapshot), &imports, |i, _: &[&()]| {
        ran.fetch_add(1, Ordering::SeqCst);
        if i == 0 {
            vfs.disk_changed();
        }
        Ok::<_, ()>(())
    });
    assert_eq!(result, Err(Cancelled));
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert!(
        run(Some(&vfs.snapshot()), &imports, |_, _: &[&()]| Ok::<_, ()>(
            ()
        ))
        .is_ok()
    );
}

#[test]
fn a_run_that_ends_stale_is_cancelled() {
    let vfs = Vfs::new();
    let snapshot = vfs.snapshot();
    let result = run(Some(&snapshot), &[vec![]], |_, _: &[&()]| {
        vfs.disk_changed();
        Ok::<_, ()>(())
    });
    assert_eq!(result, Err(Cancelled));
}

#[test]
fn modules_wait_on_their_gates() {
    // 1 imports 0; 0 waits on gate 0, 2 on gate 1, which never opens, and
    // 3 on nothing.
    let imports = vec![vec![], vec![0], vec![], vec![]];
    let gates = vec![vec![0], vec![], vec![1], vec![]];
    let opened = Mutex::new(false);
    let order = Mutex::new(Vec::new());
    let outcomes = run_gated(
        None,
        &imports,
        &gates,
        |open| {
            *opened.lock().unwrap() = true;
            open(0);
        },
        |i, _: &[&()]| {
            assert!(i == 3 || *opened.lock().unwrap(), "{i} ran before its gate");
            order.lock().unwrap().push(i);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    assert_eq!(outcomes[0], Outcome::Done(()));
    assert_eq!(outcomes[1], Outcome::Done(()));
    assert_eq!(outcomes[2], Outcome::Blocked);
    assert_eq!(outcomes[3], Outcome::Done(()));
    let order = order.into_inner().unwrap();
    assert!(order.iter().position(|&i| i == 0) < order.iter().position(|&i| i == 1));
}
