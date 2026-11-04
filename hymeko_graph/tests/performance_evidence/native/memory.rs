//! One-framework memory observations; construction still uses Core IrConstruction.
use super::{Case, Input, library};
use crate::{emit_phase, telemetry};
use hymeko::common::ids::{DeclId, SymId};
use hymeko::ir::ir::{
    AnnoR, ArcPlacement, IncidenceSign, Ir, IrBuilder, IrCapacities, IrConstruction,
    ReferenceDraft, dispatch,
};
use hymeko_graph::{
    PageRank, PageRankAutoSelection, PageRankModel, PageRankSolver, PageRankWorkspace,
};
use std::{
    env, fs,
    io::{self, Read, Write},
    path::Path,
};

struct MemoryInput {
    nodes: usize,
    offsets: Vec<u32>,
    members: Vec<u32>,
}
impl MemoryInput {
    fn read(mut source: impl Read) -> io::Result<Self> {
        let mut word = || {
            let mut bytes = [0; 4];
            source.read_exact(&mut bytes)?;
            Ok::<_, io::Error>(u32::from_le_bytes(bytes))
        };
        let nodes = word()? as usize;
        let arcs = word()? as usize;
        let incidences = word()? as usize;
        if nodes > 10_000_000 || arcs > 10_000_000 || incidences > 100_000_000 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "input limit"));
        }
        let offsets = (0..=arcs).map(|_| word()).collect::<io::Result<Vec<_>>>()?;
        let members = (0..incidences)
            .map(|_| word())
            .collect::<io::Result<Vec<_>>>()?;
        let mut extra = [0];
        if source.read(&mut extra)? != 0
            || offsets[0] != 0
            || *offsets.last().unwrap() as usize != incidences
            || offsets.windows(2).any(|w| w[0] > w[1])
            || members.iter().any(|&v| v as usize >= nodes)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid incidence input",
            ));
        }
        Ok(Self {
            nodes,
            offsets,
            members,
        })
    }
    fn fingerprint(&self) -> u64 {
        Input::hash(
            std::iter::once(self.nodes as u64).chain(
                self.offsets
                    .iter()
                    .chain(&self.members)
                    .map(|&v| u64::from(v)),
            ),
        )
    }
    fn build(&self) -> hymeko::ir::ir::Ir {
        IrBuilder::build(self).expect("canonical Core construction")
    }

    fn build_application_or_fixture(&self, source: Option<&Path>) -> Ir {
        match source {
            None => self.build(),
            Some(path) => {
                let input = Input::new(Input::compile_source(path));
                assert_eq!(
                    self.nodes,
                    input.ir.nodes.len(),
                    "application node count drift"
                );
                assert_eq!(self.offsets, input.offsets, "application offsets drift");
                assert_eq!(self.members, input.members, "application incidence drift");
                input.ir
            }
        }
    }
}

impl IrBuilder for &MemoryInput {
    type Output = Ir;
    type Error = dispatch::DispatchError;
    fn capacities(&self) -> IrCapacities {
        let arcs = self.offsets.len() - 1;
        IrCapacities {
            declarations: self.nodes + 2 * arcs,
            nodes: self.nodes,
            edges: arcs,
            arcs,
        }
    }
    fn populate(&mut self, construction: &mut IrConstruction<'_>) -> Result<(), Self::Error> {
        for _ in 0..self.nodes {
            construction.append_node(SymId::new(0), AnnoR::default(), vec![])?;
        }
        for bounds in self.offsets.windows(2) {
            let edge = construction.append_edge(SymId::new(0), AnnoR::default(), vec![])?;
            let references = self.members[bounds[0] as usize..bounds[1] as usize]
                .iter()
                .map(|&node| {
                    ReferenceDraft::plain(DeclId::new(node as usize), IncidenceSign::Neutral)
                });
            construction.append_arc_iter(
                ArcPlacement::under_owner(edge),
                AnnoR::default(),
                references,
            )?;
        }
        Ok(())
    }
    fn finish(self, ir: Ir) -> Ir {
        ir
    }
}

struct Phase {
    run: String,
    workload: String,
    subject: String,
}

#[derive(Clone, Copy)]
struct MemoryStrategy {
    solver: PageRankSolver,
    reuse: bool,
}
impl MemoryStrategy {
    fn select(subject: &str) -> Self {
        match subject {
            "hymeko" => Self {
                solver: PageRankSolver::default(),
                reuse: false,
            },
            "hymeko-gmres-fresh" | "hymeko-gmres-reused" => Self {
                solver: PageRankSolver::Gmres { restart: 20 },
                reuse: subject == "hymeko-gmres-reused",
            },
            "hymeko-auto-fresh" | "hymeko-auto-reused" => Self {
                solver: PageRankSolver::Auto(Default::default()),
                reuse: subject == "hymeko-auto-reused",
            },
            "hymeko-auto-quotient-fresh" | "hymeko-auto-quotient-reused" => Self {
                solver: PageRankSolver::Auto(hymeko_graph::PageRankAutoConfig {
                    quotient: hymeko_graph::PageRankQuotient::Pendant,
                    ..Default::default()
                }),
                reuse: subject.ends_with("reused"),
            },
            _ => panic!("unknown HyMeKo memory strategy"),
        }
    }

    fn solve(
        self,
        rank: &PageRank<'_>,
        workspace: &mut PageRankWorkspace,
        scores: &mut [f64],
    ) -> (usize, usize, Option<PageRankAutoSelection>) {
        let (iterations, automatic) = Input::solve_using(rank, workspace, scores, 1, self.solver);
        let retained = workspace.retained_bytes();
        if !self.reuse {
            *workspace = PageRankWorkspace::new();
        }
        (iterations, retained, automatic)
    }
}
impl Phase {
    fn mark(&self, name: &str) {
        #[cfg(feature = "evidence-heap")]
        println!(
            "EVIDENCE_HEAP\tphase={name}\trust_live_requested_b={}\trust_peak_requested_b={}",
            crate::heap::live(),
            crate::heap::peak()
        );
        emit_phase(&self.run, &self.workload, &self.subject, name);
        io::stdout().flush().unwrap();
        // Linux clear_refs=5 resets VmHWM, without clearing referenced/dirty bits.
        #[cfg(target_os = "linux")]
        fs::write("/proc/self/clear_refs", b"5\n").expect("phase peak reset");
        #[cfg(windows)]
        {
            assert_eq!(env::var("HYMEKO_MEMORY_HANDSHAKE").as_deref(), Ok("1"));
            let mut reply = String::new();
            io::stdin()
                .read_line(&mut reply)
                .expect("memory-controller checkpoint");
            assert_eq!(reply.trim(), "continue");
        }
        #[cfg(feature = "evidence-heap")]
        crate::heap::reset_peak();
    }
}

#[test]
#[ignore = "explicit isolated memory measurement requires HYMEKO_MEMORY_BENCH=1"]
fn memory_cell() {
    assert_eq!(env::var("HYMEKO_MEMORY_BENCH").as_deref(), Ok("1"));
    let phase = Phase {
        run: env::var("EVIDENCE_RUN").unwrap(),
        workload: env::var("EVIDENCE_WORKLOAD").unwrap(),
        subject: env::var("EVIDENCE_OPERATION").unwrap(),
    };
    assert!(matches!(
        phase.subject.as_str(),
        "hymeko"
            | "hymeko-gmres-fresh"
            | "hymeko-gmres-reused"
            | "hypergraphz"
            | "hymeko-auto-fresh"
            | "hymeko-auto-reused"
            | "hymeko-auto-quotient-fresh"
            | "hymeko-auto-quotient-reused"
    ));
    let library = (phase.subject == "hypergraphz").then(library::Library::load);
    phase.mark("baseline");
    let directory = env::var("HYMEKO_MEMORY_INPUT").unwrap();
    let input = MemoryInput::read(
        fs::File::open(Path::new(&directory).join(format!("{}.bin", phase.workload))).unwrap(),
    )
    .unwrap();
    let (nodes, arcs, incidences) = (input.nodes, input.offsets.len() - 1, input.members.len());
    let fingerprint = input.fingerprint();
    phase.mark("input");
    let mut scores = Vec::new();
    if let Some(library) = &library {
        let mut graph = library.build(nodes, &input.offsets, &input.members);
        phase.mark("graph_build");
        drop(input);
        phase.mark("graph_retained");
        // Upstream preparation is opaque inside rank(); expose it as combined.
        phase.mark("prepared");
        for iteration in 0..7 {
            scores = vec![0.0; nodes];
            graph.rank(&mut scores);
            phase.mark(&format!(
                "{}{}",
                if iteration < 2 { "warmup_" } else { "solve_" },
                if iteration < 2 {
                    iteration
                } else {
                    iteration - 2
                }
            ));
        }
    } else {
        #[cfg(feature = "evidence-heap")]
        let before_ir = crate::heap::live();
        let source = Input::application_source();
        let ir = input.build_application_or_fixture(source.as_deref());
        #[cfg(feature = "evidence-heap")]
        let ir_bytes = crate::heap::live() - before_ir;
        #[cfg(feature = "evidence-heap")]
        let construction_peak = crate::heap::peak().saturating_sub(before_ir);
        phase.mark("graph_build");
        drop(input);
        phase.mark("graph_retained");
        let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
        let strategy = MemoryStrategy::select(&phase.subject);
        let mut workspace = PageRankWorkspace::new();
        phase.mark("prepared");
        for iteration in 0..7 {
            scores = vec![0.0; nodes];
            let (steps, retained, automatic) = strategy.solve(&rank, &mut workspace, &mut scores);
            if let Some(auto) = automatic {
                Input::emit_auto(auto, strategy.solver, false, Some(iteration));
            }
            println!(
                "EVIDENCE_WORKSPACE\titeration={iteration}\tsolver={}\treused={}\tcapacity_b={retained}\tafter_call_b={}\toutput_b={}\tsteps={steps}",
                match strategy.solver {
                    PageRankSolver::Auto(_) => "auto",
                    PageRankSolver::Gmres { .. } => "gmres",
                    _ => "power",
                },
                strategy.reuse,
                workspace.retained_bytes(),
                scores.capacity() * size_of::<f64>()
            );
            phase.mark(&format!(
                "{}{}",
                if iteration < 2 { "warmup_" } else { "solve_" },
                if iteration < 2 {
                    iteration
                } else {
                    iteration - 2
                }
            ));
        }
        drop(workspace);
        drop(rank);
        #[cfg(feature = "evidence-heap")]
        {
            let reference_len: usize = ir
                .arcs
                .iter()
                .map(|a| a.refs.len() * size_of::<hymeko::ir::ir::SignedRefR>())
                .sum();
            let reference_capacity: usize = ir
                .arcs
                .iter()
                .map(|a| a.refs.capacity() * size_of::<hymeko::ir::ir::SignedRefR>())
                .sum();
            let before_drop = crate::heap::live();
            drop(ir);
            let released = before_drop - crate::heap::live();
            assert_eq!(
                released, ir_bytes,
                "IR heap ownership changed or unowned construction allocation"
            );
            println!(
                "EVIDENCE_IR_HEAP\tlive_requested_b={ir_bytes}\treleased_on_drop_b={released}\tconstruction_peak_extra_b={construction_peak}\treference_len_b={reference_len}\treference_capacity_b={reference_capacity}"
            );
        }
    }
    let checksum = Input::rank_hash(&scores);
    let mut file = fs::File::create(env::var("HYMEKO_MEMORY_SCORES").unwrap()).unwrap();
    for value in &scores {
        file.write_all(&value.to_le_bytes()).unwrap();
    }
    println!(
        "EVIDENCE_MEMORY\tfingerprint={fingerprint}\tinput_released=true\tframeworks=1\tverification=controller\tplatform={}\tpreparation={}",
        if cfg!(windows) { "windows" } else { "linux" },
        if library.is_some() {
            "inside_solve"
        } else {
            "separate"
        }
    );
    println!(
        "EVIDENCE_CELL\tsection=memory\tsubject={}\tworkload={}\tstatus=OK\tnodes={nodes}\tarcs={arcs}\tincidences={incidences}\tchecksum={checksum}",
        phase.subject, phase.workload
    );
    assert!(telemetry::memory_snapshot().unwrap().working_set_bytes > 0);
}

#[test]
fn compiled_application_roundtrip_retains_canonical_records_and_checks_input() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/application_rank.hymeko");
    let expected = Input::new(Input::compile_source(&source));
    let path = env::temp_dir().join(format!(
        "hymeko-application-roundtrip-{}.bin",
        std::process::id()
    ));
    expected.write_incidence(&path).unwrap();
    let mut input = MemoryInput::read(fs::File::open(&path).unwrap()).unwrap();
    fs::remove_file(&path).unwrap();
    let actual = input.build_application_or_fixture(Some(&source));
    assert_eq!(actual.decl_nodes.len(), expected.ir.decl_nodes.len());
    assert_eq!(actual.files.len(), expected.ir.files.len());
    assert!(actual.decl_nodes.len() > actual.nodes.len() + actual.edges.len());
    assert_eq!(Input::new(actual).fingerprint(), expected.fingerprint());
    assert_eq!(
        Input::new(input.build_application_or_fixture(None)).fingerprint(),
        expected.fingerprint()
    );
    input.nodes += 1;
    assert!(
        std::panic::catch_unwind(|| input.build_application_or_fixture(Some(&source))).is_err()
    );
}

#[test]
fn memory_solver_selection_preserves_scores_and_workspace_lifetime() {
    let ir = Case::unsigned(5, &[&[0, 1, 2], &[2, 3]]).build().unwrap();
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let mut expected = vec![0.0; 5];
    Input::solve(&rank, &mut PageRankWorkspace::new(), &mut expected, 1);
    for subject in [
        "hymeko",
        "hymeko-gmres-fresh",
        "hymeko-gmres-reused",
        "hymeko-auto-fresh",
        "hymeko-auto-reused",
        "hymeko-auto-quotient-fresh",
        "hymeko-auto-quotient-reused",
    ] {
        let strategy = MemoryStrategy::select(subject);
        let mut workspace = PageRankWorkspace::new();
        let mut scores = vec![0.0; 5];
        let mut previous = None;
        for _ in 0..3 {
            let (steps, capacity, automatic) = strategy.solve(&rank, &mut workspace, &mut scores);
            assert_eq!(automatic.is_some(), subject.contains("auto"));
            if let Some(auto) = automatic {
                assert_eq!(auto.power_iterations + auto.gmres_iterations, steps);
                Input::emit_auto(auto, strategy.solver, false, Some(0));
            }
            assert!(capacity > 0);
            assert_eq!(
                workspace.retained_bytes(),
                if strategy.reuse { capacity } else { 0 }
            );
            assert!(
                scores
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>()
                    < 2e-10
            );
            if let Some(old) = &previous {
                assert_eq!(&scores, old);
            }
            previous = Some(scores.clone());
        }
    }
}

#[test]
fn single_input_roundtrip_rejects_corruption_and_uses_canonical_builder() {
    let words = [4u32, 2, 4, 0, 2, 4, 0, 1, 1, 2];
    let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    let input = MemoryInput::read(bytes.as_slice()).unwrap();
    let built = Input::new(input.build());
    assert_eq!(input.fingerprint(), built.fingerprint());
    assert_eq!(built.ir.nodes.len(), 4);
    for bad in [&bytes[..bytes.len() - 1], &[0u8; 15][..]] {
        assert!(MemoryInput::read(bad).is_err());
    }
    let mut bad = bytes.clone();
    bad.extend([0]);
    assert!(MemoryInput::read(bad.as_slice()).is_err());
    let mut bad = bytes;
    bad[36..40].copy_from_slice(&9u32.to_le_bytes());
    assert!(MemoryInput::read(bad.as_slice()).is_err());
}

#[test]
fn streamed_input_matches_fixture_hierarchy_records_and_references() {
    for rows in [
        vec![],
        vec![vec![]],
        vec![vec![0, 0, 1], vec![2], vec![3, 1]],
    ] {
        let mut offsets = vec![0];
        let mut members = Vec::new();
        for row in &rows {
            members.extend(row.iter().map(|&v| v as u32));
            offsets.push(members.len() as u32);
        }
        let input = MemoryInput {
            nodes: 4,
            offsets,
            members,
        };
        let actual = input.build();
        let expected = Case::unsigned(4, &rows.iter().map(Vec::as_slice).collect::<Vec<_>>())
            .build()
            .unwrap();
        assert_eq!(actual.decl_nodes.len(), expected.decl_nodes.len());
        assert_eq!(
            (actual.nodes.len(), actual.edges.len(), actual.arcs.len()),
            (
                expected.nodes.len(),
                expected.edges.len(),
                expected.arcs.len()
            )
        );
        for (a, b) in actual.decl_nodes.iter().zip(expected.decl_nodes.iter()) {
            assert_eq!(
                (
                    a.kind,
                    a.name,
                    a.parent,
                    a.first_child,
                    a.last_child,
                    a.next_sibling,
                    a.anno
                ),
                (
                    b.kind,
                    b.name,
                    b.parent,
                    b.first_child,
                    b.last_child,
                    b.next_sibling,
                    b.anno
                )
            );
        }
        for (a, b) in actual.nodes.iter().zip(expected.nodes.iter()) {
            assert_eq!((a.decl, a.bases), (b.decl, b.bases));
        }
        for (a, b) in actual.edges.iter().zip(expected.edges.iter()) {
            assert_eq!((a.decl, a.bases, &a.arcs), (b.decl, b.bases, &b.arcs));
        }
        for (a, b) in actual.arcs.iter().zip(expected.arcs.iter()) {
            assert_eq!((a.anno, a.in_edge, &a.refs), (b.anno, b.in_edge, &b.refs));
            assert_eq!(a.refs.len(), a.refs.capacity());
        }
    }
}
