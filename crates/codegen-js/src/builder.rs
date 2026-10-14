//! `Generate/JavaScript/Builder.hs`: the JavaScript syntax code generation
//! builds, and its printing with upstream's exact whitespace: a tab per
//! level, short arrays, objects and calls on one line, and long ones one
//! item a line. All three releases print alike.

use std::fmt::Write;

use crate::name::Name;

/// `Expr`.
#[derive(Debug)]
pub(crate) enum Expr {
    /// A string's text, already escaped for single quotes.
    String(String),
    /// A float as its text.
    Float(String),
    Int(i64),
    Bool(bool),
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
    If {
        condition: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    Assign(LValue, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    /// An anonymous function.
    Function {
        params: Vec<Name>,
        body: Vec<Stmt>,
    },
}

/// `LValue`.
#[derive(Debug)]
pub(crate) enum LValue {
    Ref(Name),
    Bracket(Box<Expr>, Box<Expr>),
}

/// `Stmt`.
#[derive(Debug)]
pub(crate) enum Stmt {
    Block(Vec<Stmt>),
    Empty,
    Expr(Expr),
    If {
        condition: Expr,
        then: Box<Stmt>,
        otherwise: Box<Stmt>,
    },
    Switch(Expr, Vec<Case>),
    While(Expr, Box<Stmt>),
    Break(Name),
    Continue(Name),
    Labelled(Name, Box<Stmt>),
    Try {
        body: Box<Stmt>,
        error: Name,
        catch: Box<Stmt>,
    },
    Throw(Expr),
    Return(Expr),
    Var(Name, Expr),
    Vars(Vec<(Name, Expr)>),
    Function {
        name: Name,
        params: Vec<Name>,
        body: Vec<Stmt>,
    },
}

/// `Case`: a switch's clause.
#[derive(Debug)]
pub(crate) enum Case {
    Case(Expr, Vec<Stmt>),
    Default(Vec<Stmt>),
}

/// `InfixOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfixOp {
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
pub(crate) enum PrefixOp {
    Not,
    Negate,
    Complement,
}

/// `stmtToBuilder`, onto `out`.
pub(crate) fn stmt(out: &mut String, stmt: &Stmt) {
    from_stmt(out, 0, stmt);
}

/// `exprToBuilder`, onto `out`.
pub(crate) fn expr(out: &mut String, expr: &Expr) {
    from_expr(out, 0, Grouping::Whatever, expr);
}

/// [`expr`]'s text alone.
#[cfg(test)]
pub(crate) fn text(e: &Expr) -> String {
    let mut out = String::new();
    expr(&mut out, e);
    out
}

/// The indentation of `level`: a tab each.
fn indent(out: &mut String, level: usize) {
    out.extend(std::iter::repeat_n('\t', level));
}

/// A line at `level`: `before`, the expression `e` and `after`.
fn line(out: &mut String, level: usize, before: &str, e: &Expr, after: &str) {
    indent(out, level);
    out.push_str(before);
    from_expr(out, level, Grouping::Whatever, e);
    out.push_str(after);
}

/// The `}` closing a statement's block at `level`.
fn close(out: &mut String, level: usize) {
    indent(out, level);
    out.push_str("}\n");
}

fn from_block(out: &mut String, level: usize, stmts: &[Stmt]) {
    for stmt in stmts {
        from_stmt(out, level, stmt);
    }
}

/// `fromStmt`.
fn from_stmt(out: &mut String, level: usize, stmt: &Stmt) {
    match stmt {
        Stmt::Block(stmts) => from_block(out, level, stmts),
        Stmt::Empty => {}
        Stmt::Expr(e) => line(out, level, "", e, ";\n"),
        Stmt::If {
            condition,
            then,
            otherwise,
        } => {
            line(out, level, "if (", condition, ") {\n");
            from_stmt(out, level + 1, then);
            indent(out, level);
            out.push_str("} else {\n");
            from_stmt(out, level + 1, otherwise);
            close(out, level);
        }
        Stmt::Switch(e, clauses) => {
            line(out, level, "switch (", e, ") {\n");
            for clause in clauses {
                from_clause(out, level + 1, clause);
            }
            close(out, level);
        }
        Stmt::While(e, body) => {
            line(out, level, "while (", e, ") {\n");
            from_stmt(out, level + 1, body);
            close(out, level);
        }
        Stmt::Break(label) => jump(out, level, "break", label),
        Stmt::Continue(label) => jump(out, level, "continue", label),
        Stmt::Labelled(label, body) => {
            indent(out, level);
            _ = writeln!(out, "{label}:");
            from_stmt(out, level, body);
        }
        Stmt::Try { body, error, catch } => {
            indent(out, level);
            out.push_str("try {\n");
            from_stmt(out, level + 1, body);
            indent(out, level);
            _ = writeln!(out, "}} catch ({error}) {{");
            from_stmt(out, level + 1, catch);
            close(out, level);
        }
        // Upstream ends a `throw` without a newline.
        Stmt::Throw(e) => line(out, level, "throw ", e, ";"),
        Stmt::Return(e) => line(out, level, "return ", e, ";\n"),
        Stmt::Var(name, e) => vars(out, level, [(name, e)]),
        Stmt::Vars(declared) if declared.is_empty() => {}
        Stmt::Vars(declared) => vars(out, level, declared.iter().map(|(name, e)| (name, e))),
        Stmt::Function { name, params, body } => {
            indent(out, level);
            function(out, level, name.as_str(), params, body);
            out.push('\n');
        }
    }
}

/// `break` or `continue` to `label`.
fn jump(out: &mut String, level: usize, keyword: &str, label: &Name) {
    indent(out, level);
    _ = writeln!(out, "{keyword} {label};");
}

/// `var`, then each name and its value, a line each.
fn vars<'a>(
    out: &mut String,
    level: usize,
    declared: impl IntoIterator<Item = (&'a Name, &'a Expr)>,
) {
    indent(out, level);
    out.push_str("var ");
    for (i, (name, e)) in declared.into_iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
            indent(out, level + 1);
        }
        out.push_str(name.as_str());
        out.push_str(" = ");
        from_expr(out, level, Grouping::Whatever, e);
    }
    out.push_str(";\n");
}

/// A function, named or not, its body a level below `level`.
fn function(out: &mut String, level: usize, name: &str, params: &[Name], body: &[Stmt]) {
    out.push_str("function ");
    out.push_str(name);
    out.push('(');
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(param.as_str());
    }
    out.push_str(") {\n");
    from_block(out, level + 1, body);
    indent(out, level);
    out.push('}');
}

/// `fromClause`.
fn from_clause(out: &mut String, level: usize, clause: &Case) {
    match clause {
        Case::Case(e, stmts) => {
            line(out, level, "case ", e, ":\n");
            from_block(out, level + 1, stmts);
        }
        Case::Default(stmts) => {
            indent(out, level);
            out.push_str("default:\n");
            from_block(out, level + 1, stmts);
        }
    }
}

/// `Lines`: whether an expression takes one line or more.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Lines {
    One,
    Many,
}

/// `Grouping`: whether an expression must be parenthesized.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grouping {
    Atomic,
    Whatever,
}

/// `parensFor`: what `write` writes, parenthesized where an atom is
/// wanted.
fn parens_for(out: &mut String, grouping: Grouping, write: impl FnOnce(&mut String)) {
    match grouping {
        Grouping::Atomic => {
            out.push('(');
            write(out);
            out.push(')');
        }
        Grouping::Whatever => write(out),
    }
}

/// The `Lines` `fromExpr` gives with an expression's text: arrays,
/// objects, conditionals, calls and functions count as many lines
/// wherever they print.
fn lines(expr: &Expr) -> Lines {
    match expr {
        Expr::String(_)
        | Expr::Float(_)
        | Expr::Int(_)
        | Expr::Bool(_)
        | Expr::Json(_)
        | Expr::Ref(_) => Lines::One,
        Expr::Array(_)
        | Expr::Object(_)
        | Expr::If { .. }
        | Expr::Call(..)
        | Expr::Function { .. } => Lines::Many,
        Expr::Access(e, _) | Expr::Prefix(_, e) | Expr::Assign(LValue::Ref(_), e) => lines(e),
        Expr::Index(left, right) | Expr::Infix(_, left, right) => lines(left).max(lines(right)),
        Expr::Assign(LValue::Bracket(left, right), e) => {
            lines(left).max(lines(right)).max(lines(e))
        }
    }
}

/// `commaSep`, or `commaNewlineSep` when any item takes many lines: each
/// then on a line of its own a level below `level`. Whether one did.
fn items<T>(
    out: &mut String,
    level: usize,
    entries: &[T],
    lines_of: impl Fn(&T) -> Lines,
    write: impl Fn(&mut String, &T),
) -> Lines {
    let many = entries.iter().map(lines_of).max().unwrap_or(Lines::One);
    for (i, entry) in entries.iter().enumerate() {
        match many {
            Lines::Many => {
                out.push_str(if i == 0 { "\n" } else { ",\n" });
                indent(out, level + 1);
            }
            Lines::One if i > 0 => out.push_str(", "),
            Lines::One => {}
        }
        write(out, entry);
    }
    many
}

/// An array or object: its items, then `close`, on a line of its own if
/// the items took many.
fn group<T>(
    out: &mut String,
    level: usize,
    [open, close]: [char; 2],
    entries: &[T],
    lines_of: impl Fn(&T) -> Lines,
    write: impl Fn(&mut String, &T),
) {
    out.push(open);
    if items(out, level, entries, lines_of, write) == Lines::Many {
        out.push('\n');
        indent(out, level);
    }
    out.push(close);
}

/// `fromExpr`, its text onto `out`; [`lines`] tells the rest.
fn from_expr(out: &mut String, level: usize, grouping: Grouping, expr: &Expr) {
    match expr {
        Expr::String(text) => {
            out.push('\'');
            out.push_str(text);
            out.push('\'');
        }
        Expr::Float(text) | Expr::Json(text) => out.push_str(text),
        Expr::Int(n) => _ = write!(out, "{n}"),
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Array(entries) => group(out, level, ['[', ']'], entries, lines, |out, e| {
            from_expr(out, level, Grouping::Whatever, e);
        }),
        Expr::Object(fields) => group(
            out,
            level,
            ['{', '}'],
            fields,
            |(_, e)| lines(e),
            |out, (name, e)| {
                out.push_str(name.as_str());
                out.push_str(": ");
                from_expr(out, level + 1, Grouping::Whatever, e);
            },
        ),
        Expr::Ref(name) => out.push_str(name.as_str()),
        Expr::Access(e, field) => {
            from_expr(out, level, Grouping::Atomic, e);
            out.push('.');
            out.push_str(field.as_str());
        }
        Expr::Index(e, bracketed) => make_bracketed(out, level, e, bracketed),
        Expr::Prefix(op, e) => parens_for(out, grouping, |out| {
            out.push_str(prefix(*op));
            from_expr(out, level, Grouping::Atomic, e);
        }),
        Expr::Infix(op, left, right) => parens_for(out, grouping, |out| {
            from_expr(out, level, Grouping::Atomic, left);
            out.push_str(infix(*op));
            from_expr(out, level, Grouping::Atomic, right);
        }),
        Expr::If {
            condition,
            then,
            otherwise,
        } => parens_for(out, grouping, |out| {
            from_expr(out, level, Grouping::Atomic, condition);
            out.push_str(" ? ");
            from_expr(out, level, Grouping::Atomic, then);
            out.push_str(" : ");
            from_expr(out, level, Grouping::Atomic, otherwise);
        }),
        Expr::Assign(lvalue, e) => parens_for(out, grouping, |out| {
            match lvalue {
                LValue::Ref(name) => out.push_str(name.as_str()),
                LValue::Bracket(left, bracketed) => make_bracketed(out, level, left, bracketed),
            }
            out.push_str(" = ");
            from_expr(out, level, Grouping::Whatever, e);
        }),
        Expr::Call(function, args) => {
            from_expr(out, level, Grouping::Atomic, function);
            out.push('(');
            items(out, level, args, lines, |out, e| {
                from_expr(out, level + 1, Grouping::Whatever, e);
            });
            out.push(')');
        }
        Expr::Function { params, body } => function(out, level, "", params, body),
    }
}

/// `e[bracketed]`.
fn make_bracketed(out: &mut String, level: usize, e: &Expr, bracketed: &Expr) {
    from_expr(out, level, Grouping::Atomic, e);
    out.push('[');
    from_expr(out, level, Grouping::Whatever, bracketed);
    out.push(']');
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
        Name::from_local(s)
    }

    fn statement(s: &Stmt) -> String {
        let mut out = String::new();
        stmt(&mut out, s);
        out
    }

    #[test]
    fn short_and_long_calls_arrays_and_objects() {
        let call = || {
            Expr::Call(
                Box::new(Expr::Ref(n("f"))),
                vec![Expr::Int(1), Expr::Int(2)],
            )
        };
        assert_eq!(text(&call()), "f(1, 2)");
        let long = Expr::Call(
            Box::new(Expr::Ref(n("g"))),
            vec![
                Expr::Function {
                    params: vec![n("x")],
                    body: vec![Stmt::Return(Expr::Ref(n("x")))],
                },
                Expr::Int(1),
            ],
        );
        assert_eq!(
            text(&long),
            "g(\n\tfunction (x) {\n\t\treturn x;\n\t},\n\t1)"
        );
        let array = Expr::Array(vec![Expr::Int(1), call()]);
        assert_eq!(text(&array), "[\n\t1,\n\tf(1, 2)\n]");
        assert_eq!(
            text(&Expr::Array(vec![Expr::Int(1), Expr::Bool(true)])),
            "[1, true]"
        );
        let object = Expr::Object(vec![
            (n("a"), Expr::Int(1)),
            (n("b"), Expr::String("x".into())),
        ]);
        assert_eq!(text(&object), "{a: 1, b: 'x'}");
        let nested = Expr::Object(vec![(n("a"), Expr::Array(vec![call()]))]);
        assert_eq!(text(&nested), "{\n\ta: [\n\t\tf(1, 2)\n\t]\n}");
    }

    #[test]
    fn many_lines_pass_through_operators_and_assignments() {
        let bracketed = |e| {
            Expr::Assign(
                LValue::Bracket(Box::new(Expr::Ref(n("a"))), Box::new(e)),
                Box::new(Expr::Int(1)),
            )
        };
        let call = Expr::Call(Box::new(Expr::Ref(n("f"))), Vec::new());
        let args = vec![bracketed(Expr::Int(0)), Expr::Int(2)];
        assert_eq!(
            text(&Expr::Call(Box::new(Expr::Ref(n("g"))), args)),
            "g(a[0] = 1, 2)"
        );
        let args = vec![bracketed(call), Expr::Int(2)];
        assert_eq!(
            text(&Expr::Call(Box::new(Expr::Ref(n("g"))), args)),
            "g(\n\ta[f()] = 1,\n\t2)"
        );
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
                Stmt::Continue(n("loop")),
            ])),
        );
        assert_eq!(
            statement(&body),
            "while (true) {\n\tvar x = 1 + 2;\n\tcontinue loop;\n}\n"
        );
        let ternary = Expr::If {
            condition: Box::new(Expr::Ref(n("c"))),
            then: Box::new(Expr::Prefix(PrefixOp::Negate, Box::new(Expr::Int(1)))),
            otherwise: Box::new(Expr::Int(2)),
        };
        assert_eq!(text(&ternary), "c ? (-1) : 2");
        assert_eq!(statement(&Stmt::Throw(Expr::Ref(n("e")))), "throw e;");
        let vars = Stmt::Vars(vec![(n("a"), Expr::Int(1)), (n("b"), Expr::Int(2))]);
        assert_eq!(statement(&vars), "var a = 1,\n\tb = 2;\n");
    }
}
