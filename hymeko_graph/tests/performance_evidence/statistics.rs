//! Distribution-preserving timing helpers for performance evidence.

use std::hint::black_box;
use std::time::Instant;

use super::telemetry::process_cpu_us;

pub const DEFAULT_WARMUPS: usize = 2;
pub const DEFAULT_SAMPLES: usize = 9;
const CPU_RESOLUTION_TARGET_US: f64 = 250_000.0;
const MAX_REPETITIONS_PER_SAMPLE: usize = 262_144;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub ordinal: usize,
    pub wall_us: f64,
    pub cpu_us: Option<f64>,
    pub checksum: u64,
}

pub use hymeko::util::measurement::Distribution;

#[derive(Debug)]
pub struct Measurement {
    pub warmups: usize,
    pub repetitions_per_sample: usize,
    pub samples: Vec<Sample>,
    pub wall: Distribution,
    pub cpu: Option<Distribution>,
    pub checksum: u64,
}

#[derive(Default)]
pub struct PhaseSamples {
    calls: Vec<[u128; 4]>,
}
impl PhaseSamples {
    /// The clock is injected so correctness tests never collect real timings.
    pub fn observe<T>(
        &mut self,
        mut clock: impl FnMut() -> u128,
        operation: impl FnOnce(&mut dyn FnMut(usize)) -> T,
    ) -> T {
        let mut previous = clock();
        let mut next = 0;
        let mut durations = [0; 4];
        let result = operation(&mut |phase| {
            assert_eq!(phase, next, "phase order");
            let now = clock();
            durations[phase] = now.checked_sub(previous).expect("monotonic phase clock");
            previous = now;
            next += 1;
        });
        assert_eq!(next, 4, "complete phase coverage");
        self.calls.push(durations);
        result
    }

    pub fn normalized(&self, measurement: &Measurement) -> Vec<[f64; 4]> {
        let repetitions = measurement.repetitions_per_sample;
        assert!(repetitions > 0);
        // measure_calibrated_verified: one probe, warmups, then measured calls.
        // Its post-batch verification calls deliberately have no observer.
        let start = 1 + measurement.warmups * repetitions;
        assert_eq!(
            self.calls.len(),
            start + measurement.samples.len() * repetitions
        );
        self.calls[start..]
            .chunks_exact(repetitions)
            .map(|chunk| {
                std::array::from_fn(|phase| {
                    chunk.iter().map(|row| row[phase] as f64).sum::<f64>()
                        / repetitions as f64
                        / 1000.0
                })
            })
            .collect()
    }
}

fn summarize(mut values: Vec<f64>) -> Distribution {
    assert!(
        values.len() >= 5,
        "performance evidence requires at least five samples"
    );
    Distribution::summarize(&mut values)
}

fn finish(warmups: usize, repetitions_per_sample: usize, samples: Vec<Sample>) -> Measurement {
    let checksum = samples[0].checksum;
    assert!(
        samples.iter().all(|sample| sample.checksum == checksum),
        "checksum drift between measured samples"
    );
    let wall = summarize(samples.iter().map(|sample| sample.wall_us).collect());
    let cpu_values: Option<Vec<f64>> = samples.iter().map(|sample| sample.cpu_us).collect();
    let cpu = cpu_values
        .map(summarize)
        .filter(|distribution| distribution.median > 0.0);
    Measurement {
        warmups,
        repetitions_per_sample,
        samples,
        wall,
        cpu,
        checksum,
    }
}

fn measure_repeated(
    repetitions_per_sample: usize,
    mut operation: impl FnMut() -> u64,
) -> Measurement {
    for _ in 0..DEFAULT_WARMUPS {
        for _ in 0..repetitions_per_sample {
            black_box(operation());
        }
    }
    let mut samples = Vec::with_capacity(DEFAULT_SAMPLES);
    for ordinal in 0..DEFAULT_SAMPLES {
        let cpu_before = process_cpu_us();
        let start = Instant::now();
        let mut checksum = None;
        for _ in 0..repetitions_per_sample {
            let current = black_box(operation());
            match checksum {
                None => checksum = Some(current),
                Some(expected) => assert_eq!(current, expected, "checksum drift within a sample"),
            }
        }
        let wall_us = start.elapsed().as_secs_f64() * 1e6 / repetitions_per_sample as f64;
        let cpu_after = process_cpu_us();
        let cpu_us = cpu_before
            .zip(cpu_after)
            .map(|(before, after)| (after - before).max(0.0) / repetitions_per_sample as f64);
        samples.push(Sample {
            ordinal,
            wall_us,
            cpu_us,
            checksum: checksum.expect("positive repetition count"),
        });
    }
    finish(DEFAULT_WARMUPS, repetitions_per_sample, samples)
}

/// Batch enough identical operations into each sample to rise above the coarse Windows process-CPU
/// accounting quantum, then report normalized per-operation wall and CPU values.
pub fn measure_calibrated(mut operation: impl FnMut() -> u64) -> Measurement {
    let start = Instant::now();
    black_box(operation());
    let probe_us = start.elapsed().as_secs_f64() * 1e6;
    let repetitions = if probe_us > 0.0 {
        (CPU_RESOLUTION_TARGET_US / probe_us).ceil() as usize
    } else {
        MAX_REPETITIONS_PER_SAMPLE
    }
    .clamp(1, MAX_REPETITIONS_PER_SAMPLE);
    measure_repeated(repetitions, operation)
}

/// Measure a cheap observation of each operation while computing one stronger checksum outside every
/// sample's wall/CPU boundary. This keeps an O(output) integrity digest from becoming benchmark work.
pub fn measure_calibrated_verified(mut operation: impl FnMut(bool) -> u64) -> Measurement {
    let start = Instant::now();
    black_box(operation(false));
    let probe_us = start.elapsed().as_secs_f64() * 1e6;
    let repetitions = if probe_us > 0.0 {
        (CPU_RESOLUTION_TARGET_US / probe_us).ceil() as usize
    } else {
        MAX_REPETITIONS_PER_SAMPLE
    }
    .clamp(1, MAX_REPETITIONS_PER_SAMPLE);

    for _ in 0..DEFAULT_WARMUPS {
        for _ in 0..repetitions {
            black_box(operation(false));
        }
    }

    let mut measured_sentinel = None;
    let mut samples = Vec::with_capacity(DEFAULT_SAMPLES);
    for ordinal in 0..DEFAULT_SAMPLES {
        let cpu_before = process_cpu_us();
        let start = Instant::now();
        for _ in 0..repetitions {
            let current = black_box(operation(false));
            match measured_sentinel {
                None => measured_sentinel = Some(current),
                Some(expected) => {
                    assert_eq!(current, expected, "measured result drift within a sample")
                }
            }
        }
        let wall_us = start.elapsed().as_secs_f64() * 1e6 / repetitions as f64;
        let cpu_after = process_cpu_us();
        let cpu_us = cpu_before
            .zip(cpu_after)
            .map(|(before, after)| (after - before).max(0.0) / repetitions as f64);
        let checksum = black_box(operation(true));
        samples.push(Sample {
            ordinal,
            wall_us,
            cpu_us,
            checksum,
        });
    }
    finish(DEFAULT_WARMUPS, repetitions, samples)
}

/// Measure creation through the returned ready-to-query object, but exclude its destruction.
/// The final measured object is retained for the subsequent warm-query measurement.
pub fn measure_factory<T>(mut factory: impl FnMut() -> (T, u64)) -> (Measurement, T) {
    for _ in 0..DEFAULT_WARMUPS {
        let (value, checksum) = factory();
        black_box(checksum);
        drop(value);
    }
    let mut samples = Vec::with_capacity(DEFAULT_SAMPLES);
    let mut retained = None;
    for ordinal in 0..DEFAULT_SAMPLES {
        let cpu_before = process_cpu_us();
        let start = Instant::now();
        let (value, checksum) = factory();
        black_box(&value);
        black_box(checksum);
        let wall_us = start.elapsed().as_secs_f64() * 1e6;
        let cpu_after = process_cpu_us();
        let cpu_us = cpu_before
            .zip(cpu_after)
            .map(|(before, after)| (after - before).max(0.0));
        samples.push(Sample {
            ordinal,
            wall_us,
            cpu_us,
            checksum,
        });
        retained = Some(value);
    }
    (
        finish(DEFAULT_WARMUPS, 1, samples),
        retained.expect("at least five samples retain a final value"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_samples_exclude_calibration_warmups_and_verification() {
        let mut phases = PhaseSamples::default();
        for call in 0..23 {
            let mut tick = 0;
            let step = if call < 5 { 99_000 } else { 1000 };
            let output = phases.observe(
                || {
                    tick += step;
                    tick
                },
                |boundary| {
                    for phase in 0..4 {
                        boundary(phase);
                    }
                    17
                },
            );
            assert_eq!(output, 17);
        }
        let measurement = finish(
            2,
            2,
            (0..9)
                .map(|ordinal| Sample {
                    ordinal,
                    wall_us: 10.0,
                    cpu_us: None,
                    checksum: 17,
                })
                .collect(),
        );
        assert_eq!(phases.normalized(&measurement), vec![[1.0; 4]; 9]);
    }

    #[test]
    #[should_panic(expected = "phase order")]
    fn phase_samples_reject_missing_boundary() {
        PhaseSamples::default().observe(|| 0, |boundary| boundary(1));
    }

    #[test]
    #[should_panic(expected = "monotonic phase clock")]
    fn phase_samples_reject_reverse_clock() {
        let mut tick = 10;
        PhaseSamples::default().observe(
            || {
                tick -= 1;
                tick
            },
            |boundary| boundary(0),
        );
    }

    #[test]
    fn summary_is_ordered_and_uses_every_sample() {
        let distribution = summarize(vec![9.0, 1.0, 7.0, 3.0, 5.0, 2.0, 8.0, 4.0, 6.0]);
        assert_eq!(distribution.q1, 3.0);
        assert_eq!(distribution.median, 5.0);
        assert_eq!(distribution.q3, 7.0);
        assert_eq!(distribution.worst, 9.0);
    }

    #[test]
    #[should_panic(expected = "at least five samples")]
    fn summary_rejects_single_shot_evidence() {
        let _ = summarize(vec![1.0]);
    }
}
