//! `elm-test install` and `init` (`Install.js`): a package
//! added to the test dependencies by `elm install` in a scratch copy of
//! the project, what it added moved there, and `elm.json` written as
//! elm-test writes it.

use std::fs;
use std::path::Path;

use nomi_engine::console::{Console, Personality};
use nomi_engine::{Http, install};
use nomi_project::Outline;

use crate::TEST_PACKAGE;
use crate::build::{Env, generated_dir};
use crate::files::Project;
use crate::json::Json;

/// elm-test's template for `tests/Example.elm`.
const EXAMPLE: &str = include_str!("../runner/Example.elm");

/// What an install did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Installed {
    /// The package was a direct test dependency already.
    Already,
    Added,
}

/// Why an install stopped.
#[derive(Debug)]
pub(crate) enum InstallError {
    /// elm-test's message.
    Message(String),
    /// `elm install`'s report, which it showed.
    Shown,
}

/// A list of dependencies in `elm.json`: a package's one, or an
/// application's direct or indirect one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Package,
    Direct,
    Indirect,
}

impl Section {
    /// Its key within the list `field` of an application.
    fn key(self) -> Option<&'static str> {
        match self {
            Section::Package => None,
            Section::Direct => Some("direct"),
            Section::Indirect => Some("indirect"),
        }
    }

    /// This list of `json`'s `field`, `dependencies` or `test-dependencies`.
    fn of<'j>(self, json: &'j Json, field: &str) -> Option<&'j Json> {
        let list = json.get(field)?;
        self.key().map_or(Some(list), |key| list.get(key))
    }

    fn of_mut<'j>(self, json: &'j mut Json, field: &str) -> Option<&'j mut Json> {
        let list = json.get_mut(field)?;
        match self.key() {
            Some(key) => list.get_mut(key),
            None => Some(list),
        }
    }
}

/// `Install.install`: `package` added to `project`'s test dependencies.
pub(crate) fn install<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
    package: &str,
) -> Result<Installed, InstallError> {
    let old = read(&project.root.join("elm.json"))?;
    let application = matches!(project.outline, Outline::App(_));
    // Where a direct dependency goes.
    let direct = if application {
        Section::Direct
    } else {
        Section::Package
    };
    if direct
        .of(&old, "test-dependencies")
        .and_then(|tests| tests.get(package))
        .is_some()
    {
        return Ok(Installed::Already);
    }
    let scratch = generated_dir(env.cx.cache, project).join("install");
    let created = (match fs::remove_dir_all(&scratch) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    })
    .and_then(|()| fs::create_dir_all(&scratch));
    if let Err(e) = created {
        return Err(InstallError::Message(format!(
            "Unable to create temporary directory for elm-test install: {e}"
        )));
    }
    let mut copy = old.clone();
    if application {
        // Without this, `elm install` would want the source directories in
        // the scratch copy.
        copy.set(
            "source-directories",
            Json::Array(vec![Json::String(".".to_owned())]),
        );
    }
    write(&scratch, &copy)?;
    if !install::run(&env.at(&scratch), console, Personality::Elm, Some(package)) {
        return Err(InstallError::Shown);
    }
    let mut new = read(&scratch.join("elm.json"))?;
    if application {
        move_to_tests(&old, &mut new, Section::Direct);
        move_to_tests(&old, &mut new, Section::Indirect);
        if let Some(dirs) = old.get("source-directories") {
            new.set("source-directories", dirs.clone());
        }
    } else {
        move_to_tests(&old, &mut new, Section::Package);
    }
    write(&project.root, &new)?;
    Ok(Installed::Added)
}

/// What `elm install` added to `section` of the dependencies moved to the
/// same section of the test dependencies. An indirect one already a
/// direct test dependency stays one.
fn move_to_tests(old: &Json, new: &mut Json, section: Section) {
    let had = section.of(old, "dependencies");
    let Some(now) = section.of(new, "dependencies") else {
        return;
    };
    let added: Vec<String> = (now.keys())
        .filter(|key| had.and_then(|had| had.get(key)).is_none())
        .map(str::to_owned)
        .collect();
    for key in added {
        let Some(value) = (section.of_mut(new, "dependencies")).and_then(|deps| deps.remove(&key))
        else {
            continue;
        };
        let already = section == Section::Indirect
            && (Section::Direct.of(new, "test-dependencies"))
                .and_then(|tests| tests.get(&key))
                .is_some();
        if !already && let Some(tests) = section.of_mut(new, "test-dependencies") {
            tests.set(&key, value);
        }
    }
}

/// `ElmJson.read`, its fields in order.
fn read(path: &Path) -> Result<Json, InstallError> {
    let text = fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str::<Json>(&text).map_err(|e| e.to_string()));
    text.map_err(|e| {
        InstallError::Message(format!("{}\nFailed to read elm.json:\n{e}", path.display()))
    })
}

/// `ElmJson.write`: `JSON.stringify` with four spaces, and a line break.
fn write(dir: &Path, json: &Json) -> Result<(), InstallError> {
    let path = dir.join("elm.json");
    fs::write(&path, format!("{}\n", json.stringify())).map_err(|e| {
        InstallError::Message(format!(
            "{}\nFailed to write elm.json:\n{e}",
            path.display()
        ))
    })
}

/// `elm-test init`: elm-explorations/test installed, and
/// `tests/Example.elm`, written whether or not one is there.
pub(crate) fn init<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
) -> Result<(), InstallError> {
    install(env, console, project, TEST_PACKAGE)?;
    let example = project.tests_dir.join("Example.elm");
    (fs::create_dir_all(&project.tests_dir).and_then(|()| fs::write(&example, EXAMPLE)))
        .map_err(|e| InstallError::Message(e.to_string()))?;
    console.out(&format!(
        "\nCheck out the documentation for getting started at https://package.elm-lang.org/packages/{TEST_PACKAGE}/latest\n"
    ));
    Ok(())
}
