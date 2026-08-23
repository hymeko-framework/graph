//! `pgraph` — command-line driver for the P-graph engine, working over the
//! underlying structures (meta-model resolution → `LoweredPGraph` → MSG/SSG/ABB).
//!
//! ```text
//! pgraph read      <file>
//! pgraph transform <file>
//! pgraph solve     <file> [--algorithm msg|ssg|abb|all] [--relaxed]
//!                         [--weights "w1,..,wD"] [--json]
//! pgraph generate  <file> [--format dot|pgip] [--out PATH]
//! ```
//!
//! Input is `.hymeko` (meta-model or literal-tag) or `.pgip`, auto-detected by
//! [`hymeko_pgraph::cli::load_pgraph`]. Exit codes: 0 ok; 2 load/parse failure;
//! 1 usage / IO.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use hymeko_pgraph::abb::solve_with_options;
use hymeko_pgraph::cli::{
    load_pgraph, render_entities, render_pgraph, render_solution, to_dot, to_friedler_dot,
};
use hymeko_pgraph::dump::{DumpAlgorithm, analyze_lowered_with_full_options};
use hymeko_pgraph::{
    AbbOptions, LoweredPGraph, MaximalStructureOptions, maximal_structure, write_pgip,
};

const USAGE: &str = "\
pgraph — P-graph CLI

USAGE:
    pgraph <command> <file> [options]

COMMANDS:
    read       report the materials and operating units declared in the file
    transform  show the bipartite P-graph (M/O partition + signed incidence)
    solve      run MSG, SSG, and ABB and report the cost-optimal structure
    generate   emit a graph artifact (Graphviz DOT, or a P-graph Studio .pgip)

SOLVE OPTIONS:
    --algorithm msg|ssg|abb   focus the --json emission (default: abb)
    --strict-no-excess        non-canonical no-waste filter (MSG + ABB); default off
    --relaxed                 deprecated no-op (canonical/relaxed is the default)
    --weights \"w1,..,wD\"      multi-objective ABB weights
    --json                    emit machine-readable JSON instead of text

GENERATE OPTIONS:
    --format dot|png|svg|pgip artifact kind (default: dot). png/svg need Graphviz.
    --out PATH                write to PATH (dot prints to stdout if omitted;
                              png/svg/pgip require --out)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return ExitCode::from(1);
    };
    if matches!(cmd, "-h" | "--help" | "help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let Some(file) = args.get(1).map(PathBuf::from) else {
        eprintln!("error: `{cmd}` needs a <file> argument\n");
        eprint!("{USAGE}");
        return ExitCode::from(1);
    };
    let rest = &args[2..];
    match cmd {
        "read" => run_render(&file, rest, render_entities),
        "transform" => run_render(&file, rest, render_pgraph),
        "solve" => run_solve(&file, rest),
        "generate" => run_generate(&file, rest),
        other => {
            eprintln!("error: unknown command `{other}` (read|transform|solve|generate)\n");
            eprint!("{USAGE}");
            ExitCode::from(1)
        }
    }
}

fn title_of(file: &Path) -> String {
    file.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("pgraph")
        .to_string()
}

fn load_or_report(file: &Path) -> Result<LoweredPGraph, ExitCode> {
    load_pgraph(file).map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::from(2)
    })
}

fn run_render(
    file: &Path,
    rest: &[String],
    render: fn(&LoweredPGraph, &str) -> String,
) -> ExitCode {
    if !rest.is_empty() {
        eprintln!("warning: ignoring unexpected arguments: {rest:?}");
    }
    match load_or_report(file) {
        Ok(g) => {
            print!("{}", render(&g, &title_of(file)));
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn run_solve(file: &Path, rest: &[String]) -> ExitCode {
    let mut algorithm = DumpAlgorithm::Abb;
    // Canonical (book) semantics by default (2026-05-27, Pimentel report).
    let mut strict_no_excess = false;
    let mut json = false;
    let mut weights: Option<Vec<f64>> = None;

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--strict-no-excess" => {
                strict_no_excess = true;
                i += 1;
            }
            "--relaxed" => {
                // Deprecated no-op: relaxed (canonical) is now the default.
                i += 1;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            "--algorithm" => match rest.get(i + 1).map(|s| DumpAlgorithm::from_str(s.trim())) {
                Some(Ok(a)) => {
                    algorithm = a;
                    i += 2;
                }
                Some(Err(e)) => return arg_error(&e),
                None => return arg_error("--algorithm requires a value"),
            },
            "--weights" => match rest.get(i + 1).map(|s| parse_weights(s)) {
                Some(Ok(w)) => {
                    weights = Some(w);
                    i += 2;
                }
                Some(Err(e)) => return arg_error(&e),
                None => return arg_error("--weights requires a comma-separated list"),
            },
            other => return arg_error(&format!("unexpected argument `{other}`")),
        }
    }

    let g = match load_or_report(file) {
        Ok(g) => g,
        Err(code) => return code,
    };
    let msg_opts = MaximalStructureOptions { strict_no_excess };
    let abb_opts = AbbOptions {
        cost_weights: weights,
        strict_no_excess,
        ..AbbOptions::default()
    };
    if json {
        let (out, _) =
            analyze_lowered_with_full_options(&g, title_of(file), algorithm, msg_opts, abb_opts);
        match serde_json::to_string_pretty(&out) {
            Ok(s) => println!("{s}"),
            Err(e) => return arg_error(&format!("json: {e}")),
        }
    } else {
        print!(
            "{}",
            render_solution(&g, &title_of(file), msg_opts, abb_opts)
        );
    }
    ExitCode::SUCCESS
}

fn run_generate(file: &Path, rest: &[String]) -> ExitCode {
    let mut format = String::from("dot");
    let mut out: Option<PathBuf> = None;
    // 2026-06-03: `default` is the existing debug-style colour-coded
    // rendering. `friedler` uses canonical PSE notation (circle M-nodes,
    // bar O-nodes, no fill colours beyond raw/product distinction).
    let mut style = String::from("default");
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--format" => match rest.get(i + 1) {
                Some(v) => {
                    format = v.clone();
                    i += 2;
                }
                None => return arg_error("--format requires a value (dot|pgip)"),
            },
            "--out" => match rest.get(i + 1) {
                Some(v) => {
                    out = Some(PathBuf::from(v));
                    i += 2;
                }
                None => return arg_error("--out requires a path"),
            },
            "--style" => match rest.get(i + 1) {
                Some(v) => {
                    style = v.clone();
                    i += 2;
                }
                None => return arg_error("--style requires a value (default|friedler)"),
            },
            other => return arg_error(&format!("unexpected argument `{other}`")),
        }
    }

    let g = match load_or_report(file) {
        Ok(g) => g,
        Err(code) => return code,
    };
    let render_dot = |g: &_, title: &str| match style.as_str() {
        "default" => to_dot(g, title),
        "friedler" | "pse" => to_friedler_dot(g, title),
        other => panic!("unknown style {other}"),
    };
    if !matches!(style.as_str(), "default" | "friedler" | "pse") {
        return arg_error(&format!(
            "unknown --style `{style}` (default|friedler|pse)"
        ));
    }
    match format.as_str() {
        "dot" => {
            let dot = render_dot(&g, &title_of(file));
            match &out {
                Some(p) => match std::fs::write(p, dot) {
                    Ok(()) => eprintln!("wrote DOT to {}", p.display()),
                    Err(e) => return arg_error(&format!("write {}: {e}", p.display())),
                },
                None => print!("{dot}"),
            }
            ExitCode::SUCCESS
        }
        "png" | "svg" | "pdf" => {
            let Some(p) = out else {
                return arg_error(&format!("--format {format} requires --out PATH"));
            };
            let dot = render_dot(&g, &title_of(file));
            match hymeko_pgraph::render_graphviz(&dot, &format, &p) {
                Ok(()) => {
                    eprintln!("wrote {format} to {}", p.display());
                    ExitCode::SUCCESS
                }
                Err(e) => arg_error(&format!("{e}")),
            }
        }
        "pgip" => {
            let Some(p) = out else {
                return arg_error("--format pgip requires --out PATH");
            };
            // Bake the ABB optimum into the generated .pgip.
            let msg = maximal_structure(&g);
            let sol = solve_with_options(&g, &msg, AbbOptions::default());
            match write_pgip(&g, &p, sol.as_ref()) {
                Ok(()) => {
                    eprintln!("wrote .pgip to {}", p.display());
                    ExitCode::SUCCESS
                }
                Err(e) => arg_error(&format!("write_pgip {}: {e}", p.display())),
            }
        }
        other => arg_error(&format!("unknown --format `{other}` (dot|png|svg|pgip)")),
    }
}

fn parse_weights(s: &str) -> Result<Vec<f64>, String> {
    let v: Result<Vec<f64>, _> = s
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|e| format!("--weights {t:?}: {e}"))
        })
        .collect();
    let v = v?;
    if v.is_empty() {
        return Err("--weights needs at least one value".into());
    }
    if v.iter().any(|w| *w < 0.0) {
        return Err("--weights entries must be non-negative".into());
    }
    Ok(v)
}

fn arg_error(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::from(1)
}
