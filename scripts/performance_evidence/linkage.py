"""Opt-in static/DLL Windows resident-memory control over published Graph owners."""
from pathlib import Path
import argparse
import ctypes
import hashlib
import json
import os
import queue
import subprocess
import threading
import time
from datetime import datetime, timezone
from host import HostState
from working_set import WindowsWorkingSet


class ResidentCohort:
    phases = ('loaded', 'graph', 'prepared', 'resident')

    def __init__(self, binaries, case, log_dir):
        self.children=[]; self.case=case; self.log_dir=log_dir
        self.deadline=time.monotonic()+180
        self.affinity=ctypes.WinDLL('kernel32',use_last_error=True).SetProcessAffinityMask
        self.affinity.argtypes=[ctypes.c_void_p,ctypes.c_size_t]
        try:
            for ordinal,binary in enumerate(binaries):
                error=(log_dir/f'child-{ordinal}.stderr').open('w',encoding='utf-8')
                env=dict(os.environ)
                env['PATH']=str(binary.parent)+os.pathsep+env.get('PATH','')
                process=subprocess.Popen([str(binary),'--enable-memory-measurement',case],
                    stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=error,text=True,
                    env=env,creationflags=subprocess.CREATE_NO_WINDOW|subprocess.BELOW_NORMAL_PRIORITY_CLASS)
                messages=queue.Queue()
                child=dict(process=process,messages=messages,error=error,pages=None)
                self.children.append(child)
                if not self.affinity(ctypes.c_void_p(int(process._handle)),1<<(2+ordinal*2)):
                    raise ctypes.WinError(ctypes.get_last_error())
                child['pages']=WindowsWorkingSet(process.pid)
                threading.Thread(target=self.read,args=(process,messages),daemon=True).start()
        except BaseException:
            self.close();raise

    @staticmethod
    def read(process,messages):
        try:
            for line in process.stdout:messages.put(json.loads(line))
        except BaseException as error:messages.put(error)
        finally:messages.put(None)

    def run(self):
        result=[]
        try:
            for phase in self.phases:
                checkpoints=[]
                for child in self.children:
                    row=child['messages'].get(timeout=max(.1,self.deadline-time.monotonic()))
                    if not isinstance(row,dict):raise RuntimeError(f'child failed before {phase}: {row}')
                    if row['phase']!=phase or row['pid']!=child['process'].pid:raise ValueError('checkpoint identity/order')
                    checkpoints.append(row)
                if phase!='loaded':
                    expected=(65536,131070 if self.case=='chain65k' else 1000000)
                    if any((r['nodes'],r['incidences'])!=expected for r in checkpoints):raise ValueError('input dimensions')
                if phase=='resident' and len({(r['checksum'],r['steps']) for r in checkpoints})!=1:
                    raise ValueError('cohort outputs differ')
                samples=[]
                for _ in range(3):
                    pages=[child['pages'].snapshot() for child in self.children]
                    if any(p['working_set_b']>2**31 for p in pages):raise RuntimeError('child memory budget')
                    samples.append(dict(processes=pages,cohort=WindowsWorkingSet.cohort(pages)))
                    time.sleep(.025)
                result.append(dict(phase=phase,checkpoints=checkpoints,samples=samples))
                for child in self.children:
                    child['process'].stdin.write('continue\n');child['process'].stdin.flush()
            for child in self.children:
                if child['process'].wait(timeout=10)!=0:raise RuntimeError('client failed after final checkpoint')
            return result
        finally:self.close()

    def close(self):
        for child in self.children:
            process=child['process']
            if process.poll() is None:process.kill();process.wait(timeout=10)
            if child['pages']:child['pages'].close()
            child['error'].close()


class LinkageStudy:
    def __init__(self, root, repeats):
        self.root=root.resolve();self.repeats=repeats
        self.bundles=json.loads((self.root/'bundles.json').read_text())

    def run(self,smoke):
        cells=[]; witnesses={}
        path=self.root/('smoke.json' if smoke else 'campaign.json')
        if path.exists():raise FileExistsError('refusing to replace a retained campaign')
        for run in range(self.repeats):
            for case in (['chain65k'] if smoke else ['chain65k','hub32_1m']):
                for count in ([1] if smoke else [1,4]):
                    order=['static','dynamic'] if (run+count)%2 else ['dynamic','static']
                    for mode in order:
                        observation=HostState.windows_observation()
                        if observation['available_bytes']<8*2**30:raise RuntimeError('insufficient host memory headroom')
                        directory=self.root/'raw'/f'{"smoke-" if smoke else ""}{run}-{case}-{count}-{mode}'
                        directory.mkdir(parents=True,exist_ok=False)
                        binaries=[Path(p) for p in self.bundles[mode]['executables'][:count]]
                        started=datetime.now(timezone.utc).isoformat()
                        phases=ResidentCohort(binaries,case,directory).run()
                        row=phases[-1]['checkpoints'][0];key=(row['checksum'],row['steps'])
                        if witnesses.setdefault(case,key)!=key:raise ValueError('cross-linkage full-vector mismatch')
                        cell=dict(run=run,case=case,count=count,linkage=mode,started=started,
                                  host=observation,phases=phases)
                        cells.append(cell)
                        (directory/'observations.json').write_text(json.dumps(cell,indent=2)+'\n')
                        path.write_text(json.dumps(dict(complete=False,cells=cells),indent=2)+'\n')
                        print(f'Completed {len(cells)} cohorts: {case}, {count} clients, {mode}',flush=True)
        path.write_text(json.dumps(dict(complete=True,repeats=self.repeats,smoke=smoke,cells=cells),indent=2)+'\n')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--repeats',type=int,default=5)
    parser.add_argument('--smoke',action='store_true')
    parser.add_argument('--enable-memory-measurement',action='store_true')
    args=parser.parse_args()
    if not args.enable_memory_measurement:parser.error('explicit measurement opt-in required')
    if not 1<=args.repeats<=5:parser.error('repeats must be 1..5')
    LinkageStudy(args.root,args.repeats).run(args.smoke)
