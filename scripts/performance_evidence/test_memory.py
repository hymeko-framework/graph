"""Correctness-only memory contracts; no RSS collection in these tests."""
import copy
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from campaign import Cell, Evidence
from memory import MemoryEvidence, MemorySummary, LinuxMemory


class MemoryContract(unittest.TestCase):
    def test_summary_uses_process_peaks_and_rejects_incomplete_or_uncertified_runs(self):
        campaign = {'suite':'memory','complete':True,'runs':5,'cells':[]}
        for run in range(1,6):
            for cell in Cell.select('memory'):
                records = {'CELL':[{'section':'memory','status':'OK','workload':cell.workload,'subject':cell.subject}],
                    'PHASE':[{'phase':p,'working_set_b':'1024','self_peak_b':str(run*(i+1)*1024)}
                             for i,p in enumerate(MemoryEvidence.phases)],
                    'MEMORY':[{'input_released':'true','frameworks':'1','verification':'controller'}]}
                campaign['cells'].append({'run':run,'id':cell.key,'records':records,
                    'resources':{'memory_verification':{'reference_l1':0.,'residual_bound':0.}}})
        rows=MemorySummary.rows(campaign)
        solve=next(r for r in rows if r['metric']=='solve_peak')
        self.assertEqual(solve['median_mib'],36/1024)
        self.assertEqual(solve['worst_mib'],60/1024)
        bad=copy.deepcopy(campaign);bad['cells'].pop()
        with self.assertRaises(ValueError):MemorySummary.rows(bad)
        bad=copy.deepcopy(campaign);bad['cells'][0]['resources']['memory_verification']['residual_bound']=float('nan')
        with self.assertRaises(ValueError):MemorySummary.rows(bad)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'campaign.json').write_text(json.dumps(campaign))
            result=MemorySummary.write(root)
            self.assertEqual(result['processes'],240)
            self.assertEqual(result['aggregate_rows'],480)
            self.assertGreater((root/'memory-report.pdf').stat().st_size,1000)

    def test_matrix_is_complete_and_opt_in_is_explicit(self):
        cells = Cell.select("memory")
        self.assertEqual(len(cells), 48)
        self.assertEqual(len(Cell.select("memory", workloads=["chain65k"])), 6)
        for cell in cells:
            self.assertEqual(cell.environment(1)["HYMEKO_MEMORY_BENCH"], "1")
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(SystemExit, "disabled"):
                LinuxMemory.run()
        with self.assertRaises(ValueError):
            Cell.select("memory", workloads=["invalid"])

    def test_split_exports_one_case_and_native_binary_without_generating_topology(self):
        cases = [dict(family="a", nodes=4, arcs=2, incidences=4, memberships=[[0,1],[1,2]]),
                 dict(family="b", nodes=1, arcs=0, incidences=0, memberships=[])]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "all.json"
            source.write_text(json.dumps({"representation":"canonical Core Ir", "cases":cases}))
            manifest = MemoryEvidence.split(source, root / "single")
            self.assertEqual(set(manifest['cases']), {'a','b'})
            payload = (root / 'single/a.bin').read_bytes()
            self.assertEqual(struct.unpack('<10I',payload), (4,2,4,0,2,4,0,1,1,2))
            self.assertEqual(json.loads((root/'single/a.json').read_text())['cases'], [cases[0]])
            with self.assertRaises(FileExistsError):MemoryEvidence.split(source, root/'single')
            source.write_text('{}')
            with self.assertRaises(ValueError):MemoryEvidence.split(source, root/'bad')

    def test_memory_contract_rejects_missing_phase_and_contaminated_ownership(self):
        records = {'CELL':[{'section':'memory','status':'OK'}],
            'PHASE':[{'phase':p,'working_set_b':'1024','self_peak_b':'2048'} for p in MemoryEvidence.phases],
            'MEMORY':[{'input_released':'true','frameworks':'1','verification':'controller'}]}
        MemoryEvidence.verify(records)
        raw = '\n'.join('EVIDENCE_'+kind+'\t'+'\t'.join(k+'='+v for k,v in row.items())
                        for kind, rows in records.items() for row in rows)
        self.assertEqual(Evidence.parse(raw), records)
        for key,value in [('input_released','false'),('frameworks','2')]:
            bad=copy.deepcopy(records);bad['MEMORY'][0][key]=value
            with self.assertRaises(ValueError):MemoryEvidence.verify(bad)
        bad=copy.deepcopy(records);bad['PHASE'].pop()
        with self.assertRaises(ValueError):MemoryEvidence.verify(bad)
        bad=copy.deepcopy(records);bad['PHASE'][0]['self_peak_b']='1'
        with self.assertRaises(ValueError):MemoryEvidence.verify(bad)

    def test_phase_reads_peak_and_only_resets_hwm(self):
        with patch('pathlib.Path.read_text',return_value='VmRSS: 2 kB\nVmHWM: 4 kB\n'), \
             patch('pathlib.Path.write_text') as write, patch('builtins.print') as emit:
            LinuxMemory.mark('graph_retained')
        write.assert_called_once_with('5\n')
        self.assertIn('working_set_b=2048',emit.call_args.args[0])
        self.assertIn('self_peak_b=4096',emit.call_args.args[0])

    def test_controller_verification_rejects_mass_preserving_wrong_vector(self):
        import numpy as np
        from test_ranking import RankingContract
        case=RankingContract.case(4,[[0,1],[1,2]])
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            (root/'ranking-contract.json').write_text(json.dumps({'representation':'canonical Core Ir','cases':[case.data]}))
            values=np.array(case.data['pagerank'],dtype='<f8')
            values.tofile(root/'scores')
            fingerprint=MemoryEvidence.fingerprint(4,case.edges)
            result=MemoryEvidence.verify_scores(root,'ranking-contract',root/'scores',fingerprint)
            self.assertLess(result['residual_bound'],1e-10)
            values[0]+=.01;values[1]-=.01;values.tofile(root/'scores')
            with self.assertRaises(ValueError):MemoryEvidence.verify_scores(root,'ranking-contract',root/'scores',fingerprint)


if __name__ == '__main__':
    unittest.main()
