//! A trava comum do crate.

use std::sync::{Mutex, MutexGuard};

/// Trava `m` ignorando o envenenamento: o que fica sob estas travas é
/// coerente a cada passo, e um pânico numa tarefa não deve derrubar as outras.
pub(crate) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
