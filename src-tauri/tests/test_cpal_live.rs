#[test]
#[ignore] // Run with: cargo test test_live_audio -- --ignored --nocapture
fn test_live_audio() {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let host = cpal::default_host();
    let device = host.default_input_device().expect("no input device");
    eprintln!("Device: {}", device.name().unwrap_or_default());

    let default_config = device.default_input_config().expect("no config");
    eprintln!("Default config: {:?}", default_config);

    if let Ok(configs) = device.supported_input_configs() {
        for c in configs {
            eprintln!("  Supported: {:?}", c);
        }
    }

    let config: cpal::StreamConfig = default_config.into();
    eprintln!("Using: {:?}", config);

    let sample_count = Arc::new(AtomicU64::new(0));
    let nonzero_count = Arc::new(AtomicU64::new(0));
    let sc = sample_count.clone();
    let nz = nonzero_count.clone();

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            sc.fetch_add(data.len() as u64, Ordering::Relaxed);
            let nzc: u64 = data.iter().filter(|&&s| s.abs() > 1e-8).count() as u64;
            nz.fetch_add(nzc, Ordering::Relaxed);
        },
        |err| eprintln!("Error: {err}"),
        None,
    ).expect("build stream");
    stream.play().expect("play");

    for i in 0..3 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let total = sample_count.load(Ordering::Relaxed);
        let nonzero = nonzero_count.load(Ordering::Relaxed);
        eprintln!("{}s: total={total}, nonzero={nonzero} ({:.1}%)", i + 1, if total > 0 { nonzero as f64 / total as f64 * 100.0 } else { 0.0 });
    }

    let total = sample_count.load(Ordering::Relaxed);
    let nonzero = nonzero_count.load(Ordering::Relaxed);
    assert!(total > 0, "Should receive samples");
    eprintln!("FINAL: {nonzero}/{total} non-zero ({:.1}%)", nonzero as f64 / total as f64 * 100.0);
}
