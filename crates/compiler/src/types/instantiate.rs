//! Canonical types as constraint types (`Type/Instantiate.hs`): each type
//! variable replaced by the type `free` gives it, and an alias kept, with
//! what it stands for.

use std::collections::BTreeMap;

use nomi_base::Text;

use super::Type;
use crate::canonical::{self, AliasType};

/// `fromSrcType`. Every variable of `ty` is in `free`, as canonicalization
/// made sure.
pub fn from_src_type(free: &BTreeMap<String, Type>, ty: &canonical::Type) -> Type {
    use canonical::Type as C;
    let go = |t: &canonical::Type| from_src_type(free, t);
    match ty {
        C::Lambda(arg, result) => super::fun(go(arg), go(result)),
        C::Var(name) => free[name].clone(),
        C::Type { home, name, args } => Type::App(
            home.clone(),
            Text::from(name),
            args.iter().map(go).collect(),
        ),
        C::Alias {
            home,
            name,
            args,
            aliased,
        } => {
            let targs: Vec<(String, Type)> = args.iter().map(|(n, t)| (n.clone(), go(t))).collect();
            let real = match aliased {
                AliasType::Filled(real) => go(real),
                AliasType::Holey(real) => {
                    let free: BTreeMap<String, Type> = targs.iter().cloned().collect();
                    from_src_type(&free, real)
                }
            };
            Type::Alias(home.clone(), Text::from(name), targs, Box::new(real))
        }
        C::Tuple(a, b, c) => Type::Tuple(
            Box::new(go(a)),
            Box::new(go(b)),
            c.as_ref().map(|c| Box::new(go(c))),
        ),
        C::Unit => Type::Unit,
        C::Record(fields, ext) => {
            let fields = (fields.iter())
                .map(|(n, f)| (Text::new(n), go(&f.ty)))
                .collect();
            let ext = match ext {
                None => Type::EmptyRecord,
                Some(ext) => free[ext].clone(),
            };
            Type::Record(fields, Box::new(ext))
        }
    }
}
