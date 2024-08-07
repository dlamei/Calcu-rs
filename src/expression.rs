use crate::{
    egraph::{Analysis, Construct, EGraph, Rewrite},
    *,
};
use std::{
    cell::{Ref, RefCell},
    cmp::Ordering,
    collections::VecDeque,
    fmt::{self, Debug, Display, Formatter},
    hash::{Hash, Hasher},
    io,
    ops::{self, Deref, Index},
};

/// Most often used to store an index into an array of Nodes
#[derive(Clone, Debug, Copy, Ord, PartialOrd, Eq, PartialEq)]
#[repr(transparent)]
pub struct ID(pub(crate) u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    // Primitives
    Rational(Rational),
    Var(Symbol),
    Undef,

    // Operators
    Add([ID; 2]),
    Mul([ID; 2]),
    Pow([ID; 2]),
}

pub type NodeSet = IndexSet<Node>;

pub struct ExprContext {
    pub(crate) symbols: SymbolTable,
    pub(crate) nodes: RefCell<NodeSet>,
}

#[derive(Debug, Clone)]
pub struct Expr<'a> {
    id: ID,
    root: Node,
    cntxt: &'a ExprContext,
}

impl ExprContext {
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            nodes: RefCell::new(IndexSet::default()),
        }
    }

    fn sort_ids(n: &mut Node) {
        match n {
            Node::Add(ids) | Node::Mul(ids) => ids.sort_unstable(),
            Node::Rational(_) | Node::Var(_) | Node::Undef | Node::Pow(_) => {}
        }
    }

    pub(crate) fn insert(&self, mut n: Node) -> ID {
        Self::sort_ids(&mut n);
        let (indx, _) = self.nodes.borrow_mut().insert_full(n);
        ID::new(indx)
    }

    pub fn get(&self, id: ID) -> Ref<Node> {
        Ref::map(self.nodes.borrow(), |nodes| {
            nodes.get_index(id.val()).unwrap()
        })
    }
    pub fn get_rational(&self, id: ID) -> Option<Ref<Rational>> {
        Ref::filter_map(self.get(id), |n| {
            if let Node::Rational(r) = n {
                Some(r)
            } else {
                None
            }
        }).ok()
    }

    fn make_expr(&self, mut root: Node) -> Expr {
        Self::sort_ids(&mut root);
        let id = ID::new(self.nodes.borrow_mut().insert_full(root.clone()).0);
        Expr { id, root, cntxt: self }
    }

    pub fn var<S: AsRef<str>>(&self, s: S) -> Expr {
        self.make_expr(Node::Var(self.symbols.insert(s.as_ref())))
    }
    pub fn undef(&self) -> Expr {
        self.make_expr(Node::Undef)
    }
    pub fn rational(&self, r: Rational) -> Expr {
        self.make_expr(Node::Rational(r))
    }
    pub fn add(&self, lhs: Expr, rhs: Expr) -> Expr {
        self.make_expr(Node::Add([lhs.id(), rhs.id()]))
    }
    pub fn sub(&self, lhs: Expr, rhs: Expr) -> Expr {
        let min_one = self.rational(Rational::MINUS_ONE);
        let min_rhs = self.mul(min_one, rhs);
        self.add(lhs, min_rhs)
    }
    pub fn mul(&self, lhs: Expr, rhs: Expr) -> Expr {
        self.make_expr(Node::Mul([lhs.id(), rhs.id()]))
    }
    pub fn div(&self, lhs: Expr, rhs: Expr) -> Expr {
        let min_one = self.rational(Rational::MINUS_ONE);
        let div_rhs = self.pow(rhs, min_one);
        self.mul(lhs, div_rhs)
    }
    pub fn pow(&self, lhs: Expr, rhs: Expr) -> Expr {
        self.make_expr(Node::Pow([lhs.id(), rhs.id()]))
    }

    pub fn var_str(&self, s: &Symbol) -> &str {
        self.symbols.get(s)
    }

    pub fn is_rational(&self, id: ID, r: &Rational) -> bool {
        if let Node::Rational(rational) = &*self.get(id) {
            rational == r
        } else {
            false
        }
    }

    /// Check if we have the ast: Add([LHS, MUL([-1, RHS])])
    pub fn is_sub(&self, n: &Node) -> Option<(ID, ID)> {
        let (lhs, mul) = if let Node::Add([lhs, rhs]) = n {
            (*lhs, *rhs)
        } else {
            return None;
        };

        let (mut min_one, mut rhs) = if let Node::Mul([lhs, rhs]) = &*self.get(mul) {
            (*lhs, *rhs)
        } else {
            return None;
        };

        if self.is_rational(min_one, &Rational::MINUS_ONE) {
            return Some((lhs, rhs));
        }
        std::mem::swap(&mut min_one, &mut rhs);
        if self.is_rational(min_one, &Rational::MINUS_ONE) {
            return Some((lhs, rhs));
        }

        None
    }

    /// Check if we have the ast: MUL([LHS, DIV([-1, RHS])])
    pub fn is_div(&self, n: &Node) -> Option<(ID, ID)> {
        let (mut lhs, mut pow) = if let Node::Mul([lhs, rhs]) = n {
            (*lhs, *rhs)
        } else {
            return None;
        };

        let (rhs, min_one) = if let Node::Pow([lhs, rhs]) = &*self.get(pow) {
            (*lhs, *rhs)
        } else {
            std::mem::swap(&mut lhs, &mut pow);
            if let Node::Pow([lhs, rhs]) = &*self.get(pow) {
                (*lhs, *rhs)
            } else {
                return None;
            }
        };

        if self.is_rational(min_one, &Rational::MINUS_ONE) {
            Some((lhs, rhs))
        } else {
            None
        }
    }

    pub fn fmt_id(&self, id: ID) -> FmtAst<'_> {
        use f::FmtAst as E;
        use fmt_ast as f;
        let n = self.get(id);
        match n.deref() {
            Node::Rational(_) => E::Atom(f::Atom::Rational(self.get_rational(id).unwrap())),
            Node::Var(v) => E::Atom(f::Atom::Var(self.var_str(v))),
            Node::Undef => E::Atom(f::Atom::Undefined),
            n @ Node::Add([lhs, rhs]) => {
                if let Some((lhs, rhs)) = self.is_sub(n) {
                    self.fmt_id(lhs) - self.fmt_id(rhs)
                } else {
                    self.fmt_id(*lhs) + self.fmt_id(*rhs)
                }
            }
            n @ Node::Mul([lhs, rhs]) => {
                if let Some((lhs, rhs)) = self.is_div(n) {
                    self.fmt_id(lhs) / self.fmt_id(rhs)
                } else {
                    self.fmt_id(*lhs) * self.fmt_id(*rhs)
                }
            }
            Node::Pow([lhs, rhs]) => self.fmt_id(*lhs).pow(self.fmt_id(*rhs)),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn to_dot_to_png(&self, name: &str) -> io::Result<()> {
        let enodes: Vec<_> = self
            .nodes
            .borrow()
            .clone()
            .into_iter()
            .enumerate()
            .map(|(indx, node)| (node, ID::new(indx)))
            .collect();
        let egraph = EGraph::from_enodes(enodes, ());
        egraph.dot(&self.symbols).to_png(name)
    }
}

impl<'a> Expr<'a> {
    pub fn id(&self) -> ID {
        self.id
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    pub fn from_id(id: ID, cntxt: &'a ExprContext) -> Self {
        let root = cntxt.get(id).clone();
        Self {
            cntxt,
            id,
            root,
        }
    }

    // TODO: cache egraph
    pub fn apply_rules(self, analysis: ExprFold, rules: &[Rewrite<ExprFold>]) -> Expr<'a>
    {
        let start = Instant::now();
        let runner = egraph::Runner::<ExprFold, ()>::new(analysis)
            .with_explanations_enabled()
            .with_time_limit(Duration::from_millis(500))
            .with_expr(&self)
            .run(rules);
        info!("apply_rules time: {} ms", start.elapsed().as_millis());


        let extractor = egraph::Extractor::new(&runner.egraph, ExprCost { egraph: &runner.egraph });
        let (_, be) = extractor.find_best2(runner.roots[0], self.cntxt);
        be
    }

    pub fn get_node(&self, id: ID) -> Ref<Node> {
        self.cntxt.get(id)
    }

    pub fn extract_nodes(&self) -> Vec<Node> {
        // map old ids to new ones
        let mut ids = hashmap_with_capacity::<ID, ID>(self.cntxt.nodes.borrow().len());
        let mut nodes = Vec::default();
        let mut stack = VecDeque::default();
        let mut bfs_order_ids = Vec::default();

        stack.push_back(self.id());
        while let Some(id) = stack.pop_front() {
            bfs_order_ids.push(id);
            let n = &self.get_node(id);
            n.ids().iter().for_each(|id| stack.push_back(*id));
        }

        // reverse bfs -> children of node should already exist
        bfs_order_ids.into_iter().rev().for_each(|id| {
            let mut n = self.get_node(id).clone();
            n.ids_mut().iter_mut().for_each(|id| *id = ids[id]);
            nodes.push(n);
            let new_id = ID::new(nodes.len() - 1);
            ids.insert(id, new_id);
        });

        // ensure the operands appear before the operator
        if cfg!(debug_assertions) {
            nodes.iter().enumerate().for_each(|(id, n)| {
                n.ids()
                    .iter()
                    .for_each(|op_id| debug_assert!(op_id.val() < id))
            })
        }

        nodes
    }

    pub fn fmt_ast(&self) -> FmtAst {
        self.cntxt.fmt_id(self.id)
    }
}



impl Hash for ID {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0 as u64);
    }
}

impl ID {
    pub const MAX: ID = ID::new(u32::MAX as usize);

    #[inline(always)]
    pub(crate) const fn val(self) -> usize {
        self.0 as usize
    }

    /// because rust has no private implementations of public types
    #[inline(always)]
    pub(crate) const fn new(val: usize) -> Self {
        // is ok because if we overflow we have other problems
        // u32::MAX * sizeof(Rational) ~= 240 gb
        debug_assert!(val <= u32::MAX as usize);
        ID(val as u32)
    }
}

impl Node {
    pub const MINUS_TWO: Self = Node::Rational(Rational::MINUS_TWO);
    pub const MINUS_ONE: Self = Node::Rational(Rational::MINUS_ONE);
    pub const ZERO: Self = Node::Rational(Rational::ZERO);
    pub const ONE: Self = Node::Rational(Rational::ONE);
    pub const TWO: Self = Node::Rational(Rational::TWO);

    pub(crate) fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Node::Rational(r1), Node::Rational(r2)) => r1 == r2,
            (Node::Var(s1), Node::Var(s2)) => s1 == s2,
            (Node::Add(_), Node::Add(_))
            | (Node::Mul(_), Node::Mul(_))
            | (Node::Pow(_), Node::Pow(_)) => true,
            _ => false,
        }
    }

    pub(crate) const fn ids(&self) -> &[ID] {
        match self {
            Node::Rational(_) | Node::Var(_) | Node::Undef => &[],
            Node::Add(ids) | Node::Mul(ids) | Node::Pow(ids) => ids,
        }
    }
    pub(crate) fn ids_mut(&mut self) -> &mut [ID] {
        match self {
            Node::Rational(_) | Node::Var(_) | Node::Undef => &mut [],
            Node::Add(ids) | Node::Mul(ids) | Node::Pow(ids) => ids,
        }
    }

    pub const fn is_atom(&self) -> bool {
        self.ids().is_empty()
    }
}

impl Construct for Expr<'_> {
    type Discriminant = std::mem::Discriminant<Node>;

    fn discriminant(&self) -> Self::Discriminant {
        std::mem::discriminant(&self.root)
    }

    fn matches(&self, other: &Self) -> bool {
        self.root.matches(&other.root)
    }

    fn operands(&self) -> &[ID] {
        self.root.ids()
    }

    fn operands_mut(&mut self) -> &mut [ID] {
        self.root.ids_mut()
    }
}


impl Default for ExprContext {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for ExprContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExprContext")
            .field("nodes", &self.nodes)
            .finish()
    }
}

impl Hash for Expr<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.hash(state)
    }
}

impl Eq for Expr<'_> {}
impl PartialEq for Expr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.cntxt, other.cntxt)
    }
}

impl Ord for Expr<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.root.cmp(&other.root)
    }
}
impl PartialOrd for Expr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for ID {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct NodeFmt<'a> {
    node: &'a Node,
    symbol_table: &'a SymbolTable,
}
impl Debug for NodeFmt<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.node {
            Node::Rational(n) => write!(f, "{n}"),
            Node::Var(s) => write!(f, "{}", self.symbol_table.get(s)),
            Node::Undef => write!(f, "undef"),
            Node::Add(_) => write!(f, "+"),
            Node::Mul(_) => write!(f, "*"),
            Node::Pow(_) => write!(f, "^"),
        }
    }
}
impl Display for NodeFmt<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl Node {
    pub fn fmt_symbols<'a>(&'a self, symbol_table: &'a SymbolTable) -> NodeFmt {
        NodeFmt {
            node: self,
            symbol_table,
        }
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Node::Rational(n) => write!(f, "{n}"),
            Node::Var(s) => write!(f, "{s}"),
            Node::Undef => write!(f, "undef"),
            Node::Add(_) => write!(f, "+"),
            Node::Mul(_) => write!(f, "*"),
            Node::Pow(_) => write!(f, "^"),
        }
    }
}

fn dbg_fmt_graph(graph: &Expr, n: &Node, f: &mut Formatter<'_>) -> fmt::Result {
    match n {
        Node::Rational(r) => write!(f, "{}", r),
        Node::Var(s) => write!(f, "{}", &graph.cntxt.var_str(s)),
        Node::Undef => write!(f, "undef"),
        Node::Add(_) => write!(f, "Add"),
        Node::Mul(_) => write!(f, "Mul"),
        Node::Pow(_) => write!(f, "Pow"),
    }?;

    if !n.is_atom() {
        write!(f, "[")?;
        let ids = n.ids();
        for i in 0..ids.len() - 1 {
            let id = ids[i];
            dbg_fmt_graph(graph, &graph.get_node(id), f)?;
            write!(f, ", ")?;
        }
        let last_id = ids[ids.len() - 1];
        dbg_fmt_graph(graph, &graph.get_node(last_id), f)?;
        write!(f, "]")?;
    }

    Ok(())
}

impl Display for Expr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.cntxt.nodes.borrow().is_empty() {
            return write!(f, "[]");
        };
        let root = &self.root;
        if self.cntxt.nodes.borrow().len() == 1 {
            return write!(f, "{:?}", root);
        }
        dbg_fmt_graph(self, root, f)
    }
}
