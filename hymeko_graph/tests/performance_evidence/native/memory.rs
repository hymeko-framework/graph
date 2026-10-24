//! One-framework memory observations; construction still uses Core IrConstruction.
use super::{Case, Input, library};
use crate::{emit_phase, telemetry};
use hymeko::ir::ir::IrBuilder;
use hymeko_graph::{PageRank, PageRankModel, PageRankWorkspace};
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
        let rows: Vec<Vec<usize>> = self
            .offsets
            .windows(2)
            .map(|w| {
                self.members[w[0] as usize..w[1] as usize]
                    .iter()
                    .map(|&v| v as usize)
                    .collect()
            })
            .collect();
        Case::unsigned(
            self.nodes,
            &rows.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        )
        .build()
        .expect("canonical Core construction")
    }
}

struct Phase {
    run: String,
    workload: String,
    subject: String,
}
impl Phase {
    fn mark(&self, name: &str) {
        emit_phase(&self.run, &self.workload, &self.subject, name);
        io::stdout().flush().unwrap();
        // Linux clear_refs=5 resets VmHWM, without clearing referenced/dirty bits.
        fs::write("/proc/self/clear_refs", b"5\n").expect("phase peak reset");
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
    assert!(matches!(phase.subject.as_str(), "hymeko" | "hypergraphz"));
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
        let ir = input.build();
        phase.mark("graph_build");
        drop(input);
        phase.mark("graph_retained");
        let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
        phase.mark("prepared");
        for iteration in 0..7 {
            scores = vec![0.0; nodes];
            Input::solve(&rank, &mut PageRankWorkspace::new(), &mut scores, 1);
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
    }
    let checksum = Input::rank_hash(&scores);
    let mut file = fs::File::create(env::var("HYMEKO_MEMORY_SCORES").unwrap()).unwrap();
    for value in &scores {
        file.write_all(&value.to_le_bytes()).unwrap();
    }
    println!(
        "EVIDENCE_MEMORY\tfingerprint={fingerprint}\tinput_released=true\tframeworks=1\tverification=controller\tpreparation={}",
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
