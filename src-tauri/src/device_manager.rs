use cpal::traits::{DeviceTrait, HostTrait};

/// Substrings that identify a Bluetooth audio device. Bluetooth inputs trigger
/// the A2DP→HFP codec switch on macOS, which degrades all system audio output
/// quality, so we skip them entirely.
pub const BLUETOOTH_KEYWORDS: &[&str] =
    &["airpods", "bluetooth", "beats", "bose", "sony wh", "sony wf", "jabra"];

/// Substrings that identify the macOS built-in microphone.
pub const BUILTIN_KEYWORDS: &[&str] = &["macbook", "built-in", "internal"];

pub fn is_bluetooth_device(name: &str) -> bool {
    let lower = name.to_lowercase();
    BLUETOOTH_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

pub fn is_builtin_device(name: &str) -> bool {
    let lower = name.to_lowercase();
    BUILTIN_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Pick the best input device, preferring built-in mic over arbitrary inputs
/// and skipping all Bluetooth devices. Falls back to the host's default input
/// if no preferred device is available.
pub fn select_best_input(host: &cpal::Host) -> Option<cpal::Device> {
    let devices = host.input_devices().ok()?;
    let mut built_in: Option<cpal::Device> = None;
    let mut fallback: Option<cpal::Device> = None;

    for device in devices {
        let name = device.name().unwrap_or_default();

        if is_bluetooth_device(&name) {
            log::info!("Skipping Bluetooth input: {name}");
            continue;
        }

        if is_builtin_device(&name) {
            log::info!("Found built-in mic: {name}");
            built_in = Some(device);
        } else if fallback.is_none() {
            fallback = Some(device);
        }
    }

    built_in.or(fallback).or_else(|| host.default_input_device())
}

pub fn get_current_device_name() -> String {
    let host = cpal::default_host();
    select_best_input(&host)
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "None".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bluetooth_airpods() {
        assert!(is_bluetooth_device("AirPods Pro"));
        assert!(is_bluetooth_device("Beats Studio"));
        assert!(is_bluetooth_device("Sony WH-1000XM5"));
        assert!(is_bluetooth_device("Jabra Evolve"));
    }

    #[test]
    fn test_is_bluetooth_negatives() {
        assert!(!is_bluetooth_device("MacBook Pro Microphone"));
        assert!(!is_bluetooth_device("Blue Yeti USB"));
        assert!(!is_bluetooth_device(""));
    }

    #[test]
    fn test_is_builtin() {
        assert!(is_builtin_device("MacBook Pro Microphone"));
        assert!(is_builtin_device("Built-in Microphone"));
        assert!(is_builtin_device("Internal Mic"));
        assert!(!is_builtin_device("AirPods Pro"));
    }
}
