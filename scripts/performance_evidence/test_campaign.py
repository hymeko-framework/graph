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
    def test_solver_summary_names_the_solver_contract_without_parsing_or_qemu_claims(self):
        cell=Cell.select('ranking_solvers',workloads=['rank2_1m'],subjects=['rank_hymeko'])[0]
        campaign={'complete':True,'smoke':False,'runs':1,'suite':'ranking_solvers',
                  'platform':'native','source_commit':'a'*40,'workloads':['rank2_1m'],
                  'subjects':['rank_hymeko'],'cells':[dict(id=cell.key,run=1,
                      records=Evidence.parse(self.native_fixture('rank2_1m')),
                      resources={'exit_code':0,'peak_rss_bytes':1000,'wall_seconds':1.})]}
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'native').mkdir()
            (root/'native/campaign.json').write_text(json.dumps(campaign))
            CampaignSummary(root).write(('native',))
            provenance=json.loads((root/'source-provenance.json').read_text())
            self.assertIn('matrix-free GMRES',provenance['measurement_contract'])
            self.assertNotIn('File phases',provenance['measurement_contract'])
            svg=(root/'search-platforms.svg').read_text()
            self.assertIn('PageRank solver strategies',svg)
            self.assertNotIn('QEMU',svg)

    def test_solver_matrix_separates_structural_power_ordered_power_and_krylov(self):
        self.assert_matrix_sizes('ranking_solvers', {None:48, 'rank2_1m':6})
        cells=Cell.select('ranking_solvers',workloads=['chain65k','chain65k@17'],subjects=['rank_ordered','rank_gmres'])
        self.assertEqual(len(cells),4)
        self.assertTrue(all(c.environment(1)['HYMEKO_NATIVE_BENCH']=='1' for c in cells))
        self.assertTrue(all(c.environment(1)['RAYON_NUM_THREADS']=='1' for c in cells))
        with self.assertRaises(ValueError): Cell.select('ranking_solvers',workloads=['steiner255'])

    def test_auto_selection_requires_timed_policy_and_consistent_execution_evidence(self):
        cell = Cell.select('ranking_solvers', workloads=['rank2_1m'], subjects=['rank_auto'])[0]
        self.assertEqual(cell.environment(1)['HYMEKO_NATIVE_BENCH'], '1')
        records = Evidence.parse(self.native_fixture('rank2_1m'))
        records['CELL'][0]['subject'] = 'rank_auto'
        records['AUTO'] = [dict(policy='periodic_power_probe', selection_timed='true',
            reason='PowerConverged', probe_interval='4', restart_limit='20', max_basis_bytes='67108864',
            iteration_limit='200', probe_updates='2', selection_checks='1', power_updates='2', gmres_updates='0',
            restart='0', basis_bytes='none', predicted_power_updates='0', power_work='0', gmres_work='0')]
        Evidence.verify_native(records, records['CELL'][0])
        for key, value in [('selection_timed', 'false'), ('power_updates', '1'),
                           ('reason', 'KrylovEstimate'), ('restart_limit', '21')]:
            broken = copy.deepcopy(records); broken['AUTO'][0][key] = value
            with self.assertRaises(ValueError): Evidence.verify_native(broken, broken['CELL'][0])
        missing = copy.deepcopy(records); del missing['AUTO']
        with self.assertRaises(ValueError): Evidence.verify_native(missing, missing['CELL'][0])
        records['NATIVE'][0]['hymeko_iterations'] = '15'
        records['AUTO'][0].update(reason='KrylovEstimate', probe_updates='4', power_updates='12',
            gmres_updates='3', restart='3', basis_bytes='72', predicted_power_updates='30',
            power_work='10000', gmres_work='5000')
        Evidence.verify_native(records, records['CELL'][0])
        records['AUTO'][0]['basis_bytes'] = '80'
        with self.assertRaises(ValueError): Evidence.verify_native(records, records['CELL'][0])
        records['AUTO'][0]['basis_bytes'] = '67108865'
        with self.assertRaises(ValueError): Evidence.verify_native(records, records['CELL'][0])

    def test_auto_summary_retains_selection_and_rejects_mixed_process_decisions(self):
        cell = Cell.select('ranking_solvers', workloads=['rank2_1m'], subjects=['rank_auto'])[0]
        source = self.native_fixture('rank2_1m', 'rank_auto')
        source += ('\nEVIDENCE_AUTO\tpolicy=periodic_power_probe\tselection_timed=true'
                   '\treason=PowerConverged\tprobe_interval=4\trestart_limit=20\tmax_basis_bytes=67108864'
                   '\titeration_limit=200\tprobe_updates=2\tselection_checks=1\tpower_updates=2\tgmres_updates=0'
                   '\trestart=0\tbasis_bytes=none\tpredicted_power_updates=0\tpower_work=0\tgmres_work=0')
        campaign = dict(complete=True, smoke=False, runs=2, suite='ranking_solvers',
            platform='native', source_commit='a'*40, workloads=['rank2_1m'], subjects=['rank_auto'],
            cells=[dict(id=cell.key, run=run, records=Evidence.parse(source),
                        resources=dict(exit_code=0, peak_rss_bytes=1000, wall_seconds=1.)) for run in (1, 2)])
        summary = CampaignSummary(Path('unused')); summary.accept(campaign)
        self.assertEqual(summary.rows[0]['algorithm'], 'rank_auto')
        self.assertEqual(summary.rows[0]['auto_reason'], 'PowerConverged')
        self.assertEqual(summary.rows[0]['auto_power_updates'], '2')
        campaign['cells'][1]['records']['AUTO'][0]['reason'] = 'PowerEstimate'
        with self.assertRaises(ValueError): CampaignSummary(Path('unused')).accept(campaign)

    def assert_matrix_sizes(self, suite, cases):
        for workload, expected in cases.items():
            with self.subTest(suite=suite, workload=workload):
                selected = Cell.select(suite, workloads=None if workload is None else [workload])
                self.assertEqual(len(selected), expected)

    def test_phase_matrix_pairs_observed_and_unobserved_calls_and_is_opt_in(self):
        cells=Cell.select('ranking_phases')
        self.assert_matrix_sizes('ranking_phases', {None:32, 'rank2_1m':4})
        for cell in cells:
            observed=cell.subject=='rank_phases' or cell.subject.endswith('-profile')
            self.assertEqual(cell.environment(1).get('HYMEKO_PHASE_BENCH'), '1' if observed else None)
        with self.assertRaises(ValueError):Cell.select('ranking_phases',workloads=['steiner255'])

    def test_phase_partition_rejects_missing_samples_and_unrelated_totals(self):
        cell=dict(subject='rank_phases',repetitions='2',workload='chain65k')
        records={'CELL':[cell],
            'SAMPLE':[dict(ordinal=str(i),wall_us='12',repetitions='2') for i in range(9)],
            'STAGE_SAMPLE':[dict(ordinal=str(i),repetitions='2',incidence_us='2',preparation_us='3',
                solve_us='5',cleanup_us='1',inner_total_us='11',boundary='same_call_partition') for i in range(9)]}
        Evidence.verify_stages(records,cell)
        rows=CampaignSummary.phase_rows('native',dict(id='example',run=1,records=records))
        self.assertEqual(len(rows),9)
        self.assertEqual(rows[0]['total_us'],12.)
        self.assertEqual(rows[0]['inner_total_us'],11.)
        for key,value in [('inner_total_us','10'),('solve_us','nan'),('repetitions','1'),('boundary','independent_calls')]:
            bad=copy.deepcopy(records);bad['STAGE_SAMPLE'][0][key]=value
            with self.subTest(field=key), self.assertRaisesRegex(ValueError,'partition'):
                Evidence.verify_stages(bad,cell)
        bad=copy.deepcopy(records);bad['STAGE_SAMPLE'].pop()
        with self.assertRaises(ValueError):Evidence.verify_stages(bad,cell)
        bad=copy.deepcopy(records);bad['SAMPLE'][0]['wall_us']='9'
        with self.assertRaises(ValueError):Evidence.verify_stages(bad,cell)
        with self.assertRaisesRegex(ValueError,'uninstrumented control'):
            Evidence.verify_stages(records,dict(subject='rank_hymeko'))

    def test_scaling_matrix_keeps_candidates_and_default_matching_limits(self):
        cells=Cell.select("routing_scale")
        self.assert_matrix_sizes('routing_scale', {None:54, 'tour64':5, 'grid512_d511':12})
        self.assertTrue(any(c.subject=="reticula-unitpath-first" for c in cells))
        with self.assertRaises(ValueError):Cell.select("routing_scale",workloads=["tour16"])

    def test_routing_matrix_has_full_paths_and_one_metric_tour_case(self):
        cells=Cell.select("routing")
        self.assert_matrix_sizes('routing', {None:38, 'tour16':5, 'map128':11})
        for cell in cells:
            switch="HYMEKO_EXTERNAL_BENCH" if cell.suite=="external" else "HYMEKO_ASSISTED_BENCH"
            self.assertEqual(cell.environment(1)[switch],"1")
        with self.assertRaises(ValueError): Cell.select("routing",workloads=["chain65k"])
        unit=Cell.select("routing_unit")
        self.assertEqual(len(unit),4)
        self.assertEqual({c.workload for c in unit},{"grid200_d80"})
        self.assertTrue(all(c.subject.endswith("-unitpath-first") for c in unit))

    def test_assisted_selection_reuses_informed_client_and_requires_opt_in(self):
        cells=Cell.select("assisted")
        self.assertEqual(len(cells),21)
        self.assertEqual({c.suite for c in cells},{"informed"})
        self.assertEqual({c.test for c in cells},{"assisted::assisted_cell"})
        self.assertTrue(all(c.environment(1)["HYMEKO_ASSISTED_BENCH"]=="1" for c in cells))
        self.assertTrue(all(c.environment(1)["RAYON_NUM_THREADS"]=="1" for c in cells))
        self.assertEqual(len(Cell.select("assisted",workloads=["map128"],subjects=["assisted","first_assisted"])),2)
        with self.assertRaises(ValueError):Cell.select("assisted",workloads=["chain65k"])

    def test_assisted_contract_checks_first_result_cost_and_timing_boundary(self):
        cell={"subject":"first_assisted","nodes":"40","section":"informed_search"}
        records={"INPUT":[{"pipeline":"berge_bound_assist","verified":"true","flow":"undirected",
            "construction_timed":"false","index_timed":"true","caller_setup_timed":"true",
            "goal_preparation_timed":"true","metric_scope":"resident_ir_to_index_caller_setup_search_and_drop",
            "start":"0","goal":"39","expected_cost_micros":"12000000"}],
            "RESULT":[{"cost_micros":"12000000"}]}
        Evidence.verify_assisted(records,cell)
        for field,value in [("index_timed","false"),("caller_setup_timed","false"),
                            ("goal_preparation_timed","false"),("metric_scope","query_only"),("goal","40")]:
            bad=copy.deepcopy(records);bad["INPUT"][0][field]=value
            with self.assertRaises(ValueError):Evidence.verify_assisted(bad,cell)
        bad=copy.deepcopy(records);bad["RESULT"][0]["cost_micros"]="13000000"
        with self.assertRaises(ValueError):Evidence.verify_assisted(bad,cell)

    def test_external_selection_reuses_native_controls_and_explicit_opt_in(self):
        matrix = Cell.matrix("external", workloads=["steiner255"])
        self.assertEqual(len(matrix), 29)
        selected = Cell.select("external", workloads=["steiner255"],
            subjects=["reticula-distance-prepared", "components_hymeko", "berge_path"])
        self.assertEqual(len(selected), 3)
        foreign = next(c for c in selected if c.suite == "external")
        self.assertEqual(foreign.test, "external_cell")
        self.assertEqual(foreign.environment(1)["HYMEKO_EXTERNAL_BENCH"], "1")
        self.assertEqual(foreign.environment(1)["OPENBLAS_NUM_THREADS"], "1")
        self.assertEqual({c.test for c in selected if c.suite == "algorithm"},
                         {"native::native_cell", "ranking::ranking_cell"})

    def test_external_ranking_matrix_keeps_native_controls_and_all_numberings(self):
        cells=Cell.select('external_ranking')
        self.assertEqual(len(cells),48)
        self.assertEqual(len({c.workload for c in cells}),8)
        selected=Cell.select('external_ranking',workloads=['hub32_1m@17'],
                             subjects=['xgi-pagerank-first','rank_hymeko','rank_hypergraphz'])
        self.assertEqual(len(selected),3)
        self.assertEqual(selected[0].environment(1)['HYMEKO_EXTERNAL_BENCH'],'1')
        with self.assertRaises(ValueError):Cell.select('external_ranking',workloads=['steiner255'])

    def test_external_ranking_requires_full_residual_and_composition_attribution(self):
        cell=dict(subject='xgi-pagerank-first',checksum='verified',threads='1',warmups='2',nodes='5')
        records={'INPUT':[dict(construction_timed='false',cache_policy='first',metric_scope='full_rank',fixture_sha256='a'*64)],
                 'EXTERNAL':[dict(backend='xgi',operation='pagerank',mode='first',full_result='true',checksum='verified',
                    version='0.10.2',python='3.12.3',interface='python_api',completed_isolates='0',declared_isolates='2',
                    native_nodes='5',implementation_sha256='b'*64,native_pagerank='composed_scipy_gmres',query_strategy='factorized')],
                 'EXTERNAL_RANK':[dict(solver='scipy_gmres',solver_version='1.18.1',flow='undirected',weights='unit',
                    self_return='true',personalization='uniform',converged='true',score_hash='verified',composition='factorized',
                    damping='.85',tolerance='1e-10',residual_bound='1e-12',reference_l1='1e-12',mass='1',
                    max_iterations='200',iterations='8',matvecs='10')]}
        Evidence.verify_external(records,cell)
        for key,value in [('solver','native'),('self_return','false'),('converged','false'),('score_hash','wrong'),
                          ('residual_bound','nan'),('residual_bound','1e-8'),('reference_l1','1e-8'),('mass','0.9'),
                          ('iterations','201'),('composition','native')]:
            invalid=copy.deepcopy(records);invalid['EXTERNAL_RANK'][0][key]=value
            with self.subTest(field=key),self.assertRaises(ValueError):Evidence.verify_external(invalid,cell)

    def test_external_contract_rejects_unverified_domain_and_cache_drift(self):
        source = self.fixture().replace("EVIDENCE_CELL\t", "EVIDENCE_CELL\tsection=external_graph\tsubject=hypernetx-components-cold\tthreads=1\twarmups=2\trepetitions=1\tnodes=5\t")
        source += "\nEVIDENCE_INPUT\tconstruction_timed=false\tcache_policy=cold\tmetric_scope=first_query\tfixture_sha256=" + "a"*64
        source += "\nEVIDENCE_EXTERNAL\tbackend=hypernetx\toperation=components\tmode=cold\tfull_result=true\tchecksum=123\tversion=2.4.3\tpython=3.12.3\tinterface=python_api\tcompleted_isolates=2\tdeclared_isolates=2\tnative_nodes=3\timplementation_sha256=" + "b"*64
        Evidence.parse(source)
        self.assert_rejected_mutations(source, [
            ("full_result=true", "full_result=false"),
            ("native_nodes=3", "native_nodes=5"),
            ("cache_policy=cold", "cache_policy=prepared"),
            ("repetitions=1", "repetitions=2"),
            ("construction_timed=false", "construction_timed=true"),
            ("operation=components", "operation=reachable"),
        ])

    def test_external_dependency_identity_detects_changed_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory)/"inputs.json"
            data.write_text("{}")
            args = SimpleNamespace(suite="external", algorithm=None, external_python=__file__,
                external_input=str(data), platform="native", cpu=2)
            runner = ProcessRunner(args)
            self.assertEqual(len(runner.dependency_identity()["external_input"]), 64)
            data.write_text('{"changed":true}')
            with self.assertRaises(ValueError):
                runner.dependency_identity()

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

    def summary_campaign(self, platform):
        """Synthetic protocol records; no archived timings or local reports required."""
        cells = []
        for run in (1, 2):
            for cell in Cell.select("graph", subjects=["discrete_hops"]):
                records = Evidence.parse(self.fixture())
                records["CELL"][0].update(
                    section="heuristic_preparation", workload=cell.workload,
                    subject=cell.subject, nodes="3", arcs="2", incidences="4", threads="1",
                )
                cells.append(dict(id=cell.key, run=run, records=records,
                                  resources=dict(exit_code=0, peak_rss_bytes=1000, wall_seconds=1.0)))
        return dict(complete=True, smoke=False, suite="graph", runs=2,
                    subjects=["discrete_hops"], source_commit="a" * 40,
                    platform=platform, cells=cells)

    def test_summary_publishes_measured_boundaries_and_exact_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for platform in ("windows", "native"):
                (root / platform).mkdir()
                (root / platform / "campaign.json").write_text(json.dumps(self.summary_campaign(platform)))
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

    def test_campaign_rejects_corrupted_provenance_and_process_identity(self):
        original = self.summary_campaign("windows")
        for fault in ("source", "checksum", "repetition", "missing"):
            summary = CampaignSummary(Path("unused"))
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
