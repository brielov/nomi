use std::collections::{BTreeMap, HashMap};

use nomi_base::{Compat, TextRange};

use super::env::{Env, TypeInfo, Var};
use super::error::{Duplicated, Error, NameKind};
use super::foreign::{initial_env, with_defaults};
use crate::canonical::{CtorOpts, Home, Type};
use crate::hir::{self, lowered};
use crate::interface::Interface;
use crate::testing::{canonical, interfaces};

/// Asserts `errors` are one error, matching `pattern`.
macro_rules! one_error {
    ($errors:expr, $pattern:pat $(if $guard:expr)?) => {
        let errors = $errors;
        assert!(matches!(&errors[..], [$pattern] $(if $guard)?), "{errors:?}");
    };
}

fn env<'a>(interfaces: &'a HashMap<String, Interface>, text: &str) -> Result<Env<'a>, Vec<Error>> {
    let main = Home::author_project("Main");
    initial_env(
        Compat::V0_19_1,
        &main,
        interfaces,
        &with_defaults(&main.package, &lowered(text).imports),
        &Default::default(),
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
        matches!(&errors[0], Error::ImportExposingNotFound { name, possible, .. }
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
    assert!(
        matches!(&error("module Main exposing (..)\nimport A exposing (Nope(..))\n")[0],
        Error::ImportExposingNotFound { possible, .. } if possible == &["Hidden", "Point", "T"])
    );
    assert!(
        matches!(&error("module Main exposing (..)\nimport Basics exposing ((+++))\n")[0],
        Error::ImportExposingNotFound { possible, .. } if possible == &["*", "+"])
    );
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

/// The type the annotation of `text`'s one value names.
fn canonical_type(text: &str) -> Result<Type, Vec<Error>> {
    let interfaces = interfaces();
    let env = env(&interfaces, text).unwrap();
    let module = lowered(text);
    let hir::Def::Define { annotation, .. } = &module.defs[module.values[0]] else {
        panic!("no value");
    };
    super::types::canonicalize(&env, &module, annotation.expect("an annotation"))
}

#[test]
fn types_resolve_and_check_their_arity() {
    let ty = canonical_type(
        "module Main exposing (..)\nimport A as X\n\n\nx : Maybe X.T -> List Int\nx = 1\n",
    )
    .unwrap();
    let Type::Lambda(arg, result) = ty else {
        panic!()
    };
    let a_t = Type::Type {
        home: Home::author_project("A"),
        name: "T".to_owned(),
        args: vec![],
    };
    let maybe = Type::Type {
        home: Home::core("Maybe"),
        name: "Maybe".to_owned(),
        args: vec![a_t],
    };
    assert_eq!(*arg, maybe);
    assert!(
        matches!(*result, Type::Type { home, name, args } if home.module == "List" && name == "List" && args.len() == 1)
    );
    let errors =
        canonical_type("module Main exposing (..)\n\n\nx : Int String\nx = 1\n").unwrap_err();
    one_error!(errors, Error::BadArity { name, expected: 0, actual: 1, .. } if name == "Int");
}

#[test]
fn type_errors_collect_as_upstream_collects_them() {
    // Both sides of an arrow are canonicalized; a missing type stops
    // its own arguments.
    let errors =
        canonical_type("module Main exposing (..)\n\n\nx : Nope Gone -> Itn\nx = 1\n").unwrap_err();
    let names: Vec<_> = errors
        .iter()
        .map(|e| match e {
            Error::NotFound { name, .. } => name.as_str(),
            e => panic!("{e:?}"),
        })
        .collect();
    assert_eq!(names, ["Nope", "Itn"]);
    // A tuple's items, then its size.
    let errors =
        canonical_type("module Main exposing (..)\n\n\nx : ( Itn, Int, Int, Int )\nx = 1\n")
            .unwrap_err();
    assert!(matches!(
        &errors[..],
        [Error::NotFound { .. }, Error::TupleLargerThanThree { .. }]
    ));
    // A field written twice stops the record before its types.
    let errors =
        canonical_type("module Main exposing (..)\n\n\nx : { a : Itn, b : Int, a : Int }\nx = 1\n")
            .unwrap_err();
    one_error!(
        errors,
        Error::Duplicate { what: Duplicated::Field, name, first, second }
            if name == "a" && first.start() < second.start()
    );
}

#[test]
fn a_later_place_comes_first_unless_a_fold_adds_from_the_right() {
    use super::dups::{Dict, check_fields};
    let at = |n: u32| TextRange::new(n.into(), (n + 1).into());
    let mut dict = Dict::default();
    dict.insert(Compat::V0_19_1, "x", at(1), ());
    dict.insert(Compat::V0_19_1, "x", at(5), ());
    let errors = (dict.detect(Compat::V0_19_1, Duplicated::Decl.error())).unwrap_err();
    one_error!(errors, Error::Duplicate { first, second, .. } if *first == at(5) && *second == at(1));
    let a = |n| hir::Spanned {
        text: "a".to_owned(),
        range: at(n),
    };
    let fields = [(a(1), ()), (a(5), ())];
    let errors = check_fields(Compat::V0_19_1, fields.iter().map(|(n, v)| (n, *v))).unwrap_err();
    one_error!(errors, Error::Duplicate { first, .. } if *first == at(1));
}

fn local(text: &str) -> Result<super::local::Unions, Vec<Error>> {
    let interfaces = interfaces();
    let env = env(&interfaces, text).unwrap();
    super::local::add(&lowered(text), env).map(|(env, unions, _)| {
        // The module's own values replace imported ones.
        if let Some(var) = env.vars.get("identity") {
            assert!(matches!(var, Var::TopLevel(_)));
        }
        unions
    })
}

#[test]
fn local_declarations_join_the_imports() {
    let unions = local(
        "module Main exposing (..)\n\n\ntype Shape a\n    = Circle a\n    | Dot\n\n\ntype alias Point =\n    { x : Int }\n\n\nidentity =\n    1\n",
    )
    .unwrap();
    assert_eq!(unions["Shape"].ctors.len(), 2);
    assert_eq!(unions["Shape"].opts, CtorOpts::Normal);
}

#[test]
fn local_errors_are_upstreams() {
    let errors = |text: &str| local(&format!("module Main exposing (..)\n\n\n{text}")).unwrap_err();
    one_error!(
        errors("x =\n    1\n\n\nx =\n    2\n"),
        Error::Duplicate {
            what: Duplicated::Decl,
            ..
        }
    );
    one_error!(
        errors("type T\n    = A\n\n\ntype alias T =\n    Int\n"),
        Error::Duplicate {
            what: Duplicated::Type,
            ..
        }
    );
    one_error!(
        errors("type T\n    = C\n\n\ntype U\n    = C\n"),
        Error::Duplicate {
            what: Duplicated::Ctor,
            ..
        }
    );
    one_error!(
        errors("type U\n    = U a b\n"),
        Error::TypeVarsUnboundInUnion { .. }
    );
    one_error!(
        errors("type alias A a =\n    Int\n"),
        Error::TypeVarsMessedUpInAlias { .. }
    );
    one_error!(
        errors("type alias A a a =\n    a\n"),
        Error::DuplicateAliasArg { .. }
    );
    one_error!(
        errors("type alias A =\n    { a : A }\n"),
        Error::RecursiveAlias { .. }
    );
    one_error!(
        errors("type alias A =\n    { b : B }\n\n\ntype alias B =\n    { a : A }\n"),
        Error::RecursiveAlias { .. }
    );
    // Types fail first and stop the values.
    one_error!(
        errors("type U\n    = U a\n\n\nx =\n    1\n\n\nx =\n    2\n"),
        Error::TypeVarsUnboundInUnion { .. }
    );
    // A value defined twice is met at its later definition first, as
    // upstream keeps values in reverse, and reported at the later.
    one_error!(
        errors("x =\n    1\n\n\nx =\n    2\n"),
        Error::Duplicate { first, second, .. } if first.start() < second.start()
    );
}

#[test]
fn unbound_variables_are_placed_by_the_first_constructor_using_them() {
    let text = "module Main exposing (..)\n\n\ntype U\n    = A (List b) b\n    | B b\n";
    let Err(errors) = local(text) else { panic!() };
    let Error::TypeVarsUnboundInUnion { first, rest, .. } = &errors[0] else {
        panic!()
    };
    assert!(rest.is_empty());
    // The last place in the first constructor.
    assert_eq!(&text[first.range].trim(), &"b");
    // Node ranges keep leading trivia, which the report trims.
    assert_eq!(
        u32::from(first.range.end()) as usize,
        text.find(") b").unwrap() + 3
    );
}

#[test]
fn parameters_are_checked_before_their_duplicates() {
    let errors =
        |text: &str| canonical(&format!("module Main exposing (..)\n\n\n{text}")).unwrap_err();
    one_error!(errors("f a a =\n    a\n"), Error::DuplicatePattern { .. });
    // A pattern that fails shows only its own errors.
    one_error!(errors("f (Juts a) a =\n    a\n"), Error::NotFound { .. });
    one_error!(
        errors("f : Int\nf a b =\n    a\n"),
        Error::AnnotationTooShort { .. }
    );
    one_error!(
        errors("g =\n    1\n\n\nf g =\n    g\n"),
        Error::Shadowing { .. }
    );
    one_error!(errors("f (Just x y) =\n    1\n"), Error::BadArity { .. });
}

#[test]
fn references_record_what_each_name_resolved_to() {
    use super::references::Target;
    let text = "module Main exposing (main, Color(..))\n\nimport A exposing (a, T(..))\n\ntype Color = Red\n\nmain : Int\nmain =\n    let y = a in\n    case Red of\n        Red -> y + A.x + identity 1\n";
    let (canonical, references) = super::module::canonicalize_with_references(
        Compat::V0_19_1,
        "author/project",
        &interfaces(),
        &lowered(text),
    );
    assert!(canonical.is_ok());
    let found: Vec<String> = (references.iter())
        .map(|r| {
            let at = text[r.range].trim();
            match &r.target {
                Target::Local(binder) => format!("{at} -> local {}", text[*binder].trim()),
                Target::Global { home, name, kind } => {
                    format!("{at} -> {kind:?} {}.{name}", home.module)
                }
            }
        })
        .collect();
    assert_eq!(
        found,
        [
            "main -> Value Main.main",
            "Color -> Type Main.Color",
            "A -> Module A.A",
            "a -> Value A.a",
            "T -> Type A.T",
            "Int -> Type Basics.Int",
            "a -> Value A.a",
            "Red -> Ctor Main.Red",
            "Red -> Ctor Main.Red",
            "y -> local y",
            "+ -> Binop Basics.+",
            "A.x -> Value A.x",
            "+ -> Binop Basics.+",
            "identity -> Value Basics.identity",
        ]
    );
}

#[test]
fn canonicalization_builds_upstreams_tree() {
    use crate::canonical::{Decl, Def as CDef, ExprKind};
    let canonical = canonical(
        "module Main exposing (..)\n\nimport A exposing (T(..))\n\n\nf x =\n    let\n        a = 1 + 2 * x\n        g n = h n\n        h n = g n\n    in\n    ( a, C, g )\n",
    )
    .unwrap();
    let [
        Decl::Declare(CDef::Def {
            name, params, body, ..
        }),
    ] = &canonical.decls[..]
    else {
        panic!("one declaration: {:?}", canonical.decls);
    };
    assert_eq!((name.as_str(), params.len()), ("f", 1));
    // The functions that use each other are one `LetRec`, `a` its own
    // `Let`, a binding before what uses it.
    let mut kinds = Vec::new();
    let mut at = body;
    loop {
        match &at.kind {
            ExprKind::Let(def, rest) => {
                kinds.push(format!("let {}", def.name()));
                if def.name() == "a" {
                    let CDef::Def { body, .. } = &**def else {
                        panic!("untyped")
                    };
                    let ExprKind::Binop {
                        operator,
                        left,
                        right,
                    } = &body.kind
                    else {
                        panic!("an operator: {body:?}");
                    };
                    assert_eq!(
                        (operator.op.as_str(), operator.function.as_str()),
                        ("+", "add")
                    );
                    assert!(matches!(left.kind, ExprKind::Int(1)));
                    assert!(
                        matches!(&right.kind, ExprKind::Binop { operator, .. } if operator.op == "*"),
                        "{right:?}"
                    );
                }
                at = rest;
            }
            ExprKind::LetRec(defs, rest) => {
                let names: Vec<&str> = defs.iter().map(|d| d.name()).collect();
                kinds.push(format!("rec {}", names.join(" ")));
                at = rest;
            }
            ExprKind::Tuple(_, ctor, _) => {
                assert!(
                    matches!(&ctor.kind, ExprKind::VarCtor { home, name, index: 0, .. } if home.module == "A" && name == "C"),
                    "{ctor:?}"
                );
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(
        kinds.contains(&"let a".to_owned()) && kinds.iter().any(|k| k.starts_with("rec ")),
        "{kinds:?}"
    );
}
