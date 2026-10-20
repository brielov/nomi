//! `cargo xtask services`: the services tier 2 tests nomi's drivers
//! against, each run in a container from the image references.toml pins,
//! with podman or docker, and stopped once its suites have run: its smoke
//! test, `xtask/services/smoke.js`, then the tests of nomi-cli's
//! `nomi_services` target named for it, which the `services` feature
//! builds.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use nomi_testkit::process::run_within;

use crate::references::{Image, References};

/// A service, by its image's name in references.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Service {
    Postgres,
    Mysql,
    Valkey,
    Garage,
}

/// What runs the containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Runtime {
    Podman,
    Docker,
}

// Each container listens on the loopback interface alone and lives for
// one run, so its credentials are constants.
const USER: &str = "nomi";
const PASSWORD: &str = "nomi";
const BUCKET: &str = "nomi";
/// Garage's key: an id is `GK` and 24 hex digits, a secret 64.
const KEY_ID: &str = "GK000000000000000000000000";
const SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Garage's configuration: a single node, keeping one copy of each
/// object. The RPC secret lets nodes join, and no other node does.
const GARAGE_CONFIG: &str = r#"metadata_dir = "/var/lib/garage/meta"
data_dir = "/var/lib/garage/data"
db_engine = "sqlite"
replication_factor = 1
rpc_bind_addr = "[::]:3901"
rpc_public_addr = "127.0.0.1:3901"
rpc_secret = "0000000000000000000000000000000000000000000000000000000000000000"

[s3_api]
s3_region = "garage"
api_bind_addr = "[::]:3900"
"#;

/// How long a service has to answer once started: MySQL sets its data
/// directory up first, which takes seconds, and longer on a busy runner.
const STARTUP: Duration = Duration::from_secs(180);

/// How long a smoke test has.
const SMOKE: Duration = Duration::from_secs(120);

impl Service {
    const ALL: [Service; 4] = [
        Service::Postgres,
        Service::Mysql,
        Service::Valkey,
        Service::Garage,
    ];

    fn name(self) -> &'static str {
        match self {
            Service::Postgres => "postgres",
            Service::Mysql => "mysql",
            Service::Valkey => "valkey",
            Service::Garage => "garage",
        }
    }

    /// Whether nomi-cli's `nomi_services` target has tests for it, in a
    /// module of its name.
    fn tested(self) -> bool {
        matches!(self, Service::Postgres | Service::Mysql)
    }

    /// The port it listens on in its container.
    fn port(self) -> u16 {
        match self {
            Service::Postgres => 5432,
            Service::Mysql => 3306,
            Service::Valkey => 6379,
            Service::Garage => 3900,
        }
    }

    /// The environment its image's entrypoint reads.
    fn env(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Service::Postgres => &[
                ("POSTGRES_USER", USER),
                ("POSTGRES_PASSWORD", PASSWORD),
                ("POSTGRES_DB", USER),
            ],
            Service::Mysql => &[
                ("MYSQL_ROOT_PASSWORD", PASSWORD),
                ("MYSQL_USER", USER),
                ("MYSQL_PASSWORD", PASSWORD),
                ("MYSQL_DATABASE", USER),
            ],
            Service::Valkey => &[],
            Service::Garage => &[("GARAGE_CONFIG_FILE", "/garage.toml")],
        }
    }

    /// Files copied into the container before it starts, by their path
    /// there. Garage's image has no `/etc` to hold its configuration.
    fn files(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Service::Garage => &[("garage.toml", GARAGE_CONFIG)],
            Service::Postgres | Service::Mysql | Service::Valkey => &[],
        }
    }

    /// A command run in the container that succeeds once the service
    /// answers. Postgres and MySQL first run a server that sets their data
    /// up and listens on no TCP port, so theirs ask over TCP.
    fn ready(self) -> &'static [&'static str] {
        match self {
            Service::Postgres => &["pg_isready", "--quiet", "--host=127.0.0.1"],
            Service::Mysql => &["mysqladmin", "ping", "--silent", "--host=127.0.0.1"],
            Service::Valkey => &["valkey-cli", "ping"],
            Service::Garage => &["/garage", "node", "id", "--quiet"],
        }
    }

    /// The commands that set the service up once it is ready, given what
    /// [`Service::ready`] printed. Garage's node needs a place in the
    /// cluster's layout before it stores anything, and then the key and
    /// bucket.
    fn setup(self, ready: &str) -> Vec<Vec<&str>> {
        match self {
            Service::Garage => {
                // `<node id>@<address>`
                let node = ready.trim().split('@').next().unwrap_or_default();
                vec![
                    vec![
                        "/garage",
                        "layout",
                        "assign",
                        "--zone=dc1",
                        "--capacity=1G",
                        node,
                    ],
                    vec!["/garage", "layout", "apply", "--version=1"],
                    vec![
                        "/garage", "key", "import", "--yes", "-n", USER, KEY_ID, SECRET,
                    ],
                    vec!["/garage", "bucket", "create", BUCKET],
                    vec![
                        "/garage", "bucket", "allow", "--read", "--write", "--owner", BUCKET,
                        "--key", USER,
                    ],
                ]
            }
            Service::Postgres | Service::Mysql | Service::Valkey => Vec::new(),
        }
    }

    /// The URL its smoke test is given, credentials included, for the
    /// address its port is published on.
    fn url(self, address: &str) -> String {
        match self {
            Service::Postgres => format!("postgres://{USER}:{PASSWORD}@{address}/{USER}"),
            Service::Mysql => format!("mysql://{USER}:{PASSWORD}@{address}/{USER}"),
            Service::Valkey => format!("redis://{address}"),
            Service::Garage => {
                format!("http://{KEY_ID}:{SECRET}@{address}/{BUCKET}?region=garage")
            }
        }
    }
}

impl Runtime {
    fn program(self) -> &'static str {
        match self {
            Runtime::Podman => "podman",
            Runtime::Docker => "docker",
        }
    }

    fn command(self) -> Command {
        Command::new(self.program())
    }

    /// Whether it is installed: `--version` asks no daemon, so a docker
    /// whose daemon is down fails at its first real command, saying why.
    fn installed(self) -> bool {
        self.command()
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Runs it with the arguments, bailing with what it printed when it
    /// fails.
    fn run(self, args: &[&str]) -> Result<String> {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(
                "`{} {}` failed: {}",
                self.program(),
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn output(self, args: &[&str]) -> Result<Output> {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running {}", self.program()))
    }
}

/// The runtime asked for, or else podman or docker, whichever `installed`
/// finds first. The services run nowhere else, so one must be.
fn detect(given: Option<Runtime>, installed: impl Fn(Runtime) -> bool) -> Result<Runtime> {
    match given {
        Some(runtime) if installed(runtime) => Ok(runtime),
        Some(runtime) => bail!(
            "--runtime {0}: {0} isn't on the PATH, or doesn't run",
            runtime.program()
        ),
        None => [Runtime::Podman, Runtime::Docker]
            .into_iter()
            .find(|&runtime| installed(runtime))
            .context(
                "the services run in containers, with podman or docker, \
                 and neither is on the PATH",
            ),
    }
}

/// Starts each service, every one if `only` names none, runs its smoke
/// test, and stops them all, on failure and on SIGINT, SIGTERM or SIGHUP
/// too.
pub(crate) fn run(
    root: &Path,
    refs: &References,
    only: &[Service],
    runtime: Option<Runtime>,
) -> Result<()> {
    crate::oracle::bun_version(&refs.bun_minimum)?;
    let runtime = detect(runtime, Runtime::installed)?;
    let stop = stop_on_signals()?;
    let services: Vec<Service> = (Service::ALL.into_iter())
        .filter(|service| only.is_empty() || only.contains(service))
        .collect();
    let images = services
        .iter()
        .map(|service| {
            refs.image
                .iter()
                .find(|image| image.name == service.name())
                .with_context(|| format!("references.toml pins no image for {}", service.name()))
        })
        .collect::<Result<Vec<_>>>()?;
    for image in &images {
        pull(runtime, image)?;
    }
    let mut containers = Containers {
        runtime,
        label: format!("nomi.services={}", std::process::id()),
        names: Vec::new(),
    };
    for (&service, image) in services.iter().zip(&images) {
        containers.start(service, image)?;
    }
    let mut failures = Vec::new();
    let mut urls = Vec::new();
    for (&service, image) in services.iter().zip(&images) {
        let url = containers.ready(service, &stop)?;
        if smoke(root, service, &url)? {
            println!("ok   {} {}", service.name(), image.tag);
        } else {
            failures.push(service.name());
        }
        if stop.load(Ordering::Relaxed) {
            bail!("interrupted");
        }
        urls.push((service, url));
    }
    if !failures.is_empty() {
        bail!("smoke tests failed: {}", failures.join(", "));
    }
    let tested: Vec<(Service, String)> = (urls.into_iter())
        .filter(|(service, _)| service.tested())
        .collect();
    if !tested.is_empty() {
        suites(root, &tested)?;
    }
    containers.stop()
}

/// The tests of nomi-cli's `nomi_services` target for each service, given
/// its URL in `NOMI_SERVICE_<NAME>`.
fn suites(root: &Path, tested: &[(Service, String)]) -> Result<()> {
    let envs: Vec<(String, &str)> = (tested.iter())
        .map(|(service, url)| {
            let var = format!("NOMI_SERVICE_{}", service.name().to_uppercase());
            (var, url.as_str())
        })
        .collect();
    let filters: Vec<String> = (tested.iter())
        .map(|(service, _)| format!("{}::", service.name()))
        .collect();
    let mut args = vec![
        "test",
        "--package",
        "nomi-cli",
        "--features",
        "services",
        "--test",
        "nomi_services",
        "--locked",
        "--",
    ];
    args.extend(filters.iter().map(String::as_str));
    let envs: Vec<(&str, &str)> = (envs.iter())
        .map(|(var, url)| (var.as_str(), *url))
        .collect();
    crate::cargo_with(root, &args, &envs)
}

/// A flag set by the signals that would end xtask, which instead lets it
/// stop its containers. A terminal's ^C reaches the whole process group,
/// so a child that runs then ends on its own.
#[cfg_attr(
    not(unix),
    expect(
        clippy::unnecessary_wraps,
        reason = "only Unix's signals are caught, which can fail"
    )
)]
fn stop_on_signals() -> Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&stop))
            .context("catching the signals that stop the services")?;
    }
    Ok(stop)
}

/// Pulls the image by its digest, unless the runtime has it.
fn pull(runtime: Runtime, image: &Image) -> Result<()> {
    let reference = image.reference();
    if runtime
        .output(&["image", "inspect", &reference])?
        .status
        .success()
    {
        return Ok(());
    }
    eprintln!("==> pulling {} {} ({reference})", image.name, image.tag);
    let status = runtime
        .command()
        .args(["pull", &reference])
        .status()
        .with_context(|| format!("running {}", runtime.program()))?;
    if !status.success() {
        bail!("`{} pull {reference}` failed", runtime.program());
    }
    Ok(())
}

/// The containers this run started, each labelled with xtask's process
/// id. Dropping it removes them, on an error's way out.
struct Containers {
    runtime: Runtime,
    label: String,
    names: Vec<String>,
}

impl Containers {
    fn name(service: Service) -> String {
        format!("nomi-{}-{}", service.name(), std::process::id())
    }

    /// Creates the service's container, its port published on a free one
    /// of the loopback interface, copies its files in, and starts it.
    fn start(&mut self, service: Service, image: &Image) -> Result<()> {
        let name = Self::name(service);
        let publish = format!("127.0.0.1::{}", service.port());
        let env: Vec<String> = (service.env().iter())
            .map(|(key, value)| format!("--env={key}={value}"))
            .collect();
        let reference = image.reference();
        let mut args = vec!["create", "--name", &name, "--label", &self.label];
        args.extend(["--publish", &publish]);
        args.extend(env.iter().map(String::as_str));
        args.push(&reference);
        self.runtime.run(&args)?;
        self.names.push(name.clone());
        if !service.files().is_empty() {
            self.copy(&name, service.files())?;
        }
        self.runtime.run(&["start", &name])?;
        Ok(())
    }

    /// Copies the files into the container's root, as the tar archive
    /// `cp -` reads.
    fn copy(&self, name: &str, files: &[(&str, &str)]) -> Result<()> {
        let mut archive = tar::Builder::new(Vec::new());
        for (path, text) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(text.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, text.as_bytes())?;
        }
        let archive = archive.into_inner()?;
        let mut command = self.runtime.command();
        command.args(["cp", "-", &format!("{name}:/")]);
        let Ok(output) = run_within(&mut command, &archive, Some(SMOKE))? else {
            bail!("`{} cp` into {name} hung", self.runtime.program());
        };
        if !output.status.success() {
            bail!(
                "copying files into {name}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Waits for the service to answer, sets it up, and gives the URL it
    /// answers on from outside its container.
    fn ready(&self, service: Service, stop: &AtomicBool) -> Result<String> {
        let name = Self::name(service);
        let mut exec = vec!["exec", &name];
        exec.extend(service.ready());
        let deadline = Instant::now() + STARTUP;
        let ready = loop {
            if stop.load(Ordering::Relaxed) {
                bail!("interrupted");
            }
            let output = self.runtime.output(&exec)?;
            if output.status.success() {
                break String::from_utf8_lossy(&output.stdout).into_owned();
            }
            let running =
                self.runtime
                    .run(&["inspect", "--format", "{{.State.Running}}", &name])?;
            if running.trim() != "true" || Instant::now() > deadline {
                let logs = self.runtime.output(&["logs", "--tail=20", &name])?;
                bail!(
                    "{} {} within {} s; its last lines:\n{}{}",
                    service.name(),
                    if running.trim() == "true" {
                        "didn't answer"
                    } else {
                        "stopped"
                    },
                    STARTUP.as_secs(),
                    String::from_utf8_lossy(&logs.stdout),
                    String::from_utf8_lossy(&logs.stderr)
                );
            }
            thread::sleep(Duration::from_millis(250));
        };
        for command in service.setup(&ready) {
            let mut exec = vec!["exec", &name];
            exec.extend(command);
            self.runtime.run(&exec)?;
        }
        let port = format!("{}/tcp", service.port());
        let published = self.runtime.run(&["port", &name, &port])?;
        let address = published
            .lines()
            .next()
            .with_context(|| format!("{name} publishes no port {port}"))?;
        Ok(service.url(address.trim()))
    }

    /// Removes every container, and fails if any of this run's is left.
    fn stop(&mut self) -> Result<()> {
        if self.names.is_empty() {
            return Ok(());
        }
        let mut args = vec!["rm", "--force", "--volumes"];
        args.extend(self.names.iter().map(String::as_str));
        let removed = self.runtime.run(&args);
        self.names.clear();
        removed?;
        let filter = format!("label={}", self.label);
        let left = self
            .runtime
            .run(&["ps", "--all", "--quiet", "--filter", &filter])?;
        if !left.trim().is_empty() {
            bail!(
                "containers left behind: {}",
                left.split_whitespace().collect::<Vec<_>>().join(", ")
            );
        }
        Ok(())
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("error: stopping the services: {error:#}");
        }
    }
}

/// Runs the service's smoke test, printing its output when it fails.
fn smoke(root: &Path, service: Service, url: &str) -> Result<bool> {
    let mut command = Command::new("bun");
    command
        .arg(root.join("xtask/services/smoke.js"))
        .arg(service.name())
        .env("NOMI_SERVICE_URL", url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (output, hung) = match run_within(&mut command, &[], Some(SMOKE))? {
        Ok(output) => (output, false),
        Err(output) => (output, true),
    };
    if output.status.success() && !hung {
        return Ok(true);
    }
    println!(
        "FAIL {}{}\n{}{}",
        service.name(),
        if hung { " (hung)" } else { "" },
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_runtime_fails_loudly() {
        let error = detect(None, |_| false).unwrap_err().to_string();
        assert_eq!(
            error,
            "the services run in containers, with podman or docker, and neither is on the PATH"
        );
        let error = detect(Some(Runtime::Docker), |runtime| runtime == Runtime::Podman)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "--runtime docker: docker isn't on the PATH, or doesn't run"
        );
    }

    #[test]
    fn podman_comes_first() {
        assert_eq!(detect(None, |_| true).unwrap(), Runtime::Podman);
        assert_eq!(
            detect(None, |runtime| runtime == Runtime::Docker).unwrap(),
            Runtime::Docker
        );
        assert_eq!(
            detect(Some(Runtime::Docker), |_| true).unwrap(),
            Runtime::Docker
        );
    }

    #[test]
    fn every_service_has_a_pin() {
        let refs = crate::references::parse(include_str!("../../references.toml")).unwrap();
        for service in Service::ALL {
            assert!(
                refs.image.iter().any(|image| image.name == service.name()),
                "{}",
                service.name()
            );
        }
    }

    #[test]
    fn garage_is_set_up_for_its_node() {
        let setup = Service::Garage.setup("9e23e3cd2dc2f6d0@127.0.0.1:3901\n");
        assert_eq!(
            setup[0],
            [
                "/garage",
                "layout",
                "assign",
                "--zone=dc1",
                "--capacity=1G",
                "9e23e3cd2dc2f6d0"
            ]
        );
        assert_eq!(
            setup[4],
            [
                "/garage", "bucket", "allow", "--read", "--write", "--owner", "nomi", "--key",
                "nomi"
            ]
        );
    }
}
