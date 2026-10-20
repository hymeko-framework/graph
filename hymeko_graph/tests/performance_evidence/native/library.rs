//! Explicit native benchmark dependency, never loaded by production code.
use std::{
    ffi::{CStr, c_void},
    path::Path,
    ptr::NonNull,
};

/// Own the module even while symbol admission can fail. Graph borrows still
/// prevent unloading the library before its exported destructors have run.
struct Module(NonNull<c_void>);
impl Module {
    fn open(path: &Path) -> Result<Self, String> {
        platform::open(path).map(Self)
    }
    fn symbol(&self, name: &CStr) -> Result<NonNull<c_void>, String> {
        // SAFETY: self keeps the module live and CStr supplies a terminated name.
        unsafe { platform::symbol(self.0, name) }
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: this owner closes its one module handle after all borrows end.
        unsafe { platform::close(self.0) };
    }
}

type Create = unsafe extern "C" fn(u32, u32, *const u32, *const u32, u32) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Rank = unsafe extern "C" fn(*mut c_void, *mut f64, u32, *mut u64) -> u32;
type Components = unsafe extern "C" fn(*mut c_void, *mut u32, u32, *mut u32) -> u32;

pub(super) struct Library {
    _module: Module,
    create: Create,
    destroy: Destroy,
    rank: Rank,
    components: Components,
}
impl Library {
    pub(super) fn load() -> Self {
        let path = std::env::var_os("HYMEKO_NATIVE_LIBRARY").expect("explicit native adapter path");
        assert!(std::path::Path::new(&path).is_absolute());
        let module = Module::open(Path::new(&path)).expect("load adapter");
        let symbol =
            |name: &std::ffi::CStr| module.symbol(name).expect("native adapter symbol").as_ptr();
        // SAFETY: signatures exactly match the archived native_adapter.zig C ABI.
        // The module remains live until all graph borrows end.
        unsafe {
            Self {
                create: std::mem::transmute::<*mut c_void, Create>(symbol(c"hgz_native_create")),
                destroy: std::mem::transmute::<*mut c_void, Destroy>(symbol(c"hgz_native_destroy")),
                rank: std::mem::transmute::<*mut c_void, Rank>(symbol(c"hgz_native_rank")),
                components: std::mem::transmute::<*mut c_void, Components>(symbol(
                    c"hgz_native_components",
                )),
                _module: module,
            }
        }
    }
    pub(super) fn build(&self, nodes: usize, offsets: &[u32], members: &[u32]) -> Graph<'_> {
        assert!(!offsets.is_empty());
        assert_eq!(offsets[0], 0);
        assert_eq!(*offsets.last().unwrap() as usize, members.len());
        assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(members.iter().all(|&member| (member as usize) < nodes));
        let nodes = u32::try_from(nodes).unwrap();
        let edges = u32::try_from(offsets.len() - 1).unwrap();
        let length = u32::try_from(members.len()).unwrap();
        // SAFETY: all arrays have the checked lengths and bounds; the adapter copies
        // input into its own graph and keeps no pointers into these slices.
        let raw =
            unsafe { (self.create)(nodes, edges, offsets.as_ptr(), members.as_ptr(), length) };
        Graph {
            raw: NonNull::new(raw).expect("native construction"),
            nodes,
            owner: self,
        }
    }
}
pub(super) struct Graph<'a> {
    raw: NonNull<c_void>,
    nodes: u32,
    owner: &'a Library,
}
impl Graph<'_> {
    pub(super) fn rank(&mut self, scores: &mut [f64]) -> usize {
        assert_eq!(scores.len(), self.nodes as usize);
        let mut stats = [0u64; 2];
        // SAFETY: unique live adapter graph, exact output lengths, no retained pointers.
        assert_eq!(
            unsafe {
                (self.owner.rank)(
                    self.raw.as_ptr(),
                    scores.as_mut_ptr(),
                    self.nodes,
                    stats.as_mut_ptr(),
                )
            },
            1
        );
        assert_eq!(stats[1], 1, "upstream rank did not converge");
        stats[0] as usize
    }
    pub(super) fn components(&mut self, labels: &mut [u32]) -> u32 {
        self.try_components(labels).expect("native components")
    }
    pub(super) fn try_components(&mut self, labels: &mut [u32]) -> Result<u32, u32> {
        assert_eq!(labels.len(), self.nodes as usize);
        let mut count = 0;
        // SAFETY: unique live adapter graph, exact output lengths, no retained pointers.
        let status = unsafe {
            (self.owner.components)(
                self.raw.as_ptr(),
                labels.as_mut_ptr(),
                self.nodes,
                &mut count,
            )
        };
        if status == 1 { Ok(count) } else { Err(status) }
    }
}
impl Drop for Graph<'_> {
    fn drop(&mut self) {
        // SAFETY: unique graph handle from this still-live adapter, destroyed once.
        unsafe { (self.owner.destroy)(self.raw.as_ptr()) };
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    pub(super) fn open(path: &Path) -> Result<NonNull<c_void>, String> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err("NUL in library path".into());
        }
        wide.push(0);
        // SAFETY: terminated Windows path; the returned handle has one owner.
        NonNull::new(unsafe { LoadLibraryW(wide.as_ptr()) })
            .ok_or_else(|| std::io::Error::last_os_error().to_string())
    }
    pub(super) unsafe fn symbol(
        module: NonNull<c_void>,
        name: &CStr,
    ) -> Result<NonNull<c_void>, String> {
        // SAFETY: the caller keeps module live; name is terminated.
        NonNull::new(unsafe { GetProcAddress(module.as_ptr(), name.as_ptr().cast()) })
            .ok_or_else(|| std::io::Error::last_os_error().to_string())
    }
    pub(super) unsafe fn close(module: NonNull<c_void>) {
        // SAFETY: the caller transfers its live, uniquely owned module handle.
        assert_ne!(unsafe { FreeLibrary(module.as_ptr()) }, 0);
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, flags: std::ffi::c_int) -> *mut c_void;
        fn dlsym(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
        fn dlerror() -> *const std::ffi::c_char;
        fn dlclose(module: *mut c_void) -> std::ffi::c_int;
    }
    fn error() -> String {
        // SAFETY: dlerror returns null or a thread-local terminated error string.
        let pointer = unsafe { dlerror() };
        if pointer.is_null() {
            "dynamic library lookup failed".into()
        } else {
            // SAFETY: the pointer is valid until the next loader operation.
            unsafe { CStr::from_ptr(pointer) }
                .to_string_lossy()
                .into_owned()
        }
    }
    pub(super) fn open(path: &Path) -> Result<NonNull<c_void>, String> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        // SAFETY: terminated path; RTLD_NOW resolves dependencies before use.
        // RTLD_LOCAL is the default, so adapter symbols do not become global.
        NonNull::new(unsafe { dlopen(path.as_ptr(), 2) }).ok_or_else(error)
    }
    pub(super) unsafe fn symbol(
        module: NonNull<c_void>,
        name: &CStr,
    ) -> Result<NonNull<c_void>, String> {
        // SAFETY: the caller keeps module live; name is terminated.
        NonNull::new(unsafe { dlsym(module.as_ptr(), name.as_ptr()) }).ok_or_else(error)
    }
    pub(super) unsafe fn close(module: NonNull<c_void>) {
        // SAFETY: the caller transfers its live, uniquely owned module handle.
        assert_eq!(unsafe { dlclose(module.as_ptr()) }, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_is_rejected() {
        let path = std::env::temp_dir().join("hymeko-does-not-exist-library-581d6387");
        assert!(Module::open(&path).is_err());
    }

    #[test]
    fn module_lifetime_supports_symbol_lookup_and_missing_symbol_rejection() {
        #[cfg(windows)]
        let (path, symbol) = (Path::new("kernel32.dll"), c"GetCurrentProcessId");
        #[cfg(target_os = "linux")]
        let (path, symbol) = (Path::new("libc.so.6"), c"getpid");
        let module = Module::open(path).unwrap();
        assert!(module.symbol(symbol).is_ok());
        assert!(module.symbol(c"hymeko_missing_symbol_581d6387").is_err());
        assert!(module.symbol(symbol).is_ok());
    }
}
