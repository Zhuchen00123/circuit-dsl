//! The parameter dependency graph (round-4 phase B).
//!
//! Contract: `docs/review-evidence/round4/design-contract.md` §4. This module is
//! deliberately free of the AST and of the elaborator's state: it takes the
//! declarations of one body ([Decl]), the names their definitions read ([Read])
//! and the topology use sites the elaborator saw ([UseSite]), and answers three
//! questions:
//!
//! 1. in which order one body's parameters can be evaluated
//!    ([BodyGraph::order]), deterministically, with source order breaking every
//!    tie;
//! 2. which closed dependency path exists when no such order does
//!    ([BodyGraph::cycle]), with the span of every participating declaration;
//! 3. which parameters can move the circuit topology, and along which path
//!    ([DesignGraph::topology_path]), which is what makes a parameter sweep
//!    refusable before a single point is solved (contract §4.6).
//!
//! # Scope identity
//!
//! A node is a parameter **in one body instance**, identified by its [ScopePath]
//! and its name: `top`, `top.stage1`, `top.stage1.inner`. The bare name is never an
//! identity across scopes, so two instances of one subcircuit have two
//! independent `r` nodes and sweeping `top.r` can never implicate
//! `top.stage1.r`.
//!
//! Edges exist inside one body (contract §4.1). The one cross-scope channel is
//! an instance `params:` binding: `params: { x: <expr reading p> }` gives the
//! instance-local `x` the parent's `p` as a dependency
//! ([DesignGraph::bind_from_parent]), so a topology use site inside the instance
//! body propagates back to `p`. A binding that reaches no topology use point
//! leaves its parameter sweepable: the binding is the channel the value crosses
//! into the instance through, not a use point by itself.
//!
//! # Unknown names
//!
//! A name a definition reads that the body declares nowhere is not a node and
//! never becomes an edge ([BodyGraph::unknown_reads]), so an unknown name can
//! never be reported as a cycle. The elaborator still reports it as `E_NAME` at
//! the reference, which is the documented behaviour; the graph only has to keep
//! the two apart. An overridden declaration carries no reads at all (contract
//! §4.3), so a name only a replaced default mentions is not unknown either.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use circuit_core::span::SourceSpan;

/// The first segment of every path: the top-level circuit.
const ROOT: &str = "top";

// ---------------------------------------------------------------------------
// Scope identity
// ---------------------------------------------------------------------------

/// The instantiation path of one body instance.
///
/// `top` is the top-level circuit, `top.stage1` an instance of it, and so on —
/// the same path the elaborator already builds for node and device names.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct ScopePath {
    segments: Vec<String>,
}

impl ScopePath {
    /// The top-level circuit's body.
    pub fn root() -> Self {
        Self {
            segments: vec![ROOT.to_string()],
        }
    }

    /// The body one instance deeper.
    pub fn child(&self, segment: &str) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment.to_string());
        Self { segments }
    }

    /// The path segments, outermost first.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

impl fmt::Display for ScopePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.segments.is_empty() {
            // Only reachable for a default-constructed path; `root()` always has
            // a segment. Printed honestly rather than as an empty string.
            return f.write_str(ROOT);
        }
        f.write_str(&self.segments.join("."))
    }
}

// ---------------------------------------------------------------------------
// One body
// ---------------------------------------------------------------------------

/// A bare name read by a definition expression, with the span it was written at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Read {
    pub name: String,
    pub span: SourceSpan,
}

/// One `param` declaration of one body, as the graph sees it.
#[derive(Clone, Debug)]
pub struct Decl {
    /// The parameter's name, without the leading colon.
    pub name: String,
    /// The name span of `param :name, ...`.
    pub span: SourceSpan,
    /// The names the *effective* definition reads. Empty when the parameter has
    /// no default, and empty when an override supplies the value: an overridden
    /// declaration contributes no edge (contract §4.3).
    pub reads: Vec<Read>,
    /// True when the override chain, not the default, is the effective
    /// definition.
    pub overridden: bool,
}

/// One participating declaration of a dependency cycle.
#[derive(Clone, Debug)]
pub struct CycleStep {
    pub name: String,
    /// Where the declaration was written.
    pub decl_span: SourceSpan,
    /// The reference to the next parameter on the path, as written.
    pub read_span: SourceSpan,
}

/// A closed dependency path: `a -> b -> a`.
#[derive(Clone, Debug)]
pub struct Cycle {
    /// The participating declarations, in path order and without repeating the
    /// first one at the end.
    pub steps: Vec<CycleStep>,
    /// The reference that closes the path: the last step reads the first name.
    /// This is the primary span of the `E_PARAM_CYCLE` diagnostic.
    pub close_span: SourceSpan,
}

impl Cycle {
    /// The closed path, e.g. `a -> b -> a`.
    pub fn render_path(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            out.push_str(&step.name);
            out.push_str(" -> ");
        }
        if let Some(first) = self.steps.first() {
            out.push_str(&first.name);
        }
        out
    }

    /// The declaration the path closes at — the one whose definition reads the
    /// first name again.
    pub fn closing_decl(&self) -> Option<&CycleStep> {
        self.steps.last()
    }

    /// The most useful span to point at: the closing reference, falling back to
    /// the closing declaration when the reference has no real location.
    pub fn primary_span(&self) -> SourceSpan {
        if self.close_span.is_synthetic() {
            self.closing_decl()
                .map(|step| step.decl_span)
                .unwrap_or_else(SourceSpan::synthetic)
        } else {
            self.close_span
        }
    }
}

/// The dependency graph of one body.
///
/// A [BodyGraph] never resolves a bare name across scopes: the caller gives it
/// one body's declarations and each declaration's own reads, and every edge it
/// builds stays inside that body.
#[derive(Clone, Debug)]
pub struct BodyGraph {
    scope: ScopePath,
    decls: Vec<Decl>,
    /// The declarations `i`'s effective definition reads, ascending.
    dependencies: Vec<Vec<usize>>,
    /// The reference span of each dependency, parallel to `dependencies`.
    dependency_spans: Vec<Vec<SourceSpan>>,
    /// The declarations whose definition reads `i`, ascending.
    dependents: Vec<Vec<usize>>,
    /// Reads that name no declaration of this body.
    unknown: Vec<Read>,
}

impl BodyGraph {
    pub fn new(scope: ScopePath, decls: Vec<Decl>) -> Self {
        let mut by_name: HashMap<&str, usize> = HashMap::new();
        for (i, decl) in decls.iter().enumerate() {
            // The caller rejects duplicate declarations, so the first one wins
            // here rather than silently retargeting an earlier edge.
            by_name.entry(decl.name.as_str()).or_insert(i);
        }

        let count = decls.len();
        let mut dependencies: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut dependency_spans: Vec<Vec<SourceSpan>> = vec![Vec::new(); count];
        let mut unknown: Vec<Read> = Vec::new();

        for (i, decl) in decls.iter().enumerate() {
            let mut edges: Vec<(usize, SourceSpan)> = Vec::new();
            for read in &decl.reads {
                match by_name.get(read.name.as_str()) {
                    // A self reference is kept: it is the one-node cycle.
                    Some(&j) => edges.push((j, read.span)),
                    None => {
                        let seen = unknown
                            .iter()
                            .any(|r| r.name == read.name && r.span == read.span);
                        if !seen {
                            unknown.push(read.clone());
                        }
                    }
                }
            }
            edges.sort_by_key(|(j, _)| *j);
            edges.dedup_by_key(|(j, _)| *j);
            dependencies[i] = edges.iter().map(|(j, _)| *j).collect();
            dependency_spans[i] = edges.iter().map(|(_, span)| *span).collect();
        }

        // `dependents[i]` is filled in declaration order, so it is ascending
        // without another sort.
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (i, deps) in dependencies.iter().enumerate() {
            for &j in deps {
                dependents[j].push(i);
            }
        }

        Self {
            scope,
            decls,
            dependencies,
            dependency_spans,
            dependents,
            unknown,
        }
    }

    /// The body this graph describes.
    pub fn scope(&self) -> &ScopePath {
        &self.scope
    }

    pub fn decls(&self) -> &[Decl] {
        &self.decls
    }

    pub fn len(&self) -> usize {
        self.decls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }

    /// The declarations `i`'s effective definition reads.
    pub fn dependencies(&self, i: usize) -> &[usize] {
        &self.dependencies[i]
    }

    /// Names read by an effective definition that this body declares nowhere.
    pub fn unknown_reads(&self) -> &[Read] {
        &self.unknown
    }

    /// A deterministic evaluation order: every parameter comes after the
    /// parameters its definition reads, and of the parameters that are ready at
    /// the same time the one declared first comes first.
    ///
    /// `Err` carries the cycle that makes such an order impossible.
    pub fn order(&self) -> Result<Vec<usize>, Cycle> {
        let mut remaining: Vec<usize> = self.dependencies.iter().map(Vec::len).collect();
        let mut ready: BTreeSet<usize> = remaining
            .iter()
            .enumerate()
            .filter(|(_, deps)| **deps == 0)
            .map(|(i, _)| i)
            .collect();
        let mut order = Vec::with_capacity(remaining.len());

        while let Some(&i) = ready.iter().next() {
            ready.remove(&i);
            order.push(i);
            for &dependent in &self.dependents[i] {
                remaining[dependent] -= 1;
                if remaining[dependent] == 0 {
                    ready.insert(dependent);
                }
            }
        }

        if order.len() == self.decls.len() {
            Ok(order)
        } else {
            Err(self.find_cycle(&remaining))
        }
    }

    /// The cycle the order would have to break, if there is one.
    pub fn cycle(&self) -> Option<Cycle> {
        self.order().err()
    }

    /// Follow dependencies from the first unresolved declaration until a node
    /// repeats; the path from that repetition on is the cycle.
    ///
    /// Deterministic: the start is the lowest unresolved declaration index and
    /// every step takes the lowest-indexed dependency.
    fn find_cycle(&self, remaining: &[usize]) -> Cycle {
        let start = remaining.iter().position(|deps| *deps > 0).unwrap_or(0);
        let mut path = vec![start];
        let mut position: HashMap<usize, usize> = HashMap::new();
        position.insert(start, 0);
        // `leave[i]` is the reference that goes from `path[i]` to `path[i + 1]`.
        let mut leave: Vec<SourceSpan> = Vec::new();
        let mut close_at = 0usize;
        let mut close_span = self.decls[start].span;

        loop {
            let current = *path.last().expect("the path starts non-empty");
            let Some(&next) = self.dependencies[current].first() else {
                // Unreachable for a leftover node (it has an unresolved
                // dependency, so its dependency list is not empty), but a graph
                // is not allowed to panic on a caller's mistake.
                break;
            };
            let span = self.dependency_spans[current][0];
            if let Some(&at) = position.get(&next) {
                close_at = at;
                close_span = span;
                break;
            }
            leave.push(span);
            position.insert(next, path.len());
            path.push(next);
        }

        let nodes = &path[close_at..];
        let mut steps = Vec::with_capacity(nodes.len());
        for (offset, &node) in nodes.iter().enumerate() {
            let read_span = if offset + 1 < nodes.len() {
                leave[close_at + offset]
            } else {
                close_span
            };
            steps.push(CycleStep {
                name: self.decls[node].name.clone(),
                decl_span: self.decls[node].span,
                read_span,
            });
        }
        Cycle { steps, close_span }
    }
}

// ---------------------------------------------------------------------------
// Topology use points and the whole design
// ---------------------------------------------------------------------------

/// The constructs that put a parameter in a topology position (contract §4.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UseKind {
    /// An `if` condition: it decides which statements exist.
    IfCondition,
    /// A `for` iteration source (list or range): it decides how many
    /// statements exist.
    ForIteration,
    /// A computed device, node, instance or terminal name: it decides what the
    /// statements connect.
    ComputedName,
}

impl UseKind {
    /// How the use point is named in a diagnostic.
    pub fn describe(self) -> &'static str {
        match self {
            UseKind::IfCondition => "`if` condition",
            UseKind::ForIteration => "`for` iteration source",
            UseKind::ComputedName => "computed name",
        }
    }
}

/// One place the elaborator read a parameter in a topology position.
#[derive(Clone, Debug)]
pub struct UseSite {
    /// The body the use site was written in.
    pub scope: ScopePath,
    pub kind: UseKind,
    /// What a computed name names, e.g. `device name`; empty otherwise.
    pub what: String,
    /// The span of the use point itself.
    pub span: SourceSpan,
    /// The names the use point's expression reads.
    pub reads: Vec<Read>,
}

impl UseSite {
    /// How the use point is named in a diagnostic.
    pub fn describe(&self) -> String {
        match self.kind {
            UseKind::ComputedName if !self.what.is_empty() => format!("computed {}", self.what),
            other => other.describe().to_string(),
        }
    }
}

/// One step of a topology explanation path.
#[derive(Clone, Debug)]
pub enum PathStep {
    /// A parameter on the path: the swept one first, then every parameter whose
    /// value moves with it.
    Param {
        name: String,
        decl_span: SourceSpan,
        /// The reference to the previous parameter on the path, as written.
        read_span: Option<SourceSpan>,
    },
    /// The topology use point the path ends at.
    Use {
        description: String,
        span: SourceSpan,
    },
}

impl PathStep {
    /// How the step is named in a rendered path.
    pub fn label(&self) -> &str {
        match self {
            PathStep::Param { name, .. } => name,
            PathStep::Use { description, .. } => description,
        }
    }

    /// Where the step was written.
    pub fn span(&self) -> SourceSpan {
        match self {
            PathStep::Param { decl_span, .. } => *decl_span,
            PathStep::Use { span, .. } => *span,
        }
    }
}

/// Render an explanation path, e.g. `n -> width -> for iteration source`.
pub fn render_path(steps: &[PathStep]) -> String {
    steps
        .iter()
        .map(PathStep::label)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// One parameter node of one design.
struct Node {
    name: String,
    decl_span: SourceSpan,
    /// The declarations this node's effective definition reads.
    deps: Vec<usize>,
    /// The reference span of each dependency, parallel to `deps`.
    dep_spans: Vec<SourceSpan>,
}

/// Every parameter of one elaboration, keyed by body instance, plus the
/// topology use sites that were seen while elaborating it.
///
/// One design is built per top-level circuit elaboration and cleared when the
/// next one starts.
#[derive(Default)]
pub struct DesignGraph {
    nodes: Vec<Node>,
    /// Scope path -> parameter name -> node.
    index: HashMap<ScopePath, HashMap<String, usize>>,
    /// The nodes whose definition reads `i`, ascending.
    dependents: Vec<Vec<usize>>,
    uses: Vec<UseSite>,
    /// For each node, the first use site that reads it.
    read_at_use: Vec<Option<usize>>,
}

impl DesignGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything: a new top-level circuit is a new design.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.index.clear();
        self.dependents.clear();
        self.uses.clear();
        self.read_at_use.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Register one body's declarations, resolving their reads inside that same
    /// body. Returns the node index of each declaration, in declaration order.
    pub fn add_body(&mut self, scope: &ScopePath, decls: &[Decl]) -> Vec<usize> {
        let mut by_name: HashMap<&str, usize> = HashMap::new();
        let mut ids = Vec::with_capacity(decls.len());
        for (i, decl) in decls.iter().enumerate() {
            let id = self.nodes.len();
            self.nodes.push(Node {
                name: decl.name.clone(),
                decl_span: decl.span,
                deps: Vec::new(),
                dep_spans: Vec::new(),
            });
            self.dependents.push(Vec::new());
            self.read_at_use.push(None);
            self.index
                .entry(scope.clone())
                .or_default()
                .insert(decl.name.clone(), id);
            by_name.entry(decl.name.as_str()).or_insert(i);
            ids.push(id);
        }

        let mut edges: Vec<Vec<(usize, SourceSpan)>> = vec![Vec::new(); decls.len()];
        for (i, decl) in decls.iter().enumerate() {
            for read in &decl.reads {
                if let Some(&j) = by_name.get(read.name.as_str()) {
                    edges[i].push((ids[j], read.span));
                }
            }
        }
        for (i, mut list) in edges.into_iter().enumerate() {
            list.sort_by_key(|(node, _)| *node);
            list.dedup_by_key(|(node, _)| *node);
            let id = ids[i];
            self.nodes[id].deps = list.iter().map(|(node, _)| *node).collect();
            self.nodes[id].dep_spans = list.iter().map(|(_, span)| *span).collect();
            for (dep, _) in list {
                self.dependents[dep].push(id);
            }
        }
        ids
    }

    /// Point an instance-local parameter at the names its `params:` binding
    /// reads, which is the one edge that crosses scopes.
    ///
    /// The reads were written in the instance statement, so they are resolved in
    /// the parent scope first and only then in the instance's own body — the
    /// same order the evaluator uses for a binding (the parent's value wins when
    /// both scopes declare the name).
    pub fn bind_from_parent(
        &mut self,
        scope: &ScopePath,
        name: &str,
        parent: &ScopePath,
        reads: &[Read],
    ) {
        let Some(&node) = self.index.get(scope).and_then(|by_name| by_name.get(name)) else {
            return;
        };

        // The default's edge (if any) is replaced, never merged: the binding is
        // the effective definition (contract §4.3).
        let previous: Vec<usize> = std::mem::take(&mut self.nodes[node].deps);
        self.nodes[node].dep_spans.clear();
        for dep in previous {
            if let Some(list) = self.dependents.get_mut(dep) {
                list.retain(|&dependent| dependent != node);
            }
        }

        let mut edges: Vec<(usize, SourceSpan)> = Vec::new();
        for read in reads {
            let target = self
                .index
                .get(parent)
                .and_then(|by_name| by_name.get(read.name.as_str()))
                .or_else(|| {
                    self.index
                        .get(scope)
                        .and_then(|by_name| by_name.get(read.name.as_str()))
                });
            // A binding that names its own parameter is not a dependency: the
            // value could not have been computed from itself.
            if let Some(&target) = target
                && target != node
            {
                edges.push((target, read.span));
            }
        }
        edges.sort_by_key(|(target, _)| *target);
        edges.dedup_by_key(|(target, _)| *target);

        self.nodes[node].deps = edges.iter().map(|(target, _)| *target).collect();
        self.nodes[node].dep_spans = edges.iter().map(|(_, span)| *span).collect();
        for (dep, _) in edges {
            self.dependents[dep].push(node);
        }
    }

    /// Record a topology use point, so a sweep that reaches it can be refused.
    pub fn add_use(&mut self, site: UseSite) {
        let use_index = self.uses.len();
        for read in &site.reads {
            let node = self
                .index
                .get(&site.scope)
                .and_then(|by_name| by_name.get(read.name.as_str()))
                .copied();
            if let Some(node) = node
                && self.read_at_use[node].is_none()
            {
                self.read_at_use[node] = Some(use_index);
            }
        }
        self.uses.push(site);
    }

    /// How many use points have been registered.
    pub fn use_count(&self) -> usize {
        self.uses.len()
    }

    /// Why sweeping `name` in `scope` can move the topology, if it can.
    ///
    /// The walk goes the other way round from evaluation: `name` is the
    /// parameter that would change, so the path visits every parameter whose
    /// value follows from it until one of them is read at a use point (contract
    /// §4.5). `None` means the parameter reaches no use point and a sweep of
    /// it changes only numbers.
    pub fn topology_path(&self, scope: &ScopePath, name: &str) -> Option<Vec<PathStep>> {
        let start = *self.index.get(scope)?.get(name)?;
        let mut parent: Vec<Option<(usize, SourceSpan)>> = vec![None; self.nodes.len()];
        let mut visited = vec![false; self.nodes.len()];
        let mut queue: VecDeque<usize> = VecDeque::new();
        visited[start] = true;
        queue.push_back(start);

        while let Some(node) = queue.pop_front() {
            if let Some(use_index) = self.read_at_use[node] {
                return Some(self.explain(&parent, start, node, use_index));
            }
            for &dependent in &self.dependents[node] {
                if !visited[dependent] {
                    visited[dependent] = true;
                    parent[dependent] = Some((node, self.dep_span(dependent, node)));
                    queue.push_back(dependent);
                }
            }
        }
        None
    }

    /// The span of `dependent`'s reference to `dependency`.
    fn dep_span(&self, dependent: usize, dependency: usize) -> SourceSpan {
        let node = &self.nodes[dependent];
        node.deps
            .iter()
            .position(|&dep| dep == dependency)
            .map(|at| node.dep_spans[at])
            .unwrap_or_else(SourceSpan::synthetic)
    }

    /// Build the parameter chain from the swept parameter `origin` to the use
    /// point, following the parents the walk recorded.
    fn explain(
        &self,
        parent: &[Option<(usize, SourceSpan)>],
        origin: usize,
        found: usize,
        use_index: usize,
    ) -> Vec<PathStep> {
        let mut chain: Vec<(usize, SourceSpan)> = Vec::new();
        let mut current = found;
        while let Some((previous, span)) = parent[current] {
            chain.push((current, span));
            current = previous;
        }
        chain.reverse();

        let first = &self.nodes[origin];
        let mut steps = vec![PathStep::Param {
            name: first.name.clone(),
            decl_span: first.decl_span,
            read_span: None,
        }];
        for (node, span) in chain {
            steps.push(PathStep::Param {
                name: self.nodes[node].name.clone(),
                decl_span: self.nodes[node].decl_span,
                read_span: Some(span),
            });
        }
        let site = &self.uses[use_index];
        steps.push(PathStep::Use {
            description: site.describe(),
            span: site.span,
        });
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuit_core::span::SourceId;

    /// A one-byte span, so every assertion can name the source position it
    /// means without building a source map.
    fn span(at: u32) -> SourceSpan {
        SourceSpan::new(SourceId(0), at, at + 1)
    }

    fn read(name: &str, at: u32) -> Read {
        Read {
            name: name.to_string(),
            span: span(at),
        }
    }

    fn decl(name: &str, at: u32, reads: Vec<Read>) -> Decl {
        Decl {
            name: name.to_string(),
            span: span(at),
            reads,
            overridden: false,
        }
    }

    fn names<'g>(graph: &'g BodyGraph, order: &[usize]) -> Vec<&'g str> {
        order
            .iter()
            .map(|&i| graph.decls()[i].name.as_str())
            .collect()
    }

    // ---- order -----------------------------------------------------------

    #[test]
    fn a_forward_reference_is_evaluated_after_the_declaration_it_reads() {
        // `b` reads `a`, and `a` is declared second.
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![decl("b", 0, vec![read("a", 10)]), decl("a", 20, Vec::new())],
        );
        let order = graph.order().expect("no cycle");
        assert_eq!(names(&graph, &order), vec!["a", "b"]);
    }

    #[test]
    fn source_order_breaks_every_tie() {
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![
                decl("x", 0, Vec::new()),
                decl("y", 10, Vec::new()),
                decl("z", 20, Vec::new()),
            ],
        );
        let order = graph.order().expect("no cycle");
        assert_eq!(names(&graph, &order), vec!["x", "y", "z"]);
    }

    #[test]
    fn a_diamond_is_evaluated_once_per_node() {
        // top = left + right, right = 3 * base, left = 2 * base, declared in
        // that source order.
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![
                decl("top", 0, vec![read("left", 5), read("right", 6)]),
                decl("right", 10, vec![read("base", 15)]),
                decl("left", 20, vec![read("base", 25)]),
                decl("base", 30, Vec::new()),
            ],
        );
        let order = graph.order().expect("no cycle");
        // `base` has no dependency, so it goes first; of the two branches that
        // then become ready the tie is broken by declaration index, not by name,
        // so `right` (index 1) precedes `left` (index 2).
        assert_eq!(names(&graph, &order), vec!["base", "right", "left", "top"]);
    }

    #[test]
    fn an_unknown_name_is_not_an_edge_and_not_a_cycle() {
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![decl("a", 0, vec![read("nope", 10)])],
        );
        assert_eq!(graph.order().expect("no cycle"), vec![0]);
        assert!(graph.cycle().is_none());
        assert_eq!(graph.unknown_reads().len(), 1);
        assert_eq!(graph.unknown_reads()[0].name, "nope");
    }

    #[test]
    fn a_name_only_a_replaced_default_reads_is_not_unknown() {
        // The caller passes no reads for an overridden declaration (§4.3), so
        // the graph sees no reference at all.
        let overridden = Decl {
            name: "r".to_string(),
            span: span(0),
            reads: Vec::new(),
            overridden: true,
        };
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![overridden, decl("b", 10, vec![read("r", 20)])],
        );
        assert!(graph.unknown_reads().is_empty());
        assert_eq!(graph.order().expect("no cycle").len(), 2);
    }

    // ---- cycles ----------------------------------------------------------

    #[test]
    fn a_self_reference_is_the_one_node_cycle() {
        let graph = BodyGraph::new(ScopePath::root(), vec![decl("a", 0, vec![read("a", 10)])]);
        let cycle = graph.cycle().expect("a self reference is a cycle");
        assert_eq!(cycle.render_path(), "a -> a");
        assert_eq!(cycle.steps.len(), 1);
        assert_eq!(cycle.primary_span(), span(10));
        assert_eq!(cycle.steps[0].decl_span, span(0));
    }

    #[test]
    fn a_two_node_cycle_names_both_declarations_and_the_closing_read() {
        // a reads b at offset 10, b reads a at offset 30.
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![
                decl("a", 0, vec![read("b", 10)]),
                decl("b", 20, vec![read("a", 30)]),
            ],
        );
        let cycle = graph.cycle().expect("a -> b -> a");
        assert_eq!(cycle.render_path(), "a -> b -> a");
        assert_eq!(cycle.steps[0].decl_span, span(0));
        assert_eq!(cycle.steps[0].read_span, span(10));
        assert_eq!(cycle.steps[1].decl_span, span(20));
        assert_eq!(cycle.steps[1].read_span, span(30));
        // The path closes at the read inside `b`.
        assert_eq!(cycle.primary_span(), span(30));
    }

    #[test]
    fn a_three_node_cycle_is_deterministic() {
        // a reads c, b reads a, c reads b: the path follows the lowest-indexed
        // dependency each time.
        let build = || {
            BodyGraph::new(
                ScopePath::root(),
                vec![
                    decl("a", 0, vec![read("c", 10)]),
                    decl("b", 20, vec![read("a", 30)]),
                    decl("c", 40, vec![read("b", 50)]),
                ],
            )
        };
        let cycle = build().cycle().expect("cycle");
        assert_eq!(cycle.render_path(), "a -> c -> b -> a");
        assert_eq!(build().cycle().unwrap().render_path(), cycle.render_path());
    }

    #[test]
    fn a_cycle_only_depends_on_declared_names() {
        let graph = BodyGraph::new(
            ScopePath::root(),
            vec![
                decl("a", 0, vec![read("nope", 5), read("b", 10)]),
                decl("b", 20, vec![read("a", 30)]),
            ],
        );
        assert_eq!(graph.cycle().unwrap().render_path(), "a -> b -> a");
    }

    // ---- topology --------------------------------------------------------

    fn use_site(scope: &ScopePath, kind: UseKind, at: u32, reads: Vec<Read>) -> UseSite {
        UseSite {
            scope: scope.clone(),
            kind,
            what: String::new(),
            span: span(at),
            reads,
        }
    }

    #[test]
    fn a_parameter_no_use_site_reads_is_not_topology_affecting() {
        let mut design = DesignGraph::new();
        design.add_body(&ScopePath::root(), &[decl("rf", 0, Vec::new())]);
        design.add_use(use_site(
            &ScopePath::root(),
            UseKind::IfCondition,
            10,
            vec![read("other", 12)],
        ));
        assert!(design.topology_path(&ScopePath::root(), "rf").is_none());
    }

    #[test]
    fn a_use_site_reading_the_parameter_marks_it_directly() {
        let mut design = DesignGraph::new();
        design.add_body(&ScopePath::root(), &[decl("n", 3, Vec::new())]);
        design.add_use(use_site(
            &ScopePath::root(),
            UseKind::ForIteration,
            40,
            vec![read("n", 45)],
        ));
        let path = design
            .topology_path(&ScopePath::root(), "n")
            .expect("the loop range reads n");
        assert_eq!(render_path(&path), "n -> `for` iteration source");
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn the_path_lists_the_intermediate_parameters() {
        // n -> width, and the loop range reads width.
        let mut design = DesignGraph::new();
        design.add_body(
            &ScopePath::root(),
            &[
                decl("n", 3, Vec::new()),
                decl("width", 20, vec![read("n", 30)]),
            ],
        );
        design.add_use(use_site(
            &ScopePath::root(),
            UseKind::ForIteration,
            40,
            vec![read("width", 45)],
        ));
        let path = design
            .topology_path(&ScopePath::root(), "n")
            .expect("n reaches the loop through width");
        assert_eq!(render_path(&path), "n -> width -> `for` iteration source");
        // The second step says where `width` reads `n`.
        match &path[1] {
            PathStep::Param { read_span, .. } => assert_eq!(*read_span, Some(span(30))),
            other => panic!("expected a parameter step, found {other:?}"),
        }
    }

    #[test]
    fn same_named_parameters_in_two_scopes_are_different_nodes() {
        // The instance body has its own `n` driving its own loop; the top-level
        // `n` is a plain value. Sweeping the top-level one is legal.
        let instance = ScopePath::root().child("stage");
        let mut design = DesignGraph::new();
        design.add_body(&ScopePath::root(), &[decl("n", 0, Vec::new())]);
        design.add_body(&instance, &[decl("n", 10, Vec::new())]);
        design.add_use(use_site(
            &instance,
            UseKind::ForIteration,
            20,
            vec![read("n", 25)],
        ));
        assert!(design.topology_path(&ScopePath::root(), "n").is_none());
        assert!(design.topology_path(&instance, "n").is_some());
    }

    #[test]
    fn an_instance_params_binding_wires_the_parent_parameter_into_the_body() {
        // params: { x: p } and a loop range inside the instance reading x.
        let instance = ScopePath::root().child("stage");
        let mut design = DesignGraph::new();
        design.add_body(&ScopePath::root(), &[decl("p", 0, Vec::new())]);
        design.add_body(&instance, &[decl("x", 10, Vec::new())]);
        design.add_use(use_site(
            &instance,
            UseKind::ForIteration,
            20,
            vec![read("x", 25)],
        ));
        design.bind_from_parent(&instance, "x", &ScopePath::root(), &[read("p", 15)]);

        let path = design
            .topology_path(&ScopePath::root(), "p")
            .expect("the binding carries p into the instance loop");
        assert_eq!(render_path(&path), "p -> x -> `for` iteration source");
    }

    #[test]
    fn a_binding_that_reaches_no_use_site_keeps_the_parameter_sweepable() {
        // The same binding without a topology use inside the instance: the value
        // only ever lands in a numeric position.
        let instance = ScopePath::root().child("stage");
        let mut design = DesignGraph::new();
        design.add_body(&ScopePath::root(), &[decl("p", 0, Vec::new())]);
        design.add_body(&instance, &[decl("x", 10, Vec::new())]);
        design.bind_from_parent(&instance, "x", &ScopePath::root(), &[read("p", 15)]);
        assert!(design.topology_path(&ScopePath::root(), "p").is_none());
    }

    #[test]
    fn a_binding_replaces_the_default_edge_instead_of_merging_it() {
        let instance = ScopePath::root().child("stage");
        let mut design = DesignGraph::new();
        design.add_body(
            &ScopePath::root(),
            &[decl("p", 0, Vec::new()), decl("q", 5, Vec::new())],
        );
        // The instance's `x` defaults to reading `q`; the binding replaces that
        // with `p`, so sweeping `q` can no longer move `x`.
        design.add_body(&instance, &[decl("x", 10, vec![read("q", 15)])]);
        design.add_use(use_site(
            &instance,
            UseKind::ForIteration,
            20,
            vec![read("x", 25)],
        ));
        design.bind_from_parent(&instance, "x", &ScopePath::root(), &[read("p", 30)]);
        assert!(design.topology_path(&ScopePath::root(), "q").is_none());
        assert!(design.topology_path(&ScopePath::root(), "p").is_some());
    }

    #[test]
    fn clearing_a_design_forgets_the_previous_circuit() {
        let mut design = DesignGraph::new();
        design.add_body(&ScopePath::root(), &[decl("n", 0, Vec::new())]);
        design.add_use(use_site(
            &ScopePath::root(),
            UseKind::IfCondition,
            10,
            vec![read("n", 12)],
        ));
        assert!(design.topology_path(&ScopePath::root(), "n").is_some());
        design.clear();
        assert!(design.is_empty());
        assert!(design.topology_path(&ScopePath::root(), "n").is_none());
    }
}
