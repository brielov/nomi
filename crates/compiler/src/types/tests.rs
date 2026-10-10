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
