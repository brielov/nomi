//! Names, versions, constraints and outlines. Expected positions and suggestions
//! follow upstream's code for each release; suggestions were ranked by a
//! separate model of `Reporting/Suggest.hs`.

use nomi_base::{Compat, utf8};

use crate::json::decode::{Expectation, Problem};
use crate::json::{Cursor, Position};
use crate::outline::{DetectError, Exposed, SrcDir};
use crate::{
    Constraint, ConstraintError, License, ModuleName, Name, Op, Outline, OutlineError, Version,
    license,
};

const V1: Compat = Compat::V0_19_1;
const V2: Compat = Compat::V0_19_2;
const V3: Compat = Compat::V0_19_3;

fn at(col: u32) -> Position {
    Position { row: 1, col }
}

fn version(src: &str, compat: Compat) -> Result<Version, Position> {
    Version::parse(src.as_bytes(), Cursor::start(compat), compat)
}

fn constraint(src: &str, compat: Compat) -> Result<Constraint, ConstraintError> {
    Constraint::parse(src.as_bytes(), Cursor::start(compat), compat)
}

fn name(src: &str) -> Result<Name, Position> {
    let name = Name::parse(src.as_bytes(), Cursor::start(V1), V1);
    for compat in Compat::ALL {
        let again = Name::parse(src.as_bytes(), Cursor::start(compat), compat);
        assert_eq!(again, name, "{src:?} in {compat}");
    }
    name
}

fn module(src: &[u8], compat: Compat) -> Result<ModuleName, Position> {
    ModuleName::parse(src, Cursor::start(compat), compat)
}

fn v(major: u32, minor: u32, patch: u32) -> Version {
    Version::new(major, minor, patch)
}

#[test]
fn versions_parse() {
    for compat in Compat::ALL {
        assert_eq!(version("1.0.0", compat), Ok(v(1, 0, 0)));
        assert_eq!(version("10.20.30", compat), Ok(v(10, 20, 30)));
        assert_eq!(version("", compat), Err(at(1)));
        assert_eq!(version("1.0", compat), Err(at(4)));
        assert_eq!(version("1.0.", compat), Err(at(5)));
        assert_eq!(version("v1.0.0", compat), Err(at(1)));
        // A leading zero is the whole number.
        assert_eq!(version("01.0.0", compat), Err(at(2)));
        assert_eq!(version("1.00.0", compat), Err(at(4)));
        assert_eq!(version("1.0.0 ", compat), Err(at(6)));
    }
}

#[test]
fn versions_wrap_to_their_width() {
    assert_eq!(version("65536.1.65537", V1), Ok(v(0, 1, 1)));
    assert_eq!(version("65536.1.65537", V2), Ok(v(0, 1, 1)));
    assert_eq!(version("65536.1.65537", V3), Ok(v(65536, 1, 65537)));
    // 0.19.3 sums in 32 bits, then keeps 21 or 22 of them.
    assert_eq!(version("2097152.2097153.4194304", V3), Ok(v(0, 1, 0)));
    assert_eq!(version("4294967297.0.0", V3), Ok(v(1, 0, 0)));
    // Text parses as the newest release reads it.
    assert_eq!("65536.1.65537".parse(), version("65536.1.65537", V3));
    assert_eq!(Version::max(V1), v(65535, 0, 0));
    assert_eq!(Version::max(V3), v(2097151, 0, 0));
    assert_eq!(v(1, 2, 3).to_string(), "1.2.3");
}

#[test]
fn versions_bump() {
    for compat in Compat::ALL {
        assert_eq!(v(1, 2, 3).bump_patch(compat), v(1, 2, 4));
        assert_eq!(v(1, 2, 3).bump_minor(compat), v(1, 3, 0));
        assert_eq!(v(1, 2, 3).bump_major(compat), v(2, 0, 0));
    }
    // 16-bit numbers wrap on their own.
    assert_eq!(v(1, 2, 65535).bump_patch(V1), v(1, 2, 0));
    assert_eq!(v(1, 65535, 3).bump_minor(V2), v(1, 0, 0));
    assert_eq!(v(65535, 2, 3).bump_major(V1), v(0, 0, 0));
    // One packed word carries.
    assert_eq!(v(1, 2, 4194303).bump_patch(V3), v(1, 3, 0));
    assert_eq!(v(1, 2097151, 4194303).bump_patch(V3), v(2, 0, 0));
    assert_eq!(v(1, 2097151, 3).bump_minor(V3), v(2, 0, 0));
    assert_eq!(v(2097151, 2097151, 4194303).bump_patch(V3), v(0, 0, 0));
    assert_eq!(v(2097151, 2, 3).bump_major(V3), v(0, 0, 0));
}

#[test]
fn constraints_parse() {
    let range = Constraint {
        lower: v(1, 0, 0),
        lower_op: Op::LessOrEqual,
        upper_op: Op::Less,
        upper: v(2, 0, 0),
    };
    for compat in Compat::ALL {
        assert_eq!(constraint("1.0.0 <= v < 2.0.0", compat), Ok(range));
        let bad = |col| Err(ConstraintError::BadFormat(at(col)));
        assert_eq!(constraint("1.0.0 =< v < 2.0.0", compat), bad(7));
        assert_eq!(constraint("1.0.0 <= v< 2.0.0", compat), bad(11));
        assert_eq!(constraint("1.0.0 <= V < 2.0.0", compat), bad(10));
        assert_eq!(constraint("1.0.0  <= v < 2.0.0", compat), bad(7));
        assert_eq!(constraint("1.0.0 <= v < 2.0", compat), bad(17));
        assert_eq!(constraint("1.0.0 <= v < 2.0.0 ", compat), bad(19));
        // The range is checked before the end.
        let invalid = |a, b| Err(ConstraintError::InvalidRange(a, b));
        assert_eq!(
            constraint("1.0.0 <= v < 1.0.0", compat),
            invalid(v(1, 0, 0), v(1, 0, 0))
        );
        assert_eq!(
            constraint("2.0.0 < v < 1.0.0 and more", compat),
            invalid(v(2, 0, 0), v(1, 0, 0))
        );
    }
    // Wrapped versions can make a range invalid.
    assert_eq!(
        constraint("1.0.0 <= v < 65536.0.0", V3).map(|c| c.upper),
        Ok(v(65536, 0, 0))
    );
    assert_eq!(
        constraint("1.0.0 <= v < 65536.0.0", V1),
        Err(ConstraintError::InvalidRange(v(1, 0, 0), v(0, 0, 0)))
    );
}

#[test]
fn constraints_display() {
    for src in [
        "1.0.0 <= v < 2.0.0",
        "1.0.0 < v <= 2.0.0",
        "0.0.1 < v < 0.0.2",
    ] {
        assert_eq!(constraint(src, V2).unwrap().to_string(), src);
    }
    assert_eq!(
        Constraint::anything(V1).to_string(),
        "1.0.0 <= v <= 65535.0.0"
    );
    assert_eq!(
        Constraint::exactly(v(1, 2, 3)).to_string(),
        "1.2.3 <= v <= 1.2.3"
    );
    assert_eq!(
        Constraint::until_next_minor(v(1, 2, 3), V1).to_string(),
        "1.2.3 <= v < 1.3.0"
    );
    assert_eq!(
        Constraint::until_next_major(v(1, 2, 3), V3).to_string(),
        "1.2.3 <= v < 2.0.0"
    );
}

#[test]
fn constraints_hold_versions() {
    let range = constraint("1.0.0 <= v < 2.0.0", V1).unwrap();
    assert!(!range.satisfies(v(0, 9, 9)));
    assert!(range.satisfies(v(1, 0, 0)));
    assert!(range.satisfies(v(1, 5, 0)));
    assert!(!range.satisfies(v(2, 0, 0)));
    let open = constraint("1.0.0 < v <= 2.0.0", V1).unwrap();
    assert!(!open.satisfies(v(1, 0, 0)));
    assert!(open.satisfies(v(2, 0, 0)));
}

#[test]
fn constraints_intersect() {
    let c = |src| constraint(src, V1).unwrap();
    assert_eq!(
        c("1.0.0 <= v < 2.0.0").intersect(&c("1.5.0 < v <= 3.0.0")),
        Some(c("1.5.0 < v < 2.0.0"))
    );
    assert_eq!(
        c("1.0.0 <= v <= 2.0.0").intersect(&c("1.0.0 < v <= 2.0.0")),
        Some(c("1.0.0 < v <= 2.0.0"))
    );
    assert_eq!(
        c("1.0.0 <= v < 2.0.0").intersect(&c("3.0.0 <= v < 4.0.0")),
        None
    );
    // Ranges that meet at an excluded version still intersect.
    let met = c("1.0.0 <= v < 2.0.0")
        .intersect(&c("2.0.0 <= v < 3.0.0"))
        .unwrap();
    assert_eq!(met.to_string(), "2.0.0 <= v < 2.0.0");
    assert!(!met.satisfies(v(2, 0, 0)));
}

#[test]
fn package_names_parse() {
    let ok = |src: &str| name(src).map(|n| n.to_string());
    assert_eq!(ok("elm/core"), Ok("elm/core".into()));
    assert_eq!(
        ok("Elm-Community/list-extra2"),
        Ok("Elm-Community/list-extra2".into())
    );
    assert_eq!(ok("1/a1"), Ok("1/a1".into()));
    assert_eq!(name("elm/Core"), Err(at(5)));
    assert_eq!(name("elm/1core"), Err(at(5)));
    assert_eq!(name("elm/co_re"), Err(at(7)));
    assert_eq!(name("elm"), Err(at(4)));
    assert_eq!(name("/core"), Err(at(1)));
    assert_eq!(name("-elm/core"), Err(at(1)));
    assert_eq!(name("elm/core "), Err(at(9)));
    // A double dash fails at the second dash.
    assert_eq!(name("elm--x/core"), Err(at(5)));
    assert_eq!(name("elm/a--b"), Err(at(7)));
    // A trailing dash fails only at the end of the string.
    assert_eq!(ok("elm-/core"), Ok("elm-/core".into()));
    assert_eq!(name("elm/core-"), Err(at(10)));
    // Each part is under 256 bytes.
    let long = "a".repeat(255);
    assert!(name(&format!("{long}/{long}")).is_ok());
    assert_eq!(name(&format!("{long}a/core")), Err(at(257)));
    assert_eq!(name(&format!("elm/{long}a")), Err(at(261)));
}

#[test]
fn package_names_order_and_kernel() {
    let n = |src| name(src).unwrap();
    assert!(n("Zed/a") < n("abc/a"));
    assert!(n("elm/core") < n("elm/core-extra"));
    assert!(n("elm/core").is_kernel());
    assert!(n("elm-explorations/test").is_kernel());
    assert!(!n("elm-community/list-extra").is_kernel());
    assert_eq!("elm/virtual-dom".parse(), name("elm/virtual-dom"));
}

#[test]
fn package_names_nearby() {
    let names: Vec<Name> = [
        "elm/core",
        "foo/json",
        "elm/html",
        "elm-explorations/test",
        "elm/json",
    ]
    .into_iter()
    .map(|src| name(src).unwrap())
    .collect();
    let target = name("elm/jsn").unwrap();
    let nearby: Vec<String> = target
        .nearby(&names)
        .iter()
        .map(ToString::to_string)
        .collect();
    // Official authors cost nothing; ties keep their order.
    assert_eq!(
        nearby,
        ["elm/json", "elm-explorations/test", "elm/core", "foo/json"]
    );
}

#[test]
fn module_names_parse() {
    for compat in Compat::ALL {
        assert!(module(b"Main", compat).is_ok());
        assert!(module(b"Html.Attributes", compat).is_ok());
        assert!(module(b"A_1.B2", compat).is_ok());
        assert_eq!(module(b"", compat), Err(at(1)));
        assert_eq!(module(b"main", compat), Err(at(1)));
        assert_eq!(module(b"Html.", compat), Err(at(6)));
        assert_eq!(module(b"Html..A", compat), Err(at(6)));
        assert_eq!(module(b"Html.attributes", compat), Err(at(6)));
        assert_eq!(module(b"Html x", compat), Err(at(5)));
    }
}

#[test]
fn kernel_names() {
    use crate::module_name::{is_kernel, kernel_short};
    assert!(is_kernel("Elm.Kernel.List") && !is_kernel("Elm.Kernelish"));
    assert_eq!(kernel_short("Elm.Kernel.List"), Some("List"));
    assert_eq!(kernel_short("List"), None);
}

#[test]
fn module_names_outside_ascii() {
    let src = "Éclair.Ünder".as_bytes();
    for compat in Compat::ALL {
        assert_eq!(module(src, compat).unwrap().to_str(), Some("Éclair.Ünder"));
    }
    // Columns are characters, then bytes.
    assert_eq!(module("Éa!".as_bytes(), V1), Err(at(3)));
    assert_eq!(module("Éa!".as_bytes(), V2), Err(at(3)));
    assert_eq!(module("Éa!".as_bytes(), V3), Err(at(4)));
    // A lowercase letter can't start a segment.
    assert_eq!(module("éclair".as_bytes(), V2), Err(at(1)));
    // 0.19.1 assembles `C3 09` into `I` without checking it.
    let unchecked = b"\xC3\x09x";
    assert_eq!(module(unchecked, V1).unwrap().as_bytes(), unchecked);
    assert_eq!(module(unchecked, V1).unwrap().to_str(), None);
    assert_eq!(module(unchecked, V2), Err(at(1)));
}

#[test]
fn module_names_under_256_bytes_before_0_19_3() {
    let fits = format!("A{}", "b".repeat(254));
    let long = format!("A{}", "b".repeat(255));
    for compat in Compat::ALL {
        assert!(module(fits.as_bytes(), compat).is_ok());
    }
    assert_eq!(module(long.as_bytes(), V1), Err(at(257)));
    assert_eq!(module(long.as_bytes(), V2), Err(at(257)));
    assert!(module(long.as_bytes(), V3).is_ok());
    // The limit is on bytes, the position in characters.
    let wide = format!("A{}", "é".repeat(128));
    assert_eq!(module(wide.as_bytes(), V2), Err(at(130)));
}

#[test]
fn licenses() {
    assert_eq!(license::LICENSES.len(), 99);
    assert!(
        license::LICENSES
            .windows(2)
            .all(|w| w[0].0.as_bytes() < w[1].0.as_bytes())
    );
    for compat in Compat::ALL {
        for code in ["BSD-3-Clause", "MIT"] {
            let license = License::check(code.as_bytes(), compat).unwrap();
            assert_eq!(license.as_str(), code);
        }
        // The raw string, escapes and all.
        let suggest = |raw: &[u8]| {
            let nearest = License::check(raw, compat).unwrap_err();
            nearest.into_iter().map(License::as_str).collect::<Vec<_>>()
        };
        assert_eq!(suggest(br"MI\u0054")[0], "MIT");
        assert_eq!(suggest(b"mit"), ["MIT", "AAL", "Fair", "IPA"]);
        assert_eq!(suggest(b"BSD3"), ["0BSD", "ISC", "AAL", "Fair"]);
        assert_eq!(
            suggest(b"apache"),
            ["AAL", "Apache-1.1", "Apache-2.0", "IPA"]
        );
        assert_eq!(suggest(b"MIT License"), ["MIT", "Fair", "ISC", "NTP"]);
        // Once for the identifier and once for the full name.
        assert_eq!(
            suggest(b"PostgreSQL License"),
            ["PostgreSQL", "PostgreSQL", "Motosoto", "Multics"]
        );
    }
}

const APP: &str = r#"{"type":"application","source-directories":[SRC],"elm-version":"0.19.1",
"dependencies":{"direct":{"elm/json":"1.1.3","elm/core":"1.0.5"},"indirect":{}},
"test-dependencies":{"indirect":{},"direct":{}}}"#;

/// [`APP`] with `src_dir`, quoted, as its source directory.
fn app_src(src_dir: &[u8]) -> Vec<u8> {
    let (before, after) = APP.split_once("SRC").expect("a source directory");
    [before.as_bytes(), b"\"", src_dir, b"\"", after.as_bytes()].concat()
}

fn app(src_dir: &[u8], compat: Compat) -> Result<Outline, OutlineError> {
    Outline::read(&app_src(src_dir), compat)
}

fn dirs(outline: &Outline) -> Vec<&[u8]> {
    match outline {
        Outline::App(app) => app.source_dirs.iter().map(SrcDir::raw).collect(),
        Outline::Pkg(_) => Vec::new(),
    }
}

#[test]
fn outlines_write_in_upstreams_layout() {
    let outline = app(b"src", V2).unwrap();
    let expected = r#"{
    "type": "application",
    "source-directories": [
        "src"
    ],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": {
            "elm/core": "1.0.5",
            "elm/json": "1.1.3"
        },
        "indirect": {}
    },
    "test-dependencies": {
        "direct": {},
        "indirect": {}
    }
}
"#;
    assert_eq!(String::from_utf8(outline.write()).unwrap(), expected);
}

#[test]
fn source_directories_are_read_raw_and_written_escaped() {
    for compat in Compat::ALL {
        let outline = app(br"a\\b\u0063", compat).unwrap();
        assert_eq!(dirs(&outline), [br"a\\b\u0063".as_slice()]);
        let again = Outline::read(&outline.write(), compat).unwrap();
        assert_eq!(dirs(&again), [br"a\\\\b\\u0063".as_slice()]);
    }
}

#[test]
fn source_directories_outside_utf8_mangle_in_0_19_1() {
    // `C0 22` is one character to 0.19.1, of code -94, which `charUtf8`
    // writes as the byte A2; 0.19.2 rejects the string.
    let outline = app(b"src\xC0\"", V1).unwrap();
    assert_eq!(dirs(&outline), [b"src\xC0\"".as_slice()]);
    let Outline::App(a) = &outline else {
        unreachable!()
    };
    let chars: Vec<i64> = utf8::unchecked_chars(a.source_dirs[0].raw()).collect();
    assert_eq!(chars, [0x73, 0x72, 0x63, -94]);
    assert_eq!(a.source_dirs[0].path(), None);
    let written = outline.write();
    assert!(written.windows(6).any(|w| w == b"\"src\xA2\""));
    assert!(app(b"src\xC0\"", V2).is_err());
}

#[test]
fn outlines_check_core_and_json() {
    let no_json = String::from_utf8(app_src(b"src"))
        .unwrap()
        .replace(r#""elm/json":"1.1.3","#, "");
    assert_eq!(
        Outline::read(no_json.as_bytes(), V3),
        Err(OutlineError::NoAppJson)
    );
    let indirect = no_json.replace(r#""indirect":{}},"#, r#""indirect":{"elm/json":"1.1.3"}},"#);
    assert!(Outline::read(indirect.as_bytes(), V3).is_ok());
    let no_core = String::from_utf8(app_src(b"src"))
        .unwrap()
        .replace(r#","elm/core":"1.0.5""#, "");
    assert_eq!(
        Outline::read(no_core.as_bytes(), V3),
        Err(OutlineError::NoAppCore)
    );
}

#[test]
fn exposed_modules_keep_their_sections() {
    let src = br#"{"type":"package","name":"elm/core","summary":"s","license":"BSD-3-Clause",
"version":"1.0.5","exposed-modules":{"B":["Basics"],"A":["List","Maybe"]},
"elm-version":"0.19.0 <= v < 0.20.0","dependencies":{},"test-dependencies":{}}"#;
    let Ok(Outline::Pkg(pkg)) = Outline::read(src, V3) else {
        panic!("not a package")
    };
    let Exposed::Dict(sections) = &pkg.exposed else {
        panic!("not sections")
    };
    let headers: Vec<&[u8]> = sections.iter().map(|(h, _)| &**h).collect();
    assert_eq!(headers, [b"B".as_slice(), b"A"]);
    let modules: Vec<&[u8]> = pkg.exposed.modules().map(ModuleName::as_bytes).collect();
    assert_eq!(modules, [b"Basics".as_slice(), b"List", b"Maybe"]);
}

#[test]
fn exposed_modules_are_a_list_or_else_sections() {
    let src = String::from_utf8(pkg("0.19.0 <= v < 0.20.0", "A")).unwrap();
    let src = src.replace(r#"["A"]"#, r#""A""#);
    let Err(OutlineError::Decode(Problem::Field(key, problem))) = Outline::read(src.as_bytes(), V3)
    else {
        panic!("decoded")
    };
    assert_eq!(key, b"exposed-modules");
    let Problem::OneOf(list, dict) = *problem else {
        panic!("{problem:?}")
    };
    assert!(matches!(*list, Problem::Expecting(_, Expectation::Array)));
    assert!(matches!(
        dict[..],
        [Problem::Expecting(_, Expectation::Object)]
    ));
}

fn pkg(elm: &str, module: &str) -> Vec<u8> {
    format!(
        r#"{{"type":"package","name":"a/b","summary":"s","license":"MIT","version":"1.0.0",
"exposed-modules":["{module}"],"elm-version":"{elm}",
"dependencies":{{"elm/core":"1.0.0 <= v < 2.0.0"}},"test-dependencies":{{}}}}"#
    )
    .into_bytes()
}

#[test]
fn applications_are_built_as_the_release_they_name() {
    let src = String::from_utf8(app_src(b"src")).unwrap();
    for (elm, compat) in [("0.19.1", V1), ("0.19.2", V2), ("0.19.3", V3)] {
        let src = src.replace("0.19.1", elm);
        assert_eq!(Outline::detect(src.as_bytes()).unwrap().1, compat);
    }
    let src = src.replace("0.19.1", "0.19.0");
    assert_eq!(
        Outline::detect(src.as_bytes()),
        Err(DetectError::AppElm(v(0, 19, 0)))
    );
}

#[test]
fn packages_are_built_as_the_newest_release_they_admit() {
    for (elm, compat) in [
        ("0.19.0 <= v < 0.20.0", V3),
        ("0.19.0 <= v < 0.19.3", V2),
        ("0.19.1 <= v < 0.19.2", V1),
    ] {
        assert_eq!(Outline::detect(&pkg(elm, "A")).unwrap().1, compat);
    }
    let range = constraint("0.18.0 <= v < 0.19.0", V3);
    assert_eq!(
        Outline::detect(&pkg("0.18.0 <= v < 0.19.0", "A")),
        Err(DetectError::PkgElm(range.unwrap()))
    );
}

#[test]
fn outlines_are_read_as_the_release_they_name() {
    // Only 0.19.1 reads this file, so it names its release itself.
    let src = app_src(b"src\xC0\"");
    assert!(matches!(Outline::detect(&src), Ok((_, V1))));
    // 0.19.3 reads a 300-byte module name, but 0.19.1 doesn't.
    let long = "A".repeat(300);
    assert!(Outline::detect(&pkg("0.19.0 <= v < 0.20.0", &long)).is_ok());
    assert!(matches!(
        Outline::detect(&pkg("0.19.1 <= v < 0.19.2", &long)),
        Err(DetectError::Outline(V1, OutlineError::Decode(_)))
    ));
    // A file no release reads names none: the newest one's error.
    assert!(matches!(
        Outline::detect(b"{"),
        Err(DetectError::Outline(V3, OutlineError::Parse(_)))
    ));
}
