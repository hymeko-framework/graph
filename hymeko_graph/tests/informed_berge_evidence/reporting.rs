//! Typed identity and result rows for the existing TSV evidence protocol.
use super::statistics::Measurement;
use super::{bound_key, telemetry};
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{BergePath, BudgetKind, EnumBudget, SearchStats, WeightedBergePath};
use std::io::{self, Write};

#[derive(Clone, Copy)]
pub(super) struct EvidenceCell<'a> {
    pub run: &'a str,
    pub section: &'a str,
    pub workload: &'a str,
    pub subject: &'a str,
}

pub(super) enum ResultSummary {
    Path {
        reachable: bool,
        cost: Option<f64>,
        hops: Option<usize>,
        hash: u64,
    },
    Items {
        count: usize,
        bound: Option<BudgetKind>,
        budget: Option<EnumBudget>,
        hash: u64,
    },
}
impl ResultSummary {
    pub fn weighted(path: &Option<WeightedBergePath>) -> Self {
        Self::Path {
            reachable: path.is_some(),
            cost: path.as_ref().map(|p| p.total_cost),
            hops: path.as_ref().map(|p| p.hyperedge_len()),
            hash: path.as_ref().map_or(0, |p| p.path_hash().raw()),
        }
    }
    pub fn shortest(path: &Option<BergePath>, hash: u64) -> Self {
        Self::Path {
            reachable: path.is_some(),
            cost: None,
            hops: path.as_ref().map(|p| p.hyperedge_len()),
            hash,
        }
    }
}
pub(super) fn dimensions(ir: &Ir) -> (usize, usize, usize) {
    (
        ir.nodes.len(),
        ir.arcs.len(),
        ir.arcs.iter().map(|arc| arc.refs.len()).sum(),
    )
}
fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "NA".to_string(), |value| format!("{value:.3}"))
}
impl EvidenceCell<'_> {
    pub fn phase(&self, phase: &str) {
        let Self {
            run,
            workload,
            subject,
            ..
        } = *self;

        match telemetry::memory_snapshot() {
            Some(memory) => println!(
                "EVIDENCE_PHASE\trun={run}\tworkload={workload}\tsubject={subject}\tphase={phase}\tworking_set_b={}\tself_peak_b={}",
                memory.working_set_bytes, memory.peak_working_set_bytes
            ),
            None => println!(
                "EVIDENCE_PHASE\trun={run}\tworkload={workload}\tsubject={subject}\tphase={phase}\tworking_set_b=NA\tself_peak_b=NA"
            ),
        }
    }

    pub fn measurement(&self, ir: &Ir, measurement: &Measurement) {
        let Self {
            run,
            section,
            workload,
            subject,
        } = *self;

        let (nodes, arcs, incidences) = dimensions(ir);
        for sample in &measurement.samples {
            println!(
                "EVIDENCE_SAMPLE\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\tordinal={}\trepetitions={}\twall_us={:.3}\tcpu_us={}\tchecksum={}",
                sample.ordinal,
                measurement.repetitions_per_sample,
                sample.wall_us,
                optional(sample.cpu_us),
                sample.checksum
            );
        }
        println!(
            "EVIDENCE_CELL\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\tstatus=OK\tnodes={nodes}\tarcs={arcs}\tincidences={incidences}\twarmups={}\tsamples={}\trepetitions={}\twall_median_us={:.3}\twall_q1_us={:.3}\twall_q3_us={:.3}\twall_worst_us={:.3}\tcpu_median_us={}\tcpu_q1_us={}\tcpu_q3_us={}\tcpu_worst_us={}\tchecksum={}",
            measurement.warmups,
            measurement.samples.len(),
            measurement.repetitions_per_sample,
            measurement.wall.median,
            measurement.wall.q1,
            measurement.wall.q3,
            measurement.wall.worst,
            optional(measurement.cpu.map(|distribution| distribution.median)),
            optional(measurement.cpu.map(|distribution| distribution.q1)),
            optional(measurement.cpu.map(|distribution| distribution.q3)),
            optional(measurement.cpu.map(|distribution| distribution.worst)),
            measurement.checksum
        );
    }
    pub fn work(&self, stats: SearchStats) {
        let Self {
            run,
            workload,
            subject,
            ..
        } = *self;

        println!(
            "EVIDENCE_WORK\trun={run}\tworkload={workload}\tsubject={subject}\texpanded_nodes={}\texpanded_arcs={}\tresolved_incidences={}\tpushes={}\tpops={}\tstale_pops={}\theuristic_evals={}\treopened={}",
            stats.expanded_nodes,
            stats.expanded_arcs,
            stats.resolved_incidences,
            stats.pushes,
            stats.pops,
            stats.stale_pops,
            stats.heuristic_evals,
            stats.reopened
        );
    }
    pub fn result(&self, result: &ResultSummary) {
        self.write_result(&mut io::stdout().lock(), result)
            .expect("write evidence result");
    }

    pub fn write_result(&self, writer: &mut impl Write, result: &ResultSummary) -> io::Result<()> {
        let Self {
            run,
            section,
            workload,
            subject,
        } = *self;
        let (reachable, cost, hops, path_hash, count, bound, budget) = match *result {
            ResultSummary::Path {
                reachable,
                cost,
                hops,
                hash,
            } => (
                if reachable { "true" } else { "false" },
                cost,
                hops,
                hash,
                None,
                "NA",
                None,
            ),
            ResultSummary::Items {
                count,
                bound,
                budget,
                hash,
            } => (
                "NA",
                None,
                None,
                hash,
                Some(count),
                bound_key(bound),
                budget,
            ),
        };
        let cost_micros =
            cost.map_or_else(|| "NA".to_string(), |c| format!("{:.0}", c * 1_000_000.0));
        let hops = hops.map_or_else(|| "NA".to_string(), |h| h.to_string());
        let items = count.map_or_else(|| "NA".to_string(), |n| n.to_string());
        let (max_depth, max_results, max_expansions) = budget.map_or_else(
            || ("NA".to_string(), "NA".to_string(), "NA".to_string()),
            |b| {
                (
                    b.max_depth.to_string(),
                    b.max_results.to_string(),
                    b.max_expansions.to_string(),
                )
            },
        );
        writeln!(
            writer,
            "EVIDENCE_RESULT\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\treachable={reachable}\tcost_micros={cost_micros}\thops={hops}\tpath_hash={path_hash}\titems={items}\tbound={bound}\tmax_depth={max_depth}\tmax_results={max_results}\tmax_expansions={max_expansions}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(result: ResultSummary) -> String {
        let cell = EvidenceCell {
            run: "r",
            section: "s",
            workload: "w",
            subject: "q",
        };
        let mut bytes = Vec::new();
        cell.write_result(&mut bytes, &result).unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn path_rows_preserve_keys_units_order_and_missing_values() {
        assert_eq!(
            line(ResultSummary::Path {
                reachable: true,
                cost: Some(1.25),
                hops: Some(2),
                hash: 17
            }),
            "EVIDENCE_RESULT\trun=r\tsection=s\tworkload=w\tsubject=q\treachable=true\tcost_micros=1250000\thops=2\tpath_hash=17\titems=NA\tbound=NA\tmax_depth=NA\tmax_results=NA\tmax_expansions=NA\n"
        );
        assert_eq!(
            line(ResultSummary::weighted(&None)),
            "EVIDENCE_RESULT\trun=r\tsection=s\tworkload=w\tsubject=q\treachable=false\tcost_micros=NA\thops=NA\tpath_hash=0\titems=NA\tbound=NA\tmax_depth=NA\tmax_results=NA\tmax_expansions=NA\n"
        );
        assert_eq!(
            line(ResultSummary::shortest(&None, 0)),
            line(ResultSummary::weighted(&None))
        );
    }

    #[test]
    fn real_weighted_and_unweighted_results_keep_cost_presence_and_hashes() {
        use hymeko::common::ids::NodeId;
        use hymeko::ir::incidence::build_incidence_index;
        use hymeko_graph::direct_ir::{
            HyperTraversal, InformedSearchWorkspace, TraversalWorkspace, Undirected, UnitCost,
            dijkstra_berge_path,
        };
        let ir = super::super::workloads::window_ir(
            3,
            2,
            false,
            super::super::workloads::SignMode::AllPlus,
        );
        let index = build_incidence_index(&ir).unwrap();
        let traversal = HyperTraversal::new(&index);
        let path = dijkstra_berge_path(
            &traversal,
            NodeId::new(0),
            NodeId::new(2),
            &UnitCost,
            &Undirected,
            &mut InformedSearchWorkspace::new(),
        )
        .unwrap();
        let hash = path.as_ref().unwrap().path_hash().raw();
        assert_eq!(
            line(ResultSummary::weighted(&path)),
            format!(
                "EVIDENCE_RESULT\trun=r\tsection=s\tworkload=w\tsubject=q\treachable=true\tcost_micros=2000000\thops=2\tpath_hash={hash}\titems=NA\tbound=NA\tmax_depth=NA\tmax_results=NA\tmax_expansions=NA\n"
            )
        );
        let path = traversal
            .berge_shortest_path(
                &mut TraversalWorkspace::new(),
                NodeId::new(0),
                NodeId::new(2),
            )
            .unwrap();
        let hash = super::super::hash_berge_path(path.as_ref().unwrap());
        assert_eq!(
            line(ResultSummary::shortest(&path, hash)),
            format!(
                "EVIDENCE_RESULT\trun=r\tsection=s\tworkload=w\tsubject=q\treachable=true\tcost_micros=NA\thops=2\tpath_hash={hash}\titems=NA\tbound=NA\tmax_depth=NA\tmax_results=NA\tmax_expansions=NA\n"
            )
        );
    }

    #[test]
    fn enumeration_and_preparation_rows_keep_distinct_budget_columns() {
        for (bound, label) in [
            (None, "none"),
            (Some(BudgetKind::Depth), "depth"),
            (Some(BudgetKind::Results), "results"),
            (Some(BudgetKind::Expansions), "expansions"),
        ] {
            assert_eq!(
                line(ResultSummary::Items {
                    count: 16,
                    bound,
                    budget: Some(EnumBudget::new(5, 100, 10_000)),
                    hash: 23
                }),
                format!(
                    "EVIDENCE_RESULT\trun=r\tsection=s\tworkload=w\tsubject=q\treachable=NA\tcost_micros=NA\thops=NA\tpath_hash=23\titems=16\tbound={label}\tmax_depth=5\tmax_results=100\tmax_expansions=10000\n"
                )
            );
        }
        assert_eq!(
            line(ResultSummary::Items {
                count: 8,
                bound: None,
                budget: None,
                hash: 31
            }),
            "EVIDENCE_RESULT\trun=r\tsection=s\tworkload=w\tsubject=q\treachable=NA\tcost_micros=NA\thops=NA\tpath_hash=31\titems=8\tbound=none\tmax_depth=NA\tmax_results=NA\tmax_expansions=NA\n"
        );
    }

    #[test]
    fn output_errors_are_propagated() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected write failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cell = EvidenceCell {
            run: "r",
            section: "s",
            workload: "w",
            subject: "q",
        };
        assert!(
            cell.write_result(&mut Broken, &ResultSummary::weighted(&None))
                .is_err()
        );
    }
}
