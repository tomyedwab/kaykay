//! Dependency graph views: `dep tree` and `dep cycle`.

use crate::store::Ticket;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

struct Graph<'a> {
    nodes: BTreeMap<&'a str, &'a Ticket>,
}

impl<'a> Graph<'a> {
    fn new(tickets: &'a [Ticket], include_closed: bool) -> Self {
        let nodes = tickets
            .iter()
            .filter(|t| !t.id.is_empty() && (include_closed || t.status != "closed"))
            .map(|t| (t.id.as_str(), t))
            .collect();
        Graph { nodes }
    }

    fn deps(&self, id: &str) -> &'a [String] {
        self.nodes.get(id).map(|t| t.deps.as_slice()).unwrap_or(&[])
    }

    fn label(&self, id: &str) -> String {
        let t = self.nodes[id];
        format!("{} [{}] {}", id, t.status, t.title)
    }
}

struct TreePrinter<'a> {
    graph: Graph<'a>,
    full: bool,
    /// Deepest level at which each ticket appears below the root.
    max_depth: HashMap<&'a str, usize>,
    /// Deepest level reached anywhere in each ticket's subtree.
    subtree_depth: HashMap<&'a str, usize>,
    printed: HashSet<&'a str>,
    out: String,
}

impl<'a> TreePrinter<'a> {
    fn compute_max_depth(&mut self, id: &'a str, depth: usize, path: &mut Vec<&'a str>) {
        if !self.graph.nodes.contains_key(id) || path.contains(&id) {
            return;
        }
        let entry = self.max_depth.entry(id).or_insert(depth);
        *entry = (*entry).max(depth);
        path.push(id);
        for child in self.graph.deps(id) {
            self.compute_max_depth(child, depth + 1, path);
        }
        path.pop();
    }

    fn compute_subtree_depth(&mut self, id: &'a str, path: &mut Vec<&'a str>) {
        if !self.graph.nodes.contains_key(id)
            || path.contains(&id)
            || self.subtree_depth.contains_key(id)
        {
            return;
        }
        path.push(id);
        for child in self.graph.deps(id) {
            self.compute_subtree_depth(child, path);
        }
        path.pop();
        let mut depth = self.max_depth.get(id).copied().unwrap_or(0);
        for child in self.graph.deps(id) {
            if let Some(&d) = self.subtree_depth.get(child.as_str()) {
                depth = depth.max(d);
            }
        }
        self.subtree_depth.insert(id, depth);
    }

    fn print_children(&mut self, id: &'a str, depth: usize, prefix: &str, path: &mut Vec<&'a str>) {
        let mut children: Vec<&'a str> = Vec::new();
        for child in self.graph.deps(id) {
            let child = child.as_str();
            let Some(&child_depth) = self.max_depth.get(child) else {
                continue;
            };
            // Without --full, each ticket is shown once, at its deepest level.
            if !self.full && (self.printed.contains(child) || child_depth != depth + 1) {
                continue;
            }
            if path.contains(&child) {
                continue;
            }
            children.push(child);
        }
        children.sort_by_key(|c| (self.subtree_depth.get(c).copied().unwrap_or(0), *c));

        let count = children.len();
        for (i, child) in children.into_iter().enumerate() {
            if !self.full && !self.printed.insert(child) {
                continue;
            }
            let last = i + 1 == count;
            let connector = if last { "└── " } else { "├── " };
            let _ = writeln!(self.out, "{prefix}{connector}{}", self.graph.label(child));
            let new_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
            path.push(child);
            self.print_children(child, depth + 1, &new_prefix, path);
            path.pop();
        }
    }
}

/// Render the dependency tree rooted at the ticket matching `pattern`.
pub fn tree(tickets: &[Ticket], pattern: &str, full: bool) -> Result<String, String> {
    let graph = Graph::new(tickets, true);
    let root = if graph.nodes.contains_key(pattern) {
        pattern
    } else {
        let mut matches = graph.nodes.keys().filter(|id| id.contains(pattern));
        match (matches.next(), matches.next()) {
            (Some(id), None) => *id,
            (Some(_), Some(_)) => return Err(format!("Error: ambiguous ID {pattern}")),
            _ => return Err(format!("Error: ticket {pattern} not found")),
        }
    };
    // Borrow the root ID from the ticket so it outlives `pattern`.
    let root = graph.nodes[root].id.as_str();

    let mut p = TreePrinter {
        graph,
        full,
        max_depth: HashMap::new(),
        subtree_depth: HashMap::new(),
        printed: HashSet::new(),
        out: String::new(),
    };
    p.compute_max_depth(root, 0, &mut Vec::new());
    p.compute_subtree_depth(root, &mut Vec::new());

    let _ = writeln!(p.out, "{}", p.graph.label(root));
    p.printed.insert(root);
    p.print_children(root, 0, "", &mut vec![root]);
    Ok(p.out)
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Visiting,
    Done,
}

fn find_cycles<'a>(
    graph: &Graph<'a>,
    id: &'a str,
    marks: &mut HashMap<&'a str, Mark>,
    path: &mut Vec<&'a str>,
    cycles: &mut Vec<Vec<&'a str>>,
) {
    match marks.get(id) {
        Some(Mark::Done) => return,
        Some(Mark::Visiting) => {
            if let Some(start) = path.iter().position(|p| *p == id) {
                cycles.push(path[start..].to_vec());
            }
            return;
        }
        None => {}
    }
    if !graph.nodes.contains_key(id) {
        return;
    }
    marks.insert(id, Mark::Visiting);
    path.push(id);
    for child in graph.deps(id) {
        find_cycles(graph, child, marks, path, cycles);
    }
    path.pop();
    marks.insert(id, Mark::Done);
}

/// Report dependency cycles among tickets that are not closed.
pub fn cycles(tickets: &[Ticket]) -> String {
    let graph = Graph::new(tickets, false);
    let mut marks = HashMap::new();
    let mut found = Vec::new();
    for id in graph.nodes.keys() {
        find_cycles(&graph, id, &mut marks, &mut Vec::new(), &mut found);
    }

    // Normalize each cycle to start at its smallest ID, then de-duplicate.
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for cycle in found {
        let min = (0..cycle.len()).min_by_key(|&i| cycle[i]).unwrap_or(0);
        let mut normalized = cycle.clone();
        normalized.rotate_left(min);
        if seen.insert(normalized.clone()) {
            unique.push((cycle, normalized));
        }
    }

    if unique.is_empty() {
        return "No dependency cycles found\n".to_string();
    }
    let mut out = String::new();
    for (n, (cycle, members)) in unique.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "Cycle {}: {} -> {}",
            n + 1,
            cycle.join(" -> "),
            cycle[0]
        );
        for id in members {
            let t = graph.nodes[id];
            let _ = writeln!(out, "  {:<8} [{}] {}", id, t.status, t.title);
        }
    }
    out
}
