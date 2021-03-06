//! Provides a struct [`Dcg`], which can be used to create and compose Dynamic
//! Computation Graphs (DCGs).

use std::{cell::RefCell, marker::PhantomData, ops::Deref};

use petgraph::Direction;
use petgraph::{
    graph::{DiGraph, NodeIndex},
    visit::{depth_first_search, DfsEvent},
};

/// Internal graph node type. Stores the type and data of a [`Dcg`] graph node.
pub enum Node<'a, T>
where
    T: Clone,
{
    /// Contains a value of type `T`.
    ///
    /// This value may be retrieved or replaced by calling [`Dcg::get`] or
    /// [`Dcg::set`] on the corresponding [`DcgNode<Cell>`].
    Cell(T),

    /// Contains a thunk which produces a value of type `T`.
    ///
    /// The result of the thunk may be retrieved by calling [`Dcg::get`] on
    /// the corresponding [`DcgNode<Thunk>`].
    Thunk(&'a dyn Fn() -> T),

    /// Contains a thunk which produces a value of type `T` and a cached value
    /// of type `T` which holds the result of the previous evaluation of the
    /// thunk.
    ///
    /// When [`Dcg::get`] is called on the corresponding [`DcgNode<Memo>`] the
    /// value returned depends on the cleanliness of the node's dependencies.
    /// - If any dependency is dirty, the [`DcgNode<Memo>`] is dirty and thunk
    /// is re-evaluated, cached and returned.
    /// - Otherwise, the cached value is returned.
    Memo(&'a dyn Fn() -> T, Option<T>),
}

/// [`DcgNode`] marker denoting a [`Dcg::cell`].
pub struct Cell {}

/// [`DcgNode`] marker denoting a [`Dcg::thunk`] or [`Dcg::lone_thunk`].
pub struct Thunk {}

/// [`DcgNode`] marker denoting a [`Dcg::memo`] or [`Dcg::lone_memo`].
pub struct Memo {}

/// Shallow wrapper around a [`NodeIndex`]. Contains information about the indexed node's type.
///
/// The `Ty` marker provides compile-time information about a [`DcgNode`]'s type.
/// Valid types are:
///
/// - [`Cell`]
/// - [`Thunk`]
/// - [`Memo`]
///
/// Conversion to the underlying [`NodeIndex`] is provided via a
/// [`From<DcgNode>`] implementation.
pub struct DcgNode<Ty>(NodeIndex, PhantomData<Ty>);

/// Workaround for a [bug](https://github.com/rust-lang/rust/issues/26925) in
/// [`PhantomData`]
impl<Ty> Clone for DcgNode<Ty> {
    fn clone(&self) -> Self {
        Self(self.0, PhantomData)
    }
}

/// Workaround for a [bug](https://github.com/rust-lang/rust/issues/26925) in
/// [`PhantomData`]
impl<Ty> Copy for DcgNode<Ty> {}

impl<Ty> From<DcgNode<Ty>> for NodeIndex {
    fn from(node: DcgNode<Ty>) -> Self {
        node.0
    }
}

impl<Ty> From<NodeIndex> for DcgNode<Ty> {
    fn from(idx: NodeIndex) -> Self {
        Self(idx, PhantomData)
    }
}

impl<T> std::fmt::Debug for Node<'_, T>
where
    T: Clone + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Node::Cell(value) => write!(f, "{:?}", value),
            Node::Thunk(_) => f.debug_tuple("Thunk").finish(),
            Node::Memo(_, last_value) => f.debug_tuple("Memo").field(&last_value).finish(),
        }
    }
}

type GraphRepr<'a, T> = RefCell<DiGraph<Node<'a, T>, bool>>;

/// The central data structure responsible for building, editing and
/// querying DCGs.
///
/// [`Dcg`]s are generic over their stored type `T` and can only contain nodes
/// that "generate" values of type `T`. This is enforced at compile time, so is
/// of little concern to general usage.
///
/// This struct is a thin wrapper around a [`DiGraph`] with nodes of type
/// [`Node`] and edges of type [`bool`].
///
/// # Concepts
///
/// ## Nodes
///
/// Fundamentally, a [`Dcg`] handles the instantiation, alteration and querying
/// of [`DcgNode`]s.
///
/// [`DcgNode`]s are one of three types:
///
/// - [`Cell`] - A primitive type used to wrap a value of type `T`.
/// - [`Thunk`] - A generator of `T`s, essentially a closure of type `Fn() -> T`.
/// - [`Memo`] - A [`Thunk`] that caches its results.
///
/// The dependencies of [`Thunk`] and [`Memo`] nodes **must be provided
/// fully and explicitly** upon instantiation. A [`Dcg`] stores this dependency data.
///
/// ## "Dirtiness"
///
/// It may be helpful to view dirtiness through the lens of cache validity.
///
/// A node in the DCG is "dirty" (or invalid) if the next value it
/// generates may differ from the last time it was queried. [`DcgNode`]s are
/// consequently dirty if their underlying value is changed, or if any of their
/// dependencies are dirty; dirtiness is transitive.
///
/// Edges may be considered dirty if they originate from a dirty node.
///
/// We can now examine the consequence of dirtiness on nodes:
///
/// - [`Cell`] is independent, i.e. have no incoming dependency edges, so can
/// only be dirtied by a change to their inner value.
/// - [`Thunk`] always eagerly executes its closure, so while it *can* be
/// dirty, this does not affect its behaviour.
/// - [`Memo`] is most affected by dirtiness. If dirty, its cache may be
/// invalid. When queried, to safely return an up-to-date value, it must
/// generate and cache a new value. Otherwise, its cached value may be used.
///
/// ## Generation
///
/// Now that dirtiness has been covered, the method of generating values is
/// clear:
///
/// - [`Cell`] yields a copy of its inner `T`.
/// - [`Thunk`] yields a copy of the result of executing its thunk.
/// - [`Memo`] yields a copy of its cache if clean; otherwise it behaves like
/// a thunk, also caching the yielded value.
///
/// # Example Usage
///
/// A [`Dcg`] is instantiated without referring to its generic type `T`.
/// Instead, `T` is typically inferred by the compiler via further usage and
/// hence cannot be instantiated without use:
/// ```compile_fail
/// use dcg::Dcg;
///
/// let dcg = Dcg::new();
///
/// // This snippet compiles successfully if the line below is uncommented
/// // dcg.cell(1);
/// ```
/// [`Cell`]s can be created with [`Dcg::cell`].
/// ```
/// # use dcg::Dcg;
/// # let dcg = Dcg::new();
/// let a = dcg.cell(1);
/// let b = dcg.cell(2);
/// ```
/// [`Thunk`]s and [`Memo`]s can also be created by passing a reference to a
/// closure. This closure may or may not depend on other nodes via calls to
/// [`Dcg::get`].
///
/// A closure's dependencies must also be specified in an accompanying
/// [`DcgNode`] slice.
/// ```
/// # use dcg::Dcg;
/// # let dcg = Dcg::new();
/// # let a = dcg.cell(1);
/// # let b = dcg.cell(2);
/// let add_ab = || dcg.get(a) + dcg.get(b);
/// let thunk1 = dcg.thunk(&add_ab, &[a, b]);
///
/// let add_one = || dcg.get(thunk1) + 1;
/// let thunk2 = dcg.thunk(&add_one, &[thunk1]);
///
/// let add_two = || dcg.get(thunk1) + 2;
/// let thunk3 = dcg.thunk(&add_two, &[thunk1]);
///
/// let two_times_thunk_plus_three = || dcg.get(thunk2) + dcg.get(thunk3);
/// let _ = dcg.thunk(&two_times_thunk_plus_three, &[thunk2, thunk3]);
///
/// dcg.set(a, 2);
/// ```
pub struct Dcg<'a, T>(pub GraphRepr<'a, T>)
where
    T: Clone;

impl<'a, T> Deref for Dcg<'a, T>
where
    T: Clone,
{
    type Target = GraphRepr<'a, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T> Dcg<'a, T>
where
    T: Clone,
{
    /// Creates an empty DCG.
    /// # Examples
    /// ```
    /// use dcg::Dcg;
    ///
    /// let dcg: Dcg<i64> = Dcg::new();
    ///
    /// assert_eq!(dcg.borrow().node_count(), 0);
    /// ```
    pub fn new() -> Self {
        Self(RefCell::new(DiGraph::new()))
    }

    fn is_dirty(&self, node: NodeIndex) -> bool {
        self.borrow()
            .edges_directed(node, Direction::Incoming)
            .any(|edge| *edge.weight())
    }

    fn add_dependencies(&self, node: NodeIndex, dependencies: &[NodeIndex]) {
        let dep_states: Vec<_>;
        {
            dep_states = dependencies
                .iter()
                .map(|&dep| (dep, self.is_dirty(dep)))
                .collect();
        }
        let mut dcg = self.borrow_mut();
        dep_states.iter().for_each(|(dep, dirty)| {
            dcg.add_edge(*dep, node, *dirty);
        });
    }

    /// Creates and adds a [`Node::Cell`] to the dependency graph, returning
    /// a corresponding [`DcgNode<Cell>`].
    /// # Examples
    /// ```
    /// use dcg::Dcg;
    ///
    /// let dcg = Dcg::new();
    ///
    /// let cell = dcg.cell(1);
    ///
    /// assert_eq!(dcg.borrow().node_count(), 1);
    /// assert_eq!(dcg.get(cell), 1);
    /// ```
    pub fn cell(&self, value: T) -> DcgNode<Cell> {
        DcgNode(self.borrow_mut().add_node(Node::Cell(value)), PhantomData)
    }

    /// Creates and adds a [`Node::Thunk`] and its [`DcgNode<Ty>`] dependencies
    /// to the dependency graph, returning a corresponding [`DcgNode<Thunk>`].
    /// # Examples
    /// ```
    /// use dcg::Dcg;
    ///
    /// let dcg = Dcg::new();
    ///
    /// let cell = dcg.cell(1);
    ///
    /// let get_cell = || dcg.get(cell);
    /// let thunk = dcg.thunk(&get_cell, &[cell]);
    ///
    /// let borrowed = dcg.borrow();
    ///
    /// assert_eq!(borrowed.node_count(), 2);
    ///
    /// assert!(borrowed.contains_edge(cell.into(), thunk.into()));
    ///
    /// assert_eq!(dcg.get(thunk), dcg.get(cell));
    /// ```
    pub fn thunk<F, Ty>(&self, thunk: &'a F, dependencies: &[DcgNode<Ty>]) -> DcgNode<Thunk>
    where
        F: Fn() -> T,
    {
        let node = self.borrow_mut().add_node(Node::Thunk(thunk));
        self.add_dependencies(
            node,
            dependencies
                .iter()
                .map(|node| (*node).into())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        DcgNode(node, PhantomData)
    }

    /// Creates and adds a memo'd thunk and its dependencies to the dependency graph.
    /// # Examples
    /// ```
    /// use dcg::{Dcg, Node};
    ///
    /// let dcg = Dcg::new();
    ///
    /// let cell = dcg.cell(1);
    ///
    /// let get_cell = || dcg.get(cell);
    /// let memo = dcg.memo(&get_cell, &[cell]);
    ///
    /// let borrowed = dcg.borrow();
    ///
    /// assert_eq!(borrowed.node_count(), 2);
    ///
    /// assert!(borrowed.contains_edge(cell.into(), memo.into()));
    ///
    /// assert_eq!(dcg.get(memo), dcg.get(cell));
    ///
    /// match dcg.borrow().node_weight(memo.into()).unwrap() {
    ///     Node::Memo(_, Some(value)) => assert_eq!(*value, 1),
    ///     _ => (),
    /// };
    /// ```
    pub fn memo<F, Ty>(&self, thunk: &'a F, dependencies: &[DcgNode<Ty>]) -> DcgNode<Memo>
    where
        F: Fn() -> T,
    {
        let node = self.borrow_mut().add_node(Node::Memo(thunk, None));
        self.add_dependencies(
            node,
            dependencies
                .iter()
                .map(|node| (*node).into())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        DcgNode(node, PhantomData)
    }

    /// Creates and adds a thunk with no dependencies to the dependency graph.
    ///
    /// To be used where the thunk is not dependent on any DCG nodes. If this is not the case, instead use [`Dcg::thunk`].
    /// # Examples
    /// ```
    /// use dcg::Dcg;
    ///
    /// let dcg = Dcg::new();
    ///
    /// let meaning_of_life = || 42;
    /// let thunk = dcg.lone_thunk(&meaning_of_life);
    ///
    /// assert_eq!(dcg.borrow().node_count(), 1);
    ///
    /// assert_eq!(dcg.get(thunk), 42);
    /// ```
    pub fn lone_thunk<F>(&self, thunk: &'a F) -> DcgNode<Thunk>
    where
        F: Fn() -> T,
    {
        DcgNode(self.borrow_mut().add_node(Node::Thunk(thunk)), PhantomData)
    }

    /// Creates and adds a memo'd thunk with no dependencies to the dependency graph.
    ///
    /// To be used where the thunk is not dependent on any DCG nodes. If this is not the case, instead use [`Dcg::memo`].
    /// # Examples
    /// ```
    /// use dcg::{Dcg, Node};
    ///
    /// let dcg = Dcg::new();
    ///
    /// let meaning_of_life = || 42;
    /// let memo = dcg.lone_memo(&meaning_of_life);
    ///
    /// assert_eq!(dcg.borrow().node_count(), 1);
    ///
    /// assert_eq!(dcg.get(memo), 42);
    ///
    /// match dcg.borrow().node_weight(memo.into()).unwrap() {
    ///     Node::Memo(_, Some(value)) => assert_eq!(*value, 42),
    ///     _ => (),
    /// };
    /// ```
    pub fn lone_memo<F>(&self, thunk: &'a F) -> DcgNode<Memo>
    where
        F: Fn() -> T,
    {
        DcgNode(
            self.borrow_mut().add_node(Node::Memo(thunk, Some(thunk()))),
            PhantomData,
        )
    }

    pub fn get<Ty>(&self, node: DcgNode<Ty>) -> T {
        // TODO: The tricky bit...
        match self.borrow().node_weight(node.into()).unwrap() {
            Node::Cell(value) => value.clone(),
            Node::Thunk(thunk) => thunk().clone(),
            Node::Memo(thunk, value) => match value {
                Some(value) => value.clone(),
                None => thunk().clone(),
            },
        }
    }

    /// Sets the value of `node` to `new_value`, "dirtying" all dependent
    /// nodes.
    ///
    /// Dirties all nodes that are transitively dependent on `node` and
    /// returns the previous cell value.
    ///
    /// This function only accepts nodes generates by [`Dcg::cell`]:
    /// ```compile_fail
    /// use dcg::Dcg;
    ///
    /// let dcg = Dcg::new();
    ///
    /// let x = || 42;
    /// let thunk = dcg.lone_thunk(&x);
    ///
    /// dcg.set(thunk, &x);
    /// ```
    ///
    /// The dirtying phase performs a Depth-First-Search from `node` and sets
    /// the weight of each tree/cross edge encountered to [`true`]
    /// # Examples
    /// ```
    /// use dcg::Dcg;
    ///
    /// let dcg = Dcg::new();
    ///
    /// let cell = dcg.cell(1);
    ///
    /// let get_cell = || dcg.get(cell);
    /// let thunk1 = dcg.thunk(&get_cell, &[cell]);
    /// let thunk2 = dcg.thunk(&get_cell, &[cell]);
    /// let get_thunk1 = || dcg.get(thunk1);
    /// let thunk3 = dcg.thunk(&get_thunk1, &[thunk1]);
    ///
    /// /* BEFORE: no dirty edges
    /// *
    /// *     thunk1 -- thunk3
    /// *    /
    /// *   a
    /// *    \
    /// *     thunk2
    /// */
    ///
    /// assert_eq!(dcg.set(cell, 42), 1);
    ///
    /// /* AFTER: all edges dirtied
    /// *
    /// *      thunk1 == thunk3
    /// *    //
    /// *   a
    /// *    \\
    /// *      thunk2
    /// */
    ///
    /// assert_eq!(dcg.get(cell), 42);
    ///
    /// assert!(dcg.borrow_mut().edge_weights_mut().all(|weight| *weight));
    /// ```
    pub fn set(&self, node: DcgNode<Cell>, new_value: T) -> T {
        let idx = node.into();
        let value = match self.borrow_mut().node_weight_mut(idx).unwrap() {
            Node::Cell(ref mut value) => {
                let tmp = value.clone();
                *value = new_value;
                tmp
            }
            _ => unreachable!(),
        };

        let mut transitive_edges = Vec::new();
        {
            let dcg = self.borrow();
            depth_first_search(&*dcg, Some(idx), |event| {
                let uv = match event {
                    DfsEvent::TreeEdge(u, v) => Some((u, v)),
                    DfsEvent::CrossForwardEdge(u, v) => Some((u, v)),
                    _ => None,
                };
                match uv {
                    Some((u, v)) => transitive_edges.push(dcg.find_edge(u, v).unwrap()),
                    None => (),
                }
            });
        }

        let mut dcg = self.borrow_mut();
        transitive_edges.iter().for_each(|&edge| {
            *dcg.edge_weight_mut(edge).unwrap() = true;
        });
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_cell() {
        let dcg = Dcg::new();

        let a = dcg.cell(1);

        assert_eq!(dcg.borrow().node_count(), 1);

        assert_eq!(dcg.get(a), 1);
    }

    #[test]
    fn create_thunk() {
        let dcg = Dcg::new();
        let a = dcg.cell(1);

        let get_a = || dcg.get(a);
        let thunk = dcg.thunk(&get_a, &[a]);

        {
            let graph = dcg.borrow();

            assert_eq!(graph.node_count(), 2);

            assert!(graph.contains_edge(a.into(), thunk.into()));
        }

        assert!(dcg.borrow_mut().edge_weights_mut().all(|weight| !*weight));

        assert_eq!(dcg.get(thunk), 1);
    }

    #[test]
    fn create_memo() {
        let dcg = Dcg::new();
        let a = dcg.cell(1);

        let get_a = || dcg.get(a);
        let memo = dcg.memo(&get_a, &[a]);

        {
            let graph = dcg.borrow();

            assert_eq!(graph.node_count(), 2);

            assert!(graph.contains_edge(a.into(), memo.into()));
        }

        assert!(dcg.borrow_mut().edge_weights_mut().all(|weight| !*weight));

        assert_eq!(dcg.get(memo), 1);
    }

    #[test]
    fn create_lone_thunk() {
        let dcg = Dcg::new();
        let thunk = dcg.lone_thunk(&|| 42);

        assert_eq!(dcg.borrow().node_count(), 1);

        assert_eq!(dcg.get(thunk), 42);
    }

    #[test]
    fn create_lone_memo() {
        let dcg = Dcg::new();
        let memo = dcg.lone_memo(&|| 42);

        assert_eq!(dcg.borrow().node_count(), 1);

        assert_eq!(dcg.get(memo), 42);
    }

    #[test]
    fn thunk_nested() {
        let dcg = Dcg::new();

        let a = dcg.cell(1);

        let get_a = || dcg.get(a);

        let thunk1 = dcg.thunk(&get_a, &[a]);
        let thunk2 = dcg.thunk(&get_a, &[a]);

        let add = || dcg.get(thunk1) + dcg.get(thunk2);
        let thunk3 = dcg.thunk(&add, &[thunk1, thunk2]);

        {
            let graph = dcg.borrow();

            assert_eq!(graph.node_count(), 4);

            assert!(graph.contains_edge(a.into(), thunk1.into()));
            assert!(graph.contains_edge(a.into(), thunk2.into()));
            assert!(graph.contains_edge(thunk1.into(), thunk3.into()));
            assert!(graph.contains_edge(thunk2.into(), thunk3.into()));
        }

        assert!(dcg.borrow_mut().edge_weights_mut().all(|weight| !*weight));

        assert_eq!(dcg.get(thunk1), 1);
        assert_eq!(dcg.get(thunk2), 1);
        assert_eq!(dcg.get(thunk3), 2);
    }

    #[test]
    fn dirtying_phase() {
        let dcg = Dcg::new();

        let a = dcg.cell(1);

        let get_a = || dcg.get(a);
        let thunk = dcg.thunk(&get_a, &[a]);

        assert_eq!(dcg.get(thunk), 1);

        assert_eq!(dcg.set(a, 2), 1);

        let graph = dcg.borrow();

        assert_eq!(
            *graph
                .edge_weight(graph.find_edge(a.into(), thunk.into()).unwrap())
                .unwrap(),
            true
        );

        assert_eq!(dcg.get(thunk), 2);
    }
}