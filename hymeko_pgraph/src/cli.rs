//! Library backing the `pgraph` command-line tool (`src/bin/pgraph.rs`).
//!
//! All real work lives here as testable functions; the binary is a thin
//! subcommand dispatcher. The single input path is [`load_pgraph`], which
//! routes `.pgip` / meta-model `.hymeko` / literal-tag `.hymeko` to a
//! [`LoweredPGraph`]; the `render_*` / [`to_dot`] formatters and the solve
//! reuse the existing engine (no algorithm duplication).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use hymeko::common::ids::DeclId;
use thiserror::Error;

use crate::abb::{AbbOptions, solve_with_options};
use crate::lowering::{LoweredPGraph, lower};
use crate::meta_resolve::{MetaResolveError, compile_to_lowered};
use crate::msg::{MaximalStructureOptions, maximal_structure_with_options};
#[cfg(feature = "pgip")]
use crate::pgip_io::read_pgip;
use crate::ssg::{SsgOptions, enumerate_with_options};

/// Errors surfaced by the CLI loader.
#[derive(Debug, Error)]
pub enum CliError {
    /// Filesystem / IO failure.
    #[error("io: {0}")]
    Io(String),
    /// The source failed to parse (literal-tag fallback path).
    #[error("parse: {0}")]
    Parse(String),
    /// Literal-tag lowering failed.
    #[error("lower: {0}")]
    Lower(String),
    /// Meta-model resolution failed (not a missing-archetype fallback case).
    #[error(transparent)]
    Meta(#[from] MetaResolveError),
    /// `.pgip` read failed.
    #[error("pgip: {0}")]
    Pgip(String),
    /// Rendering via the external Graphviz `dot` tool failed.
    #[error("render: {0}")]
    Render(String),
}

/// Load a P-graph from any supported input.
///
/// Routing: `.pgip` → [`read_pgip`]; otherwise treat as `.hymeko` and try the
/// meta-model path ([`compile_to_lowered`], resolving includes / `using` /
/// `<isa>`). A [`MetaResolveError::MissingArchetype`] means the file is a
/// literal-tag P-graph, so fall back to [`lower`]; any other meta error is a
/// genuinely malformed meta-model P-graph and is surfaced.
pub fn load_pgraph(path: &Path) -> Result<LoweredPGraph, CliError> {
    if !path.exists() {
        return Err(CliError::Io(format!("file not found: {}", path.display())));
    }
    let is_pgip = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("pgip"))
        .unwrap_or(false);
    if is_pgip {
        #[cfg(feature = "pgip")]
        {
            return read_pgip(path).map_err(|e| CliError::Pgip(format!("{e}")));
        }
        #[cfg(not(feature = "pgip"))]
        {
            return Err(CliError::Pgip(
                "`.pgip` support not compiled in (enable the `pgip` feature)".into(),
            ));
        }
    }
    match compile_to_lowered(path) {
        Ok(g) => Ok(g),
        Err(MetaResolveError::MissingArchetype(_)) => {
            let src = std::fs::read_to_string(path).map_err(|e| CliError::Io(format!("{e}")))?;
            let desc =
                parser::parse_description(&src).map_err(|e| CliError::Parse(format!("{e:?}")))?;
            lower(&desc).map_err(|e| CliError::Lower(format!("{e}")))
        }
        Err(e) => Err(CliError::Meta(e)),
    }
}

/// The P-graph role of a material, for display.
fn role(g: &LoweredPGraph, d: DeclId) -> &'static str {
    if g.raws.contains(&d) {
        "raw"
    } else if g.products.contains(&d) {
        "product"
    } else {
        "intermediate"
    }
}

fn sorted_names(g: &LoweredPGraph, set: &BTreeSet<DeclId>) -> Vec<String> {
    let mut v: Vec<String> = set.iter().map(|d| g.decl_to_name[d].clone()).collect();
    v.sort();
    v
}

fn io_names(g: &LoweredPGraph, set: &BTreeSet<DeclId>) -> String {
    let mut v: Vec<&str> = set.iter().map(|d| g.decl_to_name[d].as_str()).collect();
    v.sort_unstable();
    if v.is_empty() {
        "∅".into()
    } else {
        v.join(", ")
    }
}

/// `read`: the entities declared in the file — materials with roles, units
/// with cost and signed I/O.
pub fn render_entities(g: &LoweredPGraph, title: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "P-graph: {title}");
    let _ = writeln!(s, "  materials ({}):", g.materials.len());
    for name in sorted_names(g, &g.materials) {
        let d = g.name_to_decl[&name];
        let _ = writeln!(s, "    {name:<14} [{}]", role(g, d));
    }
    let _ = writeln!(s, "  operating units ({}):", g.units.len());
    for name in sorted_names(g, &g.units) {
        let d = g.name_to_decl[&name];
        let _ = writeln!(
            s,
            "    {name:<10} cost {:>8.2}   in: {:<18} out: {}",
            g.costs.get(&d).copied().unwrap_or(1.0),
            io_names(g, g.inputs(d)),
            io_names(g, g.outputs(d)),
        );
    }
    s
}

/// `transform`: the bipartite P-graph — the M/O partition and the directed
/// signed-incidence edge set (`m → u` consumed, `u → m` produced).
pub fn render_pgraph(g: &LoweredPGraph, title: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "P-graph: {title}");
    let _ = writeln!(
        s,
        "  M-nodes ({}): {}",
        g.materials.len(),
        sorted_names(g, &g.materials).join(" ")
    );
    let _ = writeln!(
        s,
        "  O-nodes ({}): {}",
        g.units.len(),
        sorted_names(g, &g.units).join(" ")
    );
    let _ = writeln!(s, "  signed incidence ({} edges):", g.schema.n_edges());
    for (_, src, dst) in g.schema.edges() {
        let (sn, dn) = (&g.decl_to_name[&src], &g.decl_to_name[&dst]);
        let kind = if g.materials.contains(&src) {
            "consumed"
        } else {
            "produced"
        };
        let _ = writeln!(s, "    {sn:<14} ──{kind:<8}──▶ {dn}");
    }
    s
}

/// `solve`: MSG, SSG (guarded), and ABB over the P-graph.
pub fn render_solution(
    g: &LoweredPGraph,
    title: &str,
    msg_opts: MaximalStructureOptions,
    abb_opts: AbbOptions,
) -> String {
    let regime = if msg_opts.strict_no_excess {
        "strict"
    } else {
        "relaxed"
    };
    let mut s = String::new();
    let _ = writeln!(s, "P-graph: {title}  ({regime} no-excess)");

    let msg = maximal_structure_with_options(g, msg_opts);
    let pruned = g.units.len().saturating_sub(msg.units.len());
    let _ = writeln!(
        s,
        "  MSG  maximal structure: {{ {} }}   [{pruned} of {} units pruned]",
        sorted_names(g, &msg.units).join(", "),
        g.units.len(),
    );

    if msg.units.len() <= 30 {
        let ssg_opts = SsgOptions {
            strict_no_excess: msg_opts.strict_no_excess,
            ..SsgOptions::default()
        };
        let n = enumerate_with_options(g, &msg, ssg_opts).len();
        let _ = writeln!(s, "  SSG  feasible solution structures: {n}");
    } else {
        let _ = writeln!(
            s,
            "  SSG  skipped (|MSG| = {} > 30; use ABB)",
            msg.units.len()
        );
    }

    match solve_with_options(g, &msg, abb_opts) {
        Some(sol) => {
            let _ = writeln!(
                s,
                "  ABB  optimum: {{ {} }}   cost {:.2}   [explored {}]",
                sorted_names(g, &sol.units).join(", "),
                sol.cost,
                sol.explored,
            );
        }
        None => {
            let _ = writeln!(s, "  ABB  infeasible: no structure reaches all products");
        }
    }
    s
}

/// `generate dot`: a Graphviz DOT bipartite graph (materials as ellipses,
/// units as boxes; raws green, products gold).
pub fn to_dot(g: &LoweredPGraph, title: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "digraph {:?} {{", title);
    let _ = writeln!(s, "  rankdir=LR;");
    for name in sorted_names(g, &g.materials) {
        let d = g.name_to_decl[&name];
        let style = match role(g, d) {
            "raw" => "shape=ellipse, style=filled, fillcolor=palegreen",
            "product" => "shape=ellipse, style=filled, fillcolor=gold",
            _ => "shape=ellipse",
        };
        let _ = writeln!(s, "  {name:?} [{style}];");
    }
    for name in sorted_names(g, &g.units) {
        let _ = writeln!(
            s,
            "  {name:?} [shape=box, style=filled, fillcolor=lightblue];"
        );
    }
    for (_, src, dst) in g.schema.edges() {
        let _ = writeln!(
            s,
            "  {:?} -> {:?};",
            g.decl_to_name[&src], g.decl_to_name[&dst]
        );
    }
    let _ = writeln!(s, "}}");
    s
}

/// Render a P-graph as Graphviz DOT in the **canonical Friedler-PSE
/// notation** (Friedler, Tarján, Huang, Fan 1992; reproduced in the
/// book *P-graphs for Process Systems Engineering*, Springer):
///
/// - **Material nodes** are circles. Raw materials are filled in
///   light grey; required products are filled in white with a thicker
///   stroke; intermediates are unfilled circles.
/// - **Operating units** are short horizontal bars (thin black
///   rectangles), matching the "═" glyph the book uses.
/// - Edges are directed: M → O for inputs, O → M for outputs. The
///   layout direction is left-to-right (rankdir=LR).
///
/// Added 2026-06-03 for the Pimentel-facing reply report; the
/// default [`to_dot`] above is a more verbose debug-style rendering
/// that emphasises the bipartite types via colour, not the canonical
/// PSE convention.
pub fn to_friedler_dot(g: &LoweredPGraph, title: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "digraph {:?} {{", title);
    let _ = writeln!(s, "  rankdir=LR;");
    let _ = writeln!(s, "  bgcolor=\"transparent\";");
    let _ = writeln!(s, "  node [fontname=\"Helvetica\"];");
    let _ = writeln!(s, "  edge [arrowhead=normal, arrowsize=0.7, color=\"#333333\"];");
    let _ = writeln!(s);
    let _ = writeln!(s, "  // M-nodes (materials) — circles per Friedler PSE convention");
    for name in sorted_names(g, &g.materials) {
        let d = g.name_to_decl[&name];
        let attrs = match role(g, d) {
            "raw" => "shape=circle, style=filled, fillcolor=\"#d0d0d0\", \
                       width=0.45, fixedsize=true, penwidth=1.2",
            "product" => "shape=doublecircle, style=filled, fillcolor=\"#ffffff\", \
                           width=0.40, fixedsize=true, penwidth=1.5",
            _ => "shape=circle, style=filled, fillcolor=\"#ffffff\", \
                   width=0.40, fixedsize=true, penwidth=1.0",
        };
        let _ = writeln!(s, "  {name:?} [{attrs}];");
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "  // O-nodes (operating units) — short horizontal bars");
    for name in sorted_names(g, &g.units) {
        // A short, very thin black rectangle approximates the
        // "═" bar Friedler uses for operating units. The label sits
        // below the bar via headlabel/xlabel; for compactness we
        // keep the unit ID inside.
        let _ = writeln!(
            s,
            "  {name:?} [shape=box, style=\"filled,rounded\", \
              fillcolor=\"#222222\", fontcolor=\"#ffffff\", \
              height=0.18, width=0.55, fixedsize=true, fontsize=10, \
              penwidth=0];",
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "  // Directed edges (M -> O = input, O -> M = output)");
    for (_, src, dst) in g.schema.edges() {
        let _ = writeln!(
            s,
            "  {:?} -> {:?};",
            g.decl_to_name[&src], g.decl_to_name[&dst]
        );
    }
    let _ = writeln!(s, "}}");
    s
}

/// Render Graphviz `dot` source to an image file via the system `dot` binary,
/// e.g. `format = "png"` or `"svg"`. The DOT source is fed on stdin and written
/// to `out` with `dot -T<format> -o <out>`.
///
/// # Errors
/// [`CliError::Render`] if `dot` is not on `PATH` (with an install hint), if it
/// exits non-zero (surfacing its stderr), or on an I/O failure. Graphviz is an
/// optional runtime tool — this never panics when it is absent.
pub fn render_graphviz(dot: &str, format: &str, out: &Path) -> Result<(), CliError> {
    let mut child = Command::new("dot")
        .arg(format!("-T{format}"))
        .arg("-o")
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CliError::Render(
                "Graphviz `dot` not found on PATH — install graphviz \
                 (e.g. `apt install graphviz`, `brew install graphviz`, \
                 `choco install graphviz`), or use `--format dot` and render it yourself"
                    .into(),
            ),
            _ => CliError::Render(format!("could not start `dot`: {e}")),
        })?;

    // stdin is `Some` because we configured `Stdio::piped()` above; treat the
    // impossible `None` as an error rather than unwrapping.
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CliError::Render("`dot` stdin was not piped".into()))?;
    stdin
        .write_all(dot.as_bytes())
        .map_err(|e| CliError::Render(format!("writing DOT to `dot`: {e}")))?;
    drop(stdin); // close stdin so `dot` sees EOF

    let output = child
        .wait_with_output()
        .map_err(|e| CliError::Render(format!("waiting for `dot`: {e}")))?;
    if !output.status.success() {
        return Err(CliError::Render(format!(
            "`dot` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}
