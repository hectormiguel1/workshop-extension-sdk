//! The game's loaded main image (`game_base`, `game_size`): RVA bounds and,
//! on Windows, bounded reads and `VirtualProtect` code writes.

/// The main executable's mapped image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Image {
    /// Load address (`HostContext::game_base`).
    pub base: usize,
    /// `SizeOfImage` (`HostContext::game_size`).
    pub size: usize,
}

impl Image {
    /// Whether `rva..rva + len` lies inside the image.
    pub const fn contains(&self, rva: u32, len: usize) -> bool {
        match (rva as usize).checked_add(len) {
            Some(end) => end <= self.size,
            None => false,
        }
    }

    /// Whether the absolute `address` lies inside the image.
    pub const fn holds(&self, address: usize) -> bool {
        address >= self.base && address - self.base < self.size
    }

    /// The absolute address of `rva`.
    pub const fn address(&self, rva: u32) -> usize {
        self.base.wrapping_add(rva as usize)
    }
}

#[cfg(windows)]
impl Image {
    /// Copy `out.len()` bytes at `rva`; `false` outside the image.
    ///
    /// # Safety
    /// `base..base + size` must be the mapped image.
    pub unsafe fn read(&self, rva: u32, out: &mut [u8]) -> bool {
        if !self.contains(rva, out.len()) {
            return false;
        }
        // SAFETY: inside the mapped image (caller contract).
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.address(rva) as *const u8,
                out.as_mut_ptr(),
                out.len(),
            )
        };
        true
    }

    /// Write `bytes` at `rva` through a temporary `PAGE_EXECUTE_READWRITE`
    /// window, restore the protection and flush the instruction cache.
    /// `false` outside the image or when the protection cannot be changed.
    ///
    /// # Safety
    /// `base..base + size` must be the mapped image, and the bytes must be
    /// verified code/data no other thread executes mid-write.
    pub unsafe fn write(&self, rva: u32, bytes: &[u8]) -> bool {
        if !self.contains(rva, bytes.len()) {
            return false;
        }
        // SAFETY: caller contract.
        unsafe { crate::win::write_code(self.address(rva), bytes) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_addresses() {
        let image = Image {
            base: 0x40_0000,
            size: 0x1000,
        };
        assert!(image.contains(0xff0, 0x10));
        assert!(!image.contains(0xff0, 0x11));
        assert!(!image.contains(u32::MAX, usize::MAX));
        assert!(image.holds(0x40_0fff) && !image.holds(0x40_1000) && !image.holds(0x3f_ffff));
        assert_eq!(image.address(0x10), 0x40_0010);
        assert_eq!(Image::default().size, 0);
    }
}
