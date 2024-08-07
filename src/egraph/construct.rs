use calcu_rs::{
    egraph::explain::pretty_print,
    egraph::{explain::SExpr, *},
    *,
};
use std::{
    cmp::Ordering,
    fmt::{self, Debug, Display, Formatter},
    hash::Hash,
    ops::{BitOr, Index, IndexMut},
};

impl Construct for Node {
    type Discriminant = std::mem::Discriminant<Node>;

    fn discriminant(&self) -> Self::Discriminant {
        std::mem::discriminant(self)
    }

    fn matches(&self, other: &Self) -> bool {
        self.matches(other)
    }

    fn operands(&self) -> &[ID] {
        self.ids()
    }

    fn operands_mut(&mut self) -> &mut [ID] {
        self.ids_mut()
    }
}

/// A container for graph based expressions
pub trait Construct: Debug + Clone + Eq + Ord + Hash {
    /// Type representing the cases of this language.
    ///
    /// Used for short-circuiting the search for equivalent nodes.
    type Discriminant: Debug + Clone + Eq + Hash;

    /// Return the `Discriminant` of this node.
    #[allow(enum_intrinsics_non_enums)]
    fn discriminant(&self) -> Self::Discriminant;

    /// Returns true if this enode matches another enode.
    /// This should only consider the operator, not the children `Id`s.
    fn matches(&self, other: &Self) -> bool;

    /// Returns the operands of this e-node.
    fn operands(&self) -> &[ID];

    /// Returns a mutable slice of the operands of this e-node.
    fn operands_mut(&mut self) -> &mut [ID];

    /// Runs a given function on each operand `Id`.
    fn for_each_oprnd<F: FnMut(ID)>(&self, f: F) {
        self.operands().iter().copied().for_each(f)
    }

    /// Runs a given function on each operand `Id`, allowing mutation of that `Id`.
    fn for_each_oprnd_mut<F: FnMut(&mut ID)>(&mut self, f: F) {
        self.operands_mut().iter_mut().for_each(f)
    }

    /// Runs a falliable function on each operand, stopping if the function returns
    /// an error.
    fn try_for_each_oprnd<E, F>(&self, mut f: F) -> Result<(), E>
    where
        F: FnMut(ID) -> Result<(), E>,
        E: Clone,
    {
        self.fold(Ok(()), |res, id| res.and_then(|_| f(id)))
    }

    /// Returns the number of the operands this enode has.
    ///
    /// The default implementation uses `fold` to accumulate the number of
    /// children.
    fn len(&self) -> usize {
        self.fold(0, |len, _| len + 1)
    }

    /// Returns true if this enode has no operands.
    fn is_leaf(&self) -> bool {
        self.all(|_| false)
    }

    /// Runs a given function to replace the operands.
    fn update_operands<F: FnMut(ID) -> ID>(&mut self, mut f: F) {
        self.for_each_oprnd_mut(|id| *id = f(*id))
    }

    /// Creates a new enode with operands determined by the given function.
    fn map_operands<F: FnMut(ID) -> ID>(mut self, f: F) -> Self {
        self.update_operands(f);
        self
    }

    /// Folds over the operands, given an initial accumulator.
    fn fold<F, T>(&self, init: T, mut f: F) -> T
    where
        F: FnMut(T, ID) -> T,
        T: Clone,
    {
        let mut acc = init;
        self.for_each_oprnd(|id| acc = f(acc.clone(), id));
        acc
    }

    /// Returns true if the predicate is true on all operands.
    /// Does not short circuit.
    fn all<F: FnMut(ID) -> bool>(&self, mut f: F) -> bool {
        self.fold(true, |acc, id| acc && f(id))
    }

    /// Returns true if the predicate is true on any operand.
    /// Does not short circuit.
    fn any<F: FnMut(ID) -> bool>(&self, mut f: F) -> bool {
        self.fold(false, |acc, id| acc || f(id))
    }

    /// Make a [`RecExpr`] by mapping this enodes operands to other [`RecExpr`]s.
    ///
    /// This can be used to join together different expression with a new node.
    fn join_recexprs<F, E>(&self, mut child_recexpr: F) -> RecExpr<Self>
    where
        F: FnMut(ID) -> E,
        E: AsRef<[Self]>,
    {
        fn build<L: Construct>(to: &mut RecExpr<L>, from: &[L]) -> ID {
            let last = from.last().unwrap().clone();
            let new_node = last.map_operands(|id| {
                let i = id.val() + 1;
                build(to, &from[0..i])
            });
            to.add(new_node)
        }

        let mut expr = RecExpr::default();
        let node = self
            .clone()
            .map_operands(|id| build(&mut expr, child_recexpr(id).as_ref()));
        expr.add(node);
        expr
    }

    /// Build a [`RecExpr`] from an e-node.
    ///
    /// The provided `get_node` function must return the same node for a given
    /// [`Id`] on multiple invocations.
    fn build_recexpr<F>(&self, mut get_node: F) -> RecExpr<Self>
    where
        F: FnMut(ID) -> Self,
    {
        self.try_build_recexpr::<_, std::convert::Infallible>(|id| Ok(get_node(id)))
            .unwrap()
    }

    /// Same as [`Construct::build_recexpr`], but fallible.
    fn try_build_recexpr<F, Err>(&self, mut get_node: F) -> Result<RecExpr<Self>, Err>
    where
        F: FnMut(ID) -> Result<Self, Err>,
    {
        let mut set = IndexSet::<Self>::default();
        let mut ids = HashMap::<ID, ID>::default();
        let mut todo = self.operands().to_vec();

        while let Some(id) = todo.last().copied() {
            if ids.contains_key(&id) {
                todo.pop();
                continue;
            }

            let node = get_node(id)?;

            // check to see if we can do this node yet
            let mut ids_has_all_children = true;
            for child in node.operands() {
                if !ids.contains_key(child) {
                    ids_has_all_children = false;
                    todo.push(*child)
                }
            }

            // all children are processed, so we can lookup this node safely
            if ids_has_all_children {
                let node = node.map_operands(|id| ids[&id]);
                let new_id = set.insert_full(node).0;
                ids.insert(id, ID::new(new_id));
                todo.pop();
            }
        }

        // finally, add the root node and create the expression
        let mut nodes: Vec<Self> = set.into_iter().collect();
        nodes.push(self.clone().map_operands(|id| ids[&id]));
        Ok(RecExpr::from(nodes))
    }
}

/// This conceptually represents a recursive expression, but it's actually just
/// a list of enodes.
///
/// [`RecExpr`]s must satisfy the invariant that enodes' children must refer to
/// elements that come before it in the list. For example, the expression
/// `(+ (* x 5) x)` could be represented by a recursive expression of the form
/// `[Num(5), Var("x"), Mul(1, 0), Add(2, 1)]`.
///
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecExpr<L> {
    pub(crate) nodes: Vec<L>,
}

impl<L> Default for RecExpr<L> {
    fn default() -> Self {
        Self::from(vec![])
    }
}

impl<L> AsRef<[L]> for RecExpr<L> {
    fn as_ref(&self) -> &[L] {
        &self.nodes
    }
}

impl<L> From<Vec<L>> for RecExpr<L> {
    fn from(nodes: Vec<L>) -> Self {
        Self { nodes }
    }
}

impl<L> From<RecExpr<L>> for Vec<L> {
    fn from(val: RecExpr<L>) -> Self {
        val.nodes
    }
}

impl<L: Construct> RecExpr<L> {
    /// Adds a given enode to this `RecExpr`.
    /// The enode's children `Id`s must refer to elements already in this list.
    pub(crate) fn add(&mut self, node: L) -> ID {
        debug_assert!(
            node.all(|id| id.val() < self.nodes.len()),
            "node {:?} has children not in this expr: {:?}",
            node,
            self
        );
        self.nodes.push(node);
        ID::new(self.nodes.len() - 1)
    }

    pub(crate) fn compact(mut self) -> Self {
        let mut ids = hashmap_with_capacity::<ID, ID>(self.nodes.len());
        let mut set = IndexSet::new();
        for (i, node) in self.nodes.drain(..).enumerate() {
            let node = node.map_operands(|id| ids[&id]);
            let new_id = set.insert_full(node).0;
            ids.insert(ID::new(i), ID::new(new_id));
        }
        self.nodes.extend(set);
        self
    }

    pub(crate) fn extract(&self, new_root: ID) -> Self {
        self[new_root].build_recexpr(|id| self[id].clone())
    }

    /// Checks if this expr is a DAG, i.e. doesn't have any back edges
    pub fn is_dag(&self) -> bool {
        for (i, n) in self.nodes.iter().enumerate() {
            for &child in n.operands() {
                if child.val() >= i {
                    return false;
                }
            }
        }
        true
    }
}

impl<L: Construct> Index<ID> for RecExpr<L> {
    type Output = L;
    fn index(&self, id: ID) -> &L {
        &self.nodes[id.val()]
    }
}

impl<L: Construct> IndexMut<ID> for RecExpr<L> {
    fn index_mut(&mut self, id: ID) -> &mut L {
        &mut self.nodes[id.val()]
    }
}

impl<L: Construct + Display> Display for RecExpr<L> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.nodes.is_empty() {
            Display::fmt("()", f)
        } else {
            let s = self.to_sexpr().to_string();
            Display::fmt(&s, f)
        }
    }
}

impl<L: Construct + Display> RecExpr<L> {
    /// Convert this RecExpr into an Sexpr
    pub(crate) fn to_sexpr(&self) -> SExpr {
        let last = self.nodes.len() - 1;
        if !self.is_dag() {
            log::warn!("Tried to print a non-dag: {:?}", self.nodes);
        }
        self.to_sexpr_rec(last, &mut |_| None)
    }

    fn to_sexpr_rec(&self, i: usize, f: &mut impl FnMut(usize) -> Option<String>) -> SExpr {
        let node = &self.nodes[i];
        let op = SExpr::String(node.to_string());
        if node.is_leaf() {
            op
        } else {
            let mut vec = vec![op];
            for child in node.operands().iter().map(|i| i.val()) {
                vec.push(if let Some(s) = f(child) {
                    return SExpr::String(s);
                } else if child < i {
                    self.to_sexpr_rec(child, f)
                } else {
                    SExpr::String(format!("<<<< CYCLE to {} = {:?} >>>>", i, node))
                })
            }
            SExpr::List(vec)
        }
    }

    /// Pretty print with a maximum line length.
    ///
    /// This gives you a nice, indented, pretty-printed s-expression.
    pub fn pretty(&self, width: usize) -> String {
        let sexpr = self.to_sexpr();

        let mut buf = String::new();
        pretty_print(&mut buf, &sexpr, width, 1).unwrap();
        buf
    }
}

/// Result of [`Analysis::merge`] indicating which of the inputs
/// are different from the merged result.
///
/// The fields correspond to whether the initial `a` and `b` inputs to [`Analysis::merge`]
/// were different from the final merged value.
///
/// In both cases the result may be conservative -- they may indicate `true` even
/// when there is no difference between the input and the result.
///
/// `DidMerge`s can be "or"ed together using the `|` operator.
/// This can be useful for composing analyses.
pub struct DidMerge(pub bool, pub bool);

impl BitOr for DidMerge {
    type Output = DidMerge;

    fn bitor(mut self, rhs: Self) -> Self::Output {
        self.0 |= rhs.0;
        self.1 |= rhs.1;
        self
    }
}

/** Arbitrary data associated with an [`EClass`].

`egg` allows you to associate arbitrary data with each eclass.
The [`Analysis`] allows that data to behave well even across eclasses merges.

[`Analysis`] can prove useful in many situtations.
One common one is constant folding, a kind of partial evaluation.
In that case, the metadata is basically `Option<L>`, storing
the cheapest constant expression (if any) that's equivalent to the
enodes in this eclass.
See the test files [`math.rs`] and [`prop.rs`] for more complex
examples on this usage of [`Analysis`].

If you don't care about [`Analysis`], `()` implements it trivally,
just use that.
**/
pub trait Analysis: Sized {
    /// The per-[`EClass`] data for this analysis.
    type Data: Debug;

    /// Makes a new [`Analysis`] data for a given e-node.
    ///
    /// Note the mutable `egraph` parameter: this is needed for some
    /// advanced use cases, but most use cases will not need to mutate
    /// the e-graph in any way.
    /// It is **not** `make`'s responsiblity to insert the e-node;
    /// the e-node is "being inserted" when this function is called.
    /// Doing so will create an infinite loop.
    ///
    /// Note that `enode`'s children may not be canonical
    fn make(egraph: &mut EGraph<Self>, enode: &Node) -> Self::Data;

    /// An optional hook that allows inspection before a [`union`] occurs.
    /// When explanations are enabled, it gives two ids that represent the two particular terms being unioned, not the canonical ids for the two eclasses.
    /// It also gives a justification for the union when explanations are enabled.
    ///
    /// By default it does nothing.
    ///
    /// `pre_union` is called _a lot_, so doing anything significant
    /// (like printing) will cause things to slow down.
    ///
    /// [`union`]: EGraph::union()
    #[allow(unused_variables)]
    fn pre_union(egraph: &EGraph<Self>, id1: ID, id2: ID, justification: &Option<Justification>) {}

    /// Defines how to merge two `Data`s when their containing
    /// [`EClass`]es merge.
    ///
    /// This should update `a` to correspond to the merged analysis
    /// data.
    ///
    /// The result is a `DidMerge(a_merged, b_merged)` indicating whether
    /// the merged result is different from `a` and `b` respectively.
    ///
    /// Since `merge` can modify `a`, let `a0`/`a1` be the value of `a`
    /// before/after the call to `merge`, respectively.
    ///
    /// If `a0 != a1` the result must have `a_merged == true`. This may be
    /// conservative -- it may be `true` even if even if `a0 == a1`.
    ///
    /// If `b != a1` the result must have `b_merged == true`. This may be
    /// conservative -- it may be `true` even if even if `b == a1`.
    ///
    /// This function may modify the [`Analysis`], which can be useful as a way
    /// to store information for the [`Analysis::modify`] hook to process, since
    /// `modify` has access to the e-graph.
    fn merge(&mut self, a: &mut Self::Data, b: Self::Data) -> DidMerge;

    /// A hook that allows the modification of the
    /// [`EGraph`].
    ///
    /// By default this does nothing.
    ///
    /// This function is called immediately following
    /// `Analysis::merge` when unions are performed.
    #[allow(unused_variables)]
    fn modify(egraph: &mut EGraph<Self>, id: ID) {}

    /// Whether or not e-matching should allow finding cycles.
    ///
    /// By default, this returns `true`.
    ///
    /// Setting this to `false` can improve performance in some cases, but risks
    /// missing some equalities depending on the use case.
    fn allow_ematching_cycles(&self) -> bool {
        true
    }
}

/// A utility for implementing [`Analysis::merge`]
/// when the `Data` type has a total ordering.
/// This will take the maximum of the two values.
pub fn merge_max<T: Ord>(to: &mut T, from: T) -> DidMerge {
    let cmp = (*to).cmp(&from);
    match cmp {
        Ordering::Less => {
            *to = from;
            DidMerge(true, false)
        }
        Ordering::Equal => DidMerge(false, false),
        Ordering::Greater => DidMerge(false, true),
    }
}

/// A utility for implementing [`Analysis::merge`]
/// when the `Data` type has a total ordering.
/// This will take the minimum of the two values.
pub fn merge_min<T: Ord>(to: &mut T, from: T) -> DidMerge {
    let cmp = (*to).cmp(&from);
    match cmp {
        Ordering::Less => DidMerge(false, true),
        Ordering::Equal => DidMerge(false, false),
        Ordering::Greater => {
            *to = from;
            DidMerge(true, false)
        }
    }
}

/// A utility for implementing [`Analysis::merge`]
/// when the `Data` type is an [`Option`].
///
/// Always take a `Some` over a `None`
/// and calls the given function to merge two `Some`s.
pub fn merge_option<T>(
    to: &mut Option<T>,
    from: Option<T>,
    merge_fn: impl FnOnce(&mut T, T) -> DidMerge,
) -> DidMerge {
    match (to.as_mut(), from) {
        (None, None) => DidMerge(false, false),
        (None, from @ Some(_)) => {
            *to = from;
            DidMerge(true, false)
        }
        (Some(_), None) => DidMerge(false, true),
        (Some(a), Some(b)) => merge_fn(a, b),
    }
}
