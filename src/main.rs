//! anyshake-spsc-bridge: Lock-Free SPSC Ring Buffer & O(1) Recursive STA/LTA Engine
//! Target Hardware: SensePlex AnyShake Explorer (32-bit Sigma-Delta ADC @ 250 SPS)
//! Host Architecture: Apple Silicon M4 / ARM64 Edge Telemetry Node (Pier A-01, IIT Patna)

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 64-byte cache-line alignment for ARM64 / Apple Silicon M4 to prevent L1D false sharing
#[repr(align(64))]
pub struct CachePadded<T>(pub T);

/// Normalized 32-bit triaxial frame from AnyShake Explorer (EHZ, EHN, EHE + ENZ, ENN, ENE)
#[repr(C, align(32))]
#[derive(Copy, Clone, Debug, Default)]
pub struct AnyShakeFrame {
    pub timestamp_ns: u64,
    pub ehz_vel: i32,
    pub ehn_vel: i32,
    pub ehe_vel: i32,
    pub enz_acc: i32,
    pub enn_acc: i32,
    pub ene_acc: i32,
}

/// Single-Producer Single-Consumer (SPSC) Lock-Free Ring Buffer (Power-of-Two Capacity)
pub struct SpscRingBuffer<const N: usize> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    buffer: [UnsafeCell<AnyShakeFrame>; N],
}

unsafe impl<const N: usize> Sync for SpscRingBuffer<N> {}

impl<const N: usize> SpscRingBuffer<N> {
    #[inline(always)]
    pub fn try_push(&self, frame: AnyShakeFrame) -> bool {
        let head = self.head.0.load(Ordering::Relaxed);
        let next_head = (head + 1) & (N - 1);
        if next_head == self.tail.0.load(Ordering::Acquire) {
            return false; // Ring buffer full; backpressure engaged
        }
        unsafe { *self.buffer[head].get() = frame };
        self.head.0.store(next_head, Ordering::Release);
        true
    }

    #[inline(always)]
    pub fn try_pop(&self) -> Option<AnyShakeFrame> {
        let tail = self.tail.0.load(Ordering::Relaxed);
        if tail == self.head.0.load(Ordering::Acquire) {
            return None; // Ring buffer empty
        }
        let frame = unsafe { *self.buffer[tail].get() };
        self.tail.0.store((tail + 1) & (N - 1), Ordering::Release);
        Some(frame)
    }
}

/// O(1) Recursive Short-Term Average / Long-Term Average (STA/LTA) P-Wave Trigger
pub struct RecursiveStaLta<const STA_LEN: usize, const LTA_LEN: usize> {
    sta_window: [f64; STA_LEN],
    lta_window: [f64; LTA_LEN],
    sta_idx: usize,
    lta_idx: usize,
    sta_sum: f64,
    lta_sum: f64,
}

impl<const STA_LEN: usize, const LTA_LEN: usize> RecursiveStaLta<STA_LEN, LTA_LEN> {
    pub const fn new() -> Self {
        Self {
            sta_window: [0.0; STA_LEN],
            lta_window: [0.0; LTA_LEN],
            sta_idx: 0,
            lta_idx: 0,
            sta_sum: 0.0,
            lta_sum: 1e-9, // Prevent division by zero during cold start
        }
    }

    /// Updates STA/LTA ratio in strictly O(1) time per 250 SPS triaxial sample
    #[inline(always)]
    pub fn update(&mut self, ehz: i32, ehn: i32, ehe: i32) -> f64 {
        let energy = (ehz as f64).powi(2) + (ehn as f64).powi(2) + (ehe as f64).powi(2);

        self.sta_sum += energy - self.sta_window[self.sta_idx];
        self.sta_window[self.sta_idx] = energy;
        self.sta_idx = (self.sta_idx + 1) % STA_LEN;

        self.lta_sum += energy - self.lta_window[self.lta_idx];
        self.lta_window[self.lta_idx] = energy;
        self.lta_idx = (self.lta_idx + 1) % LTA_LEN;

        let sta_mean = self.sta_sum / (STA_LEN as f64);
        let lta_mean = self.lta_sum / (LTA_LEN as f64);
        sta_mean / lta_mean
    }
}

fn main() {
    let mut trigger = RecursiveStaLta::<250, 2500>::new(); // 1s STA / 10s LTA @ 250 SPS
    let ratio = trigger.update(1420, -310, 890);
    println!(
        "[anyshake-spsc-bridge] Pier A-01 (21.8803N, 87.5897E) | O(1) STA/LTA Ratio: {:.4}",
        ratio
    );
}
