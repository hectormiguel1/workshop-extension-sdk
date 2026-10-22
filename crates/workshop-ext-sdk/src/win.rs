//! Windows-only helpers for units: process memory queries for the site
//! policy ([`LiveMemory`]), code writes and module pinning. Minimal
//! hand-written kernel32 bindings; no other dependency.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use crate::sites::Memory;
use core::ffi::c_void;

const MEM_COMMIT: u32 = 0x1000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_GUARD: u32 = 0x100;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
/// Any of READONLY, READWRITE, WRITECOPY and their EXECUTE_ variants.
const PAGE_READABLE: u32 = 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;
/// EXECUTE, EXECUTE_READ, EXECUTE_READWRITE, EXECUTE_WRITECOPY.
const PAGE_EXECUTABLE: u32 = 0x10 | 0x20 | 0x40 | 0x80;
const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 1;
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 4;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct MEMORY_BASIC_INFORMATION {
    BaseAddress: usize,
    AllocationBase: usize,
    AllocationProtect: u32,
    #[cfg(target_pointer_width = "64")]
    PartitionId: u16,
    RegionSize: usize,
    State: u32,
    Protect: u32,
    Type: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualQuery(addr: *const c_void, out: *mut MEMORY_BASIC_INFORMATION, len: usize) -> usize;
    fn VirtualProtect(addr: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, name: *const c_void, out: *mut *mut c_void) -> i32;
}

fn region(at: usize, len: usize) -> Option<MEMORY_BASIC_INFORMATION> {
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: VirtualQuery only reads page tables; `mbi` is writable.
    let n = unsafe { VirtualQuery(at as *const c_void, &mut mbi, size_of_val(&mbi)) };
    let end = at.checked_add(len)?;
    (n != 0
        && mbi.State == MEM_COMMIT
        && mbi.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
        && end <= mbi.BaseAddress.saturating_add(mbi.RegionSize))
    .then_some(mbi)
}

/// The running process's memory, queried with `VirtualQuery` before every
/// read: what [`crate::sites`] consults to accept a foreign detour.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiveMemory;

impl Memory for LiveMemory {
    fn read_u32(&self, at: usize) -> Option<u32> {
        region(at, 4).filter(|m| m.Protect & PAGE_READABLE != 0)?;
        // SAFETY: committed and readable for four bytes.
        Some(unsafe { core::ptr::read_unaligned(at as *const u32) })
    }

    fn executable(&self, at: usize) -> bool {
        region(at, 1).is_some_and(|m| m.Protect & PAGE_EXECUTABLE != 0)
    }
}

/// Write `bytes` at `address` through a temporary `PAGE_EXECUTE_READWRITE`
/// window, then restore the protection and flush the instruction cache.
///
/// # Safety
/// `address..address + bytes.len()` must be mapped, verified code or data
/// that no other thread executes while it changes.
pub unsafe fn write_code(address: usize, bytes: &[u8]) -> bool {
    let at = address as *mut c_void;
    let mut old = 0u32;
    // SAFETY: caller contract; the protection is put back right after.
    unsafe {
        if VirtualProtect(at, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return false;
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
        let mut ignored = 0u32;
        VirtualProtect(at, bytes.len(), old, &mut ignored);
        FlushInstructionCache(GetCurrentProcess(), at, bytes.len());
    }
    true
}

/// Pin the module containing `address` (any function of the unit) for the
/// process lifetime: required before writing hooks that cannot be removed
/// while the game runs.
pub fn pin_module(address: usize) -> bool {
    let mut module = core::ptr::null_mut();
    // SAFETY: `module` is writable; FROM_ADDRESS takes any code address.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            address as *const c_void,
            &mut module,
        ) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static WORD: u32 = 0x1234_5678;

    #[test]
    fn live_memory_queries_the_process() {
        let code = live_memory_queries_the_process as *const () as usize;
        assert!(LiveMemory.executable(code));
        assert!(!LiveMemory.executable(&WORD as *const u32 as usize));
        assert!(!LiveMemory.executable(0));
        assert_eq!(
            LiveMemory.read_u32(&WORD as *const u32 as usize),
            Some(0x1234_5678)
        );
        assert_eq!(LiveMemory.read_u32(0), None);
        assert_eq!(LiveMemory.read_u32(usize::MAX - 1), None);
        assert!(pin_module(code));
        let mut page = vec![0u8; 16];
        assert!(unsafe { write_code(page.as_mut_ptr() as usize, b"\xc3") });
        assert_eq!(page[0], 0xc3);
        let image = crate::image::Image {
            base: page.as_ptr() as usize,
            size: page.len(),
        };
        let mut out = [0u8; 2];
        assert!(unsafe { image.read(0, &mut out) });
        assert!(unsafe { image.write(1, b"\x90") });
        assert!(!unsafe { image.read(15, &mut out) });
        assert!(!unsafe { image.write(16, b"\x90") });
        assert_eq!(page[1], 0x90);
    }
}
