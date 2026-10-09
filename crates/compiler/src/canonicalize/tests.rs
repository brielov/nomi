use std::collections::{BTreeMap, HashMap};

use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::parse_module;

use super::env::{Env, TypeInfo, Var};
use super::error::{Error, NameKind};
use super::foreign::{initial_env, with_defaults};
use crate::canonical::{self, Annotation, Associativity, Ctor, CtorOpts, Home, Type};
use crate::hir;
use crate::interface::{Alias, Binop, Interface, Union};

fn home(package: &str, module: &str) -> Home {
    Home {
        package: package.to_owned(),
        module: module.to_owned(),
    }
}

fn int() -> Type {
    Type::Type(home("elm/core", "Basics"), "Int".to_owned(), vec![])
}

fn value(ty: Type) -> Annotation {
    Annotation { free: vec![], ty }
}

fn union(ctors: &[&str]) -> canonical::Union {
    canonical::Union {
        vars: vec![],
        ctors: (ctors.iter().enumerate())
            .map(|(index, name)| Ctor {
                name: (*name).to_owned(),
                index,
                args: vec![],
            })
            .collect(),
        count: ctors.len(),
        opts: CtorOpts::Enum,
    }
}

/// Interfaces like elm/core's defaults, plus local modules `A` and `B`,
/// which both expose `x`, a type `T` and a constructor `C`.
fn interfaces() -> HashMap<String, Interface> {
    let core = |values: &[&str], unions: Vec<(&str, Union)>| Interface {
        home: "elm/core".to_owned(),
        values: values
            .iter()
            .map(|v| ((*v).to_owned(), value(int())))
            .collect(),
        unions: unions.into_iter().map(|(n, u)| (n.to_owned(), u)).collect(),
        ..Interface::default()
    };
    let mut basics = core(
        &["identity", "toFloat"],
        vec![("Int", Union::Open(union(&[])))],
    );
    basics.binops.insert(
        "+".to_owned(),
        Binop {
            function: "add".to_owned(),
            annotation: value(int()),
            associativity: Associativity::Left,
            precedence: 6,
        },
    );
    let mut list = core(&["map"], vec![("List", Union::Open(union(&[])))]);
    list.binops.insert(
        "::".to_owned(),
        Binop {
            function: "cons".to_owned(),
            annotation: value(int()),
            associativity: Associativity::Right,
            precedence: 5,
        },
    );
    let local = |extra: &str| Interface {
        home: "author/project".to_owned(),
        values: BTreeMap::from([
            ("x".to_owned(), value(int())),
            (extra.to_owned(), value(int())),
        ]),
        unions: BTreeMap::from([
            ("T".to_owned(), Union::Open(union(&["C"]))),
            ("Hidden".to_owned(), Union::Closed(union(&["Secret"]))),
        ]),
        aliases: BTreeMap::from([(
            "Point".to_owned(),
            Alias::Public(canonical::Alias {
                vars: vec![],
                ty: Type::Record(BTreeMap::new(), None),
            }),
        )]),
        binops: BTreeMap::new(),
    };
    HashMap::from([
        ("Basics".to_owned(), basics),
        ("List".to_owned(), list),
        (
            "Maybe".to_owned(),
            core(
                &["withDefault"],
                vec![("Maybe", Union::Open(union(&["Just", "Nothing"])))],
            ),
        ),
        (
            "Result".to_owned(),
            core(&[], vec![("Result", Union::Open(union(&["Ok", "Err"])))]),
        ),
        (
            "String".to_owned(),
            core(&["fromInt"], vec![("String", Union::Open(union(&[])))]),
        ),
        (
            "Char".to_owned(),
            core(&[], vec![("Char", Union::Open(union(&[])))]),
        ),
        ("Debug".to_owned(), core(&["todo"], vec![])),
        ("Tuple".to_owned(), core(&["first"], vec![])),
        (
            "Platform".to_owned(),
            core(&[], vec![("Program", Union::Open(union(&[])))]),
        ),
        (
            "Platform.Cmd".to_owned(),
            core(&["none"], vec![("Cmd", Union::Open(union(&[])))]),
        ),
        (
            "Platform.Sub".to_owned(),
            core(&["none"], vec![("Sub", Union::Open(union(&[])))]),
        ),
        ("A".to_owned(), local("a")),
        ("B".to_owned(), local("b")),
    ])
}

fn imports(text: &str) -> Vec<hir::Import> {
    let parse = parse_module(text);
    hir::lower(&ast::Module::cast(parse.syntax()).unwrap()).imports
}

fn env<'a>(interfaces: &'a HashMap<String, Interface>, text: &str) -> Result<Env<'a>, Vec<Error>> {
    let main = home("author/project", "Main");
    initial_env(
        &main,
        interfaces,
        &with_defaults(&main.package, &imports(text)),
    )
}

#[test]
fn defaults_bring_basics_maybe_and_qualified_names() {
    let interfaces = interfaces();
    let env = env(&interfaces, "module Main exposing (..)\n").unwrap();
    assert!(matches!(env.vars["identity"], Var::Foreign(_)));
    assert!(!env.vars.contains_key("map"));
    assert!(env.ctors.contains_key("Just"));
    assert!(env.types.contains_key("Cmd") && env.types.contains_key("List"));
    assert!(env.q_vars["Cmd"].contains_key("none"));
    assert!(env.q_vars["List"].contains_key("map"));
    assert!(env.binops.contains_key("+") && env.binops.contains_key("::"));
    assert!(env.find_binop(Default::default(), "+").is_ok());
}

#[test]
fn an_explicit_type_import_overwrites_and_an_open_one_merges() {
    let interfaces = interfaces();
    let env = env(
        &interfaces,
        "module Main exposing (..)\nimport A exposing (T)\nimport B exposing (T)\n",
    )
    .unwrap();
    let Ok(TypeInfo::Union { home, .. }) = env.find_type(Default::default(), None, "T") else {
        panic!("T is ambiguous");
    };
    assert_eq!(home.module, "B");
    let open = env_open(&interfaces);
    assert!(matches!(
        open.find_type(Default::default(), None, "T"),
        Err(e) if matches!(&e[0], Error::Ambiguous { kind: NameKind::Type, homes, .. } if homes.len() == 2)
    ));
    assert!(matches!(&open.vars["x"], Var::Foreign(homes) if homes.len() == 2));
    assert!(open.find_ctor(Default::default(), None, "C").is_err());
}

fn env_open(interfaces: &HashMap<String, Interface>) -> Env<'_> {
    env(
        interfaces,
        "module Main exposing (..)\nimport A exposing (..)\nimport B exposing (..)\n",
    )
    .unwrap()
}

#[test]
fn aliases_qualify_and_closed_unions_hide_their_constructors() {
    let interfaces = interfaces();
    let env = env(
        &interfaces,
        "module Main exposing (..)\nimport A as X exposing (Hidden, Point)\n",
    )
    .unwrap();
    assert!(env.find_type(Default::default(), Some("X"), "T").is_ok());
    assert!(env.find_ctor(Default::default(), Some("X"), "C").is_ok());
    assert!(
        env.find_ctor(Default::default(), Some("X"), "Secret")
            .is_err()
    );
    assert!(env.types.contains_key("Hidden"));
    // A record alias exposes its constructor.
    assert!(env.ctors.contains_key("Point"));
    let Err(errors) = env.find_type(Default::default(), Some("A"), "T") else {
        panic!("A is aliased");
    };
    assert!(matches!(&errors[0], Error::NotFound { prefix: Some(p), .. } if p == "A"));
}

#[test]
fn import_errors_stop_at_the_first() {
    let interfaces = interfaces();
    let error = |text: &str| env(&interfaces, text).unwrap_err();
    let errors =
        error("module Main exposing (..)\nimport A exposing (y, Nope)\nimport B exposing (z)\n");
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], Error::ImportExposingNotFound { kind: NameKind::Value, name, possible, .. }
        if name == "y" && possible == &["a", "x"])
    );
    assert!(
        matches!(&error("module Main exposing (..)\nimport A exposing (C)\n")[0],
        Error::ImportCtorByName { ctor, union, .. } if ctor == "C" && union == "T")
    );
    assert!(
        matches!(&error("module Main exposing (..)\nimport A exposing (Point(..))\n")[0],
        Error::ImportOpenAlias { name, .. } if name == "Point")
    );
    assert!(matches!(
        &error("module Main exposing (..)\nimport A exposing (Nope(..))\n")[0],
        Error::ImportExposingNotFound {
            kind: NameKind::Type,
            ..
        }
    ));
    assert!(matches!(
        &error("module Main exposing (..)\nimport Basics exposing ((+++))\n")[0],
        Error::ImportExposingNotFound {
            kind: NameKind::Operator,
            ..
        }
    ));
}

#[test]
fn locals_shadow_locals_but_not_imports() {
    let interfaces = interfaces();
    let env = env(&interfaces, "module Main exposing (..)\n").unwrap();
    let names = BTreeMap::from([
        ("identity".to_owned(), Default::default()),
        ("n".to_owned(), Default::default()),
    ]);
    let env = env.add_locals(&names).unwrap();
    assert!(matches!(env.vars["identity"], Var::Local(_)));
    let errors = env.add_locals(&names).unwrap_err();
    assert_eq!(errors.len(), 2);
    assert!(matches!(&errors[0], Error::Shadowing { name, .. } if name == "identity"));
}
