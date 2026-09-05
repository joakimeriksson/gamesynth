//! Main-thread → audio-thread parameter sharing without blocking the audio thread.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use godot::classes::native::AudioFrame;
use gamesynth_core::StereoFrame;

/// A value published by the main thread and polled by audio threads. Writers lock;
/// readers check `version` atomically and copy with `try_lock`, so mixing never blocks.
pub struct Shared<T: Copy> {
    value: Mutex<T>,
    version: AtomicU32,
}

impl<T: Copy> Shared<T> {
    pub fn new(value: T) -> Arc<Self> {
        Arc::new(Shared { value: Mutex::new(value), version: AtomicU32::new(1) })
    }

    pub fn publish(&self, value: T) {
        *self.value.lock().unwrap_or_else(|e| e.into_inner()) = value;
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Blocking read (main thread).
    pub fn snapshot(&self) -> (T, u32) {
        let v = *self.value.lock().unwrap_or_else(|e| e.into_inner());
        (v, self.version.load(Ordering::Acquire))
    }

    pub fn version(&self) -> u32 {
        self.version.load(Ordering::Acquire)
    }

    /// Non-blocking read (audio thread): returns the new value if `seen` is stale and the
    /// lock is free, and updates `seen`.
    pub fn poll(&self, seen: &mut u32) -> Option<T> {
        let v = self.version.load(Ordering::Acquire);
        if v == *seen {
            return None;
        }
        let guard = self.value.try_lock().ok()?;
        *seen = v;
        Some(*guard)
    }
}

/// Fill Godot's output buffer from a mono renderer, in bounded stack-sized chunks.
///
/// # Safety
/// `ptr` must point to at least `frames` writable `AudioFrame`s (Godot guarantees this for
/// the buffer passed to `_mix`).
pub unsafe fn fill_frames(ptr: *mut AudioFrame, frames: usize, mut render: impl FnMut(&mut [f32])) {
    let mut block = [0.0f32; gamesynth_core::MAX_BLOCK];
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(block.len());
        render(&mut block[..n]);
        for (i, s) in block[..n].iter().enumerate() {
            unsafe { ptr.add(done + i).write(AudioFrame { left: *s, right: *s }) };
        }
        done += n;
    }
}

/// Same as [`fill_frames`] for renderers that produce stereo directly.
///
/// # Safety
/// See [`fill_frames`].
pub unsafe fn fill_frames_stereo(ptr: *mut AudioFrame, frames: usize, mut render: impl FnMut(&mut [StereoFrame])) {
    let mut block = [StereoFrame::default(); gamesynth_core::MAX_BLOCK];
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(block.len());
        render(&mut block[..n]);
        for (i, s) in block[..n].iter().enumerate() {
            unsafe { ptr.add(done + i).write(AudioFrame { left: s.left, right: s.right }) };
        }
        done += n;
    }
}
