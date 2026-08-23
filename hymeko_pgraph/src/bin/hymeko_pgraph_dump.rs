//! CLI: dump MSG / SSG / ABB analysis for a P-graph file.
//!
//! Accepts `.hymeko` source or `.pgip` (P-graph Studio SQLite) input,
//! auto-detected by file extension.
//!
//! ```text
//! hymeko_pgraph_dump path/to/graph.{hymeko|pgip}
//!     [--algorithm msg|ssg|abb]
//!     [--weights "w1,w2,...,wD"]
//!     [--strict-no-excess]   (non-canonical no-waste filter; default off)
//!     [--regime SPEC]        (canonical|no-excess|cost-dominance, '+'-joined,
//!                             e.g. --regime cost-dominance+no-excess)
//!     [--relaxed-msg]        (deprecated no-op; canonical is the default)
//!     [--write-pgip path/to/out.pgip]
//! ```
//!
//! Writes one JSON document to stdout (pretty-printed). Exit code 0 on
//! success, 2 when parse/lower fails, 1 on usage / IO errors.
//!
//! Stage P-mo (2026-05-19): `--weights` for multi-objective ABB.
//! Stage P-io (2026-05-19): `.pgip` direct read; `--write-pgip` emits
//! a P-graph Studio-loadable file with the ABB result baked in.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use hymeko_pgraph::abb::AbbOptions;
use hymeko_pgraph::regime::{CANONICAL, COST_DOMINANCE, Composite, NO_EXCESS, Regime};
use hymeko_pgraph::{
    DumpAlgorithm, SsgAlgorithm,
    analyze_lowered_with_regime, analyze_lowered_with_regime_full,
    analyze_lowered_with_regime_topk,
    analyze_source_with_regime, analyze_source_with_regime_full,
    analyze_source_with_regime_topk,
    read_pgip, write_pgip,
};

/// Parse a `--regime` spec: one or more regime names joined by `+`
/// (e.g. `cost-dominance+no-excess`). Returns the component strategies.
fn parse_regime_spec(spec: &str) -> Result<Vec<&'static dyn Regime>, String> {
    spec.split('+')
        .map(|name| match name.trim() {
            "canonical" => Ok(&CANONICAL as &dyn Regime),
            "no-excess" => Ok(&NO_EXCESS as &dyn Regime),
            "cost-dominance" => Ok(&COST_DOMINANCE as &dyn Regime),
            other => Err(format!(
                "unknown regime '{other}' (expected canonical | no-excess | cost-dominance, '+'-joined)"
            )),
        })
        .collect()
}

fn parse_weights(s: &str) -> Result<Vec<f64>, String> {
    let v: Result<Vec<f64>, _> = s
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|e| format!("--weights entry {t:?}: {e}"))
        })
        .collect();
    let v = v?;
    if v.is_empty() {
        return Err("--weights requires at least one comma-separated value".into());
    }
    if v.iter().any(|w| *w < 0.0) {
        return Err(
            "--weights entries must be non-negative (admissibility of the inclusion bound)".into(),
        );
    }
    Ok(v)
}

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).map(|s| s.to_string()).collect();
    if argv.is_empty() {
        eprintln!(
            "usage: hymeko_pgraph_dump <file.hymeko> [--algorithm msg|ssg|abb] [--weights \"w1,w2,...\"]"
        );
        return std::process::ExitCode::from(1u8);
    }
    let path = argv[0].clone();
    let mut algorithm = DumpAlgorithm::Ssg;
    let mut cost_weights: Option<Vec<f64>> = None;
    // Canonical (book) semantics by default (2026-05-27, Pimentel report):
    // no no-excess rule. Opt into the non-canonical no-waste filter with
    // `--strict-no-excess`. `--relaxed-msg` is kept as a no-op (the relaxed
    // regime is now the default).
    let mut strict_no_excess = false;
    let mut regime_spec: Option<String> = None;
    let mut write_pgip_path: Option<PathBuf> = None;
    // 2026-06-03 (Pimentel benchmark): expose top-K ABB so the CLI
    // returns the 2nd-best / 3rd-best alternatives, not just the
    // single optimum. `top_k <= 1` keeps the legacy single-best
    // JSON schema unchanged.
    let mut top_k: usize = 1;
    // 2026-06-03 (Pimentel benchmark): SSG algorithm choice.
    // `brute` (default, back-compat) enumerates the 2^|O_MSG| subset
    // lattice via forward-DFS feasibility. `decision-mapping` (or `dm`)
    // uses the canonical Friedler 1992 SSG that matches the book's
    // published structure counts.
    let mut ssg_algorithm: SsgAlgorithm = SsgAlgorithm::Brute;
    let mut i = 1usize;
    while i < argv.len() {
        if argv[i] == "--algorithm" {
            if i + 1 >= argv.len() {
                eprintln!("--algorithm requires a value");
                return std::process::ExitCode::from(1u8);
            }
            match DumpAlgorithm::from_str(argv[i + 1].trim()) {
                Ok(x) => algorithm = x,
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 2;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--algorithm=") {
            match DumpAlgorithm::from_str(rest.trim()) {
                Ok(x) => algorithm = x,
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 1;
            continue;
        }
        if argv[i] == "--weights" {
            if i + 1 >= argv.len() {
                eprintln!("--weights requires a comma-separated list");
                return std::process::ExitCode::from(1u8);
            }
            match parse_weights(&argv[i + 1]) {
                Ok(v) => cost_weights = Some(v),
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 2;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--weights=") {
            match parse_weights(rest) {
                Ok(v) => cost_weights = Some(v),
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 1;
            continue;
        }
        if argv[i] == "--strict-no-excess" {
            strict_no_excess = true;
            i += 1;
            continue;
        }
        if argv[i] == "--regime" {
            if i + 1 >= argv.len() {
                eprintln!("--regime requires a value (e.g. cost-dominance+no-excess)");
                return std::process::ExitCode::from(1u8);
            }
            regime_spec = Some(argv[i + 1].clone());
            i += 2;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--regime=") {
            regime_spec = Some(rest.to_string());
            i += 1;
            continue;
        }
        if argv[i] == "--relaxed-msg" {
            // Deprecated no-op: relaxed (canonical) is now the default.
            i += 1;
            continue;
        }
        if argv[i] == "--top-k" {
            if i + 1 >= argv.len() {
                eprintln!("--top-k requires a positive integer");
                return std::process::ExitCode::from(1u8);
            }
            match argv[i + 1].parse::<usize>() {
                Ok(k) if k >= 1 => top_k = k,
                _ => {
                    eprintln!("--top-k: expected a positive integer, got {:?}", argv[i + 1]);
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 2;
            continue;
        }
        if argv[i] == "--ssg-algorithm" {
            if i + 1 >= argv.len() {
                eprintln!("--ssg-algorithm requires a value (brute | decision-mapping | dm)");
                return std::process::ExitCode::from(1u8);
            }
            match SsgAlgorithm::from_str(argv[i + 1].trim()) {
                Ok(s) => ssg_algorithm = s,
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 2;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--ssg-algorithm=") {
            match SsgAlgorithm::from_str(rest.trim()) {
                Ok(s) => ssg_algorithm = s,
                Err(e) => {
                    eprintln!("{e}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 1;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--top-k=") {
            match rest.parse::<usize>() {
                Ok(k) if k >= 1 => top_k = k,
                _ => {
                    eprintln!("--top-k: expected a positive integer, got {rest:?}");
                    return std::process::ExitCode::from(1u8);
                }
            }
            i += 1;
            continue;
        }
        if argv[i] == "--write-pgip" {
            if i + 1 >= argv.len() {
                eprintln!("--write-pgip requires a path argument");
                return std::process::ExitCode::from(1u8);
            }
            write_pgip_path = Some(PathBuf::from(&argv[i + 1]));
            i += 2;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--write-pgip=") {
            write_pgip_path = Some(PathBuf::from(rest));
            i += 1;
            continue;
        }
        eprintln!("unexpected argument: {}", argv[i]);
        return std::process::ExitCode::from(1u8);
    }
    let opts = AbbOptions {
        cost_weights,
        strict_no_excess,
        ..AbbOptions::default()
    };

    // Build the solving regime. `--regime <spec>` (one or more of
    // canonical|no-excess|cost-dominance, '+'-joined) takes precedence;
    // otherwise the `--strict-no-excess` flag picks no-excess vs canonical.
    // A single component is used directly (so its name echoes correctly);
    // multiple components compose into a `Composite`.
    let components: Vec<&'static dyn Regime> = match &regime_spec {
        Some(spec) => match parse_regime_spec(spec) {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => {
                eprintln!("--regime: empty spec");
                return std::process::ExitCode::from(1u8);
            }
            Err(e) => {
                eprintln!("{e}");
                return std::process::ExitCode::from(1u8);
            }
        },
        None => vec![if strict_no_excess {
            &NO_EXCESS
        } else {
            &CANONICAL
        }],
    };
    let composite = Composite::new(components.clone());
    let regime: &dyn Regime = if components.len() == 1 {
        components[0]
    } else {
        &composite
    };

    // Auto-detect input format by extension. `.pgip` reads the
    // SQLite directly; anything else is treated as `.hymeko` source.
    let path_buf = PathBuf::from(&path);
    let is_pgip = path_buf
        .extension()
        .map(|e| e.eq_ignore_ascii_case("pgip"))
        .unwrap_or(false);

    let (out, abb_solution, graph_for_write) = if is_pgip {
        // Stage P-io: direct .pgip read.
        let graph = match read_pgip(Path::new(&path)) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("read_pgip {path}: {e}");
                return std::process::ExitCode::from(1u8);
            }
        };
        let description = path_buf
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("pgip_input")
            .to_string();
        let need_full = top_k > 1
            || matches!(ssg_algorithm, SsgAlgorithm::DecisionMapping);
        let (json, abb) = if need_full {
            analyze_lowered_with_regime_full(
                &graph, description, algorithm, regime, opts, top_k, ssg_algorithm,
            )
        } else {
            analyze_lowered_with_regime(&graph, description, algorithm, regime, opts)
        };
        (json, abb, Some(graph))
    } else {
        let src = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("read {path}: {e}");
                return std::process::ExitCode::from(1u8);
            }
        };
        // For --write-pgip we also need the lowered graph; re-parse on
        // demand to keep the back-compat shim hot-path unchanged.
        if write_pgip_path.is_some() {
            let desc = match parser::parse_description(&src) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("parse for --write-pgip: {e:?}");
                    return std::process::ExitCode::from(1u8);
                }
            };
            let description = desc.name.to_string();
            let p = match hymeko_pgraph::lower(&desc) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("lower for --write-pgip: {e}");
                    return std::process::ExitCode::from(1u8);
                }
            };
            let need_full = top_k > 1
                || matches!(ssg_algorithm, SsgAlgorithm::DecisionMapping);
            let (json, abb) = if need_full {
                analyze_lowered_with_regime_full(
                    &p, description, algorithm, regime, opts, top_k, ssg_algorithm,
                )
            } else {
                analyze_lowered_with_regime(&p, description, algorithm, regime, opts)
            };
            (json, abb, Some(p))
        } else {
            let need_full = top_k > 1
                || matches!(ssg_algorithm, SsgAlgorithm::DecisionMapping);
            let json = if need_full {
                analyze_source_with_regime_full(
                    &src, algorithm, regime, opts, top_k, ssg_algorithm,
                )
            } else {
                analyze_source_with_regime(&src, algorithm, regime, opts)
            };
            (json, None, None)
        }
    };
    match serde_json::to_string_pretty(&out) {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("json: {e}");
            return std::process::ExitCode::from(1u8);
        }
    }

    // Stage P-io: --write-pgip emits a P-graph-Studio-loadable file.
    if let Some(out_path) = write_pgip_path {
        let graph = match graph_for_write {
            Some(ref g) => g,
            None => {
                // Should not happen: we only set write_pgip_path on
                // input paths that produce graph_for_write.
                eprintln!("--write-pgip: internal — no lowered graph available");
                return std::process::ExitCode::from(1u8);
            }
        };
        if let Err(e) = write_pgip(graph, &out_path, abb_solution.as_ref()) {
            eprintln!("write_pgip {}: {e}", out_path.display());
            return std::process::ExitCode::from(1u8);
        }
        eprintln!("wrote {}", out_path.display());
    }

    if out.ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(2u8)
    }
}
