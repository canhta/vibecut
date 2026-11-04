//! Admitted native callback boundary; see ADR 0003. Remove after binding repair.
use crate::{Error, ExecutionHooks, Stage};
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};
use whisper_rs::{FullParams, WhisperState};

#[derive(Default)]
struct State {
    cancel: AtomicBool,
    progress: AtomicU32,
    done: AtomicBool,
}

unsafe extern "C" fn abort(data: *mut c_void) -> bool {
    catch_unwind(|| {
        // SAFETY: run retains this Arc allocation until full returns and the
        // observer joins. The native caller only reads shared atomic fields.
        let state = unsafe { &*data.cast::<State>() };
        state.cancel.load(Ordering::Acquire)
    })
    .unwrap_or(true)
}

unsafe extern "C" fn progress(
    _: *mut whisper_rs::whisper_rs_sys::whisper_context,
    _: *mut whisper_rs::whisper_rs_sys::whisper_state,
    percent: i32,
    data: *mut c_void,
) {
    let _ = catch_unwind(|| {
        // SAFETY: same call-scoped allocation as abort; concurrent writes are atomic.
        let state = unsafe { &*data.cast::<State>() };
        state
            .progress
            .store(percent.clamp(0, 100) as u32, Ordering::Release);
    });
}

/// Params and their raw pointers never escape this call and are never cloned.
pub(super) fn run(
    native: &mut WhisperState,
    mut params: FullParams<'_, '_>,
    samples: &[f32],
    hooks: &dyn ExecutionHooks,
) -> Result<(), Error> {
    let state = Arc::new(State::default());
    let data = Arc::as_ptr(&state).cast_mut().cast::<c_void>();
    // SAFETY: trampolines have the exact native ABI, access only atomics, never
    // unwind into C, and do not access native context/state. Arc remains owned
    // here until synchronous full completes and the observer joins. No params
    // clone or callback replacement is exposed to consumers.
    unsafe {
        params.set_abort_callback(Some(abort));
        params.set_abort_callback_user_data(data);
        params.set_progress_callback(Some(progress));
        params.set_progress_callback_user_data(data);
    }
    std::thread::scope(|scope| {
        let observed = Arc::clone(&state);
        let observer = std::thread::Builder::new()
            .name("whisper-hooks".into())
            .spawn_scoped(scope, move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let mut last = 0;
                    while !observed.done.load(Ordering::Acquire) {
                        if !observed.cancel.load(Ordering::Acquire) && hooks.cancelled() {
                            observed.cancel.store(true, Ordering::Release);
                            hooks.cancelling();
                        }
                        let percent = observed.progress.load(Ordering::Acquire) as u8;
                        if !observed.cancel.load(Ordering::Acquire) && percent != last {
                            hooks.progress(Stage::Transcribe, percent);
                            last = percent;
                        }
                        std::thread::park_timeout(Duration::from_millis(2));
                    }
                }));
                if result.is_err() {
                    observed.cancel.store(true, Ordering::Release);
                }
                result.is_ok()
            })
            .map_err(|_| Error::Inference)?;
        let result = catch_unwind(AssertUnwindSafe(|| native.full(params, samples)));
        state.done.store(true, Ordering::Release);
        observer.thread().unpark();
        let healthy = observer.join().map_err(|_| Error::Inference)?;
        if !healthy {
            return Err(Error::Inference);
        }
        if hooks.cancelled() && !state.cancel.swap(true, Ordering::AcqRel) {
            hooks.cancelling();
        }
        if state.cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        result
            .map_err(|_| Error::Inference)?
            .map_err(|_| Error::Inference)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_abi_callbacks_share_only_bounded_atomic_state() {
        let state = Arc::new(State::default());
        let data = Arc::as_ptr(&state).cast_mut().cast::<c_void>();
        // SAFETY: data points to the live Arc allocation and native context/state
        // are deliberately unused by the callback. This exercises the real ABI.
        unsafe {
            assert!(!abort(data));
            progress(std::ptr::null_mut(), std::ptr::null_mut(), -1, data);
            assert_eq!(state.progress.load(Ordering::Acquire), 0);
            progress(std::ptr::null_mut(), std::ptr::null_mut(), 137, data);
            assert_eq!(state.progress.load(Ordering::Acquire), 100);
            state.cancel.store(true, Ordering::Release);
            assert!(abort(data));
        }
        assert_eq!(Arc::strong_count(&state), 1);
    }
}
