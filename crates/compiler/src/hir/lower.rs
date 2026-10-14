//! From the CST's typed views to the HIR, as upstream's parser builds
//! `Src.Module`: parentheses go, `else if` chains flatten into one `If`,
//! and each declaration lands in its list. What elm rejects but the
//! grammar reads (legacy ports and infixes, standalone doc comments)
//! lowers to nothing, and missing pieces to `Missing`.

use nomi_base::{TextRange, grow_large};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, significant_tokens};

use super::{
    Alias, Associativity, Def, DefId, Exposed, Exposing, Expr, ExprId, Fixity, Import, Infix,
    ManagerFields, Module, ModuleKind, NodePtr, PatId, Pattern, Port, Spanned, Type, TypeId, Union,
    VarType,
};

/// The HIR of `module`.
pub fn lower(module: &ast::Module) -> Module {
    let mut lower = Lower {
        module: Module::default(),
    };
    lower.module_(module);
    lower.module
}

struct Lower {
    module: Module,
}

fn spanned(token: &SyntaxToken) -> Spanned {
    Spanned {
        text: token.text().to_owned(),
        range: token.text_range(),
    }
}

/// The node's text without trivia, and the range from its first token
/// to its last: a qualified name, or an operator.
fn unspaced(node: &SyntaxNode) -> Spanned {
    let mut text = String::new();
    let mut range: Option<TextRange> = None;
    for token in significant_tokens(node) {
        text.push_str(token.text());
        let at = token.text_range();
        range = Some(range.map_or(at, |range| range.cover(at)));
    }
    Spanned {
        text,
        range: range.unwrap_or_else(|| TextRange::empty(node.text_range().start())),
    }
}

fn type_params(params: impl Iterator<Item = ast::TypeParam>) -> Vec<Spanned> {
    params
        .filter_map(|p| p.lower_ident_token())
        .map(|t| spanned(&t))
        .collect()
}

/// A value's name, an identifier or an operator in parentheses.
fn value_name(lower: Option<SyntaxToken>, operator: Option<ast::OperatorName>) -> Option<Spanned> {
    lower
        .map(|t| spanned(&t))
        .or_else(|| operator.map(|o| unspaced(o.syntax())))
}

/// From `keyword` to the end of `node`, which ends with code.
fn from_keyword(keyword: Option<SyntaxToken>, node: &SyntaxNode) -> TextRange {
    let end = node.text_range().end();
    let start = keyword.map_or_else(|| node.text_range().start(), |k| k.text_range().start());
    TextRange::new(start, end)
}

/// A name's qualifier, and the name, located from its qualifier when it
/// has one, as upstream locates `TTypeQual` and `PCtorQual`.
fn qualified(q: Option<ast::Qualifier>, name: &SyntaxToken) -> (Option<String>, Spanned) {
    let q = q.map(|q| unspaced(q.syntax()));
    let start = match &q {
        Some(q) => q.range.start(),
        None => name.text_range().start(),
    };
    let name = Spanned {
        text: name.text().to_owned(),
        range: TextRange::new(start, name.text_range().end()),
    };
    (q.map(|q| q.text.trim_end_matches('.').to_owned()), name)
}

fn qualifier(q: Option<ast::Qualifier>) -> Option<String> {
    q.map(|q| unspaced(q.syntax()).text.trim_end_matches('.').to_owned())
}

impl Lower {
    fn module_(&mut self, module: &ast::Module) {
        if let Some(header) = module.module_header() {
            self.module.name = header.module_name().map(|n| unspaced(n.syntax()));
            self.module.kind = if header.effect_token().is_some() {
                ModuleKind::Effect(manager(header.effect_fields().as_ref()))
            } else if header.port_token().is_some() {
                ModuleKind::Port
            } else {
                ModuleKind::Plain
            };
            if let Some(list) = header.exposing_list() {
                self.module.exports = exposing(&list);
                // From its `(`, as `addLocation` around `exposing` has it.
                let range = unspaced(list.syntax()).range;
                let open = list
                    .l_paren_token()
                    .map_or(range, |t| t.text_range())
                    .start();
                self.module.exports_range = TextRange::new(open, range.end());
                let mut next = list.syntax().last_token().and_then(|t| t.next_token());
                while let Some(token) = next.as_ref().filter(|t| t.kind().is_trivia()) {
                    next = token.next_token();
                }
                let end = next.map_or_else(
                    || module.syntax().text_range().end(),
                    |t| t.text_range().start(),
                );
                let start = self.module.exports_range.end();
                self.module.docs_gap = TextRange::new(start, end.max(start));
            }
        }
        self.module.docs = module.doc_comment_token().map(|t| spanned(&t));
        self.module.imports = module.imports().filter_map(|i| import(&i)).collect();
        for decl in module.decls() {
            self.decl(&decl);
        }
        // Without a header, a module that declares ports is a port module
        // (`checkModule`).
        if module.module_header().is_none() && !self.module.ports.is_empty() {
            self.module.kind = ModuleKind::Port;
        }
    }

    fn decl(&mut self, decl: &ast::Decl) {
        match decl {
            ast::Decl::ValueDecl(value) => {
                let docs = value.doc_comment_token().map(|t| spanned(&t));
                if let Some(def) = self.value(
                    value.signature(),
                    value.definition().as_ref(),
                    docs,
                    decl.syntax(),
                ) {
                    self.module.values.push(def);
                }
            }
            ast::Decl::DestructureDecl(d) => {
                let pattern = self.pattern(d.pattern());
                let body = self.expr(d.expr());
                let def = self.def(Def::Destruct { pattern, body }, Some(decl.syntax()));
                self.module.values.push(def);
            }
            ast::Decl::TypeAliasDecl(alias) => {
                let Some(name) = alias.upper_ident_token() else {
                    return;
                };
                let body = self.ty(alias.ty());
                self.module.aliases.push(Alias {
                    name: spanned(&name),
                    params: type_params(alias.type_params()),
                    body,
                    docs: alias.doc_comment_token().map(|t| spanned(&t)),
                    range: from_keyword(alias.type_token(), decl.syntax()),
                });
            }
            ast::Decl::CustomTypeDecl(union) => {
                let Some(name) = union.upper_ident_token() else {
                    return;
                };
                let params = type_params(union.type_params());
                let variants = union
                    .variants()
                    .filter_map(|v| {
                        let name = spanned(&v.upper_ident_token()?);
                        let args = v.types().map(|t| self.ty(Some(t))).collect();
                        Some((name, args))
                    })
                    .collect();
                self.module.unions.push(Union {
                    name: spanned(&name),
                    params,
                    variants,
                    docs: union.doc_comment_token().map(|t| spanned(&t)),
                    range: from_keyword(union.type_token(), decl.syntax()),
                });
            }
            ast::Decl::PortDecl(port) => {
                let Some(name) = port.lower_ident_token() else {
                    return;
                };
                let ty = self.ty(port.ty());
                self.module.ports.push(Port {
                    name: spanned(&name),
                    ty,
                    docs: port.doc_comment_token().map(|t| spanned(&t)),
                    range: from_keyword(port.port_token(), decl.syntax()),
                });
            }
            ast::Decl::InfixDecl(infix) => {
                let associativity = match (infix.left_token(), infix.right_token()) {
                    (Some(_), _) => Associativity::Left,
                    (None, Some(_)) => Associativity::Right,
                    (None, None) => Associativity::Non,
                };
                let (Some(op), Some(precedence), Some(function)) = (
                    infix.op(),
                    infix.int_token().and_then(|t| t.text().parse().ok()),
                    infix.lower_ident_token(),
                ) else {
                    return;
                };
                self.module.binops.push(Infix {
                    op: unspaced(op.syntax()),
                    fixity: Fixity {
                        associativity,
                        precedence,
                    },
                    function: spanned(&function),
                });
            }
            ast::Decl::LegacyPortDecl(_)
            | ast::Decl::LegacyInfixDecl(_)
            | ast::Decl::DocDecl(_) => {}
        }
    }

    /// A value from its signature and definition: `Src.Value` at the top,
    /// `Src.Define` in a `let`.
    fn value(
        &mut self,
        signature: Option<ast::Signature>,
        definition: Option<&ast::Definition>,
        docs: Option<Spanned>,
        node: &SyntaxNode,
    ) -> Option<DefId> {
        let name = definition
            .and_then(|d| value_name(d.lower_ident_token(), d.operator_name()))
            .or_else(|| {
                let s = signature.as_ref()?;
                value_name(s.lower_ident_token(), s.operator_name())
            })?;
        let annotation = signature.map(|s| self.ty(s.ty()));
        let params = definition
            .iter()
            .flat_map(|d| d.patterns())
            .map(|p| self.pattern(Some(p)))
            .collect();
        let body = self.expr(definition.and_then(|d| d.expr()));
        let def = Def::Define {
            name,
            params,
            body,
            annotation,
            docs,
        };
        Some(self.def(def, Some(node)))
    }

    fn def(&mut self, def: Def, node: Option<&SyntaxNode>) -> DefId {
        self.module.source_map.defs.push(node.map(NodePtr::new));
        self.module.defs.alloc(def)
    }

    fn alloc_expr(&mut self, expr: Expr, node: Option<&SyntaxNode>) -> ExprId {
        self.module.source_map.exprs.push(node.map(NodePtr::new));
        self.module.exprs.alloc(expr)
    }

    fn alloc_pattern(&mut self, pattern: Pattern, node: Option<&SyntaxNode>) -> PatId {
        self.module.source_map.patterns.push(node.map(NodePtr::new));
        self.module.patterns.alloc(pattern)
    }

    fn alloc_type(&mut self, ty: Type, node: Option<&SyntaxNode>) -> TypeId {
        self.module.source_map.types.push(node.map(NodePtr::new));
        self.module.types.alloc(ty)
    }

    fn expr(&mut self, expr: Option<ast::Expr>) -> ExprId {
        let Some(expr) = expr else {
            return self.alloc_expr(Expr::Missing, None);
        };
        grow_large(|| self.expr_(expr))
    }

    fn expr_(&mut self, expr: ast::Expr) -> ExprId {
        use ast::Expr as E;
        let node = expr.syntax().clone();
        let lowered = match expr {
            E::ParenExpr(paren) => return self.expr(paren.expr()),
            E::LiteralExpr(_) => match significant_tokens(&node).next() {
                Some(token) => {
                    let text = token.text().to_owned();
                    match token.kind() {
                        SyntaxKind::Int => Expr::Int(text),
                        SyntaxKind::Float => Expr::Float(text),
                        SyntaxKind::Char => Expr::Chr(text),
                        _ => Expr::Str(text),
                    }
                }
                None => Expr::Missing,
            },
            E::NameExpr(name) => {
                let lower = name.lower_ident_token().map(|t| (t, VarType::Low));
                match lower.or_else(|| name.upper_ident_token().map(|t| (t, VarType::Cap))) {
                    Some((token, kind)) => Expr::Var {
                        qualifier: qualifier(name.qualifier()),
                        name: token.text().to_owned(),
                        kind,
                    },
                    None => Expr::Missing,
                }
            }
            E::OperatorFunction(op) => match op.op() {
                Some(op) => Expr::Op(unspaced(op.syntax()).text),
                None => Expr::Missing,
            },
            E::NegateExpr(negate) => Expr::Negate(self.expr(negate.expr())),
            E::CallExpr(call) => {
                let function = self.expr(call.function());
                let args = call.arguments().map(|a| self.expr(Some(a))).collect();
                Expr::Call { function, args }
            }
            E::OperatorChain(chain) => self.binops(&chain),
            E::UnitExpr(_) => Expr::Unit,
            E::TupleExpr(tuple) => Expr::Tuple(tuple.exprs().map(|e| self.expr(Some(e))).collect()),
            E::ListExpr(list) => Expr::List(list.exprs().map(|e| self.expr(Some(e))).collect()),
            E::RecordExpr(record) => Expr::Record(self.fields(record.fields())),
            E::RecordUpdateExpr(update) => match update.lower_ident_token() {
                Some(record) => Expr::Update {
                    record: spanned(&record),
                    fields: self.fields(update.fields()),
                },
                None => Expr::Missing,
            },
            E::FieldAccessExpr(access) => {
                let record = self.expr(access.expr());
                match access.lower_ident_token() {
                    Some(field) => Expr::Access {
                        record,
                        field: spanned(&field),
                    },
                    None => Expr::Missing,
                }
            }
            E::AccessorExpr(accessor) => match accessor.lower_ident_token() {
                Some(field) => Expr::Accessor(field.text().to_owned()),
                None => Expr::Missing,
            },
            E::IfExpr(branch) => self.if_chain(branch),
            E::CaseExpr(case) => {
                let scrutinee = self.expr(case.expr());
                let branches = case
                    .case_branches()
                    .map(|b| (self.pattern(b.pattern()), self.expr(b.expr())))
                    .collect();
                Expr::Case {
                    scrutinee,
                    branches,
                }
            }
            E::LetExpr(let_) => self.let_(&let_),
            E::LambdaExpr(lambda) => {
                let params = lambda.patterns().map(|p| self.pattern(Some(p))).collect();
                let body = self.expr(lambda.expr());
                Expr::Lambda { params, body }
            }
            E::GlslExpr(glsl) => match glsl.glsl_token() {
                Some(token) => {
                    let text = token.text();
                    let source = text.strip_prefix("[glsl|").unwrap_or(text);
                    Expr::Shader(source.strip_suffix("|]").unwrap_or(source).to_owned())
                }
                None => Expr::Missing,
            },
            // elm rejects both; elm-format reads them.
            E::TupleConstructorExpr(_) | E::RangeExpr(_) => Expr::Missing,
        };
        self.alloc_expr(lowered, Some(&node))
    }

    fn binops(&mut self, chain: &ast::OperatorChain) -> Expr {
        let mut operands: Vec<_> = chain.exprs().map(|e| self.expr(Some(e))).collect();
        let ops: Vec<_> = chain.ops().map(|op| unspaced(op.syntax())).collect();
        // A recovered chain still has an operand around each operator:
        // missing ones are `Missing`, extra ones go.
        while operands.len() < ops.len() + 1 {
            operands.push(self.expr(None));
        }
        operands.truncate(ops.len() + 1);
        Expr::Binops { operands, ops }
    }

    /// An `if` and the `else if`s after it, as one expression.
    fn if_chain(&mut self, mut branch: ast::IfExpr) -> Expr {
        let mut branches = Vec::new();
        loop {
            let condition = self.expr(branch.condition());
            let then = self.expr(branch.then_branch());
            branches.push((condition, then));
            match branch.else_branch() {
                Some(ast::Expr::IfExpr(next)) => branch = next,
                otherwise => {
                    let otherwise = self.expr(otherwise);
                    return Expr::If {
                        branches,
                        otherwise,
                    };
                }
            }
        }
    }

    fn let_(&mut self, let_: &ast::LetExpr) -> Expr {
        let defs = let_
            .let_decls()
            .filter_map(|decl| match decl {
                ast::LetDecl::LetValue(value) => self.value(
                    value.signature(),
                    value.definition().as_ref(),
                    None,
                    value.syntax(),
                ),
                ast::LetDecl::LetDestructure(d) => {
                    let pattern = self.pattern(d.pattern());
                    let body = self.expr(d.expr());
                    Some(self.def(Def::Destruct { pattern, body }, Some(d.syntax())))
                }
            })
            .collect();
        let body = self.expr(let_.expr());
        Expr::Let { defs, body }
    }

    fn fields(&mut self, fields: impl Iterator<Item = ast::Field>) -> Vec<(Spanned, ExprId)> {
        fields
            .filter_map(|f| {
                let name = spanned(&f.lower_ident_token()?);
                Some((name, self.expr(f.expr())))
            })
            .collect()
    }

    fn pattern(&mut self, pattern: Option<ast::Pattern>) -> PatId {
        let Some(pattern) = pattern else {
            return self.alloc_pattern(Pattern::Missing, None);
        };
        grow_large(|| self.pattern_(pattern))
    }

    fn pattern_(&mut self, pattern: ast::Pattern) -> PatId {
        use ast::Pattern as P;
        let node = pattern.syntax().clone();
        let lowered = match pattern {
            P::ParenPattern(paren) => return self.pattern(paren.pattern()),
            P::VarPattern(var) => match var.lower_ident_token() {
                Some(name) => Pattern::Var(name.text().to_owned()),
                None => Pattern::Missing,
            },
            P::WildcardPattern(_) => Pattern::Anything,
            P::LiteralPattern(literal) => {
                let text = unspaced(&node).text;
                if literal.int_token().is_some() {
                    Pattern::Int(text)
                } else if literal.char_token().is_some() {
                    Pattern::Chr(text)
                } else if literal.float_token().is_some() {
                    // elm rejects float patterns.
                    Pattern::Missing
                } else {
                    Pattern::Str(text)
                }
            }
            P::UnitPattern(_) => Pattern::Unit,
            P::TuplePattern(tuple) => {
                Pattern::Tuple(tuple.patterns().map(|p| self.pattern(Some(p))).collect())
            }
            P::ListPattern(list) => {
                Pattern::List(list.patterns().map(|p| self.pattern(Some(p))).collect())
            }
            P::RecordPattern(record) => Pattern::Record(
                record
                    .field_patterns()
                    .filter_map(|f| f.lower_ident_token())
                    .map(|t| spanned(&t))
                    .collect(),
            ),
            P::ConstructorPattern(ctor) => match ctor.upper_ident_token() {
                Some(name) => {
                    let args = ctor.patterns().map(|p| self.pattern(Some(p))).collect();
                    let (qualifier, name) = qualified(ctor.qualifier(), &name);
                    Pattern::Ctor {
                        qualifier,
                        name,
                        args,
                    }
                }
                None => Pattern::Missing,
            },
            P::ConsPattern(cons) => {
                let head = self.pattern(cons.head());
                let tail = self.pattern(cons.tail());
                Pattern::Cons(head, tail)
            }
            P::AliasPattern(alias) => {
                let inner = self.pattern(alias.pattern());
                match alias.lower_ident_token() {
                    Some(name) => Pattern::Alias(inner, spanned(&name)),
                    None => Pattern::Missing,
                }
            }
        };
        self.alloc_pattern(lowered, Some(&node))
    }

    fn ty(&mut self, ty: Option<ast::Type>) -> TypeId {
        let Some(ty) = ty else {
            return self.alloc_type(Type::Missing, None);
        };
        grow_large(|| self.ty_(ty))
    }

    fn ty_(&mut self, ty: ast::Type) -> TypeId {
        use ast::Type as T;
        let node = ty.syntax().clone();
        let lowered = match ty {
            T::ParenType(paren) => return self.ty(paren.ty()),
            T::NamedType(named) => match named.upper_ident_token() {
                Some(name) => {
                    let args = named.types().map(|t| self.ty(Some(t))).collect();
                    let (qualifier, name) = qualified(named.qualifier(), &name);
                    Type::Named {
                        qualifier,
                        name,
                        args,
                    }
                }
                None => Type::Missing,
            },
            T::TypeVar(var) => match var.lower_ident_token() {
                Some(name) => Type::Var(name.text().to_owned()),
                None => Type::Missing,
            },
            T::FunctionType(function) => {
                let param = self.ty(function.param());
                let result = self.ty(function.result());
                Type::Lambda(param, result)
            }
            T::UnitType(_) => Type::Unit,
            T::TupleType(tuple) => Type::Tuple(tuple.types().map(|t| self.ty(Some(t))).collect()),
            T::RecordType(record) => {
                let fields = record
                    .field_types()
                    .filter_map(|f| {
                        let name = spanned(&f.lower_ident_token()?);
                        Some((name, self.ty(f.ty())))
                    })
                    .collect();
                Type::Record {
                    fields,
                    extends: record
                        .record_extension()
                        .and_then(|e| e.lower_ident_token())
                        .map(|t| spanned(&t)),
                }
            }
            // elm rejects it; elm-format reads it.
            T::TupleConstructorType(_) => Type::Missing,
        };
        self.alloc_type(lowered, Some(&node))
    }
}

/// An effect manager's fields.
fn manager(fields: Option<&ast::EffectFields>) -> ManagerFields {
    let mut manager = ManagerFields::default();
    let mut first = None;
    for field in fields.into_iter().flat_map(|f| f.effect_fields()) {
        let (Some(key), Some(value)) = (field.lower_ident_token(), field.upper_ident_token())
        else {
            continue;
        };
        first.get_or_insert_with(|| key.text_range());
        let slot = match key.text() {
            "command" => &mut manager.command,
            "subscription" => &mut manager.subscription,
            _ => continue,
        };
        slot.get_or_insert_with(|| spanned(&value));
    }
    manager.first = first.unwrap_or_default();
    manager
}

fn exposing(list: &ast::ExposingList) -> Exposing {
    if list.dot_dot_token().is_some() {
        return Exposing::Open;
    }
    Exposing::Explicit(
        list.items()
            .filter_map(|item| match item {
                ast::Exposed::ExposedValue(v) => {
                    Some(Exposed::Lower(spanned(&v.lower_ident_token()?)))
                }
                // `(..)` is located at its `..`.
                ast::Exposed::ExposedType(t) => Some(Exposed::Upper(
                    spanned(&t.upper_ident_token()?),
                    t.exposed_variants().map(|v| {
                        let dots = v.dot_dot_token();
                        dots.map_or_else(|| v.syntax().text_range(), |d| d.text_range())
                    }),
                )),
                // An operator is located with its parentheses.
                ast::Exposed::ExposedOperator(o) => Some(Exposed::Operator(Spanned {
                    text: unspaced(o.op()?.syntax()).text,
                    range: unspaced(o.syntax()).range,
                })),
            })
            .collect(),
    )
}

fn import(import: &ast::Import) -> Option<Import> {
    Some(Import {
        name: unspaced(import.module_name()?.syntax()),
        alias: import
            .import_alias()
            .and_then(|a| a.upper_ident_token())
            .map(|t| spanned(&t)),
        exposing: import.exposing_list().map(|list| exposing(&list)),
    })
}
