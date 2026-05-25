use core_foundation::base::{TCFType, CFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::{CFString, CFStringRef};
use io_kit_sys::ps::power_sources::*;

use crate::platform::PlatformAdapter;
use crate::state::model::BatteryState;

#[derive(Debug, Default)]
pub struct MacosPlatformAdapter;

impl PlatformAdapter for MacosPlatformAdapter {
    fn battery_state(&self) -> Option<BatteryState> {
        unsafe {
            let blob = IOPSCopyPowerSourcesInfo();
            if blob.is_null() {
                return None;
            }

            let sources = IOPSCopyPowerSourcesList(blob);
            if sources.is_null() {
                return None;
            }

            let count = core_foundation::array::CFArrayGetCount(sources);
            if count == 0 {
                return None;
            }

            let source = core_foundation::array::CFArrayGetValueAtIndex(sources, 0);
            let description = IOPSGetPowerSourceDescription(blob, source);
            if description.is_null() {
                return None;
            }

            let description: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_get_rule(description as CFDictionaryRef);
            
            let percent_key = CFString::new("Current Capacity");
            let max_key = CFString::new("Max Capacity");
            let power_source_key = CFString::new("Power Source State");

            let current_capacity = description.find(&percent_key);
            let max_capacity = description.find(&max_key);
            let power_source = description.find(&power_source_key);

            let percent = if let (Some(cur), Some(max)) = (current_capacity, max_capacity) {
                let cur = CFNumber::wrap_under_get_rule(cur.as_CFTypeRef() as CFNumberRef).to_f64().unwrap_or(0.0);
                let max = CFNumber::wrap_under_get_rule(max.as_CFTypeRef() as CFNumberRef).to_f64().unwrap_or(100.0);
                (cur / max * 100.0) as f32
            } else {
                0.0
            };

            let on_ac_power = if let Some(ps) = power_source {
                let ps = CFString::wrap_under_get_rule(ps.as_CFTypeRef() as CFStringRef).to_string();
                ps != "Battery Power"
            } else {
                false
            };

            Some(BatteryState {
                percent,
                on_ac_power,
            })
        }
    }

    fn gpu_usage_percent(&self) -> Option<f32> {
        Some(0.0) 
    }

    fn thermal_state(&self) -> Option<String> {
        Some("Nominal".into())
    }

    fn accessibility_available(&self) -> bool {
        false
    }

    fn startup_registration_supported(&self) -> bool {
        true
    }
}
