import copy
import csv
import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from campaign import (
    COMPARISON_WORKLOADS,
    PHASES,
    Campaign,
    CampaignSummary,
    Cell,
    Evidence,
    ProcessRunner,
)


class EvidenceContract(unittest.TestCase):
    def assert_rejected_mutations(self, source, replacements):
        for old, new in replacements:
            with self.subTest(replacement=(old, new)), self.assertRaises(ValueError):
                Evidence.parse(source.replace(old, new))

    def test_combinatorial_workloads_and_construction_are_explicit(self):
        cells = Cell.select("traversal", workloads=["steiner255", "mycielski7"],
                            subjects=["generate_ir", "incidence_index", "levi"])
        self.assertEqual(len(cells), 6)
        self.assertTrue(all(c.test == "ranking::ranking_cell" and c.environment(1)["HYMEKO_RANK_BENCH"] == "1" for c in cells))
        self.assertEqual(len(Cell.matrix("traversal")), 16)
        self.assertEqual(len(Cell.select("comparison", workloads=["sunflower1024"],
                                         subjects=["components_hymeko", "components_hypergraphz"])), 2)

    def construction_fixture(self):
        source = self.fixture().replace("EVIDENCE_CELL\t", "EVIDENCE_CELL\tsection=construction\tsubject=generate_ir\tworkload=steiner255\tnodes=255\tarcs=10795\tincidences=32385\t")
        source += "\nEVIDENCE_INPUT\tfingerprint=123\tconstruction_timed=true\tindex_timed=false\tmetric_scope=core_generator_excluding_drop"
        source += "\nEVIDENCE_CONSTRUCTION\tchecksum=123\tverified=true\tdestruction_timed=false\tfull_validation=retained_result"
        return source

    def test_construction_and_algebra_boundaries_fail_closed(self):
        source = self.construction_fixture()
        Evidence.parse(source)
        self.assert_rejected_mutations(source, [
            ("construction_timed=true", "construction_timed=false"),
            ("destruction_timed=false", "destruction_timed=true"),
            ("verified=true", "verified=false"), ("fingerprint=123", "fingerprint=456")])
        source = source.replace("section=construction", "section=algebra").replace("subject=generate_ir", "subject=s_components2").replace("construction_timed=true", "construction_timed=false")
        source = "\n".join(line for line in source.splitlines() if not line.startswith("EVIDENCE_CONSTRUCTION"))
        source += "\nEVIDENCE_ALGEBRA\tchecksum=123\tverified=true\tvalue=10795\tpair_work=2040255\tmax_pair_work=5000000"
        Evidence.parse(source)
        self.assert_rejected_mutations(source, [
            ("pair_work=2040255", "pair_work=5000001"), ("value=10795", "value=10796")])

    def test_quiet_guard_checkpoints_blockage_without_starting_work(self):
        from host import HostBusy
        with tempfile.TemporaryDirectory() as directory:
            campaign = Campaign.__new__(Campaign)
            campaign.output = Path(directory)
            campaign.guard = Mock()
            campaign.guard.check.side_effect = HostBusy({"human_uids": [99]}, ["another user"])
            state = {"cells": []}
            with self.assertRaises(SystemExit) as failed: campaign.admit(state)
            self.assertEqual(failed.exception.code, 75)
            saved = json.loads((Path(directory) / "checkpoint.json").read_text())
            self.assertEqual(saved["cells"], [])
            self.assertEqual(saved["blocked"]["reasons"], ["another user"])
            campaign.guard.check.side_effect = None
            campaign.guard.check.return_value = {"quiet": True}
            self.assertEqual(campaign.admit(state), {"quiet": True})
            self.assertNotIn("blocked", state)
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint({"quiet_guard": {"cpus": [2]}}, {"quiet_guard": {"cpus": [3]}, "cells": []})

    def test_manifest_campaign_runs_without_claiming_a_git_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            args = SimpleNamespace(suite="traversal", algorithm=__file__, platform="native",
                cpu=None, output=directory, source_ref=None, source_manifest="frozen.json",
                runs=1, smoke=False, phases=None, subjects=["generate_ir"],
                workloads=["steiner255"], resume=False)
            def invoke(_suite, _arguments, _environment, log):
                log.write_text(self.construction_fixture(), encoding="utf-8")
                return {"exit_code": 0, "peak_rss_bytes": 1000, "wall_seconds": 1.0}
            check_output = subprocess.check_output
            def reject_git(command, *values, **options):
                if command[0] == "git":
                    raise AssertionError("snapshot must not query Git")
                return check_output(command, *values, **options)
            with patch.object(Campaign, "verify_sources", return_value={"sha256": "a" * 64}), \
                    patch.object(ProcessRunner, "invoke", side_effect=invoke), \
                    patch("campaign.subprocess.check_output", side_effect=reject_git):
                Campaign(args).run()
            result = json.loads((Path(directory) / "campaign.json").read_text())
            self.assertIsNone(result["source_commit"])
            self.assertIsNone(result["driver_commit"])
            self.assertEqual(len(result["cells"]), 1)
            self.assertTrue(result["complete"])

    def test_worker_cpu_sets_validate_and_preserve_single_cpu_options(self):
        args = SimpleNamespace(suite="comparison", algorithm=__file__, platform="native", cpu=2, cpus=None)
        runner = ProcessRunner(args)
        self.assertEqual(runner.cpu_set(), [2])
        args.cpus = [5, 2, 4, 3]
        self.assertEqual(runner.cpu_set(), [2, 3, 4, 5])
        for bad in ([], [2, 2], [-1], [True], ["2"]):
            args.cpus = bad
            with self.assertRaises(ValueError):
                runner.cpu_set()
        args.cpus = None
        args.cpu = None
        self.assertIsNone(runner.cpu_set())

    def test_worker_affinity_checks_threads_and_rejects_escape(self):
        with patch("campaign.Path.iterdir", return_value=[Path("101"), Path("102")]), \
                patch("campaign.os.sched_getaffinity", create=True, return_value={2, 3}):
            self.assertEqual(ProcessRunner.verify_thread_affinity(100, [2, 3], 0), 2)
            with self.assertRaises(RuntimeError):
                ProcessRunner.verify_thread_affinity(100, [2], 0)
        with patch("campaign.Path.iterdir", side_effect=FileNotFoundError):
            self.assertEqual(ProcessRunner.verify_thread_affinity(100, [2], 3), 3)

    def query_fixture(self, workload="chain65k", subject="dijkstra"):
        kind = {"reachable": "nodes", "dual_reachable": "arcs", "berge_path": "path", "dijkstra": "path"}[subject]
        source = self.fixture().replace("EVIDENCE_CELL\t", f"EVIDENCE_CELL\tsection=traversal\tsubject={subject}\tworkload={workload}\tnodes=6\tarcs=5\tincidences=10\t")
        source += "\nEVIDENCE_INPUT\tfingerprint=456\tconstruction_timed=false\tmetric_scope=prepared_index_reused_workspace_query_result_drop"
        source += f"\nEVIDENCE_QUERY\tstart=0\tstart_kind={'arc' if kind == 'arcs' else 'node'}\tgoal_node={'5' if kind == 'path' else 'none'}\tkind={kind}\tvisited={5 if kind == 'arcs' else 6}\treachable=true\thops={5 if kind == 'path' else 0}\tcost={5 if kind == 'path' else 0}\tchecksum=123\tflow=undirected\tweights=unit"
        return source

    def test_traversal_matrix_reuses_opt_in_client_and_explicit_workloads(self):
        cells = Cell.matrix("traversal")
        self.assertEqual(len(cells), 16)
        self.assertEqual({c.test for c in cells}, {"ranking::ranking_cell"})
        self.assertTrue(all(c.environment(1)["HYMEKO_RANK_BENCH"] == "1" for c in cells))
        selected = Cell.select("traversal", workloads=["mixed_1m", "chain65k"], subjects=["dijkstra"], smoke=True)
        self.assertEqual([c.workload for c in selected], ["mixed_1m", "chain65k"])
        self.assertEqual(len(Cell.matrix("ranking")), 12)
        for bad in ([], ["missing"], ["chain65k", "chain65k"]):
            with self.assertRaises(ValueError):
                Cell.select("traversal", workloads=bad)

    def test_query_contract_rejects_wrong_domains_costs_and_missing_results(self):
        for subject in ("reachable", "dual_reachable", "berge_path", "dijkstra"):
            Evidence.parse(self.query_fixture(subject=subject))
        source = self.query_fixture()
        for old, new in (("cost=5", "cost=4"), ("cost=5", "cost=NaN"),
                         ("visited=6", "visited=7"), ("goal_node=5", "goal_node=6"),
                         ("start_kind=node", "start_kind=arc"), ("reachable=true", "reachable=false")):
            with self.assertRaises(ValueError):
                Evidence.parse(source.replace(old, new))
        with self.assertRaises(ValueError):
            Evidence.parse("\n".join(line for line in source.splitlines() if not line.startswith("EVIDENCE_QUERY")))

    def test_traversal_summary_preserves_complete_work_and_query_results(self):
        cells = []
        for run in range(1, 6):
            for cell in Cell.matrix("traversal"):
                cells.append({"id": cell.key, "run": run,
                              "records": Evidence.parse(self.query_fixture(cell.workload, cell.subject)),
                              "resources": {"exit_code": 0, "peak_rss_bytes": 1000, "wall_seconds": 1.0}})
        campaign = {"complete": True, "smoke": False, "suite": "traversal", "runs": 5,
                    "source_commit": "c" * 40, "platform": "windows", "cells": cells}
        summary = CampaignSummary(Path("unused"))
        summary.accept(campaign)
        self.assertEqual(len(summary.rows), 16)
        self.assertTrue(all(row["samples"] == 45 and row["query_reachable"] == "true" for row in summary.rows))
        broken = copy.deepcopy(campaign)
        broken["cells"][-1]["records"]["QUERY"][0]["hops"] = "4"
        with self.assertRaises(ValueError):
            CampaignSummary(Path("unused")).accept(broken)

    def native_fixture(self, workload="rank32_1m", subject="rank_hymeko"):
        source = self.fixture().replace(
            "EVIDENCE_CELL\t",
            f"EVIDENCE_CELL\tsection=native\tsubject={subject}\tworkload={workload}\tnodes=3\tarcs=2\tincidences=4\t",
        )
        source += "\nEVIDENCE_INPUT\tfingerprint=456\tconstruction_timed=false\tmetric_scope=rank_prepare_allocate_solve_copy_drop\tshape_reported=true"
        source += "\nEVIDENCE_SHAPE\tarity_min=2\tarity_q1=2\tarity_median=2\tarity_q3=2\tarity_max=2\tdegree_min=1\tdegree_q1=1\tdegree_median=1\tdegree_q3=2\tdegree_max=2\tisolated_nodes=0\tdegree_sum=4"
        source += "\nEVIDENCE_NATIVE\toperation=rank\tparity=true\tchecksum=123\tvector_l1=1e-15\thymeko_bound=1e-11\thypergraphz_bound=1e-11\thymeko_iterations=2\thypergraphz_iterations=2\tpersonalization=uniform\ttolerance=1e-10"
        return source

    def test_workload_selection_is_explicit_and_historical_matrix_is_preserved(self):
        self.assertEqual(len(Cell.matrix("comparison")), 15)
        self.assertEqual(len(Cell.matrix("comparison", workloads=list(COMPARISON_WORKLOADS))), 45)
        chosen = ["mixed_1m", "scale_large"]
        cells = Cell.select("comparison", subjects=["rank_hymeko"], smoke=True, workloads=chosen)
        self.assertEqual([c.workload for c in cells], chosen)
        for bad in ([], ["missing"], ["mixed_1m", "mixed_1m"]):
            with self.assertRaises(ValueError):
                Cell.select("comparison", workloads=bad)
        with self.assertRaises(ValueError):
            Cell.select("graph", workloads=chosen)
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint({"workloads": chosen}, {"workloads": None})

    def test_expanded_summary_preserves_selected_cells_and_shape_fields(self):
        subjects = ["rank_hymeko", "rank_hypergraphz"]
        cells = []
        for repeat in range(1, 6):
            for cell in Cell.select("comparison", subjects=subjects, workloads=list(COMPARISON_WORKLOADS)):
                cells.append({"id": cell.key, "run": repeat,
                              "records": Evidence.parse(self.native_fixture(cell.workload, cell.subject)),
                              "resources": {"exit_code": 0, "peak_rss_bytes": 1000, "wall_seconds": 1.0}})
        campaign = {"complete": True, "smoke": False, "suite": "comparison", "runs": 5,
                    "source_commit": "c" * 40, "platform": "windows", "cells": cells,
                    "workloads": list(COMPARISON_WORKLOADS), "subjects": subjects}
        summary = CampaignSummary(Path("unused"))
        summary.accept(campaign)
        self.assertEqual(len(summary.rows), 18)
        self.assertTrue(all(r["samples"] == 45 and r["arity_max"] == "2" and r["isolated_nodes"] == "0" for r in summary.rows))
        broken = copy.deepcopy(campaign)
        broken["workloads"].pop()
        with self.assertRaises(ValueError):
            CampaignSummary(Path("unused")).accept(broken)

    def test_shape_metadata_rejects_missing_nonfinite_or_inconsistent_counts(self):
        source = self.native_fixture()
        Evidence.parse(source)
        for old, new in [("degree_sum=4", "degree_sum=5"), ("arity_max=2", "arity_max=4"),
                         ("degree_median=1", "degree_median=NaN"), ("isolated_nodes=0", "isolated_nodes=1")]:
            with self.assertRaises(ValueError):
                Evidence.parse(source.replace(old, new))
        no_shape = "\n".join(line for line in source.splitlines() if not line.startswith("EVIDENCE_SHAPE"))
        with self.assertRaises(ValueError):
            Evidence.parse(no_shape)
        Evidence.parse(no_shape.replace("\tshape_reported=true", ""))

    def test_subject_selection_is_explicit_validated_and_part_of_checkpoint_identity(self):
        subjects = ["rank_hymeko", "rank_hypergraphz", "rank_prepared"]
        selected = Cell.select("comparison", subjects=subjects)
        self.assertEqual(len(selected), 9)
        self.assertEqual({c.subject for c in selected}, set(subjects))
        self.assertTrue(all(c in Cell.matrix("comparison") for c in selected))
        for invalid in ([], ["rank_hymeko", "rank_hymeko"], ["missing"]):
            with self.assertRaises(ValueError):
                Cell.select("comparison", subjects=invalid)
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint({"subjects": subjects}, {"subjects": None})

    def test_selected_smoke_never_duplicates_a_single_cell_or_silently_omits_the_subject(self):
        cells = Cell.select("graph", subjects=["paths_all"], smoke=True)
        self.assertEqual(len(cells), 1)
        self.assertEqual(cells[0].subject, "paths_all")
        cells = Cell.select("parsing", subjects=["ast_to_ir"], smoke=True)
        self.assertEqual(len(cells), 2)
        self.assertEqual({c.subject for c in cells}, {"ast_to_ir"})
        cells = Cell.select("comparison", subjects=["rank_prepared"], smoke=True)
        self.assertEqual([(c.workload, c.subject) for c in cells], [("rank32_1m", "rank_prepared")])

    def test_native_comparison_requires_full_parity_and_matching_tolerance(self):
        source = self.native_fixture()
        Evidence.parse(source)
        for old, new in (("parity=true", "parity=false"), ("vector_l1=1e-15", "vector_l1=NaN"),
                         ("hypergraphz_bound=1e-11", "hypergraphz_bound=1e-5"), ("tolerance=1e-10", "tolerance=1e-6")):
            with self.assertRaises(ValueError):
                Evidence.parse(source.replace(old, new))
        cells = Cell.matrix("comparison")
        self.assertEqual(len(cells), 15)
        self.assertTrue(all(c.environment(1)["HYMEKO_NATIVE_BENCH"] == "1" for c in cells))

    def test_ranking_summary_preserves_five_processes_shapes_and_convergence(self):
        cells = []
        for run in range(1, 6):
            for cell in Cell.matrix("ranking"):
                output = self.fixture().replace(
                    "EVIDENCE_CELL\t", f"EVIDENCE_CELL\tsection=pagerank\tsubject={cell.subject}\tworkload={cell.workload}\tnodes=65536\tarcs=31250\tincidences=1000000\t"
                )
                output += "\nEVIDENCE_INPUT\tfingerprint=123\tconstruction_timed=false"
                if cell.subject != "reachable":
                    output += "\nEVIDENCE_RANK\tconverged=true\terror_bound_l1=1e-12\ttolerance=1e-10\tmass=1.0\tscore_hash=123\titerations=12"
                else:
                    output += "\nEVIDENCE_REACHABLE\tvisited=65536"
                cells.append({"run": run, "id": cell.key, "records": Evidence.parse(output),
                              "resources": {"exit_code": 0, "peak_rss_bytes": 1000, "wall_seconds": 1.0}})
        campaign = {"complete": True, "smoke": False, "suite": "ranking", "runs": 5,
                    "source_commit": "a" * 40, "platform": "windows", "cells": cells}
        summary = CampaignSummary(Path("unused"))
        summary.accept(campaign)
        self.assertEqual(len(summary.rows), 12)
        self.assertTrue(all(row["samples"] == 45 and row["processes"] == 5 for row in summary.rows))
        self.assertTrue(all(row["process_median_ns"] == 5000 for row in summary.rows))
        self.assertTrue(all(row["incidences"] == "1000000" for row in summary.rows))
        self.assertTrue(all(row["iterations"] == "12" for row in summary.rows if row["algorithm"] == "solve"))
        selected = copy.deepcopy(campaign)
        selected["subjects"] = ["solve"]
        selected["cells"] = [c for c in selected["cells"] if c["records"]["CELL"][0]["subject"] == "solve"]
        selected_summary = CampaignSummary(Path("unused"))
        selected_summary.accept(selected)
        self.assertEqual(len(selected_summary.rows), 4)
        with self.assertRaises(ValueError):
            summary.accept(selected)
        selected["subjects"] = ["prepare", "solve"]
        with self.assertRaises(ValueError):
            CampaignSummary(Path("unused")).accept(selected)
        broken = copy.deepcopy(campaign)
        broken["cells"][0]["run"] = 2
        with self.assertRaises(ValueError):
            CampaignSummary(Path("unused")).accept(broken)

    def test_ranking_matrix_is_explicit_and_separate_from_construction(self):
        cells = Cell.matrix("ranking")
        self.assertEqual(len(cells), 12)
        self.assertEqual({cell.subject for cell in cells}, {"prepare", "solve", "reachable"})
        self.assertTrue(all(cell.test == "ranking::ranking_cell" for cell in cells))
        self.assertTrue(all(cell.environment(1)["HYMEKO_RANK_BENCH"] == "1" for cell in cells))
        self.assertTrue(all(cell.environment(1)["RAYON_NUM_THREADS"] == "1" for cell in cells))

    def test_local_source_manifest_rejects_changed_missing_and_added_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            source = root / "model.rs"
            source.write_text("fn model() {}", encoding="utf8")
            saved = Campaign.capture_sources({"graph": root})
            manifest = root / "snapshot.json"
            manifest.write_text(json.dumps(saved), encoding="utf8")
            identity = Campaign.verify_sources(root, manifest)
            self.assertEqual(identity["files"], 1)
            source.write_text("fn changed() {}", encoding="utf8")
            with self.assertRaises(ValueError):
                Campaign.verify_sources(root, manifest)
            source.write_text("fn model() {}", encoding="utf8")
            added = root / "added.rs"
            added.write_text("struct Added;", encoding="utf8")
            with self.assertRaises(ValueError):
                Campaign.verify_sources(root, manifest)
            added.unlink()
            source.unlink()
            with self.assertRaises(ValueError):
                Campaign.verify_sources(root, manifest)

    def test_ranking_evidence_requires_actual_convergence_and_full_vector_identity(self):
        source = self.fixture().replace("EVIDENCE_CELL\t", "EVIDENCE_CELL\tsection=pagerank\tsubject=solve\t")
        source += "\nEVIDENCE_INPUT\tfingerprint=456\tconstruction_timed=false"
        source += "\nEVIDENCE_RANK\tconverged=true\terror_bound_l1=1e-12\ttolerance=1e-10\tmass=1.0\tscore_hash=123"
        self.assertEqual(Evidence.parse(source)["RANK"][0]["score_hash"], "123")
        for mutated in (
            source.replace("converged=true", "converged=false"),
            source.replace("error_bound_l1=1e-12", "error_bound_l1=NaN"),
            source.replace("mass=1.0", "mass=0.5"),
            source.replace("score_hash=123", "score_hash=789"),
            source.replace("construction_timed=false", "construction_timed=true"),
        ):
            with self.assertRaises(ValueError):
                Evidence.parse(mutated)

    def test_unpacked_snapshot_verifies_without_git_and_rejects_added_or_changed_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "model.rs"
            source.write_text("fn model() {}", encoding="utf8")
            saved = Campaign.capture_sources({"graph": root}, inventory="snapshot")
            manifest = root / "snapshot.json"
            manifest.write_text(json.dumps(saved), encoding="utf8")
            self.assertEqual(Campaign.verify_sources(root, manifest)["files"], 1)
            source.write_text("fn changed() {}", encoding="utf8")
            with self.assertRaises(ValueError):
                Campaign.verify_sources(root, manifest)
            source.write_text("fn model() {}", encoding="utf8")
            added = root / "added.rs"
            added.write_text("struct Added;", encoding="utf8")
            with self.assertRaises(ValueError):
                Campaign.verify_sources(root, manifest)
            added.unlink()
            source.unlink()
            with self.assertRaises(ValueError):
                Campaign.verify_sources(root, manifest)

    def test_summary_publishes_measured_boundaries_and_exact_hashes(self):
        original = (
            Path(__file__).resolve().parents[2]
            / "reports/cross-platform-benchmarks/2026-09-06"
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for platform in ("windows", "native"):
                (root / platform).mkdir()
                (root / platform / "campaign.json").write_bytes(
                    (original / platform / "campaign.json").read_bytes()
                )
            summary = CampaignSummary(root)
            (root / "stages").mkdir()
            (root / "stages" / "independent-evidence.json").write_text("{}")
            summary.write(("windows", "native"))
            manifest = json.loads((root / "artifacts-manifest.json").read_text())
            self.assertNotIn("stages/independent-evidence.json", manifest)
            for filename, expected in manifest.items():
                self.assertEqual(
                    hashlib.sha256((root / filename).read_bytes()).hexdigest(), expected
                )
            with (root / "platform-summary.csv").open(newline="") as stream:
                rows = list(csv.DictReader(stream))
            self.assertEqual(
                {r["phase"] for r in rows if r["algorithm"] == "discrete_hops"},
                {"preparation"},
            )
            self.assertTrue(all(r["measurement_scope"] for r in rows))
            self.assertEqual({r["platform"] for r in rows}, {"windows", "native"})
            self.assertTrue((root / "search-platforms.svg").is_file())

    def test_phase_breakdown_is_explicit_and_file_boundaries_are_required(self):
        phases = [p[0] for p in PHASES]
        self.assertEqual(len(phases), 10)
        self.assertEqual(len(Cell.matrix("parsing", phases)), 160)
        with self.assertRaises(ValueError):
            Cell.matrix("parsing", ["unknown"])
        source = self.fixture().replace(
            "EVIDENCE_CELL\t",
            "EVIDENCE_CELL\tsection=parsing\tsubject=file_to_dijkstra\t",
        )
        source += "\nEVIDENCE_SOURCE\tbytes=100\tfnv1a64=123\tprojection=graph_fixture_v1\tfile_io=true\tfile_cache=warm"
        self.assertEqual(Evidence.parse(source)["SOURCE"][0]["file_cache"], "warm")
        for changed in (
            source.replace("file_io=true", "file_io=false"),
            source.replace("file_cache=warm", "file_cache=cold"),
        ):
            with self.assertRaises(ValueError):
                Evidence.parse(changed)

    def test_checkpoint_is_atomic_and_rejects_changed_provenance(self):
        current = {
            "source_commit": "a" * 40,
            "binaries": {"informed": {"sha256": "b" * 64}},
            "suite": "parsing",
            "platform": "windows",
            "runtime": {},
            "runs": 3,
            "smoke": False,
            "cells": [],
        }
        saved = copy.deepcopy(current)
        saved["cells"] = [{"run": 1, "id": "parse"}]
        self.assertEqual(Campaign.resume_checkpoint(current, saved), saved)
        for key in (
            "source_commit",
            "binaries",
            "suite",
            "platform",
            "runtime",
            "runs",
            "smoke",
        ):
            changed = copy.deepcopy(saved)
            changed[key] = "changed"
            with self.assertRaises(ValueError):
                Campaign.resume_checkpoint(current, changed)
        with tempfile.TemporaryDirectory() as directory:
            campaign = Campaign.__new__(Campaign)
            campaign.output = Path(directory)
            campaign.save_checkpoint(saved)
            self.assertEqual(
                json.loads((campaign.output / "checkpoint.json").read_text()), saved
            )
            self.assertFalse((campaign.output / "checkpoint.pending.json").exists())
        saved["cells"] *= 2
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint(current, saved)

    def fixture(self):
        rows = [
            f"EVIDENCE_SAMPLE\tordinal={i}\twall_us={i + 1}\tchecksum=123"
            for i in range(9)
        ]
        rows.append(
            "EVIDENCE_CELL\tstatus=OK\tsamples=9\tchecksum=123\twall_q1_us=3\twall_median_us=5\twall_q3_us=7\twall_worst_us=9"
        )
        return "\n".join(rows)

    def test_accepts_matching_raw_and_summary(self):
        self.assertEqual(len(Evidence.parse(self.fixture())["SAMPLE"]), 9)

    def test_rejects_drift_missing_duplicate_and_fabricated_summary(self):
        source = self.fixture()
        for changed in (
            source.replace("checksum=123", "checksum=999", 1),
            "\n".join(source.splitlines()[1:]),
            source.replace("ordinal=1", "ordinal=0"),
            source.replace("wall_median_us=5", "wall_median_us=6"),
            source.replace("wall_us=1", "wall_us=nan"),
            source + "\n" + source.splitlines()[-1],
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                Evidence.parse(changed)

    def test_matrix_has_complete_existing_search_suite(self):
        cells = Cell.matrix()
        self.assertEqual(sum(c.suite == "informed" for c in cells), 48)
        self.assertEqual(len(cells), len({c.key for c in cells}))
        self.assertEqual(
            {c.threads for c in cells if c.subject == "independent_query_batch"},
            {1, 2, 4},
        )

    def test_parsing_covers_every_existing_source_topology_and_phase(self):
        matrix = Cell.matrix("parsing")
        originals = {
            c.workload if not c.workload.startswith("grid200_d") else "grid200_d80"
            for c in Cell.matrix()
        }
        self.assertEqual({c.workload for c in matrix}, originals)
        self.assertEqual(len(matrix), 5 * len(originals))
        selected = Cell.matrix("parsing", ["parse_ast"])
        self.assertEqual({c.workload for c in selected}, originals)
        self.assertTrue(all(c.subject == "parse_ast" for c in selected))
        changed = {
            "source_commit": "a",
            "binaries": {},
            "suite": "parsing",
            "platform": "arm64",
            "runtime": {},
            "runs": 3,
            "smoke": False,
            "phases": ["parse_ast"],
            "cells": [],
        }
        with self.assertRaises(ValueError):
            Campaign.resume_checkpoint(changed, {**changed, "phases": ["compile"]})
        self.assertTrue(all(":" not in c.key for c in matrix))
        with self.assertRaises(ValueError):
            Cell.matrix("unknown")

    def test_parsing_requires_source_identity_and_positive_bytes(self):
        source = self.fixture().replace(
            "EVIDENCE_CELL\t", "EVIDENCE_CELL\tsection=parsing\t"
        )
        valid = (
            source
            + "\nEVIDENCE_SOURCE\tbytes=100\tfnv1a64=123\tprojection=graph_fixture_v1"
        )
        self.assertEqual(Evidence.parse(valid)["SOURCE"][0]["bytes"], "100")
        for invalid in (
            source,
            valid.replace("bytes=100", "bytes=0"),
            valid.replace("fnv1a64=123", "fnv1a64=456"),
        ):
            with self.assertRaises(ValueError):
                Evidence.parse(invalid)

    def test_archived_campaign_rejects_corrupted_provenance_and_process_identity(self):
        root = (
            Path(__file__).resolve().parents[2]
            / "reports/cross-platform-benchmarks/2026-09-06"
        )
        original = json.loads(
            (root / "windows/campaign.json").read_text(encoding="utf8")
        )
        for fault in ("source", "checksum", "repetition", "missing"):
            summary = CampaignSummary(root)
            summary.accept(original)
            changed = copy.deepcopy(original)
            if fault == "source":
                changed["source_commit"] = "0" * 40
            elif fault == "checksum":
                changed["cells"][0]["records"]["CELL"][0]["checksum"] = "0"
            elif fault == "repetition":
                changed["cells"][0]["run"] = 2
            else:
                changed["cells"].pop()
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                summary.accept(changed)


if __name__ == "__main__":
    unittest.main()
