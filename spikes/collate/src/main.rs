//! Which CompareStringEx flags reproduce R's sort order on Windows?
use std::cmp::Ordering;
use windows_sys::Win32::Globalization::{CompareStringEx, SORT_STRINGSORT};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn cmp(a: &str, b: &str, flags: u32) -> Ordering {
    let (wa, wb) = (wide(a), wide(b));
    let r = unsafe {
        CompareStringEx(
            std::ptr::null(), // LOCALE_NAME_USER_DEFAULT
            flags,
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
        2 => Ordering::Equal,
        3 => Ordering::Greater,
        _ => panic!("CompareStringEx failed"),
    }
}

fn main() {
    // R's sort() of these strings (English_India.utf8), from sort_probe.R
    let want = [
        "a/#x.wav", "a/(x).wav", "a/_x.wav", "a/~x.wav", "a/1.wav", "a/10.wav", "a/9.wav", "a/a-b.wav", "a/a b.wav",
        "a/a_b.wav", "a/ab-.wav", "a/ab.wav", "a/aB.wav", "a/Ab.wav", "a/AB.wav", "a/deep/x.wav", "a/e.wav", "a/é.wav",
        "a/f.wav", "a/G-0007/x.wav", "a/G 0007/x.wav", "a/G.0007/x.wav", "a/G_0007/x.wav", "a/g0001_x.wav",
        "a/G0001_x.wav", "a/G0007/x.wav", "a/G00070.wav", "a/G0007x.wav", "a/x(1).wav", "a/x.flac", "a/x.FLAC",
        "a/x1.wav", "a/xa.wav", "a/z.wav", "a/Z.wav",
    ];
    for (name, flags) in [("default", 0u32), ("SORT_STRINGSORT", SORT_STRINGSORT)] {
        let mut v: Vec<&str> = want.iter().rev().copied().collect();
        v.sort_by(|a, b| cmp(a, b, flags));
        let same = v == want;
        println!("{name}: {}", if same { "same order as R" } else { "differs" });
        if !same {
            for (x, y) in v.iter().zip(want.iter()) {
                if x != y {
                    println!("   first difference: got {x}, R has {y}");
                    break;
                }
            }
        }
    }
}
