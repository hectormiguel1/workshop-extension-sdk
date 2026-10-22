//! A unit DLL's export table, read from the PE32 file: a unit must export
//! exactly `init` and `shutdown` ([`is_unit`]). Used by the `export_check`
//! example and by tests; portable (no Windows API).

/// The two exports of every unit, in PE name-table order.
pub const UNIT_EXPORTS: [&str; 2] = ["init", "shutdown"];

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Export names of a PE32 (x86) image file, in name-table order; `None`
/// when the bytes are not a well-formed PE32 image.
pub fn exports(b: &[u8]) -> Option<Vec<String>> {
    let pe = u32_at(b, 0x3c)? as usize;
    if u32_at(b, pe)? != 0x4550 || u16_at(b, pe + 24)? != 0x10b {
        return None;
    }
    let sections = u16_at(b, pe + 6)? as usize;
    let optional = u16_at(b, pe + 20)? as usize;
    let export_rva = u32_at(b, pe + 24 + 96)? as usize;
    let table = pe + 24 + optional;
    let file_offset = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let va = u32_at(b, s + 12)? as usize;
            let size = (u32_at(b, s + 8)? as usize).max(u32_at(b, s + 16)? as usize);
            (va..va + size)
                .contains(&rva)
                .then(|| Some(rva - va + u32_at(b, s + 20)? as usize))?
        })
    };
    if export_rva == 0 {
        return Some(Vec::new());
    }
    let dir = file_offset(export_rva)?;
    let count = u32_at(b, dir + 24)? as usize;
    let names = file_offset(u32_at(b, dir + 32)? as usize)?;
    (0..count)
        .map(|i| {
            let at = file_offset(u32_at(b, names + i * 4)? as usize)?;
            let end = at + b.get(at..)?.iter().position(|&c| c == 0)?;
            Some(String::from_utf8_lossy(&b[at..end]).into_owned())
        })
        .collect()
}

/// Whether `names` is exactly [`UNIT_EXPORTS`].
pub fn is_unit(names: &[String]) -> bool {
    names == UNIT_EXPORTS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pe_image as image;

    #[test]
    fn reads_export_names_and_checks_the_unit_pair() {
        let ok = exports(&image(&["init", "shutdown"], 0x10b)).unwrap();
        assert!(is_unit(&ok));
        let extra = exports(&image(&["DllMain", "init", "shutdown"], 0x10b)).unwrap();
        assert!(!is_unit(&extra));
        assert_eq!(exports(&image(&[], 0x10b)), Some(vec![]));
        assert_eq!(exports(&image(&["init"], 0x20b)), None);
        assert_eq!(exports(b"MZ"), None);
        let mut bad = image(&["init"], 0x10b);
        bad[0x240..0x244].copy_from_slice(&0x9000u32.to_le_bytes());
        assert_eq!(exports(&bad), None);
    }
}
