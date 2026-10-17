//! `nomi run --watch` of several entries: each built and run or served
//! as `nomi run` does, then, on each change, the members it touches built
//! again with their dependents, through one engine, each Node program
//! started again and each page reloaded only where its code changed, and
//! a page reloaded without a build when its static files change. A failed
//! build leaves a Node program stopped and a page on its old code, and
//! the others run on.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use nomi_base::{Compat, Platform, lock};
use nomi_project::Entry;
use nomi_project::workspace::{Member, Workspace};
use nomi_registry::Http;

use super::{Running, label_of, prefixes_of, served, serving, unrunnable};
use crate::Context;
use crate::artifacts::Hash;
use crate::console::{self, Console};
use crate::make::Shared;
use crate::paths::{ELM_JSON, NOMI_TOML};
use crate::run::port::{Port, ports};
use crate::run::{Caught, Stderr, Stop, code_hash, counted, exit_code, no_node, scoped};
use crate::serve::{Reloads, Server};
use crate::watch::{Change, POLL, Snapshot, message};
use crate::workspace::{self, EntryBuilt, Making, with_deps};

/// Runs the entries of the members of `workspace` in `dirs` as `nomi
/// run` does, then again as they change, the scope `member` names read
/// again when a manifest does, until nomi is sent SIGINT or SIGTERM; the
/// code to exit with, 130 or 143, or 1 if the watch couldn't start.
pub(in crate::run) fn watch(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    member: Option<&str>,
    workspace: Workspace,
    dirs: Vec<PathBuf>,
) -> u8 {
    let mut caught = match Caught::new() {
        Ok(caught) => caught,
        Err(e) => {
            console.err(&format!("I could not watch for signals: {e}\n"));
            return 1;
        }
    };
    let root = workspace.root().to_owned();
    let shared = Shared::new(cx.cache, Some(&root));
    let mut snapshot = Snapshot::new(&root, vec![root.join(NOMI_TOML)]);
    let mut watch = Watch {
        workspace,
        dirs: Vec::new(),
        slots: Vec::new(),
        running: Running::new(Vec::new()),
    };
    let mut engine = Engine {
        cx,
        shared: &shared,
        caught: &mut caught,
    };
    let mut caught_now = watch.read(&mut engine, console, dirs);
    loop {
        if let Some((signal, sent)) = caught_now {
            return (watch.running).drain(
                console,
                engine.caught,
                sent.then_some(signal),
                signal.code(),
            );
        }
        snapshot.watch(watch.paths(), watch.statics());
        console.err("[nomi] Watching for changes...\n");
        let queue = match watch.wait(&mut engine, console, &mut snapshot) {
            Ok(queue) => queue,
            Err(signal) => {
                caught_now = Some(signal);
                continue;
            }
        };
        console.err(&format!("[nomi] {}\n", message(&queue)));
        cx.vfs.disk_changed();
        caught_now = watch.changed(&mut engine, console, member, &queue);
    }
}

/// What builds each time: the files, the cache, and what the builds
/// before left; and the signals nomi is sent meanwhile.
struct Engine<'a, 'c, H> {
    cx: &'a Context<'c, H>,
    shared: &'a Shared<'a>,
    caught: &'a mut Caught,
}

/// The workspace watched, and what of it runs.
struct Watch {
    workspace: Workspace,
    /// The members whose entries run, by their directories.
    dirs: Vec<PathBuf>,
    /// Their entries, in the members' order.
    slots: Vec<Slot>,
    running: Running,
}

/// An entry watched.
struct Slot {
    /// The directory of its member, from the root.
    member: PathBuf,
    entry: Entry,
    /// Its name in nomi's lines, and its programs' prefix.
    label: String,
    prefix: String,
    /// Where it is served if it's a browser program.
    port: Option<Port>,
    /// Where it ran when last built.
    platform: Option<Platform>,
    /// Its code, absolute, once built.
    output: Option<PathBuf>,
    /// The hash of the code it runs or serves.
    code: Option<Hash>,
    live: Live,
}

/// What of an entry runs.
enum Live {
    /// Nothing: it hasn't built, its build failed, or its program ended.
    Idle,
    /// Its Node program, by its place among the programs.
    Program(usize),
    /// Its page.
    Served(Server),
}

/// A member's code, absolute.
struct Sources {
    /// Its source directories.
    dirs: Vec<PathBuf>,
    /// Its entries' mains and FFI modules.
    files: Vec<PathBuf>,
}

impl Sources {
    /// The code of `member` of the workspace at `root`.
    fn of(root: &Path, member: &Member) -> Sources {
        let dir = root.join(&member.dir);
        let entries = member.manifest.iter().flat_map(|m| &m.entries);
        Sources {
            dirs: (member.outline.source_dirs().iter())
                .map(|d| dir.join(d))
                .collect(),
            files: entries
                .flat_map(|e| [Some(&e.main), e.ffi.as_ref()].into_iter().flatten())
                .map(|path| dir.join(path))
                .collect(),
        }
    }

    /// Whether `path` is among them.
    fn contain(&self, path: &Path) -> bool {
        self.dirs.iter().any(|dir| path.starts_with(dir)) || self.files.iter().any(|f| f == path)
    }
}

/// What a build makes an entry do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    /// Run on as it is.
    Keep,
    /// Stop its program, as its build failed.
    Stop,
    /// Stop what runs, and start its Node program.
    Run,
    /// Stop what runs, and serve its page.
    Serve,
    /// Tell its page to reload.
    Reload,
}

/// What an entry that runs as `live`, the code whose hash is `was`, does
/// once built as `now`: where and the hash of its code, or `None` where
/// its build failed.
fn next(live: &Live, was: Option<Hash>, now: Option<(Platform, Hash)>) -> Next {
    let same = |hash| was == Some(hash);
    match (live, now) {
        (Live::Program(_), None) => Next::Stop,
        (Live::Idle | Live::Served(_), None) => Next::Keep,
        (Live::Program(_), Some((Platform::Node, hash))) if same(hash) => Next::Keep,
        (_, Some((Platform::Node, _))) => Next::Run,
        (Live::Served(_), Some((Platform::Browser, hash))) if same(hash) => Next::Keep,
        (Live::Served(_), Some((Platform::Browser, _))) => Next::Reload,
        (Live::Idle | Live::Program(_), Some((Platform::Browser, _))) => Next::Serve,
    }
}

impl Watch {
    /// Takes `dirs` as the members whose entries run, each entry kept
    /// running where it is unchanged, and builds them all.
    fn read(
        &mut self,
        engine: &mut Engine<'_, '_, impl Http + Sync>,
        console: &mut dyn Console,
        dirs: Vec<PathBuf>,
    ) -> Option<(Stop, bool)> {
        let entries: Vec<(&Member, &Entry)> = (self.workspace.members().iter())
            .filter(|member| dirs.contains(&member.dir))
            .flat_map(|member| {
                (member.manifest.iter())
                    .flat_map(move |m| &m.entries)
                    .map(move |e| (member, e))
            })
            .collect();
        let labels: Vec<String> = entries.iter().map(|(m, e)| label_of(m, e)).collect();
        let prefixes = prefixes_of(&labels, console.err_is_terminal());
        let ports = ports(entries.iter().map(|&(_, entry)| entry));
        let mut old = std::mem::take(&mut self.slots);
        let each = entries.into_iter().zip(labels).zip(prefixes).zip(ports);
        for ((((member, entry), label), prefix), port) in each {
            let kept = old
                .iter()
                .position(|s| s.member == member.dir && s.entry == *entry && s.port == port);
            let slot = match kept {
                Some(i) => Slot {
                    label,
                    prefix,
                    ..old.remove(i)
                },
                None => Slot {
                    member: member.dir.clone(),
                    entry: entry.clone(),
                    label,
                    prefix,
                    port,
                    platform: None,
                    output: None,
                    code: None,
                    live: Live::Idle,
                },
            };
            self.slots.push(slot);
        }
        let gone: Vec<usize> = (old.iter())
            .filter_map(|slot| match slot.live {
                Live::Program(i) => Some(i),
                Live::Idle | Live::Served(_) => None,
            })
            .collect();
        drop(old);
        if let Some(signal) =
            (self.running).stop(console, engine.caught, &gone, Some(Stop::Terminate))
        {
            return Some(signal);
        }
        self.dirs = dirs;
        let every = self.dirs.clone();
        self.rebuild(engine, console, &every)
    }

    /// The paths watched besides the root's `nomi.toml`: each member's
    /// manifests and source directories, with those of the member
    /// packages they depend on, and its entries' mains and FFI modules.
    fn paths(&self) -> Vec<PathBuf> {
        let wanted = with_deps(&self.workspace, &self.dirs);
        let root = self.workspace.root();
        let mut paths = Vec::new();
        for member in (self.workspace.members().iter()).filter(|m| wanted.contains(m.dir.as_path()))
        {
            let dir = root.join(&member.dir);
            let sources = Sources::of(root, member);
            paths.extend([dir.join(ELM_JSON), dir.join(NOMI_TOML)]);
            paths.extend(sources.dirs);
            paths.extend(sources.files);
        }
        paths
    }

    /// The directories of the entries' static files.
    fn statics(&self) -> Vec<PathBuf> {
        (self.slots.iter())
            .filter_map(|slot| self.static_dir(slot))
            .collect()
    }

    /// The directory of `slot`'s static files, absolute.
    fn static_dir(&self, slot: &Slot) -> Option<PathBuf> {
        let dir = self.workspace.root().join(&slot.member);
        Some(dir.join(slot.entry.static_dir.as_ref()?))
    }

    /// Shows the programs' lines, and says when one ends, until the files
    /// change; the changes, those made together joined, but not the
    /// entries' code, which builds write; or the signal nomi is sent.
    fn wait(
        &mut self,
        engine: &mut Engine<'_, '_, impl Http + Sync>,
        console: &mut dyn Console,
        snapshot: &mut Snapshot,
    ) -> Result<Vec<Change>, (Stop, bool)> {
        let mut looked = Instant::now();
        loop {
            self.running.show(console);
            if let Some(signal) = engine.caught.take() {
                return Err(signal);
            }
            self.reap(console);
            if looked.elapsed() < POLL {
                continue;
            }
            let mut queue = snapshot.changes();
            looked = Instant::now();
            if queue.is_empty() {
                continue;
            }
            thread::sleep(POLL);
            queue.extend(snapshot.changes());
            queue.retain(|change| {
                !self
                    .slots
                    .iter()
                    .any(|s| s.output.as_deref() == Some(change.path()))
            });
            if !queue.is_empty() {
                return Ok(queue);
            }
        }
    }

    /// Says which programs have ended by themselves since last looked at.
    fn reap(&mut self, console: &mut dyn Console) {
        self.running.reap();
        for slot in &mut self.slots {
            if let Live::Program(i) = slot.live
                && let Some(status) = self.running.programs[i].ended
            {
                let code = exit_code(status);
                console.err(&format!("[nomi] {} exited with code {code}\n", slot.label));
                slot.live = Live::Idle;
            }
        }
    }

    /// Builds what `queue` changed again: every member where a manifest
    /// changed, the scope read again; or else the members whose files
    /// changed and their dependents. Pages whose static files changed
    /// reload. The signal nomi is sent meanwhile, if one is.
    fn changed(
        &mut self,
        engine: &mut Engine<'_, '_, impl Http + Sync>,
        console: &mut dyn Console,
        member: Option<&str>,
        queue: &[Change],
    ) -> Option<(Stop, bool)> {
        for slot in &self.slots {
            if let (Live::Served(server), Some(dir)) = (&slot.live, self.static_dir(slot))
                && queue.iter().any(|change| change.path().starts_with(&dir))
            {
                console.err(&format!("[nomi] Reloading {}\n", slot.label));
                server.reload();
            }
        }
        if queue.iter().any(|change| self.is_manifest(change.path())) {
            return match scoped(engine.cx, member) {
                Ok(Some((workspace, dirs))) => {
                    let dirs = counted(&workspace, &dirs);
                    let dirs = dirs.into_iter().map(|(dir, _)| dir.clone()).collect();
                    self.workspace = workspace;
                    self.read(engine, console, dirs)
                }
                Ok(None) => self.read(engine, console, Vec::new()),
                Err(report) => {
                    console::report(console, &report, Compat::LATEST);
                    None
                }
            };
        }
        let touched = self.touched(queue);
        if touched.is_empty() {
            return None;
        }
        self.rebuild(engine, console, &touched)
    }

    /// Whether `path` is the root's `nomi.toml` or a member's manifest.
    fn is_manifest(&self, path: &Path) -> bool {
        let root = self.workspace.root();
        path == root.join(NOMI_TOML)
            || (self.workspace.members().iter()).any(|member| {
                let dir = root.join(&member.dir);
                path == dir.join(ELM_JSON) || path == dir.join(NOMI_TOML)
            })
    }

    /// The members whose entries run that a change in `queue` touches,
    /// in a member watched or a member package they depend on.
    fn touched(&self, queue: &[Change]) -> Vec<PathBuf> {
        let root = self.workspace.root();
        let mut touched: BTreeSet<&Path> = BTreeSet::new();
        for member in self.workspace.members() {
            let sources = Sources::of(root, member);
            if queue.iter().any(|change| sources.contain(change.path())) {
                touched.insert(&member.dir);
                touched.extend(
                    self.workspace
                        .dependents(member)
                        .into_iter()
                        .map(|m| m.dir.as_path()),
                );
            }
        }
        (self.dirs.iter())
            .filter(|dir| touched.contains(dir.as_path()))
            .cloned()
            .collect()
    }

    /// Builds the members in `dirs` again, with what they depend on, and
    /// starts, stops or reloads each of their entries as [`next`] says;
    /// the signal nomi is sent meanwhile, if one is.
    fn rebuild(
        &mut self,
        engine: &mut Engine<'_, '_, impl Http + Sync>,
        console: &mut dyn Console,
        dirs: &[PathBuf],
    ) -> Option<(Stop, bool)> {
        lock(&engine.shared.members).clear();
        let Watch {
            workspace,
            slots,
            running,
            ..
        } = self;
        let builds = workspace::builds_in(
            engine.cx,
            &mut Stderr(console),
            engine.shared,
            workspace,
            dirs,
            Making::Run,
        );
        let root = workspace.root();
        let built: Vec<Option<(&EntryBuilt<'_>, Hash)>> = (slots.iter())
            .map(|slot| {
                if !dirs.contains(&slot.member) {
                    return None;
                }
                let found = (builds.entries.iter())
                    .find(|b| b.member.dir == slot.member && *b.entry == slot.entry)?;
                if let Some(report) = unrunnable(found, root) {
                    console::report(console, &report, Compat::LATEST);
                    return None;
                }
                Some((found, code_hash(&found.output)))
            })
            .collect();
        let nexts: Vec<Next> = (slots.iter().zip(&built))
            .map(|(slot, built)| {
                if !dirs.contains(&slot.member) {
                    return Next::Keep;
                }
                let now = built.map(|(b, hash)| (b.platform, hash));
                next(&slot.live, slot.code, now)
            })
            .collect();
        let stopping: Vec<usize> = (slots.iter().zip(&nexts))
            .filter_map(|(slot, next)| match (&slot.live, next) {
                (Live::Program(i), Next::Stop | Next::Run | Next::Serve) => Some(*i),
                _ => None,
            })
            .collect();
        if let Some(signal) = running.stop(console, engine.caught, &stopping, Some(Stop::Terminate))
        {
            return Some(signal);
        }
        let ports: Vec<Option<Port>> = slots.iter().map(|slot| slot.port).collect();
        for ((slot, built), next) in slots.iter_mut().zip(built).zip(nexts) {
            if let Some((built, _)) = built {
                slot.platform = Some(built.platform);
                slot.output = Some(built.output.clone());
            }
            apply(console, running, root, slot, built, next, &ports);
        }
        None
    }
}

/// Does what `next` says to `slot`, built as `built`, its code's hash
/// with it, of the workspace at `root`, a page served on its port of
/// `ports`.
fn apply(
    console: &mut dyn Console,
    running: &mut Running,
    root: &Path,
    slot: &mut Slot,
    built: Option<(&EntryBuilt<'_>, Hash)>,
    next: Next,
    ports: &[Option<Port>],
) {
    let label = &slot.label;
    match (next, built) {
        (Next::Keep, _) => {}
        (Next::Stop, _) => {
            console.err(&format!("[nomi] Stopping {label}, as its build failed\n"));
            slot.live = Live::Idle;
        }
        (Next::Run, Some((built, hash))) => {
            let verb = match slot.live {
                Live::Program(_) => "Restarting",
                Live::Idle | Live::Served(_) => "Starting",
            };
            console.err(&format!("[nomi] {verb} {label}\n"));
            slot.live = Live::Idle;
            match running.start(built, slot.prefix.clone()) {
                Ok(program) => {
                    slot.live = Live::Program(program);
                    slot.code = Some(hash);
                }
                Err(e) => console::report(console, &no_node(&e), Compat::LATEST),
            }
        }
        (Next::Serve, Some((built, hash))) => {
            slot.live = Live::Idle;
            let port = slot
                .port
                .expect("a browser program doesn't run on Node by its manifest");
            match served(root, built, port, ports, Reloads::Heard) {
                Ok(server) => {
                    console.err(&format!("[nomi] {}", serving(label, &server, port)));
                    slot.live = Live::Served(server);
                    slot.code = Some(hash);
                }
                Err(report) => console::report(console, &report, Compat::LATEST),
            }
        }
        (Next::Reload, Some((_, hash))) => {
            if let Live::Served(server) = &slot.live {
                console.err(&format!("[nomi] Reloading {label}\n"));
                server.reload();
                slot.code = Some(hash);
            }
        }
        (Next::Run | Next::Serve | Next::Reload, None) => {
            unreachable!("an entry is started or reloaded only once built")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::Site;

    #[test]
    fn an_entry_restarts_or_reloads_only_where_its_code_changed() {
        let (old, new) = (Hash::of(b"old"), Hash::of(b"new"));
        let site = Site {
            code: PathBuf::from("Main.js"),
            module: "Main".to_owned(),
            title: "web".to_owned(),
            static_dir: None,
        };
        let served = Live::Served(Server::start(site, [0], Reloads::Heard).unwrap());
        let (node, browser) = (Platform::Node, Platform::Browser);
        let cases = [
            (&Live::Idle, None, None, Next::Keep),
            (&Live::Idle, None, Some((node, new)), Next::Run),
            (&Live::Idle, Some(old), Some((node, old)), Next::Run),
            (&Live::Program(0), Some(old), Some((node, old)), Next::Keep),
            (&Live::Program(0), Some(old), Some((node, new)), Next::Run),
            (&Live::Program(0), Some(old), None, Next::Stop),
            (
                &Live::Program(0),
                Some(old),
                Some((browser, old)),
                Next::Serve,
            ),
            (&Live::Idle, None, Some((browser, new)), Next::Serve),
            (&served, Some(old), Some((browser, old)), Next::Keep),
            (&served, Some(old), Some((browser, new)), Next::Reload),
            (&served, Some(old), None, Next::Keep),
            (&served, Some(old), Some((node, old)), Next::Run),
        ];
        for (i, (live, was, now, wanted)) in cases.into_iter().enumerate() {
            assert_eq!(next(live, was, now), wanted, "case {i}");
        }
    }
}
