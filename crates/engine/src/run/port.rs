//! The port each browser entry of a run is served on, assigned once from
//! the entries' manifests, so neither the order the entries build in nor
//! a failed build moves one.

use nomi_base::Platform;
use nomi_project::Entry;

/// The port the first entry without one is served on, and the next the
/// next.
pub(super) const FIRST_PORT: u16 = 8000;

/// Where a browser entry is served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Port {
    /// Its entry's own `port`, which something else listening on is an
    /// error.
    Given(u16),
    /// The one assigned it, or where something else listens there the
    /// next that is free and no other entry's.
    Assigned(u16),
}

impl Port {
    /// Its number.
    pub(super) fn number(self) -> u16 {
        match self {
            Port::Given(port) | Port::Assigned(port) => port,
        }
    }
}

/// The port of each of `entries`, in order: its `port` where it has one,
/// and in turn the next from [`FIRST_PORT`] that no entry gives where it
/// has none; or `None` where it runs on Bun by its manifest. An entry
/// whose `main` decides its platform holds a number either way, which a
/// build can't change.
pub(super) fn ports<'e>(entries: impl IntoIterator<Item = &'e Entry>) -> Vec<Option<Port>> {
    let entries: Vec<&Entry> = entries.into_iter().collect();
    let given: Vec<u16> = (entries.iter())
        .filter(|entry| entry.platform != Some(Platform::Bun))
        .filter_map(|entry| entry.port)
        .collect();
    let mut free = (FIRST_PORT..=u16::MAX).filter(|port| !given.contains(port));
    (entries.iter())
        .map(|entry| match (entry.platform, entry.port) {
            (Some(Platform::Bun), _) => None,
            (Some(Platform::Browser) | None, Some(port)) => Some(Port::Given(port)),
            (Some(Platform::Browser) | None, None) => {
                Some(Port::Assigned(free.next().unwrap_or(u16::MAX)))
            }
        })
        .collect()
}

/// The ports to try in turn for `port`, one of `ports`: a given one
/// alone; an assigned one, then each after it that none of `ports` is.
pub(super) fn tries(port: Port, ports: &[Option<Port>]) -> impl Iterator<Item = u16> + '_ {
    let after = match port {
        Port::Given(_) => None,
        Port::Assigned(first) => first.checked_add(1).map(|next| next..=u16::MAX),
    };
    let others = after.into_iter().flatten();
    std::iter::once(port.number())
        .chain(others.filter(move |&next| !ports.iter().flatten().any(|p| p.number() == next)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(platform: Option<Platform>, port: Option<u16>) -> Entry {
        Entry {
            main: "src/Main.elm".to_owned(),
            output: None,
            platform,
            args: Vec::new(),
            static_dir: None,
            port,
        }
    }

    #[test]
    fn given_ports_are_kept_and_the_others_assigned_around_them() {
        let (browser, bun) = (Some(Platform::Browser), Some(Platform::Bun));
        let (given, assigned) = (Port::Given, Port::Assigned);
        let cases = [
            (vec![entry(browser, None)], vec![Some(assigned(8000))]),
            // A member without a port that sorts before one given 8000.
            (
                vec![entry(browser, None), entry(browser, Some(8000))],
                vec![Some(assigned(8001)), Some(given(8000))],
            ),
            (
                vec![
                    entry(None, Some(8001)),
                    entry(browser, None),
                    entry(browser, None),
                    entry(browser, Some(8000)),
                ],
                vec![
                    Some(given(8001)),
                    Some(assigned(8002)),
                    Some(assigned(8003)),
                    Some(given(8000)),
                ],
            ),
            // A Bun program holds none, even with a port.
            (
                vec![entry(bun, Some(8000)), entry(browser, None)],
                vec![None, Some(assigned(8000))],
            ),
            (
                vec![entry(None, None), entry(browser, None)],
                vec![Some(assigned(8000)), Some(assigned(8001))],
            ),
        ];
        for (i, (entries, wanted)) in cases.into_iter().enumerate() {
            assert_eq!(ports(&entries), wanted, "case {i}");
        }
    }

    #[test]
    fn an_assigned_port_moves_past_the_others() {
        let ports = [
            Some(Port::Assigned(8000)),
            Some(Port::Given(8001)),
            None,
            Some(Port::Assigned(8002)),
        ];
        let first: Vec<u16> = tries(Port::Assigned(8000), &ports).take(3).collect();
        assert_eq!(first, [8000, 8003, 8004]);
        let given: Vec<u16> = tries(Port::Given(8001), &ports).collect();
        assert_eq!(given, [8001]);
        let last: Vec<u16> = tries(Port::Assigned(u16::MAX), &[]).collect();
        assert_eq!(last, [u16::MAX]);
    }
}
