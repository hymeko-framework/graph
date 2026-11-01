"""Single-framework memory client and evidence contract for Campaign.

No scheduler or graph algorithm lives here. Default execution runs only tests.
Linux phase peaks use clear_refs=5, not a subtraction of lifetime high-water marks.
"""
from pathlib import Path
import gc
import hashlib
import importlib
import json
import math
import os
import struct
import sys
import queue
import threading
import time
from host import ProcessMemory


class PhaseSampler:
    """Controller-side samples between child checkpoints; never an exact OS HWM."""
    def __init__(self):
        self.peak=0
        self.samples=0
        self.last=None
        self.gap=0.0
        self.phases=0

    def sample(self, snapshot, now):
        self.peak=max(self.peak,snapshot['working_set_b'])
        self.samples+=1
        if self.last is not None:self.gap=max(self.gap,now-self.last)
        self.last=now

    def finish(self, line):
        row=dict(x.split('=',1) for x in line[line.index('EVIDENCE_PHASE'):].strip().split('\t')[1:] if '=' in x)
        current=int(row['working_set_b'])
        assert row['phase']==MemoryEvidence.phases[self.phases], 'checkpoint order changed'
        output=(line.rstrip('\r\n')+f'\tphase_peak_b={max(self.peak,current)}'
                f'\tpeak_method=sampled_working_set\tsample_interval_us=2000'
                f'\tsample_count={self.samples+1}\tmax_sample_gap_us={round(self.gap*1e6)}\n')
        self.peak=current;self.samples=0;self.last=None;self.gap=0.0;self.phases+=1
        return output


class WindowsMemoryCapture:
    """One stdout reader and sampler per measured child; child waits at checkpoints."""
    def __init__(self,process,stream,resolve_pid=None):
        self.process,self.stream=process,stream
        self.resolve_pid=resolve_pid
        self.lines=queue.Queue()
        self.sampler=PhaseSampler()
        self.reader=threading.Thread(target=self.read,daemon=True)
        self.reader.start()

    def read(self):
        for line in iter(self.process.stdout.readline,b''):
            self.lines.put(line.decode('utf8',errors='replace'))

    def sample(self,snapshot):
        self.sampler.sample(snapshot,time.perf_counter())

    def drain(self):
        while True:
            try:line=self.lines.get_nowait()
            except queue.Empty:return
            checkpoint='EVIDENCE_PHASE\t' in line
            if checkpoint:
                fields=dict(x.split('=',1) for x in line.strip().split('\t')[1:] if '=' in x)
                target=int(fields.get('process_id',self.process.pid))
                if self.resolve_pid and self.resolve_pid(target):
                    if self.sampler.phases:raise ValueError('measurement process changed mid-run')
                    self.sampler=PhaseSampler()  # Discard launcher samples before the ready checkpoint.
                line=self.sampler.finish(line).rstrip('\n')+f'\tsampled_process_id={target}\n'
            self.stream.write(line)
            if checkpoint and self.process.poll() is None:
                self.process.stdin.write(b'continue\n');self.process.stdin.flush()

    def close(self,complete):
        self.reader.join(timeout=2)
        if self.reader.is_alive():raise RuntimeError('memory output reader did not finish')
        self.drain()
        if complete and self.sampler.phases!=len(MemoryEvidence.phases):
            raise ValueError('missing Windows memory checkpoints')


class MemoryEvidence:
    phases = ("baseline", "input", "graph_build", "graph_retained", "prepared",
              "warmup_0", "warmup_1", *(f"solve_{i}" for i in range(5)))

    @staticmethod
    def fingerprint(nodes, edges):
        offsets = [0]
        for edge in edges:
            offsets.append(offsets[-1] + len(edge))
        value = 0xcbf29ce484222325
        from itertools import chain
        for word in chain([nodes], offsets, chain.from_iterable(edges)):
            for byte in struct.pack("<Q", word):
                value = ((value ^ byte) * 0x100000001b3) & (2**64-1)
        return str(value)

    @classmethod
    def split(cls, source, destination):
        """Copy each existing canonical case without generating any topology."""
        destination = Path(destination)
        destination.mkdir(parents=True, exist_ok=False)
        raw = Path(source).read_bytes()
        document = json.loads(raw)
        if document.get("representation") != "canonical Core Ir":
            raise ValueError("canonical IR export required")
        manifest = {"source_sha256": hashlib.sha256(raw).hexdigest(), "cases": {}}
        for case in document["cases"]:
            name, n, edges = case["family"], case["nodes"], case["memberships"]
            if Path(name).name != name or name in manifest["cases"]:
                raise ValueError("invalid or duplicate case name")
            offsets = [0]
            for edge in edges:
                offsets.append(offsets[-1] + len(edge))
            if len(edges) != case["arcs"] or offsets[-1] != case["incidences"]:
                raise ValueError("canonical shape mismatch")
            payload = struct.pack("<III", n, len(edges), offsets[-1])
            payload += struct.pack(f"<{len(offsets)}I", *offsets)
            from itertools import chain
            members = list(chain.from_iterable(edges))
            if any(not 0 <= v < n for v in members):
                raise ValueError("node outside domain")
            payload += struct.pack(f"<{len(members)}I", *members)
            single = json.dumps({"representation": document["representation"], "cases": [case]},
                                separators=(",", ":")).encode()
            (destination / f"{name}.bin").write_bytes(payload)
            (destination / f"{name}.json").write_bytes(single)
            manifest["cases"][name] = {"nodes": n, "arcs": len(edges), "incidences": offsets[-1],
                "fingerprint": cls.fingerprint(n, edges),
                "binary_sha256": hashlib.sha256(payload).hexdigest(),
                "json_sha256": hashlib.sha256(single).hexdigest()}
        (destination / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf8")
        return manifest

    @classmethod
    def verify(cls, records):
        cells, phases, metadata = records.get("CELL", []), records.get("PHASE", []), records.get("MEMORY", [])
        if len(cells) != 1 or cells[0].get("status") != "OK" or records.get("SAMPLE"):
            raise ValueError("memory result must be successful and contain no timing samples")
        if [row["phase"] for row in phases] != list(cls.phases):
            raise ValueError("incomplete or reordered memory phases")
        for row in phases:
            current, peak = int(row["working_set_b"]), int(row["self_peak_b"])
            if current <= 0 or peak < current:
                raise ValueError("invalid memory observation")
            if 'phase_peak_b' in row:
                if (int(row['phase_peak_b'])<current or row.get('peak_method')!='sampled_working_set'
                        or int(row.get('sample_count','0'))<1 or int(row.get('sample_interval_us','0'))!=2000):
                    raise ValueError('invalid sampled phase peak')
                if 'process_id' in row and row['process_id']!=row.get('sampled_process_id'):
                    raise ValueError('sampled the launcher instead of the measured process')
        sampled=[p.get('peak_method','reset_os_hwm') for p in phases]
        if len(set(sampled))!=1:raise ValueError('mixed memory observation methods')
        if len(metadata) != 1 or any(metadata[0].get(k) != v for k, v in
                {"input_released": "true", "frameworks": "1", "verification": "controller"}.items()):
            raise ValueError("memory isolation contract missing")
        if metadata[0].get('platform')=='windows' and sampled[0]!='sampled_working_set':
            raise ValueError('Windows lifetime peak cannot stand in for a phase peak')
        subject = cells[0].get("subject", "")
        for owned in records.get('IR_HEAP',[]):
            if (int(owned['live_requested_b'])!=int(owned['released_on_drop_b'])
                    or int(owned['construction_peak_extra_b'])<int(owned['live_requested_b'])
                    or int(owned['reference_capacity_b'])<int(owned['reference_len_b'])):
                raise ValueError('invalid IR heap ownership evidence')
        if subject in {"hymeko-gmres-fresh", "hymeko-gmres-reused", "hymeko-auto-fresh", "hymeko-auto-reused",
                       "hymeko-auto-quotient-fresh", "hymeko-auto-quotient-reused"}:
            workspaces = records.get("WORKSPACE", [])
            reuse = subject.endswith("reused")
            solver = "auto" if "auto" in subject else "gmres"
            if len(workspaces) != 7:
                raise ValueError("missing solver workspace observations")
            for i, row in enumerate(workspaces):
                capacity, after = int(row["capacity_b"]), int(row["after_call_b"])
                if (int(row["iteration"]) != i or row["solver"] != solver
                        or row["reused"] != str(reuse).lower() or capacity <= 0
                        or after != (capacity if reuse else 0)):
                    raise ValueError("incorrect solver workspace lifecycle")
            if solver == "auto":
                from campaign import Evidence
                decisions = records.get("AUTO", [])
                if len(decisions) != 7:
                    raise ValueError("missing Auto memory decisions")
                quotients = records.get("QUOTIENT", [])
                if (quotients or "quotient" in subject) and len(quotients) != 7:
                    raise ValueError("missing quotient memory decisions")
                for i, (decision, workspace) in enumerate(zip(decisions, workspaces)):
                    if int(decision['iteration']) != i:
                        raise ValueError("Auto memory decision order changed")
                    evidence = {'AUTO':[decision], 'CELL':cells}
                    if quotients: evidence['QUOTIENT'] = [quotients[i]]
                    Evidence.verify_auto(evidence,
                                         {'hymeko_iterations':workspace['steps']}, selection_timed=False)

    @staticmethod
    def verify_scores(directory, workload, path, fingerprint):
        """Independent oracle executes only after the measured process exits."""
        import numpy as np
        from external import CanonicalCase
        from ranking import RankingBackend, RankResult
        case = CanonicalCase.load(Path(directory) / f"{workload}.json", workload)
        if MemoryEvidence.fingerprint(case.n, case.edges) != fingerprint:
            raise ValueError("memory input fingerprint mismatch")
        scores = np.fromfile(path, dtype="<f8")
        # Use the existing independent verifier without constructing a native graph.
        oracle = RankingBackend.__new__(RankingBackend)
        oracle.case, oracle.reference = case, None
        oracle.verify(RankResult(scores, "controller", 0, 0, 0.0))
        return {"reference_l1": oracle.reference_l1, "residual_bound": oracle.verified_bound,
                "scores_sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest()}


class MemoryClient:
    @staticmethod
    def mark(phase):
        snapshot=ProcessMemory.snapshot(os.getpid())
        if snapshot is None:raise RuntimeError('process memory counters unavailable')
        print(f"EVIDENCE_PHASE\tphase={phase}\tprocess_id={os.getpid()}"+''.join(f'\t{k}={v}' for k,v in snapshot.items()),flush=True)
        if os.name=='nt':
            if os.environ.get('HYMEKO_MEMORY_HANDSHAKE')!='1' or sys.stdin.readline().strip()!='continue':
                raise RuntimeError('missing Windows memory checkpoint controller')
        else:Path("/proc/self/clear_refs").write_text("5\n")

    @classmethod
    def run(cls):
        if os.environ.get("HYMEKO_MEMORY_BENCH") != "1":
            raise SystemExit("isolated memory measurement disabled; set HYMEKO_MEMORY_BENCH=1 explicitly")
        from external import CanonicalCase, QueryDeadline
        from ranking import RankingBackend
        import numpy as np
        backend, workload = os.environ["EVIDENCE_OPERATION"], os.environ["EVIDENCE_WORKLOAD"]
        importlib.import_module(backend)
        cls.mark("baseline")
        directory = Path(os.environ["HYMEKO_MEMORY_INPUT"])
        case = CanonicalCase.load(directory / f"{workload}.json", workload)
        n, arcs, incidences = case.n, case.data["arcs"], case.data["incidences"]
        fingerprint = MemoryEvidence.fingerprint(n, case.edges)
        cls.mark("input")
        adapter = RankingBackend(case, backend)
        cls.mark("graph_build")
        adapter.detach_fixture()
        del case
        gc.collect()
        cls.mark("graph_retained")
        with QueryDeadline():
            prepared = adapter.prepare()
        cls.mark("prepared")
        result = None
        for i in range(7):
            result = None
            with QueryDeadline():
                result = prepared.solve()
            cls.mark(f"warmup_{i}" if i < 2 else f"solve_{i-2}")
        np.asarray(result.scores, dtype="<f8").tofile(os.environ["HYMEKO_MEMORY_SCORES"])
        print(f"EVIDENCE_MEMORY\tfingerprint={fingerprint}\tinput_released=true\tframeworks=1"
              f"\tverification=controller\tpreparation=separate\tplatform={'windows' if os.name=='nt' else 'linux'}", flush=True)
        print(f"EVIDENCE_CELL\tsection=memory\tsubject={backend}\tworkload={workload}\tstatus=OK"
              f"\tnodes={n}\tarcs={arcs}\tincidences={incidences}\tchecksum={result.checksum}", flush=True)


class MemorySummary:
    """Aggregate fresh-process peaks, keeping retained RSS and peaks distinct."""
    @staticmethod
    def rows(campaign):
        from campaign import Cell
        if campaign.get("suite") != "memory" or not campaign.get("complete") or campaign.get("runs", 0) < 5:
            raise ValueError("complete five-process memory campaign required")
        selected = Cell.select("memory", workloads=campaign.get("workloads"), subjects=campaign.get("subjects"),
                               application_workloads=tuple(campaign.get("application_inputs", {})))
        expected = {(run, cell.key) for run in range(1, campaign['runs']+1) for cell in selected}
        actual = {(cell['run'], cell['id']) for cell in campaign['cells']}
        if actual != expected or len(actual) != len(campaign['cells']):
            raise ValueError("missing or duplicate memory process")
        groups = {}
        methods={c['records']['PHASE'][0].get('peak_method','reset_os_hwm') for c in campaign['cells']}
        if len(methods)!=1:raise ValueError('campaign mixes exact and sampled phase peaks')
        for cell in campaign['cells']:
            records = cell['records']
            MemoryEvidence.verify(records)
            checked = cell['resources']['memory_verification']
            for key, limit in [('reference_l1', 2.01e-10), ('residual_bound', 1.01e-10)]:
                if not math.isfinite(checked[key]) or not 0 <= checked[key] <= limit:
                    raise ValueError("invalid independently checked ranking result")
            header = records['CELL'][0]
            phases = {p['phase']:p for p in records['PHASE']}
            values = {name: int(phases[name]['working_set_b']) for name in
                      ('baseline', 'input', 'graph_retained', 'prepared')}
            peak=lambda phase:int(phase.get('phase_peak_b',phase['self_peak_b']))
            values['construction_peak'] = peak(phases['graph_build'])
            values['preparation_peak'] = peak(phases['prepared'])
            values['solve_peak'] = max(peak(phases[f'solve_{i}']) for i in range(5))
            values['lifetime_peak'] = max(int(p['self_peak_b']) for p in phases.values())
            values['graph_above_baseline'] = values['graph_retained']-values['baseline']
            values['solve_above_prepared'] = max(0, values['solve_peak']-values['prepared'])
            if records.get('WORKSPACE'):
                values['workspace_capacity'] = max(int(row['capacity_b']) for row in records['WORKSPACE'])
                values['workspace_retained'] = max(int(row['after_call_b']) for row in records['WORKSPACE'])
            if records.get('IR_HEAP'):
                for key,value in records['IR_HEAP'][0].items():values['ir_'+key]=int(value)
            if phases['graph_retained'].get('private_committed_b','NA')!='NA':
                values['private_graph_retained']=int(phases['graph_retained']['private_committed_b'])
            for metric, value in values.items():
                groups.setdefault((header['workload'],header['subject'],metric),[]).append(value/1024**2)
        rows=[]
        for (workload,backend,metric),values in sorted(groups.items()):
            values.sort();n=len(values)
            rows.append(dict(workload=workload,backend=backend,metric=metric,processes=n,
                median_mib=values[n//2],q1_mib=values[n//4],q3_mib=values[3*n//4],worst_mib=values[-1]))
        return rows

    @classmethod
    def write(cls, directory):
        import csv
        import matplotlib
        matplotlib.use('Agg')
        import matplotlib.pyplot as plt
        from matplotlib.backends.backend_pdf import PdfPages
        directory=Path(directory)
        campaign=json.loads((directory/'campaign.json').read_text())
        rows=cls.rows(campaign)
        with (directory/'memory-summary.csv').open('w',newline='',encoding='utf8') as stream:
            writer=csv.DictWriter(stream,fieldnames=list(rows[0]));writer.writeheader();writer.writerows(rows)
        indexed={(r['workload'],r['backend'],r['metric']):r for r in rows}
        catalog = {'hymeko-auto-fresh':'Auto fresh', 'hymeko-auto-reused':'Auto reused',
                   'hymeko-auto-quotient-fresh':'Auto quotient fresh', 'hymeko-auto-quotient-reused':'Auto quotient reused',
                   'hymeko':'HyMeKo power', 'hymeko-gmres-fresh':'GMRES fresh',
                   'hymeko-gmres-reused':'GMRES reused', 'hypergraphz':'Hypergraphz',
                   'hypernetx':'HNX + SciPy', 'xgi':'XGI + SciPy',
                   'hypergraphx':'HGX + SciPy', 'reticula':'Reticula + SciPy'}
        frameworks=tuple(b for b in catalog if any(r['backend']==b for r in rows))
        labels=tuple(catalog[b] for b in frameworks)
        family_labels={'chain65k':'Chain: 131,070 incidences','rank2_1m':'Binary: 1,000,000 incidences',
                  'hub32_1m':'Hub: 1,000,000 incidences','components8_1m':'Eight regions: 1,000,000 incidences'}
        families={name:family_labels.get(name,name) for name in dict.fromkeys(row['workload'].split('@')[0] for row in rows)}
        palette=('#004c54','#63b7af','#9654a0','#c490bd','#087f8c','#319fce','#184e77','#dc8c24','#7755aa','#4267a5','#c26470','#6b8055')
        colors=tuple(palette[list(catalog).index(b)] for b in frameworks)
        sampled=campaign['cells'][0]['records']['PHASE'][0].get('peak_method')=='sampled_working_set'
        plt.rcParams.update({'font.family':'DejaVu Sans','font.size':10,'axes.spines.top':False,'axes.spines.right':False})
        with PdfPages(directory/'memory-report.pdf') as pdf:
            for view in ('chart', 'table'):
                for suffix,layout in [('', 'Original numbering'),('@17','Seed-17 permutation')]:
                    if not any(row['workload']==family+suffix for family in families for row in rows):
                        continue
                    count=(len(families)+1)//2
                    fig,axes=plt.subplots(count,2,figsize=(12,4.5*count),squeeze=False,layout='constrained')
                    for ax,(family,title) in zip(axes.flat,families.items()):
                        if not all((family+suffix,b,'solve_peak') in indexed for b in frameworks):
                            ax.set_visible(False)
                            continue
                        if view == 'chart':
                            selected=[indexed[(family+suffix,b,'solve_peak')] for b in frameworks]
                            values=[r['median_mib'] for r in selected]
                            errors=[[r['median_mib']-r['q1_mib'] for r in selected],[r['q3_mib']-r['median_mib'] for r in selected]]
                            bars=ax.bar(range(len(frameworks)),values,color=colors,yerr=errors,capsize=3)
                            ax.bar_label(bars,fmt='%.1f',padding=4,fontsize=9)
                            ax.set(xticks=range(len(frameworks)),xticklabels=labels,ylabel='Sampled phase peak WS [MiB]' if sampled else 'Process peak RSS [MiB]',title=title,ylim=(0,max(values)*1.22))
                            ax.tick_params(axis='x',labelrotation=25)
                            ax.grid(axis='y',alpha=.15);ax.set_axisbelow(True)
                        else:
                            data=[]
                            for label,b in zip(labels,frameworks):
                                data.append([label]+[f"{indexed[(family+suffix,b,m)]['median_mib']:.1f}" for m in ('baseline','graph_retained','prepared','solve_peak','lifetime_peak')])
                            ax.axis('off');ax.set_title(title,fontweight='bold')
                            table=ax.table(cellText=data,colLabels=['Workflow','Base','Graph','Prepared','Solve peak','Life peak'],loc='center',cellLoc='right',colWidths=[.29,.12,.13,.14,.16,.16])
                            table.auto_set_font_size(False);table.set_fontsize(9);table.scale(1,1.8)
                            for (r,c),cell in table.get_celld().items():
                                cell.set_edgecolor('#d9e0e5')
                                if r==0:cell.set_facecolor('#e5eef2');cell.get_text().set_fontweight('bold')
                                elif r==1:cell.set_facecolor('#e5f4f2')
                    if view == 'chart':
                        heading='Isolated PageRank memory | '+layout+'\n65,536 nodes; one framework and one input per process'
                        note='Median of five fresh-process solve peaks; IQR error bars. Two warmups + five solves.\nIncludes runtime, resident graph and prepared operator. Hypergraphz prepares internally during solve.'
                    else:
                        heading='Memory boundaries [MiB] | '+layout
                        note='Base: imports loaded. Graph: input/oracle released. Prepared: algorithm factors retained.\nPeaks reset per phase; Life peak includes input and construction. No allocator trimming.\nRSS is process residency, not intrinsic live heap bytes. Full-vector verification runs after child exit.'
                    if sampled:
                        note += '\nWindows phase peaks: 2 ms controller samples + endpoints, may miss short spikes. Life peak: OS cumulative HWM.'
                        note=note.replace('Peaks reset per phase;','Sampled peaks separated by acknowledged checkpoints;')
                    fig.suptitle(heading,fontsize=16,fontweight='bold')
                    fig.supxlabel(note,fontsize=9)
                    pdf.savefig(fig);plt.close(fig)
        return {'processes':len(campaign['cells']),'aggregate_rows':len(rows),'report':str(directory/'memory-report.pdf')}


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "memory_cell":
        MemoryClient.run()
    elif len(sys.argv) == 3 and sys.argv[1] == '--summarize':
        print(json.dumps(MemorySummary.write(sys.argv[2])))
    else:
        import unittest
        unittest.main(module="test_memory", argv=[sys.argv[0]])
