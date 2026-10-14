//! Constraints for expressions and definitions
//! (`Type/Constrain/Expression.hs`). Each case makes its variables and
//! constraints in upstream's order: a `let`'s body before its
//! definition, a call's function before its arguments, a record's fields
//! by name.

use std::collections::BTreeMap;

use nomi_base::{Text, TextRange};

use super::pattern::{self, State};
use crate::canonical::{
    self, Def, Expr, ExprKind, FieldUpdate, Home, Operator, Pattern, ShaderType, ShaderTypes,
};
use crate::types::expected::{
    Category, Context, Expected, MaybeName, PContext, PExpected, SubContext,
};
use crate::types::instantiate::from_src_type;
use crate::types::{self, Constraint, Type, Variable, Variables, exists, fun, in_name_order, list};

/// `RTV`: the rigid type variables the annotations around an expression
/// introduced, by name.
pub(super) type Rtv = BTreeMap<String, Type>;

fn var(v: Variable) -> Type {
    Type::Var(v)
}

fn and(cons: Vec<Constraint>) -> Constraint {
    Constraint::And(cons)
}

fn equal(region: TextRange, category: Category, ty: Type, expected: Expected<Type>) -> Constraint {
    Constraint::Equal {
        region,
        category,
        ty,
        expected,
    }
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

impl State {
    /// `CLet` of what the patterns bound over `body`.
    fn over(self, body: Constraint) -> Constraint {
        flex_let(self.vars, self.headers, and(self.cons), body)
    }
}

/// `constrain`.
pub(crate) fn constrain(
    vars: &mut Variables,
    rtv: &Rtv,
    expr: &Expr,
    expected: Expected<Type>,
) -> Constraint {
    nomi_base::grow_large(|| constrain_(vars, rtv, expr, expected))
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
    let foreign = |name: &str, annotation: &canonical::Annotation, expected| Constraint::Foreign {
        region,
        name: name.to_owned(),
        annotation: Box::new(annotation.clone()),
        expected,
    };
    match &expr.kind {
        ExprKind::VarLocal(name) | ExprKind::VarTopLevel(_, name) => Constraint::Local {
            region,
            name: name.clone(),
            expected,
        },
        ExprKind::VarKernel { .. } => Constraint::True,
        ExprKind::VarForeign {
            name, annotation, ..
        }
        | ExprKind::VarCtor {
            name, annotation, ..
        }
        | ExprKind::VarDebug {
            name, annotation, ..
        }
        | ExprKind::VarOperator(Operator {
            op: name,
            annotation,
            ..
        }) => foreign(name, annotation, expected),
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
        ExprKind::Binop {
            operator: Operator { op, annotation, .. },
            left,
            right,
        } => {
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
                    state.over(body_con),
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
        ExprKind::LetDestruct {
            pattern,
            value,
            body,
        } => {
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
        ExprKind::Access {
            record,
            field,
            field_region,
        } => {
            let ext_var = vars.flex_var();
            let field_var = vars.flex_var();
            let record_type = Type::Record(
                BTreeMap::from([(Text::new(field), var(field_var))]),
                Box::new(var(ext_var)),
            );
            let context = Context::RecordAccess {
                record: record.region,
                record_name: access_name(record),
                field_region: *field_region,
                field: field.clone(),
            };
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
        ExprKind::Update {
            name,
            record,
            fields,
        } => update(vars, rtv, region, name, record, fields, expected),
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
            let c = c.as_ref().map(|c| {
                let c_var = vars.flex_var();
                let c_con = constrain(vars, rtv, c, Expected::NoExpectation(var(c_var)));
                (c_var, c_con)
            });
            let ty = Type::Tuple(
                Box::new(var(a_var)),
                Box::new(var(b_var)),
                c.as_ref().map(|(c_var, _)| Box::new(var(*c_var))),
            );
            let tuple_con = equal(region, Category::Tuple, ty, expected);
            let (c_var, c_con) = c.unzip();
            let flex_vars = [a_var, b_var].into_iter().chain(c_var).collect();
            let cons = [a_con, b_con].into_iter().chain(c_con).chain([tuple_con]);
            exists(flex_vars, and(cons.collect()))
        }
        ExprKind::Shader(_, shader) => shader_con(vars, region, shader, expected),
    }
}

/// `getName`: what a call calls.
fn call_name(expr: &Expr) -> MaybeName {
    match &expr.kind {
        ExprKind::VarLocal(name)
        | ExprKind::VarTopLevel(_, name)
        | ExprKind::VarForeign { name, .. }
        | ExprKind::VarKernel { name, .. } => MaybeName::FuncName(name.clone()),
        ExprKind::VarCtor { name, .. } => MaybeName::CtorName(name.clone()),
        ExprKind::VarOperator(operator) => MaybeName::OpName(operator.op.clone()),
        ExprKind::VarDebug { .. }
        | ExprKind::Chr(_)
        | ExprKind::Str(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::List(_)
        | ExprKind::Negate(_)
        | ExprKind::Binop { .. }
        | ExprKind::Lambda(..)
        | ExprKind::Call(..)
        | ExprKind::If(..)
        | ExprKind::Let(..)
        | ExprKind::LetRec(..)
        | ExprKind::LetDestruct { .. }
        | ExprKind::Case(..)
        | ExprKind::Accessor(_)
        | ExprKind::Access { .. }
        | ExprKind::Update { .. }
        | ExprKind::Record(_)
        | ExprKind::Unit
        | ExprKind::Tuple(..)
        | ExprKind::Shader(..) => MaybeName::NoName,
    }
}

/// `getAccessName`.
fn access_name(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::VarLocal(name)
        | ExprKind::VarTopLevel(_, name)
        | ExprKind::VarForeign { name, .. } => Some(name.clone()),
        ExprKind::VarKernel { .. }
        | ExprKind::VarCtor { .. }
        | ExprKind::VarDebug { .. }
        | ExprKind::VarOperator(_)
        | ExprKind::Chr(_)
        | ExprKind::Str(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::List(_)
        | ExprKind::Negate(_)
        | ExprKind::Binop { .. }
        | ExprKind::Lambda(..)
        | ExprKind::Call(..)
        | ExprKind::If(..)
        | ExprKind::Let(..)
        | ExprKind::LetRec(..)
        | ExprKind::LetDestruct { .. }
        | ExprKind::Case(..)
        | ExprKind::Accessor(_)
        | ExprKind::Access { .. }
        | ExprKind::Update { .. }
        | ExprKind::Record(_)
        | ExprKind::Unit
        | ExprKind::Tuple(..)
        | ExprKind::Shader(..) => None,
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
    let mut arg_cons = Vec::new();
    for (i, arg) in arguments.iter().enumerate() {
        let v = vars.flex_var();
        let context = Context::CallArg(name.clone(), i);
        let expected = Expected::FromContext(region, context, var(v));
        arg_cons.push(constrain(vars, rtv, arg, expected));
        arg_vars.push(v);
    }
    let arity_type =
        (arg_vars.iter().rev()).fold(var(result_var), |result, arg| fun(var(*arg), result));
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
    let exprs = (branches.iter().map(|(_, e)| e)).chain([otherwise]);
    match expected {
        Expected::FromAnnotation {
            name, arity, ty, ..
        } => {
            let branch_cons = (exprs.enumerate())
                .map(|(i, e)| {
                    let expected = Expected::FromAnnotation {
                        name: name.clone(),
                        arity,
                        sub: SubContext::TypedIfBranch(i),
                        ty: ty.clone(),
                    };
                    constrain(vars, rtv, e, expected)
                })
                .collect();
            and(vec![and(cond_cons), and(branch_cons)])
        }
        expected => {
            let branch_var = vars.flex_var();
            let branch_cons = (exprs.enumerate())
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
        Expected::FromAnnotation {
            name, arity, ty, ..
        } => {
            let mut cons = vec![expr_con];
            for (i, (pattern, body)) in branches.iter().enumerate() {
                let expected = Expected::FromAnnotation {
                    name: name.clone(),
                    arity,
                    sub: SubContext::TypedCaseBranch(i),
                    ty: ty.clone(),
                };
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
    state.over(body_con)
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
    let webgl = Home {
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
    vars: &mut Variables,
    rtv: &Rtv,
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
pub(super) fn new_rigids(vars: &mut Variables, rtv: &Rtv, free: &[String]) -> (Vec<Variable>, Rtv) {
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

/// A definition constrained, before it is bound: the variables it makes,
/// its type, and its body's constraint under its parameters.
struct Defined {
    /// An unannotated definition's parameters' and result's variables,
    /// or an annotated one's new rigid variables.
    vars: Vec<Variable>,
    ty: Type,
    con: Constraint,
}

/// An unannotated definition, its parameters' patterns bound after what
/// `seed` holds.
fn untyped(
    vars: &mut Variables,
    rtv: &Rtv,
    params: &[Pattern],
    body: &Expr,
    seed: State,
) -> Defined {
    let Args {
        vars: arg_vars,
        ty,
        result,
        state,
    } = args_help(vars, params, seed);
    let expr_con = constrain(vars, rtv, body, Expected::NoExpectation(result));
    Defined {
        vars: arg_vars,
        ty,
        con: state.over(expr_con),
    }
}

/// An annotated definition, named `name`, with the annotation's `free`
/// variables, its typed `params` and its `result`.
fn typed(
    vars: &mut Variables,
    rtv: &Rtv,
    name: &str,
    free: &[String],
    params: &[(Pattern, canonical::Type)],
    body: &Expr,
    result: &canonical::Type,
) -> Defined {
    let (rigids, new_rtv) = new_rigids(vars, rtv, free);
    let TypedArgs { ty, result, state } = typed_args(vars, &new_rtv, name, params, result);
    let expected = Expected::FromAnnotation {
        name: name.to_owned(),
        arity: params.len(),
        sub: SubContext::TypedBody,
        ty: result,
    };
    let expr_con = constrain(vars, &new_rtv, body, expected);
    Defined {
        vars: rigids,
        ty,
        con: state.over(expr_con),
    }
}

/// `constrainDef`.
pub(super) fn constrain_def(
    vars: &mut Variables,
    rtv: &Rtv,
    def: &Def,
    body_con: Constraint,
) -> Constraint {
    let bind = |rigid_vars, flex_vars, ty, con| Constraint::Let {
        rigid_vars,
        flex_vars,
        header: BTreeMap::from([(def.name().to_owned(), (def.region(), ty))]),
        header_con: Box::new(con),
        body_con: Box::new(body_con),
    };
    match def {
        Def::Def { params, body, .. } => {
            let defined = untyped(vars, rtv, params, body, State::default());
            bind(Vec::new(), defined.vars, defined.ty, defined.con)
        }
        Def::TypedDef {
            name,
            free,
            params,
            body,
            result,
            ..
        } => {
            let defined = typed(vars, rtv, name, free, params, body, result);
            bind(defined.vars, Vec::new(), defined.ty, defined.con)
        }
    }
}

/// `Info`: what the annotated or the unannotated definitions of a
/// recursive group gathered.
#[derive(Default)]
struct Group {
    vars: Vec<Variable>,
    /// Latest first, as upstream's list.
    cons: Vec<Constraint>,
    headers: BTreeMap<String, (TextRange, Type)>,
}

/// `constrainRecursiveDefs`.
pub(super) fn constrain_recursive_defs<'d>(
    vars: &mut Variables,
    rtv: &Rtv,
    defs: impl IntoIterator<Item = &'d Def>,
    body_con: Constraint,
) -> Constraint {
    let mut rigid = Group::default();
    let mut flex = Group::default();
    for def in defs {
        match def {
            Def::Def { params, body, .. } => {
                let seed = State {
                    vars: std::mem::take(&mut flex.vars),
                    ..State::default()
                };
                // The group's earlier variables end up in this one's
                // pattern scope, and the group keeps only this
                // definition's parameters and result, as upstream's do.
                let defined = untyped(vars, rtv, params, body, seed);
                flex.vars = defined.vars;
                flex.cons.insert(0, defined.con);
                flex.headers
                    .insert(def.name().to_owned(), (def.region(), defined.ty));
            }
            Def::TypedDef {
                name,
                free,
                params,
                body,
                result,
                ..
            } => {
                let defined = typed(vars, rtv, name, free, params, body, result);
                // `Map.foldr (:) rigidVars newRigids`.
                let mut new_vars = defined.vars.clone();
                new_vars.extend(std::mem::take(&mut rigid.vars));
                rigid.vars = new_vars;
                rigid.cons.insert(
                    0,
                    Constraint::Let {
                        rigid_vars: defined.vars,
                        flex_vars: Vec::new(),
                        header: BTreeMap::new(),
                        header_con: Box::new(defined.con),
                        body_con: Box::new(Constraint::True),
                    },
                );
                rigid
                    .headers
                    .insert(def.name().to_owned(), (def.region(), defined.ty));
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
