//! Deterministic HyMeKo **source** generator for P-graph workloads.
//!
//! APPROVED-GRAPH-EDIT: m2b-pgraph-meta-resolve-accessors (2026-08-30).
//!
//! F.11–F.16: every workload's primary representation is a valid HyMeKo
//! description that goes through the real parser → `ModuleStore` → lowering →
//! `meta_resolve` pipeline. This module produces that source *together with* an
//! independently-computed [`ExpectedStructure`] oracle, so a test never derives
//! its expectation from the code under test. Nothing here builds an `Ir` or a
//! `LoweredPGraph` directly — that is reserved for the malformed-IR negative
//! tests (`meta_resolve_ir_faults.rs`), where the parser could not produce the
//! state under test.
//!
//! A workload is a signed material–unit incidence structure. Each abstract
//! "vertex" is a P-graph material and each "hyperedge" is an operating unit
//! whose single arc produces its first incident material (`+`) and consumes the
//! rest (`-`). Roles: the first vertex is `raw`, the last is `product`, the rest
//! `intermediate` — enough to make MSG/SSG/ABB well-posed while keeping the
//! counts a pure function of the abstract structure.

#![allow(dead_code)] // shared across test binaries; not every one uses every item

use std::fmt::Write as _;

/// Workload families. Each maps an abstract combinatorial structure to a
/// P-graph so the oracle is derivable from the construction, not from the code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadFamily {
    /// Fano plane, the Steiner triple system S(2,3,7): 7 points, 7 lines of 3.
    Fano,
    /// A sunflower: `petals` blocks sharing a common `core`, each with `petal`
    /// private vertices.
    Sunflower {
        core: usize,
        petal: usize,
        petals: usize,
    },
    /// The 5-cycle C5 (a Mycielski seed): 5 vertices, 5 edges.
    MycielskiC5,
    /// The Grötzsch graph, the Mycielskian of C5: 11 vertices, 20 edges.
    MycielskiGrotzsch,
    /// A spine/leaf network: `spines` cores each linked to every one of
    /// `leaves` leaves (one unit per (spine, leaf) link).
    SpineLeaf { spines: usize, leaves: usize },
    /// A serial robot kinematic chain of `joints` links.
    RobotChain { joints: usize },
}

/// Scale label, purely descriptive for reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadScale {
    Tiny,
    Small,
    Medium,
    Large,
    Stress,
}

/// A workload request.
#[derive(Clone, Copy, Debug)]
pub struct WorkloadSpec {
    pub family: WorkloadFamily,
    pub scale: WorkloadScale,
    pub seed: u64,
}

/// The independently-derived structural oracle for a workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpectedStructure {
    /// Materials = abstract vertices.
    pub materials: usize,
    /// Units = abstract hyperedges.
    pub units: usize,
    /// One arc per unit.
    pub arcs: usize,
    /// Total signed incidences = sum of hyperedge sizes.
    pub incidences: usize,
    /// Raw materials (the first vertex).
    pub raws: usize,
    /// Product materials (the last vertex).
    pub products: usize,
}

/// Generated source paired with its oracle.
pub struct GeneratedHymeko {
    pub source: String,
    pub expected: ExpectedStructure,
    /// A stable, human-meaningful root name for the description.
    pub root_name: String,
}

/// Why generation could not proceed.
#[derive(Debug)]
pub enum WorkloadGenerationError {
    /// A family parameter was degenerate (e.g. an empty structure).
    Degenerate(&'static str),
    /// The estimated source size exceeded this phase's budget.
    TooLarge { est_bytes: usize, cap_bytes: usize },
}

/// This phase's per-source memory guard: refuse to build a giant string.
const SOURCE_CAP_BYTES: usize = 64 * 1024 * 1024;

/// One abstract hyperedge: the vertex indices it is incident on, in order.
type Edge = Vec<usize>;

/// Expand a family into `(vertex_count, edges)`.
fn structure(family: WorkloadFamily) -> Result<(usize, Vec<Edge>), WorkloadGenerationError> {
    match family {
        WorkloadFamily::Fano => Ok((
            7,
            vec![
                vec![0, 1, 2],
                vec![0, 3, 4],
                vec![0, 5, 6],
                vec![1, 3, 5],
                vec![1, 4, 6],
                vec![2, 3, 6],
                vec![2, 4, 5],
            ],
        )),
        WorkloadFamily::Sunflower {
            core,
            petal,
            petals,
        } => {
            if petals == 0 || (core == 0 && petal == 0) {
                return Err(WorkloadGenerationError::Degenerate("empty sunflower"));
            }
            // Vertices: [0..core) shared core, then each petal's private block.
            let n = core + petals * petal;
            let mut edges = Vec::with_capacity(petals);
            for p in 0..petals {
                let mut e: Edge = (0..core).collect();
                let base = core + p * petal;
                e.extend(base..base + petal);
                edges.push(e);
            }
            Ok((n, edges))
        }
        WorkloadFamily::MycielskiC5 => Ok((
            5,
            vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 4], vec![4, 0]],
        )),
        WorkloadFamily::MycielskiGrotzsch => Ok(grotzsch()),
        WorkloadFamily::SpineLeaf { spines, leaves } => {
            if spines == 0 || leaves == 0 {
                return Err(WorkloadGenerationError::Degenerate("empty spine/leaf"));
            }
            // Vertices: spines then leaves. One unit per (spine, leaf) link.
            let n = spines + leaves;
            let mut edges = Vec::with_capacity(spines * leaves);
            for s in 0..spines {
                for l in 0..leaves {
                    edges.push(vec![s, spines + l]);
                }
            }
            Ok((n, edges))
        }
        WorkloadFamily::RobotChain { joints } => {
            if joints == 0 {
                return Err(WorkloadGenerationError::Degenerate("empty chain"));
            }
            // joints+1 links; one unit per joint connecting consecutive links.
            let n = joints + 1;
            let edges = (0..joints).map(|j| vec![j, j + 1]).collect();
            Ok((n, edges))
        }
    }
}

/// The Grötzsch graph as an explicit edge list (11 vertices, 20 edges).
///
/// Mycielskian of C5: outer cycle `u0..u4`, shadow `v0..v4`, apex `w`.
/// Edges: the C5 among `u`; each `vi` joined to the two `u`-neighbours of `ui`;
/// each `vi` joined to `w`.
fn grotzsch() -> (usize, Vec<Edge>) {
    // 0..5 = u0..u4 ; 5..10 = v0..v4 ; 10 = w
    let u = |i: usize| i % 5;
    let v = |i: usize| 5 + (i % 5);
    let w = 10usize;
    let mut edges = Vec::new();
    // C5 on u
    for i in 0..5 {
        edges.push(vec![u(i), u(i + 1)]);
    }
    // vi ~ u(i-1), u(i+1)
    for i in 0..5 {
        edges.push(vec![v(i), u(i + 4)]); // i-1 mod 5
        edges.push(vec![v(i), u(i + 1)]);
    }
    // vi ~ w
    for i in 0..5 {
        edges.push(vec![v(i), w]);
    }
    (11, edges)
}

/// Deterministic vertex name: `m{index}`.
fn vertex_name(i: usize) -> String {
    format!("m{i}")
}

/// Deterministic unit name: `u{index}`.
fn unit_name(j: usize) -> String {
    format!("u{j}")
}

/// The role alias a vertex declares `<isa>`.
fn role_of(i: usize, n: usize) -> &'static str {
    if i == 0 {
        "raw"
    } else if i + 1 == n {
        "product"
    } else {
        "inter"
    }
}

/// Generate the HyMeKo source and its oracle for `spec`.
///
/// # Errors
/// [`WorkloadGenerationError`] for a degenerate family or an over-budget size.
pub fn generate_hymeko(spec: &WorkloadSpec) -> Result<GeneratedHymeko, WorkloadGenerationError> {
    let (n, edges) = structure(spec.family)?;
    if n < 2 {
        return Err(WorkloadGenerationError::Degenerate("needs >= 2 vertices"));
    }

    let incidences: usize = edges.iter().map(Vec::len).sum();
    // Size estimate with checked arithmetic: ~24 bytes/material decl, ~10
    // bytes/incidence, plus a fixed header.
    let est = 512usize
        .saturating_add(n.saturating_mul(28))
        .saturating_add(incidences.saturating_mul(10));
    if est > SOURCE_CAP_BYTES {
        return Err(WorkloadGenerationError::TooLarge {
            est_bytes: est,
            cap_bytes: SOURCE_CAP_BYTES,
        });
    }

    let root_name = family_root_name(spec.family);
    let mut s = String::with_capacity(est);

    // Header scope: include the meta-model and alias the archetypes.
    writeln!(s, "{root_name} {{").unwrap();
    writeln!(s, "    @\"meta_pgraph.hymeko\";").unwrap();
    writeln!(s, "    using pgraph.raw as raw;").unwrap();
    writeln!(s, "    using pgraph.product as product;").unwrap();
    writeln!(s, "    using pgraph.intermediate as inter;").unwrap();
    writeln!(s, "    using pgraph.process as process;").unwrap();
    writeln!(s, "}}").unwrap();
    s.push('\n');

    // Body scope: materials then units.
    writeln!(s, "body {{").unwrap();
    for i in 0..n {
        writeln!(s, "    {}: + <isa> {} {{}}", vertex_name(i), role_of(i, n)).unwrap();
    }
    s.push('\n');
    for (j, e) in edges.iter().enumerate() {
        // First incident vertex produced (+), the rest consumed (-).
        let mut arc = String::new();
        for (k, &v) in e.iter().enumerate() {
            if k > 0 {
                arc.push_str(", ");
            }
            let sign = if k == 0 { '+' } else { '-' };
            write!(arc, "{sign}{}", vertex_name(v)).unwrap();
        }
        writeln!(s, "    @{} {{ ({arc}); }}", unit_name(j)).unwrap();
    }
    writeln!(s, "}}").unwrap();

    let expected = ExpectedStructure {
        materials: n,
        units: edges.len(),
        arcs: edges.len(),
        incidences,
        raws: 1,
        products: 1,
    };
    Ok(GeneratedHymeko {
        source: s,
        expected,
        root_name,
    })
}

fn family_root_name(family: WorkloadFamily) -> String {
    match family {
        WorkloadFamily::Fano => "steiner_fano_7".into(),
        WorkloadFamily::Sunflower { .. } => "sunflower".into(),
        WorkloadFamily::MycielskiC5 => "mycielski_c5".into(),
        WorkloadFamily::MycielskiGrotzsch => "mycielski_grotzsch".into(),
        WorkloadFamily::SpineLeaf { .. } => "network_spine_leaf".into(),
        WorkloadFamily::RobotChain { .. } => "robot_chain".into(),
    }
}

/// Compile a generated workload through the real parser + lowering pipeline.
pub fn lower_generated(
    g: &GeneratedHymeko,
) -> Result<hymeko_pgraph::LoweredPGraph, hymeko_pgraph::MetaResolveError> {
    let meta = include_str!("../../data/meta_pgraph.hymeko");
    hymeko_pgraph::compile_sources(
        "workload.hymeko",
        &[
            ("workload.hymeko", g.source.as_str()),
            ("meta_pgraph.hymeko", meta),
        ],
    )
}

/// Total signed incidences realised in a lowered graph (inputs + outputs, per
/// unit) — the observable counterpart of [`ExpectedStructure::incidences`].
pub fn realised_incidences(g: &hymeko_pgraph::LoweredPGraph) -> usize {
    g.units
        .iter()
        .map(|&u| g.inputs(u).len() + g.outputs(u).len())
        .sum()
}

/// Assert a lowered graph matches its independently-derived oracle.
pub fn assert_expected_structure(g: &hymeko_pgraph::LoweredPGraph, e: &ExpectedStructure) {
    assert_eq!(g.materials.len(), e.materials, "materials");
    assert_eq!(g.units.len(), e.units, "units");
    assert_eq!(g.raws.len(), e.raws, "raws");
    assert_eq!(g.products.len(), e.products, "products");
    assert_eq!(realised_incidences(g), e.incidences, "incidences");
}

/// Newline normalisation for byte-identity comparison (F.12).
pub fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}
