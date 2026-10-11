//! Decoding a 3D mouse's input reports.
//!
//! Every report starts with its report id:
//! - `0x01` motion: six signed 16-bit little-endian values, translation X, Y, Z then rotation
//!   about X, Y, Z (12 data bytes); or, on devices that split them, the translation alone
//!   (6 bytes).
//! - `0x02` rotation alone (6 bytes), on devices that split them.
//! - `0x03` buttons: a bit mask, button 0 the lowest bit of the first data byte.
//!
//! Other reports (status, battery) are ignored. Report bytes are untrusted: any length and any
//! value decode without panicking.

/// An axis value at full deflection, in device units (devices report about ±350).
pub const FULL_SCALE: f32 = 350.0;

/// Where the cap is pushed: each axis in −1..=1 (full deflection), in the device's own axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    /// Translation along X, Y, Z.
    pub translation: [f32; 3],
    /// Rotation about X, Y, Z.
    pub rotation: [f32; 3],
}

impl Motion {
    /// The cap is at rest.
    pub fn is_zero(&self) -> bool {
        self.translation.iter().chain(&self.rotation).all(|v| *v == 0.0)
    }
}

/// Which buttons are down: bit `n` is button `n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons(pub u64);

impl Buttons {
    /// Whether button `n` is down (buttons past 63 never are).
    pub fn is_down(self, n: u32) -> bool {
        n < 64 && self.0 & (1 << n) != 0
    }

    /// The buttons down now that were up in `before`.
    pub fn pressed_since(self, before: Buttons) -> Buttons {
        Buttons(self.0 & !before.0)
    }
}

/// What one report says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The cap's whole state (axes a report doesn't carry keep their last value).
    Motion(Motion),
    /// The buttons' whole state.
    Buttons(Buttons),
}

/// Decodes one device's reports. It keeps the last motion, so devices that send translation and
/// rotation in separate reports still yield the whole state each time.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    motion: Motion,
    /// The device sends rotation in its own report: report 1 carries translation only, whatever
    /// its length (Windows pads every report to the device's longest one).
    split: bool,
}

const REPORT_MOTION: u8 = 0x01;
const REPORT_ROTATION: u8 = 0x02;
const REPORT_BUTTONS: u8 = 0x03;

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Decode one input report (its first byte the report id). `None` for reports that say
    /// nothing about motion or buttons, and for ones too short to read.
    pub fn decode(&mut self, report: &[u8]) -> Option<Event> {
        let (&id, data) = report.split_first()?;
        match id {
            REPORT_MOTION => {
                let t = axes(data)?;
                self.motion.translation = t;
                if !self.split
                    && let Some(r) = data.get(6..).and_then(axes)
                {
                    self.motion.rotation = r;
                }
                Some(Event::Motion(self.motion))
            }
            REPORT_ROTATION => {
                self.motion.rotation = axes(data)?;
                self.split = true;
                Some(Event::Motion(self.motion))
            }
            REPORT_BUTTONS => {
                let mut bits = [0u8; 8];
                for (b, d) in bits.iter_mut().zip(data) {
                    *b = *d;
                }
                Some(Event::Buttons(Buttons(u64::from_le_bytes(bits))))
            }
            _ => None,
        }
    }
}

/// Three axes from the first 6 bytes, scaled to −1..=1.
fn axes(data: &[u8]) -> Option<[f32; 3]> {
    let value = |i: usize| -> Option<f32> {
        let raw = i16::from_le_bytes([*data.get(i)?, *data.get(i + 1)?]);
        Some((f32::from(raw) / FULL_SCALE).clamp(-1.0, 1.0))
    };
    Some([value(0)?, value(2)?, value(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    fn motion(d: &mut Decoder, s: &str) -> Motion {
        match d.decode(&hex(s)) {
            Some(Event::Motion(m)) => m,
            other => panic!("{s}: {other:?}"),
        }
    }

    fn units(v: [f32; 3]) -> [i32; 3] {
        v.map(|x| (x * FULL_SCALE).round() as i32)
    }

    // Reports recorded from a SpaceMouse Wireless (Bluetooth, 256F:C63A) on Windows.

    #[test]
    fn six_axes_in_one_report() {
        let mut d = Decoder::new();
        let m = motion(&mut d, "01 D4 FF 7C 00 28 00 F8 00 24 00 EF FF");
        assert_eq!(units(m.translation), [-44, 124, 40]);
        assert_eq!(units(m.rotation), [248, 36, -17]);
        let m = motion(&mut d, "01 00 00 E7 FF 00 00 00 00 00 00 00 00");
        assert_eq!(units(m.translation), [0, -25, 0]);
        assert_eq!(units(m.rotation), [0, 0, 0]);
        assert!(motion(&mut d, "01 00 00 00 00 00 00 00 00 00 00 00 00").is_zero());
    }

    #[test]
    fn full_deflection_is_one() {
        let mut d = Decoder::new();
        // Pushed all the way: 0x015E = 350.
        let m = motion(&mut d, "01 00 00 B2 FF 5E 01 8D FF 00 00 00 00");
        assert_eq!(m.translation[2], 1.0);
        // Past full scale (other devices, or noise) stays within −1..=1.
        let m = motion(&mut d, "01 00 80 FF 7F 00 00 00 00 00 00 00 00");
        assert_eq!(m.translation, [-1.0, 1.0, 0.0]);
    }

    #[test]
    fn buttons_are_a_bit_mask() {
        let mut d = Decoder::new();
        let mut buttons = |s: &str| match d.decode(&hex(s)) {
            Some(Event::Buttons(b)) => b,
            other => panic!("{s}: {other:?}"),
        };
        // Left, right, both, none.
        let left = buttons("03 01 00 00 00 00 00 00 00 00 00 00 00");
        assert!(left.is_down(0) && !left.is_down(1));
        let right = buttons("03 02 00 00 00 00 00 00 00 00 00 00 00");
        assert!(!right.is_down(0) && right.is_down(1));
        let both = buttons("03 03 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(both.pressed_since(left), right);
        assert_eq!(buttons("03 00 00 00 00 00 00 00 00 00 00 00 00"), Buttons(0));
        // Buttons in later bytes (larger devices): the second byte's bit 0 is button 8.
        assert!(buttons("03 00 01").is_down(8));
        assert!(!Buttons(u64::MAX).is_down(64));
    }

    #[test]
    fn other_reports_are_ignored() {
        let mut d = Decoder::new();
        motion(&mut d, "01 0A 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(d.decode(&hex("17 64 00 00 00 00 00 00 00 00 00 00 00")), None);
        // and leave the motion as it was.
        assert_eq!(units(motion(&mut d, "02 00 00 00 00 00 00").translation), [10, 0, 0]);
    }

    #[test]
    fn split_devices_keep_the_other_half() {
        let mut d = Decoder::new();
        let m = motion(&mut d, "01 0A 00 F6 FF 14 00");
        assert_eq!(units(m.translation), [10, -10, 20]);
        assert_eq!(units(m.rotation), [0, 0, 0]);
        let m = motion(&mut d, "02 05 00 00 00 FB FF");
        assert_eq!(units(m.translation), [10, -10, 20], "rotation keeps the translation");
        assert_eq!(units(m.rotation), [5, 0, -5]);
        // Report 1 padded with zeros (as Windows reads it) is translation only now.
        let m = motion(&mut d, "01 01 00 02 00 03 00 00 00 00 00 00 00");
        assert_eq!(units(m.translation), [1, 2, 3]);
        assert_eq!(units(m.rotation), [5, 0, -5], "padding doesn't clear the rotation");
    }

    #[test]
    fn short_reports_say_nothing() {
        let mut d = Decoder::new();
        for s in ["", "01", "01 00 00 00 00 00", "02 00 00", "03"] {
            let r = hex(s);
            let e = d.decode(&r);
            // A buttons report with no data is "no buttons"; anything else short is nothing.
            assert!(e.is_none() || e == Some(Event::Buttons(Buttons(0))), "{s}: {e:?}");
        }
    }

    /// Hostile input: random reports of any length decode without panicking, and every axis
    /// stays finite and within −1..=1.
    #[test]
    fn random_reports_never_panic() {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut d = Decoder::new();
        for _ in 0..100_000 {
            let len = (next() % 80) as usize;
            let mut report: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            // Mostly the ids that mean something.
            if let Some(id) = report.first_mut() {
                *id %= 5;
            }
            if let Some(Event::Motion(m)) = d.decode(&report) {
                for v in m.translation.iter().chain(&m.rotation) {
                    assert!(v.is_finite() && (-1.0..=1.0).contains(v), "{v} from {report:?}");
                }
            }
        }
    }
}
