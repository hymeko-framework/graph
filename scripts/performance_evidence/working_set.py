"""Windows resident-page observations. No workload or timing campaign runs here."""
from bisect import bisect_right
import ctypes
import mmap
import os
from host import ProcessMemory


class WindowsWorkingSet:
    """Private/sharable page accounting; image identities are not physical PFNs."""
    class ModuleInfo(ctypes.Structure):
        _fields_ = [('base', ctypes.c_void_p), ('size', ctypes.c_uint32), ('entry', ctypes.c_void_p)]

    def __init__(self, pid):
        if os.name != 'nt' or ctypes.sizeof(ctypes.c_void_p) != 8:
            raise RuntimeError('64-bit native Windows required')
        self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        self.psapi = ctypes.WinDLL('psapi', use_last_error=True)
        self.kernel.OpenProcess.argtypes = [ctypes.c_uint32,ctypes.c_int,ctypes.c_uint32]
        self.kernel.OpenProcess.restype = ctypes.c_void_p
        self.kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        self.handle = self.kernel.OpenProcess(0x0400 | 0x0010, False, pid)
        if not self.handle: raise ctypes.WinError(ctypes.get_last_error())
        self.pid = pid
        self.psapi.QueryWorkingSet.argtypes = [ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32]
        self.psapi.EnumProcessModulesEx.argtypes = [ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32]
        self.psapi.GetModuleInformation.argtypes = [ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32]
        self.psapi.GetModuleFileNameExW.argtypes = [ctypes.c_void_p,ctypes.c_void_p,ctypes.c_wchar_p,ctypes.c_uint32]

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle); self.handle = None

    def modules(self):
        handles = (ctypes.c_void_p * 1024)(); needed = ctypes.c_uint32()
        if not self.psapi.EnumProcessModulesEx(self.handle,handles,ctypes.sizeof(handles),ctypes.byref(needed),3):
            raise ctypes.WinError(ctypes.get_last_error())
        if needed.value > ctypes.sizeof(handles): raise RuntimeError('module budget exceeded')
        result=[]
        for handle in handles[:needed.value//ctypes.sizeof(ctypes.c_void_p)]:
            info=self.ModuleInfo(); name=ctypes.create_unicode_buffer(32768)
            if not self.psapi.GetModuleInformation(self.handle,handle,ctypes.byref(info),ctypes.sizeof(info)):
                raise ctypes.WinError(ctypes.get_last_error())
            if not self.psapi.GetModuleFileNameExW(self.handle,handle,name,len(name)):
                raise ctypes.WinError(ctypes.get_last_error())
            result.append((int(info.base),int(info.size),name.value.casefold()))
        return sorted(result)

    @staticmethod
    def classify(entries, modules, page_size=4096):
        if page_size != 4096: raise ValueError('PSAPI layout requires 4 KiB pages in this client')
        bases=[m[0] for m in modules]
        private=sharable=actually_shared=other=0; images=set(); saturated=0
        for flags in entries:
            address=(flags>>12)*page_size
            if not ((flags>>8)&1): private+=page_size; continue
            sharable+=page_size
            shares=(flags>>5)&7
            actually_shared+=page_size if shares>1 else 0
            saturated+=int(shares==7)
            index=bisect_right(bases,address)-1
            if index>=0 and address < modules[index][0]+modules[index][1]:
                base,_,path=modules[index]
                # Different image bases are kept separate (e.g. relocated images).
                images.add((path,base,(address-base)//page_size))
            else: other+=page_size
        return dict(private_working_set_b=private,sharable_working_set_b=sharable,
                    shared_with_other_processes_b=actually_shared,other_sharable_b=other,
                    share_count_saturated_pages=saturated,image_pages=sorted(images),
                    enumerated_working_set_b=private+sharable)

    def snapshot(self):
        modules=self.modules(); capacity=65536
        for _ in range(8):
            if capacity>4_194_304: raise RuntimeError('resident page budget exceeded')
            buffer=(ctypes.c_size_t*(capacity+1))()
            if self.psapi.QueryWorkingSet(self.handle,buffer,ctypes.sizeof(buffer)):
                count=buffer[0]
                if count>capacity: raise RuntimeError('invalid working-set length')
                result=self.classify(buffer[1:count+1],modules,mmap.PAGESIZE)
                counters=ProcessMemory.snapshot(self.pid,ctypes.c_void_p(self.handle))
                if counters is None: raise RuntimeError('process counters unavailable')
                result.update(counters,modules=modules,pid=self.pid)
                return result
            error=ctypes.get_last_error()
            if error != 24: raise ctypes.WinError(error)
            capacity=max(capacity*2,int(buffer[0])+4096)
        raise RuntimeError('working set did not stabilize')

    @staticmethod
    def cohort(rows):
        if not rows: raise ValueError('empty cohort')
        keys={tuple(key) for row in rows for key in row['image_pages']}
        private=sum(r['private_working_set_b'] for r in rows)
        other=sum(r['other_sharable_b'] for r in rows)
        return dict(private_working_set_b=private,
                    private_committed_b=sum(r['private_committed_b'] for r in rows),
                    summed_working_set_b=sum(r['enumerated_working_set_b'] for r in rows),
                    summed_sharable_b=sum(r['sharable_working_set_b'] for r in rows),
                    unique_image_mapping_b=len(keys)*mmap.PAGESIZE,
                    image_deduplicated_estimate_b=private+len(keys)*mmap.PAGESIZE+other,
                    other_sharable_counted_per_process_b=other,
                    semantics='resident image file/base/page identities deduplicated; not PFN-exact physical RAM')
