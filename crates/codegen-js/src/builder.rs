//! `Generate/JavaScript/Builder.hs`: the JavaScript syntax code generation
//! builds, and its printing with upstream's exact whitespace: a tab per
//! level, short arrays, objects and calls on one line, and long ones one
//! item a line. All three releases print alike.

use crate::name::Name;

/// `Expr`.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A string's text, already escaped for single quotes.
    String(String),
    /// A float as its text.
    Float(String),
    Int(i64),
    Bool(bool),
    Null,
    /// JSON, already encoded as `encodeUgly` writes it.
    Json(String),
    Array(Vec<Expr>),
    Object(Vec<(Name, Expr)>),
    Ref(Name),
    /// `foo.bar`.
    Access(Box<Expr>, Name),
    /// `foo[bar]`.
    Index(Box<Expr>, Box<Expr>),
    Prefix(PrefixOp, Box<Expr>),
    Infix(InfixOp, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Assign(LValue, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Function(Option<Name>, Vec<Name>, Vec<Stmt>),
}

/// `LValue`.
#[derive(Clone, Debug, PartialEq)]
pub enum LValue {
    Ref(Name),
    Dot(Box<Expr>, Name),
    Bracket(Box<Expr>, Box<Expr>),
}

/// `Stmt`.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Block(Vec<Stmt>),
    Empty,
    Expr(Expr),
    If(Expr, Box<Stmt>, Box<Stmt>),
    Switch(Expr, Vec<Case>),
    While(Expr, Box<Stmt>),
    Break(Option<Name>),
    Continue(Option<Name>),
    Labelled(Name, Box<Stmt>),
    Try(Box<Stmt>, Name, Box<Stmt>),
    Throw(Expr),
    Return(Expr),
    Var(Name, Expr),
    Vars(Vec<(Name, Expr)>),
    Function(Name, Vec<Name>, Vec<Stmt>),
}

/// `Case`: a switch's clause.
#[derive(Clone, Debug, PartialEq)]
pub enum Case {
    Case(Expr, Vec<Stmt>),
    Default(Vec<Stmt>),
}

/// `InfixOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfixOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LShift,
    SpRShift,
    ZfRShift,
}

/// `PrefixOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixOp {
    Not,
    Negate,
    Complement,
}

/// `stmtToBuilder`.
pub fn stmt(stmt: &Stmt) -> String {
    let mut out = String::new();
    from_stmt(&mut out, 0, stmt);
    out
}

/// `exprToBuilder`.
pub fn expr(expr: &Expr) -> String {
    from_expr(0, Grouping::Whatever, expr).1
}

/// The indentation of `level`: a tab each.
fn indent(out: &mut String, level: usize) {
    out.extend(std::iter::repeat_n('\t', level));
}

fn tabs(level: usize) -> String {
    "\t".repeat(level)
}

fn comma_sep(items: &[String]) -> String {
    items.join(", ")
}

/// `commaNewlineSep`: items a line each, at the level below `level`.
fn comma_newline_sep(level: usize, items: &[String]) -> String {
    items.join(&format!(",\n{}", tabs(level + 1)))
}

fn from_block(out: &mut String, level: usize, stmts: &[Stmt]) {
    for stmt in stmts {
        from_stmt(out, level, stmt);
    }
}

/// `fromStmt`.
fn from_stmt(out: &mut String, level: usize, stmt: &Stmt) {
    let expr = |e: &Expr| from_expr(level, Grouping::Whatever, e).1;
    match stmt {
        Stmt::Block(stmts) => from_block(out, level, stmts),
        Stmt::Empty => {}
        Stmt::Expr(e) => {
            indent(out, level);
            out.push_str(&expr(e));
            out.push_str(";\n");
        }
        Stmt::If(condition, then, otherwise) => {
            indent(out, level);
            out.push_str(&format!("if ({}) {{\n", expr(condition)));
            from_stmt(out, level + 1, then);
            indent(out, level);
            out.push_str("} else {\n");
            from_stmt(out, level + 1, otherwise);
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::Switch(e, clauses) => {
            indent(out, level);
            out.push_str(&format!("switch ({}) {{\n", expr(e)));
            for clause in clauses {
                from_clause(out, level + 1, clause);
            }
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::While(e, body) => {
            indent(out, level);
            out.push_str(&format!("while ({}) {{\n", expr(e)));
            from_stmt(out, level + 1, body);
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::Break(label) => {
            indent(out, level);
            match label {
                None => out.push_str("break;\n"),
                Some(label) => out.push_str(&format!("break {label};\n")),
            }
        }
        Stmt::Continue(label) => {
            indent(out, level);
            match label {
                None => out.push_str("continue;\n"),
                Some(label) => out.push_str(&format!("continue {label};\n")),
            }
        }
        Stmt::Labelled(label, body) => {
            indent(out, level);
            out.push_str(&format!("{label}:\n"));
            from_stmt(out, level, body);
        }
        Stmt::Try(body, error, catch) => {
            indent(out, level);
            out.push_str("try {\n");
            from_stmt(out, level + 1, body);
            indent(out, level);
            out.push_str(&format!("}} catch ({error}) {{\n"));
            from_stmt(out, level + 1, catch);
            indent(out, level);
            out.push_str("}\n");
        }
        // Upstream ends a `throw` without a newline.
        Stmt::Throw(e) => {
            indent(out, level);
            out.push_str(&format!("throw {};", expr(e)));
        }
        Stmt::Return(e) => {
            indent(out, level);
            out.push_str(&format!("return {};\n", expr(e)));
        }
        Stmt::Var(name, e) => {
            indent(out, level);
            out.push_str(&format!("var {name} = {};\n", expr(e)));
        }
        Stmt::Vars(vars) if vars.is_empty() => {}
        Stmt::Vars(vars) => {
            indent(out, level);
            let vars: Vec<String> = (vars.iter())
                .map(|(name, e)| format!("{name} = {}", expr(e)))
                .collect();
            out.push_str(&format!("var {};\n", comma_newline_sep(level, &vars)));
        }
        Stmt::Function(name, args, body) => {
            indent(out, level);
            out.push_str(&format!("function {name}({}) {{\n", names(args)));
            from_block(out, level + 1, body);
            indent(out, level);
            out.push_str("}\n");
        }
    }
}

fn names(args: &[Name]) -> String {
    let args: Vec<String> = args.iter().map(Name::to_string).collect();
    comma_sep(&args)
}

/// `fromClause`.
fn from_clause(out: &mut String, level: usize, clause: &Case) {
    indent(out, level);
    match clause {
        Case::Case(e, stmts) => {
            out.push_str(&format!(
                "case {}:\n",
                from_expr(level, Grouping::Whatever, e).1
            ));
            from_block(out, level + 1, stmts);
        }
        Case::Default(stmts) => {
            out.push_str("default:\n");
            from_block(out, level + 1, stmts);
        }
    }
}

/// `Lines`: whether an expression takes one line or more.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lines {
    One,
    Many,
}

fn merge(a: Lines, b: Lines) -> Lines {
    if a == Lines::Many || b == Lines::Many {
        Lines::Many
    } else {
        Lines::One
    }
}

/// `Grouping`: whether an expression must be parenthesized.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grouping {
    Atomic,
    Whatever,
}

fn parens_for(grouping: Grouping, text: String) -> String {
    match grouping {
        Grouping::Atomic => format!("({text})"),
        Grouping::Whatever => text,
    }
}

/// `linesMap`: each printed, and whether any took many lines.
fn lines_map<T>(items: &[T], f: impl Fn(&T) -> (Lines, String)) -> (bool, Vec<String>) {
    let pairs: Vec<(Lines, String)> = items.iter().map(f).collect();
    let many = pairs.iter().any(|(l, _)| *l == Lines::Many);
    (many, pairs.into_iter().map(|(_, s)| s).collect())
}

/// `fromExpr`.
fn from_expr(level: usize, grouping: Grouping, expr: &Expr) -> (Lines, String) {
    match expr {
        Expr::String(text) => (Lines::One, format!("'{text}'")),
        Expr::Float(text) => (Lines::One, text.clone()),
        Expr::Int(n) => (Lines::One, n.to_string()),
        Expr::Bool(b) => (Lines::One, b.to_string()),
        Expr::Null => (Lines::One, "null".to_owned()),
        Expr::Json(json) => (Lines::One, json.clone()),
        Expr::Array(items) => {
            let (many, items) = lines_map(items, |e| from_expr(level, Grouping::Whatever, e));
            let text = if many {
                format!(
                    "[\n{}{}\n{}]",
                    tabs(level + 1),
                    comma_newline_sep(level, &items),
                    tabs(level)
                )
            } else {
                format!("[{}]", comma_sep(&items))
            };
            (Lines::Many, text)
        }
        Expr::Object(fields) => {
            let (many, fields) = lines_map(fields, |(name, e)| {
                let (lines, text) = from_expr(level + 1, Grouping::Whatever, e);
                (lines, format!("{name}: {text}"))
            });
            let text = if many {
                format!(
                    "{{\n{}{}\n{}}}",
                    tabs(level + 1),
                    comma_newline_sep(level, &fields),
                    tabs(level)
                )
            } else {
                format!("{{{}}}", comma_sep(&fields))
            };
            (Lines::Many, text)
        }
        Expr::Ref(name) => (Lines::One, name.to_string()),
        Expr::Access(e, field) => make_dot(level, e, field),
        Expr::Index(e, bracketed) => make_bracketed(level, e, bracketed),
        Expr::Prefix(op, e) => {
            let (lines, text) = from_expr(level, Grouping::Atomic, e);
            (
                lines,
                parens_for(grouping, format!("{}{text}", prefix(*op))),
            )
        }
        Expr::Infix(op, left, right) => {
            let (left_lines, left) = from_expr(level, Grouping::Atomic, left);
            let (right_lines, right) = from_expr(level, Grouping::Atomic, right);
            let text = format!("{left}{}{right}", infix(*op));
            (merge(left_lines, right_lines), parens_for(grouping, text))
        }
        Expr::If(condition, then, otherwise) => {
            let condition = from_expr(level, Grouping::Atomic, condition).1;
            let then = from_expr(level, Grouping::Atomic, then).1;
            let otherwise = from_expr(level, Grouping::Atomic, otherwise).1;
            let text = format!("{condition} ? {then} : {otherwise}");
            (Lines::Many, parens_for(grouping, text))
        }
        Expr::Assign(lvalue, e) => {
            let (left_lines, left) = from_lvalue(level, lvalue);
            let (right_lines, right) = from_expr(level, Grouping::Whatever, e);
            let text = format!("{left} = {right}");
            (merge(left_lines, right_lines), parens_for(grouping, text))
        }
        Expr::Call(function, args) => {
            let function = from_expr(level, Grouping::Atomic, function).1;
            let (many, args) = lines_map(args, |e| from_expr(level + 1, Grouping::Whatever, e));
            let text = if many {
                format!(
                    "{function}(\n{}{})",
                    tabs(level + 1),
                    comma_newline_sep(level, &args)
                )
            } else {
                format!("{function}({})", comma_sep(&args))
            };
            (Lines::Many, text)
        }
        Expr::Function(name, args, body) => {
            let mut text = format!(
                "function {}({}) {{\n",
                name.as_ref().map(Name::to_string).unwrap_or_default(),
                names(args)
            );
            from_block(&mut text, level + 1, body);
            text.push_str(&tabs(level));
            text.push('}');
            (Lines::Many, text)
        }
    }
}

/// `fromLValue`.
fn from_lvalue(level: usize, lvalue: &LValue) -> (Lines, String) {
    match lvalue {
        LValue::Ref(name) => (Lines::One, name.to_string()),
        LValue::Dot(e, field) => make_dot(level, e, field),
        LValue::Bracket(e, bracketed) => make_bracketed(level, e, bracketed),
    }
}

fn make_dot(level: usize, e: &Expr, field: &Name) -> (Lines, String) {
    let (lines, text) = from_expr(level, Grouping::Atomic, e);
    (lines, format!("{text}.{field}"))
}

fn make_bracketed(level: usize, e: &Expr, bracketed: &Expr) -> (Lines, String) {
    let (lines, text) = from_expr(level, Grouping::Atomic, e);
    let (bracketed_lines, bracketed) = from_expr(level, Grouping::Whatever, bracketed);
    (
        merge(lines, bracketed_lines),
        format!("{text}[{bracketed}]"),
    )
}

fn prefix(op: PrefixOp) -> &'static str {
    match op {
        PrefixOp::Not => "!",
        PrefixOp::Negate => "-",
        PrefixOp::Complement => "~",
    }
}

fn infix(op: InfixOp) -> &'static str {
    match op {
        InfixOp::Add => " + ",
        InfixOp::Sub => " - ",
        InfixOp::Mul => " * ",
        InfixOp::Div => " / ",
        InfixOp::Mod => " % ",
        InfixOp::Eq => " === ",
        InfixOp::Ne => " !== ",
        InfixOp::Lt => " < ",
        InfixOp::Le => " <= ",
        InfixOp::Gt => " > ",
        InfixOp::Ge => " >= ",
        InfixOp::And => " && ",
        InfixOp::Or => " || ",
        InfixOp::BitwiseAnd => " & ",
        InfixOp::BitwiseXor => " ^ ",
        InfixOp::BitwiseOr => " | ",
        InfixOp::LShift => " << ",
        InfixOp::SpRShift => " >> ",
        InfixOp::ZfRShift => " >>> ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Name {
        Name::raw(s)
    }

    #[test]
    fn short_and_long_calls_arrays_and_objects() {
        let call = Expr::Call(
            Box::new(Expr::Ref(n("f"))),
            vec![Expr::Int(1), Expr::Int(2)],
        );
        assert_eq!(expr(&call), "f(1, 2)");
        let long = Expr::Call(
            Box::new(Expr::Ref(n("g"))),
            vec![
                Expr::Function(None, vec![n("x")], vec![Stmt::Return(Expr::Ref(n("x")))]),
                Expr::Int(1),
            ],
        );
        assert_eq!(
            expr(&long),
            "g(\n\tfunction (x) {\n\t\treturn x;\n\t},\n\t1)"
        );
        let array = Expr::Array(vec![Expr::Int(1), call]);
        assert_eq!(expr(&array), "[\n\t1,\n\tf(1, 2)\n]");
        assert_eq!(
            expr(&Expr::Array(vec![Expr::Int(1), Expr::Bool(true)])),
            "[1, true]"
        );
        let object = Expr::Object(vec![
            (n("a"), Expr::Int(1)),
            (n("b"), Expr::String("x".into())),
        ]);
        assert_eq!(expr(&object), "{a: 1, b: 'x'}");
    }

    #[test]
    fn statements_indent_by_tabs() {
        let body = Stmt::While(
            Expr::Bool(true),
            Box::new(Stmt::Block(vec![
                Stmt::Var(
                    n("x"),
                    Expr::Infix(InfixOp::Add, Box::new(Expr::Int(1)), Box::new(Expr::Int(2))),
                ),
                Stmt::Continue(Some(n("loop"))),
            ])),
        );
        assert_eq!(
            stmt(&body),
            "while (true) {\n\tvar x = 1 + 2;\n\tcontinue loop;\n}\n"
        );
        let ternary = Expr::If(
            Box::new(Expr::Ref(n("c"))),
            Box::new(Expr::Prefix(PrefixOp::Negate, Box::new(Expr::Int(1)))),
            Box::new(Expr::Int(2)),
        );
        assert_eq!(expr(&ternary), "c ? (-1) : 2");
        assert_eq!(stmt(&Stmt::Throw(Expr::Ref(n("e")))), "throw e;");
        let vars = Stmt::Vars(vec![(n("a"), Expr::Int(1)), (n("b"), Expr::Int(2))]);
        assert_eq!(stmt(&vars), "var a = 1,\n\tb = 2;\n");
    }
}
