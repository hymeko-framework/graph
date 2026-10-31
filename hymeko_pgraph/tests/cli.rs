//! Tests for the `pgraph` CLI library surface ([`hymeko_pgraph::cli`]).
//! The binary itself is a thin dispatcher; all logic is here and tested
//! directly.

use std::path::PathBuf;

use hymeko_pgraph::cli::{
    CliError, load_pgraph, render_entities, render_graphviz, render_pgraph, render_solution, to_dot,
};
use hymeko_pgraph::{AbbOptions, MaximalStructureOptions};

fn pgraph_data(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn repo_data(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(rel)
}

#[test]
fn load_routes_meta_model_file() {
    // Meta-model style (include + using + <isa>) → compile_to_lowered.
    let g = load_pgraph(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    assert_eq!(g.materials.len(), 7);
    assert_eq!(g.units.len(), 5);
    assert_eq!(g.raws.len(), 2);
    assert_eq!(g.products.len(), 1);
}

#[test]
fn load_falls_back_to_literal_tag_file() {
    // hda.hymeko has no pgraph meta archetypes → literal lower() fallback.
    let g = load_pgraph(&repo_data("data/pgraph/hda.hymeko")).unwrap();
    assert!(!g.materials.is_empty());
    assert!(!g.units.is_empty());
}

#[test]
fn load_missing_file_errors() {
    match load_pgraph(&pgraph_data("data/does_not_exist.hymeko")) {
        Err(CliError::Io(_)) => {}
        other => panic!("expected Io error, got {other:?}"),
    }
}

#[test]
fn constrained_source_binds_core_digest_and_refuses_unbounded_or_foreign_requests() {
    use hymeko::{
        module_store::{execution::CompiledModel, source_provider::StdFsProvider},
        util::real_parser::RealParser,
    };
    use hymeko_pgraph::cli::{SsgRequest, constrained_source};
    use hymeko_pgraph::ssg_dm::SsgConstraints;
    let path = pgraph_data("data/prgraph_ex_3_1.hymeko");
    let model = CompiledModel::compile(StdFsProvider::new(), RealParser, &path).unwrap();
    let digest: String = model
        .semantic_digest()
        .0
        .0
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut request = SsgRequest {
        provenance: None,
        model_digest: digest,
        max_structures: 100,
        constraints: SsgConstraints {
            max_explored: 1000,
            ..Default::default()
        },
    };
    let result = constrained_source(&path, &request).unwrap();
    assert!(!result["structures"].as_array().unwrap().is_empty());
    assert_eq!(result["model_digest"], request.model_digest);
    assert_eq!(result["capped"], false);
    request.constraints.max_explored = 0;
    assert!(constrained_source(&path, &request).is_err());
    request.constraints.max_explored = 1000;
    request.model_digest = "f".repeat(64);
    assert!(constrained_source(&path, &request).is_err());
}

#[test]
fn render_entities_lists_roles_and_io() {
    let g = load_pgraph(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    let out = render_entities(&g, "ex");
    assert!(out.contains("materials (7)"));
    assert!(out.contains("operating units (5)"));
    assert!(out.contains("[raw]"));
    assert!(out.contains("[product]"));
    // u2 consumes F and produces D, E.
    assert!(out.contains("u2"));
    assert!(out.contains("in: F"));
}

#[test]
fn render_pgraph_shows_signed_incidence() {
    let g = load_pgraph(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    let out = render_pgraph(&g, "ex");
    assert!(out.contains("M-nodes (7)"));
    assert!(out.contains("O-nodes (5)"));
    assert!(out.contains("consumed"));
    assert!(out.contains("produced"));
}

#[test]
fn render_solution_reports_msg_and_abb() {
    let g = load_pgraph(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    let out = render_solution(
        &g,
        "ex",
        MaximalStructureOptions::default(),
        AbbOptions::default(),
    );
    // F is unproduced ⇒ MSG keeps {u1,u4,u5}; ABB reaches a finite optimum.
    assert!(out.contains("MSG"));
    assert!(out.contains("u1") && out.contains("u4") && out.contains("u5"));
    assert!(!out.contains("u2")); // pruned
    assert!(out.contains("ABB") && out.contains("cost"));
}

#[test]
fn render_graphviz_makes_png_or_errors_clearly() {
    // Deterministic regardless of environment: if Graphviz `dot` is installed,
    // we get a real PNG (magic bytes); if not, a clear, actionable error.
    let g = load_pgraph(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    let dot = to_dot(&g, "ex");
    let out = std::env::temp_dir().join("hymeko_pgraph_cli_test.png");
    let _ = std::fs::remove_file(&out);
    match render_graphviz(&dot, "png", &out) {
        Ok(()) => {
            let bytes = std::fs::read(&out).unwrap();
            assert!(
                bytes.starts_with(&[0x89, b'P', b'N', b'G']),
                "expected PNG magic bytes"
            );
            let _ = std::fs::remove_file(&out);
        }
        Err(CliError::Render(msg)) => {
            let m = msg.to_lowercase();
            assert!(
                m.contains("graphviz") || m.contains("dot"),
                "render error should name graphviz/dot: {msg}"
            );
        }
        Err(other) => panic!("unexpected error variant: {other:?}"),
    }
}

#[test]
fn to_dot_is_valid_digraph_with_edges() {
    let g = load_pgraph(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    let dot = to_dot(&g, "ex");
    assert!(dot.starts_with("digraph"));
    assert!(dot.contains("rankdir=LR"));
    assert!(dot.contains("\"u1\"")); // a unit node
    assert!(dot.contains("->")); // at least one incidence edge
    assert!(dot.trim_end().ends_with('}'));
}
