//! `Elm/Compiler/Type/Extract.hs` and `encodeMetadata`: what `--debug`
//! tells the debugger about a program's messages, as JSON: the message
//! type, and every alias and union it reaches, each written as the type
//! printer writes it on one line. Each round of the search lists what it
//! found in the order of upstream's sets: by name, then module, before
//! 0.19.3, and by module, then name, in it.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use nomi_base::Compat;
use nomi_compiler::canonical::{Alias, AliasType, Home, Type, Union};
use nomi_compiler::interface::Interface;
use nomi_compiler::optimized::binary::{homes, names};

/// `Extract.Types`: each module's unions and aliases.
pub type Types = HashMap<Home, ModuleTypes>;

/// `Types_`.
#[derive(Clone, Debug, Default)]
pub struct ModuleTypes {
    pub unions: BTreeMap<String, Union>,
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
}

impl ModuleTypes {
    /// An interface of the module `package`'s unions and aliases alone,
    /// for storing them as interfaces are stored.
    pub fn to_interface(&self, package: &str) -> Interface {
        use nomi_compiler::interface::{Alias as IAlias, Union as IUnion};
        Interface {
            home: package.to_owned(),
            values: BTreeMap::new(),
            unions: (self.unions.iter())
                .map(|(name, union)| (name.clone(), IUnion::Private(union.clone())))
                .collect(),
            aliases: (self.aliases.iter())
                .map(|(name, alias)| (name.clone(), IAlias::Private(alias.clone())))
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

/// A union or alias found: its module and name.
type Seen = (Home, String);

/// What extraction found, by kind.
#[derive(Default)]
struct Deps {
    aliases: Vec<Seen>,
    unions: Vec<Seen>,
}

impl Deps {
    fn add(list: &mut Vec<Seen>, home: &Home, name: &str) {
        if !list.iter().any(|(h, n)| h == home && n == name) {
            list.push((home.clone(), name.to_owned()));
        }
    }
}

/// The JSON `--debug` passes a program of messages of type `message`
/// (`toDebugMetadata`).
pub fn metadata(compat: Compat, types: &Types, message: &Type) -> String {
    let mut deps = Deps::default();
    let message = extract(compat, &mut deps, message);
    let mut seen = Deps::default();
    let mut aliases = Vec::new();
    let mut unions = Vec::new();
    // `extractTransitive`: what each round finds and the last didn't.
    loop {
        let order = |a: &Seen, b: &Seen| seen_order(compat, a, b);
        let mut next_aliases: Vec<Seen> = (deps.aliases.iter())
            .filter(|s| !seen.aliases.contains(s))
            .cloned()
            .collect();
        let mut next_unions: Vec<Seen> = (deps.unions.iter())
            .filter(|s| !seen.unions.contains(s))
            .cloned()
            .collect();
        if next_aliases.is_empty() && next_unions.is_empty() {
            break;
        }
        next_aliases.sort_by(order);
        next_unions.sort_by(order);
        let mut found = Deps::default();
        for (home, name) in &next_aliases {
            let alias = (types.get(home).and_then(|t| t.aliases.get(name)))
                .unwrap_or_else(|| panic!("no alias {}.{name}", home.module));
            let ty = extract(compat, &mut found, &alias.ty);
            aliases.push((public_name(home, name), alias.vars.clone(), ty));
        }
        for (home, name) in &next_unions {
            let public = public_name(home, name);
            if name == "List" && home.package == "elm/core" && home.module == "List" {
                unions.push((public, vec!["a".to_owned()], Vec::new()));
                continue;
            }
            let union = (types.get(home).and_then(|t| t.unions.get(name)))
                .unwrap_or_else(|| panic!("no union {}.{name}", home.module));
            let ctors = (union.ctors.iter())
                .map(|ctor| {
                    let args: Vec<Public> = (ctor.args.iter())
                        .map(|arg| extract(compat, &mut found, arg))
                        .collect();
                    (ctor.name.clone(), args)
                })
                .collect();
            unions.push((public, union.vars.clone(), ctors));
        }
        seen.aliases.extend(next_aliases);
        seen.unions.extend(next_unions);
        deps = found;
    }
    let mut out = String::from("{\"versions\":{\"elm\":\"");
    out.push_str(&compat.to_string());
    out.push_str("\"},\"types\":{\"message\":");
    out.push_str(&string(&line(&message)));
    out.push_str(",\"aliases\":{");
    let aliases: Vec<String> = (aliases.iter())
        .map(|(name, args, ty)| {
            format!(
                "{}:{{\"args\":{},\"type\":{}}}",
                string(name),
                strings(args),
                string(&line(ty))
            )
        })
        .collect();
    out.push_str(&aliases.join(","));
    out.push_str("},\"unions\":{");
    let unions: Vec<String> = (unions.iter())
        .map(|(name, args, ctors)| {
            let tags: Vec<String> = (ctors.iter())
                .map(|(ctor, args)| {
                    let args: Vec<String> = args.iter().map(|a| string(&line(a))).collect();
                    format!("{}:[{}]", string(ctor), args.join(","))
                })
                .collect();
            format!(
                "{}:{{\"args\":{},\"tags\":{{{}}}}}",
                string(name),
                strings(args),
                tags.join(",")
            )
        })
        .collect();
    out.push_str(&unions.join(","));
    out.push_str("}}}");
    out
}

/// `Ord Seen`: `Opt.Global`'s, name then module, before 0.19.3; derived,
/// module then name, in it.
fn seen_order(compat: Compat, a: &Seen, b: &Seen) -> Ordering {
    if compat == Compat::V0_19_3 {
        homes(compat, &a.0, &b.0).then_with(|| names(compat, &a.1, &b.1))
    } else {
        names(compat, &a.1, &b.1).then_with(|| homes(compat, &a.0, &b.0))
    }
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
        Type::Type(home, name, args) => {
            Deps::add(&mut deps.unions, home, name);
            let args = args.iter().map(|a| extract(compat, deps, a)).collect();
            Public::Type(public_name(home, name), args)
        }
        Type::Record(fields, ext) => {
            // `fieldsToList`: by the order written, ties as the map lists.
            let mut listed: Vec<(&String, _)> = fields.iter().collect();
            listed.sort_by(|a, b| names(compat, a.0, b.0));
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
        Type::Alias(home, name, args, aliased) => {
            Deps::add(&mut deps.aliases, home, name);
            extract(compat, deps, &dealias(args, aliased));
            let args = args.iter().map(|(_, a)| extract(compat, deps, a)).collect();
            Public::Type(public_name(home, name), args)
        }
    }
}

/// `Type.dealias`.
fn dealias(args: &[(String, Type)], aliased: &AliasType) -> Type {
    match aliased {
        AliasType::Holey(ty) => substitute(args, ty),
        AliasType::Filled(ty) => (**ty).clone(),
    }
}

/// A holey alias's type with its variables filled in.
fn substitute(args: &[(String, Type)], ty: &Type) -> Type {
    let go = |t: &Type| substitute(args, t);
    match ty {
        Type::Lambda(a, b) => Type::Lambda(Box::new(go(a)), Box::new(go(b))),
        Type::Var(x) => (args.iter())
            .find(|(name, _)| name == x)
            .map_or_else(|| ty.clone(), |(_, t)| t.clone()),
        Type::Type(home, name, ts) => {
            Type::Type(home.clone(), name.clone(), ts.iter().map(go).collect())
        }
        Type::Record(fields, ext) => {
            let fields = (fields.iter())
                .map(|(name, field)| {
                    let mut field = field.clone();
                    field.ty = go(&field.ty);
                    (name.clone(), field)
                })
                .collect();
            Type::Record(fields, ext.clone())
        }
        Type::Unit => Type::Unit,
        Type::Tuple(a, b, c) => Type::Tuple(
            Box::new(go(a)),
            Box::new(go(b)),
            c.as_ref().map(|c| Box::new(go(c))),
        ),
        Type::Alias(home, name, ts, aliased) => {
            let ts = ts.iter().map(|(n, t)| (n.clone(), go(t))).collect();
            Type::Alias(home.clone(), name.clone(), ts, aliased.clone())
        }
    }
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

/// A JSON string.
fn string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON array of strings.
fn strings(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|i| string(i)).collect();
    format!("[{}]", items.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_compiler::canonical::{Ctor, CtorOpts, FieldType};

    fn home(module: &str) -> Home {
        Home {
            package: "author/project".into(),
            module: module.into(),
        }
    }

    fn ty(module: &str, name: &str, args: Vec<Type>) -> Type {
        Type::Type(home(module), name.to_owned(), args)
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
            let entry = types.entry(home(module)).or_default();
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
