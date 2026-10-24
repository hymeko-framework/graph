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
        if len(metadata) != 1 or any(metadata[0].get(k) != v for k, v in
                {"input_released": "true", "frameworks": "1", "verification": "controller"}.items()):
            raise ValueError("memory isolation contract missing")

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


class LinuxMemory:
    @staticmethod
    def mark(phase):
        status = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines() if ":" in line)
        print(f"EVIDENCE_PHASE\tphase={phase}\tworking_set_b={int(status['VmRSS'].split()[0])*1024}"
              f"\tself_peak_b={int(status['VmHWM'].split()[0])*1024}", flush=True)
        Path("/proc/self/clear_refs").write_text("5\n")

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
              "\tverification=controller\tpreparation=separate", flush=True)
        print(f"EVIDENCE_CELL\tsection=memory\tsubject={backend}\tworkload={workload}\tstatus=OK"
              f"\tnodes={n}\tarcs={arcs}\tincidences={incidences}\tchecksum={result.checksum}", flush=True)


class MemorySummary:
    """Aggregate fresh-process peaks, keeping retained RSS and peaks distinct."""
    @staticmethod
    def rows(campaign):
        from campaign import Cell
        if campaign.get("suite") != "memory" or not campaign.get("complete") or campaign.get("runs", 0) < 5:
            raise ValueError("complete five-process memory campaign required")
        selected = Cell.select("memory", workloads=campaign.get("workloads"), subjects=campaign.get("subjects"))
        expected = {(run, cell.key) for run in range(1, campaign['runs']+1) for cell in selected}
        actual = {(cell['run'], cell['id']) for cell in campaign['cells']}
        if actual != expected or len(actual) != len(campaign['cells']):
            raise ValueError("missing or duplicate memory process")
        groups = {}
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
            values['construction_peak'] = int(phases['graph_build']['self_peak_b'])
            values['preparation_peak'] = int(phases['prepared']['self_peak_b'])
            values['solve_peak'] = max(int(phases[f'solve_{i}']['self_peak_b']) for i in range(5))
            values['lifetime_peak'] = max(int(p['self_peak_b']) for p in phases.values())
            values['graph_above_baseline'] = values['graph_retained']-values['baseline']
            values['solve_above_prepared'] = max(0, values['solve_peak']-values['prepared'])
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
        frameworks=('hymeko','hypergraphz','hypernetx','xgi','hypergraphx','reticula')
        labels=('HyMeKo','Hypergraphz','HNX + SciPy','XGI + SciPy','HGX + SciPy','Reticula + SciPy')
        families={'chain65k':'Chain: 131,070 incidences','rank2_1m':'Binary: 1,000,000 incidences',
                  'hub32_1m':'Hub: 1,000,000 incidences','components8_1m':'Eight regions: 1,000,000 incidences'}
        colors=('#087f8c','#dc8c24','#7755aa','#4267a5','#c26470','#6b8055')
        plt.rcParams.update({'font.family':'DejaVu Sans','font.size':10,'axes.spines.top':False,'axes.spines.right':False})
        with PdfPages(directory/'memory-report.pdf') as pdf:
            for view in ('chart', 'table'):
                for suffix,layout in [('', 'Original numbering'),('@17','Seed-17 permutation')]:
                    fig,axes=plt.subplots(2,2,figsize=(12,9),layout='constrained')
                    for ax,(family,title) in zip(axes.flat,families.items()):
                        if view == 'chart':
                            selected=[indexed[(family+suffix,b,'solve_peak')] for b in frameworks]
                            values=[r['median_mib'] for r in selected]
                            errors=[[r['median_mib']-r['q1_mib'] for r in selected],[r['q3_mib']-r['median_mib'] for r in selected]]
                            bars=ax.bar(range(6),values,color=colors,yerr=errors,capsize=3)
                            ax.bar_label(bars,fmt='%.1f',padding=4,fontsize=9)
                            ax.set(xticks=range(6),xticklabels=labels,ylabel='Process peak RSS [MiB]',title=title,ylim=(0,max(values)*1.22))
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
                    fig.suptitle(heading,fontsize=16,fontweight='bold')
                    fig.supxlabel(note,fontsize=9)
                    pdf.savefig(fig);plt.close(fig)
        return {'processes':len(campaign['cells']),'aggregate_rows':len(rows),'report':str(directory/'memory-report.pdf')}


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "memory_cell":
        LinuxMemory.run()
    elif len(sys.argv) == 3 and sys.argv[1] == '--summarize':
        print(json.dumps(MemorySummary.write(sys.argv[2])))
    else:
        import unittest
        unittest.main(module="test_memory", argv=[sys.argv[0]])
