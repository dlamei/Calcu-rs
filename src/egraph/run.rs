use std::{
    cmp::Ordering,
    fmt::{self, Debug, Display, Formatter},
};

use log::*;

use crate::egraph::*;

/** Faciliates running rewrites over an [`EGraph`].

One use for [`EGraph`]s is as the basis of a rewriting system.
Since an egraph never "forgets" state when applying a [`Rewrite`], you
can apply many rewrites many times quite efficiently.
After the egraph is "full" (the rewrites can no longer find new
equalities) or some other condition, the egraph compactly represents
many, many equivalent expressions.
At this point, the egraph is ready for extraction (see [`Extractor`])
which can pick the represented expression that's best according to
some cost function.

This technique is called
[equality saturation](https://www.cs.cornell.edu/~ross/publications/eqsat/)
in general.
However, there can be many challenges in implementing this "outer
loop" of applying rewrites, mostly revolving around which rules to run
and when to stop.

[`Runner`] is `egg`'s provided equality saturation engine that has
reasonable defaults and implements many useful things like saturation
checking, egraph size limits, and customizable rule
[scheduling](RewriteScheduler).
Consider using [`Runner`] before rolling your own outer loop.

Here are some of the things [`Runner`] does for you:

- Saturation checking

  [`Runner`] checks to see if any of the rules added anything
  new to the [`EGraph`]. If none did, then it stops, returning
  [`StopReason::Saturated`].

- Iteration limits

  You can set a upper limit of iterations to do in case the search
  doesn't stop for some other reason. If this limit is hit, it stops with
  [`StopReason::IterationLimit`].

- [`EGraph`] size limit

  You can set a upper limit on the number of enodes in the egraph.
  If this limit is hit, it stops with
  [`StopReason::NodeLimit`].

- Time limit

  You can set a time limit on the runner.
  If this limit is hit, it stops with
  [`StopReason::TimeLimit`].

- Rule scheduling

  Some rules enable themselves, blowing up the [`EGraph`] and
  preventing other rewrites from running as many times.
  To prevent this, you can provide your own [`RewriteScheduler`] to
  govern when to run which rules.

  [`BackoffScheduler`] is the default scheduler.

[`Runner`] generates [`Iteration`]s that record some data about
each iteration.
You can add your own data to this by implementing the
[`IterationData`] trait.
[`Runner`] is generic over the [`IterationData`] that it will be in the
[`Iteration`]s, but by default it uses `()`.
**/

pub struct Runner<A: Analysis, IterData = ()> {
    /// The [`EGraph`] used.
    pub egraph: EGraph<A>,
    /// Data accumulated over each [`Iteration`].
    pub iterations: Vec<Iteration<IterData>>,
    /// The roots of expressions added by the
    /// [`with_expr`](Runner::with_expr()) method, in insertion order.
    pub roots: Vec<ID>,
    /// Why the `Runner` stopped. This will be `None` if it hasn't
    /// stopped yet.
    pub stop_reason: Option<StopReason>,

    /// The hooks added by the
    /// [`with_hook`](Runner::with_hook()) method, in insertion order.
    #[allow(clippy::type_complexity)]
    pub hooks: Vec<Box<dyn FnMut(&mut Self) -> Result<(), String>>>,

    // limits
    pub(crate) iter_limit: usize,
    pub(crate) node_limit: usize,
    pub(crate) time_limit: Duration,

    start_time: Option<Instant>,
    scheduler: Box<dyn RewriteScheduler<A>>,
}

impl Analysis for () {
    type Data = ();

    fn make(_egraph: &mut EGraph<Self>, _enode: &Node) -> Self::Data {}

    fn merge(&mut self, _a: &mut Self::Data, _b: Self::Data) -> DidMerge {
        DidMerge(false, false)
    }
}

impl<A: Analysis + Default> Default for Runner<A, ()> {
    fn default() -> Self {
        Runner::new(A::default())
    }
}

impl<A: Analysis, IterData> Debug for Runner<A, IterData>
where
    IterData: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Use an exhaustive pattern match to ensure the Debug implementation and the struct stay in sync.
        let Runner {
            egraph,
            iterations,
            roots,
            stop_reason,
            hooks,
            iter_limit,
            node_limit,
            time_limit,
            start_time,
            scheduler: _,
        } = self;

        f.debug_struct("Runner")
            .field("egraph", egraph)
            .field("iterations", iterations)
            .field("roots", roots)
            .field("stop_reason", stop_reason)
            .field("hooks", &vec![format_args!("<dyn FnMut ..>"); hooks.len()])
            .field("iter_limit", iter_limit)
            .field("node_limit", node_limit)
            .field("time_limit", time_limit)
            .field("start_time", start_time)
            .field("scheduler", &format_args!("<dyn RewriteScheduler ..>"))
            .finish()
    }
}

/// Error returned by [`Runner`] when it stops.
///
#[derive(Debug, Clone)]
pub enum StopReason {
    /// The egraph saturated, i.e., there was an iteration where we
    /// didn't learn anything new from applying the rules.
    Saturated,
    /// The iteration limit was hit. The data is the iteration limit.
    IterationLimit(usize),
    /// The enode limit was hit. The data is the enode limit.
    NodeLimit(usize),
    /// The time limit was hit. The data is the time limit in seconds.
    TimeLimit(f64),
    /// Some other reason to stop.
    Other(String),
}

/// A report containing data about an entire [`Runner`] run.
///
/// This is basically a summary of the [`Iteration`] data,
/// but summed across iterations.
/// See [`Iteration`] docs for details about fields.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Report {
    /// The number of iterations this runner performed.
    pub iterations: usize,
    pub stop_reason: StopReason,
    pub egraph_nodes: usize,
    pub egraph_classes: usize,
    pub memo_size: usize,
    pub rebuilds: usize,
    pub total_time: f64,
    pub search_time: f64,
    pub apply_time: f64,
    pub rebuild_time: f64,
}

impl Display for Report {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "Runner report")?;
        writeln!(f, "=============")?;
        writeln!(f, "  Stop reason: {:?}", self.stop_reason)?;
        writeln!(f, "  Iterations: {}", self.iterations)?;
        writeln!(f, "  Egraph size: {} nodes, {} classes, {} memo", self.egraph_nodes, self.egraph_classes, self.memo_size)?;
        writeln!(f, "  Rebuilds: {}", self.rebuilds)?;
        writeln!(f, "  Total time: {}", self.total_time)?;
        writeln!(f, "    Search:  ({:.2}) {}", self.search_time / self.total_time, self.search_time)?;
        writeln!(f, "    Apply:   ({:.2}) {}", self.apply_time / self.total_time, self.apply_time)?;
        writeln!(f, "    Rebuild: ({:.2}) {}", self.rebuild_time / self.total_time, self.rebuild_time)?;
        Ok(())
    }
}

/// Data generated by running a [`Runner`] one iteration.
///
/// [ser]: https://docs.rs/serde/latest/serde/trait.Serialize.html
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Iteration<IterData> {
    /// The number of enodes in the egraph at the start of this
    /// iteration.
    pub egraph_nodes: usize,
    /// The number of eclasses in the egraph at the start of this
    /// iteration.
    pub egraph_classes: usize,
    /// A map from rule name to number of times it was _newly_ applied
    /// in this iteration.
    pub applied: IndexMap<GlobalSymbol, usize>,
    /// Seconds spent running hooks.
    pub hook_time: f64,
    /// Seconds spent searching in this iteration.
    pub search_time: f64,
    /// Seconds spent applying rules in this iteration.
    pub apply_time: f64,
    /// Seconds spent [`rebuild`](EGraph::rebuild())ing
    /// the egraph in this iteration.
    pub rebuild_time: f64,
    /// Total time spent in this iteration, including data generation time.
    pub total_time: f64,
    /// The user provided annotation for this iteration
    pub data: IterData,
    /// The number of rebuild iterations done after this iteration completed.
    pub n_rebuilds: usize,
    /// If the runner stopped on this iterations, this is the reason
    pub stop_reason: Option<StopReason>,
}

type RunnerResult<T> = std::result::Result<T, StopReason>;

impl<A: Analysis, IterData> Runner<A, IterData>
where
    IterData: IterationData<A>,
{
    /// Create a new `Runner` with the given analysis and default parameters.
    pub fn new(analysis: A) -> Self {
        Self {
            iter_limit: 30,
            node_limit: 10_000,
            time_limit: Duration::from_secs(5),

            egraph: EGraph::new(analysis),
            roots: vec![],
            iterations: vec![],
            stop_reason: None,
            hooks: vec![],

            start_time: None,
            scheduler: Box::<BackoffScheduler>::default(),
        }
    }

    /// Sets the iteration limit. Default: 30
    pub fn with_iter_limit(self, iter_limit: usize) -> Self {
        Self { iter_limit, ..self }
    }

    /// Sets the egraph size limit (in enodes). Default: 10,000
    pub fn with_node_limit(self, node_limit: usize) -> Self {
        Self { node_limit, ..self }
    }

    /// Sets the runner time limit. Default: 5 seconds
    pub fn with_time_limit(self, time_limit: Duration) -> Self {
        Self { time_limit, ..self }
    }

    /// Add a hook to instrument or modify the behavior of a [`Runner`].
    /// Each hook will run at the beginning of each iteration, i.e. before
    /// all the rewrites.
    ///
    /// If your hook modifies the e-graph, make sure to call
    /// [`rebuild`](EGraph::rebuild()).
    pub fn with_hook<F>(mut self, hook: F) -> Self
    where
        F: FnMut(&mut Self) -> Result<(), String> + 'static,
    {
        self.hooks.push(Box::new(hook));
        self
    }

    /// Change out the [`RewriteScheduler`] used by this [`Runner`].
    /// The default one is [`BackoffScheduler`].
    ///
    pub fn with_scheduler(self, scheduler: impl RewriteScheduler<A> + 'static) -> Self {
        let scheduler = Box::new(scheduler);
        Self { scheduler, ..self }
    }

    /// Add an expression to the egraph to be run.
    ///
    /// The eclass id of this addition will be recorded in the
    /// [`roots`](Runner::roots) field, ordered by
    /// insertion order.
    pub fn with_expr(mut self, expr: &Expr) -> Self {
        let id = self.egraph.add_expr(expr);
        self.roots.push(id);
        self
    }

    /// Replace the [`EGraph`] of this `Runner`.
    pub fn with_egraph(self, egraph: EGraph<A>) -> Self {
        Self { egraph, ..self }
    }

    /// Run this `Runner` until it stops.
    /// After this, the field
    /// [`stop_reason`](Runner::stop_reason) is guaranteed to be
    /// set.
    pub fn run<'a, R>(mut self, rules: R) -> Self
    where
        A: Analysis + 'a,
        R: IntoIterator<Item = &'a Rewrite<A>>,
    {
        let rules: Vec<&Rewrite<A>> = rules.into_iter().collect();
        check_rules(&rules);
        self.egraph.rebuild();
        loop {
            let iter = self.run_one(&rules);
            self.iterations.push(iter);
            let stop_reason = self.iterations.last().unwrap().stop_reason.clone();
            // we need to check_limits after the iteration is complete to check for iter_limit
            if let Some(stop_reason) = stop_reason.or_else(|| self.check_limits().err()) {
                info!("Stopping: {:?}", stop_reason);
                self.stop_reason = Some(stop_reason);
                break;
            }
        }

        assert!(!self.iterations.is_empty());
        assert!(self.stop_reason.is_some());
        self
    }

    /// Enable explanations for this runner's egraph.
    /// This allows the runner to explain why two expressions are
    /// equivalent with the [`explain_equivalence`](Runner::explain_equivalence) function.
    pub fn with_explanations_enabled(mut self) -> Self {
        self.egraph = self.egraph.with_explanations_enabled();
        self
    }

    /// By default, egg runs a greedy algorithm to reduce the size of resulting explanations (without complexity overhead).
    /// Use this function to turn this algorithm off.
    pub fn without_explanation_length_optimization(mut self) -> Self {
        self.egraph = self.egraph.without_explanation_length_optimization();
        self
    }

    /// By default, egg runs a greedy algorithm to reduce the size of resulting explanations (without complexity overhead).
    /// Use this function to turn this algorithm on again if you have turned it off.
    pub fn with_explanation_length_optimization(mut self) -> Self {
        self.egraph = self.egraph.with_explanation_length_optimization();
        self
    }

    /// Disable explanations for this runner's egraph.
    pub fn with_explanations_disabled(mut self) -> Self {
        self.egraph = self.egraph.with_explanations_disabled();
        self
    }

    /// Calls [`EGraph::explain_equivalence`](EGraph::explain_equivalence()).
    pub fn explain_equivalence(&mut self, left: &Expr, right: &Expr) -> Explanation {
        self.egraph.explain_equivalence(left, right)
    }

    /// Calls [`EGraph::explain_existance`](EGraph::explain_existance()).
    pub fn explain_existance(&mut self, expr: &Expr) -> Explanation {
        self.egraph.explain_existance(expr)
    }

    /// Calls [EGraph::explain_existance_pattern`](EGraph::explain_existance_pattern()).
    pub fn explain_existance_pattern(
        &mut self,
        pattern: &PatternAst,
        subst: &Subst,
    ) -> Explanation {
        self.egraph.explain_existance_pattern(pattern, subst)
    }

    /// Get an explanation for why an expression matches a pattern.
    pub fn explain_matches(
        &mut self,
        left: &Expr,
        right: &PatternAst,
        subst: &Subst,
    ) -> Explanation {
        self.egraph.explain_matches(left, right, subst)
    }

    /// Prints some information about a runners run.
    pub fn print_report(&self) {
        println!("{}", self.report())
    }

    /// Creates a [`Report`] summarizing this `Runner`s run.
    pub fn report(&self) -> Report {
        Report {
            stop_reason: self.stop_reason.clone().unwrap(),
            iterations: self.iterations.len(),
            egraph_nodes: self.egraph.total_number_of_nodes(),
            egraph_classes: self.egraph.number_of_classes(),
            memo_size: self.egraph.total_size(),
            rebuilds: self.iterations.iter().map(|i| i.n_rebuilds).sum(),
            search_time: self.iterations.iter().map(|i| i.search_time).sum(),
            apply_time: self.iterations.iter().map(|i| i.apply_time).sum(),
            rebuild_time: self.iterations.iter().map(|i| i.rebuild_time).sum(),
            total_time: self.iterations.iter().map(|i| i.total_time).sum(),
        }
    }

    fn run_one(&mut self, rules: &[&Rewrite<A>]) -> Iteration<IterData> {
        assert!(self.stop_reason.is_none());

        info!("\nIteration {}", self.iterations.len());

        self.try_start();
        let mut result = self.check_limits();

        let egraph_nodes = self.egraph.total_size();
        let egraph_classes = self.egraph.number_of_classes();

        let hook_time = Instant::now();
        let mut hooks = std::mem::take(&mut self.hooks);
        result = result.and_then(|_| {
            hooks
                .iter_mut()
                .try_for_each(|hook| hook(self).map_err(StopReason::Other))
        });
        self.hooks = hooks;
        let hook_time = hook_time.elapsed().as_secs_f64();

        let egraph_nodes_after_hooks = self.egraph.total_size();
        let egraph_classes_after_hooks = self.egraph.number_of_classes();

        let i = self.iterations.len();
        trace!("EGraph {:?}", self.egraph.dump());

        let start_time = Instant::now();

        let mut matches = Vec::new();
        let mut applied = IndexMap::default();
        result = result.and_then(|_| {
            rules.iter().try_for_each(|rw| {
                let ms = self.scheduler.search_rewrite(i, &self.egraph, rw);
                matches.push(ms);
                self.check_limits()
            })
        });

        let search_time = start_time.elapsed().as_secs_f64();
        info!("Search time: {}", search_time);

        let apply_time = Instant::now();

        result = result.and_then(|_| {
            rules.iter().zip(matches).try_for_each(|(rw, ms)| {
                let total_matches: usize = ms.iter().map(|m| m.substs.len()).sum();
                debug!("Applying {} {} times", rw.name, total_matches);

                let actually_matched = self.scheduler.apply_rewrite(i, &mut self.egraph, rw, ms);
                if actually_matched > 0 {
                    if let Some(count) = applied.get_mut(&rw.name) {
                        *count += actually_matched;
                    } else {
                        applied.insert(rw.name.to_owned(), actually_matched);
                    }
                    debug!("Applied {} {} times", rw.name, actually_matched);
                }
                self.check_limits()
            })
        });

        let apply_time = apply_time.elapsed().as_secs_f64();
        info!("Apply time: {}", apply_time);

        let rebuild_time = Instant::now();
        let n_rebuilds = self.egraph.rebuild();
        if self.egraph.are_explanations_enabled() {
            debug_assert!(self.egraph.check_each_explain(rules));
        }

        let rebuild_time = rebuild_time.elapsed().as_secs_f64();
        info!("Rebuild time: {}", rebuild_time);
        info!(
            "Size: n={}, e={}",
            self.egraph.total_size(),
            self.egraph.number_of_classes()
        );

        let can_be_saturated = applied.is_empty()
            && self.scheduler.can_stop(i)
            // now make sure the hooks didn't do anything
            && (egraph_nodes == egraph_nodes_after_hooks)
            && (egraph_classes == egraph_classes_after_hooks)
            // now make sure that conditional rules (which might add
            // nodes without applying) didn't do anything
            && (egraph_nodes == self.egraph.total_size())
            && (egraph_classes == self.egraph.number_of_classes());

        if can_be_saturated {
            result = result.and(Err(StopReason::Saturated))
        }

        Iteration {
            applied,
            egraph_nodes,
            egraph_classes,
            hook_time,
            search_time,
            apply_time,
            rebuild_time,
            n_rebuilds,
            data: IterData::make(self),
            total_time: start_time.elapsed().as_secs_f64(),
            stop_reason: result.err(),
        }
    }

    fn try_start(&mut self) {
        self.start_time.get_or_insert_with(Instant::now);
    }

    fn check_limits(&self) -> RunnerResult<()> {
        let elapsed = self.start_time.unwrap().elapsed();
        if elapsed > self.time_limit {
            return Err(StopReason::TimeLimit(elapsed.as_secs_f64()));
        }

        let size = self.egraph.total_size();
        if size > self.node_limit {
            return Err(StopReason::NodeLimit(size));
        }

        if self.iterations.len() >= self.iter_limit {
            return Err(StopReason::IterationLimit(self.iterations.len()));
        }

        Ok(())
    }
}

fn check_rules<A>(rules: &[&Rewrite<A>]) {
    let mut name_counts = IndexMap::new();
    for rw in rules {
        *name_counts.entry(rw.name).or_default() += 1
    }

    name_counts.retain(|_, count: &mut usize| *count > 1);
    if !name_counts.is_empty() {
        eprintln!("WARNING: Duplicated rule names may affect rule reporting and scheduling.");
        log::warn!("Duplicated rule names may affect rule reporting and scheduling.");
        for (name, &count) in name_counts.iter() {
            assert!(count > 1);
            eprintln!("Rule '{}' appears {} times", name, count);
            log::warn!("Rule '{}' appears {} times", name, count);
        }
    }
}

/** A way to customize how a [`Runner`] runs [`Rewrite`]s.

This gives you a way to prevent certain [`Rewrite`]s from exploding
the [`EGraph`] and dominating how much time is spent while running the
[`Runner`].

*/
#[allow(unused_variables)]
pub trait RewriteScheduler<A: Analysis> {
    /// Whether or not the [`Runner`] is allowed
    /// to say it has saturated.
    ///
    /// This is only called when the runner is otherwise saturated.
    /// Default implementation just returns `true`.
    fn can_stop(&mut self, iteration: usize) -> bool {
        true
    }

    /// A hook allowing you to customize rewrite searching behavior.
    /// Useful to implement rule management.
    ///
    /// Default implementation just calls
    /// [`Rewrite::search`](Rewrite::search()).
    fn search_rewrite<'a>(
        &mut self,
        iteration: usize,
        egraph: &EGraph<A>,
        rewrite: &'a Rewrite<A>,
    ) -> Vec<SearchMatches<'a>> {
        rewrite.search(egraph)
    }

    /// A hook allowing you to customize rewrite application behavior.
    /// Useful to implement rule management.
    ///
    /// Default implementation just calls
    /// [`Rewrite::apply`](Rewrite::apply())
    /// and returns number of new applications.
    fn apply_rewrite(
        &mut self,
        iteration: usize,
        egraph: &mut EGraph<A>,
        rewrite: &Rewrite<A>,
        matches: Vec<SearchMatches>,
    ) -> usize {
        rewrite.apply(egraph, &matches).len()
    }
}

/// A very simple [`RewriteScheduler`] that runs every rewrite every
/// time.
///
/// Using this is basically turning off rule scheduling.
/// It uses the default implementation for all [`RewriteScheduler`]
/// methods.
///
/// This is not the default scheduler; choose it with the
/// [`with_scheduler`](Runner::with_scheduler())
/// method.
///
#[derive(Debug)]
pub struct SimpleScheduler;

impl<A: Analysis> RewriteScheduler<A> for SimpleScheduler {}

/// A [`RewriteScheduler`] that implements exponentional rule backoff.
///
/// For each rewrite, there exists a configurable initial match limit.
/// If a rewrite search yield more than this limit, then we ban this
/// rule for number of iterations, double its limit, and double the time
/// it will be banned next time.
///
/// This seems effective at preventing explosive rules like
/// associativity from taking an unfair amount of resources.
///
/// [`BackoffScheduler`] is configurable in the builder-pattern style.
///
#[derive(Debug)]
pub struct BackoffScheduler {
    default_match_limit: usize,
    default_ban_length: usize,
    stats: IndexMap<GlobalSymbol, RuleStats>,
}

#[derive(Debug)]
struct RuleStats {
    times_applied: usize,
    banned_until: usize,
    times_banned: usize,
    match_limit: usize,
    ban_length: usize,
}

impl BackoffScheduler {
    /// Set the initial match limit after which a rule will be banned.
    /// Default: 1,000
    pub fn with_initial_match_limit(mut self, limit: usize) -> Self {
        self.default_match_limit = limit;
        self
    }

    /// Set the initial ban length.
    /// Default: 5 iterations
    pub fn with_ban_length(mut self, ban_length: usize) -> Self {
        self.default_ban_length = ban_length;
        self
    }

    fn rule_stats(&mut self, name: GlobalSymbol) -> &mut RuleStats {
        if self.stats.contains_key(&name) {
            &mut self.stats[&name]
        } else {
            self.stats.entry(name).or_insert(RuleStats {
                times_applied: 0,
                banned_until: 0,
                times_banned: 0,
                match_limit: self.default_match_limit,
                ban_length: self.default_ban_length,
            })
        }
    }

    /// Never ban a particular rule.
    pub fn do_not_ban(mut self, name: impl Into<GlobalSymbol>) -> Self {
        self.rule_stats(name.into()).match_limit = usize::MAX;
        self
    }

    /// Set the initial match limit for a rule.
    pub fn rule_match_limit(mut self, name: impl Into<GlobalSymbol>, limit: usize) -> Self {
        self.rule_stats(name.into()).match_limit = limit;
        self
    }

    /// Set the initial ban length for a rule.
    pub fn rule_ban_length(mut self, name: impl Into<GlobalSymbol>, length: usize) -> Self {
        self.rule_stats(name.into()).ban_length = length;
        self
    }
}

impl Default for BackoffScheduler {
    fn default() -> Self {
        Self {
            stats: Default::default(),
            default_match_limit: 1_000,
            default_ban_length: 5,
        }
    }
}

impl<A: Analysis> RewriteScheduler<A> for BackoffScheduler {
    fn can_stop(&mut self, iteration: usize) -> bool {
        let n_stats = self.stats.len();

        let mut banned: Vec<_> = self
            .stats
            .iter_mut()
            .filter(|(_, s)| s.banned_until > iteration)
            .collect();

        if banned.is_empty() {
            true
        } else {
            let min_ban = banned
                .iter()
                .map(|(_, s)| s.banned_until)
                .min()
                .expect("banned cannot be empty here");

            assert!(min_ban >= iteration);
            let delta = min_ban - iteration;

            let mut unbanned = vec![];
            for (name, s) in &mut banned {
                s.banned_until -= delta;
                if s.banned_until == iteration {
                    unbanned.push(name.as_str());
                }
            }

            assert!(!unbanned.is_empty());
            info!(
                "Banned {}/{}, fast-forwarded by {} to unban {}",
                banned.len(),
                n_stats,
                delta,
                unbanned.join(", "),
            );

            false
        }
    }

    fn search_rewrite<'a>(
        &mut self,
        iteration: usize,
        egraph: &EGraph<A>,
        rewrite: &'a Rewrite<A>,
    ) -> Vec<SearchMatches<'a>> {
        let stats = self.rule_stats(rewrite.name);

        if iteration < stats.banned_until {
            debug!(
                "Skipping {} ({}-{}), banned until {}...",
                rewrite.name, stats.times_applied, stats.times_banned, stats.banned_until,
            );
            return vec![];
        }

        let threshold = stats
            .match_limit
            .checked_shl(stats.times_banned as u32)
            .unwrap();
        let matches = rewrite.search_with_limit(egraph, threshold.saturating_add(1));
        let total_len: usize = matches.iter().map(|m| m.substs.len()).sum();
        if total_len > threshold {
            let ban_length = stats.ban_length << stats.times_banned;
            stats.times_banned += 1;
            stats.banned_until = iteration + ban_length;
            info!(
                "Banning {} ({}-{}) for {} iters: {} < {}",
                rewrite.name,
                stats.times_applied,
                stats.times_banned,
                ban_length,
                threshold,
                total_len,
            );
            vec![]
        } else {
            stats.times_applied += 1;
            matches
        }
    }
}

/// Custom data to inject into the [`Iteration`]s recorded by a [`Runner`]
///
/// This trait allows you to add custom data to the [`Iteration`]s
/// recorded as a [`Runner`] applies rules.
///
/// See the [`Runner`] docs for an example.
///
/// [`Runner`] is generic over the [`IterationData`] that it will be in the
/// [`Iteration`]s, but by default it uses `()`.
///
pub trait IterationData<A: Analysis>: Sized {
    /// Given the current [`Runner`], make the
    /// data to be put in this [`Iteration`].
    fn make(runner: &Runner<A, Self>) -> Self;
}

impl<A: Analysis> IterationData<A> for () {
    fn make(_: &Runner<A, Self>) -> Self {}
}

/// Extracting a single [`RecExpr`] from an [`EGraph`].
#[derive(Debug)]
pub struct Extractor<'a, A: Analysis, CF: CostFunction> {
    cost_function: CF,
    costs: HashMap<ID, (CF::Cost, Node)>,
    egraph: &'a EGraph<A>,
}

/** A cost function that can be used by an [`Extractor`].

If you'd like to access the [`Analysis`] data or anything else in the e-graph,
you can put a reference to the e-graph in your [`CostFunction`]:

Note that a particular e-class might occur in an expression multiple times.
This means that pathological, but nevertheless realistic cases
might overflow `usize` if you implement a cost function like [`AstSize`],
even if the actual [`RecExpr`] fits compactly in memory.
You might want to use [`saturating_add`](u64::saturating_add) to
ensure your cost function is still monotonic in this situation.
 **/
pub trait CostFunction {
    /// The `Cost` type. It only requires `PartialOrd` so you can use
    /// floating point types, but failed comparisons (`NaN`s) will
    /// result in a panic.
    type Cost: PartialOrd + Debug + Clone;

    /// Calculates the cost of an enode whose children are `Cost`s.
    ///
    /// For this to work properly, your cost function should be
    /// _monotonic_, i.e. `cost` should return a `Cost` greater than
    /// any of the child costs of the given enode.
    fn cost<C>(&mut self, enode: &Node, costs: C) -> Self::Cost
    where
        C: FnMut(ID) -> (Self::Cost, Node);

    // Calculates the total cost of a [`RecExpr`].
    //
    // As provided, this just recursively calls `cost` all the way
    // down the [`RecExpr`].
    //
    //fn cost_rec(&mut self, expr: &RecExpr<Node>) -> Self::Cost {
    //    let nodes = expr.as_ref();
    //    let mut costs = hashmap_with_capacity::<ID, Self::Cost>(nodes.len());
    //    for (i, node) in nodes.iter().enumerate() {
    //        let cost = self.cost(node, |i| costs[&i].clone());
    //        costs.insert(ID::new(i), cost);
    //    }
    //    let last_id = ID::new(expr.as_ref().len() - 1);
    //    costs[&last_id].clone()
    //}
}

/// A simple [`CostFunction`] that counts total AST size.
#[derive(Debug)]
pub struct AstSize;
impl CostFunction for AstSize {
    type Cost = usize;
    fn cost<C>(&mut self, enode: &Node, mut costs: C) -> Self::Cost
    where
        C: FnMut(ID) -> (Self::Cost, Node),
    {
        enode.fold(1, |sum, id| sum.saturating_add(costs(id).0))
    }
}

/// A simple [`CostFunction`] that counts maximum AST depth.
#[derive(Debug)]
pub struct AstDepth;
impl CostFunction for AstDepth {
    type Cost = usize;
    fn cost<C>(&mut self, enode: &Node, mut costs: C) -> Self::Cost
    where
        C: FnMut(ID) -> (Self::Cost, Node),
    {
        1 + enode.fold(0, |max, id| max.max(costs(id).0))
    }
}

fn cmp<T: PartialOrd>(a: &Option<T>, b: &Option<T>) -> Ordering {
    // None is high
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => a.partial_cmp(b).unwrap(),
    }
}

impl<'a, A, CF> Extractor<'a, A, CF>
where
    A: Analysis,
    CF: CostFunction,
{
    /// Create a new `Extractor` given an `EGraph` and a
    /// `CostFunction`.
    ///
    /// The extraction does all the work on creation, so this function
    /// performs the greedy search for cheapest representative of each
    /// eclass.
    pub fn new(egraph: &'a EGraph<A>, cost_function: CF) -> Self {
        let costs = HashMap::default();
        let mut extractor = Extractor {
            costs,
            egraph,
            cost_function,
        };
        extractor.find_costs();

        extractor
    }

    /// Find the cheapest (lowest cost) represented `RecExpr` in the
    /// given eclass.
    pub fn find_best(&self, eclass: ID) -> (CF::Cost, RecExpr<Node>) {
        let (cost, root) = self.costs[&self.egraph.canon_id(eclass)].clone();
        let expr = root.build_recexpr(|id| self.find_best_node(id).clone());
        (cost, expr)
    }

    // TODO: somehow finds cycles?
    fn extract_into_nodes<F>(root: Node, cntxt: &ExprContext, mut get_node: F) -> Expr
    where
        F: FnMut(ID) -> Node,
    {
        //let mut set = IndexSet::<Node>::default();
        let mut ids = HashMap::<ID, ID>::default();
        let mut todo = root.operands().to_vec();

        while let Some(id) = todo.last().copied() {
            if ids.contains_key(&id) {
                todo.pop();
                continue;
            }

            let node = get_node(id);

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
                let new_id = cntxt.insert(node);
                ids.insert(id, new_id);
                todo.pop();
            }
        }

        // finally, add the root node and create the expression
        //let root_id = ID::new(cntxt.nodes.insert_full(root.map_operands(|id| ids[&id])).0);
        let root_id = cntxt.insert(root.map_operands(|id| ids[&id]));
        Expr::from_id(root_id, cntxt)
    }

    pub(crate) fn dbg_node_cost(&self, eclass: ID) {
        let (cost, node) = self.costs[&self.egraph.canon_id(eclass)].clone();
        println!("({eclass}) {:?}: {:?}", node, cost);
        node.operands().iter().for_each(|i| self.dbg_node_cost(*i));
    }

    pub fn find_best2<'b>(&self, eclass: ID, cntxt: &'b ExprContext) -> (CF::Cost, Expr<'b>) {
        let (cost, root) = self.costs[&self.egraph.canon_id(eclass)].clone();

        let expr = Self::extract_into_nodes(root, cntxt, |id| self.find_best_node(id).clone());
        (cost, expr)
        //let expr = root.build_recexpr(|id| self.find_best_node(id).clone());
        //let expr = root.build_recexpr(|id| self.find_best_node(id).clone());
        //(cost, expr)
    }

    /// Find the cheapest e-node in the given e-class.
    pub fn find_best_node(&self, eclass: ID) -> &Node {
        &self.costs[&self.egraph.canon_id(eclass)].1
    }

    /// Find the cost of the term that would be extracted from this e-class.
    pub fn find_best_cost(&self, eclass: ID) -> CF::Cost {
        let (cost, _) = &self.costs[&self.egraph.canon_id(eclass)];
        cost.clone()
    }

    fn node_total_cost(&mut self, node: &Node) -> Option<CF::Cost> {
        let eg = &self.egraph;
        let has_cost = |id| self.costs.contains_key(&eg.canon_id(id));
        if node.all(has_cost) {
            let costs = &self.costs;
            let cost_f = |id| costs[&eg.canon_id(id)].clone();
            Some(self.cost_function.cost(node, cost_f))
        } else {
            None
        }
    }

    fn find_costs(&mut self) {
        let mut did_something = true;
        while did_something {
            did_something = false;

            for class in self.egraph.classes() {
                let pass = self.make_pass(class);
                match (self.costs.get(&class.id), pass) {
                    (None, Some(new)) => {
                        self.costs.insert(class.id, new);
                        did_something = true;
                    }
                    (Some(old), Some(new)) if new.0 < old.0 => {
                        self.costs.insert(class.id, new);
                        did_something = true;
                    }
                    _ => (),
                }
            }
        }

        for class in self.egraph.classes() {
            if !self.costs.contains_key(&class.id) {
                log::warn!(
                    "Failed to compute cost for eclass {}: {:?}",
                    class.id,
                    class.nodes
                )
            }
        }
    }

    fn make_pass(&mut self, eclass: &EClass<A::Data>) -> Option<(CF::Cost, Node)> {
        let (cost, node) = eclass
            .iter()
            .map(|n| (self.node_total_cost(n), n))
            .min_by(|a, b| cmp(&a.0, &b.0))
            .unwrap_or_else(|| panic!("Can't extract, eclass is empty: {:#?}", eclass));
        cost.map(|c| (c, node.clone()))
    }
}
