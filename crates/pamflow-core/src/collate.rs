//! Sorting text the way R does (`sort`, `list.files`).
//!
//! On Windows R compares with the C library's `strcoll`, which asks Windows:
//! `CompareStringEx(user locale, SORT_STRINGSORT)`. Calling it directly gives
//! exactly R's order (checked against R on punctuation, case, digits and
//! accents: spikes/collate). Elsewhere a close approximation is used:
//! case-insensitive, lowercase first.

use std::cmp::Ordering;

#[cfg(windows)]
pub fn compare(a: &str, b: &str) -> Ordering {
    use windows_sys::Win32::Globalization::{CompareStringEx, SORT_STRINGSORT};
    let wa: Vec<u16> = a.encode_utf16().collect();
    let wb: Vec<u16> = b.encode_utf16().collect();
    // SAFETY: both buffers live for the call and their lengths are passed.
    let r = unsafe {
        CompareStringEx(
            std::ptr::null(), // LOCALE_NAME_USER_DEFAULT
            SORT_STRINGSORT,
            wa.as_ptr(),
            wa.len() as i32,
            wb.as_ptr(),
            wb.len() as i32,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    match r {
        1 => Ordering::Less,
        3 => Ordering::Greater,
        2 => a.cmp(b), // equal for Windows (e.g. only case differs at no level): keep a fixed order
        _ => fallback(a, b),
    }
}

#[cfg(not(windows))]
pub fn compare(a: &str, b: &str) -> Ordering {
    fallback(a, b)
}

fn fallback(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

/// Sort like R's `sort()`.
pub fn sort(v: &mut [String]) {
    v.sort_by(|a, b| compare(a, b));
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn same_order_as_r() {
        // R 4.5 sort() on Windows, English locale
        let want = [
            "a/#x.wav", "a/(x).wav", "a/_x.wav", "a/~x.wav", "a/1.wav", "a/10.wav", "a/9.wav", "a/a-b.wav",
            "a/a b.wav", "a/a_b.wav", "a/ab-.wav", "a/ab.wav", "a/aB.wav", "a/Ab.wav", "a/AB.wav", "a/deep/x.wav",
            "a/e.wav", "a/é.wav", "a/f.wav", "a/G-0007/x.wav", "a/G 0007/x.wav", "a/G.0007/x.wav", "a/G_0007/x.wav",
            "a/g0001_x.wav", "a/G0001_x.wav", "a/G0007/x.wav", "a/G00070.wav", "a/G0007x.wav", "a/x(1).wav",
            "a/x.flac", "a/x.FLAC", "a/x1.wav", "a/xa.wav", "a/z.wav", "a/Z.wav",
        ];
        let mut v: Vec<String> = want.iter().rev().map(|s| s.to_string()).collect();
        super::sort(&mut v);
        assert_eq!(v, want);
    }
}
