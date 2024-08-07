use calcu_rs::egraph::*;

/// A set of open expressions bound to variables.
///
/// Multipatterns bind many expressions to variables,
/// allowing for simultaneous searching or application of many terms
/// constrained to the same substitution.
///
/// Multipatterns are good for writing graph rewrites or datalog-style rules.
///
/// You can create multipatterns via the [`MultiPattern::new`] function or the
/// [`multi_rewrite!`] macro.
///
/// [`MultiPattern`] implements both [`Searcher`] and [`Applier`].
/// When searching a multipattern, the result ensures that
/// patterns bound to the same variable are equivalent.
/// When applying a multipattern, patterns bound a variable occuring in the
/// searcher are unioned with that e-class.
///
/// Multipatterns currently do not support the explanations feature.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MultiPattern {
    asts: Vec<(GlobalSymbol, PatternAst)>,
    program: machine::Program,
}

impl MultiPattern {
    /// Creates a new multipattern, binding the given patterns to the corresponding variables.
    pub fn new(asts: Vec<(GlobalSymbol, PatternAst)>) -> Self {
        let program = machine::Program::compile_from_multi_pat(&asts);
        Self { asts, program }
    }
}

impl<A: Analysis> Searcher<A> for MultiPattern {
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
            Some(SearchMatches {
                eclass,
                substs,
                ast: None,
            })
        }
    }

    fn vars(&self) -> Vec<GlobalSymbol> {
        let mut vars = vec![];
        for (v, pat) in &self.asts {
            vars.push(*v);
            for n in pat.as_ref() {
                if let ENodeOrVar::Var(v) = n {
                    vars.push(*v)
                }
            }
        }
        vars.sort();
        vars.dedup();
        vars
    }
}

impl<A: Analysis> Applier<A> for MultiPattern {
    fn apply_one(
        &self,
        _egraph: &mut EGraph<A>,
        _eclass: ID,
        _subst: &Subst,
        _searcher_ast: Option<&PatternAst>,
        _rule_name: GlobalSymbol,
    ) -> Vec<ID> {
        panic!("Multipatterns do not support apply_one")
    }

    fn apply_matches(
        &self,
        egraph: &mut EGraph<A>,
        matches: &[SearchMatches],
        _rule_name: GlobalSymbol,
    ) -> Vec<ID> {
        // TODO explanations?
        // the ids returned are kinda garbage
        let mut added = vec![];
        for mat in matches {
            for subst in &mat.substs {
                let mut subst = subst.clone();
                let mut id_buf = vec![];
                for (i, (v, p)) in self.asts.iter().enumerate() {
                    id_buf.resize(p.as_ref().len(), ID::new(0));
                    let id1 = pattern::apply_pat(&mut id_buf, p.as_ref(), egraph, &subst);
                    if let Some(id2) = subst.insert(*v, id1) {
                        egraph.union(id1, id2);
                    }
                    if i == 0 {
                        added.push(id1)
                    }
                }
            }
        }
        added
    }

    fn vars(&self) -> Vec<GlobalSymbol> {
        let mut bound_vars = HashSet::new();
        let mut vars = vec![];
        for (bv, pat) in &self.asts {
            for n in pat.as_ref() {
                if let ENodeOrVar::Var(v) = n {
                    // using vars that are already bound doesn't count
                    if !bound_vars.contains(v) {
                        vars.push(*v)
                    }
                }
            }
            bound_vars.insert(bv);
        }
        vars.sort();
        vars.dedup();
        vars
    }
}
