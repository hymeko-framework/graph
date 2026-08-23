//! Serialize MSG / SSG / ABB results for tooling (Python drivers, CI).
//!
//! [`analyze_source`] parses HyMeKo P-graph text, lowers it, runs MSG,
//! then optionally full SSG or cost-minimal ABB.  SSG is omitted when
//! `|O_MSG| > 30` (see [`ssg::enumerate_with_options`]).
#![allow(missing_docs)] // JSON DTO field names are self-describing for tooling.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use hymeko::common::ids::{DeclId, EdgeId};
use serde::Serialize;

use crate::abb::{AbbOptions, AbbSolution, solve_with_regime};
use crate::axiom_extensions::{ExtensionAxiomBundle, ExtensionAxiomViolation};
use crate::axioms::{AxiomBundle, AxiomViolation};
use crate::lowering::{LoweredPGraph, lower};
use crate::msg::{MaximalStructureOptions, maximal_structure_with_regime};
use crate::schema::{PGraphSchema, PNodeKind};
use crate::ssg::{SolutionStructure, SsgOptions, enumerate_with_options};
use parser::parse_description;

/// Which analysis stages to emit beyond MSG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpAlgorithm {
    /// Maximal structure (unit id set) only.
    Msg,
    /// All combinatorially feasible solution structures inside MSG.
    Ssg,
    /// Cost-minimal feasible structure (ABB).
    Abb,
}

/// Which SSG implementation drives the `--algorithm ssg` output.
///
/// 2026-06-03 (Pimentel benchmark): the brute path enumerates the
/// $2^{|O_\mathrm{MSG}|}$ subset lattice and applies forward-DFS
/// feasibility (the existing default, here for $|O_\mathrm{MSG}| \le 30$).
/// The decision-mapping path is the canonical Friedler 1992 SSG
/// (`ssg_dm.rs`, Ch. 5 Def. 5.1) — it generates each solution-structure
/// exactly once via per-material production decisions and matches the
/// published structure counts for every example in
/// *P-graphs for Process Systems Engineering*.
///
/// Two outputs can differ when supersets are involved: the brute
/// algorithm emits both `{O1, O3}` and `{O1, O3, O6}` when both pass
/// the forward-DFS feasibility check, while the decision-mapping
/// algorithm consolidates them via the per-material decision recursion.
/// On `data/pgraph/Chapter6/pimentel_distractors.hymeko` the brute path
/// returns 25 structures and the decision-mapping path returns the
/// canonical 19 (matching Pimentel's docx).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsgAlgorithm {
    /// Subset-lattice brute force (default, back-compat).
    Brute,
    /// Decision-mapping (canonical Friedler 1992).
    DecisionMapping,
}

impl FromStr for SsgAlgorithm {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "brute" => Ok(SsgAlgorithm::Brute),
            "decision-mapping" | "decision_mapping" | "dm" => {
                Ok(SsgAlgorithm::DecisionMapping)
            }
            other => Err(format!(
                "unknown ssg algorithm '{other}' (use brute | decision-mapping | dm)"
            )),
        }
    }
}

/// Parse CLI / config spelling: `msg`, `ssg`, `abb`.
impl FromStr for DumpAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "msg" => Ok(DumpAlgorithm::Msg),
            "ssg" => Ok(DumpAlgorithm::Ssg),
            "abb" => Ok(DumpAlgorithm::Abb),
            other => Err(format!("unknown algorithm `{other}` (use msg|ssg|abb)")),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AbbJson {
    /// Operating unit names in the optimal solution.
    pub units: Vec<String>,
    pub cost: f64,
    pub explored: u64,
    pub pruned_by_inclusion: u64,
    pub pruned_by_reachability: u64,
}

/// Phase 7 (2026-05-19): canonical-Friedler / extension-axiom
/// certificate emitted alongside each MSG/SSG/ABB analysis.
///
/// `status` is `"PASS"` or `"FAIL"`. When `status == "FAIL"`,
/// `violation_tags` lists the short axiom names that fired
/// (`"S1".."S5"` for canonical or `"E-NoExcess"`, `"E-WellFormed"`,
/// `"E-ConsumedHasProducer"` for the extension bundle) and
/// `offenders` lists the per-tag offending declaration names.
#[derive(Debug, Serialize)]
pub struct AxiomCertificateJson {
    pub status: String,
    pub violation_tags: Vec<String>,
    pub offenders: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Serialize)]
pub struct PgraphAnalysisJson {
    /// True when parse + lower + MSG succeeded.
    pub ok: bool,
    pub description: String,
    pub algorithm: String,
    pub parse_error: Option<String>,
    pub lower_error: Option<String>,
    /// Unit names in the maximal structure (post-MSG), sorted.
    pub msg_units: Vec<String>,
    /// Feasible SSG structures as name lists (sorted within each structure).
    pub ssg_structures: Option<Vec<Vec<String>>>,
    /// Set when SSG was skipped (e.g. MSG too large).
    pub ssg_note: Option<String>,
    pub abb: Option<AbbJson>,
    /// Phase 7: canonical Friedler 1992 S1..S5 certificate on the
    /// full schema (before any MSG pruning).
    pub canonical_full: AxiomCertificateJson,
    /// Phase 7: orthogonal extension-axiom bundle certificate on the
    /// full schema.
    pub extension_full: AxiomCertificateJson,
    /// Phase 7: canonical certificate on the ABB-selected sub-schema.
    /// `None` when the algorithm is `msg`/`ssg` or no feasible
    /// solution was found.
    pub canonical_abb_subschema: Option<AxiomCertificateJson>,
    /// Phase 7: extension certificate on the ABB-selected sub-schema.
    pub extension_abb_subschema: Option<AxiomCertificateJson>,
    /// Phase 7: echo of the engine's `strict_no_excess` flag used for
    /// this analysis.
    pub strict_no_excess: bool,
    /// Phase 10 (2026-05-19): names of multi-cost dimensions
    /// declared in the lowered P-graph, in canonical (alphabetised)
    /// order. Empty when the `.hymeko` source carries no
    /// `cost <dim_name> N;` tagged children — i.e.\ the scalar-only
    /// path that pre-Phase-10 callers used.
    pub cost_dimensions: Vec<String>,
    /// Phase 10: echo of the active `cost_weights` for the ABB
    /// inclusion bound. `None` ⇒ scalar-cost fallback;
    /// `Some(weights)` ⇒ weighted-sum dot product over
    /// [`cost_dimensions`].
    pub cost_weights_echo: Option<Vec<f64>>,
    /// Phase 10: per-dimension cost contribution of the ABB
    /// selection, in `cost_dimensions` order. Each entry is
    /// `(dim_name, sum_of_unit_costs_in_that_dim)`. Echoes the
    /// selection's full cost vector before the dot product is
    /// taken, so downstream analysis can see the multi-cost trade
    /// the framework picked. `None` when no ABB selection or no
    /// `cost_dimensions`.
    pub abb_cost_breakdown: Option<Vec<(String, f64)>>,
    /// 2026-06-03 (Pimentel benchmark): top-$k$ feasible structures
    /// ranked ascending by cost. Populated only when the analysis
    /// is invoked via the `_topk` entry point with `top_k > 1` (the
    /// CLI flag is `--top-k N`). The first entry (cost-minimal)
    /// matches [`Self::abb`] exactly. The decision-mapping SSG
    /// produces these solutions, so the `explored` / `pruned_*`
    /// counters are zero on every entry (the recursive B&B
    /// counters apply only to the single-best [`solve_with_regime`]
    /// path). `None` ⇒ only the single best is requested.
    pub abb_top_k: Option<Vec<AbbJson>>,
}

fn unit_names(p: &LoweredPGraph, sol: &SolutionStructure) -> Vec<String> {
    let mut names: Vec<String> = sol
        .units
        .iter()
        .map(|id| p.decl_to_name[id].clone())
        .collect();
    names.sort();
    names
}

// ─── Phase 7: axiom certificate helpers ──────────────────────────────

fn name(p: &LoweredPGraph, d: DeclId) -> String {
    p.decl_to_name
        .get(&d)
        .cloned()
        .unwrap_or_else(|| format!("decl#{}", d.raw()))
}

fn cert_pass() -> AxiomCertificateJson {
    AxiomCertificateJson {
        status: "PASS".into(),
        violation_tags: vec![],
        offenders: vec![],
    }
}

fn canonical_cert(
    p: &LoweredPGraph,
    schema: &PGraphSchema,
    raws: &BTreeSet<DeclId>,
    products: &BTreeSet<DeclId>,
) -> AxiomCertificateJson {
    let bundle = AxiomBundle::new(raws.iter().copied(), []);
    match bundle.validate(schema, products) {
        Ok(()) => cert_pass(),
        Err(violations) => {
            let mut tags: Vec<String> = Vec::new();
            let mut offenders: Vec<(String, Vec<String>)> = Vec::new();
            for v in &violations {
                match v {
                    AxiomViolation::MissingProducts { missing } => {
                        tags.push("S1".into());
                        offenders
                            .push(("S1".into(), missing.iter().map(|d| name(p, *d)).collect()));
                    }
                    AxiomViolation::RawMaterialDirectionFailures {
                        non_raw_without_producer,
                        raw_with_producer,
                    } => {
                        tags.push("S2".into());
                        let mut combined: Vec<String> = Vec::new();
                        for d in non_raw_without_producer {
                            combined.push(format!("non_raw:{}", name(p, *d)));
                        }
                        for d in raw_with_producer {
                            combined.push(format!("raw:{}", name(p, *d)));
                        }
                        offenders.push(("S2".into(), combined));
                    }
                    AxiomViolation::InvalidUnits { invalid } => {
                        tags.push("S3".into());
                        offenders
                            .push(("S3".into(), invalid.iter().map(|d| name(p, *d)).collect()));
                    }
                    AxiomViolation::UnitsWithoutPathToProduct { offenders: o } => {
                        tags.push("S4".into());
                        offenders.push(("S4".into(), o.iter().map(|d| name(p, *d)).collect()));
                    }
                    AxiomViolation::IsolatedMaterials { offenders: o } => {
                        tags.push("S5".into());
                        offenders.push(("S5".into(), o.iter().map(|d| name(p, *d)).collect()));
                    }
                }
            }
            AxiomCertificateJson {
                status: "FAIL".into(),
                violation_tags: tags,
                offenders,
            }
        }
    }
}

fn extension_cert(
    p: &LoweredPGraph,
    schema: &PGraphSchema,
    raws: &BTreeSet<DeclId>,
    products: &BTreeSet<DeclId>,
) -> AxiomCertificateJson {
    let bundle = ExtensionAxiomBundle::new(raws.iter().copied());
    match bundle.validate(schema, products) {
        Ok(()) => cert_pass(),
        Err(violations) => {
            let mut tags: Vec<String> = Vec::new();
            let mut offenders: Vec<(String, Vec<String>)> = Vec::new();
            for v in &violations {
                match v {
                    ExtensionAxiomViolation::NonReachingMaterials { offenders: o } => {
                        tags.push("E-NoExcess".into());
                        offenders
                            .push(("E-NoExcess".into(), o.iter().map(|d| name(p, *d)).collect()));
                    }
                    ExtensionAxiomViolation::UnitsWithDegreeZero { offenders: o } => {
                        tags.push("E-WellFormed".into());
                        offenders.push((
                            "E-WellFormed".into(),
                            o.iter().map(|d| name(p, *d)).collect(),
                        ));
                    }
                    ExtensionAxiomViolation::ConsumedMaterialWithoutProducer { offenders: o } => {
                        tags.push("E-ConsumedHasProducer".into());
                        offenders.push((
                            "E-ConsumedHasProducer".into(),
                            o.iter().map(|d| name(p, *d)).collect(),
                        ));
                    }
                }
            }
            AxiomCertificateJson {
                status: "FAIL".into(),
                violation_tags: tags,
                offenders,
            }
        }
    }
}

/// Project the lowered schema onto a subset of surviving units; used
/// to validate the ABB-selected sub-schema against both bundles.
fn project_subschema(p: &LoweredPGraph, surviving_units: &BTreeSet<DeclId>) -> PGraphSchema {
    let mut kinds: BTreeMap<DeclId, PNodeKind> = BTreeMap::new();
    let mut materials: BTreeSet<DeclId> = BTreeSet::new();
    for u in surviving_units {
        kinds.insert(*u, PNodeKind::OperatingUnit);
        materials.extend(p.inputs(*u).iter().copied());
        materials.extend(p.outputs(*u).iter().copied());
    }
    materials.extend(p.products.iter().copied());
    for m in &materials {
        kinds.insert(*m, PNodeKind::Material);
    }
    let mut edges: BTreeMap<EdgeId, (DeclId, DeclId)> = BTreeMap::new();
    let mut next_eid = 0usize;
    for (_, src, dst) in p.schema.edges() {
        let keep = match (p.schema.kind(src), p.schema.kind(dst)) {
            (Some(PNodeKind::Material), Some(PNodeKind::OperatingUnit)) => {
                materials.contains(&src) && surviving_units.contains(&dst)
            }
            (Some(PNodeKind::OperatingUnit), Some(PNodeKind::Material)) => {
                surviving_units.contains(&src) && materials.contains(&dst)
            }
            _ => false,
        };
        if keep {
            edges.insert(EdgeId::new(next_eid), (src, dst));
            next_eid += 1;
        }
    }
    PGraphSchema::try_new(kinds, edges).expect("projected schema must be bipartite")
}

fn empty_cert_with(status: &str) -> AxiomCertificateJson {
    AxiomCertificateJson {
        status: status.into(),
        violation_tags: vec![],
        offenders: vec![],
    }
}

fn abb_to_json(p: &LoweredPGraph, abb: &AbbSolution) -> AbbJson {
    let mut names: Vec<String> = abb
        .units
        .iter()
        .map(|id| p.decl_to_name[id].clone())
        .collect();
    names.sort();
    AbbJson {
        units: names,
        cost: abb.cost,
        explored: abb.explored,
        pruned_by_inclusion: abb.pruned_by_inclusion,
        pruned_by_reachability: abb.pruned_by_reachability,
    }
}

/// Parse `hymeko_src`, lower to a P-graph, run MSG, then `algorithm` stages.
///
/// Back-compat shim: same as
/// [`analyze_source_with_options`] with default ABB options.
pub fn analyze_source(hymeko_src: &str, algorithm: DumpAlgorithm) -> PgraphAnalysisJson {
    analyze_source_with_options(hymeko_src, algorithm, AbbOptions::default())
}

/// Stage P-mo (2026-05-19): explicit ABB options entry point.
///
/// Back-compat with the relaxed-MSG addition: same as
/// [`analyze_source_with_full_options`] with
/// [`MaximalStructureOptions::default()`].
pub fn analyze_source_with_options(
    hymeko_src: &str,
    algorithm: DumpAlgorithm,
    opts: AbbOptions,
) -> PgraphAnalysisJson {
    analyze_source_with_full_options(
        hymeko_src,
        algorithm,
        MaximalStructureOptions::default(),
        opts,
    )
}

/// Stage relaxed-MSG (2026-05-19): full-options entry point — both
/// MSG and ABB options surfaced. Most callers want the
/// back-compat shim [`analyze_source_with_options`]; the textbook-
/// chapter validation pipeline (which needs the relaxed MSG to match
/// P-graph Studio) uses this entry point with
/// `MaximalStructureOptions { strict_no_excess: false }`.
pub fn analyze_source_with_full_options(
    hymeko_src: &str,
    algorithm: DumpAlgorithm,
    msg_opts: MaximalStructureOptions,
    opts: AbbOptions,
) -> PgraphAnalysisJson {
    analyze_source_with_regime(
        hymeko_src,
        algorithm,
        crate::regime::from_strict_flag(msg_opts.strict_no_excess),
        opts,
    )
}

/// As [`analyze_source_with_full_options`] but driven by an explicit
/// [`Regime`](crate::regime) (supports composites). Parses, lowers, then
/// delegates to [`analyze_lowered_with_regime`].
pub fn analyze_source_with_regime(
    hymeko_src: &str,
    algorithm: DumpAlgorithm,
    regime: &dyn crate::regime::Regime,
    opts: AbbOptions,
) -> PgraphAnalysisJson {
    let algo_label = match algorithm {
        DumpAlgorithm::Msg => "msg",
        DumpAlgorithm::Ssg => "ssg",
        DumpAlgorithm::Abb => "abb",
    };
    let strict_echo = regime.name() != "canonical";
    let cost_weights_echo = opts.cost_weights.clone();
    let desc = parse_description(hymeko_src);
    let d = match desc {
        Ok(d) => d,
        Err(e) => {
            return PgraphAnalysisJson {
                ok: false,
                description: String::new(),
                algorithm: algo_label.into(),
                parse_error: Some(format!("{e:?}")),
                lower_error: None,
                msg_units: vec![],
                ssg_structures: None,
                ssg_note: None,
                abb: None,
                canonical_full: empty_cert_with("UNKNOWN"),
                extension_full: empty_cert_with("UNKNOWN"),
                canonical_abb_subschema: None,
                extension_abb_subschema: None,
                strict_no_excess: strict_echo,
                cost_dimensions: vec![],
                cost_weights_echo,
                abb_cost_breakdown: None,
                abb_top_k: None,
            };
        }
    };
    let description = d.name.to_string();
    let p = match lower(&d) {
        Ok(p) => p,
        Err(e) => {
            return PgraphAnalysisJson {
                ok: false,
                description,
                algorithm: algo_label.into(),
                parse_error: None,
                lower_error: Some(e.to_string()),
                msg_units: vec![],
                ssg_structures: None,
                ssg_note: None,
                abb: None,
                canonical_full: empty_cert_with("UNKNOWN"),
                extension_full: empty_cert_with("UNKNOWN"),
                canonical_abb_subschema: None,
                extension_abb_subschema: None,
                strict_no_excess: strict_echo,
                cost_dimensions: vec![],
                cost_weights_echo,
                abb_cost_breakdown: None,
                abb_top_k: None,
            };
        }
    };

    let (json, _abb) = analyze_lowered_with_regime(&p, description, algorithm, regime, opts);
    json
}

/// Top-$k$ variant of [`analyze_source_with_regime`]. Same parse +
/// lower pipeline, then delegates to
/// [`analyze_lowered_with_regime_topk`]. The CLI binary uses this
/// path when `--top-k N` is supplied.
pub fn analyze_source_with_regime_topk(
    hymeko_src: &str,
    algorithm: DumpAlgorithm,
    regime: &dyn crate::regime::Regime,
    opts: AbbOptions,
    top_k: usize,
) -> PgraphAnalysisJson {
    if top_k <= 1 {
        return analyze_source_with_regime(hymeko_src, algorithm, regime, opts);
    }
    let algo_label = match algorithm {
        DumpAlgorithm::Msg => "msg",
        DumpAlgorithm::Ssg => "ssg",
        DumpAlgorithm::Abb => "abb",
    };
    let strict_echo = regime.name() != "canonical";
    let cost_weights_echo = opts.cost_weights.clone();
    let desc = parse_description(hymeko_src);
    let d = match desc {
        Ok(d) => d,
        Err(e) => {
            return PgraphAnalysisJson {
                ok: false,
                description: String::new(),
                algorithm: algo_label.into(),
                parse_error: Some(format!("{e:?}")),
                lower_error: None,
                msg_units: vec![],
                ssg_structures: None,
                ssg_note: None,
                abb: None,
                canonical_full: empty_cert_with("UNKNOWN"),
                extension_full: empty_cert_with("UNKNOWN"),
                canonical_abb_subschema: None,
                extension_abb_subschema: None,
                strict_no_excess: strict_echo,
                cost_dimensions: vec![],
                cost_weights_echo,
                abb_cost_breakdown: None,
                abb_top_k: None,
            };
        }
    };
    let description = d.name.to_string();
    let p = match lower(&d) {
        Ok(p) => p,
        Err(e) => {
            return PgraphAnalysisJson {
                ok: false,
                description,
                algorithm: algo_label.into(),
                parse_error: None,
                lower_error: Some(e.to_string()),
                msg_units: vec![],
                ssg_structures: None,
                ssg_note: None,
                abb: None,
                canonical_full: empty_cert_with("UNKNOWN"),
                extension_full: empty_cert_with("UNKNOWN"),
                canonical_abb_subschema: None,
                extension_abb_subschema: None,
                strict_no_excess: strict_echo,
                cost_dimensions: vec![],
                cost_weights_echo,
                abb_cost_breakdown: None,
                abb_top_k: None,
            };
        }
    };
    let (json, _abb) = analyze_lowered_with_regime_topk(
        &p, description, algorithm, regime, opts, top_k,
    );
    json
}

/// Source variant of [`analyze_lowered_with_regime_full`]. Parses,
/// lowers, then delegates with both `top_k` and `ssg_algorithm`. The
/// CLI binary uses this when `--ssg-algorithm` and/or `--top-k` are
/// supplied.
pub fn analyze_source_with_regime_full(
    hymeko_src: &str,
    algorithm: DumpAlgorithm,
    regime: &dyn crate::regime::Regime,
    opts: AbbOptions,
    top_k: usize,
    ssg_algorithm: SsgAlgorithm,
) -> PgraphAnalysisJson {
    if top_k <= 1 && matches!(ssg_algorithm, SsgAlgorithm::Brute) {
        return analyze_source_with_regime(hymeko_src, algorithm, regime, opts);
    }
    let algo_label = match algorithm {
        DumpAlgorithm::Msg => "msg",
        DumpAlgorithm::Ssg => "ssg",
        DumpAlgorithm::Abb => "abb",
    };
    let strict_echo = regime.name() != "canonical";
    let cost_weights_echo = opts.cost_weights.clone();
    let desc = parse_description(hymeko_src);
    let d = match desc {
        Ok(d) => d,
        Err(e) => {
            return PgraphAnalysisJson {
                ok: false,
                description: String::new(),
                algorithm: algo_label.into(),
                parse_error: Some(format!("{e:?}")),
                lower_error: None,
                msg_units: vec![],
                ssg_structures: None,
                ssg_note: None,
                abb: None,
                canonical_full: empty_cert_with("UNKNOWN"),
                extension_full: empty_cert_with("UNKNOWN"),
                canonical_abb_subschema: None,
                extension_abb_subschema: None,
                strict_no_excess: strict_echo,
                cost_dimensions: vec![],
                cost_weights_echo,
                abb_cost_breakdown: None,
                abb_top_k: None,
            };
        }
    };
    let description = d.name.to_string();
    let p = match lower(&d) {
        Ok(p) => p,
        Err(e) => {
            return PgraphAnalysisJson {
                ok: false,
                description,
                algorithm: algo_label.into(),
                parse_error: None,
                lower_error: Some(e.to_string()),
                msg_units: vec![],
                ssg_structures: None,
                ssg_note: None,
                abb: None,
                canonical_full: empty_cert_with("UNKNOWN"),
                extension_full: empty_cert_with("UNKNOWN"),
                canonical_abb_subschema: None,
                extension_abb_subschema: None,
                strict_no_excess: strict_echo,
                cost_dimensions: vec![],
                cost_weights_echo,
                abb_cost_breakdown: None,
                abb_top_k: None,
            };
        }
    };
    let (json, _abb) = analyze_lowered_with_regime_full(
        &p, description, algorithm, regime, opts, top_k, ssg_algorithm,
    );
    json
}

/// Stage P-io (2026-05-19): analysis entry point for a graph that is
/// already lowered (e.g.\ read directly from `.pgip` via
/// [`crate::pgip_io::read_pgip`]).
///
/// Returns the JSON dump alongside the raw [`AbbSolution`] (when the
/// algorithm is ABB and a feasible solution exists) so callers can
/// pipe the optimum into [`crate::pgip_io::write_pgip`].
pub fn analyze_lowered_with_full_options(
    p: &LoweredPGraph,
    description: String,
    algorithm: DumpAlgorithm,
    msg_opts: MaximalStructureOptions,
    opts: AbbOptions,
) -> (PgraphAnalysisJson, Option<AbbSolution>) {
    analyze_lowered_with_regime(
        p,
        description,
        algorithm,
        crate::regime::from_strict_flag(msg_opts.strict_no_excess),
        opts,
    )
}

/// Top-$k$ variant of [`analyze_lowered_with_regime`] — populates
/// [`PgraphAnalysisJson::abb_top_k`] with the $k$ cost-minimal feasible
/// structures (ascending by cost). When `top_k == 1` the result is
/// byte-identical to the single-best path. Added 2026-06-03 for the
/// Pimentel benchmark CLI verification (`--top-k 3` checks the 2nd-best
/// and 3rd-best alternatives the docx lists).
pub fn analyze_lowered_with_regime_topk(
    p: &LoweredPGraph,
    description: String,
    algorithm: DumpAlgorithm,
    regime: &dyn crate::regime::Regime,
    opts: AbbOptions,
    top_k: usize,
) -> (PgraphAnalysisJson, Option<AbbSolution>) {
    analyze_lowered_with_regime_full(
        p, description, algorithm, regime, opts, top_k,
        SsgAlgorithm::Brute,
    )
}

/// Full-options variant of [`analyze_lowered_with_regime`]: accepts both
/// the top-$k$ for ABB and the SSG implementation choice (brute vs
/// canonical decision-mapping). Added 2026-06-03 for the Pimentel
/// benchmark CLI; reproduces the docx-expected 19 solution structures
/// via `SsgAlgorithm::DecisionMapping`.
pub fn analyze_lowered_with_regime_full(
    p: &LoweredPGraph,
    description: String,
    algorithm: DumpAlgorithm,
    regime: &dyn crate::regime::Regime,
    opts: AbbOptions,
    top_k: usize,
    ssg_algorithm: SsgAlgorithm,
) -> (PgraphAnalysisJson, Option<AbbSolution>) {
    let (mut out, abb_single) = analyze_lowered_with_regime(
        p, description, algorithm, regime, opts.clone(),
    );
    // Re-emit SSG via decision-mapping if requested; replaces the
    // brute output that the inner function produced.
    if matches!(algorithm, DumpAlgorithm::Ssg)
        && matches!(ssg_algorithm, SsgAlgorithm::DecisionMapping)
    {
        let m = maximal_structure_with_regime(p, regime);
        let dm_sols = crate::ssg_dm::enumerate(p, &m);
        let structures: Vec<Vec<String>> = dm_sols
            .iter()
            .map(|s| {
                let mut v: Vec<String> = s
                    .units
                    .iter()
                    .map(|d| p.decl_to_name[d].clone())
                    .collect();
                v.sort();
                v
            })
            .collect();
        out.ssg_structures = Some(structures);
        out.ssg_note = Some("decision-mapping (canonical Friedler)".into());
    }
    // Only ABB requests carry a top-K extension; MSG / SSG ignore it.
    if matches!(algorithm, DumpAlgorithm::Abb) && top_k > 1 {
        let m = maximal_structure_with_regime(p, regime);
        let solutions = crate::abb::solve_top_k_with_regime(
            p, &m, top_k, opts, regime,
        );
        out.abb_top_k = Some(
            solutions
                .iter()
                .map(|sol| abb_to_json(p, sol))
                .collect(),
        );
    }
    (out, abb_single)
}

/// As [`analyze_lowered_with_full_options`] but driven by an explicit
/// [`Regime`](crate::regime) (the general path; supports composites). The
/// JSON `strict_no_excess` echo is `true` for any non-canonical regime.
pub fn analyze_lowered_with_regime(
    p: &LoweredPGraph,
    description: String,
    algorithm: DumpAlgorithm,
    regime: &dyn crate::regime::Regime,
    opts: AbbOptions,
) -> (PgraphAnalysisJson, Option<AbbSolution>) {
    let algo_label = match algorithm {
        DumpAlgorithm::Msg => "msg",
        DumpAlgorithm::Ssg => "ssg",
        DumpAlgorithm::Abb => "abb",
    };

    let strict_echo = regime.name() != "canonical";
    let cost_weights_echo = opts.cost_weights.clone();
    let cost_dimensions: Vec<String> = p.cost_dimensions.clone();
    let m = maximal_structure_with_regime(p, regime);
    let mut msg_units: Vec<String> = m
        .units
        .iter()
        .map(|id| p.decl_to_name[id].clone())
        .collect();
    msg_units.sort();

    // Phase 7: validate the FULL schema against both bundles. These
    // run independently of the engine selection, so they're computed
    // for every algorithm (msg / ssg / abb).
    let canonical_full = canonical_cert(p, &p.schema, &p.raws, &p.products);
    let extension_full = extension_cert(p, &p.schema, &p.raws, &p.products);

    let mut out = PgraphAnalysisJson {
        ok: true,
        description,
        algorithm: algo_label.into(),
        parse_error: None,
        lower_error: None,
        msg_units,
        ssg_structures: None,
        ssg_note: None,
        abb: None,
        canonical_full,
        extension_full,
        canonical_abb_subschema: None,
        extension_abb_subschema: None,
        strict_no_excess: strict_echo,
        cost_dimensions: cost_dimensions.clone(),
        cost_weights_echo,
        abb_cost_breakdown: None,
        abb_top_k: None,
    };

    let n = m.units.len();
    let mut abb_solution: Option<AbbSolution> = None;
    match algorithm {
        DumpAlgorithm::Msg => {}
        DumpAlgorithm::Ssg => {
            if n > 30 {
                out.ssg_note = Some(format!(
                    "MSG has {n} units (>30); SSG exponential enumeration omitted"
                ));
                out.ssg_structures = Some(vec![]);
            } else {
                let sols = enumerate_with_options(p, &m, SsgOptions::default());
                let structures: Vec<Vec<String>> = sols.iter().map(|s| unit_names(p, s)).collect();
                out.ssg_structures = Some(structures);
            }
        }
        DumpAlgorithm::Abb => {
            if let Some(abb) = solve_with_regime(p, &m, opts, regime) {
                // Phase 7: validate the ABB-selected sub-schema
                // against both bundles. Raws are restricted to
                // those that survive into the projection.
                let proj = project_subschema(p, &abb.units);
                let raws_in_proj: BTreeSet<DeclId> = p
                    .raws
                    .iter()
                    .copied()
                    .filter(|r| proj.kind(*r).is_some())
                    .collect();
                out.canonical_abb_subschema =
                    Some(canonical_cert(p, &proj, &raws_in_proj, &p.products));
                out.extension_abb_subschema =
                    Some(extension_cert(p, &proj, &raws_in_proj, &p.products));
                // Phase 10: per-dimension cost breakdown of the ABB
                // selection. Empty when the lowered graph has no
                // multi-cost dimensions.
                if !cost_dimensions.is_empty() {
                    let mut sums: Vec<f64> = vec![0.0; cost_dimensions.len()];
                    for u in &abb.units {
                        if let Some(v) = p.cost_vectors.get(u) {
                            for (i, x) in v.iter().enumerate() {
                                if i < sums.len() {
                                    sums[i] += *x;
                                }
                            }
                        }
                    }
                    out.abb_cost_breakdown =
                        Some(cost_dimensions.iter().cloned().zip(sums).collect());
                }
                out.abb = Some(abb_to_json(p, &abb));
                abb_solution = Some(abb);
            }
        }
    }

    (out, abb_solution)
}
