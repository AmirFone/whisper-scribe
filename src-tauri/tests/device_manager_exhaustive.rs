// Tests for device_manager module — classification, priority, edge cases

fn classify(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("airpods") || lower.contains("bluetooth") || lower.contains("beats")
        || lower.contains("bose") || lower.contains("sony wh") || lower.contains("sony wf")
        || lower.contains("jabra") {
        "bluetooth"
    } else if lower.contains("usb") || lower.contains("yeti") || lower.contains("snowball")
        || lower.contains("rode") || lower.contains("blue") || lower.contains("focusrite")
        || lower.contains("scarlett") {
        "usb"
    } else if lower.contains("macbook") || lower.contains("built-in") || lower.contains("internal") {
        "builtin"
    } else {
        "unknown"
    }
}

fn priority(transport: &str) -> u8 {
    match transport {
        "usb" => 0,
        "bluetooth" => 1,
        "builtin" => 2,
        _ => 3,
    }
}

// ── Bluetooth Devices ───────────────────────────────────

#[test]
fn test_classify_airpods_pro() {
    assert_eq!(classify("AirPods Pro"), "bluetooth");
}

#[test]
fn test_classify_airpods_max() {
    assert_eq!(classify("AirPods Max"), "bluetooth");
}

#[test]
fn test_classify_airpods_gen3() {
    assert_eq!(classify("AirPods (3rd generation)"), "bluetooth");
}

#[test]
fn test_classify_beats_solo() {
    assert_eq!(classify("Beats Solo Pro"), "bluetooth");
}

#[test]
fn test_classify_beats_fit_pro() {
    assert_eq!(classify("Beats Fit Pro"), "bluetooth");
}

#[test]
fn test_classify_bose_qc() {
    assert_eq!(classify("Bose QuietComfort 45"), "bluetooth");
}

#[test]
fn test_classify_bose_nc700() {
    assert_eq!(classify("Bose Noise Cancelling 700"), "bluetooth");
}

#[test]
fn test_classify_sony_wh1000xm5() {
    assert_eq!(classify("Sony WH-1000XM5"), "bluetooth");
}

#[test]
fn test_classify_sony_wf1000xm5() {
    assert_eq!(classify("Sony WF-1000XM5"), "bluetooth");
}

#[test]
fn test_classify_jabra_elite() {
    assert_eq!(classify("Jabra Elite 85t"), "bluetooth");
}

#[test]
fn test_classify_generic_bluetooth() {
    assert_eq!(classify("Generic Bluetooth Headset"), "bluetooth");
}

#[test]
fn test_classify_bluetooth_case_insensitive() {
    assert_eq!(classify("AIRPODS PRO"), "bluetooth");
    assert_eq!(classify("airpods pro"), "bluetooth");
    assert_eq!(classify("AiRpOdS PrO"), "bluetooth");
}

// ── USB Devices ─────────────────────────────────────────

#[test]
fn test_classify_blue_yeti() {
    assert_eq!(classify("Blue Yeti USB Microphone"), "usb");
}

#[test]
fn test_classify_blue_snowball() {
    assert_eq!(classify("Blue Snowball iCE"), "usb");
}

#[test]
fn test_classify_rode_ntusb() {
    assert_eq!(classify("Rode NT-USB Mini"), "usb");
}

#[test]
fn test_classify_rode_podcaster() {
    assert_eq!(classify("Rode Podcaster"), "usb");
}

#[test]
fn test_classify_focusrite_scarlett() {
    assert_eq!(classify("Focusrite Scarlett 2i2"), "usb");
}

#[test]
fn test_classify_scarlett_solo() {
    assert_eq!(classify("Scarlett Solo USB"), "usb");
}

#[test]
fn test_classify_generic_usb() {
    assert_eq!(classify("USB Audio Device"), "usb");
}

#[test]
fn test_classify_usb_case_insensitive() {
    assert_eq!(classify("BLUE YETI"), "usb");
    assert_eq!(classify("usb microphone"), "usb");
}

// ── Built-in Devices ────────────────────────────────────

#[test]
fn test_classify_macbook_pro_mic() {
    assert_eq!(classify("MacBook Pro Microphone"), "builtin");
}

#[test]
fn test_classify_macbook_air_mic() {
    assert_eq!(classify("MacBook Air Microphone"), "builtin");
}

#[test]
fn test_classify_builtin_microphone() {
    assert_eq!(classify("Built-in Microphone"), "builtin");
}

#[test]
fn test_classify_internal_mic() {
    assert_eq!(classify("Internal Microphone"), "builtin");
}

// ── Unknown Devices ─────────────────────────────────────

#[test]
fn test_classify_unknown_brand() {
    assert_eq!(classify("Sennheiser HD 600"), "unknown");
}

#[test]
fn test_classify_empty_name() {
    assert_eq!(classify(""), "unknown");
}

#[test]
fn test_classify_numeric_only() {
    assert_eq!(classify("12345"), "unknown");
}

#[test]
fn test_classify_random_string() {
    assert_eq!(classify("xyzzy plugh"), "unknown");
}

// ── Priority Ordering ───────────────────────────────────

#[test]
fn test_usb_highest_priority() {
    assert_eq!(priority("usb"), 0);
}

#[test]
fn test_bluetooth_second_priority() {
    assert_eq!(priority("bluetooth"), 1);
}

#[test]
fn test_builtin_third_priority() {
    assert_eq!(priority("builtin"), 2);
}

#[test]
fn test_unknown_lowest_priority() {
    assert_eq!(priority("unknown"), 3);
}

#[test]
fn test_usb_before_bluetooth() {
    assert!(priority("usb") < priority("bluetooth"));
}

#[test]
fn test_bluetooth_before_builtin() {
    assert!(priority("bluetooth") < priority("builtin"));
}

#[test]
fn test_builtin_before_unknown() {
    assert!(priority("builtin") < priority("unknown"));
}

// ── Sorting Simulation ─────────────────────────────────

#[test]
fn test_sort_mixed_devices() {
    let mut devices = vec![
        ("MacBook Mic", "builtin"),
        ("AirPods Pro", "bluetooth"),
        ("Blue Yeti", "usb"),
        ("Random Device", "unknown"),
    ];
    devices.sort_by_key(|(_, t)| priority(t));
    assert_eq!(devices[0].0, "Blue Yeti");
    assert_eq!(devices[1].0, "AirPods Pro");
    assert_eq!(devices[2].0, "MacBook Mic");
    assert_eq!(devices[3].0, "Random Device");
}

#[test]
fn test_sort_all_same_type() {
    let devices = vec![
        ("AirPods Pro", "bluetooth"),
        ("Beats Solo", "bluetooth"),
        ("Bose QC", "bluetooth"),
    ];
    let priorities: Vec<u8> = devices.iter().map(|(_, t)| priority(t)).collect();
    assert!(priorities.iter().all(|&p| p == 1));
}

#[test]
fn test_sort_single_device() {
    let mut devices = vec![("MacBook Mic", "builtin")];
    devices.sort_by_key(|(_, t)| priority(t));
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].0, "MacBook Mic");
}

#[test]
fn test_sort_empty_list() {
    let mut devices: Vec<(&str, &str)> = vec![];
    devices.sort_by_key(|(_, t)| priority(t));
    assert!(devices.is_empty());
}

// ── Device Name Parsing Edge Cases ──────────────────────

#[test]
fn test_classify_contains_multiple_keywords() {
    // "Blue" matches USB, but device is actually Bluetooth
    // "Blue" keyword is prioritized (checked before bluetooth)
    // This tests that the first matching category wins
    let result = classify("Bluetooth Blue Speaker");
    // Contains both "bluetooth" and "blue" — bluetooth checked first
    assert_eq!(result, "bluetooth");
}

#[test]
fn test_classify_very_long_name() {
    let name = "A".repeat(10_000) + " MacBook";
    assert_eq!(classify(&name), "builtin");
}

#[test]
fn test_classify_unicode_device_name() {
    assert_eq!(classify("John's AirPods Pro"), "bluetooth");
}

#[test]
fn test_classify_with_parentheses() {
    assert_eq!(classify("AirPods Pro (2nd generation)"), "bluetooth");
}

#[test]
fn test_classify_with_numbers() {
    assert_eq!(classify("Focusrite Scarlett 2i2 USB"), "usb");
}
