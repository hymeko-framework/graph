//! Source-first P-graph workload generator (F.11–F.16, corrective 2026-08-30).
//!
//! APPROVED-GRAPH-EDIT: m2b-source-workload-vv-corrective.
//!
//! Every valid workload's primary form is a HyMeKo description compiled through
//! the real parser → `ModuleStore` → lowering → `meta_resolve` pipeline. This
//! module turns a family + explicit parameters into that source **together with
//! an oracle computed from the parameters by formula** — separately from the
//! built edge list, so a wrong edge list and a wrong expectation cannot agree
//! (the common-mode defect the previous version had). Direct `Ir` construction
//! is reserved for the malformed-IR negatives (`meta_resolve_ir_faults.rs`).
//!
//! Safety (D): counts, byte estimate and max edge cardinality are computed with
//! checked arithmetic **before** any large `Vec`/`String` is allocated; an
//! overflow is a typed [`WorkloadError::ArithmeticOverflow`] and an over-budget
//! size a typed [`WorkloadError::TooLarge`], so a `usize::MAX`-adjacent parameter
//! never allocates or panics.

use std::fmt::Write as _;

pub mod families;
pub mod fnv;
pub mod rng;

pub use families::{WorkloadFamily, build};

/// Per-source memory guard: refuse to build a string larger than this. 64 MiB is
/// far above every committed workload and well under the phase's 16 GB cap.
pub const SOURCE_CAP_BYTES: usize = 64 * 1024 * 1024;

/// Structural oracle, computed from parameters by formula (never from the edge
/// list). Every field is independently derivable.
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

/// Generated source paired with its formula oracle.
#[derive(Debug)]
pub struct Built {
    /// The HyMeKo source.
    pub source: String,
    /// The parameter-derived oracle.
    pub oracle: ExpectedStructure,
    /// The largest edge cardinality (for reporting / invariants).
    pub max_cardinality: usize,
}

/// Why a workload could not be generated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadError {
    /// A family parameter is degenerate.
    InvalidParameter(&'static str),
    /// A size computation overflowed `usize`.
    ArithmeticOverflow(&'static str),
    /// The estimated source size exceeded [`SOURCE_CAP_BYTES`].
    TooLarge {
        /// Estimated bytes.
        est_bytes: usize,
        /// The cap.
        cap_bytes: usize,
    },
}

#[allow(dead_code)] // used by the workload test suites, not the bench example
/// Size class, **computed** from the incidence count (not a caller-set label).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadScale {
    Tiny,
    Small,
    Medium,
    Large,
    Stress,
}

/// Classify by incidence count: tiny <1e2, small 1e2–1e3, medium 1e4–1e5,
/// large 1e5–1e6, stress >1e6. (The 1e3–1e4 gap is "survey".)
#[must_use]
#[allow(dead_code)] // used by the workload test suites, not the bench example
pub fn scale_of(incidences: usize) -> WorkloadScale {
    match incidences {
        0..=99 => WorkloadScale::Tiny,
        100..=9_999 => WorkloadScale::Small,
        10_000..=99_999 => WorkloadScale::Medium,
        100_000..=999_999 => WorkloadScale::Large,
        _ => WorkloadScale::Stress,
    }
}

// ── checked arithmetic ─────────────────────────────────────────────────────

/// `a + b` or a typed overflow.
pub fn cadd(a: usize, b: usize, ctx: &'static str) -> Result<usize, WorkloadError> {
    a.checked_add(b)
        .ok_or(WorkloadError::ArithmeticOverflow(ctx))
}

/// `a * b` or a typed overflow.
pub fn cmul(a: usize, b: usize, ctx: &'static str) -> Result<usize, WorkloadError> {
    a.checked_mul(b)
        .ok_or(WorkloadError::ArithmeticOverflow(ctx))
}

/// Estimate source bytes from the oracle, with checked arithmetic, and reject an
/// over-budget workload **before** any edge list is built. ~28 bytes per material
/// declaration and ~10 bytes per incidence, plus a fixed header.
pub fn guard_size(oracle: &ExpectedStructure) -> Result<usize, WorkloadError> {
    let est = cadd(
        512,
        cadd(
            cmul(oracle.materials, 28, "materials*28")?,
            cmul(oracle.incidences, 10, "incidences*10")?,
            "decls+incidences",
        )?,
        "header+body",
    )?;
    if est > SOURCE_CAP_BYTES {
        return Err(WorkloadError::TooLarge {
            est_bytes: est,
            cap_bytes: SOURCE_CAP_BYTES,
        });
    }
    Ok(est)
}

// ── canonical structure → source ───────────────────────────────────────────

/// A canonical incidence structure: `vertices` materials and `edges` units. Each
/// edge lists its incident vertex indices in order; the emitter signs the first
/// `+` (produced) and the rest `-` (consumed).
///
/// `names`/`unit_names` carry the material and unit identifiers. The mathematical
/// families use the generic `m{i}` / `u{j}` (see [`default_names`]); the domain
/// families (robotics, network, biomed) supply semantically-meaningful names so
/// the golden fixtures read as models, not as index soup.
pub struct Structure {
    pub vertices: usize,
    pub edges: Vec<Vec<usize>>,
    pub names: Vec<String>,
    pub unit_names: Vec<String>,
}

/// Generic material names `m0..m{n-1}`.
#[must_use]
pub fn default_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("m{i}")).collect()
}

/// Generic unit names `u0..u{m-1}`.
#[must_use]
pub fn default_unit_names(m: usize) -> Vec<String> {
    (0..m).map(|j| format!("u{j}")).collect()
}

fn role_of(i: usize, n: usize) -> &'static str {
    if i == 0 {
        "raw"
    } else if i + 1 == n {
        "product"
    } else {
        "inter"
    }
}

/// Emit HyMeKo source for `s`, O(output size). The header/body shape is byte-%
/// compatible with the pre-corrective goldens.
fn emit(root_name: &str, s: &Structure, est_bytes: usize) -> String {
    let mut out = String::with_capacity(est_bytes);
    writeln!(out, "{root_name} {{").unwrap();
    writeln!(out, "    @\"meta_pgraph.hymeko\";").unwrap();
    writeln!(out, "    using pgraph.raw as raw;").unwrap();
    writeln!(out, "    using pgraph.product as product;").unwrap();
    writeln!(out, "    using pgraph.intermediate as inter;").unwrap();
    writeln!(out, "    using pgraph.process as process;").unwrap();
    writeln!(out, "}}").unwrap();
    out.push('\n');
    writeln!(out, "body {{").unwrap();
    for i in 0..s.vertices {
        writeln!(
            out,
            "    {}: + <isa> {} {{}}",
            s.names[i],
            role_of(i, s.vertices)
        )
        .unwrap();
    }
    out.push('\n');
    for (j, e) in s.edges.iter().enumerate() {
        let mut arc = String::new();
        for (k, &v) in e.iter().enumerate() {
            if k > 0 {
                arc.push_str(", ");
            }
            let sign = if k == 0 { '+' } else { '-' };
            write!(arc, "{sign}{}", s.names[v]).unwrap();
        }
        writeln!(out, "    @{} {{ ({arc}); }}", s.unit_names[j]).unwrap();
    }
    writeln!(out, "}}").unwrap();
    out
}

/// Assemble a `Built` from a formula oracle and a constructed structure, after
/// (a) the size guard and (b) cross-checking the built edges against the oracle.
///
/// The cross-check is what makes the oracle independent: the counts derived from
/// the edge list must equal the parameter-derived oracle, or the generator has a
/// bug.
pub fn assemble(
    root_name: &str,
    structure: Structure,
    oracle: ExpectedStructure,
) -> Result<Built, WorkloadError> {
    let est = guard_size(&oracle)?;

    let built_incidences: usize = structure.edges.iter().map(Vec::len).sum();
    assert_eq!(
        structure.vertices, oracle.materials,
        "vertices vs oracle materials"
    );
    assert_eq!(structure.edges.len(), oracle.units, "edges vs oracle units");
    assert_eq!(built_incidences, oracle.incidences, "incidences vs oracle");
    let max_cardinality = structure.edges.iter().map(Vec::len).max().unwrap_or(0);

    let source = emit(root_name, &structure, est);
    Ok(Built {
        source,
        oracle,
        max_cardinality,
    })
}

// ── test-side pipeline helpers ─────────────────────────────────────────────

#[allow(dead_code)] // used by the workload test suites, not the bench example
/// Compile a generated workload through the real parser + lowering pipeline
/// (in-memory provider).
pub fn lower_generated(
    b: &Built,
) -> Result<hymeko_pgraph::LoweredPGraph, hymeko_pgraph::MetaResolveError> {
    let meta = include_str!("../../data/meta_pgraph.hymeko");
    hymeko_pgraph::compile_sources(
        "workload.hymeko",
        &[
            ("workload.hymeko", b.source.as_str()),
            ("meta_pgraph.hymeko", meta),
        ],
    )
}

#[allow(dead_code)] // suite-only
/// Total realised signed incidences (inputs + outputs per unit).
pub fn realised_incidences(g: &hymeko_pgraph::LoweredPGraph) -> usize {
    g.units
        .iter()
        .map(|&u| g.inputs(u).len() + g.outputs(u).len())
        .sum()
}

#[allow(dead_code)] // suite-only
/// Assert a lowered graph matches its oracle.
pub fn assert_expected_structure(g: &hymeko_pgraph::LoweredPGraph, e: &ExpectedStructure) {
    assert_eq!(g.materials.len(), e.materials, "materials");
    assert_eq!(g.units.len(), e.units, "units");
    assert_eq!(g.raws.len(), e.raws, "raws");
    assert_eq!(g.products.len(), e.products, "products");
    assert_eq!(realised_incidences(g), e.incidences, "incidences");
}

#[allow(dead_code)] // suite-only
/// Newline normalisation for golden byte-identity (F.12).
#[must_use]
pub fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// FNV-1a-64 fingerprint of a source (stable across Rust versions).
#[must_use]
pub fn source_fingerprint(s: &str) -> u64 {
    fnv::fnv1a_64(s.as_bytes())
}
