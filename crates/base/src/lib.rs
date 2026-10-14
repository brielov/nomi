//! Shared vocabulary for every nomi crate: file identities, interned
//! names, text ranges, line indexes, upstream's reports, the releases
//! nomi emulates, and the small pieces several crates would otherwise
//! copy: GHC's character classes, upstream's suggestions, UTF-8 widths,
//! stack growth for deep recursion, hex digests and poison-tolerant
//! locks. Depends on no other nomi crate.

use std::sync::{Mutex, MutexGuard, PoisonError};

mod compat;
pub mod diagnostic;
mod file_id;
pub mod ghc;
mod interned;
mod line_index;
mod stack;
pub mod suggest;
pub mod utf8;

pub use compat::{Compat, UnknownCompat};
pub use diagnostic::{Doc, Report};
pub use file_id::FileId;
pub use interned::Text;
pub use line_index::{ColumnUnit, ElmPosition, LineCol, LineIndex, PositionError};
pub use stack::{STACK_SIZE, grow, grow_large, on_core_stack};
/// Byte offsets and ranges, shared with rowan so trees need no conversion.
pub use text_size::{TextLen, TextRange, TextSize};

/// `bytes` as lowercase hexadecimal, two digits a byte, as digests are
/// written.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 * bytes.len());
    for &byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0xF)]));
    }
    out
}

/// Locks `mutex`, recovering it if a thread panicked while holding it,
/// so one panic doesn't take down every later user of what it guards.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_two_lowercase_digits_a_byte() {
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[test]
    fn a_poisoned_lock_still_gives_its_value() {
        let mutex = Mutex::new(1);
        let _ = std::panic::catch_unwind(|| {
            let _guard = mutex.lock();
            panic!("poisoning the lock");
        });
        assert!(mutex.is_poisoned());
        *lock(&mutex) += 1;
        assert_eq!(*lock(&mutex), 2);
    }
}
