//! Host-owned persistent storage: one `mod-state.dat` per mounted mod root,
//! split into regions by stable ID. Never the game's save files.
//!
//! A region is registered once, in `init` ([`crate::unit::Init::storage`]);
//! the returned [`Region`] handle is session-local and thread-safe. A new
//! region reads as `Ok(None)` until its first [`Region::write`]. Every
//! successful write is durable and atomic; after `STATE_IO_ERROR` the
//! outcome is unknown, so reopen next session instead of retrying with
//! stale state. Pack the bytes with [`crate::packed`].
use crate::abi::{self, StateApi, StateInfo, StateRange, status};

/// Region metadata ([`StateInfo`] without its header fields).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionInfo {
    /// Schema of the stored bytes (> 0).
    pub schema: u32,
    /// Logical length in bytes.
    pub length: u64,
    /// Revision to pass to the next write.
    pub revision: u64,
}

impl From<StateInfo> for RegionInfo {
    fn from(i: StateInfo) -> Self {
        Self {
            schema: i.schema,
            length: i.length,
            revision: i.revision,
        }
    }
}

/// Whether `id` is a valid region ID: 1..=64 ASCII letters, digits, `._-`.
pub fn valid_id(id: &str) -> bool {
    (1..=abi::STATE_MAX_ID).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A registered host storage region.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    api: StateApi,
    handle: u32,
}

// SAFETY: the host documents every storage callback as thread-safe.
unsafe impl Send for Region {}
// SAFETY: as above; `Region` is immutable.
unsafe impl Sync for Region {}

fn blank() -> StateInfo {
    StateInfo {
        size: size_of::<StateInfo>() as u32,
        ..StateInfo::default()
    }
}

impl Region {
    /// Register `id` on a raw table. Prefer [`crate::unit::Init::storage`],
    /// which validates the table and enforces the init scope.
    ///
    /// # Safety
    /// Only on the thread running the unit's `init` (host contract), with a
    /// table the host advertised.
    pub unsafe fn register_raw(api: &StateApi, id: &str) -> Result<Self, i32> {
        Self::register(api, id)
    }

    /// Register `id` with a validated table (see [`crate::unit::Init::storage`]).
    pub(crate) fn register(api: &StateApi, id: &str) -> Result<Self, i32> {
        if !valid_id(id) {
            return Err(status::INVALID_ARGUMENT);
        }
        let register = api.register_region.ok_or(status::INVALID_ARGUMENT)?;
        let mut handle = 0;
        // SAFETY: host contract; `id` and `handle` are valid for the call.
        match unsafe { register(id.as_ptr(), id.len(), &mut handle) } {
            status::OK if handle != 0 => Ok(Self { api: *api, handle }),
            status::OK => Err(status::INVALID_ARGUMENT),
            rc => Err(rc),
        }
    }

    /// The session-local handle.
    pub const fn handle(&self) -> u32 {
        self.handle
    }

    /// Metadata, or `Ok(None)` for a region never written (`NOT_FOUND`).
    pub fn info(&self) -> Result<Option<RegionInfo>, i32> {
        let info = self.api.info.ok_or(status::INVALID_ARGUMENT)?;
        let mut out = blank();
        // SAFETY: host contract; `out` is writable.
        match unsafe { info(self.handle, &mut out) } {
            status::OK if (out.size as usize) >= size_of::<StateInfo>() => Ok(Some(out.into())),
            status::OK => Err(status::DECODE_FAILED),
            status::NOT_FOUND => Ok(None),
            rc => Err(rc),
        }
    }

    /// Read `out.len()` bytes at `offset`.
    pub fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), i32> {
        let read = self.api.read_at.ok_or(status::INVALID_ARGUMENT)?;
        // SAFETY: `out` is writable for its length.
        match unsafe { read(self.handle, offset, out.as_mut_ptr(), out.len()) } {
            status::OK => Ok(()),
            rc => Err(rc),
        }
    }

    /// The whole region, or `Ok(None)` when never written.
    pub fn read_all(&self) -> Result<Option<(RegionInfo, Vec<u8>)>, i32> {
        let Some(info) = self.info()? else {
            return Ok(None);
        };
        let length = usize::try_from(info.length).map_err(|_| status::STATE_LIMIT)?;
        if info.length > abi::STATE_MAX_REGION_BYTES {
            return Err(status::STATE_LIMIT);
        }
        let mut bytes = vec![0u8; length];
        for (i, chunk) in bytes.chunks_mut(abi::STATE_MAX_BATCH_BYTES).enumerate() {
            self.read((i * abi::STATE_MAX_BATCH_BYTES) as u64, chunk)?;
        }
        Ok(Some((info, bytes)))
    }

    /// One atomic batch: every `(offset, bytes)` range, the new logical
    /// `length` and `schema`, against the `expected` revision (0 for a new
    /// region). Returns the new metadata.
    pub fn write_ranges(
        &self,
        expected: u64,
        length: u64,
        schema: u32,
        ranges: &[(u64, &[u8])],
    ) -> Result<RegionInfo, i32> {
        let write = self.api.write_batch.ok_or(status::INVALID_ARGUMENT)?;
        if ranges.len() > abi::STATE_MAX_RANGES {
            return Err(status::STATE_LIMIT);
        }
        let mut raw = [StateRange {
            offset: 0,
            data: core::ptr::null(),
            length: 0,
        }; abi::STATE_MAX_RANGES];
        for (slot, (offset, data)) in raw.iter_mut().zip(ranges) {
            *slot = StateRange {
                offset: *offset,
                data: data.as_ptr(),
                length: data.len(),
            };
        }
        let mut out = blank();
        // SAFETY: `raw[..len]` points at caller slices valid for the call;
        // `out` is writable.
        let rc = unsafe {
            write(
                self.handle,
                expected,
                length,
                schema,
                0,
                raw.as_ptr(),
                ranges.len(),
                &mut out,
            )
        };
        match rc {
            status::OK => Ok(out.into()),
            rc => Err(rc),
        }
    }

    /// Replace the whole region with `data` (one range, or none when empty).
    pub fn write(&self, expected: u64, schema: u32, data: &[u8]) -> Result<RegionInfo, i32> {
        let range = [(0u64, data)];
        let ranges: &[(u64, &[u8])] = if data.is_empty() { &[] } else { &range };
        self.write_ranges(expected, data.len() as u64, schema, ranges)
    }

    /// Ask the host to flush (no-op when the host has no flush entry).
    pub fn flush(&self) -> Result<(), i32> {
        match self.api.flush {
            // SAFETY: host contract.
            Some(flush) => match unsafe { flush() } {
                status::OK => Ok(()),
                rc => Err(rc),
            },
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
