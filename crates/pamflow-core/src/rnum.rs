//! Numbers exactly as R (on Windows) computes and data.table writes them.
//!
//! - [`rsum`]: R's `sum()` adds in x87 80-bit extended precision (R for
//!   Windows is built with MinGW, where `long double` is 80-bit), then rounds
//!   to double. Emulated in software with LLVM's APFloat (rustc_apfloat).
//! - [`fwrite_num`]: data.table's `writeFloat64` (fwrite.c, 1.18.x): the 15
//!   significant digits come from a table lookup and plain double arithmetic,
//!   which sometimes rounds the 15th digit differently from correct rounding.
//!   The lookup table (`expsig`, `exppow`) is rebuilt here from its definition
//!   (2^(i-1023) = expsig[i] * 10^exppow[i], rounded to double), not copied.

use num_bigint::BigUint;
use rustc_apfloat::ieee::{Double, X87DoubleExtended};
use rustc_apfloat::{Float, FloatConvert, Round};
use std::sync::OnceLock;

/// R's `sum(x, na.rm = TRUE)` over the given values (NaN skipped).
pub fn rsum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut loses = false;
    let mut acc = X87DoubleExtended::ZERO;
    for x in values {
        if x.is_nan() {
            continue;
        }
        let v: X87DoubleExtended = Double::from_bits(x.to_bits() as u128).convert(&mut loses).value;
        acc = acc.add_r(v, Round::NearestTiesToEven).value;
    }
    let back: Double = acc.convert(&mut loses).value;
    f64::from_bits(back.to_bits() as u64)
}

/// `expsig[i]`, `exppow[i]`: 2^(i-1023) = expsig * 10^exppow, expsig in [1, 10)
/// rounded to the nearest double.
fn tables() -> &'static (Vec<f64>, Vec<i32>) {
    static T: OnceLock<(Vec<f64>, Vec<i32>)> = OnceLock::new();
    T.get_or_init(|| {
        let mut sig = Vec::with_capacity(2048);
        let mut pow = Vec::with_capacity(2048);
        for i in 0..2048i32 {
            let k = i - 1023;
            // e = floor(log10(2^k)), checked exactly below
            let mut e = ((k as f64) * std::f64::consts::LOG10_2).floor() as i32;
            let ratio = |e: i32| -> (BigUint, BigUint) {
                // 2^k / 10^e as num/den
                let two = |n: i32| BigUint::from(1u32) << (n as u64);
                let ten = |n: i32| BigUint::from(10u32).pow(n as u32);
                match (k >= 0, e >= 0) {
                    (true, true) => (two(k), ten(e)),
                    (true, false) => (two(k) * ten(-e), BigUint::from(1u32)),
                    (false, true) => (BigUint::from(1u32), two(-k) * ten(e)),
                    (false, false) => (ten(-e), two(-k)),
                }
            };
            let (mut num, mut den) = ratio(e);
            if num < den {
                e -= 1;
                (num, den) = ratio(e);
            } else if num >= &den * BigUint::from(10u32) {
                e += 1;
                (num, den) = ratio(e);
            }
            sig.push(round_ratio(&num, &den));
            pow.push(e);
        }
        (sig, pow)
    })
}

/// num/den (in [1, 10)) rounded to the nearest double, ties to even.
fn round_ratio(num: &BigUint, den: &BigUint) -> f64 {
    // scale so the quotient has 53 significant bits
    let mut s = 52u64;
    while (num << s) / den >= (BigUint::from(1u64) << 53u64) {
        s -= 1;
    }
    let scaled = num << s;
    let q = &scaled / den;
    let r = &scaled - &q * den;
    let mut q: u64 = q.try_into().unwrap();
    let twice = r << 1u64;
    if twice > *den || (twice == *den && q & 1 == 1) {
        q += 1;
    }
    q as f64 / (1u64 << s) as f64
}

/// A double as data.table's fwrite writes it (scipen = 0, dec = ".").
pub fn fwrite_num(x: f64) -> String {
    if x.is_nan() {
        return String::new();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-Inf".into() } else { "Inf".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    let mut out = String::new();
    let x = if x < 0.0 {
        out.push('-');
        -x
    } else {
        x
    };
    let bits = x.to_bits();
    let mut fraction = bits & 0xF_FFFF_FFFF_FFFF;
    let exponent = ((bits >> 52) & 0x7FF) as usize;
    let mut acc = 0.0f64;
    let mut i: i32 = 52;
    if fraction != 0 {
        while fraction & 0xFF == 0 {
            fraction >>= 8;
            i -= 8;
        }
        while fraction != 0 {
            if fraction & 1 == 1 {
                acc += (-(i as f64)).exp2(); // sigparts[i] = 2^-i (exact)
            } else {
                acc += 0.0;
            }
            i -= 1;
            fraction >>= 1;
        }
    }
    let (sig, pow) = tables();
    let mut y = (1.0 + acc) * sig[exponent];
    let mut exp = pow[exponent];
    if y >= 9.99999999999999 {
        y /= 10.0;
        exp += 1;
    }
    let mut l = (y * 1_000_000_000_000_000_f64) as u64;
    if l % 10 >= 5 {
        l += 10;
    }
    l /= 10;
    if l == 0 {
        return "0".into();
    }
    let mut trail_zero = 0;
    while l.is_multiple_of(10) {
        l /= 10;
        trail_zero += 1;
    }
    let mut sf: i32 = 15 - trail_zero;
    if sf == 0 {
        sf = 1;
        exp += 1;
    }
    let digits = l.to_string(); // sf digits
    let mut dr = sf - exp - 1;
    let width;
    let mut dl0 = 0;
    if dr <= 0 {
        dl0 = -dr;
        dr = 0;
        width = sf + dl0;
    } else if sf > dr {
        width = sf + 1;
    } else {
        dl0 = 1;
        width = dr + 1 + dl0;
    }
    let sci_width = sf + (sf > 1) as i32 + 2 + if exp.abs() > 99 { 3 } else { 2 };
    if width <= sci_width {
        if dr == 0 {
            out.push_str(&digits);
            out.push_str(&"0".repeat(dl0 as usize));
        } else if sf > dr {
            let left = (sf - dr) as usize;
            out.push_str(&digits[..left]);
            out.push('.');
            out.push_str(&digits[left..]);
        } else {
            out.push_str("0.");
            out.push_str(&"0".repeat((dr - sf) as usize));
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        if sf > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.abs()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_fwrite() {
        for (x, want) in [
            (3599.701333333333, "3599.70133333333"),
            (345571372.0, "345571372"),
            (76.7, "76.7"),
            (10.0, "10"),
            (0.0077083333333333335, "0.00770833333333333"),
            (1e-7, "1e-07"),
            (1.5e20, "1.5e+20"),
            (123456.0, "123456"),
            (30460.0, "30460"),
            (0.0072, "0.0072"),
            (-0.25, "-0.25"),
        ] {
            assert_eq!(fwrite_num(x), want, "{x}");
        }
    }

    #[test]
    fn sum_in_extended_precision() {
        assert_eq!(rsum([0.1, 0.2, 0.3]), 0.6);
        assert_eq!(rsum([1e16, 1.0, -1e16]), 1.0); // 80-bit keeps the 1
        assert_eq!(rsum([f64::NAN, 2.0]), 2.0);
    }
}
