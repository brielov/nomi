//! `Generate/JavaScript/Expression.hs`: an optimized expression as
//! JavaScript. The releases differ in little: how a character is
//! written, which position `Debug.todo` reports, and which constructors
//! of `Dict` get negative tags.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use nomi_base::Compat;
use nomi_base::diagnostic::code::{Position, Region};
use nomi_compiler::canonical::{CtorOpts, Home};
use nomi_compiler::optimized::{
    Choice, Decider, Def, Destructor, DtPath, Expr, Global, Main, Path, Test,
};

use crate::builder::{Case, Expr as Js, InfixOp, LValue, PrefixOp, Stmt};
use crate::debug;
use crate::mode::Mode;
use crate::name::Name;

/// Generates expressions for a release in a mode.
#[derive(Debug)]
pub(crate) struct Generator<'a> {
    pub(crate) compat: Compat,
    pub(crate) mode: &'a Mode,
}

/// `Code`: an expression, or statements ending in a `return`.
#[derive(Debug)]
pub(crate) enum Code {
    Expr(Js),
    Block(Vec<Stmt>),
}

/// `codeToExpr`.
pub(crate) fn code_to_expr(code: Code) -> Js {
    match code {
        Code::Expr(expr) => expr,
        Code::Block(mut stmts) => {
            if let [Stmt::Return(_)] = stmts.as_slice()
                && let Some(Stmt::Return(expr)) = stmts.pop()
            {
                return expr;
            }
            let function = Js::Function {
                params: Vec::new(),
                body: stmts,
            };
            call(function, Vec::new())
        }
    }
}

/// `codeToStmtList`.
pub(crate) fn code_to_stmt_list(code: Code) -> Vec<Stmt> {
    match code {
        Code::Expr(expr) => match iife(expr) {
            Ok(stmts) => stmts,
            Err(expr) => vec![Stmt::Return(expr)],
        },
        Code::Block(stmts) => stmts,
    }
}

/// `codeToStmt`.
fn code_to_stmt(code: Code) -> Stmt {
    match code {
        Code::Expr(expr) => match iife(expr) {
            Ok(stmts) => Stmt::Block(stmts),
            Err(expr) => Stmt::Return(expr),
        },
        Code::Block(stmts) => match <[Stmt; 1]>::try_from(stmts) {
            Ok([stmt]) => stmt,
            Err(stmts) => Stmt::Block(stmts),
        },
    }
}

/// The statements of `(function () { ... })()`, or the expression back.
fn iife(expr: Js) -> Result<Vec<Stmt>, Js> {
    match expr {
        Js::Call(function, args) if args.is_empty() => match *function {
            Js::Function { params, body } if params.is_empty() => Ok(body),
            function => Err(Js::Call(Box::new(function), args)),
        },
        expr => Err(expr),
    }
}

fn call(function: Js, args: Vec<Js>) -> Js {
    Js::Call(Box::new(function), args)
}

fn access(expr: Js, name: Name) -> Js {
    Js::Access(Box::new(expr), name)
}

fn infix(op: InfixOp, left: Js, right: Js) -> Js {
    Js::Infix(op, Box::new(left), Box::new(right))
}

fn prefix(op: PrefixOp, expr: Js) -> Js {
    Js::Prefix(op, Box::new(expr))
}

fn kernel(home: &str, name: &str) -> Js {
    Js::Ref(Name::from_kernel(home, name))
}

impl Generator<'_> {
    fn dev(&self) -> bool {
        matches!(self.mode, Mode::Dev { .. })
    }

    /// `generateJsExpr`.
    pub(crate) fn js(&self, expr: &Expr) -> Js {
        code_to_expr(self.generate(expr))
    }

    /// Each of `exprs`, in order.
    fn js_each<E: Borrow<Expr>>(&self, exprs: &[E]) -> Vec<Js> {
        exprs.iter().map(|e| self.js(e.borrow())).collect()
    }

    /// `generate`.
    pub(crate) fn generate(&self, expr: &Expr) -> Code {
        let js = match expr {
            Expr::Bool(b) => Js::Bool(*b),
            Expr::Chr(c) => {
                let string = Js::String(self.chr(c));
                if self.dev() {
                    call(kernel("Utils", "chr"), vec![string])
                } else {
                    string
                }
            }
            Expr::Str(s) => Js::String(s.clone()),
            Expr::Int(i) => Js::Int(*i),
            Expr::Float(f) => Js::Float(f.clone()),
            Expr::VarLocal(x) => Js::Ref(Name::from_local(x)),
            Expr::VarGlobal(g) => Js::Ref(Name::from_global(&g.home, &g.name)),
            Expr::VarEnum(g, index) => {
                if self.dev() {
                    Js::Ref(Name::from_global(&g.home, &g.name))
                } else {
                    Js::Int(*index as i64)
                }
            }
            Expr::VarBox(g) => Js::Ref(if self.dev() {
                Name::from_global(&g.home, &g.name)
            } else {
                Name::from_global(&Home::core("Basics"), "identity")
            }),
            Expr::VarCycle(home, name) => call(Js::Ref(Name::from_cycle(home, name)), Vec::new()),
            Expr::VarDebug {
                name,
                home,
                region,
                unhandled,
            } => self.debug(name, home, *region, unhandled.as_deref()),
            Expr::VarKernel { module, name } => kernel(module, name),
            Expr::List(entries) if entries.is_empty() => kernel("List", "Nil"),
            Expr::List(entries) => call(
                kernel("List", "fromArray"),
                vec![Js::Array(self.js_each(entries))],
            ),
            Expr::Function(args, body) => {
                let args = args.iter().map(|a| Name::from_local(a)).collect();
                return function(args, self.generate(body));
            }
            Expr::Call(function, args) => self.call(function, args),
            Expr::TailCall(name, args) => return Code::Block(self.tail_call(name, args)),
            Expr::If(branches, last) => return self.if_(branches, last),
            Expr::Let(def, body) => {
                let (name, value) = self.def(def);
                return self.var_before(name, value, body);
            }
            Expr::Destruct(Destructor { name, path }, body) => {
                return self.var_before(name, self.path(path), body);
            }
            Expr::Case {
                label,
                root,
                decider,
                jumps,
            } => {
                return Code::Block(self.case(label, root, decider, jumps));
            }
            Expr::Accessor(field) => Js::Function {
                params: vec![Name::dollar()],
                body: vec![Stmt::Return(access(
                    Js::Ref(Name::dollar()),
                    self.field(field),
                ))],
            },
            Expr::Access(record, field) => access(self.js(record), self.field(field)),
            Expr::Update(record, fields) => call(
                kernel("Utils", "update"),
                vec![self.js(record), self.record(fields)],
            ),
            Expr::Record(fields) => self.record(fields),
            Expr::Unit => {
                if self.dev() {
                    kernel("Utils", "Tuple0")
                } else {
                    Js::Int(0)
                }
            }
            Expr::Tuple(a, b, None) => {
                call(kernel("Utils", "Tuple2"), vec![self.js(a), self.js(b)])
            }
            Expr::Tuple(a, b, Some(c)) => call(
                kernel("Utils", "Tuple3"),
                vec![self.js(a), self.js(b), self.js(c)],
            ),
            Expr::Shader {
                source,
                attributes,
                uniforms,
            } => self.shader(source, attributes, uniforms),
        };
        Code::Expr(js)
    }

    /// `var name = value;`, then `body`.
    fn var_before(&self, name: &str, value: Js, body: &Expr) -> Code {
        let mut stmts = vec![Stmt::Var(Name::from_local(name), value)];
        stmts.extend(code_to_stmt_list(self.generate(body)));
        Code::Block(stmts)
    }

    /// A shader's source, and its attributes' and uniforms' field names.
    fn shader(
        &self,
        source: &[u8],
        attributes: &BTreeSet<String>,
        uniforms: &BTreeSet<String>,
    ) -> Js {
        let translation = |fields: &BTreeSet<String>| {
            Js::Object(
                (self.compat.sorted_names(fields).into_iter())
                    .map(|f| (Name::from_local(f), Js::String(self.field(f).to_string())))
                    .collect(),
            )
        };
        Js::Object(vec![
            (
                Name::from_local("src"),
                Js::String(String::from_utf8_lossy(source).into_owned()),
            ),
            (Name::from_local("attributes"), translation(attributes)),
            (Name::from_local("uniforms"), translation(uniforms)),
        ])
    }

    /// A character as a string literal's text: 0.19.1 keeps the literal's
    /// text; later releases escape the character (`charUtf8`).
    fn chr(&self, c: &str) -> String {
        if self.compat == Compat::V0_19_1 {
            return c.to_owned();
        }
        let mut out = String::new();
        for c in c.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                ' '.. => out.push(c),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => _ = write!(out, "\\u00{:02x}", c as u32),
            }
        }
        out
    }

    /// `generateRecord`.
    fn record(&self, fields: &BTreeMap<String, Expr>) -> Js {
        Js::Object(
            (self.compat.sorted_names(fields.keys()).into_iter())
                .map(|f| (self.field(f), self.js(&fields[f])))
                .collect(),
        )
    }

    /// `generateField`.
    pub(crate) fn field(&self, name: &str) -> Name {
        match self.mode {
            Mode::Dev { .. } => Name::from_local(name),
            Mode::Prod(fields) => fields
                .get(name)
                .unwrap_or_else(|| {
                    panic!("no short name for {name}, though every field used has one")
                })
                .clone(),
        }
    }

    /// `generateDebug`.
    fn debug(&self, name: &str, home: &Home, region: Region, unhandled: Option<&str>) -> Js {
        if name != "todo" {
            return Js::Ref(Name::from_global(&Home::core("Debug"), name));
        }
        let mut args = vec![Js::String(home.module.to_string()), self.region(region)];
        let todo = match unhandled {
            None => "todo",
            Some(value) => {
                args.push(Js::Ref(Name::from_local(value)));
                "todoCase"
            }
        };
        call(kernel("Debug", todo), args)
    }

    /// `regionToJsExpr`: rows and columns as kept, which in 0.19.2 count
    /// from 0; 0.19.3 adds the 1 back (`toEditorRowCol`).
    fn region(&self, region: Region) -> Js {
        let shift = self.compat.editor_shift();
        let position = |Position { row, col }| {
            Js::Object(vec![
                (Name::from_local("line"), Js::Int(i64::from(row + shift))),
                (Name::from_local("column"), Js::Int(i64::from(col + shift))),
            ])
        };
        Js::Object(vec![
            (Name::from_local("start"), position(region.start)),
            (Name::from_local("end"), position(region.end)),
        ])
    }

    /// `generateCall`: of the arguments `args`, borrowed or not.
    fn call<E: Borrow<Expr>>(&self, function: &Expr, args: &[E]) -> Js {
        match (function, args) {
            (Expr::VarGlobal(global), _) if global.home.package == "elm/core" => {
                self.core_call(global, args)
            }
            (Expr::VarBox(_), [arg]) if !self.dev() => self.js(arg.borrow()),
            _ => normal_call(self.js(function), self.js_each(args)),
        }
    }

    /// `generateCoreCall`.
    fn core_call<E: Borrow<Expr>>(&self, global: &Global, args: &[E]) -> Js {
        let Global { home, name } = global;
        if home.module == "Basics" {
            return self.basics_call(home, name, args);
        }
        let args = self.js_each(args);
        match &*home.module {
            "Bitwise" => bitwise_call(home, name, args),
            "Tuple" => tuple_call(home, name, args),
            // Upstream looks for `JsArray`, though the module is
            // `Elm.JsArray`, so its calls are never inlined.
            "JsArray" => js_array_call(home, name, args),
            _ => global_call(home, name, args),
        }
    }

    /// `generateBasicsCall`.
    fn basics_call<E: Borrow<Expr>>(&self, home: &Home, name: &str, args: &[E]) -> Js {
        match args {
            [arg] => {
                let arg = self.js(arg.borrow());
                match name {
                    "not" => prefix(PrefixOp::Not, arg),
                    "negate" => prefix(PrefixOp::Negate, arg),
                    "toFloat" => arg,
                    "truncate" => infix(InfixOp::BitwiseOr, arg, Js::Int(0)),
                    _ => global_call(home, name, vec![arg]),
                }
            }
            [left, right] => match (name, left.borrow(), right.borrow()) {
                ("append", left, right) => self.append(left, right),
                ("apL", function, value) | ("apR", value, function) => self.apply(function, value),
                (_, left, right) => {
                    let (left, right) = (self.js(left), self.js(right));
                    match name {
                        "add" => infix(InfixOp::Add, left, right),
                        "sub" => infix(InfixOp::Sub, left, right),
                        "mul" => infix(InfixOp::Mul, left, right),
                        "fdiv" => infix(InfixOp::Div, left, right),
                        "idiv" => infix(
                            InfixOp::BitwiseOr,
                            infix(InfixOp::Div, left, right),
                            Js::Int(0),
                        ),
                        "eq" => equal(left, right),
                        "neq" => not_equal(left, right),
                        "lt" => comp(InfixOp::Lt, InfixOp::Lt, 0, left, right),
                        "gt" => comp(InfixOp::Gt, InfixOp::Gt, 0, left, right),
                        "le" => comp(InfixOp::Le, InfixOp::Lt, 1, left, right),
                        "ge" => comp(InfixOp::Ge, InfixOp::Gt, -1, left, right),
                        "or" => infix(InfixOp::Or, left, right),
                        "and" => infix(InfixOp::And, left, right),
                        "xor" => infix(InfixOp::Ne, left, right),
                        "remainderBy" => infix(InfixOp::Mod, right, left),
                        _ => global_call(home, name, vec![left, right]),
                    }
                }
            },
            _ => global_call(home, name, self.js_each(args)),
        }
    }

    /// `apply`: `f <| x` as a call, an accessor's as an access.
    fn apply(&self, function: &Expr, value: &Expr) -> Js {
        match function {
            Expr::Accessor(field) => access(self.js(value), self.field(field)),
            Expr::Call(f, args) => {
                let args: Vec<&Expr> = args.iter().chain([value]).collect();
                self.call(f, &args)
            }
            _ => self.call(function, &[value]),
        }
    }

    /// `append`: a chain of appends as one `+` chain if any part is a
    /// string literal, and as nested `_Utils_ap` otherwise.
    fn append(&self, left: &Expr, right: &Expr) -> Js {
        let mut seqs = vec![self.js(left)];
        let mut rest = right;
        loop {
            match rest {
                Expr::Call(function, args)
                    if args.len() == 2
                        && matches!(&**function, Expr::VarGlobal(g)
                            if g.home.is("elm/core", "Basics") && g.name == "append") =>
                {
                    seqs.push(self.js(&args[0]));
                    rest = &args[1];
                }
                _ => {
                    seqs.push(self.js(rest));
                    break;
                }
            }
        }
        let strings = seqs.iter().any(|s| matches!(s, Js::String(_)));
        (seqs.into_iter().rev())
            .reduce(|right, left| {
                if strings {
                    infix(InfixOp::Add, left, right)
                } else {
                    call(kernel("Utils", "ap"), vec![left, right])
                }
            })
            .expect("an append has two sides")
    }

    /// `generateTailCall`.
    fn tail_call(&self, name: &str, args: &[(String, Expr)]) -> Vec<Stmt> {
        let mut stmts = vec![Stmt::Vars(
            args.iter()
                .map(|(arg, value)| (Name::make_temp(arg), self.js(value)))
                .collect(),
        )];
        stmts.extend(args.iter().map(|(arg, _)| {
            Stmt::Expr(Js::Assign(
                LValue::Ref(Name::from_local(arg)),
                Box::new(Js::Ref(Name::make_temp(arg))),
            ))
        }));
        stmts.push(Stmt::Continue(Name::from_local(name)));
        stmts
    }

    /// `generateDef`: the name defined and its value, which a local or a
    /// global names.
    pub(crate) fn def<'d>(&self, def: &'d Def) -> (&'d str, Js) {
        match def {
            Def::Def(name, body) => (name, self.js(body)),
            Def::TailDef { name, params, body } => {
                (name, code_to_expr(self.tail_def(name, params, body)))
            }
        }
    }

    /// `generateTailDef`.
    pub(crate) fn tail_def(&self, name: &str, args: &[String], body: &Expr) -> Code {
        let args = args.iter().map(|a| Name::from_local(a)).collect();
        function(
            args,
            Code::Block(vec![Stmt::Labelled(
                Name::from_local(name),
                Box::new(Stmt::While(
                    Js::Bool(true),
                    Box::new(code_to_stmt(self.generate(body))),
                )),
            )]),
        )
    }

    /// `generatePath`.
    fn path(&self, path: &Path) -> Js {
        match path {
            Path::Root(name) => Js::Ref(Name::from_local(name)),
            Path::Index(i, p) => access(self.path(p), Name::from_int(*i)),
            Path::Field(f, p) => access(self.path(p), self.field(f)),
            Path::Unbox(p) if self.dev() => access(self.path(p), Name::from_int(0)),
            Path::Unbox(p) => self.path(p),
        }
    }

    /// `generateIf`, the `if`s of the last branch crushed into this one.
    fn if_(&self, branches: &[(Expr, Expr)], last: &Expr) -> Code {
        let mut crushed: Vec<&(Expr, Expr)> = Vec::new();
        let mut last = last;
        while let Expr::If(more, after) = last {
            crushed.extend(more);
            last = after;
        }
        let branches: Vec<(Js, Code)> = (branches.iter().chain(crushed))
            .map(|(condition, branch)| (self.js(condition), self.generate(branch)))
            .collect();
        let last = self.generate(last);
        let block = |code: &Code| matches!(code, Code::Block(_));
        if block(&last) || branches.iter().any(|(_, code)| block(code)) {
            let stmt = (branches.into_iter())
                .rev()
                .fold(code_to_stmt(last), |els, (c, b)| Stmt::If {
                    condition: c,
                    then: Box::new(code_to_stmt(b)),
                    otherwise: Box::new(els),
                });
            Code::Block(vec![stmt])
        } else {
            let expr = (branches.into_iter())
                .rev()
                .fold(code_to_expr(last), |els, (c, b)| Js::If {
                    condition: Box::new(c),
                    then: Box::new(code_to_expr(b)),
                    otherwise: Box::new(els),
                });
            Code::Expr(expr)
        }
    }

    /// `generateCase`: each jump's target a labelled loop the decider
    /// breaks out of, the last jump innermost.
    fn case(
        &self,
        label: &str,
        root: &str,
        decider: &Decider<Choice>,
        jumps: &[(usize, Expr)],
    ) -> Vec<Stmt> {
        let start = self.decider(label, root, decider);
        jumps.iter().rev().fold(start, |stmts, (index, branch)| {
            let mut out = vec![Stmt::Labelled(
                Name::make_label(label, *index),
                Box::new(Stmt::While(Js::Bool(true), Box::new(Stmt::Block(stmts)))),
            )];
            out.extend(code_to_stmt_list(self.generate(branch)));
            out
        })
    }

    /// `generateDecider`.
    fn decider(&self, label: &str, root: &str, decider: &Decider<Choice>) -> Vec<Stmt> {
        match decider {
            Decider::Leaf(Choice::Inline(branch)) => code_to_stmt_list(self.generate(branch)),
            Decider::Leaf(Choice::Jump(index)) => {
                vec![Stmt::Break(Name::make_label(label, *index))]
            }
            Decider::Chain {
                tests,
                success,
                failure,
            } => {
                let mut tests = tests
                    .iter()
                    .map(|(path, test)| self.if_test(root, path, test));
                let first = tests.next().expect("a chain tests something");
                let condition = tests.fold(first, |all, test| infix(InfixOp::And, all, test));
                vec![Stmt::If {
                    condition,
                    then: Box::new(Stmt::Block(self.decider(label, root, success))),
                    otherwise: Box::new(Stmt::Block(self.decider(label, root, failure))),
                }]
            }
            Decider::FanOut {
                path,
                tests,
                fallback,
            } => {
                let mut cases: Vec<Case> = (tests.iter())
                    .map(|(test, subtree)| {
                        Case::Case(self.case_value(test), self.decider(label, root, subtree))
                    })
                    .collect();
                cases.push(Case::Default(self.decider(label, root, fallback)));
                vec![Stmt::Switch(self.case_test(root, path, &tests[0].0), cases)]
            }
        }
    }

    /// A constructor's tag: its name in development, else its number.
    fn ctor_tag(&self, home: &Home, name: &str, index: usize) -> Js {
        if self.dev() {
            Js::String(name.to_owned())
        } else {
            Js::Int(self.ctor_to_int(home, name, index))
        }
    }

    /// `ctorToInt`: the index, but negative from -1 for `Dict`'s nodes so
    /// the kernel can tell them apart. Before 0.19.3 a precedence slip
    /// counts any `RBEmpty_elm_builtin` as `Dict`'s.
    fn ctor_to_int(&self, home: &Home, name: &str, index: usize) -> i64 {
        let dict = home.is("elm/core", "Dict");
        let node = name == "RBNode_elm_builtin";
        let empty = name == "RBEmpty_elm_builtin";
        let negative = if self.compat == Compat::V0_19_3 {
            dict && (node || empty)
        } else {
            (dict && node) || empty
        };
        if negative {
            -(index as i64 + 1)
        } else {
            index as i64
        }
    }

    /// A constructor test's subject: the tag, or in production the value
    /// itself where it is a number.
    fn tag_of(&self, value: Js, opts: CtorOpts) -> Js {
        match (self.mode, opts) {
            (Mode::Prod(_), CtorOpts::Enum | CtorOpts::Unbox) => value,
            (Mode::Dev { .. }, _) | (Mode::Prod(_), CtorOpts::Normal) => {
                access(value, Name::dollar())
            }
        }
    }

    /// A character test's subject: in development, a `Char` is boxed.
    fn chr_subject(&self, value: Js) -> Js {
        if self.dev() {
            call(access(value, Name::from_local("valueOf")), Vec::new())
        } else {
            value
        }
    }

    /// `generateIfTest`.
    fn if_test(&self, root: &str, path: &DtPath, test: &Test) -> Js {
        let value = self.dt_path(root, path);
        match test {
            Test::IsCtor {
                home,
                name,
                index,
                opts,
                ..
            } => strict_eq(self.tag_of(value, *opts), self.ctor_tag(home, name, *index)),
            Test::IsBool(true) => value,
            Test::IsBool(false) => prefix(PrefixOp::Not, value),
            Test::IsInt(i) => strict_eq(value, Js::Int(*i)),
            Test::IsChr(c) => strict_eq(Js::String(self.chr(c)), self.chr_subject(value)),
            Test::IsStr(s) => strict_eq(value, Js::String(s.clone())),
            Test::IsCons => access(value, Name::from_local("b")),
            Test::IsNil => prefix(PrefixOp::Not, access(value, Name::from_local("b"))),
            Test::IsTuple => panic!("COMPILER BUG - there should never be tests on a tuple"),
        }
    }

    /// `generateCaseValue`.
    fn case_value(&self, test: &Test) -> Js {
        match test {
            Test::IsCtor {
                home, name, index, ..
            } => self.ctor_tag(home, name, *index),
            Test::IsInt(i) => Js::Int(*i),
            Test::IsChr(c) => Js::String(self.chr(c)),
            Test::IsStr(s) => Js::String(s.clone()),
            Test::IsBool(_) | Test::IsCons | Test::IsNil | Test::IsTuple => {
                panic!("COMPILER BUG - there should never be three tests on {test:?}")
            }
        }
    }

    /// `generateCaseTest`. Before 0.19.3, a test on `Bool`'s constructor
    /// looks at the value itself, though `Bool` is tested otherwise.
    fn case_test(&self, root: &str, path: &DtPath, example: &Test) -> Js {
        let value = self.dt_path(root, path);
        match example {
            Test::IsCtor { home, name, .. }
                if self.compat != Compat::V0_19_3
                    && name == "Bool"
                    && home.is("elm/core", "Basics") =>
            {
                value
            }
            Test::IsCtor { opts, .. } => self.tag_of(value, *opts),
            Test::IsInt(_) | Test::IsStr(_) => value,
            Test::IsChr(_) => self.chr_subject(value),
            Test::IsBool(_) | Test::IsCons | Test::IsNil | Test::IsTuple => {
                panic!("COMPILER BUG - there should never be three tests on {example:?}")
            }
        }
    }

    /// `pathToJsExpr`.
    fn dt_path(&self, root: &str, path: &DtPath) -> Js {
        match path {
            DtPath::Index(i, p) => access(self.dt_path(root, p), Name::from_int(*i)),
            DtPath::Unbox(p) if self.dev() => access(self.dt_path(root, p), Name::from_int(0)),
            DtPath::Unbox(p) => self.dt_path(root, p),
            DtPath::Empty => Js::Ref(Name::from_local(root)),
        }
    }

    /// `generateCtor`.
    pub(crate) fn ctor(&self, global: &Global, index: usize, arity: usize) -> Code {
        let args: Vec<Name> = (0..arity).map(Name::from_int).collect();
        let mut fields = vec![(
            Name::dollar(),
            self.ctor_tag(&global.home, &global.name, index),
        )];
        fields.extend(args.iter().map(|n| (n.clone(), Js::Ref(n.clone()))));
        function(args, Code::Expr(Js::Object(fields)))
    }

    /// `generateMain`.
    pub(crate) fn main(&self, home: &Home, main: &Main) -> Js {
        let main_ref = Js::Ref(Name::from_global(home, "main"));
        match main {
            Main::Static => {
                let init = call(kernel("VirtualDom", "init"), vec![main_ref]);
                call(call(init, vec![Js::Int(0)]), vec![Js::Int(0)])
            }
            Main::Dynamic { message, decoder } => {
                let applied = call(main_ref, vec![self.js(decoder)]);
                // `toDebugMetadata`.
                let metadata = match self.mode {
                    Mode::Dev { debug: Some(types) } => {
                        Js::Json(debug::metadata(self.compat, types, message))
                    }
                    Mode::Dev { debug: None } | Mode::Prod(_) => Js::Int(0),
                };
                call(applied, vec![metadata])
            }
        }
    }
}

/// `generateFunction`: `F2` to `F9` for those arities, nested functions
/// of one argument otherwise.
fn function(params: Vec<Name>, body: Code) -> Code {
    if (2..=9).contains(&params.len()) {
        let helper = Js::Ref(Name::make_f(params.len()));
        let body = code_to_stmt_list(body);
        return Code::Expr(call(helper, vec![Js::Function { params, body }]));
    }
    params.into_iter().rev().fold(body, |code, param| {
        Code::Expr(Js::Function {
            params: vec![param],
            body: code_to_stmt_list(code),
        })
    })
}

/// `generateNormalCall`: `A2` to `A9` for those many arguments, single
/// calls otherwise.
fn normal_call(function: Js, args: Vec<Js>) -> Js {
    if (2..=9).contains(&args.len()) {
        let helper = Js::Ref(Name::make_a(args.len()));
        return call(helper, std::iter::once(function).chain(args).collect());
    }
    args.into_iter().fold(function, |f, a| call(f, vec![a]))
}

/// `generateGlobalCall`.
fn global_call(home: &Home, name: &str, args: Vec<Js>) -> Js {
    normal_call(Js::Ref(Name::from_global(home, name)), args)
}

/// `generateTupleCall`.
fn tuple_call(home: &Home, name: &str, args: Vec<Js>) -> Js {
    let field = match name {
        "first" => "a",
        "second" => "b",
        _ => return global_call(home, name, args),
    };
    match <[Js; 1]>::try_from(args) {
        Ok([tuple]) => access(tuple, Name::from_local(field)),
        Err(args) => global_call(home, name, args),
    }
}

/// `generateJsArrayCall`.
fn js_array_call(home: &Home, name: &str, args: Vec<Js>) -> Js {
    match name {
        "singleton" if args.len() == 1 => Js::Array(args),
        "unsafeGet" => match <[Js; 2]>::try_from(args) {
            Ok([index, array]) => Js::Index(Box::new(array), Box::new(index)),
            Err(args) => global_call(home, name, args),
        },
        _ => global_call(home, name, args),
    }
}

/// `generateBitwiseCall`: the shifts take the shifted value last.
fn bitwise_call(home: &Home, name: &str, args: Vec<Js>) -> Js {
    let op = match name {
        "complement" => {
            return match <[Js; 1]>::try_from(args) {
                Ok([arg]) => prefix(PrefixOp::Complement, arg),
                Err(args) => global_call(home, name, args),
            };
        }
        "and" => InfixOp::BitwiseAnd,
        "or" => InfixOp::BitwiseOr,
        "xor" => InfixOp::BitwiseXor,
        "shiftLeftBy" => InfixOp::LShift,
        "shiftRightBy" => InfixOp::SpRShift,
        "shiftRightZfBy" => InfixOp::ZfRShift,
        _ => return global_call(home, name, args),
    };
    match <[Js; 2]>::try_from(args) {
        Ok([by, value])
            if matches!(op, InfixOp::LShift | InfixOp::SpRShift | InfixOp::ZfRShift) =>
        {
            infix(op, value, by)
        }
        Ok([left, right]) => infix(op, left, right),
        Err(args) => global_call(home, name, args),
    }
}

fn is_literal(expr: &Js) -> bool {
    matches!(
        expr,
        Js::String(_) | Js::Float(_) | Js::Int(_) | Js::Bool(_)
    )
}

/// `equal`.
fn equal(left: Js, right: Js) -> Js {
    if is_literal(&left) || is_literal(&right) {
        strict_eq(left, right)
    } else {
        call(kernel("Utils", "eq"), vec![left, right])
    }
}

/// `notEqual`.
fn not_equal(left: Js, right: Js) -> Js {
    if is_literal(&left) || is_literal(&right) {
        strict_neq(left, right)
    } else {
        prefix(
            PrefixOp::Not,
            call(kernel("Utils", "eq"), vec![left, right]),
        )
    }
}

/// `comp`.
fn comp(ideal: InfixOp, backup: InfixOp, backup_int: i64, left: Js, right: Js) -> Js {
    if is_literal(&left) || is_literal(&right) {
        infix(ideal, left, right)
    } else {
        let compared = call(kernel("Utils", "cmp"), vec![left, right]);
        infix(backup, compared, Js::Int(backup_int))
    }
}

/// `strictEq`, simplified against `0` and booleans.
fn strict_eq(left: Js, right: Js) -> Js {
    match (&left, &right) {
        (Js::Int(0) | Js::Bool(false), _) => prefix(PrefixOp::Not, right),
        (Js::Bool(true), _) => right,
        (_, Js::Int(0) | Js::Bool(false)) => prefix(PrefixOp::Not, left),
        (_, Js::Bool(true)) => left,
        _ => infix(InfixOp::Eq, left, right),
    }
}

/// `strictNEq`.
fn strict_neq(left: Js, right: Js) -> Js {
    match (&left, &right) {
        (Js::Int(0), _) => prefix(PrefixOp::Not, prefix(PrefixOp::Not, right)),
        (Js::Bool(true), _) => prefix(PrefixOp::Not, right),
        (Js::Bool(false), _) => right,
        (_, Js::Int(0)) => prefix(PrefixOp::Not, prefix(PrefixOp::Not, left)),
        (_, Js::Bool(true)) => prefix(PrefixOp::Not, left),
        (_, Js::Bool(false)) => left,
        _ => infix(InfixOp::Ne, left, right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder;

    const DEV: Mode = Mode::Dev { debug: None };

    fn basics(name: &str, args: Vec<Expr>) -> Expr {
        let global = Global {
            home: Home::core("Basics"),
            name: name.to_owned(),
        };
        Expr::Call(Box::new(Expr::VarGlobal(global)), args)
    }

    fn local(name: &str) -> Expr {
        Expr::VarLocal(name.to_owned())
    }

    fn show(compat: Compat, mode: &Mode, expr: &Expr) -> String {
        builder::text(&Generator { compat, mode }.js(expr))
    }

    fn dev(expr: &Expr) -> String {
        show(Compat::V0_19_3, &DEV, expr)
    }

    fn block(expr: &Expr) -> String {
        let generator = Generator {
            compat: Compat::V0_19_3,
            mode: &DEV,
        };
        let mut out = String::new();
        for stmt in code_to_stmt_list(generator.generate(expr)) {
            builder::stmt(&mut out, &stmt);
        }
        out
    }

    #[test]
    fn characters_are_escaped_after_0_19_1() {
        let chr = |c: &str| Expr::Chr(c.to_owned());
        assert_eq!(
            show(Compat::V0_19_1, &DEV, &chr("\\n")),
            "_Utils_chr('\\n')"
        );
        assert_eq!(dev(&chr("\n")), "_Utils_chr('\\n')");
        assert_eq!(dev(&chr("'")), "_Utils_chr('\\'')");
        assert_eq!(dev(&chr("\u{1}")), "_Utils_chr('\\u0001')");
        assert_eq!(dev(&chr("\u{1f}")), "_Utils_chr('\\u001f')");
        assert_eq!(dev(&chr("é")), "_Utils_chr('é')");
        let prod = Mode::Prod(BTreeMap::new());
        assert_eq!(show(Compat::V0_19_3, &prod, &chr("a")), "'a'");
    }

    #[test]
    fn basics_operators_are_inlined() {
        let (x, y) = (local("x"), local("y"));
        assert_eq!(dev(&basics("add", vec![Expr::Int(1), x.clone()])), "1 + x");
        assert_eq!(dev(&basics("eq", vec![x.clone(), Expr::Int(0)])), "!x");
        assert_eq!(dev(&basics("eq", vec![x.clone(), Expr::Int(2)])), "x === 2");
        assert_eq!(
            dev(&basics("neq", vec![x.clone(), y.clone()])),
            "!_Utils_eq(x, y)"
        );
        assert_eq!(
            dev(&basics("le", vec![x.clone(), y.clone()])),
            "_Utils_cmp(x, y) < 1"
        );
        assert_eq!(
            dev(&basics("idiv", vec![x.clone(), y.clone()])),
            "(x / y) | 0"
        );
        assert_eq!(
            dev(&basics("remainderBy", vec![x.clone(), y.clone()])),
            "y % x"
        );
        assert_eq!(dev(&basics("not", vec![x.clone()])), "!x");
        assert_eq!(dev(&basics("toFloat", vec![x.clone()])), "x");
        assert_eq!(
            dev(&basics("max", vec![x, y])),
            "A2($elm$core$Basics$max, x, y)"
        );
    }

    #[test]
    fn appends_chain_with_plus_only_around_a_string() {
        let (x, y) = (local("x"), local("y"));
        let s = Expr::Str("!".to_owned());
        let chained = basics(
            "append",
            vec![x.clone(), basics("append", vec![y.clone(), s])],
        );
        assert_eq!(dev(&chained), "x + (y + '!')");
        assert_eq!(dev(&basics("append", vec![x, y])), "_Utils_ap(x, y)");
    }

    #[test]
    fn pipes_become_calls() {
        let f = Expr::Call(Box::new(local("f")), vec![local("a")]);
        assert_eq!(dev(&basics("apR", vec![local("x"), f])), "A2(f, a, x)");
        let accessor = Expr::Accessor("name".to_owned());
        assert_eq!(dev(&basics("apL", vec![accessor, local("r")])), "r.name");
        let add = basics("add", vec![Expr::Int(1)]);
        assert_eq!(dev(&basics("apR", vec![local("x"), add])), "1 + x");
        assert_eq!(dev(&basics("apL", vec![local("f"), local("x")])), "f(x)");
    }

    #[test]
    fn functions_and_calls_use_the_helpers_from_two_to_nine() {
        let body = Box::new(local("a"));
        let one = Expr::Function(vec!["a".to_owned()], body.clone());
        assert_eq!(dev(&one), "function (a) {\n\treturn a;\n}");
        let two = Expr::Function(vec!["a".to_owned(), "b".to_owned()], body);
        assert_eq!(dev(&two), "F2(\n\tfunction (a, b) {\n\t\treturn a;\n\t})");
        let call = |n: usize| Expr::Call(Box::new(local("f")), vec![Expr::Int(0); n]);
        assert_eq!(dev(&call(1)), "f(0)");
        assert_eq!(dev(&call(3)), "A3(f, 0, 0, 0)");
        assert_eq!(dev(&call(10)), "f(0)(0)(0)(0)(0)(0)(0)(0)(0)(0)");
    }

    #[test]
    fn ifs_are_crushed_into_one_chain() {
        let inner = Expr::If(vec![(local("b"), Expr::Int(2))], Box::new(Expr::Int(3)));
        let outer = Expr::If(vec![(local("a"), Expr::Int(1))], Box::new(inner));
        assert_eq!(dev(&outer), "a ? 1 : (b ? 2 : 3)");
    }

    #[test]
    fn tail_calls_loop() {
        let call = Expr::TailCall("go".to_owned(), vec![("n".to_owned(), local("m"))]);
        let generator = Generator {
            compat: Compat::V0_19_3,
            mode: &DEV,
        };
        let def = generator.tail_def("go", &["n".to_owned()], &call);
        assert_eq!(
            builder::text(&code_to_expr(def)),
            "function (n) {\n\tgo:\n\twhile (true) {\n\t\tvar $temp$n = m;\n\t\tn = $temp$n;\n\t\tcontinue go;\n\t}\n}"
        );
    }

    #[test]
    fn a_case_jumps_out_of_labelled_loops() {
        let decider = Decider::Chain {
            tests: vec![(DtPath::Empty, Test::IsInt(1))],
            success: Box::new(Decider::Leaf(Choice::Jump(0))),
            failure: Box::new(Decider::Leaf(Choice::Inline(Expr::Int(9)))),
        };
        let case = Expr::Case {
            label: "_v0".to_owned(),
            root: "x".to_owned(),
            decider: Box::new(decider),
            jumps: vec![(0, Expr::Int(5))],
        };
        assert_eq!(
            block(&case),
            "_v0$0:\nwhile (true) {\n\tif (x === 1) {\n\t\tbreak _v0$0;\n\t} else {\n\t\treturn 9;\n\t}\n}\nreturn 5;\n"
        );
    }

    #[test]
    fn dict_nodes_get_negative_tags() {
        let prod = Mode::Prod(BTreeMap::new());
        let tag = |compat, home: &Home, name| {
            Generator {
                compat,
                mode: &prod,
            }
            .ctor_to_int(home, name, 1)
        };
        assert_eq!(
            tag(Compat::V0_19_3, &Home::core("Dict"), "RBNode_elm_builtin"),
            -2
        );
        assert_eq!(
            tag(
                Compat::V0_19_3,
                &Home::author_project("Main"),
                "RBEmpty_elm_builtin"
            ),
            1
        );
        assert_eq!(
            tag(
                Compat::V0_19_1,
                &Home::author_project("Main"),
                "RBEmpty_elm_builtin"
            ),
            -2
        );
        assert_eq!(
            tag(
                Compat::V0_19_1,
                &Home::author_project("Main"),
                "RBNode_elm_builtin"
            ),
            1
        );
    }

    #[test]
    fn todo_reports_positions_as_each_release_does() {
        let region = Region {
            start: Position { row: 2, col: 4 },
            end: Position { row: 2, col: 9 },
        };
        let debug = |name: &str| Expr::VarDebug {
            name: name.to_owned(),
            home: Home::author_project("Main"),
            region,
            unhandled: None,
        };
        let todo = debug("todo");
        let line = |compat| show(compat, &DEV, &todo).replace(['\n', '\t'], "");
        assert!(line(Compat::V0_19_2).contains("start: {line: 2, column: 4}"));
        assert!(line(Compat::V0_19_3).contains("start: {line: 3, column: 5}"));
        let log = debug("log");
        assert_eq!(dev(&log), "$elm$core$Debug$log");
    }

    #[test]
    fn records_list_fields_in_the_release_order() {
        let fields: BTreeMap<String, Expr> = [("b", 1), ("aa", 2)]
            .into_iter()
            .map(|(f, n)| (f.to_owned(), Expr::Int(n)))
            .collect();
        let record = Expr::Record(fields);
        assert_eq!(show(Compat::V0_19_1, &DEV, &record), "{aa: 2, b: 1}");
        assert_eq!(show(Compat::V0_19_3, &DEV, &record), "{b: 1, aa: 2}");
    }
}
