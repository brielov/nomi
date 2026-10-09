//! From the CST's typed views to the HIR, as upstream's parser builds
//! `Src.Module`: parentheses go, `else if` chains flatten into one `If`,
//! and each declaration lands in its list. What elm rejects but the
//! grammar reads (legacy ports and infixes, standalone doc comments)
//! lowers to nothing, and missing pieces to `Missing`.

use nomi_base::TextRange;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use super::{
    Alias, Associativity, Def, DefId, Exposed, Exposing, Expr, ExprId, Import, Infix, Module,
    ModuleKind, NodePtr, PatId, Pattern, Port, Spanned, Type, TypeId, Union,
};

/// Stack kept free before a recursive step, and the size of each new
/// segment, as the parser keeps.
const RED_ZONE: usize = 128 * 1024;
const STACK_SEGMENT: usize = 2 * 1024 * 1024;

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
fn words(node: &SyntaxNode) -> Spanned {
    let tokens: Vec<SyntaxToken> = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .collect();
    let range = match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => {
            TextRange::new(first.text_range().start(), last.text_range().end())
        }
        _ => TextRange::empty(node.text_range().start()),
    };
    Spanned {
        text: tokens.iter().map(|t| t.text()).collect(),
        range,
    }
}

/// The first token of `node` that isn't trivia.
fn first_token(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
}

/// The last token of `node` among `kinds`.
fn last_of(node: &SyntaxNode, kinds: &[SyntaxKind]) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| kinds.contains(&t.kind()))
        .last()
}

fn qualifier(q: Option<ast::Qualifier>) -> Option<String> {
    q.map(|q| words(q.syntax()).text.trim_end_matches('.').to_owned())
}

fn doc(token: Option<SyntaxToken>) -> Option<Spanned> {
    token.as_ref().map(spanned)
}

impl Lower {
    fn module_(&mut self, module: &ast::Module) {
        if let Some(header) = module.module_header() {
            self.module.name = header.module_name().map(|n| words(n.syntax()));
            self.module.kind = if header.effect_token().is_some() {
                let fields = header
                    .effect_fields()
                    .into_iter()
                    .flat_map(|f| f.effect_fields())
                    .filter_map(|f| {
                        Some((
                            spanned(&f.lower_ident_token()?),
                            spanned(&f.upper_ident_token()?),
                        ))
                    })
                    .collect();
                ModuleKind::Effect(fields)
            } else if header.port_token().is_some() {
                ModuleKind::Port
            } else {
                ModuleKind::Plain
            };
            self.module.exports = header
                .exposing_list()
                .map(|list| exposing(&list))
                .unwrap_or_default();
        }
        self.module.docs = doc(module.doc_comment_token());
        self.module.imports = module.imports().filter_map(|i| import(&i)).collect();
        for decl in module.decls() {
            self.decl(&decl);
        }
    }

    fn decl(&mut self, decl: &ast::Decl) {
        let node = NodePtr::new(decl.syntax());
        match decl {
            ast::Decl::ValueDecl(value) => {
                let docs = doc(value.doc_comment_token());
                if let Some(def) =
                    self.value(value.signature(), value.definition(), docs, decl.syntax())
                {
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
                let params = alias
                    .type_params()
                    .filter_map(|p| p.lower_ident_token())
                    .map(|t| spanned(&t))
                    .collect();
                let body = self.ty(alias.ty());
                self.module.aliases.push(Alias {
                    name: spanned(&name),
                    params,
                    body,
                    docs: doc(alias.doc_comment_token()),
                    node,
                });
            }
            ast::Decl::CustomTypeDecl(union) => {
                let Some(name) = union.upper_ident_token() else {
                    return;
                };
                let params = union
                    .type_params()
                    .filter_map(|p| p.lower_ident_token())
                    .map(|t| spanned(&t))
                    .collect();
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
                    docs: doc(union.doc_comment_token()),
                    node,
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
                    docs: doc(port.doc_comment_token()),
                    node,
                });
            }
            ast::Decl::InfixDecl(infix) => {
                let associativity = match last_of(
                    infix.syntax(),
                    &[SyntaxKind::LeftKw, SyntaxKind::RightKw, SyntaxKind::NonKw],
                )
                .map(|t| t.kind())
                {
                    Some(SyntaxKind::LeftKw) => Associativity::Left,
                    Some(SyntaxKind::RightKw) => Associativity::Right,
                    _ => Associativity::Non,
                };
                let (Some(op), Some(precedence), Some(function)) = (
                    infix.op(),
                    infix.int_token().and_then(|t| t.text().parse().ok()),
                    infix.lower_ident_token(),
                ) else {
                    return;
                };
                self.module.binops.push(Infix {
                    op: words(op.syntax()),
                    associativity,
                    precedence,
                    function: spanned(&function),
                    node,
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
        definition: Option<ast::Definition>,
        docs: Option<Spanned>,
        node: &SyntaxNode,
    ) -> Option<DefId> {
        let name = definition
            .as_ref()
            .and_then(|d| {
                d.lower_ident_token()
                    .map(|t| spanned(&t))
                    .or_else(|| d.operator_name().map(|o| words(o.syntax())))
            })
            .or_else(|| {
                let s = signature.as_ref()?;
                s.lower_ident_token()
                    .map(|t| spanned(&t))
                    .or_else(|| s.operator_name().map(|o| words(o.syntax())))
            })?;
        let annotation = signature.map(|s| self.ty(s.ty()));
        let params = definition
            .iter()
            .flat_map(|d| d.patterns())
            .map(|p| self.pattern(Some(p)))
            .collect();
        let body = self.expr(definition.as_ref().and_then(|d| d.expr()));
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
        stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || self.expr_(expr))
    }

    fn expr_(&mut self, expr: ast::Expr) -> ExprId {
        use ast::Expr as E;
        let node = expr.syntax().clone();
        let lowered = match expr {
            E::ParenExpr(paren) => return self.expr(paren.expr()),
            E::LiteralExpr(_) => match first_token(&node) {
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
                match last_of(&node, &[SyntaxKind::LowerIdent, SyntaxKind::UpperIdent]) {
                    Some(token) => Expr::Var {
                        qualifier: qualifier(name.qualifier()),
                        name: token.text().to_owned(),
                        upper: token.kind() == SyntaxKind::UpperIdent,
                    },
                    None => Expr::Missing,
                }
            }
            E::OperatorFunction(op) => match op.op() {
                Some(op) => Expr::Op(words(op.syntax()).text),
                None => Expr::Missing,
            },
            E::NegateExpr(negate) => Expr::Negate(self.expr(negate.expr())),
            E::CallExpr(call) => {
                let function = self.expr(call.function());
                let args = call.arguments().map(|a| self.expr(Some(a))).collect();
                Expr::Call { function, args }
            }
            E::OperatorChain(chain) => {
                let operands = chain.exprs().map(|e| self.expr(Some(e))).collect();
                let ops = chain.ops().map(|op| words(op.syntax())).collect();
                Expr::Binops { operands, ops }
            }
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
            E::IfExpr(mut branch) => {
                let mut branches = Vec::new();
                loop {
                    let condition = self.expr(branch.condition());
                    let then = self.expr(branch.then_branch());
                    branches.push((condition, then));
                    match branch.else_branch() {
                        Some(E::IfExpr(next)) => branch = next,
                        otherwise => {
                            let otherwise = self.expr(otherwise);
                            break Expr::If {
                                branches,
                                otherwise,
                            };
                        }
                    }
                }
            }
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
            E::LetExpr(let_) => {
                let defs = let_
                    .let_decls()
                    .filter_map(|decl| match decl {
                        ast::LetDecl::LetValue(value) => {
                            self.value(value.signature(), value.definition(), None, value.syntax())
                        }
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
        stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || self.pattern_(pattern))
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
            P::LiteralPattern(_) => {
                let text = words(&node).text;
                match last_of(
                    &node,
                    &[SyntaxKind::Int, SyntaxKind::Float, SyntaxKind::Char],
                )
                .map(|t| t.kind())
                {
                    Some(SyntaxKind::Int) => Pattern::Int(text),
                    Some(SyntaxKind::Char) => Pattern::Chr(text),
                    // elm rejects float patterns.
                    Some(_) => Pattern::Missing,
                    None => Pattern::Str(text),
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
                    Pattern::Ctor {
                        qualifier: qualifier(ctor.qualifier()),
                        name: spanned(&name),
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
        stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || self.ty_(ty))
    }

    fn ty_(&mut self, ty: ast::Type) -> TypeId {
        use ast::Type as T;
        let node = ty.syntax().clone();
        let lowered = match ty {
            T::ParenType(paren) => return self.ty(paren.ty()),
            T::NamedType(named) => match named.upper_ident_token() {
                Some(name) => {
                    let args = named.types().map(|t| self.ty(Some(t))).collect();
                    Type::Named {
                        qualifier: qualifier(named.qualifier()),
                        name: spanned(&name),
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

fn exposing(list: &ast::ExposingList) -> Exposing {
    let open = list
        .syntax()
        .children_with_tokens()
        .any(|e| e.kind() == SyntaxKind::DotDot);
    if open {
        return Exposing::Open;
    }
    Exposing::Explicit(
        list.items()
            .filter_map(|item| match item {
                ast::Exposed::ExposedValue(v) => {
                    Some(Exposed::Lower(spanned(&v.lower_ident_token()?)))
                }
                ast::Exposed::ExposedType(t) => Some(Exposed::Upper(
                    spanned(&t.upper_ident_token()?),
                    t.exposed_variants().map(|v| v.syntax().text_range()),
                )),
                ast::Exposed::ExposedOperator(o) => {
                    Some(Exposed::Operator(words(o.op()?.syntax())))
                }
            })
            .collect(),
    )
}

fn import(import: &ast::Import) -> Option<Import> {
    Some(Import {
        name: words(import.module_name()?.syntax()),
        alias: import
            .import_alias()
            .and_then(|a| a.upper_ident_token())
            .map(|t| spanned(&t)),
        exposing: import.exposing_list().map(|list| exposing(&list)),
    })
}
