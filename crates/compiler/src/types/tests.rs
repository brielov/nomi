use super::*;
use crate::canonical::Type as T;
use nomi_base::Text;

#[test]
fn fresh_names_skip_the_taken_and_number_after_a_digit() {
    let schemes: Vec<String> = [0, 25, 26, 27, 52].map(from_type_variable_scheme).to_vec();
    assert_eq!(schemes, ["a", "z", "a1", "b1", "a2"]);
    assert_eq!(from_type_variable("number", 0), "number");
    assert_eq!(from_type_variable("number", 2), "number2");
    assert_eq!(from_type_variable("a1", 1), "a1_1");
}

#[test]
fn an_annotation_names_unnamed_variables_after_the_users() {
    let mut vars = Variables::default();
    // a -> b -> number, where `a` is the user's and the rest unnamed.
    let user = vars.name_to_flex("a");
    let unnamed = vars.flex_var();
    let number = vars.flex_number();
    let inner = vars.fresh(Content::Structure(FlatType::Fun1(unnamed, number)));
    let outer = vars.fresh(Content::Structure(FlatType::Fun1(user, inner)));
    let annotation = vars.to_annotation(outer);
    let var = |n: &str| Box::new(T::Var(n.to_owned()));
    assert_eq!(
        *annotation.ty,
        T::Lambda(var("a"), Box::new(T::Lambda(var("b"), var("number"))))
    );
    assert_eq!(annotation.free, ["a", "b", "number"]);
}

#[test]
fn one_name_on_two_variables_is_numbered_apart() {
    let mut vars = Variables::default();
    let first = vars.name_to_rigid("a");
    let second = vars.name_to_rigid("a");
    let tuple = vars.fresh(Content::Structure(FlatType::Tuple1(first, second, None)));
    // The second is visited first, so it keeps `a`.
    assert_eq!(
        vars.to_error_type(tuple),
        error::Type::Tuple(
            Box::new(error::Type::RigidVar("a1".to_owned())),
            Box::new(error::Type::RigidVar("a".to_owned())),
            None
        )
    );
}

#[test]
fn an_error_type_flattens_functions_and_cuts_infinite_types() {
    let mut vars = Variables::default();
    let a = vars.flex_var();
    let list = vars.fresh(Content::Structure(FlatType::App1(
        core("List"),
        Text::from("List"),
        vec![a],
    )));
    // a ~ List a.
    let descriptor = vars.get(list).clone();
    vars.union(a, list, descriptor);
    let f = vars.fresh(Content::Structure(FlatType::Fun1(a, a)));
    let g = vars.fresh(Content::Structure(FlatType::Fun1(a, f)));
    let infinite = error::Type::Type(core("List"), "List".to_owned(), vec![error::Type::Infinite]);
    assert_eq!(
        vars.to_error_type(g),
        error::Type::Lambda(
            Box::new(infinite.clone()),
            Box::new(infinite.clone()),
            vec![infinite]
        )
    );
}

mod unification {
    use super::*;
    use crate::types::unify::{Answer, unify};

    fn app(vars: &mut Variables, module: &str, name: &str, args: Vec<Variable>) -> Variable {
        vars.fresh(Content::Structure(FlatType::App1(
            core(module),
            Text::from(name),
            args,
        )))
    }

    fn ok(answer: &Answer) -> bool {
        matches!(answer, Answer::Ok(_))
    }

    #[test]
    fn super_types_take_their_atoms_and_combine() {
        let mut vars = Variables::default();
        let (number, int) = (vars.flex_number(), app(&mut vars, "Basics", "Int", vec![]));
        assert!(ok(&unify(&mut vars, number, int)));
        let number = vars.flex_number();
        let string = app(&mut vars, "String", "String", vec![]);
        let Answer::Err(_, expected, actual) = unify(&mut vars, number, string) else {
            panic!("number isn't String");
        };
        assert_eq!(
            *expected,
            error::Type::FlexSuper(error::Super::Number, "number".to_owned())
        );
        assert_eq!(
            *actual,
            error::Type::Type(core("String"), "String".to_owned(), vec![])
        );
        // An error then unifies with anything.
        let float = app(&mut vars, "Basics", "Float", vec![]);
        assert!(ok(&unify(&mut vars, number, float)));
        let comparable = vars.fresh(Content::FlexSuper(SuperType::Comparable, None));
        let appendable = vars.fresh(Content::FlexSuper(SuperType::Appendable, None));
        assert!(ok(&unify(&mut vars, comparable, appendable)));
        assert_eq!(
            vars.content(comparable),
            Content::FlexSuper(SuperType::CompAppend, None)
        );
    }

    #[test]
    fn a_comparable_list_makes_its_elements_comparable() {
        let mut vars = Variables::default();
        let element = vars.flex_var();
        let list = app(&mut vars, "List", "List", vec![element]);
        let comparable = vars.fresh(Content::FlexSuper(SuperType::Comparable, None));
        let Answer::Ok(fresh) = unify(&mut vars, comparable, list) else {
            panic!("a list can be comparable");
        };
        assert_eq!(fresh.len(), 1);
        assert_eq!(
            vars.content(element),
            Content::FlexSuper(SuperType::Comparable, None)
        );
    }

    #[test]
    fn records_meet_through_their_extensions() {
        let mut vars = Variables::default();
        let (x, ext1, ext2) = (vars.flex_var(), vars.flex_var(), vars.flex_var());
        let int = app(&mut vars, "Basics", "Int", vec![]);
        // { ext1 | x : x } ~ { ext2 | y : Int }
        let one = vars.fresh(Content::Structure(FlatType::Record1(
            BTreeMap::from([(Text::new("x"), x)]),
            ext1,
        )));
        let two = vars.fresh(Content::Structure(FlatType::Record1(
            BTreeMap::from([(Text::new("y"), int)]),
            ext2,
        )));
        let Answer::Ok(fresh) = unify(&mut vars, one, two) else {
            panic!("open records meet");
        };
        // A shared extension and a record for each side's own fields.
        assert_eq!(fresh.len(), 3);
        let Content::Structure(FlatType::Record1(fields, _)) = vars.content(one) else {
            panic!("a record");
        };
        assert_eq!(fields.keys().collect::<Vec<_>>(), ["x", "y"]);
        // A rigid variable stays itself.
        let (a, b) = (vars.name_to_rigid("a"), vars.name_to_rigid("b"));
        assert!(!ok(&unify(&mut vars, a, b)));
    }

    #[test]
    fn a_type_applied_to_too_few_arguments_still_unifies_the_rest() {
        let mut vars = Variables::default();
        let (a, b) = (vars.flex_var(), vars.flex_var());
        let int = app(&mut vars, "Basics", "Int", vec![]);
        let string = app(&mut vars, "String", "String", vec![]);
        let one = app(&mut vars, "Dict", "Dict", vec![int, a]);
        let two = app(&mut vars, "Dict", "Dict", vec![string, b]);
        assert!(!ok(&unify(&mut vars, one, two)));
        // The second arguments met though the first didn't.
        assert!(vars.equivalent(a, b));
    }
}

mod pattern_constraints {
    use super::*;
    use crate::canonical::{Pattern, PatternKind};
    use crate::types::constrain::pattern::{State, add};
    use crate::types::expected::{PCategory, PExpected};
    use nomi_base::{TextRange, TextSize};

    fn at(start: u32, end: u32, kind: PatternKind) -> Pattern {
        Pattern {
            region: TextRange::new(TextSize::new(start), TextSize::new(end)),
            kind,
        }
    }

    #[test]
    fn a_tuple_binds_its_names_and_matches_after_its_parts() {
        let mut vars = Variables::default();
        // ( x, { a } :: rest )
        let x = at(1, 2, PatternKind::Var("x".to_owned()));
        let record = at(4, 9, PatternKind::Record(vec!["a".to_owned()]));
        let rest = at(13, 17, PatternKind::Var("rest".to_owned()));
        let cons = at(4, 17, PatternKind::Cons(Box::new(record), Box::new(rest)));
        let tuple = at(0, 18, PatternKind::Tuple(Box::new(x), Box::new(cons), None));
        let whole = vars.flex_var();
        let mut state = State::default();
        add(
            &mut vars,
            &tuple,
            PExpected::PNoExpectation(Type::Var(whole)),
            &mut state,
        );
        assert_eq!(state.headers.keys().collect::<Vec<_>>(), ["a", "rest", "x"]);
        // The tail is matched before the head, the parts before the whole.
        let categories: Vec<PCategory> = (state.cons.iter())
            .map(|c| match c {
                Constraint::Pattern(_, category, _, _) => category.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            categories,
            [PCategory::PRecord, PCategory::PList, PCategory::PTuple]
        );
        // The tuple's parts, then the list's entry, then the record's.
        assert_eq!(state.vars.len(), 2 + 1 + 2);
    }
}

mod expression_constraints {
    use super::*;
    use crate::canonical::{Expr, ExprKind, Pattern, PatternKind};
    use crate::types::constrain::expr::constrain;
    use crate::types::expected::{Category, Expected};
    use nomi_base::TextRange;

    #[test]
    fn a_lambda_binds_its_parameter_and_equals_a_function() {
        let mut vars = Variables::default();
        let x = Pattern {
            region: TextRange::default(),
            kind: PatternKind::Var("x".to_owned()),
        };
        let body = Expr {
            region: TextRange::default(),
            kind: ExprKind::VarLocal("x".to_owned()),
        };
        let lambda = Expr {
            region: TextRange::default(),
            kind: ExprKind::Lambda(vec![x], Box::new(body)),
        };
        let whole = vars.flex_var();
        let con = constrain(
            &mut vars,
            &Default::default(),
            &lambda,
            Expected::NoExpectation(Type::Var(whole)),
        );
        let Constraint::Let {
            flex_vars,
            header_con,
            ..
        } = con
        else {
            panic!("exists");
        };
        // The parameter's variable and the result's.
        assert_eq!(flex_vars.len(), 2);
        let Constraint::And(cons) = *header_con else {
            panic!("and");
        };
        let [
            Constraint::Let {
                header, body_con, ..
            },
            Constraint::Equal(_, Category::Lambda, Type::Fun(..), _),
        ] = &cons[..]
        else {
            panic!("{cons:?}");
        };
        assert!(header.contains_key("x"));
        assert!(matches!(&**body_con, Constraint::Local(_, name, _) if name == "x"));
    }
}

mod comparison {
    use std::collections::BTreeMap;

    use super::*;
    use crate::types::error::{Extension, Problem, Type as E, to_comparison, to_doc};
    use crate::types::localizer::Localizer;
    use crate::types::render::Context;

    fn basics(name: &str) -> E {
        E::Type(core("Basics"), name.to_owned(), vec![])
    }

    fn localizer() -> Localizer {
        Localizer::from_names(Compat::V0_19_1, ["Basics", "Maybe", "List", "String"])
    }

    #[test]
    fn a_comparison_finds_the_problem_to_explain() {
        let l = localizer();
        let (_, _, problems) = to_comparison(&l, &basics("Int"), &basics("Float"));
        assert_eq!(problems, [Problem::IntFloat]);
        let maybe = E::Type(core("Maybe"), "Maybe".to_owned(), vec![basics("Int")]);
        let (_, _, problems) = to_comparison(&l, &maybe, &basics("Int"));
        assert_eq!(problems, [Problem::AnythingFromMaybe]);
        let record = |fields: &[&str]| {
            E::Record(
                fields
                    .iter()
                    .map(|f| ((*f).to_owned(), basics("Int")))
                    .collect::<BTreeMap<_, _>>(),
                Extension::Closed,
            )
        };
        let (_, _, problems) =
            to_comparison(&l, &record(&["nmae", "age"]), &record(&["name", "age"]));
        assert_eq!(
            problems,
            [Problem::FieldTypo(
                "nmae".to_owned(),
                vec!["age".to_owned(), "name".to_owned()]
            )]
        );
    }

    #[test]
    fn functions_print_flat_and_arguments_parenthesized() {
        let l = localizer();
        let list = E::Type(core("List"), "List".to_owned(), vec![basics("Int")]);
        let f = E::Lambda(
            Box::new(E::Lambda(
                Box::new(basics("Int")),
                Box::new(basics("Bool")),
                vec![],
            )),
            Box::new(list),
            vec![E::FlexVar("a".to_owned())],
        );
        assert_eq!(
            to_doc(&l, Context::None, &f).plain(),
            "(Int -> Bool) -> List Int -> a"
        );
        let qualified = E::Type(core("Dict"), "Dict".to_owned(), vec![]);
        assert_eq!(to_doc(&l, Context::None, &qualified).plain(), "Dict.Dict");
    }
}

mod modules {
    use std::collections::BTreeMap;

    use super::*;
    use crate::testing::canonical;
    use crate::types::constrain::module::constrain;
    use crate::types::expected::{Category, Expected};
    use crate::types::solve::{TypeError, run};

    #[test]
    fn a_module_constrains_its_values_in_dependency_order() {
        let canonical =
            canonical("module Main exposing (..)\n\n\ng =\n    f 1\n\n\nf x =\n    x\n").unwrap();
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
    fn infer(text: &str) -> Result<BTreeMap<String, String>, Vec<TypeError>> {
        fn show(t: &T) -> String {
            match t {
                T::Var(v) => v.clone(),
                T::Lambda(a, b) => format!("({} -> {})", show(a), show(b)),
                T::Type { name, args, .. } if args.is_empty() => name.clone(),
                T::Type { name, args, .. } => {
                    format!(
                        "({name} {})",
                        args.iter().map(show).collect::<Vec<_>>().join(" ")
                    )
                }
                T::Tuple(a, b, None) => format!("( {}, {} )", show(a), show(b)),
                other => format!("{other:?}"),
            }
        }
        let canonical = canonical(text).unwrap();
        let mut vars = Variables::default();
        let constraint = constrain(&mut vars, &canonical);
        let annotations = run(&mut vars, &constraint)?;
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
                [TypeError::BadExpr(
                    _,
                    Category::Number,
                    _,
                    Expected::FromContext(_, expected::Context::CallArg(_, 0), _),
                )]
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn inference_records_each_expressions_type() {
        let text = "module Main exposing (..)\n\n\nxs =\n    [ 1, 2 ]\n";
        let (result, types) = infer_types(Compat::V0_19_1, &canonical(text).unwrap());
        assert!(result.is_ok());
        let shown: Vec<(&str, String)> = (types.iter())
            .map(|(region, ty)| {
                let ty = match ty {
                    error::Type::FlexSuper(_, name) => name.clone(),
                    error::Type::Type(_, name, args) => format!("{name} {}", args.len()),
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
}
