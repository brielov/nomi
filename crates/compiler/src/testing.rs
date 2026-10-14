//! What the crate's unit tests share: interfaces like elm/core's, and
//! modules canonicalized against them.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use nomi_base::Compat;

use crate::canonical::{self, Annotation, Ctor, CtorOpts, Home, Type};
use crate::canonicalize::error::Error;
use crate::canonicalize::module::{Canonical, canonicalize};
use crate::hir::{Associativity, Fixity, lowered};
use crate::interface::{Alias, Binop, Interface, Union};

/// elm/core's `Int`.
pub(crate) fn int() -> Type {
    Type::Type {
        home: Home::core("Basics"),
        name: "Int".to_owned(),
        args: vec![],
    }
}

/// A value of type `ty`, which has no free variables.
pub(crate) fn value(ty: Type) -> Annotation {
    Annotation {
        free: vec![],
        ty: ty.into(),
    }
}

fn union(vars: &[&str], ctors: &[&str]) -> Arc<canonical::Union> {
    Arc::new(canonical::Union {
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
    })
}

fn binop(function: &str, associativity: Associativity, precedence: i64) -> Binop {
    Binop {
        function: function.to_owned(),
        annotation: value(int()),
        fixity: Fixity {
            associativity,
            precedence,
        },
    }
}

/// Interfaces like elm/core's defaults, plus local modules `A` and `B`,
/// which both expose `x`, a type `T` and a constructor `C`.
pub(crate) fn interfaces() -> HashMap<String, Interface> {
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
    (basics.binops).insert("+".to_owned(), binop("add", Associativity::Left, 6));
    (basics.binops).insert("*".to_owned(), binop("mul", Associativity::Left, 7));
    let mut list = core(&["map"], vec![("List", Union::Open(union(&["a"], &[])))]);
    (list.binops).insert("::".to_owned(), binop("cons", Associativity::Right, 5));
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
                ty: Type::Record(BTreeMap::new(), None).into(),
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

/// The module `text`, an application's, canonicalized against
/// [`interfaces`] as 0.19.1 does.
pub(crate) fn canonical(text: &str) -> Result<Canonical, Vec<Error>> {
    canonicalize(
        Compat::V0_19_1,
        "author/project",
        &interfaces(),
        &lowered(text),
    )
}
