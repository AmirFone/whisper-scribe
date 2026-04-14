use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportType {
    Usb,
    Bluetooth,
    BuiltIn,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub transport: TransportType,
    pub priority: u8,
}

impl AudioDevice {
    fn new(name: String, transport: TransportType) -> Self {
        let priority = match transport {
            TransportType::Usb => 0,
            TransportType::Bluetooth => 1,
            TransportType::BuiltIn => 2,
            TransportType::Unknown => 3,
        };
        Self {
            name,
            transport,
            priority,
        }
    }
}

pub fn classify_device(name: &str) -> TransportType {
    let lower = name.to_lowercase();
    if lower.contains("airpods")
        || lower.contains("bluetooth")
        || lower.contains("beats")
        || lower.contains("bose")
        || lower.contains("sony wh")
        || lower.contains("sony wf")
        || lower.contains("jabra")
    {
        TransportType::Bluetooth
    } else if lower.contains("usb")
        || lower.contains("yeti")
        || lower.contains("snowball")
        || lower.contains("rode")
        || lower.contains("blue")
        || lower.contains("focusrite")
        || lower.contains("scarlett")
    {
        TransportType::Usb
    } else if lower.contains("macbook")
        || lower.contains("built-in")
        || lower.contains("internal")
    {
        TransportType::BuiltIn
    } else {
        TransportType::Unknown
    }
}

pub fn list_input_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let mut devices = Vec::new();

    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            let transport = classify_device(&name);
            devices.push(AudioDevice::new(name, transport));
        }
    }

    devices.sort_by_key(|d| d.priority);
    devices
}

pub fn select_best_device() -> Option<String> {
    let devices = list_input_devices();
    devices.first().map(|d| d.name.clone())
}

pub fn get_current_device_name() -> String {
    let host = cpal::default_host();
    host.default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "None".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_airpods() {
        assert_eq!(classify_device("AirPods Pro"), TransportType::Bluetooth);
    }

    #[test]
    fn test_classify_usb_mic() {
        assert_eq!(classify_device("Blue Yeti USB"), TransportType::Usb);
        assert_eq!(classify_device("Rode NT-USB"), TransportType::Usb);
    }

    #[test]
    fn test_classify_builtin() {
        assert_eq!(
            classify_device("MacBook Pro Microphone"),
            TransportType::BuiltIn
        );
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(
            classify_device("Some Random Device"),
            TransportType::Unknown
        );
    }

    #[test]
    fn test_device_priority_ordering() {
        let usb = AudioDevice::new("USB Mic".into(), TransportType::Usb);
        let bt = AudioDevice::new("AirPods".into(), TransportType::Bluetooth);
        let builtin = AudioDevice::new("MacBook Mic".into(), TransportType::BuiltIn);

        assert!(usb.priority < bt.priority);
        assert!(bt.priority < builtin.priority);
    }

    #[test]
    fn test_list_returns_sorted_by_priority() {
        let devices = list_input_devices();
        for w in devices.windows(2) {
            assert!(w[0].priority <= w[1].priority);
        }
    }
}
