//! `nomi run --watch`: each entry built and run or served as `nomi run`
//! does, then, on each change, the members it touches built again with
//! their dependents, through one engine, each Bun program started again
//! and each page reloaded only where its code changed, and a page
//! reloaded without a build when its static files change. A failed build
//! leaves a Bun program stopped and a page on its old code, and the
//! others run on. One entry is the case of a workspace whose one member
//! runs it alone, its streams nomi's own.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use nomi_base::{Compat, Platform, lock};
use nomi_project::Entry;
use nomi_project::workspace::{Member, Workspace};
use nomi_registry::Http;

use super::{Prefix, Running, Streams, label_of, prefixes_of, served, serving, unrunnable};
use crate::Context;
use crate::artifacts::Hash;
use crate::console::{self, Console};
use crate::make::Shared;
use crate::paths::{ELM_JSON, NOMI_TOML, lexical};
use crate::run::port::{Port, ports};
use crate::run::{Caught, Signalled, Stderr, Stop, Watched, exit_code, no_bun};
use crate::serve::{Reloads, Server};
use crate::watch::{Change, POLL, Snapshot, message};
use crate::workspace::{self, EntryBuilt, Making, with_deps};

/// Runs the entries of the members of `workspace` in `dirs` as `nomi
/// run` does, then again as they change, what `watched` names read again
/// when a manifest does, until nomi is sent SIGINT or SIGTERM; the code to
/// exit with, 130 or 143, or 1 if the watch couldn't start.
pub(in crate::run) fn watch(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    watched: &Watched<'_>,
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
    // Changes are named from the project run alone, or else the root.
    let (streams, args, from) = match *watched {
        Watched::Members(_) => (Streams::Prefixed, Vec::new(), root.clone()),
        Watched::One { args, .. } => {
            let dir = dirs
                .first()
                .map_or_else(|| root.clone(), |dir| root.join(dir));
            (Streams::Shared, args.to_vec(), dir)
        }
    };
    let shared = Shared::new(cx.cache, Some(&root));
    let mut snapshot = Snapshot::new(&from, vec![root.join(NOMI_TOML)]);
    let mut watch = Watch {
        workspace,
        dirs: Vec::new(),
        slots: Vec::new(),
        running: Running::new(console, Vec::new(), streams, args),
    };
    let mut engine = Engine {
        cx,
        shared: &shared,
        caught: &mut caught,
    };
    let nomi = streams.nomi();
    let mut caught_now = watch.read(&mut engine, console, dirs);
    loop {
        if let Some(signal) = caught_now {
            return (watch.running).drain(
                console,
                engine.caught,
                signal.passed_on(),
                signal.stop.code(),
            );
        }
        snapshot.watch(watch.paths(), watch.statics());
        console.err(&format!("{nomi}Watching for changes...\n"));
        let queue = match watch.wait(&mut engine, console, &mut snapshot) {
            Ok(queue) => queue,
            Err(signal) => {
                caught_now = Some(signal);
                continue;
            }
        };
        console.err(&format!("{nomi}{}\n", message(&queue)));
        cx.vfs.disk_changed();
        caught_now = watch.changed(&mut engine, console, *watched, &queue);
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
    prefix: Prefix,
    /// Where it runs, as its manifest says or its last build found.
    platform: Option<Platform>,
    /// Where it is served if it's a browser program.
    port: Option<Port>,
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
    /// Its Bun program, by its id among the programs.
    Program(usize),
    /// Its page.
    Served(Server),
}

/// A member's code, absolute.
struct Sources {
    /// Its source directories.
    dirs: Vec<PathBuf>,
    /// Its entries' mains.
    files: Vec<PathBuf>,
}

impl Sources {
    /// The code of `member` of the workspace at `root`.
    fn of(root: &Path, member: &Member) -> Sources {
        let dir = root.join(&member.dir);
        let entries = member.manifest.iter().flat_map(|m| &m.entries);
        Sources {
            dirs: (member.outline.source_dirs().iter())
                .map(|d| lexical(&dir.join(d)))
                .collect(),
            files: entries.map(|e| lexical(&dir.join(&e.main))).collect(),
        }
    }

    /// Whether `path` is among them.
    fn contain(&self, path: &Path) -> bool {
        self.dirs.iter().any(|dir| path.starts_with(dir)) || self.files.iter().any(|f| f == path)
    }
}

/// An entry's code as a build left it: what built it, where it runs,
/// and its hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fresh<B> {
    built: B,
    platform: Platform,
    hash: Hash,
}

/// What a build makes an entry do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next<B> {
    /// Run on as it is.
    Keep,
    /// Stop its program, as its build failed.
    Stop,
    /// Stop what runs, and start its Bun program.
    Run(Fresh<B>),
    /// Stop what runs, and serve its page.
    Serve(Fresh<B>),
    /// Tell its page to reload, its code now the code of this hash.
    Reload(Hash),
}

/// What an entry that runs as `live`, the code whose hash is `was`, does
/// once built as `now`, or where its build failed, `None`.
fn next<B>(live: &Live, was: Option<Hash>, now: Option<Fresh<B>>) -> Next<B> {
    let same = |fresh: &Fresh<B>| was == Some(fresh.hash);
    match (live, now) {
        (Live::Program(_), None) => Next::Stop,
        (Live::Idle | Live::Served(_), None) => Next::Keep,
        (Live::Program(_), Some(fresh)) if fresh.platform == Platform::Bun && same(&fresh) => {
            Next::Keep
        }
        (Live::Served(_), Some(fresh)) if fresh.platform == Platform::Browser && same(&fresh) => {
            Next::Keep
        }
        (Live::Served(_), Some(fresh)) if fresh.platform == Platform::Browser => {
            Next::Reload(fresh.hash)
        }
        (Live::Idle | Live::Program(_) | Live::Served(_), Some(fresh)) => match fresh.platform {
            Platform::Bun => Next::Run(fresh),
            Platform::Browser => Next::Serve(fresh),
        },
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
    ) -> Option<Signalled> {
        let entries: Vec<(&Member, &Entry)> = (self.workspace.members().iter())
            .filter(|member| dirs.contains(&member.dir))
            .flat_map(|member| {
                (member.manifest.iter())
                    .flat_map(move |m| &m.entries)
                    .map(move |e| (member, e))
            })
            .collect();
        let labels: Vec<String> = match self.running.streams {
            Streams::Prefixed => entries.iter().map(|(m, e)| label_of(m, e)).collect(),
            Streams::Shared => entries.iter().map(|(_, e)| e.main.clone()).collect(),
        };
        let ports = ports(entries.iter().map(|&(_, entry)| entry));
        let mut old = std::mem::take(&mut self.slots);
        let each = entries.into_iter().zip(labels).zip(ports);
        for (((member, entry), label), port) in each {
            let kept = old
                .iter()
                .position(|s| s.member == member.dir && s.entry == *entry && s.port == port);
            let slot = match kept {
                Some(i) => Slot {
                    label,
                    ..old.remove(i)
                },
                None => Slot {
                    member: member.dir.clone(),
                    entry: entry.clone(),
                    label,
                    prefix: Prefix::default(),
                    platform: entry.platform,
                    port,
                    output: None,
                    code: None,
                    live: Live::Idle,
                },
            };
            self.slots.push(slot);
        }
        let gone: Vec<usize> = (old.iter())
            .filter_map(|slot| match slot.live {
                Live::Program(id) => Some(id),
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
    /// packages they depend on, and its entries' mains.
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
        Some(lexical(&dir.join(slot.entry.static_dir.as_ref()?)))
    }

    /// Shows the programs' lines, and says when one ends, until the files
    /// change; the changes, those made together joined, but not the
    /// entries' code, which builds write; or the signal nomi is sent.
    fn wait(
        &mut self,
        engine: &mut Engine<'_, '_, impl Http + Sync>,
        console: &mut dyn Console,
        snapshot: &mut Snapshot,
    ) -> Result<Vec<Change>, Signalled> {
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
        let nomi = self.running.streams.nomi();
        for slot in &mut self.slots {
            if let Live::Program(id) = slot.live
                && let Some(status) = self.running.ended(id)
            {
                let code = exit_code(status);
                console.err(&format!("{nomi}{} exited with code {code}\n", slot.label));
                slot.live = Live::Idle;
            }
        }
    }

    /// Builds what `queue` changed again: every member where a manifest
    /// changed, what `watched` names read again; or else the members
    /// whose files changed and their dependents. Pages whose static files
    /// changed reload. The signal nomi is sent meanwhile, if one is.
    fn changed(
        &mut self,
        engine: &mut Engine<'_, '_, impl Http + Sync>,
        console: &mut dyn Console,
        watched: Watched<'_>,
        queue: &[Change],
    ) -> Option<Signalled> {
        let nomi = self.running.streams.nomi();
        for slot in &self.slots {
            if let (Live::Served(server), Some(dir)) = (&slot.live, self.static_dir(slot))
                && queue.iter().any(|change| change.path().starts_with(&dir))
            {
                console.err(&format!("{nomi}Reloading {}\n", slot.label));
                server.reload();
            }
        }
        if queue.iter().any(|change| self.is_manifest(change.path())) {
            return match watched.read(engine.cx) {
                Ok(Some((workspace, dirs))) => {
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
    ) -> Option<Signalled> {
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
        let mut nexts = Vec::with_capacity(slots.len());
        for slot in slots.iter_mut() {
            if !dirs.contains(&slot.member) {
                nexts.push(Next::Keep);
                continue;
            }
            let fresh = fresh(console, root, &builds.entries, slot);
            if let Some(fresh) = &fresh {
                slot.output = Some(fresh.built.output.clone());
                slot.platform = Some(fresh.platform);
            }
            nexts.push(next(&slot.live, slot.code, fresh));
        }
        if running.streams == Streams::Prefixed {
            // Until a build finds it, an entry whose `main` decides its
            // platform may print.
            let labels: Vec<(&str, bool)> = (slots.iter())
                .map(|slot| {
                    (
                        slot.label.as_str(),
                        slot.platform != Some(Platform::Browser),
                    )
                })
                .collect();
            let prefixes = prefixes_of(&labels);
            for (slot, prefix) in slots.iter_mut().zip(prefixes) {
                slot.prefix = prefix;
            }
        }
        let stopping: Vec<usize> = (slots.iter().zip(&nexts))
            .filter_map(|(slot, next)| match (&slot.live, next) {
                (Live::Program(id), Next::Stop | Next::Run(_) | Next::Serve(_)) => Some(*id),
                _ => None,
            })
            .collect();
        if let Some(signal) = running.stop(console, engine.caught, &stopping, Some(Stop::Terminate))
        {
            return Some(signal);
        }
        let ports: Vec<Option<Port>> = slots.iter().map(|slot| slot.port).collect();
        for (slot, next) in slots.iter_mut().zip(nexts) {
            apply(console, running, root, slot, next, &ports);
        }
        None
    }
}

/// The code a build of the workspace at `root`, which wrote `entries`,
/// left `slot` to run, or `None` where it wrote none it can.
fn fresh<'b, 'w>(
    console: &mut dyn Console,
    root: &Path,
    entries: &'b [EntryBuilt<'w>],
    slot: &Slot,
) -> Option<Fresh<&'b EntryBuilt<'w>>> {
    let built = (entries.iter()).find(|b| b.member.dir == slot.member && *b.entry == slot.entry)?;
    if let Some(report) = unrunnable(built, root) {
        console::report(console, &report, Compat::LATEST);
        return None;
    }
    Some(Fresh {
        built,
        platform: built.platform,
        hash: code_hash(&built.output),
    })
}

/// The hash of the code at `output`, as a build left it.
fn code_hash(output: &Path) -> Hash {
    Hash::of(&fs::read(output).unwrap_or_default())
}

/// Does what `next` says to `slot`, of the workspace at `root`, a page
/// served on its port of `ports`.
fn apply(
    console: &mut dyn Console,
    running: &mut Running,
    root: &Path,
    slot: &mut Slot,
    next: Next<&EntryBuilt<'_>>,
    ports: &[Option<Port>],
) {
    let nomi = running.streams.nomi();
    let label = &slot.label;
    match next {
        Next::Keep => {}
        Next::Stop => {
            console.err(&format!("{nomi}Stopping {label}, as its build failed\n"));
            slot.live = Live::Idle;
        }
        Next::Run(fresh) => {
            let verb = match slot.live {
                Live::Program(_) => "Restarting",
                Live::Idle | Live::Served(_) => "Starting",
            };
            console.err(&format!("{nomi}{verb} {label}\n"));
            slot.live = Live::Idle;
            match running.start(fresh.built, slot.prefix.clone()) {
                Ok(id) => {
                    slot.live = Live::Program(id);
                    slot.code = Some(fresh.hash);
                }
                Err(e) => console::report(console, &no_bun(&e), Compat::LATEST),
            }
        }
        Next::Serve(fresh) => {
            slot.live = Live::Idle;
            let port = slot
                .port
                .expect("a browser program doesn't run on Bun by its manifest");
            match served(root, fresh.built, port, ports, Reloads::Heard) {
                Ok(server) => {
                    console.err(&format!("{nomi}{}", serving(label, &server, port)));
                    slot.live = Live::Served(server);
                    slot.code = Some(fresh.hash);
                }
                Err(report) => console::report(console, &report, Compat::LATEST),
            }
        }
        Next::Reload(hash) => {
            if let Live::Served(server) = &slot.live {
                console.err(&format!("{nomi}Reloading {label}\n"));
                server.reload();
                slot.code = Some(hash);
            }
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
        let fresh = |platform, hash| Fresh {
            built: (),
            platform,
            hash,
        };
        let bun = |hash| Some(fresh(Platform::Bun, hash));
        let browser = |hash| Some(fresh(Platform::Browser, hash));
        let cases = [
            (&Live::Idle, None, None, Next::Keep),
            (
                &Live::Idle,
                None,
                bun(new),
                Next::Run(fresh(Platform::Bun, new)),
            ),
            (
                &Live::Idle,
                Some(old),
                bun(old),
                Next::Run(fresh(Platform::Bun, old)),
            ),
            (&Live::Program(0), Some(old), bun(old), Next::Keep),
            (
                &Live::Program(0),
                Some(old),
                bun(new),
                Next::Run(fresh(Platform::Bun, new)),
            ),
            (&Live::Program(0), Some(old), None, Next::Stop),
            (
                &Live::Program(0),
                Some(old),
                browser(old),
                Next::Serve(fresh(Platform::Browser, old)),
            ),
            (
                &Live::Idle,
                None,
                browser(new),
                Next::Serve(fresh(Platform::Browser, new)),
            ),
            (&served, Some(old), browser(old), Next::Keep),
            (&served, Some(old), browser(new), Next::Reload(new)),
            (&served, Some(old), None, Next::Keep),
            (
                &served,
                Some(old),
                bun(old),
                Next::Run(fresh(Platform::Bun, old)),
            ),
        ];
        for (i, (live, was, now, wanted)) in cases.into_iter().enumerate() {
            assert_eq!(next(live, was, now), wanted, "case {i}");
        }
    }
}
