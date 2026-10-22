use super::*;
use crate::testing::FakeMemory;

const HEAD: [u8; 9] = [0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x60, 0x89, 0x4D, 0xA0];
const IMAGE: Image = Image {
    base: 0x40_0000,
    size: 0x10_0000,
};
/// `jmp rel32` from 0x43e6e0 to 0x7000_0000 (outside the image).
const ENTRY: usize = 0x43_e6e0;
const DEST: usize = 0x7000_0000;

fn rel32(entry: usize, dest: usize) -> [u8; 5] {
    let rel = (dest as u32).wrapping_sub(entry as u32 + 5);
    let b = rel.to_le_bytes();
    [0xE9, b[0], b[1], b[2], b[3]]
}

fn detoured(jump: &[u8]) -> Vec<u8> {
    let mut found = HEAD.to_vec();
    found[..jump.len()].copy_from_slice(jump);
    found
}

fn memory() -> FakeMemory {
    FakeMemory {
        executable: vec![(DEST, 0x1000)],
        dwords: vec![(0x6000_0000, DEST as u32)],
    }
}

#[test]
fn classify_matches_the_shared_head_policy() {
    let m = memory();
    assert_eq!(classify(ENTRY, &HEAD, &HEAD, IMAGE, &m), Verdict::Retail);
    let rel = detoured(&rel32(ENTRY, DEST));
    assert_eq!(
        classify(ENTRY, &HEAD, &rel, IMAGE, &m),
        Verdict::Foreign {
            jump: Jump::Rel32,
            dest: DEST
        }
    );
    let indirect = detoured(&[0xFF, 0x25, 0x00, 0x00, 0x00, 0x60]);
    assert_eq!(
        classify(ENTRY, &HEAD, &indirect, IMAGE, &m),
        Verdict::Foreign {
            jump: Jump::Indirect,
            dest: DEST
        }
    );
    // Unreadable indirection slot, destination inside the image, not
    // executable, null, a changed byte after the jump, a short head, a
    // different length, and plain other bytes are all mismatches.
    let unreadable = detoured(&[0xFF, 0x25, 0x04, 0x00, 0x00, 0x60]);
    let inside = detoured(&rel32(ENTRY, 0x40_1000));
    let not_exec = detoured(&rel32(ENTRY, 0x7100_0000));
    let null = detoured(&rel32(ENTRY, 0));
    let mut tail = rel.clone();
    tail[8] ^= 1;
    for found in [
        &unreadable,
        &inside,
        &not_exec,
        &null,
        &tail,
        &HEAD[..8].to_vec(),
        &vec![0x90; 9],
    ] {
        assert_eq!(
            classify(ENTRY, &HEAD, found, IMAGE, &m),
            Verdict::Mismatch,
            "{found:x?}"
        );
    }
    let five = &HEAD[..5];
    assert_eq!(
        classify(ENTRY, five, &rel32(ENTRY, DEST), IMAGE, &m),
        Verdict::Mismatch
    );
    assert_eq!(
        classify(ENTRY, &HEAD, &rel, IMAGE, &NoForeignCode),
        Verdict::Mismatch
    );
    assert_eq!(NoForeignCode.read_u32(0), None);
    assert_eq!((Jump::Rel32.size(), Jump::Indirect.size()), (5, 6));
    assert_eq!(
        (Jump::Rel32.text(), Jump::Indirect.text()),
        ("jmp rel32", "jmp [abs32]")
    );
}

#[test]
fn only_call_sites_admit_a_foreign_detour() {
    let foreign = Verdict::Foreign {
        jump: Jump::Rel32,
        dest: DEST,
    };
    for (kind, want) in [
        (SiteKind::Call, true),
        (SiteKind::Hook, false),
        (SiteKind::Anchor, false),
    ] {
        assert!(admits(kind, Verdict::Retail));
        assert_eq!(admits(kind, foreign), want);
        assert!(!admits(kind, Verdict::Mismatch));
    }
}

sites! {
    /// Test table: all byte forms, all kinds.
    const TABLE = {
        "call head" @ 0x3e6e0 = [0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x60, 0x89, 0x4D, 0xA0], call;
        "hook head" @ 0x3e6e0 = HEAD, hook;
        "anchor" @ (0x100 + 0x10) = b"\x01\x02", anchor;
        "slice expr" @ 0x200 = { HEAD.split_at(3).0 } anchor;
    }
}

fn reader(image: &[u8]) -> impl Fn(u32, &mut [u8]) -> bool + '_ {
    move |rva, out| match image.get(rva as usize..rva as usize + out.len()) {
        Some(src) => {
            out.copy_from_slice(src);
            true
        }
        None => false,
    }
}

fn image_bytes() -> Vec<u8> {
    let mut image = vec![0xCC; 0x4_0000];
    for s in TABLE {
        image[s.rva as usize..s.rva as usize + s.bytes.len()].copy_from_slice(s.bytes);
    }
    image
}

#[test]
fn verify_reports_problems_and_accepted_detours() {
    assert_eq!(TABLE.len(), 4);
    assert_eq!(TABLE[2].rva, 0x110);
    assert_eq!(TABLE[3].bytes, &HEAD[..3]);
    let mut image = image_bytes();
    let mut seen = Vec::new();
    let summary = verify(TABLE, IMAGE, &reader(&image), &memory(), &mut |s, f| {
        seen.push((s.name, f))
    });
    assert!(summary.passed() && seen.is_empty());
    assert_eq!(summary, Summary::default());
    // Another component detours the head: the call site accepts it, the
    // hook site refuses to stomp it.
    image[0x3e6e0..0x3e6e5].copy_from_slice(&rel32(ENTRY, DEST));
    image[0x110] = 9;
    let summary = verify(TABLE, IMAGE, &reader(&image), &memory(), &mut |s, f| {
        seen.push((s.name, f))
    });
    assert_eq!(
        summary,
        Summary {
            failed: 2,
            foreign: 2
        }
    );
    assert!(!summary.passed());
    let foreign = Finding::Foreign {
        jump: Jump::Rel32,
        dest: DEST,
    };
    let occupied = Finding::Problem(Problem::Occupied {
        jump: Jump::Rel32,
        dest: DEST,
    });
    assert_eq!(
        seen,
        [
            ("call head", foreign),
            ("hook head", occupied),
            ("anchor", Finding::Problem(Problem::Bytes))
        ]
    );
    // A short image and an unreadable one.
    let small = Image {
        base: IMAGE.base,
        size: 0x150,
    };
    let mut out = Vec::new();
    let summary = verify(
        TABLE,
        small,
        &reader(&image),
        &NoForeignCode,
        &mut |_, f| out.push(f),
    );
    assert_eq!(summary.failed, 4);
    assert!(out.contains(&Finding::Problem(Problem::OutsideImage)));
    let summary = verify(TABLE, IMAGE, &|_, _| false, &NoForeignCode, &mut |_, _| {});
    assert_eq!(summary.failed, TABLE.len());
    let long = Site {
        name: "too long",
        rva: 0,
        bytes: &[0; MAX_SITE_BYTES + 1],
        kind: SiteKind::Anchor,
    };
    assert_eq!(
        check(&long, IMAGE, &reader(&image), &NoForeignCode),
        Err(Problem::OutsideImage)
    );
    assert!(!check_table(&[long]));
    assert!(!check_table(&[Site { bytes: &[], ..long }]));
    assert!(check_table(TABLE));
}

#[test]
fn problems_and_bytes_display() {
    assert_eq!(
        Problem::OutsideImage.to_string(),
        "outside the loaded image"
    );
    assert_eq!(
        Problem::Bytes.to_string(),
        "bytes differ from the verified build"
    );
    assert_eq!(
        Problem::Occupied {
            jump: Jump::Indirect,
            dest: 0x1234
        }
        .to_string(),
        "already hooked by another component (jmp [abs32] -> 0x00001234); not overwritten"
    );
    assert_eq!(hex_bytes(&[0x0f, 0xb6, 0x45]).to_string(), "0f b6 45");
    assert_eq!(hex_bytes(&[]).to_string(), "");
}
