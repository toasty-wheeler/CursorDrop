//! An OLE drop target for the window, so it can react while files are dragged over it
//! (WM_DROPFILES only reports the drop itself). Written by hand: a COM object is just a
//! pointer to a table of functions, here IUnknown's three plus IDropTarget's four.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::core::GUID;

const S_OK: i32 = 0;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const DROPEFFECT_NONE: u32 = 0;
const DROPEFFECT_COPY: u32 = 1;
const CF_HDROP: u16 = 15;
const DVASPECT_CONTENT: u32 = 1;
const TYMED_HGLOBAL: u32 = 1;

#[repr(C)]
struct FormatEtc {
    cf: u16,
    ptd: *mut c_void,
    aspect: u32,
    lindex: i32,
    tymed: u32,
}

#[repr(C)]
struct StgMedium {
    tymed: u32,
    handle: *mut c_void,
    release: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointL {
    x: i32,
    y: i32,
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn OleInitialize(reserved: *mut c_void) -> i32;
    fn RegisterDragDrop(hwnd: HWND, target: *mut c_void) -> i32;
    fn RevokeDragDrop(hwnd: HWND) -> i32;
    fn ReleaseStgMedium(medium: *mut StgMedium);
}

/// What the drop target tells the window. `Enter`/`Over` carry the files being dragged.
pub enum DragEvent {
    Enter(Vec<PathBuf>),
    Over,
    Leave,
    Drop(Vec<PathBuf>),
}

#[repr(C)]
struct Vtbl {
    query_interface: unsafe extern "system" fn(*mut Target, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut Target) -> u32,
    release: unsafe extern "system" fn(*mut Target) -> u32,
    drag_enter: unsafe extern "system" fn(*mut Target, *mut c_void, u32, PointL, *mut u32) -> i32,
    drag_over: unsafe extern "system" fn(*mut Target, u32, PointL, *mut u32) -> i32,
    drag_leave: unsafe extern "system" fn(*mut Target) -> i32,
    drop: unsafe extern "system" fn(*mut Target, *mut c_void, u32, PointL, *mut u32) -> i32,
}

#[repr(C)]
struct Target {
    vtbl: *const Vtbl,
    refs: AtomicU32,
    /// Whether the current drag holds anything we'd accept (decides the cursor's effect).
    accept: AtomicU32,
    handler: fn(DragEvent) -> bool,
}

static VTBL: Vtbl = Vtbl { query_interface, add_ref, release, drag_enter, drag_over, drag_leave, drop };

const IID_IUNKNOWN: GUID = GUID { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IDROPTARGET: GUID = GUID { data1: 0x122, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

fn same_guid(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn query_interface(this: *mut Target, riid: *const GUID, out: *mut *mut c_void) -> i32 {
    unsafe {
        if same_guid(&*riid, &IID_IUNKNOWN) || same_guid(&*riid, &IID_IDROPTARGET) {
            *out = this.cast();
            add_ref(this);
            S_OK
        } else {
            *out = null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn add_ref(this: *mut Target) -> u32 {
    unsafe { (*this).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn release(this: *mut Target) -> u32 {
    // The target lives for the whole process (leaked on purpose), so it is never freed here.
    unsafe { (*this).refs.fetch_sub(1, Ordering::SeqCst).saturating_sub(1).max(1) }
}

/// The files in a drag, via IDataObject::GetData(CF_HDROP).
unsafe fn files_of(data: *mut c_void) -> Vec<PathBuf> {
    if data.is_null() {
        return vec![];
    }
    unsafe {
        type GetData = unsafe extern "system" fn(*mut c_void, *const FormatEtc, *mut StgMedium) -> i32;
        let vtbl = *(data as *const *const usize);
        let get_data: GetData = std::mem::transmute(*vtbl.add(3));
        let fmt = FormatEtc { cf: CF_HDROP, ptd: null_mut(), aspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL };
        let mut medium = StgMedium { tymed: 0, handle: null_mut(), release: null_mut() };
        if get_data(data, &fmt, &mut medium) != S_OK {
            return vec![];
        }
        let files = hdrop_files(medium.handle as HDROP);
        ReleaseStgMedium(&mut medium);
        files
    }
}

/// The file paths in an HDROP (from a drag or WM_DROPFILES).
pub fn hdrop_files(hdrop: HDROP) -> Vec<PathBuf> {
    let mut files = Vec::new();
    unsafe {
        let n = DragQueryFileW(hdrop, 0xFFFF_FFFF, null_mut(), 0);
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, null_mut(), 0);
            let mut buf = vec![0u16; len as usize + 1];
            DragQueryFileW(hdrop, i, buf.as_mut_ptr(), len + 1);
            files.push(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])));
        }
    }
    files
}

unsafe fn effect(this: *mut Target, out: *mut u32) {
    unsafe {
        if !out.is_null() {
            *out = if (*this).accept.load(Ordering::SeqCst) != 0 { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
    }
}

unsafe extern "system" fn drag_enter(this: *mut Target, data: *mut c_void, _keys: u32, _pt: PointL, out: *mut u32) -> i32 {
    unsafe {
        let files = files_of(data);
        let ok = ((*this).handler)(DragEvent::Enter(files));
        (*this).accept.store(ok as u32, Ordering::SeqCst);
        effect(this, out);
    }
    S_OK
}

unsafe extern "system" fn drag_over(this: *mut Target, _keys: u32, _pt: PointL, out: *mut u32) -> i32 {
    unsafe {
        ((*this).handler)(DragEvent::Over);
        effect(this, out);
    }
    S_OK
}

unsafe extern "system" fn drag_leave(this: *mut Target) -> i32 {
    unsafe {
        ((*this).handler)(DragEvent::Leave);
    }
    S_OK
}

unsafe extern "system" fn drop(this: *mut Target, data: *mut c_void, _keys: u32, _pt: PointL, out: *mut u32) -> i32 {
    unsafe {
        let files = files_of(data);
        effect(this, out);
        ((*this).handler)(DragEvent::Drop(files));
        (*this).accept.store(0, Ordering::SeqCst);
    }
    S_OK
}

/// Makes `hwnd` a drop target; `handler` gets the events and, for Enter, says whether the
/// files are welcome. Returns false if OLE drag and drop isn't available (then the caller can
/// fall back to WM_DROPFILES).
pub fn register(hwnd: HWND, handler: fn(DragEvent) -> bool) -> bool {
    unsafe {
        let hr = OleInitialize(null_mut());
        if hr < 0 {
            return false;
        }
        let target = Box::into_raw(Box::new(Target {
            vtbl: &VTBL,
            refs: AtomicU32::new(1),
            accept: AtomicU32::new(0),
            handler,
        }));
        RegisterDragDrop(hwnd, target.cast()) >= 0
    }
}

pub fn unregister(hwnd: HWND) {
    unsafe {
        RevokeDragDrop(hwnd);
    }
}
