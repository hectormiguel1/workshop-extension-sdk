//! Allocation-free log lines. Hooks run on the game's threads, often while
//! the game's allocator is mid-operation, so format into a fixed stack
//! buffer ([`Line`]) and hand the bytes to [`crate::host::Host::log_bytes`].
use core::fmt;

/// Default [`Line`] capacity.
pub const DEFAULT_LINE: usize = 192;

/// A truncating UTF-8 line of at most `N` bytes, built on the stack.
///
/// Writes never fail: text past the capacity is dropped at a char
/// boundary, so the line always stays valid UTF-8.
pub struct Line<const N: usize = DEFAULT_LINE> {
    buffer: [u8; N],
    len: usize,
}

impl<const N: usize> Line<N> {
    /// An empty line.
    pub const fn new() -> Self {
        Self {
            buffer: [0; N],
            len: 0,
        }
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.get(..self.len).unwrap_or_default()
    }

    /// The text written so far.
    pub fn as_str(&self) -> &str {
        // Only whole chars are copied, so the prefix is valid UTF-8.
        core::str::from_utf8(self.as_bytes()).unwrap_or_default()
    }

    /// Bytes written.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing was written.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for Line<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Write for Line<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = N - self.len;
        // Cut at a char boundary so the line stays valid UTF-8.
        let mut take = s.len().min(room);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        if let (Some(dst), Some(src)) = (
            self.buffer.get_mut(self.len..self.len + take),
            s.as_bytes().get(..take),
        ) {
            dst.copy_from_slice(src);
            self.len += take;
        }
        Ok(())
    }
}

impl<const N: usize> fmt::Display for Line<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Format `args` into a [`Line`] of the inferred capacity.
pub fn line<const N: usize>(args: fmt::Arguments<'_>) -> Line<N> {
    let mut out = Line::new();
    let _ = fmt::write(&mut out, args);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write as _;

    #[test]
    fn formats_and_truncates_at_char_boundaries() {
        let l: Line = line(format_args!("scene {} hp {}", 2, 1234));
        assert_eq!(l.as_bytes(), b"scene 2 hp 1234");
        assert_eq!(
            (l.as_str(), l.len(), l.is_empty()),
            ("scene 2 hp 1234", 15, false)
        );
        assert_eq!(l.to_string(), "scene 2 hp 1234");
        let long = "é".repeat(DEFAULT_LINE);
        let l: Line = line(format_args!("{long}"));
        assert_eq!(l.as_bytes().len(), DEFAULT_LINE);
        let l: Line = line(format_args!("x{long}"));
        assert_eq!(l.as_bytes().len(), DEFAULT_LINE - 1);
        assert!(core::str::from_utf8(l.as_bytes()).is_ok());
        assert!(Line::<4>::default().is_empty());
        let mut small = Line::<3>::new();
        small.write_str("abcd").unwrap();
        small.write_str("e").unwrap();
        assert_eq!(small.as_str(), "abc");
    }
}
