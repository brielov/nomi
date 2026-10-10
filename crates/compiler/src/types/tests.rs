use super::*;
use crate::canonical::Type as T;

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
        annotation.ty,
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
        "List".to_owned(),
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
            name.to_owned(),
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
            BTreeMap::from([("x".to_owned(), x)]),
            ext1,
        )));
        let two = vars.fresh(Content::Structure(FlatType::Record1(
            BTreeMap::from([("y".to_owned(), int)]),
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
