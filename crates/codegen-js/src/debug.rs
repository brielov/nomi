//! `Elm/Compiler/Type/Extract.hs` and `encodeMetadata`: what `--debug`
//! tells the debugger about a program's messages, as JSON: the message
//! type, and every alias and union it reaches, each written as the type
//! printer writes it on one line. Each round of the search lists what it
//! found in the order of upstream's sets: by name, then module, before
//! 0.19.3, and by module, then name, in it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nomi_base::Compat;
use nomi_base::diagnostic::json::strict_string;
use nomi_compiler::canonical::{Alias, Home, Type, Union};
use nomi_compiler::interface::{self, Interface};
use nomi_compiler::optimized::Global;
use nomi_compiler::optimized::binary::{globals, homes};

/// `Extract.Types`: each module's unions and aliases.
pub type Types = HashMap<Home, ModuleTypes>;

/// `Types_`.
#[derive(Clone, Debug, Default)]
pub struct ModuleTypes {
    /// The module's unions, by name.
    pub unions: BTreeMap<String, Union>,
    /// The module's aliases, by name.
    pub aliases: BTreeMap<String, Alias>,
}

impl ModuleTypes {
    /// `fromInterface`: every union and alias of the module, exported or
    /// not.
    pub fn from_interface(interface: &Interface) -> ModuleTypes {
        ModuleTypes {
            unions: (interface.unions.iter())
                .map(|(name, union)| (name.clone(), union.whole().clone()))
                .collect(),
            aliases: (interface.aliases.iter())
                .map(|(name, alias)| (name.clone(), alias.whole().clone()))
                .collect(),
        }
    }

    /// An interface of the module `package`'s unions and aliases alone,
    /// for storing them as interfaces are stored.
    pub fn to_interface(&self, package: &str) -> Interface {
        Interface {
            home: package.to_owned(),
            values: BTreeMap::new(),
            unions: (self.unions.iter())
                .map(|(name, union)| {
                    let union = interface::Union::Private(union.clone().into());
                    (name.clone(), union)
                })
                .collect(),
            aliases: (self.aliases.iter())
                .map(|(name, alias)| (name.clone(), interface::Alias::Private(alias.clone())))
                .collect(),
            binops: BTreeMap::new(),
        }
    }
}

/// `T.Type`: a type as the debugger reads it.
enum Public {
    Lambda(Box<Public>, Box<Public>),
    Var(String),
    Type(String, Vec<Public>),
    Record(Vec<(String, Public)>, Option<String>),
    Unit,
    Tuple(Box<Public>, Box<Public>, Vec<Public>),
}

/// The aliases and unions types name, by kind.
#[derive(Default)]
struct Deps {
    aliases: BTreeSet<Global>,
    unions: BTreeSet<Global>,
}

/// An alias the search found, as the debugger names it.
struct FoundAlias {
    name: String,
    vars: Vec<String>,
    ty: Public,
}

/// A union the search found, as the debugger names it, with its
/// constructors' arguments.
struct FoundUnion {
    name: String,
    vars: Vec<String>,
    tags: Vec<(String, Vec<Public>)>,
}

/// The JSON `--debug` passes a program of messages of type `message`
/// (`toDebugMetadata`).
pub(crate) fn metadata(compat: Compat, types: &Types, message: &Type) -> String {
    let mut deps = Deps::default();
    let message = extract(compat, &mut deps, message);
    let (aliases, unions) = extract_transitive(compat, types, deps);
    encode(compat, &message, &aliases, &unions)
}

/// `extractTransitive`: what `deps` names, and what that names in turn,
/// round by round, each round's finds in the order of upstream's sets.
fn extract_transitive(
    compat: Compat,
    types: &Types,
    mut deps: Deps,
) -> (Vec<FoundAlias>, Vec<FoundUnion>) {
    let mut seen = Deps::default();
    let (mut aliases, mut unions) = (Vec::new(), Vec::new());
    loop {
        let next_aliases = unseen(compat, deps.aliases, &seen.aliases);
        let next_unions = unseen(compat, deps.unions, &seen.unions);
        if next_aliases.is_empty() && next_unions.is_empty() {
            return (aliases, unions);
        }
        let mut found = Deps::default();
        for Global { home, name } in &next_aliases {
            let alias = (types.get(home).and_then(|t| t.aliases.get(name))).unwrap_or_else(|| {
                panic!(
                    "no alias {}.{name}, though every module's types are here",
                    home.module
                )
            });
            aliases.push(FoundAlias {
                name: public_name(home, name),
                vars: alias.vars.clone(),
                ty: extract(compat, &mut found, &alias.ty),
            });
        }
        for Global { home, name } in &next_unions {
            if name == "List" && home.is("elm/core", "List") {
                unions.push(FoundUnion {
                    name: public_name(home, name),
                    vars: vec!["a".to_owned()],
                    tags: Vec::new(),
                });
                continue;
            }
            let union = (types.get(home).and_then(|t| t.unions.get(name))).unwrap_or_else(|| {
                panic!(
                    "no union {}.{name}, though every module's types are here",
                    home.module
                )
            });
            let tags = (union.ctors.iter())
                .map(|ctor| {
                    let args = (ctor.args.iter())
                        .map(|arg| extract(compat, &mut found, arg))
                        .collect();
                    (ctor.name.clone(), args)
                })
                .collect();
            unions.push(FoundUnion {
                name: public_name(home, name),
                vars: union.vars.clone(),
                tags,
            });
        }
        seen.aliases.extend(next_aliases);
        seen.unions.extend(next_unions);
        deps = found;
    }
}

/// What of `found` isn't `seen`, in the order of upstream's sets: name,
/// then module, before 0.19.3, as `Opt.Global` orders; module, then name,
/// in it, as derived.
fn unseen(compat: Compat, found: BTreeSet<Global>, seen: &BTreeSet<Global>) -> Vec<Global> {
    let mut unseen: Vec<Global> = found.into_iter().filter(|g| !seen.contains(g)).collect();
    unseen.sort_by(|a, b| match compat {
        Compat::V0_19_1 | Compat::V0_19_2 => globals(compat, a, b),
        Compat::V0_19_3 => {
            homes(compat, &a.home, &b.home).then_with(|| compat.compare_names(&a.name, &b.name))
        }
    });
    unseen
}

/// `toPublicName`: `Module.Name`.
fn public_name(home: &Home, name: &str) -> String {
    format!("{}.{name}", home.module)
}

/// `extract`: the type, with the unions and aliases it names added to
/// `deps`; an alias's own type is searched too.
fn extract(compat: Compat, deps: &mut Deps, ty: &Type) -> Public {
    match ty {
        Type::Lambda(arg, result) => Public::Lambda(
            Box::new(extract(compat, deps, arg)),
            Box::new(extract(compat, deps, result)),
        ),
        Type::Var(x) => Public::Var(x.clone()),
        Type::Type { home, name, args } => {
            deps.unions.insert(Global {
                home: home.clone(),
                name: name.clone(),
            });
            let args = args.iter().map(|a| extract(compat, deps, a)).collect();
            Public::Type(public_name(home, name), args)
        }
        Type::Record(fields, ext) => {
            // `fieldsToList`: by the order written, ties as the map lists.
            let mut listed: Vec<(&String, _)> = fields.iter().collect();
            listed.sort_by(|a, b| compat.compare_names(a.0, b.0));
            listed.sort_by_key(|(_, field)| field.order);
            let fields = (listed.into_iter())
                .map(|(name, field)| (name.clone(), extract(compat, deps, &field.ty)))
                .collect();
            Public::Record(fields, ext.clone())
        }
        Type::Unit => Public::Unit,
        Type::Tuple(a, b, c) => Public::Tuple(
            Box::new(extract(compat, deps, a)),
            Box::new(extract(compat, deps, b)),
            c.iter().map(|c| extract(compat, deps, c)).collect(),
        ),
        Type::Alias {
            home,
            name,
            args,
            aliased,
        } => {
            deps.aliases.insert(Global {
                home: home.clone(),
                name: name.clone(),
            });
            extract(compat, deps, &aliased.dealias(args));
            let args = args.iter().map(|(_, a)| extract(compat, deps, a)).collect();
            Public::Type(public_name(home, name), args)
        }
    }
}

/// `encodeMetadata`.
fn encode(
    compat: Compat,
    message: &Public,
    aliases: &[FoundAlias],
    unions: &[FoundUnion],
) -> String {
    let aliases: Vec<String> = (aliases.iter())
        .map(|alias| {
            format!(
                "{}:{{\"args\":{},\"type\":{}}}",
                strict_string(&alias.name),
                strings(&alias.vars),
                strict_string(&line(&alias.ty))
            )
        })
        .collect();
    let unions: Vec<String> = (unions.iter())
        .map(|union| {
            let tags: Vec<String> = (union.tags.iter())
                .map(|(ctor, args)| {
                    let args: Vec<String> = args.iter().map(|a| strict_string(&line(a))).collect();
                    format!("{}:[{}]", strict_string(ctor), args.join(","))
                })
                .collect();
            format!(
                "{}:{{\"args\":{},\"tags\":{{{}}}}}",
                strict_string(&union.name),
                strings(&union.vars),
                tags.join(",")
            )
        })
        .collect();
    format!(
        "{{\"versions\":{{\"elm\":\"{compat}\"}},\"types\":{{\"message\":{},\
         \"aliases\":{{{}}},\"unions\":{{{}}}}}}}",
        strict_string(&line(message)),
        aliases.join(","),
        unions.join(",")
    )
}

/// Where a type sits: alone, as a function's part, or as an argument.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    None,
    Func,
    App,
}

/// `D.toLine (toDoc L.empty RT.None tipe)`.
fn line(ty: &Public) -> String {
    doc(ty, Context::None)
}

/// `toDoc`, on one line.
fn doc(ty: &Public, context: Context) -> String {
    match ty {
        Public::Lambda(..) => {
            let mut parts = Vec::new();
            let mut at = ty;
            while let Public::Lambda(arg, result) = at {
                parts.push(doc(arg, Context::Func));
                at = result;
            }
            parts.push(doc(at, Context::Func));
            let lambda = parts.join(" -> ");
            match context {
                Context::None => lambda,
                Context::Func | Context::App => format!("({lambda})"),
            }
        }
        Public::Var(x) => x.clone(),
        Public::Unit => "()".to_owned(),
        Public::Tuple(a, b, cs) => {
            let mut entries = vec![doc(a, Context::None), doc(b, Context::None)];
            entries.extend(cs.iter().map(|c| doc(c, Context::None)));
            format!("( {} )", entries.join(", "))
        }
        Public::Type(name, args) if args.is_empty() => name.clone(),
        Public::Type(name, args) => {
            let mut parts = vec![name.clone()];
            parts.extend(args.iter().map(|a| doc(a, Context::App)));
            let apply = parts.join(" ");
            match context {
                Context::App => format!("({apply})"),
                Context::Func | Context::None => apply,
            }
        }
        Public::Record(fields, ext) => {
            let entries: Vec<String> = (fields.iter())
                .map(|(name, ty)| format!("{name} : {}", doc(ty, Context::None)))
                .collect();
            match (entries.is_empty(), ext) {
                (true, None) => "{}".to_owned(),
                (false, None) => format!("{{ {} }}", entries.join(", ")),
                // An empty `cat` still takes `sep`'s space.
                (true, Some(ext)) => format!("{{ {ext}  }}"),
                (false, Some(ext)) => format!("{{ {ext} | {} }}", entries.join(", ")),
            }
        }
    }
}

/// A JSON array of strings.
fn strings(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|i| strict_string(i)).collect();
    format!("[{}]", items.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_compiler::canonical::{Ctor, CtorOpts, FieldType};

    fn ty(module: &str, name: &str, args: Vec<Type>) -> Type {
        Type::Type {
            home: Home::author_project(module),
            name: name.to_owned(),
            args,
        }
    }

    fn union(ctors: Vec<(&str, Vec<Type>)>) -> Union {
        Union {
            vars: Vec::new(),
            count: ctors.len(),
            ctors: (ctors.into_iter().enumerate())
                .map(|(index, (name, args))| Ctor {
                    name: name.to_owned(),
                    index,
                    args,
                })
                .collect(),
            opts: CtorOpts::Normal,
        }
    }

    /// `Main.Msg = Go Bb.X (Int -> Int) | Set { b : Int, a : Int } A.Y`:
    /// `Bb.X`, `A.Y` and `Basics.Int` are found in the same round.
    fn types() -> Types {
        let int = || ty("Basics", "Int", Vec::new());
        let fields: BTreeMap<String, FieldType> = [("b", 0), ("a", 1)]
            .into_iter()
            .map(|(name, order)| (name.to_owned(), FieldType { order, ty: int() }))
            .collect();
        let function = Type::Lambda(Box::new(int()), Box::new(int()));
        let msg = union(vec![
            ("Go", vec![ty("Bb", "X", Vec::new()), function]),
            (
                "Set",
                vec![Type::Record(fields, None), ty("A", "Y", Vec::new())],
            ),
        ]);
        let mut types = Types::new();
        let mut add = |module: &str, name: &str, union: Union| {
            let entry = types.entry(Home::author_project(module)).or_default();
            entry.unions.insert(name.to_owned(), union);
        };
        add("Main", "Msg", msg);
        add("Bb", "X", union(vec![("X", Vec::new())]));
        add("A", "Y", union(vec![("Y", Vec::new())]));
        add("Basics", "Int", union(Vec::new()));
        types
    }

    #[test]
    fn messages_are_described_round_by_round() {
        let message = ty("Main", "Msg", Vec::new());
        let json = metadata(Compat::V0_19_1, &types(), &message);
        assert_eq!(
            json,
            "{\"versions\":{\"elm\":\"0.19.1\"},\"types\":{\"message\":\"Main.Msg\",\
             \"aliases\":{},\"unions\":{\
             \"Main.Msg\":{\"args\":[],\"tags\":{\
             \"Go\":[\"Bb.X\",\"Basics.Int -> Basics.Int\"],\
             \"Set\":[\"{ b : Basics.Int, a : Basics.Int }\",\"A.Y\"]}},\
             \"Basics.Int\":{\"args\":[],\"tags\":{}},\
             \"Bb.X\":{\"args\":[],\"tags\":{\"X\":[]}},\
             \"A.Y\":{\"args\":[],\"tags\":{\"Y\":[]}}}}}"
        );
        // 0.19.3 orders a round by module first, names shortest first.
        let json = metadata(Compat::V0_19_3, &types(), &message);
        let at = |key: &str| json.find(key).unwrap();
        assert!(at("\"A.Y\":") < at("\"Bb.X\":"));
        assert!(at("\"Bb.X\":") < at("\"Basics.Int\":"));
    }

    #[test]
    fn types_print_on_one_line() {
        let a = || Public::Var("a".to_owned());
        let maybe = |t| Public::Type("Maybe.Maybe".to_owned(), vec![t]);
        let lambda = |x, y| Public::Lambda(Box::new(x), Box::new(y));
        assert_eq!(line(&maybe(maybe(a()))), "Maybe.Maybe (Maybe.Maybe a)");
        assert_eq!(line(&lambda(lambda(a(), a()), a())), "(a -> a) -> a");
        assert_eq!(line(&lambda(maybe(a()), a())), "Maybe.Maybe a -> a");
        let tuple = Public::Tuple(Box::new(a()), Box::new(Public::Unit), Vec::new());
        assert_eq!(line(&tuple), "( a, () )");
        let fields = vec![("x".to_owned(), a())];
        let record = Public::Record(fields, Some("r".to_owned()));
        assert_eq!(line(&record), "{ r | x : a }");
    }
}
