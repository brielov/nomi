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

fn union(vars: &[&str], ctors: &[&str]) -> canonical::Union {
    canonical::Union {
        vars: vars.iter().map(|v| (*v).to_owned()).collect(),
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
        vec![("Int", Union::Open(union(&[], &[])))],
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
    basics.binops.insert(
        "*".to_owned(),
        Binop {
            function: "mul".to_owned(),
            annotation: value(int()),
            associativity: Associativity::Left,
            precedence: 7,
        },
    );
    let mut list = core(&["map"], vec![("List", Union::Open(union(&["a"], &[])))]);
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
            ("T".to_owned(), Union::Open(union(&[], &["C"]))),
            ("Hidden".to_owned(), Union::Closed(union(&[], &["Secret"]))),
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
                vec![("Maybe", Union::Open(union(&["a"], &["Just", "Nothing"])))],
            ),
        ),
        (
            "Result".to_owned(),
            core(
                &[],
                vec![("Result", Union::Open(union(&[], &["Ok", "Err"])))],
            ),
        ),
        (
            "String".to_owned(),
            core(&["fromInt"], vec![("String", Union::Open(union(&[], &[])))]),
        ),
        (
            "Char".to_owned(),
            core(&[], vec![("Char", Union::Open(union(&[], &[])))]),
        ),
        ("Debug".to_owned(), core(&["todo"], vec![])),
        ("Tuple".to_owned(), core(&["first"], vec![])),
        (
            "Platform".to_owned(),
            core(&[], vec![("Program", Union::Open(union(&[], &[])))]),
        ),
        (
            "Platform.Cmd".to_owned(),
            core(&["none"], vec![("Cmd", Union::Open(union(&[], &[])))]),
        ),
        (
            "Platform.Sub".to_owned(),
            core(&["none"], vec![("Sub", Union::Open(union(&[], &[])))]),
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
        nomi_base::Compat::V0_19_1,
        &main,
        interfaces,
        &with_defaults(&main.package, &imports(text)),
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

/// The HIR of `text`, a module of one annotated value, and its
/// annotation's type.
fn annotated(text: &str) -> (hir::Module, hir::TypeId) {
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let hir::Def::Define { annotation, .. } = &module.defs[module.values[0]] else {
        panic!("no value");
    };
    let ty = annotation.expect("an annotation");
    (module, ty)
}

fn canonical_type(text: &str) -> Result<Type, Vec<Error>> {
    let interfaces = interfaces();
    let env = env(&interfaces, text).unwrap();
    let (module, ty) = annotated(text);
    super::types::canonicalize(&env, &module, ty)
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
    let a_t = Type::Type(home("author/project", "A"), "T".to_owned(), vec![]);
    assert_eq!(
        *arg,
        Type::Type(home("elm/core", "Maybe"), "Maybe".to_owned(), vec![a_t])
    );
    assert!(
        matches!(*result, Type::Type(h, n, args) if h.module == "List" && n == "List" && args.len() == 1)
    );
    let errors =
        canonical_type("module Main exposing (..)\n\n\nx : Int String\nx = 1\n").unwrap_err();
    assert!(
        matches!(&errors[..], [Error::BadArity { name, expected: 0, actual: 1, .. }] if name == "Int")
    );
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
    assert!(
        matches!(&errors[..], [Error::DuplicateField { name, first, second }] if name == "a" && first.start() < second.start())
    );
}

#[test]
fn a_later_place_comes_first_unless_a_fold_adds_from_the_right() {
    use super::dups::Dict;
    let at = |n: u32| nomi_base::TextRange::new(n.into(), (n + 1).into());
    let mut dict = Dict::default();
    dict.insert(nomi_base::Compat::V0_19_1, "x", at(1), ());
    dict.insert(nomi_base::Compat::V0_19_1, "x", at(5), ());
    let errors = dict
        .detect(nomi_base::Compat::V0_19_1, |name, first, second| {
            Error::DuplicateDecl {
                name: name.to_owned(),
                first,
                second,
            }
        })
        .unwrap_err();
    assert!(
        matches!(&errors[..], [Error::DuplicateDecl { first, second, .. }] if *first == at(5) && *second == at(1))
    );
    let fields = vec![("a".to_owned(), at(1), ()), ("a".to_owned(), at(5), ())];
    let errors = super::dups::check_fields(nomi_base::Compat::V0_19_1, fields).unwrap_err();
    assert!(matches!(&errors[..], [Error::DuplicateField { first, .. }] if *first == at(1)));
}

fn local(text: &str) -> Result<super::local::Unions, Vec<Error>> {
    let interfaces = interfaces();
    let env = env(&interfaces, text).unwrap();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    super::local::add(&module, env).map(|(env, unions, _)| {
        // The module's own values replace imported ones.
        if let Some(var) = env.vars.get("identity") {
            assert!(matches!(var, Var::TopLevel(_)));
        }
        unions
    })
}

fn titles(errors: &[Error]) -> Vec<String> {
    errors
        .iter()
        .map(|e| {
            format!("{e:?}")
                .split([' ', '{', '('])
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
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
    let errors =
        |text: &str| titles(&local(&format!("module Main exposing (..)\n\n\n{text}")).unwrap_err());
    assert_eq!(errors("x =\n    1\n\n\nx =\n    2\n"), ["DuplicateDecl"]);
    assert_eq!(
        errors("type T\n    = A\n\n\ntype alias T =\n    Int\n"),
        ["DuplicateType"]
    );
    assert_eq!(
        errors("type T\n    = C\n\n\ntype U\n    = C\n"),
        ["DuplicateCtor"]
    );
    assert_eq!(errors("type U\n    = U a b\n"), ["TypeVarsUnboundInUnion"]);
    assert_eq!(
        errors("type alias A a =\n    Int\n"),
        ["TypeVarsMessedUpInAlias"]
    );
    assert_eq!(errors("type alias A a a =\n    a\n"), ["DuplicateAliasArg"]);
    assert_eq!(
        errors("type alias A =\n    { a : A }\n"),
        ["RecursiveAlias"]
    );
    assert_eq!(
        errors("type alias A =\n    { b : B }\n\n\ntype alias B =\n    { a : A }\n"),
        ["RecursiveAlias"]
    );
    // Types fail first and stop the values.
    assert_eq!(
        errors("type U\n    = U a\n\n\nx =\n    1\n\n\nx =\n    2\n"),
        ["TypeVarsUnboundInUnion"]
    );
    // A value defined twice is met at its later definition first, as
    // upstream keeps values in reverse, and reported at the later.
    let text = "module Main exposing (..)\n\n\nx =\n    1\n\n\nx =\n    2\n";
    let Err(errors) = local(text) else { panic!() };
    let Error::DuplicateDecl { first, second, .. } = &errors[0] else {
        panic!()
    };
    assert!(first.start() < second.start());
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

fn parameters(text: &str) -> Vec<String> {
    let interfaces = interfaces();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let package = "author/project";
    match super::module::canonicalize(nomi_base::Compat::V0_19_1, package, &interfaces, &module) {
        Ok(_) => Vec::new(),
        Err(errors) => titles(&errors),
    }
}

#[test]
fn parameters_are_checked_before_their_duplicates() {
    let header = "module Main exposing (..)\n\n\n";
    assert_eq!(
        parameters(&format!("{header}f a a =\n    a\n")),
        ["DuplicatePattern"]
    );
    // A pattern that fails shows only its own errors.
    assert_eq!(
        parameters(&format!("{header}f (Juts a) a =\n    a\n")),
        ["NotFound"]
    );
    assert_eq!(
        parameters(&format!("{header}f : Int\nf a b =\n    a\n")),
        ["AnnotationTooShort"]
    );
    assert_eq!(
        parameters(&format!("{header}g =\n    1\n\n\nf g =\n    g\n")),
        ["Shadowing"]
    );
    assert_eq!(
        parameters(&format!("{header}f (Just x y) =\n    1\n")),
        ["BadArity"]
    );
}

#[test]
fn references_record_what_each_name_resolved_to() {
    use super::references::Target;
    let text = "module Main exposing (main, Color(..))\n\nimport A exposing (a, T(..))\n\ntype Color = Red\n\nmain : Int\nmain =\n    let y = a in\n    case Red of\n        Red -> y + A.x + identity 1\n";
    let interfaces = interfaces();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let (canonical, references) = super::module::canonicalize_with_references(
        nomi_base::Compat::V0_19_1,
        "author/project",
        &interfaces,
        &module,
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
    let text = "module Main exposing (..)\n\nimport A exposing (T(..))\n\n\nf x =\n    let\n        a = 1 + 2 * x\n        g n = h n\n        h n = g n\n    in\n    ( a, C, g )\n";
    let interfaces = interfaces();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let canonical = super::module::canonicalize(
        nomi_base::Compat::V0_19_1,
        "author/project",
        &interfaces,
        &module,
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
                    let ExprKind::Binop(op, _, function, _, left, right) = &body.kind else {
                        panic!("an operator: {body:?}");
                    };
                    assert_eq!((op.as_str(), function.as_str()), ("+", "add"));
                    assert!(matches!(left.kind, ExprKind::Int(1)));
                    assert!(
                        matches!(&right.kind, ExprKind::Binop(op, ..) if op == "*"),
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
                    matches!(&ctor.kind, ExprKind::VarCtor(_, home, name, 0, _) if home.module == "A" && name == "C"),
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

#[test]
fn a_module_constrains_its_values_in_dependency_order() {
    use crate::types::{Constraint, Variables, constrain::module::constrain};
    let text = "module Main exposing (..)\n\n\ng =\n    f 1\n\n\nf x =\n    x\n";
    let interfaces = interfaces();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let canonical = super::module::canonicalize(
        nomi_base::Compat::V0_19_1,
        "author/project",
        &interfaces,
        &module,
    )
    .unwrap();
    let mut vars = Variables::default();
    let mut at = constrain(&mut vars, &canonical);
    let mut names = Vec::new();
    loop {
        match at {
            Constraint::Let {
                header, body_con, ..
            } if !header.is_empty() => {
                names.extend(header.keys().cloned());
                at = *body_con;
            }
            Constraint::SaveTheEnvironment => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    // `f` before `g`, which uses it.
    assert_eq!(names, ["f", "g"]);
}

/// The annotations inference gives `text`'s values, or its errors.
fn infer(text: &str) -> Result<BTreeMap<String, String>, Vec<crate::types::solve::TypeError>> {
    use crate::types::{Variables, constrain::module::constrain, solve::run};
    let interfaces = interfaces();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let canonical = super::module::canonicalize(
        nomi_base::Compat::V0_19_1,
        "author/project",
        &interfaces,
        &module,
    )
    .unwrap();
    let mut vars = Variables::default();
    let constraint = constrain(&mut vars, &canonical);
    let annotations = run(&mut vars, &constraint)?;
    fn show(t: &Type) -> String {
        match t {
            Type::Var(v) => v.clone(),
            Type::Lambda(a, b) => format!("({} -> {})", show(a), show(b)),
            Type::Type(_, name, args) if args.is_empty() => name.clone(),
            Type::Type(_, name, args) => {
                format!(
                    "({name} {})",
                    args.iter().map(show).collect::<Vec<_>>().join(" ")
                )
            }
            Type::Tuple(a, b, None) => format!("( {}, {} )", show(a), show(b)),
            other => format!("{other:?}"),
        }
    }
    Ok(annotations
        .into_iter()
        .map(|(n, a)| (n, show(&a.ty)))
        .collect())
}

#[test]
fn inference_generalizes_lets_and_finds_mismatches() {
    let found = infer(
        "module Main exposing (..)\n\nimport A\n\n\nidentity2 x =\n    x\n\n\npair =\n    ( identity2 1, identity2 \"s\" )\n\n\nuse =\n    let\n        f y = y\n    in\n    ( f A.x, f [] )\n",
    )
    .unwrap();
    assert_eq!(found["identity2"], "(a -> a)");
    assert_eq!(found["pair"], "( number, String )");
    assert_eq!(found["use"], "( Int, (List a) )");
    // `a -> a` takes two arguments where `a` is a function, so the first
    // is the problem, as elm reports it.
    let errors = infer("module Main exposing (..)\n\n\nbad =\n    identityish 1 \"s\"\n\n\nidentityish x =\n    x\n").unwrap_err();
    assert!(
        matches!(
            &errors[..],
            [crate::types::solve::TypeError::BadExpr(
                _,
                crate::types::expected::Category::Number,
                _,
                crate::types::expected::Expected::FromContext(
                    _,
                    crate::types::expected::Context::CallArg(_, 0),
                    _
                ),
            )]
        ),
        "{errors:?}"
    );
}

#[test]
fn inference_records_each_expressions_type() {
    use crate::types::error::Type as E;
    let text = "module Main exposing (..)\n\n\nxs =\n    [ 1, 2 ]\n";
    let interfaces = interfaces();
    let parse = parse_module(text);
    let module = hir::lower(&ast::Module::cast(parse.syntax()).unwrap());
    let canonical = super::module::canonicalize(
        nomi_base::Compat::V0_19_1,
        "author/project",
        &interfaces,
        &module,
    )
    .unwrap();
    let (result, types) = crate::types::infer_types(&canonical);
    assert!(result.is_ok());
    let shown: Vec<(&str, String)> = (types.iter())
        .map(|(region, ty)| {
            let ty = match ty {
                E::FlexSuper(_, name) => name.clone(),
                E::Type(_, name, args) => format!("{name} {}", args.len()),
                other => format!("{other:?}"),
            };
            (text[*region].trim(), ty)
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("[ 1, 2 ]", "List 1".to_owned()),
            ("1", "number".to_owned()),
            ("2", "number".to_owned())
        ]
    );
}
