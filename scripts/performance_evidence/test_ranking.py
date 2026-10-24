"""Untimed contracts for framework/SciPy ranking compositions.

The small reference explicitly enumerates transition probabilities and solves a
dense linear system. It is test-only, never a selectable measurement backend.
"""
import dataclasses
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

import networkx as nx
import numpy as np

from external import BACKENDS, CanonicalCase, ExternalCell, OperationLimit
from ranking import RankingBackend


class RankingContract(unittest.TestCase):
    def test_detached_native_ownership_preserves_prepared_scores_and_releases_oracle(self):
        import gc
        import weakref
        for name in BACKENDS:
            case = self.case(5, [[0,1], [0,1], [1,2,3]])
            expected = np.array(case.data['pagerank'])
            reference = weakref.ref(case)
            backend = RankingBackend(case, name)
            original = backend.query()
            backend.verify(original)
            backend.detach_fixture()
            del case
            gc.collect()
            self.assertIsNone(reference())
            self.assertIsNone(backend.reference)
            self.assertIsNone(backend.native.case)
            prepared = backend.prepare()
            for _ in range(2):
                actual = prepared.solve()
                self.assertEqual(actual.checksum, original.checksum)
                np.testing.assert_allclose(actual.scores, expected, rtol=0, atol=1e-11)
            backend.detach_fixture()

    @staticmethod
    def case(n, edges):
        if n > 16:
            raise ValueError("dense test oracle is limited to 16 nodes")
        transition = np.zeros((n,n))
        topology = nx.Graph()
        topology.add_nodes_from(range(n))
        for edge in edges:
            topology.add_edges_from(zip(edge,edge[1:]))
        for source in range(n):
            incident = [edge for edge in edges if source in edge]
            if not incident:
                transition[:,source] = 1/n
            for edge in incident:
                for target in edge:
                    transition[target,source] += 1/(len(incident)*len(edge))
        scores = np.linalg.solve(np.eye(n)-.85*transition,np.full(n,.15/n)) if n else np.empty(0)
        components = list(nx.connected_components(topology))
        labels = [next(min(group) for group in components if v in group) for v in range(n)]
        return CanonicalCase(dict(family="ranking-contract",nodes=n,arcs=len(edges),memberships=edges,
            incidences=sum(map(len,edges)),components=len(components),labels=labels,reachable=[],
            shortest_hops=None,pagerank=scores.tolist(),damping=.85),"0"*64)

    def test_all_frameworks_match_dense_reference_with_isolates_and_multiplicity(self):
        cases = [(5,[[0,1,2],[2,3]]),(4,[[0,1],[0,1],[1,2],[2]]),
                 (3,[]),(0,[]),(4,[[0,1,2],[],[2,3]])]
        for n,edges in cases:
            for name in BACKENDS:
                with self.subTest(backend=name,n=n,edges=edges):
                    backend = RankingBackend(self.case(n,edges),name)
                    first = backend.query()
                    backend.verify(first)
                    np.testing.assert_allclose(first.scores,backend.case.data['pagerank'],rtol=0,atol=1e-11)
                    self.assertEqual(first,backend.query(materialize=False))
                    self.assertEqual(len(backend.implementation_hash()),64)
                    self.assertLessEqual(first.iterations,200)

    def test_seeded_graphs_and_relabeling_preserve_probabilities(self):
        rng = np.random.default_rng(17)
        for _ in range(4):
            edges = [rng.choice(7,size=int(rng.integers(1,5)),replace=False).tolist() for _ in range(9)]
            permutation = rng.permutation(7)
            for name in BACKENDS:
                original = RankingBackend(self.case(7,edges),name)
                relabeled = RankingBackend(self.case(7,[[int(permutation[v]) for v in e] for e in edges]),name)
                a,b = original.query(),relabeled.query()
                original.verify(a);relabeled.verify(b)
                np.testing.assert_allclose(a.scores,b.scores[permutation],rtol=0,atol=1e-11)

    def test_native_multiedge_exports_do_not_build_a_merge_repair_map(self):
        case=self.case(4,[[0,1],[0,1],[1,2]])
        for name in ('hypernetx','xgi'):
            backend=RankingBackend(case,name)
            with patch('ranking.Counter',side_effect=AssertionError('unnecessary merge repair')):
                result=backend.query();backend.verify(result)

    def test_verifier_rejects_mass_preserving_wrong_scores_and_bad_reference(self):
        backend = RankingBackend(self.case(4,[[0,1,2],[2,3]]),'xgi')
        result = backend.query()
        corrupted = result.scores.copy();corrupted[0]+=.01;corrupted[1]-=.01
        with self.assertRaises(ValueError):backend.verify(dataclasses.replace(result,scores=corrupted))
        with self.assertRaises(ValueError):backend.verify(dataclasses.replace(result,scores=np.full(4,np.nan)))
        with self.assertRaises(ValueError):backend.verify(dataclasses.replace(result,scores=np.ones(3)))
        backend.case.data.pop('pagerank')
        with self.assertRaises(ValueError):backend.verify(result)

    def test_unsupported_input_and_wrong_operation_fail_explicitly(self):
        with self.assertRaises(ValueError):RankingBackend(self.case(3,[[0,0,1]]),'xgi')
        case=self.case(3,[[0,1,2]]);case.data['damping']=.5
        with self.assertRaises(ValueError):RankingBackend(case,'xgi')
        backend=RankingBackend(self.case(3,[[0,1]]),'xgi')
        with self.assertRaises(ValueError):backend.query('distance')
        with self.assertRaises(ValueError):ExternalCell(backend.case,'xgi','prepared','pagerank')
        cell=ExternalCell(backend.case,'xgi','first','pagerank')
        self.assertIn('scipy_gmres',cell.strategy())
        with self.assertRaises(ValueError):self.case(17,[])

    def test_solver_nonconvergence_and_false_success_are_not_measured_results(self):
        backend=RankingBackend(self.case(3,[[0,1]]),'xgi')
        with patch('ranking.gmres',return_value=(np.full(3,1/3),200)):
            with self.assertRaises(OperationLimit):backend.query()
        with patch('ranking.gmres',return_value=(np.zeros(3),0)):
            with self.assertRaises(ValueError):backend.query()

    def test_external_ranking_timing_is_disabled_without_opt_in(self):
        environment=dict(os.environ);environment.pop('HYMEKO_EXTERNAL_BENCH',None)
        result=subprocess.run([sys.executable,str(Path(__file__).with_name('external.py')),'external_cell'],
                              env=environment,capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('external timing disabled',result.stderr)


if __name__ == '__main__':
    unittest.main()
