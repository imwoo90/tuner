//! DAG running and cycle detection logic
//!
//! Provides structures and algorithms to build, validate, and schedule task dependency graphs.

//! 
//! ## Search Tags
//! #dag

use std::collections::{HashMap, HashSet, VecDeque};
use anyhow::{anyhow, Result};

/// Checks if a task dependency graph contains a cycle.
///
/// Nodes are represented by their string IDs. Edges represent dependencies:
/// `adj` maps each task ID to the list of task IDs it depends on.
pub fn check_cycle(adj: &HashMap<String, Vec<String>>) -> bool {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    for node in adj.keys() {
        if dfs(node, adj, &mut visited, &mut rec_stack) {
            return true;
        }
    }
    false
}

fn dfs(
    node: &str,
    adj: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    rec_stack: &mut HashSet<String>,
) -> bool {
    if rec_stack.contains(node) {
        return true;
    }
    if visited.contains(node) {
        return false;
    }

    rec_stack.insert(node.to_string());
    visited.insert(node.to_string());

    if let Some(deps) = adj.get(node) {
        for dep in deps {
            if dfs(dep, adj, visited, rec_stack) {
                return true;
            }
        }
    }

    rec_stack.remove(node);
    false
}

/// Helper to sort tasks topologically. Returns a list of task IDs in execution order,
/// or an error if a cycle exists.
pub fn topological_sort(adj: &HashMap<String, Vec<String>>) -> Result<Vec<String>> {
    // Kahn's algorithm
    // adj maps task -> dependencies
    // First, let's build parent -> children graph, and calculate in_degrees (number of dependencies).
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    // Initialize in_degree for all nodes
    for node in adj.keys() {
        in_degree.insert(node.clone(), 0);
        graph.entry(node.clone()).or_default();
    }

    for (node, deps) in adj {
        for dep in deps {
            // node depends on dep, so dep -> node is the edge
            // If dep is not in adj, it is a external dependency, but let's assume it's in our graph or add it
            graph.entry(dep.clone()).or_default().push(node.clone());
            *in_degree.entry(node.clone()).or_default() += 1;
            in_degree.entry(dep.clone()).or_default();
        }
    }

    let mut queue = VecDeque::new();
    for (node, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(node.clone());
        }
    }

    let mut sorted = Vec::new();
    while let Some(u) = queue.pop_front() {
        sorted.push(u.clone());
        if let Some(children) = graph.get(&u) {
            for v in children {
                if let Some(deg) = in_degree.get_mut(v) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(v.clone());
                    }
                }
            }
        }
    }

    if sorted.len() != in_degree.len() {
        return Err(anyhow!("Task dependency cycle detected"));
    }

    Ok(sorted)
}
