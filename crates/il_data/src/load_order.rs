//! Load-order resolution (T1-020, Modding SDK §3.2, REQ-MOD-004, 005).
//!
//! Hard edges come from `dependencies` and the implicit game-first rule;
//! soft edges from `load_after` and `load_before`. Cycles are found with
//! Tarjan's strongly connected components; soft edges inside a cycle are
//! dropped one at a time (declaring mod's id order, `load_after` before
//! `load_before`, then declaration order) until only hard cycles remain,
//! which are errors. The final order is Kahn's algorithm with the smallest
//! id first among ready mods.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use il_core::StateHasher;

use crate::manifest::{Manifest, ManifestWithPath};

/// A mod in resolved load order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedMod {
    pub manifest: Manifest,
    pub root: PathBuf,
    pub is_game: bool,
}

impl LoadedMod {
    /// Namespaces this mod may define new content in: its id, plus the
    /// declared `namespaces` for the flagship game.
    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.manifest.id.as_str()).chain(
            self.manifest
                .namespaces
                .iter()
                .filter(move |_| self.is_game)
                .map(String::as_str),
        )
    }

    pub fn content_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.content_root)
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.assets_root)
    }
}

/// The enabled mods in load order plus resolution warnings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModSet {
    pub mods: Vec<LoadedMod>,
    pub warnings: Vec<String>,
}

impl ModSet {
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.mods.iter().position(|m| m.manifest.id == id)
    }

    /// xxh3 over `(id, version)` in load order (SAD §5.3, Networking §4.2).
    pub fn mod_list_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u32(self.mods.len() as u32);
        for m in &self.mods {
            let id = m.manifest.id.as_bytes();
            h.write_u32(id.len() as u32);
            h.write_bytes(id);
            let v = m.manifest.version.to_string();
            h.write_u32(v.len() as u32);
            h.write_bytes(v.as_bytes());
        }
        h.finish().0
    }

    /// Enables every discovered mod.
    pub fn all(found: &[ManifestWithPath]) -> Result<ModSet, Vec<LoadOrderError>> {
        let enabled: Vec<String> = found.iter().map(|m| m.manifest.id.clone()).collect();
        resolve_load_order(found, &enabled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    Dependency,
    LoadAfter,
    LoadBefore,
    Game,
}

impl EdgeKind {
    fn is_soft(self) -> bool {
        matches!(self, EdgeKind::LoadAfter | EdgeKind::LoadBefore)
    }

    fn field(self) -> &'static str {
        match self {
            EdgeKind::Dependency => "dependencies",
            EdgeKind::LoadAfter => "load_after",
            EdgeKind::LoadBefore => "load_before",
            EdgeKind::Game => "game",
        }
    }
}

/// `from` loads before `to`, declared by `declared_by`'s manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub declared_by: String,
    pub decl_index: usize,
}

impl Edge {
    fn via(&self) -> String {
        format!("{}.{}", self.declared_by, self.kind.field())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadOrderError {
    #[error("modlist: duplicate mod id {id:?} at {} and {}", a.display(), b.display())]
    DuplicateModId { id: String, a: PathBuf, b: PathBuf },
    #[error("modlist: enabled mod {0:?} was not found")]
    UnknownEnabledId(String),
    #[error("{mod_id}: missing dependency {dep:?} ({req})")]
    MissingDependency {
        mod_id: String,
        dep: String,
        req: semver::VersionReq,
    },
    #[error("{mod_id}: dependency {dep:?} is {found}, requires {req}")]
    DependencyVersion {
        mod_id: String,
        dep: String,
        found: semver::Version,
        req: semver::VersionReq,
    },
    #[error("load order cycle: {} (via {})", path.join(" -> "), via.join(", "))]
    Cycle { path: Vec<String>, via: Vec<String> },
}

/// Resolves the load order of the `enabled` mods among `found`. The flagship
/// game is always enabled.
pub fn resolve_load_order(
    found: &[ManifestWithPath],
    enabled: &[String],
) -> Result<ModSet, Vec<LoadOrderError>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Enabled set, with duplicate detection.
    let mut by_id: BTreeMap<&str, &ManifestWithPath> = BTreeMap::new();
    for m in found {
        let id = m.manifest.id.as_str();
        let is_enabled = m.is_game || enabled.iter().any(|e| e == id);
        if !is_enabled {
            continue;
        }
        if let Some(first) = by_id.get(id) {
            errors.push(LoadOrderError::DuplicateModId {
                id: id.to_string(),
                a: first.root.clone(),
                b: m.root.clone(),
            });
        } else {
            by_id.insert(id, m);
        }
    }
    for e in enabled {
        if !by_id.contains_key(e.as_str()) {
            errors.push(LoadOrderError::UnknownEnabledId(e.clone()));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // 1. Validate dependency ranges.
    let engine = crate::manifest::engine_version();
    for (id, m) in &by_id {
        if !m.manifest.engine_version.matches(&engine) {
            warnings.push(format!(
                "{id}: targets engine {} but this engine is {engine}",
                m.manifest.engine_version
            ));
        }
        for d in &m.manifest.dependencies {
            match by_id.get(d.id.as_str()) {
                None => errors.push(LoadOrderError::MissingDependency {
                    mod_id: (*id).to_string(),
                    dep: d.id.clone(),
                    req: d.version.clone(),
                }),
                Some(dep) if !d.version.matches(&dep.manifest.version) => {
                    errors.push(LoadOrderError::DependencyVersion {
                        mod_id: (*id).to_string(),
                        dep: d.id.clone(),
                        found: dep.manifest.version.clone(),
                        req: d.version.clone(),
                    });
                }
                Some(_) => {}
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // 2. Build the graph.
    let mut edges: Vec<Edge> = Vec::new();
    for (id, m) in &by_id {
        let id = (*id).to_string();
        if m.is_game {
            for other in by_id.keys() {
                if *other != id {
                    edges.push(Edge {
                        from: id.clone(),
                        to: (*other).to_string(),
                        kind: EdgeKind::Game,
                        declared_by: id.clone(),
                        decl_index: 0,
                    });
                }
            }
        }
        for (i, d) in m.manifest.dependencies.iter().enumerate() {
            edges.push(Edge {
                from: d.id.clone(),
                to: id.clone(),
                kind: EdgeKind::Dependency,
                declared_by: id.clone(),
                decl_index: i,
            });
        }
        for (i, a) in m.manifest.load_after.iter().enumerate() {
            if by_id.contains_key(a.as_str()) && *a != id {
                edges.push(Edge {
                    from: a.clone(),
                    to: id.clone(),
                    kind: EdgeKind::LoadAfter,
                    declared_by: id.clone(),
                    decl_index: i,
                });
            }
        }
        for (i, b) in m.manifest.load_before.iter().enumerate() {
            if by_id.contains_key(b.as_str()) && *b != id {
                edges.push(Edge {
                    from: id.clone(),
                    to: b.clone(),
                    kind: EdgeKind::LoadBefore,
                    declared_by: id.clone(),
                    decl_index: i,
                });
            }
        }
    }
    let nodes: Vec<String> = by_id.keys().map(|k| (*k).to_string()).collect();

    // 3. Detect cycles, dropping soft edges one at a time.
    while let Some(component) = first_cycle_component(&nodes, &edges) {
        let mut soft: Vec<usize> = edges
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.kind.is_soft() && component.contains(&e.from) && component.contains(&e.to)
            })
            .map(|(i, _)| i)
            .collect();
        if soft.is_empty() {
            let (path, via) = describe_cycle(&component, &edges);
            errors.push(LoadOrderError::Cycle { path, via });
            return Err(errors);
        }
        soft.sort_by(|&a, &b| {
            let (ea, eb) = (&edges[a], &edges[b]);
            (&ea.declared_by, ea.kind, ea.decl_index).cmp(&(
                &eb.declared_by,
                eb.kind,
                eb.decl_index,
            ))
        });
        let dropped = edges.remove(soft[0]);
        warnings.push(format!(
            "dropped soft edge {}[{}] ({} before {}): part of a load-order cycle",
            dropped.via(),
            dropped.decl_index,
            dropped.from,
            dropped.to
        ));
    }

    // 4. Kahn with the smallest id first.
    let mut indegree: BTreeMap<&str, usize> = nodes.iter().map(|n| (n.as_str(), 0)).collect();
    for e in &edges {
        *indegree
            .get_mut(e.to.as_str())
            .expect("edge target is a node") += 1;
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut order: Vec<&str> = Vec::with_capacity(nodes.len());
    while let Some(next) = ready.iter().next().copied() {
        ready.remove(next);
        order.push(next);
        for e in edges.iter().filter(|e| e.from == next) {
            let d = indegree.get_mut(e.to.as_str()).expect("node");
            *d -= 1;
            if *d == 0 {
                ready.insert(e.to.as_str());
            }
        }
    }
    debug_assert_eq!(order.len(), nodes.len(), "cycles were removed above");

    let mods = order
        .iter()
        .map(|id| {
            let m = by_id[id];
            LoadedMod {
                manifest: m.manifest.clone(),
                root: m.root.clone(),
                is_game: m.is_game,
            }
        })
        .collect();
    Ok(ModSet { mods, warnings })
}

/// The first strongly connected component with more than one node (or a
/// self-loop), as a set of ids, via Tarjan's algorithm.
fn first_cycle_component(nodes: &[String], edges: &[Edge]) -> Option<BTreeSet<String>> {
    struct Tarjan<'a> {
        nodes: &'a [String],
        succ: Vec<Vec<usize>>,
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        components: Vec<Vec<usize>>,
    }
    impl Tarjan<'_> {
        fn visit(&mut self, v: usize) {
            self.index[v] = Some(self.next);
            self.low[v] = self.next;
            self.next += 1;
            self.stack.push(v);
            self.on_stack[v] = true;
            for i in 0..self.succ[v].len() {
                let w = self.succ[v][i];
                if self.index[w].is_none() {
                    self.visit(w);
                    self.low[v] = self.low[v].min(self.low[w]);
                } else if self.on_stack[w] {
                    self.low[v] = self.low[v].min(self.index[w].expect("visited"));
                }
            }
            if self.low[v] == self.index[v].expect("set above") {
                let mut comp = Vec::new();
                loop {
                    let w = self.stack.pop().expect("v is on the stack");
                    self.on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                self.components.push(comp);
            }
        }
    }
    let idx = |id: &str| {
        nodes
            .iter()
            .position(|n| n == id)
            .expect("edge endpoints are nodes")
    };
    let mut succ = vec![Vec::new(); nodes.len()];
    for e in edges {
        succ[idx(&e.from)].push(idx(&e.to));
    }
    let mut t = Tarjan {
        nodes,
        succ,
        index: vec![None; nodes.len()],
        low: vec![0; nodes.len()],
        on_stack: vec![false; nodes.len()],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for v in 0..nodes.len() {
        if t.index[v].is_none() {
            t.visit(v);
        }
    }
    let mut cyclic: Vec<BTreeSet<String>> = t
        .components
        .iter()
        .filter(|c| c.len() > 1 || t.succ[c[0]].contains(&c[0]))
        .map(|c| c.iter().map(|&i| t.nodes[i].clone()).collect())
        .collect();
    cyclic.sort();
    cyclic.into_iter().next()
}

/// A concrete cycle inside `component`, starting from its smallest id, with
/// the `mod.field` that declared each edge.
fn describe_cycle(component: &BTreeSet<String>, edges: &[Edge]) -> (Vec<String>, Vec<String>) {
    let start = component.iter().next().expect("non-empty").clone();
    let mut path = vec![start.clone()];
    let mut via = Vec::new();
    let mut current = start.clone();
    loop {
        let next = edges
            .iter()
            .filter(|e| e.from == current && component.contains(&e.to))
            .min_by(|a, b| {
                // Prefer closing the loop, then the smallest target id.
                (a.to != start, &a.to).cmp(&(b.to != start, &b.to))
            })
            .expect("every node of a cyclic component has a successor in it");
        via.push(next.via());
        path.push(next.to.clone());
        if next.to == start || path.len() > component.len() + 1 {
            break;
        }
        current = next.to.clone();
    }
    (path, via)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Dependency, parse_version_req};

    fn m(id: &str, deps: &[(&str, &str)], after: &[&str], before: &[&str]) -> ManifestWithPath {
        ManifestWithPath {
            manifest: Manifest {
                id: id.to_string(),
                name_key: format!("{id}.mod.name"),
                version: semver::Version::new(1, 0, 0),
                engine_version: semver::VersionReq::STAR,
                dependencies: deps
                    .iter()
                    .map(|(d, r)| Dependency {
                        id: (*d).to_string(),
                        version: parse_version_req(r).unwrap(),
                    })
                    .collect(),
                load_after: after.iter().map(|s| (*s).to_string()).collect(),
                load_before: before.iter().map(|s| (*s).to_string()).collect(),
                content_root: "content".into(),
                scripts_root: "scripts".into(),
                assets_root: "assets".into(),
                locales: vec![],
                namespaces: vec![],
            },
            root: PathBuf::from(format!("mods/{id}")),
            is_game: false,
        }
    }

    fn game(id: &str) -> ManifestWithPath {
        let mut g = m(id, &[], &[], &[]);
        g.is_game = true;
        g
    }

    fn ids(set: &ModSet) -> Vec<&str> {
        set.mods.iter().map(|m| m.manifest.id.as_str()).collect()
    }

    #[test]
    fn diamond_dependencies_resolve_with_id_tie_break() {
        let found = [
            m("d", &[("b", "*"), ("c", "*")], &[], &[]),
            m("c", &[("a", "*")], &[], &[]),
            m("b", &[("a", "*")], &[], &[]),
            m("a", &[], &[], &[]),
        ];
        let set = ModSet::all(&found).unwrap();
        assert_eq!(ids(&set), vec!["a", "b", "c", "d"]);
        let found = [
            m("d", &[("b", "*"), ("a0", "*")], &[], &[]),
            m("a0", &[("a", "*")], &[], &[]),
            m("b", &[("a", "*")], &[], &[]),
            m("a", &[], &[], &[]),
        ];
        assert_eq!(
            ids(&ModSet::all(&found).unwrap()),
            vec!["a", "a0", "b", "d"]
        );
    }

    #[test]
    fn hard_cycle_is_an_error_naming_the_cycle() {
        let found = [
            m("x", &[("y", "*")], &[], &[]),
            m("y", &[("x", "*")], &[], &[]),
        ];
        let err = ModSet::all(&found).unwrap_err();
        assert_eq!(
            err[0].to_string(),
            "load order cycle: x -> y -> x (via y.dependencies, x.dependencies)"
        );
    }

    #[test]
    fn soft_cycle_drops_the_first_soft_edge_in_manifest_order() {
        let found = [m("x", &[], &["y"], &[]), m("y", &[], &["x"], &[])];
        let set = ModSet::all(&found).unwrap();
        assert_eq!(ids(&set), vec!["x", "y"]);
        assert_eq!(set.warnings.len(), 1);
        assert!(
            set.warnings[0].starts_with("dropped soft edge x.load_after[0]"),
            "{:?}",
            set.warnings
        );
    }

    #[test]
    fn load_before_contradicting_a_dependency_is_dropped() {
        // b depends on a, but a asks to load after b (soft): the hard edge wins.
        let found = [m("a", &[], &["b"], &[]), m("b", &[("a", "*")], &[], &[])];
        let set = ModSet::all(&found).unwrap();
        assert_eq!(ids(&set), vec!["a", "b"]);
        assert_eq!(set.warnings.len(), 1);
        // Same with load_before declared by b.
        let found = [m("a", &[], &[], &[]), m("b", &[("a", "*")], &[], &["a"])];
        let set = ModSet::all(&found).unwrap();
        assert_eq!(ids(&set), vec!["a", "b"]);
        assert!(set.warnings[0].contains("b.load_before[0]"));
    }

    #[test]
    fn game_loads_first_regardless_of_id() {
        let found = [
            m("aaa", &[], &[], &[]),
            game("rome"),
            m("mid", &[], &[], &[]),
        ];
        let set = ModSet::all(&found).unwrap();
        assert_eq!(ids(&set), vec!["rome", "aaa", "mid"]);
        // The game is enabled even when the enabled list omits it.
        let set = resolve_load_order(&found, &["mid".to_string()]).unwrap();
        assert_eq!(ids(&set), vec!["rome", "mid"]);
    }

    #[test]
    fn missing_dependency_and_version_mismatch_stop_resolution() {
        let found = [m("a", &[("rome", ">=1.0.0")], &[], &[])];
        let err = ModSet::all(&found).unwrap_err();
        assert_eq!(
            err[0].to_string(),
            "a: missing dependency \"rome\" (>=1.0.0)"
        );
        let mut old = m("rome", &[], &[], &[]);
        old.manifest.version = semver::Version::new(0, 9, 0);
        let found = [m("a", &[("rome", ">=1.0.0")], &[], &[]), old];
        let err = ModSet::all(&found).unwrap_err();
        assert_eq!(
            err[0].to_string(),
            "a: dependency \"rome\" is 0.9.0, requires >=1.0.0"
        );
    }

    #[test]
    fn duplicate_ids_and_unknown_enabled_ids_are_errors() {
        let found = [m("a", &[], &[], &[]), m("a", &[], &[], &[])];
        let err = ModSet::all(&found).unwrap_err();
        assert!(
            err[0]
                .to_string()
                .starts_with("modlist: duplicate mod id \"a\" at")
        );
        let found = [m("a", &[], &[], &[])];
        let err = resolve_load_order(&found, &["zzz".to_string()]).unwrap_err();
        assert_eq!(err[0], LoadOrderError::UnknownEnabledId("zzz".to_string()));
    }

    #[test]
    fn disabled_mods_do_not_constrain_order() {
        let found = [m("a", &[], &["b"], &[]), m("b", &[], &[], &[])];
        let set = resolve_load_order(&found, &["a".to_string()]).unwrap();
        assert_eq!(ids(&set), vec!["a"]);
        assert!(set.warnings.is_empty());
    }

    #[test]
    fn mod_list_hash_depends_on_order_and_version() {
        let a = ModSet::all(&[m("a", &[], &[], &[]), m("b", &[], &[], &[])]).unwrap();
        let mut b = a.clone();
        b.mods.swap(0, 1);
        assert_ne!(a.mod_list_hash(), b.mod_list_hash());
        let mut c = a.clone();
        c.mods[0].manifest.version = semver::Version::new(1, 0, 1);
        assert_ne!(a.mod_list_hash(), c.mod_list_hash());
        assert_eq!(a.mod_list_hash(), a.clone().mod_list_hash());
    }
}
