//! `elm-test install` and `init` (`Install.js`): a package
//! added to the test dependencies by `elm install` in a scratch copy of
//! the project, what it added moved there, and `elm.json` written as
//! elm-test writes it.

use std::fs;
use std::path::Path;

use nomi_engine::console::{Console, Personality};
use nomi_engine::{Http, install};

use crate::build::{Env, generated_dir};
use crate::files::Project;
use crate::json::Json;

/// elm-test's template for `tests/Example.elm`.
const EXAMPLE: &str = include_str!("../runner/Example.elm");

/// The package `init` installs.
const TEST: &str = "elm-explorations/test";

/// What an install did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The package was a direct test dependency already.
    Already,
    Added,
}

/// `Install.install`: `package` added to `project`'s test dependencies;
/// elm-test's message, or `None` where `elm install`'s own report shows,
/// if it can't be.
pub fn install<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
    package: &str,
) -> Result<Installed, Option<String>> {
    let path = project.root.join("elm.json");
    let old = read(&path)?;
    let application = old.get("type") == Some(&Json::String("application".to_owned()));
    let tests = old.get("test-dependencies");
    let tests = if application {
        tests.and_then(|t| t.get("direct"))
    } else {
        tests
    };
    if tests.and_then(|t| t.get(package)).is_some() {
        return Ok(Installed::Already);
    }
    let scratch = generated_dir(env.cache, project).join("install");
    let created = (match fs::remove_dir_all(&scratch) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    })
    .and_then(|()| fs::create_dir_all(&scratch));
    if let Err(e) = created {
        return Err(Some(format!(
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
    let installed = install::run(&env.at(&scratch), console, Personality::Elm, Some(package));
    if !installed {
        return Err(None);
    }
    let mut new = read(&scratch.join("elm.json"))?;
    if application {
        for directness in ["direct", "indirect"] {
            move_to_tests(&old, &mut new, Some(directness));
        }
        if let Some(dirs) = old.get("source-directories") {
            new.set("source-directories", dirs.clone());
        }
    } else {
        move_to_tests(&old, &mut new, None);
    }
    write(&project.root, &new)?;
    Ok(Installed::Added)
}

/// What `elm install` added to the dependencies (of `directness`, in an
/// application) moved to the test dependencies. An indirect one already
/// a direct test dependency stays one.
fn move_to_tests(old: &Json, new: &mut Json, directness: Option<&str>) {
    let section = |json: &Json, name: &str| -> Option<Json> {
        let section = json.get(name)?;
        match directness {
            Some(d) => section.get(d).cloned(),
            None => Some(section.clone()),
        }
    };
    let had = section(old, "dependencies").unwrap_or(Json::Object(Vec::new()));
    let Some(now) = section(new, "dependencies") else {
        return;
    };
    let direct_tests = (new.get("test-dependencies"))
        .and_then(|t| t.get("direct"))
        .cloned();
    for key in now.keys() {
        if had.get(&key).is_some() {
            continue;
        }
        let value = now.get(&key).cloned().unwrap_or(Json::Null);
        let already = directness == Some("indirect")
            && direct_tests.as_ref().and_then(|t| t.get(&key)).is_some();
        if !already && let Some(tests) = new.get_mut("test-dependencies") {
            match directness {
                Some(d) => {
                    if let Some(tests) = tests.get_mut(d) {
                        tests.set(&key, value);
                    }
                }
                None => tests.set(&key, value),
            }
        }
        if let Some(deps) = new.get_mut("dependencies") {
            match directness {
                Some(d) => {
                    if let Some(deps) = deps.get_mut(d) {
                        deps.remove(&key);
                    }
                }
                None => {
                    deps.remove(&key);
                }
            }
        }
    }
}

/// `ElmJson.read`, its fields in order.
fn read(path: &Path) -> Result<Json, Option<String>> {
    let text = fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str::<Json>(&text).map_err(|e| e.to_string()));
    text.map_err(|e| Some(format!("{}\nFailed to read elm.json:\n{e}", path.display())))
}

/// `ElmJson.write`: `JSON.stringify` with four spaces, and a line break.
fn write(dir: &Path, json: &Json) -> Result<(), Option<String>> {
    let path = dir.join("elm.json");
    fs::write(&path, format!("{}\n", json.stringify())).map_err(|e| {
        Some(format!(
            "{}\nFailed to write elm.json:\n{e}",
            path.display()
        ))
    })
}

/// `elm-test init`: elm-explorations/test installed, and
/// `tests/Example.elm`, written whether or not one is there.
pub fn init<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
) -> Result<(), Option<String>> {
    install(env, console, project, TEST)?;
    let example = project.tests_dir.join("Example.elm");
    (fs::create_dir_all(&project.tests_dir).and_then(|()| fs::write(&example, EXAMPLE)))
        .map_err(|e| Some(e.to_string()))?;
    console.out(&format!(
        "\nCheck out the documentation for getting started at https://package.elm-lang.org/packages/{TEST}/latest\n"
    ));
    Ok(())
}
