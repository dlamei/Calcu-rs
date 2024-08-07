use fmt::Formatter;
use log::*;
use std::{
    borrow::Cow,
    convert::TryFrom,
    fmt::{self, Display},
};

use calcu_rs::egraph::*;

/// A pattern that can function as either a [`Searcher`] or [`Applier`].
///
/// A [`Pattern`] is essentially a for-all quantified expression with
/// [`GlobalSymbol`]s as the variables (in the logical sense).
///
/// When creating a [`Rewrite`], the most common thing to use as either
/// the left hand side (the [`Searcher`]) or the right hand side
/// (the [`Applier`]) is a [`Pattern`].
///
/// As a [`Searcher`], a [`Pattern`] does the intuitive
/// thing.
/// Here is a somewhat verbose formal-ish statement:
/// Searching for a pattern in an egraph yields substitutions
/// ([`Subst`]s) _s_ such that, for any _s'_—where instead of
/// mapping a variables to an eclass as _s_ does, _s'_ maps
/// a variable to an arbitrary expression represented by that
/// eclass—_p[s']_ (the pattern under substitution _s'_) is also
/// represented by the egraph.
///
/// As an [`Applier`], a [`Pattern`] performs the given substitution
/// and adds the result to the [`EGraph`].
///
/// Importantly, [`Pattern`] implements [`FromStr`] if the
/// [`Language`] does.
/// This is probably how you'll create most [`Pattern`]s.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Pattern {
    /// The actual pattern as a [`RecExpr`]
    pub ast: PatternAst,
    program: machine::Program,
}

/// A [`RecExpr`] that represents a
/// [`Pattern`].
pub type PatternAst = RecExpr<ENodeOrVar>;

impl Pattern {
    /// Creates a new pattern from the given pattern ast.
    pub fn new(ast: PatternAst) -> Self {
        let ast = ast.compact();
        let program = machine::Program::compile_from_pat(&ast);
        Pattern { ast, program }
    }

    /// Returns a list of the [`GlobalSymbol`]s in this pattern.
    pub fn vars(&self) -> Vec<GlobalSymbol> {
        let mut vars = vec![];
        for n in self.ast.as_ref() {
            if let ENodeOrVar::Var(v) = n {
                if !vars.contains(v) {
                    vars.push(*v)
                }
            }
        }
        vars
    }
}

impl Pattern {
    /// Pretty print this pattern as a sexpr with the given width
    pub fn pretty(&self, width: usize) -> String {
        self.ast.pretty(width)
    }
}

/// The language of [`Pattern`]s.
///
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum ENodeOrVar {
    /// An enode from the underlying [`Language`]
    ENode(Node),
    /// A pattern variable
    Var(GlobalSymbol),
}

/// The discriminant for the language of [`Pattern`]s.
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum ENodeOrVarDiscriminant {
    ENode(<Node as Construct>::Discriminant),
    Var(GlobalSymbol),
}

impl Construct for ENodeOrVar {
    type Discriminant = ENodeOrVarDiscriminant;

    #[inline(always)]
    fn discriminant(&self) -> Self::Discriminant {
        match self {
            ENodeOrVar::ENode(n) => ENodeOrVarDiscriminant::ENode(n.discriminant()),
            ENodeOrVar::Var(v) => ENodeOrVarDiscriminant::Var(*v),
        }
    }

    fn matches(&self, _other: &Self) -> bool {
        panic!("Should never call this")
    }

    fn operands(&self) -> &[ID] {
        match self {
            ENodeOrVar::ENode(n) => n.ids(),
            ENodeOrVar::Var(_) => &[],
        }
    }

    fn operands_mut(&mut self) -> &mut [ID] {
        match self {
            ENodeOrVar::ENode(n) => n.operands_mut(),
            ENodeOrVar::Var(_) => &mut [],
        }
    }
}

impl Display for ENodeOrVar {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ENode(node) => Display::fmt(node, f),
            Self::Var(var) => Display::fmt(var, f),
        }
    }
}

impl<'a> From<&'a [Node]> for Pattern {
    fn from(expr: &'a [Node]) -> Self {
        let nodes: Vec<_> = expr.iter().cloned().map(ENodeOrVar::ENode).collect();
        let ast = RecExpr::from(nodes);
        Self::new(ast)
    }
}

impl From<RecExpr<Node>> for PatternAst {
    fn from(expr: RecExpr<Node>) -> Self {
        let nodes: Vec<_> = expr.nodes.into_iter().map(ENodeOrVar::ENode).collect();
        RecExpr::from(nodes)
    }
}

impl From<&RecExpr<Node>> for Pattern {
    fn from(expr: &RecExpr<Node>) -> Self {
        Self::from(expr.as_ref())
    }
}

impl From<PatternAst> for Pattern {
    fn from(ast: PatternAst) -> Self {
        Self::new(ast)
    }
}

impl TryFrom<Pattern> for RecExpr<Node> {
    type Error = GlobalSymbol;
    fn try_from(pat: Pattern) -> Result<Self, Self::Error> {
        let nodes = pat.ast.as_ref().iter().cloned();
        let ns: Result<Vec<_>, _> = nodes
            .map(|n| match n {
                ENodeOrVar::ENode(n) => Ok(n),
                ENodeOrVar::Var(v) => Err(v),
            })
            .collect();
        ns.map(RecExpr::from)
    }
}

impl Display for Pattern {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.ast, f)
    }
}

/// The result of searching a [`Searcher`] over one eclass.
///
/// Note that one [`SearchMatches`] can contain many found
/// substitutions. So taking the length of a list of [`SearchMatches`]
/// tells you how many eclasses something was matched in, _not_ how
/// many matches were found total.
///
#[derive(Debug)]
pub struct SearchMatches<'a> {
    /// The eclass id that these matches were found in.
    pub eclass: ID,
    /// The substitutions for each match.
    pub substs: Vec<Subst>,
    /// Optionally, an ast for the matches used in proof production.
    pub ast: Option<Cow<'a, PatternAst>>,
}

impl<A: Analysis> Searcher<A> for Pattern {
    fn search_eclass_with_limit(
        &self,
        egraph: &EGraph<A>,
        eclass: ID,
        limit: usize,
    ) -> Option<SearchMatches> {
        let substs = self.program.run_with_limit(egraph, eclass, limit);
        if substs.is_empty() {
            None
        } else {
            let ast = Some(Cow::Borrowed(&self.ast));
            Some(SearchMatches {
                eclass,
                substs,
                ast,
            })
        }
    }

    fn search_with_limit(&self, egraph: &EGraph<A>, limit: usize) -> Vec<SearchMatches> {
        match self.ast.as_ref().last().unwrap() {
            ENodeOrVar::ENode(e) => {
                let key = e.discriminant();
                match egraph.classes_by_op.get(&key) {
                    None => vec![],
                    Some(ids) => rewrite::search_eclasses_with_limit(
                        self,
                        egraph,
                        ids.iter().cloned(),
                        limit,
                    ),
                }
            }
            ENodeOrVar::Var(_) => rewrite::search_eclasses_with_limit(
                self,
                egraph,
                egraph.classes().map(|e| e.id),
                limit,
            ),
        }
    }

    fn get_pattern_ast(&self) -> Option<&PatternAst> {
        Some(&self.ast)
    }

    fn vars(&self) -> Vec<GlobalSymbol> {
        Pattern::vars(self)
    }
}

impl<A: Analysis> Applier<A> for Pattern {
    fn apply_matches(
        &self,
        egraph: &mut EGraph<A>,
        matches: &[SearchMatches],
        rule_name: GlobalSymbol,
    ) -> Vec<ID> {
        let mut added = vec![];
        let ast = self.ast.as_ref();
        let mut id_buf = vec![ID::new(0); ast.len()];
        for mat in matches {
            let sast = mat.ast.as_ref().map(|cow| cow.as_ref());
            for subst in &mat.substs {
                let did_something;
                let id;
                if egraph.are_explanations_enabled() {
                    let (id_temp, did_something_temp) =
                        egraph.union_instantiations(sast.unwrap(), &self.ast, subst, rule_name);
                    did_something = did_something_temp;
                    id = id_temp;
                } else {
                    id = apply_pat(&mut id_buf, ast, egraph, subst);
                    did_something = egraph.union(id, mat.eclass);
                }

                if did_something {
                    added.push(id)
                }
            }
        }
        added
    }

    fn get_pattern_ast(&self) -> Option<&PatternAst> {
        Some(&self.ast)
    }

    fn apply_one(
        &self,
        egraph: &mut EGraph<A>,
        eclass: ID,
        subst: &Subst,
        searcher_ast: Option<&PatternAst>,
        rule_name: GlobalSymbol,
    ) -> Vec<ID> {
        let ast = self.ast.as_ref();
        let mut id_buf = vec![ID::new(0); ast.len()];
        let id = apply_pat(&mut id_buf, ast, egraph, subst);

        if let Some(ast) = searcher_ast {
            let (from, did_something) =
                egraph.union_instantiations(ast, &self.ast, subst, rule_name);
            if did_something {
                vec![from]
            } else {
                vec![]
            }
        } else if egraph.union(eclass, id) {
            vec![eclass]
        } else {
            vec![]
        }
    }

    fn vars(&self) -> Vec<GlobalSymbol> {
        Pattern::vars(self)
    }
}

pub(crate) fn apply_pat<A: Analysis>(
    ids: &mut [ID],
    pat: &[ENodeOrVar],
    egraph: &mut EGraph<A>,
    subst: &Subst,
) -> ID {
    debug_assert_eq!(pat.len(), ids.len());
    trace!("apply_rec {:2?} {:?}", pat, subst);

    for (i, pat_node) in pat.iter().enumerate() {
        let id = match pat_node {
            ENodeOrVar::Var(w) => subst[*w],
            ENodeOrVar::ENode(e) => {
                let n = e.clone().map_operands(|child| ids[child.val()]);
                trace!("adding: {:?}", n);
                egraph.add(n)
            }
        };
        ids[i] = id;
    }

    *ids.last().unwrap()
}
