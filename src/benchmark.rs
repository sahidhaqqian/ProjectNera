//! Opt-in full-application benchmark used only by `--memory-benchmark*`.
use std::time::Instant;

use bevy::prelude::*;
use bevy_vrm1::prelude::{Vrm, Vrma};

use crate::avatar::NeraAvatar;

const WARMUP_SECONDS: f32 = 10.0;
const MINIMUM_VRMA_COUNT: usize = 20;

pub(crate) struct FullBenchmarkPlugin {
    pub(crate) d3: bool,
}

#[derive(Resource, Default)]
struct FullBenchmarkState {
    start: Option<(Instant, u64)>,
}

#[derive(Resource)]
struct FullBenchmarkConfig {
    d3: bool,
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemory {
    cb: u32,
    faults: u32,
    peak_working_set: usize,
    working_set: usize,
    peak_paged_pool: usize,
    paged_pool: usize,
    peak_nonpaged_pool: usize,
    nonpaged_pool: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private: usize,
}

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemory, size: u32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetProcessTimes(
        process: isize,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
}

fn process_cpu_ticks() -> u64 {
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    assert_ne!(
        unsafe { GetProcessTimes(-1, &mut creation, &mut exit, &mut kernel, &mut user) },
        0,
    );
    let ticks = |time: FileTime| (u64::from(time.high) << 32) | u64::from(time.low);
    ticks(kernel) + ticks(user)
}

fn sample_full_nera(
    mut state: ResMut<FullBenchmarkState>,
    config: Res<FullBenchmarkConfig>,
    avatars: Query<(), (With<NeraAvatar>, With<Vrm>)>,
    vrmas: Query<(), With<Vrma>>,
    mut exit: MessageWriter<AppExit>,
) {
    if avatars.is_empty() || vrmas.iter().count() < MINIMUM_VRMA_COUNT {
        return;
    }
    let (started_at, cpu_before) = state
        .start
        .get_or_insert_with(|| (Instant::now(), process_cpu_ticks()));
    let elapsed = started_at.elapsed();
    if elapsed.as_secs_f32() < WARMUP_SECONDS {
        return;
    }

    let mut memory = ProcessMemory::default();
    memory.cb = std::mem::size_of::<ProcessMemory>() as u32;
    assert_ne!(
        unsafe {
            GetProcessMemoryInfo(-1, &mut memory, std::mem::size_of::<ProcessMemory>() as u32)
        },
        0,
    );
    let cpu_one_core_percent =
        (process_cpu_ticks() - *cpu_before) as f64 / 10_000_000.0 / elapsed.as_secs_f64() * 100.0;
    let logical_processors = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!(
        "FULL_RESULT variant={} private_mib={:.1} working_set_mib={:.1} cpu_one_core_percent={:.2} cpu_total_percent={:.2} vrmas={}",
        if config.d3 { "H3" } else { "H0" },
        memory.private as f64 / 1_048_576.0,
        memory.working_set as f64 / 1_048_576.0,
        cpu_one_core_percent,
        cpu_one_core_percent / logical_processors as f64,
        vrmas.iter().count(),
    );
    exit.write(AppExit::Success);
}

impl Plugin for FullBenchmarkPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FullBenchmarkConfig { d3: self.d3 })
            .init_resource::<FullBenchmarkState>()
            .add_systems(Update, sample_full_nera);
    }
}
