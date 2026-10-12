//! Constraints for expressions and definitions
//! (`Type/Constrain/Expression.hs`). Each case makes its variables and
//! constraints in upstream's order: a `let`'s body before its
//! definition, a call's function before its arguments, a record's fields
//! by name.

use crate::canonical::Text;
use std::collections::BTreeMap;

use nomi_base::TextRange;

use super::pattern::{self, State};
use crate::canonical::{self, Def, Expr, ExprKind, FieldUpdate, Pattern, ShaderType, ShaderTypes};
use crate::types::expected::{
    Category, Context, Expected, MaybeName, PContext, PExpected, SubContext,
};
use crate::types::instantiate::from_src_type;
use crate::types::{self, Constraint, Type, Variable, Variables, core, exists, fun, in_name_order};

/// `RTV`: the rigid type variables the annotations around an expression
/// introduced, by name.
pub type Rtv = BTreeMap<String, Type>;

fn var(v: Variable) -> Type {
    Type::Var(v)
}

fn and(cons: Vec<Constraint>) -> Constraint {
    Constraint::And(cons)
}

fn equal(region: TextRange, category: Category, ty: Type, expected: Expected<Type>) -> Constraint {
    Constraint::Equal(region, category, ty, expected)
}

/// `CLet` with no rigid variables.
fn flex_let(
    flex_vars: Vec<Variable>,
    header: BTreeMap<String, (TextRange, Type)>,
    header_con: Constraint,
    body_con: Constraint,
) -> Constraint {
    Constraint::Let {
        rigid_vars: Vec::new(),
        flex_vars,
        header,
        header_con: Box::new(header_con),
        body_con: Box::new(body_con),
    }
}

fn list(entry: Type) -> Type {
    Type::App(core("List"), Text::from("List"), vec![entry])
}

/// `constrain`.
pub fn constrain(
    vars: &mut Variables,
    rtv: &Rtv,
    expr: &Expr,
    expected: Expected<Type>,
) -> Constraint {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
        constrain_(vars, rtv, expr, expected)
    })
}

fn constrain_(
    vars: &mut Variables,
    rtv: &Rtv,
    expr: &Expr,
    expected: Expected<Type>,
) -> Constraint {
    let region = expr.region;
    if let Some(recorded) = &mut vars.recorded {
        recorded.push((region, expected.ty().clone()));
    }
    let foreign = |name: &str, annotation: &canonical::Annotation, expected| {
        Constraint::Foreign(
            region,
            name.to_owned(),
            Box::new(annotation.clone()),
            expected,
        )
    };
    match &expr.kind {
        ExprKind::VarLocal(name) | ExprKind::VarTopLevel(_, name) => {
            Constraint::Local(region, name.clone(), expected)
        }
        ExprKind::VarKernel(..) => Constraint::True,
        ExprKind::VarForeign(_, name, annotation)
        | ExprKind::VarCtor(_, _, name, _, annotation)
        | ExprKind::VarDebug(_, name, annotation)
        | ExprKind::VarOperator(name, _, _, annotation) => foreign(name, annotation, expected),
        ExprKind::Str(_) => equal(region, Category::String, types::string(), expected),
        ExprKind::Chr(_) => equal(region, Category::Char, types::char(), expected),
        ExprKind::Int(_) => {
            let v = vars.flex_number();
            exists(vec![v], equal(region, Category::Number, var(v), expected))
        }
        ExprKind::Float(_) => equal(region, Category::Float, types::float(), expected),
        ExprKind::List(entries) => {
            let entry_var = vars.flex_var();
            let entry = var(entry_var);
            let entry_cons = (entries.iter().enumerate())
                .map(|(i, e)| {
                    let expected =
                        Expected::FromContext(region, Context::ListEntry(i), entry.clone());
                    constrain(vars, rtv, e, expected)
                })
                .collect();
            exists(
                vec![entry_var],
                and(vec![
                    and(entry_cons),
                    equal(region, Category::List, list(entry), expected),
                ]),
            )
        }
        ExprKind::Negate(inner) => {
            let number_var = vars.flex_number();
            let number = var(number_var);
            let number_con = constrain(
                vars,
                rtv,
                inner,
                Expected::FromContext(region, Context::Negate, number.clone()),
            );
            let negate_con = equal(region, Category::Number, number, expected);
            exists(vec![number_var], and(vec![number_con, negate_con]))
        }
        ExprKind::Binop(op, _, _, annotation, left, right) => {
            let left_var = vars.flex_var();
            let right_var = vars.flex_var();
            let answer_var = vars.flex_var();
            let binop_type = fun(var(left_var), fun(var(right_var), var(answer_var)));
            let op_con = foreign(op, annotation, Expected::NoExpectation(binop_type));
            let left_con = constrain(
                vars,
                rtv,
                left,
                Expected::FromContext(region, Context::OpLeft(op.clone()), var(left_var)),
            );
            let right_con = constrain(
                vars,
                rtv,
                right,
                Expected::FromContext(region, Context::OpRight(op.clone()), var(right_var)),
            );
            let answer_con = equal(
                region,
                Category::CallResult(MaybeName::OpName(op.clone())),
                var(answer_var),
                expected,
            );
            exists(
                vec![left_var, right_var, answer_var],
                and(vec![op_con, left_con, right_con, answer_con]),
            )
        }
        ExprKind::Lambda(params, body) => {
            let Args {
                vars: arg_vars,
                ty,
                result,
                state,
            } = args(vars, params);
            let body_con = constrain(vars, rtv, body, Expected::NoExpectation(result));
            exists(
                arg_vars,
                and(vec![
                    flex_let(state.vars, state.headers, and(state.cons), body_con),
                    equal(region, Category::Lambda, ty, expected),
                ]),
            )
        }
        ExprKind::Call(function, arguments) => {
            call(vars, rtv, region, function, arguments, expected)
        }
        ExprKind::If(branches, otherwise) => if_(vars, rtv, region, branches, otherwise, expected),
        ExprKind::Case(scrutinee, branches) => {
            case(vars, rtv, region, scrutinee, branches, expected)
        }
        ExprKind::Let(def, body) => {
            let body_con = constrain(vars, rtv, body, expected);
            constrain_def(vars, rtv, def, body_con)
        }
        ExprKind::LetRec(defs, body) => {
            let body_con = constrain(vars, rtv, body, expected);
            constrain_recursive_defs(vars, rtv, defs, body_con)
        }
        ExprKind::LetDestruct(pattern, value, body) => {
            let body_con = constrain(vars, rtv, body, expected);
            destruct(vars, rtv, region, pattern, value, body_con)
        }
        ExprKind::Accessor(field) => {
            let ext_var = vars.flex_var();
            let field_var = vars.flex_var();
            let record = Type::Record(
                BTreeMap::from([(Text::new(field), var(field_var))]),
                Box::new(var(ext_var)),
            );
            exists(
                vec![field_var, ext_var],
                equal(
                    region,
                    Category::Accessor(field.clone()),
                    fun(record, var(field_var)),
                    expected,
                ),
            )
        }
        ExprKind::Access(record, field_region, field) => {
            let ext_var = vars.flex_var();
            let field_var = vars.flex_var();
            let record_type = Type::Record(
                BTreeMap::from([(Text::new(field), var(field_var))]),
                Box::new(var(ext_var)),
            );
            let context = Context::RecordAccess(
                record.region,
                access_name(record),
                *field_region,
                field.clone(),
            );
            let record_con = constrain(
                vars,
                rtv,
                record,
                Expected::FromContext(region, context, record_type),
            );
            exists(
                vec![field_var, ext_var],
                and(vec![
                    record_con,
                    equal(
                        region,
                        Category::Access(field.clone()),
                        var(field_var),
                        expected,
                    ),
                ]),
            )
        }
        ExprKind::Update(name, record, fields) => {
            update(vars, rtv, region, name, record, fields, expected)
        }
        ExprKind::Record(fields) => {
            let mut field_vars = Vec::new();
            let mut field_types = BTreeMap::new();
            let mut cons = Vec::new();
            for (name, value) in in_name_order(vars.compat, fields) {
                let v = vars.flex_var();
                cons.push(constrain(vars, rtv, value, Expected::NoExpectation(var(v))));
                field_vars.push(v);
                field_types.insert(Text::new(name), var(v));
            }
            let record_type = Type::Record(field_types, Box::new(Type::EmptyRecord));
            cons.push(equal(region, Category::Record, record_type, expected));
            exists(field_vars, and(cons))
        }
        ExprKind::Unit => equal(region, Category::Unit, Type::Unit, expected),
        ExprKind::Tuple(a, b, c) => {
            let a_var = vars.flex_var();
            let b_var = vars.flex_var();
            let a_con = constrain(vars, rtv, a, Expected::NoExpectation(var(a_var)));
            let b_con = constrain(vars, rtv, b, Expected::NoExpectation(var(b_var)));
            match c {
                None => {
                    let ty = Type::Tuple(Box::new(var(a_var)), Box::new(var(b_var)), None);
                    let tuple_con = equal(region, Category::Tuple, ty, expected);
                    exists(vec![a_var, b_var], and(vec![a_con, b_con, tuple_con]))
                }
                Some(c) => {
                    let c_var = vars.flex_var();
                    let c_con = constrain(vars, rtv, c, Expected::NoExpectation(var(c_var)));
                    let ty = Type::Tuple(
                        Box::new(var(a_var)),
                        Box::new(var(b_var)),
                        Some(Box::new(var(c_var))),
                    );
                    let tuple_con = equal(region, Category::Tuple, ty, expected);
                    exists(
                        vec![a_var, b_var, c_var],
                        and(vec![a_con, b_con, c_con, tuple_con]),
                    )
                }
            }
        }
        ExprKind::Shader(_, shader) => shader_con(vars, region, shader, expected),
    }
}

/// `getName`: what a call calls.
fn call_name(expr: &Expr) -> MaybeName {
    match &expr.kind {
        ExprKind::VarLocal(name)
        | ExprKind::VarTopLevel(_, name)
        | ExprKind::VarForeign(_, name, _)
        | ExprKind::VarKernel(_, name) => MaybeName::FuncName(name.clone()),
        ExprKind::VarCtor(_, _, name, _, _) => MaybeName::CtorName(name.clone()),
        ExprKind::VarOperator(op, ..) => MaybeName::OpName(op.clone()),
        _ => MaybeName::NoName,
    }
}

/// `getAccessName`.
fn access_name(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::VarLocal(name)
        | ExprKind::VarTopLevel(_, name)
        | ExprKind::VarForeign(_, name, _) => Some(name.clone()),
        _ => None,
    }
}

/// `constrainCall`.
fn call(
    vars: &mut Variables,
    rtv: &Rtv,
    region: TextRange,
    function: &Expr,
    arguments: &[Expr],
    expected: Expected<Type>,
) -> Constraint {
    let name = call_name(function);
    let func_var = vars.flex_var();
    let result_var = vars.flex_var();
    let func_con = constrain(vars, rtv, function, Expected::NoExpectation(var(func_var)));
    let mut arg_vars = Vec::new();
    let mut arg_types = Vec::new();
    let mut arg_cons = Vec::new();
    for (i, arg) in arguments.iter().enumerate() {
        let v = vars.flex_var();
        let expected = Expected::FromContext(region, Context::CallArg(name.clone(), i), var(v));
        arg_cons.push(constrain(vars, rtv, arg, expected));
        arg_vars.push(v);
        arg_types.push(var(v));
    }
    let arity_type = arg_types
        .into_iter()
        .rev()
        .fold(var(result_var), |result, arg| fun(arg, result));
    let category = Category::CallResult(name.clone());
    let mut all_vars = vec![func_var, result_var];
    all_vars.extend(arg_vars);
    exists(
        all_vars,
        and(vec![
            func_con,
            equal(
                function.region,
                category.clone(),
                var(func_var),
                Expected::FromContext(
                    region,
                    Context::CallArity(name, arguments.len()),
                    arity_type,
                ),
            ),
            and(arg_cons),
            equal(region, category, var(result_var), expected),
        ]),
    )
}

/// `constrainIf`: every condition, then every branch.
fn if_(
    vars: &mut Variables,
    rtv: &Rtv,
    region: TextRange,
    branches: &[(Expr, Expr)],
    otherwise: &Expr,
    expected: Expected<Type>,
) -> Constraint {
    let bool_expect = Expected::FromContext(region, Context::IfCondition, types::bool());
    let cond_cons: Vec<Constraint> = branches
        .iter()
        .map(|(c, _)| constrain(vars, rtv, c, bool_expect.clone()))
        .collect();
    let exprs: Vec<&Expr> = branches
        .iter()
        .map(|(_, e)| e)
        .chain(std::iter::once(otherwise))
        .collect();
    match expected {
        Expected::FromAnnotation(name, arity, _, ty) => {
            let branch_cons = (exprs.iter().enumerate())
                .map(|(i, e)| {
                    let expected = Expected::FromAnnotation(
                        name.clone(),
                        arity,
                        SubContext::TypedIfBranch(i),
                        ty.clone(),
                    );
                    constrain(vars, rtv, e, expected)
                })
                .collect();
            and(vec![and(cond_cons), and(branch_cons)])
        }
        expected => {
            let branch_var = vars.flex_var();
            let branch_cons = (exprs.iter().enumerate())
                .map(|(i, e)| {
                    let expected =
                        Expected::FromContext(region, Context::IfBranch(i), var(branch_var));
                    constrain(vars, rtv, e, expected)
                })
                .collect();
            exists(
                vec![branch_var],
                and(vec![
                    and(cond_cons),
                    and(branch_cons),
                    equal(region, Category::If, var(branch_var), expected),
                ]),
            )
        }
    }
}

/// `constrainCase`.
fn case(
    vars: &mut Variables,
    rtv: &Rtv,
    region: TextRange,
    scrutinee: &Expr,
    branches: &[(Pattern, Expr)],
    expected: Expected<Type>,
) -> Constraint {
    let ptrn_var = vars.flex_var();
    let ptrn_type = var(ptrn_var);
    let expr_con = constrain(
        vars,
        rtv,
        scrutinee,
        Expected::NoExpectation(ptrn_type.clone()),
    );
    let p_expect = |i| PExpected::PFromContext(region, PContext::PCaseMatch(i), ptrn_type.clone());
    match expected {
        Expected::FromAnnotation(name, arity, _, ty) => {
            let mut cons = vec![expr_con];
            for (i, (pattern, body)) in branches.iter().enumerate() {
                let expected = Expected::FromAnnotation(
                    name.clone(),
                    arity,
                    SubContext::TypedCaseBranch(i),
                    ty.clone(),
                );
                cons.push(case_branch(vars, rtv, pattern, body, p_expect(i), expected));
            }
            exists(vec![ptrn_var], and(cons))
        }
        expected => {
            let branch_var = vars.flex_var();
            let branch_cons = (branches.iter().enumerate())
                .map(|(i, (pattern, body))| {
                    let expected =
                        Expected::FromContext(region, Context::CaseBranch(i), var(branch_var));
                    case_branch(vars, rtv, pattern, body, p_expect(i), expected)
                })
                .collect();
            exists(
                vec![ptrn_var, branch_var],
                and(vec![
                    expr_con,
                    and(branch_cons),
                    equal(region, Category::Case, var(branch_var), expected),
                ]),
            )
        }
    }
}

/// `constrainCaseBranch`.
fn case_branch(
    vars: &mut Variables,
    rtv: &Rtv,
    pattern: &Pattern,
    body: &Expr,
    p_expect: PExpected<Type>,
    expected: Expected<Type>,
) -> Constraint {
    let mut state = State::default();
    pattern::add(vars, pattern, p_expect, &mut state);
    let body_con = constrain(vars, rtv, body, expected);
    flex_let(state.vars, state.headers, and(state.cons), body_con)
}

/// `constrainUpdate`.
fn update(
    vars: &mut Variables,
    rtv: &Rtv,
    region: TextRange,
    name: &str,
    record: &Expr,
    fields: &BTreeMap<String, FieldUpdate>,
    expected: Expected<Type>,
) -> Constraint {
    let ext_var = vars.flex_var();
    let mut field_vars = Vec::new();
    let mut field_types = BTreeMap::new();
    let mut field_cons = Vec::new();
    for (field, update) in in_name_order(vars.compat, fields) {
        let v = vars.flex_var();
        let expected =
            Expected::FromContext(region, Context::RecordUpdateValue(field.clone()), var(v));
        field_cons.push(constrain(vars, rtv, &update.value, expected));
        field_vars.push(v);
        field_types.insert(Text::new(field), var(v));
    }
    let record_var = vars.flex_var();
    let record_type = var(record_var);
    let fields_type = Type::Record(field_types, Box::new(var(ext_var)));
    // Separate, so that an error propagates better.
    let fields_con = equal(
        region,
        Category::Record,
        record_type.clone(),
        Expected::NoExpectation(fields_type),
    );
    let record_con = equal(region, Category::Record, record_type.clone(), expected);
    let mut all_vars = field_vars;
    all_vars.extend([record_var, ext_var]);
    let keys = fields.iter().map(|(n, u)| (n.clone(), u.region)).collect();
    let context = Context::RecordUpdateKeys(name.to_owned(), keys);
    let con = constrain(
        vars,
        rtv,
        record,
        Expected::FromContext(region, context, record_type),
    );
    let mut cons = vec![fields_con, con];
    cons.extend(field_cons);
    cons.push(record_con);
    exists(all_vars, and(cons))
}

/// `constrainShader`.
fn shader_con(
    vars: &mut Variables,
    region: TextRange,
    shader: &ShaderTypes,
    expected: Expected<Type>,
) -> Constraint {
    let attr_var = vars.flex_var();
    let unif_var = vars.flex_var();
    let record = |inputs: &BTreeMap<String, ShaderType>, base: Type| {
        if inputs.is_empty() {
            base
        } else {
            let fields = inputs
                .iter()
                .map(|(n, t)| (Text::new(n), gl_type(*t)))
                .collect();
            Type::Record(fields, Box::new(base))
        }
    };
    let webgl = crate::canonical::Home {
        package: "elm-explorations/webgl".into(),
        module: "WebGL".into(),
    };
    let shader_type = Type::App(
        webgl,
        Text::from("Shader"),
        vec![
            record(&shader.attribute, var(attr_var)),
            record(&shader.uniform, var(unif_var)),
            record(&shader.varying, Type::EmptyRecord),
        ],
    );
    exists(
        vec![attr_var, unif_var],
        equal(region, Category::Shader, shader_type, expected),
    )
}

/// `glToType`.
fn gl_type(ty: ShaderType) -> Type {
    match ty {
        ShaderType::V2 => types::vec2(),
        ShaderType::V3 => types::vec3(),
        ShaderType::V4 => types::vec4(),
        ShaderType::M4 => types::mat4(),
        ShaderType::Int => types::int(),
        ShaderType::Float => types::float(),
        ShaderType::Texture => types::texture(),
    }
}

/// `constrainDestruct`.
fn destruct(
    vars: &mut Variables,
    rtv: &Rtv,
    region: TextRange,
    pattern: &Pattern,
    value: &Expr,
    body_con: Constraint,
) -> Constraint {
    let pattern_var = vars.flex_var();
    let pattern_type = var(pattern_var);
    let mut state = State::default();
    pattern::add(
        vars,
        pattern,
        PExpected::PNoExpectation(pattern_type.clone()),
        &mut state,
    );
    let expr_con = constrain(
        vars,
        rtv,
        value,
        Expected::FromContext(region, Context::Destructure, pattern_type),
    );
    let mut flex_vars = vec![pattern_var];
    flex_vars.extend(state.vars);
    let mut cons = state.cons;
    cons.push(expr_con);
    flex_let(flex_vars, state.headers, and(cons), body_con)
}

/// `Args`: the variables a definition's parameters make, its type, what
/// its body must be, and its parameters' bindings.
struct Args {
    vars: Vec<Variable>,
    ty: Type,
    result: Type,
    state: State,
}

/// `constrainArgs`.
fn args(vars: &mut Variables, params: &[Pattern]) -> Args {
    args_help(vars, params, State::default())
}

/// `argsHelp`: each parameter's variable, then its pattern, then the
/// rest, and at last the result's variable.
fn args_help(vars: &mut Variables, params: &[Pattern], mut state: State) -> Args {
    let mut arg_vars = Vec::with_capacity(params.len() + 1);
    for param in params {
        let v = vars.flex_var();
        pattern::add(vars, param, PExpected::PNoExpectation(var(v)), &mut state);
        arg_vars.push(v);
    }
    let result_var = vars.flex_var();
    let result = var(result_var);
    let ty = arg_vars
        .iter()
        .rev()
        .fold(result.clone(), |ty, v| fun(var(*v), ty));
    arg_vars.push(result_var);
    Args {
        vars: arg_vars,
        ty,
        result,
        state,
    }
}

/// `TypedArgs`.
struct TypedArgs {
    ty: Type,
    result: Type,
    state: State,
}

/// `constrainTypedArgs`: each parameter against its annotated type.
fn typed_args(
    rtv: &Rtv,
    vars: &mut Variables,
    name: &str,
    params: &[(Pattern, canonical::Type)],
    result: &canonical::Type,
) -> TypedArgs {
    let mut state = State::default();
    let mut arg_types = Vec::with_capacity(params.len());
    for (index, (pattern, src)) in params.iter().enumerate() {
        let arg = from_src_type(rtv, src);
        let expected = PExpected::PFromContext(
            pattern.region,
            PContext::PTypedArg(name.to_owned(), index),
            arg.clone(),
        );
        pattern::add(vars, pattern, expected, &mut state);
        arg_types.push(arg);
    }
    let result = from_src_type(rtv, result);
    let ty = arg_types
        .into_iter()
        .rev()
        .fold(result.clone(), |ty, arg| fun(arg, ty));
    TypedArgs { ty, result, state }
}

/// The rigid variables an annotation's free variables add to `rtv`, in
/// name order, and `rtv` with them.
fn new_rigids(vars: &mut Variables, rtv: &Rtv, free: &[String]) -> (Vec<Variable>, Rtv) {
    let mut names: Vec<&String> = free.iter().filter(|n| !rtv.contains_key(*n)).collect();
    names.sort();
    names.dedup();
    let mut new_rtv = rtv.clone();
    let mut rigids = Vec::with_capacity(names.len());
    for name in names {
        let v = vars.name_to_rigid(name);
        rigids.push(v);
        new_rtv.insert(name.clone(), var(v));
    }
    (rigids, new_rtv)
}

/// `constrainDef`.
pub fn constrain_def(
    vars: &mut Variables,
    rtv: &Rtv,
    def: &Def,
    body_con: Constraint,
) -> Constraint {
    match def {
        Def::Def {
            name,
            region,
            params,
            body,
        } => {
            let Args {
                vars: arg_vars,
                ty,
                result,
                state,
            } = args(vars, params);
            let expr_con = constrain(vars, rtv, body, Expected::NoExpectation(result));
            Constraint::Let {
                rigid_vars: Vec::new(),
                flex_vars: arg_vars,
                header: BTreeMap::from([(name.clone(), (*region, ty))]),
                header_con: Box::new(flex_let(
                    state.vars,
                    state.headers,
                    and(state.cons),
                    expr_con,
                )),
                body_con: Box::new(body_con),
            }
        }
        Def::TypedDef {
            name,
            region,
            free,
            params,
            body,
            result,
        } => {
            let (rigids, new_rtv) = new_rigids(vars, rtv, free);
            let TypedArgs { ty, result, state } = typed_args(&new_rtv, vars, name, params, result);
            let expected =
                Expected::FromAnnotation(name.clone(), params.len(), SubContext::TypedBody, result);
            let expr_con = constrain(vars, &new_rtv, body, expected);
            Constraint::Let {
                rigid_vars: rigids,
                flex_vars: Vec::new(),
                header: BTreeMap::from([(name.clone(), (*region, ty))]),
                header_con: Box::new(flex_let(
                    state.vars,
                    state.headers,
                    and(state.cons),
                    expr_con,
                )),
                body_con: Box::new(body_con),
            }
        }
    }
}

/// `Info`: what the annotated or the unannotated definitions of a
/// recursive group gathered.
#[derive(Default)]
struct Info {
    vars: Vec<Variable>,
    /// Latest first, as upstream's list.
    cons: Vec<Constraint>,
    headers: BTreeMap<String, (TextRange, Type)>,
}

/// `constrainRecursiveDefs`.
pub fn constrain_recursive_defs<'d>(
    vars: &mut Variables,
    rtv: &Rtv,
    defs: impl IntoIterator<Item = &'d Def>,
    body_con: Constraint,
) -> Constraint {
    let mut rigid = Info::default();
    let mut flex = Info::default();
    for def in defs {
        match def {
            Def::Def {
                name,
                region,
                params,
                body,
            } => {
                let state = State {
                    vars: std::mem::take(&mut flex.vars),
                    ..State::default()
                };
                let Args {
                    vars: arg_vars,
                    ty,
                    result,
                    state,
                } = args_help(vars, params, state);
                let expr_con = constrain(vars, rtv, body, Expected::NoExpectation(result));
                // The group's earlier variables end up in this one's
                // pattern scope, and the group keeps only this
                // definition's parameters and result, as upstream's do.
                let def_con = flex_let(state.vars, state.headers, and(state.cons), expr_con);
                flex.vars = arg_vars;
                flex.cons.insert(0, def_con);
                flex.headers.insert(name.clone(), (*region, ty));
            }
            Def::TypedDef {
                name,
                region,
                free,
                params,
                body,
                result,
            } => {
                let (rigids, new_rtv) = new_rigids(vars, rtv, free);
                let TypedArgs { ty, result, state } =
                    typed_args(&new_rtv, vars, name, params, result);
                let expected = Expected::FromAnnotation(
                    name.clone(),
                    params.len(),
                    SubContext::TypedBody,
                    result,
                );
                let expr_con = constrain(vars, &new_rtv, body, expected);
                let def_con = flex_let(state.vars, state.headers, and(state.cons), expr_con);
                // `Map.foldr (:) rigidVars newRigids`.
                let mut new_vars = rigids.clone();
                new_vars.extend(std::mem::take(&mut rigid.vars));
                rigid.vars = new_vars;
                rigid.cons.insert(
                    0,
                    Constraint::Let {
                        rigid_vars: rigids,
                        flex_vars: Vec::new(),
                        header: BTreeMap::new(),
                        header_con: Box::new(def_con),
                        body_con: Box::new(Constraint::True),
                    },
                );
                rigid.headers.insert(name.clone(), (*region, ty));
            }
        }
    }
    Constraint::Let {
        rigid_vars: rigid.vars,
        flex_vars: Vec::new(),
        header: rigid.headers,
        header_con: Box::new(Constraint::True),
        body_con: Box::new(flex_let(
            flex.vars,
            flex.headers.clone(),
            flex_let(Vec::new(), flex.headers, Constraint::True, and(flex.cons)),
            and(vec![and(rigid.cons), body_con]),
        )),
    }
}
